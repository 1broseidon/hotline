package app

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"

	"github.com/gorilla/websocket"
)

// fakeCore models the CoreRoomDO pipe plane semantics that the box faces in core
// mode (hotline-core src/index.ts):
//   - exactly ONE socket per side ("a" app, "c" box); a new connect on a side
//     closes the prior one on that side with 4001 "replaced",
//   - webSocketMessage forwards a<->c ONLY while the peer is OPEN, else drops,
//   - webSocketClose is NOT propagated to the peer (the box never learns the app
//     left).
type fakeCore struct {
	mu sync.Mutex
	a  *websocket.Conn
	c  *websocket.Conn
	up websocket.Upgrader
}

func (fc *fakeCore) peer(side string) *websocket.Conn {
	if side == "a" {
		return fc.c
	}
	return fc.a
}

func (fc *fakeCore) set(side string, conn *websocket.Conn) *websocket.Conn {
	fc.mu.Lock()
	defer fc.mu.Unlock()
	var old *websocket.Conn
	if side == "a" {
		old, fc.a = fc.a, conn
	} else {
		old, fc.c = fc.c, conn
	}
	return old
}

func (fc *fakeCore) clear(side string, conn *websocket.Conn) {
	fc.mu.Lock()
	defer fc.mu.Unlock()
	if side == "a" && fc.a == conn {
		fc.a = nil
	}
	if side == "c" && fc.c == conn {
		fc.c = nil
	}
}

func (fc *fakeCore) handle(side string, conn *websocket.Conn) {
	if old := fc.set(side, conn); old != nil {
		_ = old.WriteControl(websocket.CloseMessage, websocket.FormatCloseMessage(4001, "replaced"), time.Now().Add(time.Second))
		_ = old.Close()
	}
	for {
		mt, raw, err := conn.ReadMessage()
		if err != nil {
			fc.clear(side, conn) // NB: no peer close propagation.
			return
		}
		fc.mu.Lock()
		peer := fc.peer(side)
		fc.mu.Unlock()
		if peer == nil {
			continue // drop while peer absent
		}
		_ = peer.WriteMessage(mt, raw)
	}
}

func newFakeCore(t *testing.T) (*httptest.Server, *fakeCore) {
	fc := &fakeCore{up: websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}}
	mux := http.NewServeMux()
	serve := func(side string) http.HandlerFunc {
		return func(w http.ResponseWriter, r *http.Request) {
			conn, err := fc.up.Upgrade(w, r, nil)
			if err != nil {
				return
			}
			fc.handle(side, conn)
			_ = conn.Close()
		}
	}
	mux.HandleFunc("/a", serve("a"))
	mux.HandleFunc("/c", serve("c"))
	ts := httptest.NewServer(mux)
	t.Cleanup(ts.Close)
	return ts, fc
}

func dialSide(t *testing.T, ts *httptest.Server, side string) *websocket.Conn {
	t.Helper()
	wsURL := "ws" + ts.URL[len("http"):] + "/" + side
	conn, _, err := websocket.DefaultDialer.Dial(wsURL, nil)
	if err != nil {
		t.Fatalf("dial %s: %v", side, err)
	}
	return conn
}

// readTypedWithin reads frames until it sees the wanted type or times out.
func readTypedWithin(t *testing.T, conn *websocket.Conn, want string, d time.Duration) []byte {
	t.Helper()
	deadline := time.Now().Add(d)
	for {
		_ = conn.SetReadDeadline(deadline)
		_, raw, err := conn.ReadMessage()
		if err != nil {
			t.Fatalf("waiting for %q: %v", want, err)
		}
		if typ, ok := exactType(raw); ok && typ == want {
			return raw
		}
	}
}

