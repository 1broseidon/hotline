package app

import (
	"context"
	"encoding/json"
	"fmt"
	"testing"
)

// ---- helpers -------------------------------------------------------------

// dialEdge stages a direction=dial edge on s by minting a room/secret on a
// throwaway serve store and joining its fleet URI.
func dialEdge(t *testing.T, s *Server, alias string) FleetEdge {
	t.Helper()
	src := newFleetServer(t)
	serve, secret, err := src.fleetStore.Link(fleetTestRelay, "peer")
	if err != nil {
		t.Fatalf("link: %v", err)
	}
	uri := FleetPairingURI(serve.RelayURL, serve.Room, secret, "peer")
	origin, err := rendezvousOrigin(serve.RelayURL)
	if err != nil {
		t.Fatalf("origin: %v", err)
	}
	edge, err := s.fleetStore.Join(uri, alias, JoinOptions{AllowedOrigins: []string{origin}})
	if err != nil {
		t.Fatalf("join: %v", err)
	}
	return edge
}

func dialFmsgRaw(cid, text, kind string, from *fleetFrom) []byte {
	m := map[string]any{"t": "fleet_msg", "cid": cid, "ts": "2026-07-23T00:00:00Z", "text": text, "kind": kind}
	if from != nil {
		m["from"] = from
	}
	b, _ := json.Marshal(m)
	return b
}

func collectWrites() (func([]byte) error, *[][]byte) {
	var frames [][]byte
	w := func(b []byte) error {
		frames = append(frames, append([]byte(nil), b...))
		return nil
	}
	return w, &frames
}

func countType(frames [][]byte, typ string) int {
	n := 0
	for _, f := range frames {
		var m map[string]any
		if json.Unmarshal(f, &m) == nil && m["t"] == typ {
			n++
		}
	}
	return n
}

// ---- device envelope codec (both roles + tamper) -------------------------

func TestFleetDeviceCodecRoundTripBothRoles(t *testing.T) {
	secret, err := randomBase64URL(32)
	if err != nil {
		t.Fatal(err)
	}
	room, _ := randomBase64URL(16)
	box, err := newEnvelopeCodec(RoomRecord{ID: room, Secret: secret, Envelope: true})
	if err != nil {
		t.Fatalf("box codec: %v", err)
	}
	dev, err := newDeviceEnvelopeCodec(room, secret)
	if err != nil {
		t.Fatalf("device codec: %v", err)
	}

	sealed, err := dev.wrap([]byte(`{"t":"hello"}`))
	if err != nil {
		t.Fatalf("device wrap: %v", err)
	}
	got, err := box.unwrap(sealed)
	if err != nil || string(got) != `{"t":"hello"}` {
		t.Fatalf("box.unwrap(device.wrap) = %q, %v", got, err)
	}
	sealed2, err := box.wrap([]byte(`{"t":"welcome_fleet"}`))
	if err != nil {
		t.Fatalf("box wrap: %v", err)
	}
	got2, err := dev.unwrap(sealed2)
	if err != nil || string(got2) != `{"t":"welcome_fleet"}` {
		t.Fatalf("device.unwrap(box.wrap) = %q, %v", got2, err)
	}
	if _, err := dev.unwrap(sealed); err == nil {
		t.Fatalf("device opened its own app→box frame; must fail (wrong direction)")
	}
	var f map[string]string
	_ = json.Unmarshal(sealed2, &f)
	ct := []byte(f["c"])
	ct[len(ct)-1] ^= 0x01
	f["c"] = string(ct)
	tampered, _ := json.Marshal(f)
	if _, err := dev.unwrap(tampered); err == nil {
		t.Fatalf("device.unwrap accepted a tampered frame")
	}
}

func TestFleetDeviceCodecRejectsNonE1(t *testing.T) {
	secret, _ := randomBase64URL(32)
	room, _ := randomBase64URL(16)
	dev, err := newDeviceEnvelopeCodec(room, secret)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := dev.unwrap([]byte(`{"t":"welcome_fleet"}`)); err == nil {
		t.Fatalf("device.unwrap accepted a non-e1 (plaintext) frame")
	}
	if _, err := newDeviceEnvelopeCodec(room, ""); err == nil {
		t.Fatalf("newDeviceEnvelopeCodec accepted an empty secret")
	}
}

