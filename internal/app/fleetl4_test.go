package app

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/mcpchan"
)

// Lane L4 (observability polish, a2a-design-v2 §6): per-edge rolling 24h counters,
// journal rotation, the fleet_state snapshot, and the enriched fleet ls columns.

// TestFleetActivityCountersRolling proves an inbound commit bumps recv_24h and an
// outbound enqueue bumps sent_24h, persisted per edge, and that a bucket older than
// the 24h window is excluded from the totals.
func TestFleetActivityCountersRolling(t *testing.T) {
	s := newFleetServer(t)
	edge, hello := serveEdge(t, s, "peer")

	// One inbound fleet_msg → recv_24h == 1.
	runFleetSession(t, s, edge, hello, fmsgRaw("cid-l4recv0001", "hello", "brief"))
	sent, recv, dropped, err := s.fleetStore.EdgeActivity(edge.EdgeID)
	if err != nil {
		t.Fatalf("edge activity: %v", err)
	}
	if sent != 0 || recv != 1 || dropped != 0 {
		t.Fatalf("after 1 inbound: sent=%d recv=%d dropped=%d, want 0/1/0", sent, recv, dropped)
	}

	// Two outbound enqueues → sent_24h == 2.
	for i := 0; i < 2; i++ {
		frame, _ := marshalFleetMsg(newFleetCID(), fleetNow(), "out", "brief", fleetFrom{Box: "me"})
		if _, _, err := s.fleetStore.EnqueueOutboundTx(edge.EdgeID, newFleetCID(), frame); err != nil {
			t.Fatalf("enqueue %d: %v", i, err)
		}
	}
	sent, recv, _, _ = s.fleetStore.EdgeActivity(edge.EdgeID)
	if sent != 2 || recv != 1 {
		t.Fatalf("after 2 outbound: sent=%d recv=%d, want 2/1", sent, recv)
	}

	// A bucket 25h old is excluded from the trailing-24h totals AND pruned on bump.
	st, err := s.fleetStore.EdgeState(edge.EdgeID)
	if err != nil {
		t.Fatalf("edge state: %v", err)
	}
	oldHour := time.Now().Add(-25*time.Hour).Unix() / 3600
	st.Activity = append(st.Activity, fleetActivityBucket{Hour: oldHour, Sent: 99, Recv: 99})
	if gotSent, gotRecv, _ := st.windowCounts(time.Now()); gotSent != 2 || gotRecv != 1 {
		t.Fatalf("25h-old bucket counted in window: sent=%d recv=%d, want 2/1", gotSent, gotRecv)
	}
	st.bumpActivity(time.Now(), 0, 0, 0)
	for _, b := range st.Activity {
		if b.Hour == oldHour {
			t.Fatalf("25h-old bucket survived pruning")
		}
	}
}

