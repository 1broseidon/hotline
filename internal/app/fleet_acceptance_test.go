package app

import (
	"context"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/mcpchan"
	"github.com/1broseidon/hotline/internal/transcript"
	"github.com/gorilla/websocket"
)

// This is Lane L3's centerpiece: a TRUE two-box duplex over a real WebSocket
// transport, driven by the REAL fleet managers (runFleetRoomManager /
// runFleetDialManager), through an in-process pipe relay (§2: dumb, one socket per
// side, forward verbatim, no close propagation). Box A links a fleet room; box B
// joins the URI and its L3 dial manager dials A's fleet handler on the DEVICE leg
// (e=1); the agents exchange fleet_send turns both ways. A's reply lands in B's OWN
// fleet room (the cross-room bug dead).
//
// Hardened per review: it runs the real managers (not hand-driven loops); it
// RECONSTRUCTS box B from the SAME state dir mid-test (a real process restart) to
// prove outbound-WAL crash recovery (B4); it provisions an operator device+mailbox
// on BOTH boxes and asserts they stay byte-untouched throughout; wait windows cover
// the ≥1s reconnect backoff; and `fleet rm` on A propagates via the serve session's
// live-check → authenticated revoked → the dialer marks the edge dead (B5b).

// ---- in-process pipe relay (§2) -----------------------------------------

type pipeConn struct {
	ws *websocket.Conn
	mu sync.Mutex
}

func (p *pipeConn) send(mt int, data []byte) error {
	p.mu.Lock()
	defer p.mu.Unlock()
	return p.ws.WriteMessage(mt, data)
}

type pipeRoom struct {
	mu    sync.Mutex
	sides map[string]*pipeConn
}

type pipeRelay struct {
	up    websocket.Upgrader
	mu    sync.Mutex
	rooms map[string]*pipeRoom
}

func newPipeRelay() *pipeRelay {
	return &pipeRelay{
		up:    websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }},
		rooms: map[string]*pipeRoom{},
	}
}

func (pr *pipeRelay) room(id string) *pipeRoom {
	pr.mu.Lock()
	defer pr.mu.Unlock()
	r := pr.rooms[id]
	if r == nil {
		r = &pipeRoom{sides: map[string]*pipeConn{}}
		pr.rooms[id] = r
	}
	return r
}

func (pr *pipeRelay) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	parts := strings.Split(strings.Trim(r.URL.Path, "/"), "/")
	if len(parts) != 3 || parts[0] != "r" || (parts[2] != "c" && parts[2] != "a") {
		http.Error(w, "bad rendezvous path", http.StatusBadRequest)
		return
	}
	roomID, side := parts[1], parts[2]
	ws, err := pr.up.Upgrade(w, r, nil)
	if err != nil {
		return
	}
	pc := &pipeConn{ws: ws}
	room := pr.room(roomID)
	room.mu.Lock()
	if prev := room.sides[side]; prev != nil {
		_ = prev.send(websocket.CloseMessage, websocket.FormatCloseMessage(4001, "replaced"))
		_ = prev.ws.Close()
	}
	room.sides[side] = pc
	room.mu.Unlock()
	other := "a"
	if side == "a" {
		other = "c"
	}
	for {
		mt, data, err := ws.ReadMessage()
		if err != nil {
			room.mu.Lock()
			if room.sides[side] == pc {
				delete(room.sides, side)
			}
			room.mu.Unlock()
			_ = ws.Close()
			return
		}
		if mt != websocket.TextMessage {
			continue
		}
		room.mu.Lock()
		dst := room.sides[other]
		room.mu.Unlock()
		if dst != nil {
			_ = dst.send(websocket.TextMessage, data)
		}
	}
}

// ---- helpers -------------------------------------------------------------

func eventually(t *testing.T, d time.Duration, desc string, fn func() bool) {
	t.Helper()
	deadline := time.Now().Add(d)
	for time.Now().Before(deadline) {
		if fn() {
			return
		}
		time.Sleep(25 * time.Millisecond)
	}
	t.Fatalf("condition not met within %s: %s", d, desc)
}