// ---- commit-before-ack (single durable tx) -------------------------------

func TestFleetInboundCommitBeforeAck(t *testing.T) {
	s := newFleetServer(t)
	s.bindFleetSink(newFakeSink())
	edge := dialEdge(t, s, "peer")
	write, frames := collectWrites()

	if act := s.deliverInbound(context.Background(), edge, edge.Alias, dialFmsgRaw("cid-aaaa00000001", "hello", "brief", nil), write, false); act != inKeepSession {
		t.Fatalf("unexpected action: %v", act)
	}
	entries, _ := s.fleetStore.JournalEntries(edge.EdgeID)
	if len(entries) != 1 || entries[0].Dir != "in" {
		t.Fatalf("expected 1 inbound journal entry, got %+v", entries)
	}
	st, _ := s.fleetStore.EdgeState(edge.EdgeID)
	if st.Cursor != "1" {
		t.Fatalf("cursor not advanced with the commit: %q", st.Cursor)
	}
	if countType(*frames, "fleet_ack") != 1 {
		t.Fatalf("expected exactly one ack after commit")
	}
}

// An uncommitted frame (dropped by the text cap) is NOT acked and NOT journaled.
func TestFleetInboundNoAckWithoutCommit(t *testing.T) {
	s := newFleetServer(t)
	edge := dialEdge(t, s, "peer")
	write, frames := collectWrites()
	big := make([]byte, fleetTextCap+1)
	for i := range big {
		big[i] = 'x'
	}
	s.deliverInbound(context.Background(), edge, edge.Alias, dialFmsgRaw("cid-bbbb00000001", string(big), "brief", nil), write, false)
	if entries, _ := s.fleetStore.JournalEntries(edge.EdgeID); len(entries) != 0 {
		t.Fatalf("oversize frame journaled: %+v", entries)
	}
	if countType(*frames, "fleet_ack") != 0 {
		t.Fatalf("oversize (uncommitted) frame was acked")
	}
}

// ---- durable redelivery dedup (journal is the authority, no ring) --------

func TestFleetInboundDurableDedup(t *testing.T) {
	s := newFleetServer(t)
	sink := newFakeSink()
	s.bindFleetSink(sink)
	edge := dialEdge(t, s, "peer")
	write, frames := collectWrites()
	raw := dialFmsgRaw("cid-cccc00000001", "once", "brief", nil)

	s.deliverInbound(context.Background(), edge, edge.Alias, raw, write, false)
	s.deliverInbound(context.Background(), edge, edge.Alias, raw, write, false) // redelivered

	if entries, _ := s.fleetStore.JournalEntries(edge.EdgeID); len(entries) != 1 {
		t.Fatalf("redelivered cid re-journaled: %d entries", len(entries))
	}
	if countType(*frames, "fleet_ack") != 2 {
		t.Fatalf("expected 2 acks (original + re-ack), got %d", countType(*frames, "fleet_ack"))
	}
	if len(sink.ch) != 1 {
		t.Fatalf("expected exactly one injection, got %d", len(sink.ch))
	}
}