// TestFleetJournalRotation proves the journal rotates to .1 at the threshold, keeps
// exactly one prior generation, continues the monotonic seq across the rotation, and
// that the two-generation readers (scan / pending / replay) see the full history so
// no dedup/WAL state is lost across a single rotation.
func TestFleetJournalRotation(t *testing.T) {
	prev := fleetJournalRotateBytesOverride
	fleetJournalRotateBytesOverride = 512 // tiny threshold for a fast, hermetic test
	defer func() { fleetJournalRotateBytesOverride = prev }()

	s := newFleetServer(t)
	edge, _ := serveEdge(t, s, "peer")
	edgeDir := filepath.Join(s.cfg.StateDir, fleetDirName, edge.EdgeID)
	live := filepath.Join(edgeDir, fleetJournalFile)
	backup := live + ".1"

	// Append dir=out frames until the FIRST rotation fires (backup .1 appears), then
	// stop — so exactly one rotation has happened and the two-generation reader must
	// still see the whole history (the "keep one generation" no-loss guarantee).
	var lastSeq uint64
	appended := 0
	for i := 0; i < 500; i++ {
		frame, _ := marshalFleetMsg("cid-rot", fleetNow(), strings.Repeat("x", 40), "brief", fleetFrom{Box: "b"})
		seq, err := s.fleetStore.AppendJournalFrame(edge.EdgeID, fleetDirOut, frame)
		if err != nil {
			t.Fatalf("append %d: %v", i, err)
		}
		if seq != lastSeq+1 {
			t.Fatalf("seq not monotonic at %d: got %d after %d", i, seq, lastSeq)
		}
		lastSeq = seq
		appended++
		if _, err := os.Stat(backup); err == nil {
			break // first rotation fired
		}
	}
	if _, err := os.Stat(backup); err != nil {
		t.Fatalf("journal.jsonl.1 not created (rotation did not fire): %v", err)
	}
	if _, err := os.Stat(live); err != nil {
		t.Fatalf("live journal missing after rotation: %v", err)
	}

	// After a SINGLE rotation, the two-generation read sees ALL appended entries with
	// a continuous seq — no dedup/WAL history lost.
	entries, err := s.fleetStore.JournalEntries(edge.EdgeID)
	if err != nil {
		t.Fatalf("journal entries: %v", err)
	}
	if len(entries) != appended {
		t.Fatalf("two-generation read saw %d entries, want %d (single rotation must lose none)", len(entries), appended)
	}
	for i, e := range entries {
		if e.Seq != uint64(i+1) {
			t.Fatalf("entry %d has seq %d, want %d (history hole across rotation)", i, e.Seq, i+1)
		}
	}

	// The outbound WAL derivation also spans both generations: every appended dir=out
	// frame is unacked, so PendingOutbound (newest-cap) must reflect the full set.
	pending, err := s.fleetStore.PendingOutbound(edge.EdgeID)
	if err != nil {
		t.Fatalf("pending: %v", err)
	}
	wantPending := appended
	if wantPending > fleetPendingCap {
		wantPending = fleetPendingCap
	}
	if len(pending) != wantPending {
		t.Fatalf("pending outbound spans one generation only: got %d, want %d", len(pending), wantPending)
	}

	// A fresh append after rotation still continues the monotonic sequence (proves
	// lastJournalSeqLocked spans both generations).
	frame, _ := marshalFleetMsg("cid-after", fleetNow(), "y", "brief", fleetFrom{Box: "b"})
	seq, err := s.fleetStore.AppendJournalFrame(edge.EdgeID, fleetDirOut, frame)
	if err != nil {
		t.Fatalf("post-rotation append: %v", err)
	}
	if seq != lastSeq+1 {
		t.Fatalf("post-rotation seq=%d, want %d", seq, lastSeq+1)
	}
}

