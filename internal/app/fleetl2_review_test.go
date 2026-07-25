package app

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/mcpchan"
)

// TestFleetInboundNoReplayWhenDelivered proves H2: an inbound fleet_msg that was
// successfully injected (its cid marked delivered) is NEVER re-injected by the
// startup fleet-journal replay.
func TestFleetInboundNoReplayWhenDelivered(t *testing.T) {
	s := newFleetServer(t)
	sink := newFakeSink()
	s.bindFleetSink(sink)
	edge, hello := serveEdge(t, s, "peerB")

	runFleetSession(t, s, edge, hello, fmsgRaw("cid-deliv00000001", "hi", "brief"))
	<-sink.ch // the one live delivery

	done, err := s.fleetStore.InboundDelivered(edge.EdgeID, "cid-deliv00000001")
	if err != nil || !done {
		t.Fatalf("cid not marked delivered: done=%t err=%v", done, err)
	}

	// Replay through a fresh sink: a delivered turn must produce nothing.
	replay := newFakeSink()
	s.bindFleetSink(replay)
	s.replayUndeliveredFleetInbound(context.Background())
	select {
	case c := <-replay.ch:
		t.Fatalf("delivered turn was replayed: %q", c.content)
	case <-time.After(200 * time.Millisecond):
	}
}