// TestCoreModeReconnectDeliversWelcome reproduces the B1 field scenario: a single
// box /c relay session serves an app that links, converses, drops WITHOUT the box
// being told, then reconnects with a fresh /a and re-sends hello. The box must
// re-emit a welcome so the reconnecting client can converse.
func TestCoreModeReconnectDeliversWelcome(t *testing.T) {
	const deviceID = "dev-af31fd290542"
	h := newSessionHarness(t, deviceID) // provisions store + mailbox for deviceID
	ts, _ := newFakeCore(t)

	// Box side: one long-lived /c relay session for the room (core mode).
	boxConn := dialSide(t, ts, "c")
	ctx, cancel := context.WithCancel(context.Background())
	boxDone := make(chan struct{})
	go func() {
		defer close(boxDone)
		h.srv.serveV2Conn(ctx, boxConn, h.room)
	}()
	t.Cleanup(func() { cancel(); _ = boxConn.Close(); <-boxDone })

	hello := func(resumeFrom string) []byte {
		b, _ := json.Marshal(map[string]any{
			"t": "hello", "v": ProtocolVersion, "device_id": deviceID,
			"secret": h.secret, "resume_from": resumeFrom,
		})
		return b
	}

	// --- First link ---
	app1 := dialSide(t, ts, "a")
	if err := app1.WriteMessage(websocket.TextMessage, hello("0")); err != nil {
		t.Fatal(err)
	}
	readTypedWithin(t, app1, "welcome", 3*time.Second)

	// Agent produces a message; app1 receives and acks it.
	item, err := h.srv.mailbox.enqueue(deviceID, "1", []byte(`{"t":"msg","seq":1,"id":"a-1","text":"hi"}`))
	if err != nil {
		t.Fatal(err)
	}
	raw := readTypedWithin(t, app1, "mailbox_item", 3*time.Second)
	var got MailboxItem
	_ = json.Unmarshal(raw, &got)
	_ = app1.WriteMessage(websocket.TextMessage, mustMarshal(map[string]any{"t": "mailbox_ack", "m": got.M}))
	// let the ack land
	time.Sleep(100 * time.Millisecond)

	// --- App drops WITHOUT the box learning (core does not propagate close) ---
	_ = app1.Close()
	time.Sleep(200 * time.Millisecond)

	// --- Reconnect: fresh /a, resume from the acked cursor ---
	app2 := dialSide(t, ts, "a")
	if err := app2.WriteMessage(websocket.TextMessage, hello(item.M)); err != nil {
		t.Fatal(err)
	}
	// The reconnecting client MUST receive a welcome so it can converse.
	readTypedWithin(t, app2, "welcome", 3*time.Second)

	// And a live message after reconnect must reach it.
	live, err := h.srv.mailbox.enqueue(deviceID, "2", []byte(`{"t":"msg","seq":2,"id":"a-2","text":"after"}`))
	if err != nil {
		t.Fatal(err)
	}
	raw = readTypedWithin(t, app2, "mailbox_item", 3*time.Second)
	_ = json.Unmarshal(raw, &got)
	if got.M != live.M {
		t.Fatalf("post-reconnect delivery M=%s want %s", got.M, live.M)
	}
}

// TestCoreModeOverlapReconnect models a reconnect where the old /a is still
// half-open (not yet closed) when the new /a arrives — the core replaces it.
func TestCoreModeOverlapReconnect(t *testing.T) {
	const deviceID = "dev-af31fd290542"
	h := newSessionHarness(t, deviceID)
	ts, _ := newFakeCore(t)
	boxConn := dialSide(t, ts, "c")
	ctx, cancel := context.WithCancel(context.Background())
	boxDone := make(chan struct{})
	go func() { defer close(boxDone); h.srv.serveV2Conn(ctx, boxConn, h.room) }()
	t.Cleanup(func() { cancel(); _ = boxConn.Close(); <-boxDone })

	hello := func(rf string) []byte {
		b, _ := json.Marshal(map[string]any{"t": "hello", "v": ProtocolVersion, "device_id": deviceID, "secret": h.secret, "resume_from": rf})
		return b
	}
	app1 := dialSide(t, ts, "a")
	_ = app1.WriteMessage(websocket.TextMessage, hello("0"))
	readTypedWithin(t, app1, "welcome", 3*time.Second)

	// app1 NOT closed. app2 connects (core replaces app1 with 4001) and hellos.
	app2 := dialSide(t, ts, "a")
	_ = app2.WriteMessage(websocket.TextMessage, hello("0"))
	readTypedWithin(t, app2, "welcome", 3*time.Second)
}

// TestCoreModeReconnectAfterStreamingIntoVoid models the agent producing many
// frames while the app is gone (box streaming into the dropped pipe), then a
// reconnect. Exercises select ordering + any write backpressure.
//
// GAP (fidelity): this fake core simply DROPS box→app frames while the app is
// absent. The real CoreRoomDO consumes its shared per-room frame-rate bucket
// BEFORE checking for a peer and closes the sender (the box /c) with 1013 once
// the bucket is exhausted (src/index.ts consumeFrame + webSocketMessage). So a
// box that floods a dead session can lose /c and enter dial backoff — a distinct
// "/c-absent at reconnect" flap mode that is out of scope here and owned by the
// separate client+box resilience follow-on. This test only asserts the box
// re-emits a welcome once /c is healthy, which the fake keeps permanently up.
func TestCoreModeReconnectAfterStreamingIntoVoid(t *testing.T) {
	const deviceID = "dev-af31fd290542"
	h := newSessionHarness(t, deviceID)
	ts, _ := newFakeCore(t)
	boxConn := dialSide(t, ts, "c")
	ctx, cancel := context.WithCancel(context.Background())
	boxDone := make(chan struct{})
	go func() { defer close(boxDone); h.srv.serveV2Conn(ctx, boxConn, h.room) }()
	t.Cleanup(func() { cancel(); _ = boxConn.Close(); <-boxDone })

	hello := func(rf string) []byte {
		b, _ := json.Marshal(map[string]any{"t": "hello", "v": ProtocolVersion, "device_id": deviceID, "secret": h.secret, "resume_from": rf})
		return b
	}
	app1 := dialSide(t, ts, "a")
	_ = app1.WriteMessage(websocket.TextMessage, hello("0"))
	readTypedWithin(t, app1, "welcome", 3*time.Second)
	_ = app1.Close()
	time.Sleep(150 * time.Millisecond)

	// Agent produces a burst while nobody is attached.
	var last MailboxItem
	for i := 0; i < 300; i++ {
		it, err := h.srv.mailbox.enqueue(deviceID, mustDec(i+1), []byte(`{"t":"msg","seq":1,"id":"a-x","text":"burst"}`))
		if err != nil {
			t.Fatalf("enqueue %d: %v", i, err)
		}
		last = it
	}
	time.Sleep(150 * time.Millisecond)

	app2 := dialSide(t, ts, "a")
	_ = app2.WriteMessage(websocket.TextMessage, hello("0"))
	readTypedWithin(t, app2, "welcome", 3*time.Second)
	_ = last
}