// Dedup is durable over the COMPLETE history: an early cid is still deduped after
// far more inbound than any ring bound would retain.
func TestFleetInboundDedupIsDurableNotRingBounded(t *testing.T) {
	s := newFleetServer(t)
	// No sink bound: this exercises the durable JOURNAL dedup (InboundSeen +
	// re-journal count), not agent injection — so injectInbound no-ops.
	edge := dialEdge(t, s, "peer")
	write, _ := collectWrites()
	early := "cid-early0000001"
	s.deliverInbound(context.Background(), edge, edge.Alias, dialFmsgRaw(early, "x", "brief", nil), write, false)
	// Far more inbound than any bounded ring would retain — the durable journal is
	// the dedup authority, so history never ages out (review B1).
	for i := 0; i < 300; i++ {
		s.deliverInbound(context.Background(), edge, edge.Alias, dialFmsgRaw(fmt.Sprintf("cid-%012d", i), "x", "brief", nil), write, false)
	}
	if seen, _ := s.fleetStore.InboundSeen(edge.EdgeID, early); !seen {
		t.Fatalf("early cid evicted from durable dedup — ring-only regression")
	}
	countIn := func() int {
		entries, _ := s.fleetStore.JournalEntries(edge.EdgeID)
		n := 0
		for _, e := range entries {
			if e.Dir == "in" {
				n++
			}
		}
		return n
	}
	before := countIn()
	s.deliverInbound(context.Background(), edge, edge.Alias, dialFmsgRaw(early, "x", "brief", nil), write, false)
	if after := countIn(); after != before {
		t.Fatalf("early cid re-journaled: %d != %d", after, before)
	}
}

// ---- reconcile: divergence in EITHER direction --------------------------

func TestReconcileCursorBothDirections(t *testing.T) {
	s := newFleetServer(t)
	edge := dialEdge(t, s, "peer")
	for _, cid := range []string{"cid-dddd00000001", "cid-dddd00000002"} {
		frame, _ := json.Marshal(map[string]any{"t": "fleet_msg", "cid": cid, "text": "m"})
		if _, err := s.fleetStore.AppendJournalFrame(edge.EdgeID, "in", frame); err != nil {
			t.Fatal(err)
		}
	}
	set := func(c string) {
		if err := s.fleetStore.mutateEdgeState(edge.EdgeID, func(st *fleetEdgeState) error { st.Cursor = c; return nil }); err != nil {
			t.Fatal(err)
		}
	}
	set("99") // ahead
	gen, resynced, _ := s.fleetStore.ReconcileInboundCursor(edge.EdgeID)
	if !resynced || gen != 1 {
		t.Fatalf("cursor-ahead: resynced=%v gen=%d", resynced, gen)
	}
	if st, _ := s.fleetStore.EdgeState(edge.EdgeID); st.Cursor != "2" {
		t.Fatalf("cursor not realigned to tail: %q", st.Cursor)
	}
	set("0") // behind
	gen, resynced, _ = s.fleetStore.ReconcileInboundCursor(edge.EdgeID)
	if !resynced || gen != 2 {
		t.Fatalf("cursor-behind: resynced=%v gen=%d", resynced, gen)
	}
	if st, _ := s.fleetStore.EdgeState(edge.EdgeID); st.Cursor != "2" {
		t.Fatalf("cursor not realigned to tail: %q", st.Cursor)
	}
	gen2, resynced2, _ := s.fleetStore.ReconcileInboundCursor(edge.EdgeID)
	if resynced2 || gen2 != 2 {
		t.Fatalf("consistent reconcile should be a no-op: resynced=%v gen=%d", resynced2, gen2)
	}
}

// ---- outbound WAL crash recovery (B4) -----------------------------------

func TestFleetOutboundPendingDerivedFromJournal(t *testing.T) {
	s := newFleetServer(t)
	edge := dialEdge(t, s, "peer")
	frame, _ := marshalFleetMsg("cid-ffff00000001", "2026-07-23T00:00:00Z", "out", "brief", fleetFrom{Box: "me"})
	// EnqueueOutboundTx journals dir=out ONLY — no separate pending write to strand.
	if _, _, err := s.fleetStore.EnqueueOutboundTx(edge.EdgeID, "cid-ffff00000001", frame); err != nil {
		t.Fatal(err)
	}
	// Pending is derived purely from the journal (survives any state.json loss).
	p, _ := s.fleetStore.PendingOutbound(edge.EdgeID)
	if len(p) != 1 || p[0].CID != "cid-ffff00000001" {
		t.Fatalf("pending not derived from journal: %+v", p)
	}
	found, remaining, err := s.fleetStore.RecordPeerAckCID(edge.EdgeID, "cid-ffff00000001")
	if err != nil || !found || remaining != 0 {
		t.Fatalf("ack should drain: found=%v remaining=%d err=%v", found, remaining, err)
	}
	if p, _ := s.fleetStore.PendingOutbound(edge.EdgeID); len(p) != 0 {
		t.Fatalf("acked frame resurrected as pending: %+v", p)
	}
	// A duplicate ack is a harmless no-op (idempotent — no second WAL marker).
	if _, _, err := s.fleetStore.RecordPeerAckCID(edge.EdgeID, "cid-ffff00000001"); err != nil {
		t.Fatalf("duplicate ack errored: %v", err)
	}
}