// TestFleetJournalSecondRotationPreservesWAL is the L4-sol BLOCKER-2 proof: a SECOND
// rotation must NOT destroy the first archived generation. It writes an old unacked
// outbound frame and an old inbound CID (the first two journal entries → the first
// generation → the first .1), then drives the journal across the rotation threshold
// many times. Under the old unconditional os.Rename(journal, journal+".1") the second
// rotation clobbered the .1 holding those old entries, so the old outbound frame
// vanished from PendingOutbound and the old inbound CID stopped deduping (re-commit +
// reinjection). The one-archived-generation guard freezes the first .1, so both
// survive. This test FAILS against the overwrite and PASSES with the guard.
func TestFleetJournalSecondRotationPreservesWAL(t *testing.T) {
	prev := fleetJournalRotateBytesOverride
	fleetJournalRotateBytesOverride = 512
	defer func() { fleetJournalRotateBytesOverride = prev }()

	s := newFleetServer(t)
	edge, _ := serveEdge(t, s, "peer")

	// The two OLD entries — the journal head, so they land in the first .1.
	const oldInboundCID = "cid-oldinbound01"
	const oldOutboundCID = "cid-oldoutbound1"
	inFrame, _ := marshalFleetMsg(oldInboundCID, fleetNow(), "old inbound", "brief", fleetFrom{Box: "peer"})
	if c, err := s.fleetStore.CommitInbound(edge.EdgeID, oldInboundCID, inFrame, nil); err != nil || !c.Committed {
		t.Fatalf("commit old inbound: committed=%v err=%v", c.Committed, err)
	}
	outFrame, _ := marshalFleetMsg(oldOutboundCID, fleetNow(), "old outbound", "brief", fleetFrom{Box: "me"})
	if _, err := s.fleetStore.AppendJournalFrame(edge.EdgeID, fleetDirOut, outFrame); err != nil {
		t.Fatalf("append old outbound: %v", err)
	}

	// Drive many threshold crossings with INBOUND filler. dir=in keeps the single old
	// outbound frame at the head of the unacked queue (it is never crowded out of
	// PendingOutbound's newest-cap), while forcing repeated rotations. >= two full
	// generations' worth guarantees the overwrite bug would have clobbered the first .1.
	backup := filepath.Join(s.cfg.StateDir, fleetDirName, edge.EdgeID, fleetJournalFile+".1")
	for i := 0; i < 200; i++ {
		f, _ := marshalFleetMsg(fmt.Sprintf("cid-fill%08d", i), fleetNow(), strings.Repeat("x", 60), "brief", fleetFrom{Box: "peer"})
		if _, err := s.fleetStore.AppendJournalFrame(edge.EdgeID, fleetDirIn, f); err != nil {
			t.Fatalf("filler append %d: %v", i, err)
		}
	}
	if _, err := os.Stat(backup); err != nil {
		t.Fatalf("no rotation happened at all: %v", err)
	}

	// The OLD unacked outbound frame is still recoverable via the WAL derivation.
	pending, err := s.fleetStore.PendingOutbound(edge.EdgeID)
	if err != nil {
		t.Fatalf("pending: %v", err)
	}
	foundOut := false
	for _, p := range pending {
		if p.CID == oldOutboundCID {
			foundOut = true
		}
	}
	if !foundOut {
		t.Fatalf("old unacked outbound frame lost across a second rotation (PendingOutbound=%d entries, none is %s)", len(pending), oldOutboundCID)
	}

	// The OLD inbound CID is still deduped: a re-commit must be a Duplicate no-op, not a
	// fresh commit that would reinject the peer message.
	seen, err := s.fleetStore.InboundSeen(edge.EdgeID, oldInboundCID)
	if err != nil {
		t.Fatalf("inbound seen: %v", err)
	}
	if !seen {
		t.Fatalf("old inbound CID lost across a second rotation → would re-commit + reinject")
	}
	re, err := s.fleetStore.CommitInbound(edge.EdgeID, oldInboundCID, inFrame, nil)
	if err != nil {
		t.Fatalf("re-commit old inbound: %v", err)
	}
	if re.Committed || !re.Duplicate {
		t.Fatalf("old inbound CID re-committed (committed=%v duplicate=%v) — WAL dedup history was destroyed", re.Committed, re.Duplicate)
	}
}

// TestFleetStateSnapshotCountsOnly proves buildFleetState reports per-edge counts,
// connected state, and flags — and NEVER carries fleet message content (the payload
// text stays out of the operator-facing snapshot).
func TestFleetStateSnapshotCountsOnly(t *testing.T) {
	s := newFleetServer(t)
	s.bindFleetSink(newFakeSink())
	edge, hello := serveEdge(t, s, "peerX")

	const secret = "top secret peer payload do not leak"
	runFleetSession(t, s, edge, hello, fmsgRaw("cid-l4snap001", secret, "brief"))

	snap, ok := s.buildFleetState()
	if !ok {
		t.Fatalf("buildFleetState not ok")
	}
	if len(snap.Edges) != 1 {
		t.Fatalf("snapshot edges=%d, want 1", len(snap.Edges))
	}
	row := snap.Edges[0]
	if row.EdgeID != edge.EdgeID || row.Alias != "peerX" {
		t.Fatalf("row identity wrong: %+v", row)
	}
	if row.Recv24h != 1 {
		t.Fatalf("row recv_24h=%d, want 1", row.Recv24h)
	}
	if row.Connected {
		t.Fatalf("no live session was registered; connected must be false")
	}
	// Content isolation: the payload text must never appear anywhere in the frame.
	body, _ := json.Marshal(fleetStateFrame(1, snap))
	if strings.Contains(string(body), secret) {
		t.Fatalf("fleet_state frame leaked peer message content")
	}
	if strings.Contains(string(body), "top secret") {
		t.Fatalf("fleet_state frame leaked peer message content (partial)")
	}

	// A removed edge is marked dead (with its tombstone reason), not counted-live.
	if _, err := s.fleetStore.Remove(edge.EdgeID); err != nil {
		t.Fatalf("remove: %v", err)
	}
	snap, _ = s.buildFleetState()
	if len(snap.Edges) != 1 || !snap.Edges[0].Dead || snap.Edges[0].DeadReason != "removed" {
		t.Fatalf("removed edge not marked dead: %+v", snap.Edges)
	}
	if snap.Edges[0].Connected {
		t.Fatalf("removed edge must not be connected")
	}
}

