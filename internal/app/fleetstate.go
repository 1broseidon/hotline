package app

import (
	"bytes"
	"encoding/json"
	"time"
)

// This file is Lane L4's operator-awareness surface (a2a-design-v2 §6): the
// fleet_state transient. Unlike fleet traffic itself (which rides its OWN transport,
// never the operator path), fleet_state is an APP-facing transient — the same family
// as agent_state — that gives George's operator app a full snapshot of the fleet
// without ever exposing peer message content. It is a FULL SNAPSHOT (the app treats
// each frame as a complete replacement, so add/remove/liveness all converge), pushed
// (a) to a device right after its mailbox drains (snapshotFleetStateTo, next to the
// agent_state snapshot) and (b) to every active device on a throttled change
// (fleetStateChanged, ≥30s apart). Old clients ignore the unknown type; the app's
// fleet screen renders it in a later mobile batch. Anything URGENT still reaches
// George through the agent reporting up — judgment, not plumbing (§6).

// fleetStateThrottle is the L4 change-broadcast floor (§6: "on throttled change,
// ≥30s apart"). Deliberately far coarser than agent_state's 2s: fleet liveness is
// ambient awareness, not interactive, so a per-frame counter tick coalesces into at
// most one operator push every 30s.
const fleetStateThrottle = 30 * time.Second

// fleetStateEdge is one edge in the fleet_state snapshot (§6, task shape). It carries
// the operator-facing view only: never a secret, never message text. edge_id (not the
// renameable alias) is the durable address (F17). dead/dead_reason project the
// tombstone (removed | revoked | unreachable); key_mismatch is the additive L2/M5
// trust-on-first-use safety signal (an operator wants to know a peer's key changed).
type fleetStateEdge struct {
	EdgeID      string `json:"edge_id"`
	Alias       string `json:"alias"`
	Direction   string `json:"direction"`
	Connected   bool   `json:"connected"`
	LastSeen    string `json:"last_seen,omitempty"`
	Sent24h     uint64 `json:"sent_24h"`
	Recv24h     uint64 `json:"recv_24h"`
	Pending     int    `json:"pending"`
	Dropped24h  uint64 `json:"dropped_24h"`
	Dead        bool   `json:"dead,omitempty"`
	DeadReason  string `json:"dead_reason,omitempty"`
	KeyMismatch bool   `json:"key_mismatch,omitempty"`
	// F2: unreachable is the RECOVERABLE cold-retry dead-mark — deliberately separate
	// from dead/dead_reason, which stay reserved for the terminal tombstones (operator
	// removed, peer revoked). stale_pending is the asymmetric-death alarm: outbound
	// queued for a peer with no session and no recent contact.
	Unreachable  bool `json:"unreachable,omitempty"`
	StalePending bool `json:"stale_pending,omitempty"`
}

// fleetStateSnapshot is the full fleet_state.state payload — the app replaces its
// whole fleet view with it. Edges is always non-nil so the frame carries [] not null.
type fleetStateSnapshot struct {
	Edges []fleetStateEdge `json:"edges"`
}

// fleetStateFrame builds the transient fleet_state snapshot frame (§6). Like the
// agent_state/typing family it is seq'd, never durable, never acked, never replayed.
func fleetStateFrame(seq uint64, snap fleetStateSnapshot) []byte {
	return mustMarshal(map[string]any{"t": "fleet_state", "seq": seq, "state": snap})
}