// ---- fleet_resume prunes without body replay (B2) -----------------------

func TestFleetResumePrunesPending(t *testing.T) {
	s := newFleetServer(t)
	edge := dialEdge(t, s, "peer")
	for _, cid := range []string{"cid-res000000001", "cid-res000000002"} {
		frame, _ := marshalFleetMsg(cid, "t", "out", "brief", fleetFrom{Box: "me"})
		if _, _, err := s.fleetStore.EnqueueOutboundTx(edge.EdgeID, cid, frame); err != nil {
			t.Fatal(err)
		}
	}
	ob := newFleetOutbox()
	s.peerResume(edge.EdgeID, fleetResumeFrame([]string{"cid-res000000001"}), ob)
	p, _ := s.fleetStore.PendingOutbound(edge.EdgeID)
	if len(p) != 1 || p[0].CID != "cid-res000000002" {
		t.Fatalf("resume did not prune cid1: %+v", p)
	}
	if !ob.isAcked("cid-res000000001") {
		t.Fatalf("resume did not mark cid1 in the session outbox")
	}
}

// ---- backoff cap ---------------------------------------------------------

func TestFleetDialBackoffCap(t *testing.T) {
	b := defaultRelayBackoffMin
	for i := 0; i < 50; i++ {
		b = nextBackoff(b, defaultRelayBackoffMax)
		if b > defaultRelayBackoffMax {
			t.Fatalf("backoff exceeded cap: %s > %s", b, defaultRelayBackoffMax)
		}
		if j := jitterBackoff(b, defaultRelayBackoffMax); j > defaultRelayBackoffMax {
			t.Fatalf("jittered backoff exceeded cap: %s", j)
		}
	}
	if b != defaultRelayBackoffMax {
		t.Fatalf("backoff did not saturate at cap: %s", b)
	}
}

// ---- welcome pin as precondition (SF2) ----------------------------------

func TestFleetApplyWelcomePrecondition(t *testing.T) {
	s := newFleetServer(t)
	edge := dialEdge(t, s, "peer")

	if ok, _ := s.applyWelcome(edge, mustMarshal(map[string]any{"t": "welcome_fleet", "box": "PeerBox"})); ok {
		t.Fatalf("empty key_fp accepted")
	}
	if ok, _ := s.applyWelcome(edge, mustMarshal(map[string]any{"t": "welcome_fleet", "box": "PeerBox", "key_fp": "fp-one"})); !ok {
		t.Fatalf("valid first welcome rejected")
	}
	live, _ := s.fleetStore.LiveEdge(edge.EdgeID)
	if live.PeerKeyFP != "fp-one" || live.PeerBoxName != "PeerBox" {
		t.Fatalf("pin/box not recorded: %+v", live)
	}
	if ok, _ := s.applyWelcome(edge, mustMarshal(map[string]any{"t": "welcome_fleet", "box": "Other", "key_fp": "fp-two"})); ok {
		t.Fatalf("mismatched fp accepted")
	}
	live, _ = s.fleetStore.LiveEdge(edge.EdgeID)
	if live.PeerKeyFP != "fp-one" || live.PeerBoxName != "PeerBox" {
		t.Fatalf("mismatch overwrote pin/box: %+v", live)
	}
	if st, _ := s.fleetStore.EdgeState(edge.EdgeID); !st.KeyFPMismatch {
		t.Fatalf("mismatch flag not set")
	}
	if ok, _ := s.applyWelcome(edge, mustMarshal(map[string]any{"t": "welcome_fleet", "box": "Renamed", "key_fp": "fp-one"})); !ok {
		t.Fatalf("matching re-pin rejected")
	}
	live, _ = s.fleetStore.LiveEdge(edge.EdgeID)
	if live.PeerBoxName != "PeerBox" {
		t.Fatalf("box name overwritten after initial pin: %q", live.PeerBoxName)
	}
}