func mustDec(i int) string {
	b, _ := json.Marshal(i)
	return string(b)
}

// TestReconnectProvisionsMailboxWhenMissing is the B1 root-cause regression: a
// device that is ACTIVE in the relay store but whose mailbox record is not
// currently provisioned (e.g. the mailbox file was reset/desynced from the
// relay-state across a box restart) must still receive a welcome on reconnect.
// Before the fix, provisioning was gated on the first-link (`linked`) branch, so
// a reconnecting device (VerifyActive, linked=false) skipped provisioning, hit
// "mailbox unavailable", and could never self-heal — it flapped forever.
func TestReconnectProvisionsMailboxWhenMissing(t *testing.T) {
	const deviceID = "dev-af31fd290542"
	h := newSessionHarness(t, deviceID)

	// Simulate the desync: device is active in the store, but its mailbox record
	// is absent (a fresh/reset mailbox that never re-provisioned this device).
	h.srv.mailbox.mu.Lock()
	delete(h.srv.mailbox.disk.Devices, deviceID)
	h.srv.mailbox.mu.Unlock()

	conn := h.dial()
	defer conn.Close()
	hello := mustMarshal(map[string]any{
		"t": "hello", "v": ProtocolVersion, "device_id": deviceID,
		"secret": h.secret, "resume_from": "0",
	})
	writeRaw(t, conn, hello)

	raw := readRawAny(t, conn)
	typ, _ := exactType(raw)
	if typ == "error" {
		t.Fatalf("reconnect to active device with missing mailbox returned error: %s", raw)
	}
	if typ != "welcome" {
		t.Fatalf("reconnect response type = %q, want welcome (raw=%s)", typ, raw)
	}
}

// TestProvisionRollsBackOnSaveFailure covers SHOULD-FIX 2: when the seed persist
// fails, provision must NOT leave a phantom in-memory record that later reports
// "already provisioned" (welcoming a client onto volatile state). After a failed
// provision the device must be absent, so the next attempt retries the save.
func TestProvisionRollsBackOnSaveFailure(t *testing.T) {
	dir := t.TempDir()
	m, err := newLocalMailbox(filepath.Join(dir, "mailboxes.json"))
	if err != nil {
		t.Fatal(err)
	}
	// Force saveLocked to fail: point the path's parent at a regular file so
	// MkdirAll(filepath.Dir(path)) errors.
	blocker := filepath.Join(dir, "blocker")
	if err := os.WriteFile(blocker, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	m.path = filepath.Join(blocker, "sub", "mailboxes.json")

	const deviceID = "dev-af31fd290542"
	seedCalls := 0
	err = m.provision(deviceID, func() []journalFrame { seedCalls++; return nil })
	if err == nil {
		t.Fatal("provision succeeded despite unwritable path")
	}
	m.mu.Lock()
	_, present := m.disk.Devices[deviceID]
	m.mu.Unlock()
	if present {
		t.Fatal("failed provision left a phantom in-memory mailbox record")
	}
	if seedCalls != 1 {
		t.Fatalf("seed loader called %d times, want 1 (lazy read on actual provision)", seedCalls)
	}

	// A second attempt against a good path must succeed (the record was rolled
	// back, so the existence check does not short-circuit).
	m.path = filepath.Join(dir, "mailboxes.json")
	if err := m.provision(deviceID, func() []journalFrame { return nil }); err != nil {
		t.Fatalf("retry provision failed: %v", err)
	}
	m.mu.Lock()
	_, present = m.disk.Devices[deviceID]
	m.mu.Unlock()
	if !present {
		t.Fatal("retry provision did not create the mailbox")
	}
}

// TestProvisionSeedLazyOnResident covers SHOULD-FIX 1: a provision call for an
// already-resident mailbox must NOT invoke the (expensive) seed loader.
func TestProvisionSeedLazyOnResident(t *testing.T) {
	dir := t.TempDir()
	m, err := newLocalMailbox(filepath.Join(dir, "mailboxes.json"))
	if err != nil {
		t.Fatal(err)
	}
	const deviceID = "dev-af31fd290542"
	if err := m.provision(deviceID, func() []journalFrame { return nil }); err != nil {
		t.Fatal(err)
	}
	seedCalls := 0
	if err := m.provision(deviceID, func() []journalFrame { seedCalls++; return nil }); err != nil {
		t.Fatal(err)
	}
	if seedCalls != 0 {
		t.Fatalf("seed loader called %d times on a resident mailbox, want 0", seedCalls)
	}
}
