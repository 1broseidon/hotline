package app

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/mcpchan"
)

// These tests are milestone F2's contract, taken straight from the 2026-07-24 field
// finding: edge death must be RECOVERABLE and one-sided failure must be AUDIBLE.

// TestFleetUnreachableIsRecoverable proves the cold-retry dead-mark is nothing like a
// tombstone: creds survive, the edge stays live and dialable, and a successful
// handshake clears it — no operator re-pair.
func TestFleetUnreachableIsRecoverable(t *testing.T) {
	s := newFleetServer(t)
	edge := dialEdge(t, s, "boxa")

	if err := s.fleetStore.MarkEdgeUnreachable(edge.EdgeID, fleetDialDeadAfter); err != nil {
		t.Fatalf("mark unreachable: %v", err)
	}
	live, ok := s.fleetStore.LiveEdge(edge.EdgeID)
	if !ok {
		t.Fatal("an unreachable edge must still be live (it is NOT a tombstone)")
	}
	if live.Removed() {
		t.Fatal("unreachable must not tombstone the edge")
	}
	if live.Unreachable == nil || live.Unreachable.Attempts != fleetDialDeadAfter || live.Unreachable.Since == "" {
		t.Fatalf("unreachable flag not recorded: %+v", live.Unreachable)
	}
	// The creds are the whole point: without them there is no revive path at all.
	if live.DeviceCreds == nil || live.DeviceCreds.Secret == "" {
		t.Fatal("unreachable zeroed the dial creds — the edge could never come back")
	}
	// The dial manager still drives it, so it keeps retrying on the cold tier.
	dials, err := s.fleetStore.DialEdges()
	if err != nil || len(dials) != 1 {
		t.Fatalf("an unreachable edge must still be dialed: %v %d", err, len(dials))
	}
	// It also still accepts and journals inbound (the peer may be fine and dialing us).
	if _, ok := s.liveEdge(edge.EdgeID); !ok {
		t.Fatal("an unreachable edge must still accept frames")
	}

	// Revive.
	s.reviveUnreachable(edge.EdgeID)
	live, _ = s.fleetStore.LiveEdge(edge.EdgeID)
	if live.Unreachable != nil {
		t.Fatalf("a successful handshake must clear the flag: %+v", live.Unreachable)
	}
	// Reviving a healthy edge is a silent no-op.
	cleared, err := s.fleetStore.ClearEdgeUnreachable(edge.EdgeID)
	if err != nil || cleared {
		t.Fatalf("clearing an unflagged edge should report nothing: cleared=%v err=%v", cleared, err)
	}
}

// TestFleetUnreachableDistinctFromRemovedAndRevoked pins the categorical separation
// the design asked for: only an operator rm or a peer revoke is terminal.
func TestFleetUnreachableDistinctFromRemovedAndRevoked(t *testing.T) {
	s := newFleetServer(t)
	cold := dialEdge(t, s, "cold")
	revoked := dialEdge(t, s, "revoked")

	if err := s.fleetStore.MarkEdgeUnreachable(cold.EdgeID, 12); err != nil {
		t.Fatalf("mark unreachable: %v", err)
	}
	if err := s.fleetStore.MarkDialEdgeDead(revoked.EdgeID, "revoked"); err != nil {
		t.Fatalf("mark dead: %v", err)
	}

	edges, err := s.fleetStore.Edges()
	if err != nil {
		t.Fatalf("edges: %v", err)
	}
	for _, e := range edges {
		switch e.EdgeID {
		case cold.EdgeID:
			if e.Tombstone != nil {
				t.Fatalf("unreachable produced a tombstone: %+v", e.Tombstone)
			}
		case revoked.EdgeID:
			if e.Tombstone == nil || e.Tombstone.Reason != "revoked" {
				t.Fatalf("revoked must stay a terminal tombstone: %+v", e.Tombstone)
			}
			if e.Unreachable != nil {
				t.Fatalf("a terminal tombstone should not also carry the cold flag: %+v", e.Unreachable)
			}
			if e.DeviceCreds != nil && e.DeviceCreds.Secret != "" {
				t.Fatal("a revoked edge must still have its creds zeroed")
			}
		}
	}
	// A removed edge is not re-flagged by a late dialer mark.
	if _, err := s.fleetStore.Remove(cold.EdgeID); err != nil {
		t.Fatalf("remove: %v", err)
	}
	if err := s.fleetStore.MarkEdgeUnreachable(cold.EdgeID, 20); err != nil {
		t.Fatalf("mark unreachable on tombstone: %v", err)
	}
	if _, ok := s.fleetStore.LiveEdge(cold.EdgeID); ok {
		t.Fatal("a removed edge came back live")
	}
}

// TestFleetStalePendingPredicate covers the liveness rule itself, including the
// never-seen edge and the not-yet-stale window.
func TestFleetStalePendingPredicate(t *testing.T) {
	now := time.Now()
	old := now.Add(-fleetStalePendingAfter - time.Minute).Format(time.RFC3339Nano)
	recent := now.Add(-time.Minute).Format(time.RFC3339Nano)

	cases := []struct {
		name      string
		pending   int
		connected bool
		lastSeen  string
		want      bool
	}{
		{"queued and long gone", 2, false, old, true},
		{"queued and never seen", 1, false, "", true},
		{"queued but connected", 5, true, old, false},
		{"queued and seen recently", 5, false, recent, false},
		{"nothing queued", 0, false, old, false},
		{"unparsable last_seen", 1, false, "not-a-time", true},
	}
	for _, tc := range cases {
		if got := FleetStalePending(tc.pending, tc.connected, tc.lastSeen, now); got != tc.want {
			t.Fatalf("%s: FleetStalePending = %v, want %v", tc.name, got, tc.want)
		}
	}
}