// ---- dial-time creds/origin validation (SF3) ----------------------------

func TestValidateDialCreds(t *testing.T) {
	s := newFleetServer(t)
	edge := dialEdge(t, s, "peer")
	if err := validateDialCreds(edge); err != nil {
		t.Fatalf("valid edge rejected: %v", err)
	}
	bad := edge
	badCreds := *edge.DeviceCreds
	badCreds.Room = "AAAAAAAAAAAAAAAAAAAAAA"
	bad.DeviceCreds = &badCreds
	if err := validateDialCreds(bad); err == nil {
		t.Fatalf("creds room mismatch accepted")
	}
	bad2 := edge
	badCreds2 := *edge.DeviceCreds
	badCreds2.RelayURL = "wss://evil.example"
	bad2.DeviceCreds = &badCreds2
	if err := validateDialCreds(bad2); err == nil {
		t.Fatalf("creds origin mismatch accepted")
	}
}

// ---- inbound crash recovery (journal committed, inject lost) ------------

// A crash between the durable CommitInbound and the agent inject is recovered on
// restart by replayUndeliveredFleetInbound — and re-injected EXACTLY once (dedup by
// InboundDelivered), never twice.
func TestFleetInboundReplayAfterCrash(t *testing.T) {
	s := newFleetServer(t)
	edge := dialEdge(t, s, "peer")
	frame, _ := json.Marshal(map[string]any{"t": "fleet_msg", "cid": "cid-crash0000001", "ts": "t", "text": "survived", "kind": "brief", "from": map[string]any{}})
	// Commit durably with NO sink bound (the "crash" is between journal and inject).
	commit, err := s.fleetStore.CommitInbound(edge.EdgeID, "cid-crash0000001", frame, nil)
	if err != nil || !commit.Committed {
		t.Fatalf("commit: %+v err=%v", commit, err)
	}
	// Restart: bind the sink and replay the journal.
	sink := newFakeSink()
	s.bindFleetSink(sink)
	s.replayUndeliveredFleetInbound(context.Background())
	if len(sink.ch) != 1 {
		t.Fatalf("crash-recovery replay injected %d, want 1", len(sink.ch))
	}
	// A second replay is deduped (InboundDelivered) — never a double inject.
	s.replayUndeliveredFleetInbound(context.Background())
	if len(sink.ch) != 1 {
		t.Fatalf("second replay re-injected: %d, want 1", len(sink.ch))
	}
}

// ---- dead-edge marking ---------------------------------------------------

func TestFleetDialMarkDeadOnRevoked(t *testing.T) {
	s := newFleetServer(t)
	edge := dialEdge(t, s, "peer")
	outcome, dead := s.dialErrorOutcome(edge, errorFrame("revoked", ""))
	if !dead || outcome.reason != "revoked" {
		t.Fatalf("revoked error should be fatal: %+v dead=%v", outcome, dead)
	}
	if err := s.fleetStore.MarkDialEdgeDead(edge.EdgeID, outcome.reason); err != nil {
		t.Fatal(err)
	}
	if _, ok := s.fleetStore.LiveEdge(edge.EdgeID); ok {
		t.Fatalf("edge should be dead/tombstoned")
	}
	edges, _ := s.fleetStore.Edges()
	if len(edges) != 1 || edges[0].Tombstone == nil || edges[0].Tombstone.Reason != "revoked" {
		t.Fatalf("tombstone reason not recorded: %+v", edges)
	}
	if _, dead := s.dialErrorOutcome(edge, errorFrame("protocol_error", "")); dead {
		t.Fatalf("protocol_error should not mark the edge dead")
	}
}