// buildFleetState assembles the current fleet snapshot from the registry + live
// session map + per-edge state (§6). ok=false when the box has no fleet store (a
// bare-Server test / empty state dir) — the caller then emits nothing. Read fresh
// each call so the snapshot is disk-honest at emit time (a CLI `fleet rm` in another
// process shows up live on the next drain/change).
func (s *Server) buildFleetState() (fleetStateSnapshot, bool) {
	if s.fleetStore == nil {
		return fleetStateSnapshot{}, false
	}
	edges, err := s.fleetStore.Edges()
	if err != nil {
		s.fleetLog.logf("fleet_state build: load edges failed: %v", err)
		return fleetStateSnapshot{}, false
	}
	now := time.Now()
	snap := fleetStateSnapshot{Edges: make([]fleetStateEdge, 0, len(edges))}
	for _, e := range edges {
		row := fleetStateEdge{
			EdgeID:    e.EdgeID,
			Alias:     e.Alias,
			Direction: string(e.Direction),
			LastSeen:  e.LastSeenAt,
		}
		if e.Removed() {
			row.Dead = true
			if e.Tombstone != nil {
				row.DeadReason = e.Tombstone.Reason
			}
			snap.Edges = append(snap.Edges, row)
			continue
		}
		// connected is the REAL in-box liveness (the live serve/dial session map) —
		// authoritative here because fleet_state is built inside the running box.
		_, row.Connected = s.fleetSessionWriter(e.EdgeID)
		if st, serr := s.fleetStore.EdgeState(e.EdgeID); serr == nil {
			row.Sent24h, row.Recv24h, row.Dropped24h = st.windowCounts(now)
			row.KeyMismatch = st.KeyFPMismatch
		}
		if depth, derr := s.fleetStore.PendingDepth(e.EdgeID); derr == nil {
			row.Pending = depth
			row.StalePending = FleetStalePending(depth, row.Connected, e.LastSeenAt, now)
		}
		row.Unreachable = e.Unreachable != nil
		snap.Edges = append(snap.Edges, row)
	}
	return snap, true
}

// fleetStateChanged is the L4 change hook (§6): it schedules a throttled full-snapshot
// broadcast to every active operator device, at most one per fleetStateThrottle with
// a trailing send (mirrors agentStateChanged, coarser window). Called whenever fleet
// liveness or a counter changes (a send, an inbound commit, a session attach/detach,
// a rm) — INCLUDING from FleetSend/inbound while a per-edge lock is held.
//
// Post-mortem fix A3: it NEVER builds the snapshot (buildFleetState / PendingDepth
// disk scans) inline on the caller's goroutine — it only ARMS the fs timer (mark-dirty),
// and the build+broadcast run on the timer goroutine. So a fleet_msg send/receive never
// pays a full-journal scan under fsMu (or the per-edge lock it is called beneath). The
// wait is clamped to 0 (not negative) so the first change after a quiet period fires
// promptly, still off the caller's stack.
func (s *Server) fleetStateChanged() {
	if s.fleetStore == nil {
		return
	}
	s.fsMu.Lock()
	defer s.fsMu.Unlock()
	if s.fsClosed {
		return
	}
	if s.fsTimer != nil {
		return // a trailing send is already scheduled; it will carry the latest state
	}
	throttle := s.fsThrottle
	if throttle <= 0 {
		throttle = fleetStateThrottle
	}
	wait := throttle - time.Since(s.fsLastSent)
	if wait < 0 {
		wait = 0
	}
	s.fsTimer = time.AfterFunc(wait, func() {
		s.fsMu.Lock()
		defer s.fsMu.Unlock()
		s.fsTimer = nil
		if s.fsClosed {
			return
		}
		s.broadcastFleetStateLocked()
	})
}

// broadcastFleetStateLocked computes and sends a full fleet snapshot to every active
// FLEET-STATE-CAPABLE device (A1 capability gate), deduping against the last broadcast
// so a no-op re-trigger is silent. Caller holds fsMu (the timer goroutine, per A3 — the
// buildFleetState disk scan runs here, never on a fleet_msg hot path). An empty fleet
// (no edges ever) is never announced before anything was shown, matching the
// agent_state "no noise" guard.
func (s *Server) broadcastFleetStateLocked() {
	s.fsLastSent = time.Now()
	snap, ok := s.buildFleetState()
	if !ok {
		return
	}
	body, err := json.Marshal(snap)
	if err != nil {
		return
	}
	if bytes.Equal(body, s.fsLastSnap) {
		return // no change since the last broadcast
	}
	if len(snap.Edges) == 0 && s.fsLastSnap == nil {
		return // never announce an empty fleet before anything was shown
	}
	s.fsLastSnap = append([]byte(nil), body...)
	s.emitFleetState(func(seq uint64) []byte { return fleetStateFrame(seq, snap) })
}