// TestFleetStateFrameShape locks the wire shape: t=fleet_state, a seq, and a state
// object whose edges is always an array (never null).
func TestFleetStateFrameShape(t *testing.T) {
	frame := fleetStateFrame(7, fleetStateSnapshot{Edges: []fleetStateEdge{}})
	var m map[string]any
	if err := json.Unmarshal(frame, &m); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if m["t"] != "fleet_state" {
		t.Fatalf("t=%v, want fleet_state", m["t"])
	}
	if m["seq"].(float64) != 7 {
		t.Fatalf("seq=%v, want 7", m["seq"])
	}
	state, ok := m["state"].(map[string]any)
	if !ok {
		t.Fatalf("state not an object: %v", m["state"])
	}
	if _, ok := state["edges"].([]any); !ok {
		t.Fatalf("state.edges must be an array, got %T", state["edges"])
	}
}

// TestFleetStateThrottleDedup proves the throttled broadcaster suppresses an
// unchanged snapshot and never emits an empty fleet before anything was shown.
func TestFleetStateThrottleDedup(t *testing.T) {
	s := newFleetServer(t)
	s.fsThrottle = time.Hour // block any trailing send during the test

	// No edges yet: an immediate broadcast must announce nothing (empty-first guard).
	s.fsMu.Lock()
	s.fsLastSent = time.Time{} // force the wait<=0 immediate path
	s.broadcastFleetStateLocked()
	firstSnap := append([]byte(nil), s.fsLastSnap...)
	s.fsMu.Unlock()
	if firstSnap != nil {
		t.Fatalf("empty fleet was announced before any edge existed: %s", firstSnap)
	}

	// Add an edge, broadcast → a snapshot is cached; a second immediate broadcast
	// with no change is a dedup no-op (cache unchanged).
	edge, _ := serveEdge(t, s, "peerT")
	_ = edge
	s.fsMu.Lock()
	s.broadcastFleetStateLocked()
	afterFirst := append([]byte(nil), s.fsLastSnap...)
	s.broadcastFleetStateLocked()
	afterSecond := append([]byte(nil), s.fsLastSnap...)
	s.fsMu.Unlock()
	if afterFirst == nil {
		t.Fatalf("a non-empty fleet was not announced")
	}
	if string(afterFirst) != string(afterSecond) {
		t.Fatalf("dedup failed: snapshot changed with no state change")
	}
}

// TestFleetSendBumpsSentCounter proves the fleet_send tool path bumps sent_24h (the
// end-to-end counter wiring, not just the raw store call).
func TestFleetSendBumpsSentCounter(t *testing.T) {
	s := newFleetServer(t)
	fp := newFleetProviderFor(s)
	edge, _, err := s.fleetStore.Link(fleetTestRelay, "peerS")
	if err != nil {
		t.Fatalf("link: %v", err)
	}
	msg, isErr := fp.FleetSend(context.Background(), mcpchan.FleetSendInput{To: "peerS", Text: "hi peer"})
	if isErr {
		t.Fatalf("fleet_send: %s", msg)
	}
	sent, _, _, err := s.fleetStore.EdgeActivity(edge.EdgeID)
	if err != nil {
		t.Fatalf("activity: %v", err)
	}
	if sent != 1 {
		t.Fatalf("sent_24h=%d after one fleet_send, want 1", sent)
	}
}