// collectSink is a non-blocking inbound sink (unlike the cap-8 fakeSink) so many
// injections across a restart never deadlock the box.
type collectSink struct {
	mu  sync.Mutex
	got []capture
}

func (c *collectSink) SendChannel(_ context.Context, content string, meta map[string]string) error {
	c.mu.Lock()
	c.got = append(c.got, capture{content: content, meta: meta})
	c.mu.Unlock()
	return nil
}
func (c *collectSink) SendVerdict(_ context.Context, _, _ string) error { return nil }

func (c *collectSink) matching(sub string) []capture {
	c.mu.Lock()
	defer c.mu.Unlock()
	var out []capture
	for _, g := range c.got {
		if strings.Contains(g.content, sub) {
			out = append(out, g)
		}
	}
	return out
}

// startFleetManagers runs a box's REAL fleet managers under a fresh ctx and returns
// a stop func that cancels and waits for a clean shutdown.
func startFleetManagers(s *Server) func() {
	ctx, cancel := context.WithCancel(context.Background())
	var wg sync.WaitGroup
	wg.Add(2)
	go func() { defer wg.Done(); _ = s.runFleetRoomManager(ctx) }()
	go func() { defer wg.Done(); _ = s.runFleetDialManager(ctx) }()
	return func() { cancel(); wg.Wait() }
}

// provisionOperator binds an active operator device with a mailbox on a box (the
// isolation baseline). Returns the device id.
func provisionOperator(t *testing.T, s *Server, name string) string {
	t.Helper()
	link, err := s.store.MintLinkMode(fleetTestRelay, name, false)
	if err != nil {
		t.Fatalf("mint operator room: %v", err)
	}
	dev := "dev-" + name
	if _, _, err := s.store.VerifyAndLink(link.Room, dev, link.Secret); err != nil {
		t.Fatalf("verify+link: %v", err)
	}
	if err := s.provisionMailbox(dev); err != nil {
		t.Fatalf("provision mailbox: %v", err)
	}
	return dev
}

func assertOperatorUntouched(t *testing.T, s *Server, dev string, outbox0, head0 uint64) {
	t.Helper()
	// L4: fleet activity now emits a CONTENT-FREE fleet_state transient (§6, the
	// operator awareness surface — same family as agent_state), which reserves a seq
	// from the SHARED server->app counter (so it may advance the number the next
	// durable frame carries — that perturbation is permitted). The DURABLE isolation
	// guarantee is what must hold: no fleet frame ever becomes a durable operator
	// frame. durableHead() counts only durable frames, so it does not observe the
	// transient; the mailbox head + queued-items checks below prove no fleet CONTENT
	// reached the operator's durable mailbox.
	if got := s.durableHead(); got != outbox0 {
		t.Fatalf("fleet traffic advanced the operator DURABLE head %d -> %d", outbox0, got)
	}
	if got := s.mailbox.contiguousHead(dev); got != head0 {
		t.Fatalf("fleet traffic moved the operator mailbox head %d -> %d", head0, got)
	}
	_, _, items, sub, err := s.mailbox.stateAndSubscribe(dev)
	if err != nil {
		t.Fatalf("subscribe: %v", err)
	}
	defer s.mailbox.unsubscribe(dev, sub)
	if len(items) != 0 {
		t.Fatalf("operator mailbox received %d item(s) from the fleet lane", len(items))
	}
}

// ---- the acceptance test -------------------------------------------------