// snapshotFleetStateTo sends the current fleet snapshot to a single device right
// after its mailbox drains (§6, the agent_state-snapshot moment). A1 capability gate:
// it is a no-op unless the device advertised fleet-state support in its hello, so a
// device that cannot render fleet_state never receives it (no shipped client does).
// Once an edge exists — even a tombstoned/dead one, which stays in the registry — the
// full snapshot is pushed to a fleet-state-capable device so it can correct a stale view.
func (s *Server) snapshotFleetStateTo(deviceID string) {
	if !s.deviceWantsFleetState(deviceID) {
		return
	}
	snap, ok := s.buildFleetState()
	if !ok || len(snap.Edges) == 0 {
		return
	}
	s.emitFleetStateTo(deviceID, func(seq uint64) []byte { return fleetStateFrame(seq, snap) })
}

// nextFleetStateSeq assigns the next fleet_state sequence from the SEPARATE fleet_state
// domain (A2) — never s.outbox.reserveTransient(), so fleet activity never perturbs the
// shared operator outbox cursor / durable frame sequencing. Caller holds fsSeqMu.
func (s *Server) nextFleetStateSeqLocked() uint64 {
	s.fsSeq++
	return s.fsSeq
}

// emitFleetState publishes a fleet_state frame to every active FLEET-STATE-CAPABLE
// device (A1). It reserves a seq from the fleet_state domain (A2) and holds fsSeqMu
// across assign+publish so a concurrent snapshot cannot interleave a stale seq. It
// takes NEITHER s.deliveryMu NOR s.outbox — the operator durable path is untouched.
func (s *Server) emitFleetState(build func(seq uint64) []byte) {
	s.fsSeqMu.Lock()
	defer s.fsSeqMu.Unlock()
	var data []byte
	for _, d := range s.store.ActiveDevices() {
		if !s.deviceWantsFleetState(d.ID) {
			continue
		}
		if data == nil {
			data = build(s.nextFleetStateSeqLocked())
		}
		s.mailbox.publishTransient(d.ID, data)
	}
}

// emitFleetStateTo publishes a fleet_state frame to a single fleet-state-capable device
// (A1/A2), same separate-seq-domain discipline as emitFleetState.
func (s *Server) emitFleetStateTo(deviceID string, build func(seq uint64) []byte) {
	s.fsSeqMu.Lock()
	defer s.fsSeqMu.Unlock()
	s.mailbox.publishTransient(deviceID, build(s.nextFleetStateSeqLocked()))
}

// deviceWantsFleetState reports whether a live device advertised fleet-state support in
// its hello (A1 capability gate).
func (s *Server) deviceWantsFleetState(deviceID string) bool {
	s.fleetStateCapMu.Lock()
	defer s.fleetStateCapMu.Unlock()
	return s.fleetStateCapRefs[deviceID] > 0
}

// markFleetStateCap records that a live session advertised fleet-state support, and
// returns a release func for the session's defer (refcounted so overlapping reconnects
// of the same device are safe).
func (s *Server) markFleetStateCap(deviceID string) func() {
	s.fleetStateCapMu.Lock()
	s.fleetStateCapRefs[deviceID]++
	s.fleetStateCapMu.Unlock()
	return func() {
		s.fleetStateCapMu.Lock()
		if s.fleetStateCapRefs[deviceID] > 0 {
			s.fleetStateCapRefs[deviceID]--
			if s.fleetStateCapRefs[deviceID] == 0 {
				delete(s.fleetStateCapRefs, deviceID)
			}
		}
		s.fleetStateCapMu.Unlock()
	}
}

// fleetStateCapToken is the hello capability string a device advertises to receive
// fleet_state transients (A1). No shipped client advertises it today.
const fleetStateCapToken = "fleet_state"

// helloAdvertisesFleetState reports whether a device hello opted in to fleet_state
// (A1): membership of fleetStateCapToken in its bounded caps list.
func helloAdvertisesFleetState(caps []string) bool {
	for _, c := range caps {
		if c == fleetStateCapToken {
			return true
		}
	}
	return false
}

// stopFleetStateEmitter marks the emitter closed and stops any pending trailing
// send, so a stopped server never emits again (mirrors stopAgentStateEmitter).
func (s *Server) stopFleetStateEmitter() {
	s.fsMu.Lock()
	defer s.fsMu.Unlock()
	s.fsClosed = true
	if s.fsTimer != nil {
		s.fsTimer.Stop()
		s.fsTimer = nil
	}
}
