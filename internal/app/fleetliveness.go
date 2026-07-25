package app

import (
	"context"
	"sync"
	"time"
)

// This file is the second half of milestone F2: making a one-sided edge failure
// AUDIBLE. The field finding that motivated it (2026-07-24) was not the outage — it
// was the silence on both sides of it. One box's dialer had written the peer edge
// off while the peer still listed it `serve` with pending=2: two fleet sends queued
// "for its next session" that could never arrive. Neither box said anything; it took
// the operator relaying a worker's own self-report to notice.
//
// The rule this encodes: QUEUED OUTBOUND WITH NO SESSION IS A SYMPTOM, NOT A STATE.
// Frames waiting for a peer that has not been seen in minutes must surface — in
// fleet.log, in `fleet ls`, and in the fleet tool — instead of accumulating silently
// until the pending cap eats the oldest one.

// fleetStalePendingAfter is how long an edge may hold queued outbound with no session
// before it is called stale. Long enough that an ordinary reconnect (backoff maxes at
// 60s) never trips it, short enough that a stuck worker is noticed within one operator
// check-in rather than one day.
const fleetStalePendingAfter = 10 * time.Minute

// fleetLivenessSweep is how often the box re-checks every live edge for the stale
// condition. Cheap: one journal-derived pending depth per edge, far off any hot path.
const fleetLivenessSweep = time.Minute

// fleetStaleRepeatEvery throttles the fleet.log alarm per edge, so an edge that stays
// stale for a day writes a couple of dozen lines, not a thousand.
const fleetStaleRepeatEvery = 30 * time.Minute

// FleetStalePending reports the F2 liveness alarm for one edge: it holds undelivered
// outbound, it has no live session, and its last contact is older than
// fleetStalePendingAfter (an edge that was NEVER seen counts as stale the moment it
// has something queued for it). Exported so the CLI — which cannot read the running
// box's session map — applies the identical predicate to its disk-derived view.
func FleetStalePending(pending int, connected bool, lastSeenAt string, now time.Time) bool {
	if pending <= 0 || connected {
		return false
	}
	if lastSeenAt == "" {
		return true
	}
	ts, err := time.Parse(time.RFC3339Nano, lastSeenAt)
	if err != nil {
		return true
	}
	return now.Sub(ts) > fleetStalePendingAfter
}

// fleetStaleWarns remembers the last alarm time per edge so the sweep can throttle.
type fleetStaleWarns struct {
	mu   sync.Mutex
	last map[string]time.Time
}

func (w *fleetStaleWarns) due(edgeID string, now time.Time) bool {
	w.mu.Lock()
	defer w.mu.Unlock()
	if w.last == nil {
		w.last = map[string]time.Time{}
	}
	if prev, ok := w.last[edgeID]; ok && now.Sub(prev) < fleetStaleRepeatEvery {
		return false
	}
	w.last[edgeID] = now
	return true
}

func (w *fleetStaleWarns) clear(edgeID string) {
	w.mu.Lock()
	defer w.mu.Unlock()
	delete(w.last, edgeID)
}

// runFleetLivenessSweep is the F2 watchdog goroutine: every fleetLivenessSweep it
// looks for edges accumulating outbound into a void and says so in fleet.log, once per
// edge per fleetStaleRepeatEvery. It only ever READS the registry and journals — it
// never tombstones, never drops frames, and never touches the operator path — because
// the correct response to "this peer went quiet" is an operator's judgment, not an
// automatic teardown. A no-op on a box with no fleet store.
func (s *Server) runFleetLivenessSweep(ctx context.Context) {
	if s.fleetStore == nil {
		<-ctx.Done()
		return
	}
	warns := &fleetStaleWarns{}
	tick := time.NewTicker(s.fleetSweepInterval())
	defer tick.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-tick.C:
			s.sweepFleetLivenessOnce(warns, time.Now())
		}
	}
}

// fleetSweepInterval is the sweep period, overridable in tests.
func (s *Server) fleetSweepInterval() time.Duration {
	if s.fleetSweepEvery > 0 {
		return s.fleetSweepEvery
	}
	return fleetLivenessSweep
}

// sweepFleetLivenessOnce is one pass, factored out so a test can drive it directly.
// It returns the edge ids it alarmed on this pass.
func (s *Server) sweepFleetLivenessOnce(warns *fleetStaleWarns, now time.Time) []string {
	edges, err := s.fleetStore.Edges()
	if err != nil {
		s.fleetLog.logf("fleet liveness sweep: load edges failed: %v", err)
		return nil
	}
	var alarmed []string
	for _, e := range edges {
		if e.Removed() {
			continue
		}
		_, connected := s.fleetSessionWriter(e.EdgeID)
		pending, perr := s.fleetStore.PendingDepth(e.EdgeID)
		if perr != nil {
			continue
		}
		if !FleetStalePending(pending, connected, e.LastSeenAt, now) {
			warns.clear(e.EdgeID)
			continue
		}
		alarmed = append(alarmed, e.EdgeID)
		if !warns.due(e.EdgeID, now) {
			continue
		}
		cold := ""
		if e.Unreachable != nil {
			cold = " (edge is flagged unreachable — cold retry)"
		}
		s.fleetLog.logf("edge=%s alias=%q STALE PENDING: %d outbound frame(s) queued, no session, last_seen=%s%s — the peer may have written this edge off; check `hotline fleet ls` on both boxes",
			shortEdgeID(e.EdgeID), e.Alias, pending, orNone(e.LastSeenAt), cold)
	}
	return alarmed
}