func TestFleetTwoBoxDuplexAcceptance(t *testing.T) {
	relay := newPipeRelay()
	srv := httptest.NewServer(relay)
	defer srv.Close()
	wsBase := "ws://" + strings.TrimPrefix(srv.URL, "http://")

	// Box A (stable across the whole test).
	cfgA := testConfig(t)
	boxA := NewServer(cfgA, transcript.New(cfgA.TranscriptFile))
	if _, _, err := boxA.store.SeedIdentityName("BoxA"); err != nil {
		t.Fatal(err)
	}
	sinkA := &collectSink{}
	boxA.bindFleetSink(sinkA)

	// Box B (will be reconstructed from the same state dir mid-test).
	cfgB := testConfig(t)
	t.Cleanup(func() {
		if t.Failed() {
			for _, p := range []string{cfgA.StateDir + "/fleet.log", cfgB.StateDir + "/fleet.log"} {
				b, _ := os.ReadFile(p)
				t.Logf("=== %s ===\n%s", p, b)
			}
		}
	})
	boxB := NewServer(cfgB, transcript.New(cfgB.TranscriptFile))
	if _, _, err := boxB.store.SeedIdentityName("BoxB"); err != nil {
		t.Fatal(err)
	}
	sinkB := &collectSink{}
	boxB.bindFleetSink(sinkB)

	// Operator provisioning + isolation baselines on both boxes.
	opA := provisionOperator(t, boxA, "opA")
	opB := provisionOperator(t, boxB, "opB")
	outboxA0, headA0 := boxA.durableHead(), boxA.mailbox.contiguousHead(opA)
	outboxB0, headB0 := boxB.durableHead(), boxB.mailbox.contiguousHead(opB)

	// A links; B joins.
	edgeA, secret, err := boxA.fleetStore.Link(wsBase, "boxB")
	if err != nil {
		t.Fatalf("A link: %v", err)
	}
	uri := FleetPairingURI(edgeA.RelayURL, edgeA.Room, secret, "BoxA")
	origin, _ := rendezvousOrigin(edgeA.RelayURL)
	edgeB, err := boxB.fleetStore.Join(uri, "boxA", JoinOptions{AllowedOrigins: []string{origin}})
	if err != nil {
		t.Fatalf("B join: %v", err)
	}

	// Real managers: A serves, B dials. Both boxes are reconstructed later, so the
	// live handles are held in curA/curB.
	curA, curB := boxA, boxB
	stopA := startFleetManagers(curA)
	defer func() { stopA() }()
	stopB := startFleetManagers(curB)
	defer func() { stopB() }()

	connected := func() bool {
		_, aok := curA.fleetSessionWriter(edgeA.EdgeID)
		_, bok := curB.fleetSessionWriter(edgeB.EdgeID)
		return aok && bok
	}
	eventually(t, 15*time.Second, "both sides connect via the real managers", connected)

	// B pinned A's identity from welcome_fleet.
	if liveB, _ := curB.fleetStore.LiveEdge(edgeB.EdgeID); liveB.PeerBoxName != "BoxA" || liveB.PeerKeyFP == "" {
		t.Fatalf("dialer did not pin A identity: %+v", liveB)
	}

	// B agent → A: fleet_send crosses the wire and injects on A.
	provB := newFleetProviderFor(curB)
	if msg, isErr := provB.FleetSend(context.Background(), mcpchan.FleetSendInput{To: "boxA", Text: "hi from B", Kind: "task"}); isErr {
		t.Fatalf("B fleet_send failed: %s", msg)
	}
	eventually(t, 10*time.Second, "A injects B's turn", func() bool { return len(sinkA.matching("hi from B")) == 1 })
	capA := sinkA.matching("hi from B")[0]
	if !strings.Contains(capA.content, "untrusted peer data") {
		t.Fatalf("A injection missing trust marker: %q", capA.content)
	}
	if capA.meta["source"] != "fleet" || capA.meta["chat_id"] != fleetChatID(edgeA.EdgeID) || capA.meta["kind"] != "fleet" {
		t.Fatalf("A injection meta wrong: %+v", capA.meta)
	}
	eventually(t, 10*time.Second, "B pending drains after A ack", func() bool {
		p, _ := curB.fleetStore.PendingOutbound(edgeB.EdgeID)
		return len(p) == 0
	})

	// A agent → B: reply lands in B's OWN room (the cross-room bug dead).
	provA := newFleetProviderFor(curA)
	if msg, isErr := provA.FleetSend(context.Background(), mcpchan.FleetSendInput{To: "boxB", Text: "reply from A", Kind: "result"}); isErr {
		t.Fatalf("A fleet_send failed: %s", msg)
	}
	eventually(t, 10*time.Second, "B injects A's reply", func() bool { return len(sinkB.matching("reply from A")) == 1 })
	capB := sinkB.matching("reply from A")[0]
	if capB.meta["chat_id"] != fleetChatID(edgeB.EdgeID) || capB.meta["source"] != "fleet" {
		t.Fatalf("B injection meta wrong (reply must land in B's room): %+v", capB.meta)
	}
	eventually(t, 10*time.Second, "A pending drains after B ack", func() bool {
		p, _ := curA.fleetStore.PendingOutbound(edgeA.EdgeID)
		return len(p) == 0
	})

	// Journals consistent both directions (ignoring dir=ack WAL markers).
	assertInOut := func(s *Server, edgeID string, wantIn, wantOut int) {
		t.Helper()
		entries, _ := s.fleetStore.JournalEntries(edgeID)
		in, out := 0, 0
		for _, e := range entries {
			switch e.Dir {
			case "in":
				in++
			case "out":
				out++
			}
		}
		if in != wantIn || out != wantOut {
			t.Fatalf("journal %s: in=%d out=%d want in=%d out=%d", edgeID, in, out, wantIn, wantOut)
		}
	}
	assertInOut(curA, edgeA.EdgeID, 1, 1)
	assertInOut(curB, edgeB.EdgeID, 1, 1)

	// Operator isolation on both boxes: the fleet traffic never touched them.
	assertOperatorUntouched(t, curA, opA, outboxA0, headA0)
	assertOperatorUntouched(t, curB, opB, outboxB0, headB0)

	// --- CRASH RECOVERY: reconstruct box B from the SAME state dir (real restart) ---
	// B "crashes" (managers stopped) with an outbound queued ONLY in its journal WAL
	// (no state.json pending write to strand); a fresh box B is then built from the
	// same state dir (identity, box key, fleet registry, journal WAL all recover) and
	// its dialer delivers the WAL-recovered frame exactly once. Box A stays up (its
	// stale /c session self-heals on B's reconnect via the hello-resend path).
	stopB()
	if msg, isErr := provB.FleetSend(context.Background(), mcpchan.FleetSendInput{To: "boxA", Text: "queued while offline", Kind: "brief"}); isErr {
		t.Fatalf("B offline fleet_send failed: %s", msg)
	}
	boxB2 := NewServer(cfgB, transcript.New(cfgB.TranscriptFile))
	boxB2.bindFleetSink(&collectSink{})
	curB = boxB2
	stopB = startFleetManagers(curB)

	eventually(t, 25*time.Second, "reconstructed B reconnects", connected)
	// The queued outbound was reconstructed from the journal WAL and delivered ONCE.
	eventually(t, 15*time.Second, "A injects the WAL-recovered message", func() bool {
		return len(sinkA.matching("queued while offline")) == 1
	})
	eventually(t, 10*time.Second, "recovered B pending drains", func() bool {
		p, _ := curB.fleetStore.PendingOutbound(edgeB.EdgeID)
		return len(p) == 0
	})
	// No duplicate across the restart, even after waiting past the reconnect backoff.
	time.Sleep(1500 * time.Millisecond)
	if n := len(sinkA.matching("queued while offline")); n != 1 {
		t.Fatalf("WAL-recovered message delivered %d times, want exactly 1", n)
	}

	// --- rm on A → the serve session's revoked propagation marks B's edge dead ---
	if _, err := curA.fleetStore.Remove(edgeA.EdgeID); err != nil {
		t.Fatalf("A rm: %v", err)
	}
	eventually(t, 20*time.Second, "B's dialer marks the edge dead after A rm", func() bool {
		_, ok := curB.fleetStore.LiveEdge(edgeB.EdgeID)
		return !ok
	})
	edges, _ := curB.fleetStore.Edges()
	if len(edges) != 1 || edges[0].Tombstone == nil {
		t.Fatalf("B edge should be tombstoned dead: %+v", edges)
	}
}