// TestFleetLivenessSweepAlarmsOnStalePending is the finding's exact shape: frames
// queued for a peer that has gone quiet must surface in fleet.log — once, then
// throttled — and stop alarming when the edge recovers.
func TestFleetLivenessSweepAlarmsOnStalePending(t *testing.T) {
	s := newFleetServer(t)
	if _, _, err := s.store.SeedIdentityName("HubBox"); err != nil {
		t.Fatalf("seed identity: %v", err)
	}
	edge, _ := serveEdge(t, s, "boxb")
	fp := newFleetProviderFor(s)

	// Two sends queue durably (no session is attached), exactly like the field case.
	for _, text := range []string{"first", "second"} {
		if msg, isErr := fp.FleetSend(context.Background(), mcpchan.FleetSendInput{To: edge.Alias, Text: text}); isErr {
			t.Fatalf("fleet_send: %s", msg)
		}
	}
	if depth, err := s.fleetStore.PendingDepth(edge.EdgeID); err != nil || depth != 2 {
		t.Fatalf("expected pending=2, got %d (%v)", depth, err)
	}

	warns := &fleetStaleWarns{}
	// Fresh edge, nothing seen yet but frames queued → stale immediately.
	if alarmed := s.sweepFleetLivenessOnce(warns, time.Now()); len(alarmed) != 1 || alarmed[0] != edge.EdgeID {
		t.Fatalf("sweep did not alarm on a never-seen edge with pending frames: %v", alarmed)
	}
	logPath := filepath.Join(s.cfg.StateDir, fleetLogFile)
	data, err := os.ReadFile(logPath)
	if err != nil {
		t.Fatalf("read fleet.log: %v", err)
	}
	if !strings.Contains(string(data), "STALE PENDING") || !strings.Contains(string(data), "2 outbound frame(s)") {
		t.Fatalf("fleet.log missing the stale-pending alarm:\n%s", data)
	}
	before := len(strings.Split(string(data), "STALE PENDING"))

	// Throttled: a second sweep a minute later still detects it but writes no new line.
	s.sweepFleetLivenessOnce(warns, time.Now().Add(time.Minute))
	data, _ = os.ReadFile(logPath)
	if got := len(strings.Split(string(data), "STALE PENDING")); got != before {
		t.Fatalf("the alarm is not throttled: %d segments, want %d", got, before)
	}
	// Past the repeat window it speaks again — a stuck edge stays visible.
	s.sweepFleetLivenessOnce(warns, time.Now().Add(fleetStaleRepeatEvery+time.Minute))
	data, _ = os.ReadFile(logPath)
	if got := len(strings.Split(string(data), "STALE PENDING")); got <= before {
		t.Fatalf("the alarm never repeated: %d segments, want > %d", got, before)
	}

	// Recovery: contact resumes → no alarm.
	s.fleetStore.TouchLastSeen(edge.EdgeID)
	if alarmed := s.sweepFleetLivenessOnce(warns, time.Now()); len(alarmed) != 0 {
		t.Fatalf("a freshly-seen edge still alarmed: %v", alarmed)
	}
}

// TestFleetStalePendingSurfacedToOperator proves the signal is not log-only: the fleet
// tool and the operator fleet_state snapshot both carry it, alongside the recoverable
// unreachable flag.
func TestFleetStalePendingSurfacedToOperator(t *testing.T) {
	s := newFleetServer(t)
	if _, _, err := s.store.SeedIdentityName("HubBox"); err != nil {
		t.Fatalf("seed identity: %v", err)
	}
	edge, _ := serveEdge(t, s, "boxb")
	fp := newFleetProviderFor(s)
	if msg, isErr := fp.FleetSend(context.Background(), mcpchan.FleetSendInput{To: edge.Alias, Text: "into the void"}); isErr {
		t.Fatalf("fleet_send: %s", msg)
	}
	if err := s.fleetStore.MarkEdgeUnreachable(edge.EdgeID, fleetDialDeadAfter); err != nil {
		t.Fatalf("mark unreachable: %v", err)
	}

	out, isErr := fp.FleetList(context.Background())
	if isErr {
		t.Fatalf("fleet list: %s", out)
	}
	if !strings.Contains(out, `"stale_pending": true`) || !strings.Contains(out, `"unreachable": true`) {
		t.Fatalf("fleet list hides the F2 liveness signals:\n%s", out)
	}

	snap, ok := s.buildFleetState()
	if !ok || len(snap.Edges) != 1 {
		t.Fatalf("fleet_state snapshot: ok=%v edges=%d", ok, len(snap.Edges))
	}
	row := snap.Edges[0]
	if !row.StalePending || !row.Unreachable {
		t.Fatalf("fleet_state row missing the liveness signals: %+v", row)
	}
	if row.Dead || row.DeadReason != "" {
		t.Fatalf("unreachable must NOT read as dead in the operator snapshot: %+v", row)
	}
}