// TestFleetInboundReplayWhenUndelivered proves H2: a turn journaled but never
// delivered (box died / sink not bound between journal and inject) IS replayed on
// startup, with fleet provenance intact (source=fleet, fleet chat_id, trust
// marker) — and a second replay is a no-op once it lands.
func TestFleetInboundReplayWhenUndelivered(t *testing.T) {
	s := newFleetServer(t)
	// No sink bound: handleFleetMsg journals the inbound, injectInbound skips
	// (sink nil), so the cid stays undelivered.
	edge, hello := serveEdge(t, s, "peerB")
	runFleetSession(t, s, edge, hello, fmsgFrom("cid-undeliv00001", "peer body", "brief", "BoxB"))

	if done, _ := s.fleetStore.InboundDelivered(edge.EdgeID, "cid-undeliv00001"); done {
		t.Fatalf("undelivered turn wrongly marked delivered before any sink existed")
	}

	sink := newFakeSink()
	s.bindFleetSink(sink)
	s.replayUndeliveredFleetInbound(context.Background())
	select {
	case c := <-sink.ch:
		if c.meta["source"] != "fleet" {
			t.Fatalf("replay lost source=fleet: %+v", c.meta)
		}
		if c.meta["chat_id"] != "fleet:"+edge.EdgeID {
			t.Fatalf("replay chat_id = %q", c.meta["chat_id"])
		}
		if strings.Count(c.content, "untrusted peer data") != 1 {
			t.Fatalf("replay content missing/duplicated trust marker: %q", c.content)
		}
		if !strings.HasSuffix(c.content, "peer body") {
			t.Fatalf("replay body = %q", c.content)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("undelivered turn was not replayed")
	}

	// Now delivered — a second replay must inject nothing.
	s.replayUndeliveredFleetInbound(context.Background())
	select {
	case c := <-sink.ch:
		t.Fatalf("second replay re-injected an already-delivered turn: %q", c.content)
	case <-time.After(200 * time.Millisecond):
	}
}

// TestFleetIdentitySpoofDisplay proves M5: a peer claiming the operator's own name
// in from.box is shown as an explicit unverified CLAIM appended to the operator-
// chosen alias, never as an authoritative identity; control chars are stripped and
// a key mismatch is flagged.
func TestFleetIdentitySpoofDisplay(t *testing.T) {
	if got := fleetUserDisplay("peerB", "george", false, fleetAttestNone); got != `peerB [claims: george, unverified]` {
		t.Fatalf("spoof display = %q, want alias + unverified claim", got)
	}
	got := fleetUserDisplay("peerB", "ge\norge\x07", false, fleetAttestNone)
	if strings.ContainsAny(got, "\n\x07") {
		t.Fatalf("control chars survived the claim: %q", got)
	}
	if got := fleetUserDisplay("peerB", "george", true, fleetAttestNone); !strings.Contains(got, "MISMATCH") {
		t.Fatalf("key mismatch not flagged: %q", got)
	}
	// No claim + no mismatch: bare authoritative alias, no brackets.
	if got := fleetUserDisplay("peerB", "", false, fleetAttestNone); got != "peerB" {
		t.Fatalf("bare alias display = %q, want \"peerB\"", got)
	}
}

// TestFleetPeerKeyFPPin proves M5 trust-on-first-use: the first key_fp pins, the
// same fp is a no-op, and a DIFFERENT fp flags a mismatch without ever overriding
// the pin.
func TestFleetPeerKeyFPPin(t *testing.T) {
	s := newFleetServer(t)
	edge, _ := serveEdge(t, s, "peerB")

	if pinned, mm, err := s.fleetStore.PinPeerKeyFP(edge.EdgeID, "fp-first"); err != nil || !pinned || mm {
		t.Fatalf("first pin: pinned=%t mismatch=%t err=%v", pinned, mm, err)
	}
	if pinned, mm, err := s.fleetStore.PinPeerKeyFP(edge.EdgeID, "fp-first"); err != nil || pinned || mm {
		t.Fatalf("same fp should be a no-op: pinned=%t mismatch=%t err=%v", pinned, mm, err)
	}
	if pinned, mm, err := s.fleetStore.PinPeerKeyFP(edge.EdgeID, "fp-DIFFERENT"); err != nil || pinned || !mm {
		t.Fatalf("different fp should flag mismatch, not override: pinned=%t mismatch=%t err=%v", pinned, mm, err)
	}
	edges, _ := s.fleetStore.Edges()
	for _, e := range edges {
		if e.EdgeID == edge.EdgeID && e.PeerKeyFP != "fp-first" {
			t.Fatalf("pinned fp was overridden: %q", e.PeerKeyFP)
		}
	}
}

// TestFleetSendRefusesAfterTombstone proves H4: once an edge is tombstoned by a
// concurrent rm, a send fails closed and NOTHING is queued against the tombstone —
// both via the tool path and directly at the atomic store transaction.
func TestFleetSendRefusesAfterTombstone(t *testing.T) {
	s := newFleetServer(t)
	s.store.SeedIdentityName("TestBox")
	fp := newFleetProviderFor(s)
	edge, _ := serveEdge(t, s, "peerB")

	if _, err := s.fleetStore.Remove("peerB"); err != nil {
		t.Fatalf("remove: %v", err)
	}

	msg, isErr := fp.FleetSend(context.Background(), mcpchan.FleetSendInput{To: edge.EdgeID, Text: "after rm"})
	if !isErr {
		t.Fatalf("fleet_send after tombstone accepted: %q", msg)
	}

	// The atomic transaction itself must revalidate the live edge and reject.
	frame := json.RawMessage(`{"t":"fleet_msg","cid":"cid-tomb00000001","text":"x"}`)
	if _, _, err := s.fleetStore.EnqueueOutboundTx(edge.EdgeID, "cid-tomb00000001", frame); err == nil {
		t.Fatalf("EnqueueOutboundTx queued against a tombstone")
	}
	st, _ := s.fleetStore.EdgeState(edge.EdgeID)
	if len(st.Pending) != 0 {
		t.Fatalf("queued %d entries against a tombstone", len(st.Pending))
	}
}

// TestFleetConcurrentSendDuringAttachNoDoubleSend proves H4 under -race: a single
// serve-session attach (register+drain) racing a burst of concurrent fleet_sends
// delivers each message to the peer socket EXACTLY ONCE — never both live-pushed
// and drained. The per-edge delivery lock linearizes the two critical sections, so
// every send is either pending-then-drained or pushed, but never both.
func TestFleetConcurrentSendDuringAttachNoDoubleSend(t *testing.T) {
	s := newFleetServer(t)
	s.store.SeedIdentityName("TestBox")
	fp := newFleetProviderFor(s)
	edge, _ := serveEdge(t, s, "peerB")

	var mu sync.Mutex
	writes := map[string]int{}
	var dereg func()
	write := func(b []byte) error {
		var m struct {
			T   string `json:"t"`
			CID string `json:"cid"`
		}
		if json.Unmarshal(b, &m) == nil && m.T == "fleet_msg" {
			mu.Lock()
			writes[m.CID]++
			mu.Unlock()
		}
		return nil
	}

	const n = 6
	start := make(chan struct{})
	var wg sync.WaitGroup

	wg.Add(1)
	go func() {
		defer wg.Done()
		<-start
		d := s.attachFleetSession(edge.EdgeID, write, true)
		mu.Lock()
		dereg = d
		mu.Unlock()
	}()
	for i := 0; i < n; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			<-start
			if _, isErr := fp.FleetSend(context.Background(), mcpchan.FleetSendInput{To: "peerB", Text: fmt.Sprintf("m%d", i)}); isErr {
				t.Errorf("fleet_send %d failed", i)
			}
		}(i)
	}
	close(start) // release all at once for maximal contention
	wg.Wait()

	mu.Lock()
	defer mu.Unlock()
	if dereg != nil {
		defer dereg()
	}
	if len(writes) != n {
		t.Fatalf("distinct frames written = %d, want %d (a send was lost or duplicated)", len(writes), n)
	}
	for cid, count := range writes {
		if count != 1 {
			t.Fatalf("cid %s written %d times — double-send window is open", cid, count)
		}
	}
}
