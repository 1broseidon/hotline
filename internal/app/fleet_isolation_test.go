package app

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestFleetOperatorMailboxIsolation is the two-room cross-talk proof (a2a-design-v2
// §3.1 acceptance): with one operator room (a live device + mailbox) and one fleet
// serve edge on the same box, a fleet_msg lands ONLY in the fleet edge journal —
// the operator device's mailbox never receives it and no durable outbox seq is
// issued. The two transports share only the Server, no durable state.
func TestFleetOperatorMailboxIsolation(t *testing.T) {
	s := newFleetServer(t)

	// Operator side: mint a room and bind an active device with a mailbox.
	link, err := s.store.MintLinkMode(fleetTestRelay, "Operator", false)
	if err != nil {
		t.Fatalf("mint operator room: %v", err)
	}
	const deviceID = "dev-abc123"
	if _, _, err := s.store.VerifyAndLink(link.Room, deviceID, link.Secret); err != nil {
		t.Fatalf("verify+link: %v", err)
	}
	if err := s.provisionMailbox(deviceID); err != nil {
		t.Fatalf("provision mailbox: %v", err)
	}

	outboxBefore := s.durableHead()
	opHeadBefore := s.mailbox.contiguousHead(deviceID)

	// Fleet side: a serve edge receives a fleet_msg.
	edge, hello := serveEdge(t, s, "peer")
	res, _ := runFleetSession(t, s, edge, hello, fmsgRaw("cid-isolation01", "fleet-only payload", "brief"))
	if res.closeCode != 0 {
		t.Fatalf("fleet session should end cleanly: %+v", res)
	}

	// The fleet frame IS in the edge journal.
	entries, _ := s.fleetStore.JournalEntries(edge.EdgeID)
	if len(entries) != 1 {
		t.Fatalf("fleet journal expected 1 entry, got %d", len(entries))
	}

	// The operator DURABLE STATE is untouched: no durable frame is issued, the mailbox
	// head does not move, and nothing is queued for the device. (Honest scope, L4: a
	// fleet_state transient — like the whole agent_state family — is drawn from the
	// shared server->app seq counter, so it MAY advance the number the next durable
	// frame will carry; what it never does is write a durable record, move a mailbox
	// head, queue an item, or reach a fleet peer. durableHead() counts only durable
	// frames, so it does not observe the transient. The live-subscriber recipient +
	// content isolation is proven by TestFleetStateOperatorIsolationLive below.)
	if got := s.durableHead(); got != outboxBefore {
		t.Fatalf("fleet_msg advanced the operator durable head %d -> %d", outboxBefore, got)
	}
	if got := s.mailbox.contiguousHead(deviceID); got != opHeadBefore {
		t.Fatalf("fleet_msg moved the operator mailbox head %d -> %d", opHeadBefore, got)
	}
	_, _, items, sub, err := s.mailbox.stateAndSubscribe(deviceID)
	if err != nil {
		t.Fatalf("subscribe: %v", err)
	}
	defer s.mailbox.unsubscribe(deviceID, sub)
	if len(items) != 0 {
		t.Fatalf("operator mailbox received %d item(s) from the fleet lane", len(items))
	}
}

// TestFleetStateOperatorIsolationLive is the L4-sol BLOCKER-1 strengthening, updated for
// post-mortem fixes A1 (capability gate) + A2 (separate seq domain). With a live operator
// device that ADVERTISED fleet-state support AND a fleet session that just carried a
// secret payload, a forced fleet_state broadcast proves:
//
//	(i)   durable-file identity — outbox.jsonl is byte-identical before/after;
//	(ii)  RAW shared outbox cursor UNCHANGED — the fleet activity + the fleet_state
//	      broadcast reserve from the SEPARATE fleet_state seq domain, never
//	      s.outbox.reserveTransient(), so the shared operator cursor never moves (A2);
//	(iii) recipient isolation — the fleet-session peer NEVER receives a fleet_state frame;
//	(iv)  content isolation — the fleet_state carries none of {peer text, cid, kind, from,
//	      peer name/fp, room, url, secret}.
func TestFleetStateOperatorIsolationLive(t *testing.T) {
	s := newFleetServer(t)

	// Operator side: room + active device + mailbox + a live subscriber.
	link, err := s.store.MintLinkMode(fleetTestRelay, "Operator", false)
	if err != nil {
		t.Fatalf("mint operator room: %v", err)
	}
	const deviceID = "dev-abc123"
	if _, _, err := s.store.VerifyAndLink(link.Room, deviceID, link.Secret); err != nil {
		t.Fatalf("verify+link: %v", err)
	}
	if err := s.provisionMailbox(deviceID); err != nil {
		t.Fatalf("provision mailbox: %v", err)
	}
	_, _, _, sub, err := s.mailbox.stateAndSubscribe(deviceID)
	if err != nil {
		t.Fatalf("subscribe: %v", err)
	}
	defer s.mailbox.unsubscribe(deviceID, sub)
	// A1: opt this device into fleet_state (as a hello advertising fleetStateCapToken
	// would). Without this it would receive NOTHING — proven by TestFleetStateCapGate.
	defer s.markFleetStateCap(deviceID)()

	outboxPath := filepath.Join(s.cfg.StateDir, "outbox.jsonl")
	before := readFileOrEmpty(t, outboxPath)
	cursorBefore := s.outbox.cursor()

	// Fleet side: a serve edge carries a secret payload. runFleetSession captures every
	// frame written to the PEER — recipient isolation asserts fleet_state is not among them.
	const secret = "top secret peer payload do not leak"
	const cid = "cid-liveisolat01"
	edge, hello := serveEdge(t, s, "peerLive")
	res, peerFrames := runFleetSession(t, s, edge, hello, fmsgRaw(cid, secret, "brief"))
	if res.closeCode != 0 {
		t.Fatalf("fleet session should end cleanly: %+v", res)
	}

	// A3: the fleet activity above only ARMED the fs timer (mark-dirty) — it never built
	// a snapshot inline and never touched the outbox cursor. Force the pending broadcast
	// by running exactly what the timer goroutine runs (deterministic).
	s.fsMu.Lock()
	if s.fsTimer != nil {
		s.fsTimer.Stop()
		s.fsTimer = nil
	}
	s.fsLastSent = time.Time{}
	s.broadcastFleetStateLocked()
	s.fsMu.Unlock()

	// (iii) recipient isolation — the fleet peer got NO fleet_state.
	for _, f := range peerFrames {
		var m map[string]any
		if json.Unmarshal(f, &m) == nil && m["t"] == "fleet_state" {
			t.Fatalf("fleet_state leaked onto the fleet peer transport")
		}
	}

	// The operator device DID receive exactly a fleet_state transient.
	var opFrame []byte
	select {
	case opFrame = <-sub.transients:
	default:
		t.Fatalf("operator device received no fleet_state transient")
	}
	var m map[string]any
	if json.Unmarshal(opFrame, &m) != nil || m["t"] != "fleet_state" {
		t.Fatalf("operator transient was not a fleet_state frame: %s", opFrame)
	}

	// (iv) content isolation — none of the peer/secret tokens appear in the frame.
	for _, tok := range []string{secret, "top secret", "do not leak", cid, "brief", link.Room, link.Secret} {
		if tok != "" && bytes.Contains(opFrame, []byte(tok)) {
			t.Fatalf("fleet_state frame leaked forbidden token %q: %s", tok, opFrame)
		}
	}
	// Structural: none of the content-carrying keys appear anywhere in the frame.
	for _, key := range []string{"text", "cid", "kind", "from", "key_fp", "room", "url", "secret", "fp"} {
		if strings.Contains(string(opFrame), `"`+key+`"`) {
			t.Fatalf("fleet_state frame carries forbidden key %q: %s", key, opFrame)
		}
	}

	// (i) durable-file identity — outbox.jsonl byte-identical.
	if after := readFileOrEmpty(t, outboxPath); !bytes.Equal(before, after) {
		t.Fatalf("durable outbox.jsonl changed across fleet activity: %d -> %d bytes", len(before), len(after))
	}
	// (ii) A2: the RAW shared outbox cursor is UNCHANGED — fleet_state rides its own seq
	// domain and never consumes s.outbox.reserveTransient().
	if got := s.outbox.cursor(); got != cursorBefore {
		t.Fatalf("fleet activity moved the raw shared outbox cursor %d -> %d (A2 violated)", cursorBefore, got)
	}
}

// TestFleetStateCapGate proves the A1 capability gate: a device that did NOT advertise
// fleet_state support receives no fleet_state transient (post-drain snapshot or change
// broadcast), while an advertising device does — and neither moves the raw outbox cursor.
func TestFleetStateCapGate(t *testing.T) {
	s := newFleetServer(t)
	link, err := s.store.MintLinkMode(fleetTestRelay, "Operator", false)
	if err != nil {
		t.Fatalf("mint operator room: %v", err)
	}
	subFor := func(id string) *mailboxSubscriber {
		if _, _, err := s.store.VerifyAndLink(link.Room, id, link.Secret); err != nil {
			t.Fatalf("verify+link %s: %v", id, err)
		}
		if err := s.provisionMailbox(id); err != nil {
			t.Fatalf("provision %s: %v", id, err)
		}
		_, _, _, sub, err := s.mailbox.stateAndSubscribe(id)
		if err != nil {
			t.Fatalf("subscribe %s: %v", id, err)
		}
		return sub
	}
	const capable, plain = "dev-capable01", "dev-plain0001"
	capSub := subFor(capable)
	defer s.mailbox.unsubscribe(capable, capSub)
	plainSub := subFor(plain)
	defer s.mailbox.unsubscribe(plain, plainSub)
	defer s.markFleetStateCap(capable)() // only the capable device advertised

	// An edge must exist or the snapshot builds nothing.
	serveEdge(t, s, "gatepeer")
	cursorBefore := s.outbox.cursor()

	// Post-drain snapshot: capable gets it, plain does not.
	s.snapshotFleetStateTo(capable)
	s.snapshotFleetStateTo(plain)
	if !hasFleetStateTransient(capSub) {
		t.Fatalf("capable device got no fleet_state snapshot")
	}
	if hasFleetStateTransient(plainSub) {
		t.Fatalf("un-advertised device received a fleet_state snapshot (A1 gate leaked)")
	}
	// Change broadcast: same gate.
	s.fsMu.Lock()
	s.fsLastSent = time.Time{}
	s.broadcastFleetStateLocked()
	s.fsMu.Unlock()
	if hasFleetStateTransient(plainSub) {
		t.Fatalf("un-advertised device received a fleet_state broadcast (A1 gate leaked)")
	}
	if got := s.outbox.cursor(); got != cursorBefore {
		t.Fatalf("fleet_state moved the raw shared outbox cursor %d -> %d (A2 violated)", cursorBefore, got)
	}
}

// hasFleetStateTransient drains a subscriber and reports whether any transient was a
// fleet_state frame.
func hasFleetStateTransient(sub *mailboxSubscriber) bool {
	for {
		select {
		case f := <-sub.transients:
			var m map[string]any
			if json.Unmarshal(f, &m) == nil && m["t"] == "fleet_state" {
				return true
			}
		default:
			return false
		}
	}
}

func readFileOrEmpty(t *testing.T, path string) []byte {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return nil
		}
		t.Fatalf("read %s: %v", path, err)
	}
	return data
}
