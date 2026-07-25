package app

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
	"time"

	"github.com/gorilla/websocket"
)

// This file pins the three P3 presence-hardening fixes from sol's xhigh review
// (committed as dee46f6) so a later refactor cannot silently revert them:
//
//  1. parsePing: a ping with a missing/null or negative `n` is a bad_frame in
//     BOTH the gap phase and steady state — no pong, no presence-lease refresh,
//     counted toward the bad-frame abuse tally.
//  2. waitGapAck applies the same 3-bad-frame -> close(4002,"bad_frame") abuse
//     accounting as steady state, and its (sessionResult, bool) return still
//     propagates restart / disconnect outcomes.
//  3. RelayStore.ActivePushTarget returns (token, room, ok) atomically and only
//     for a device that is active AND bound to the current room, so a rotated
//     room can never suppress/allow a push against a stale binding.

// pingErrDetail is the shared bad-frame detail both ping handlers emit.
const pingErrDetail = "ping.n must be a non-negative integer"

// expectCloseCode reads frames (skipping the agent_state snapshot and any
// error frames) until the peer sends a websocket close, then asserts its code.
func expectCloseCode(t *testing.T, conn *websocket.Conn, code int) {
	t.Helper()
	for {
		_ = conn.SetReadDeadline(time.Now().Add(3 * time.Second))
		_, raw, err := conn.ReadMessage()
		if err != nil {
			ce, ok := err.(*websocket.CloseError)
			if !ok || ce.Code != code {
				t.Fatalf("close = %T %v, want close code %d", err, err, code)
			}
			return
		}
		if typ, exact := exactType(raw); exact && (typ == "agent_state" || typ == "error") {
			continue
		}
		t.Fatalf("unexpected frame before close: %s", raw)
	}
}

// assertBadFrame decodes raw and fails unless it is an error/bad_frame frame
// (and specifically NOT a pong).
func assertBadFrame(t *testing.T, raw []byte) {
	t.Helper()
	var e struct {
		T    string `json:"t"`
		Code string `json:"code"`
	}
	if err := json.Unmarshal(raw, &e); err != nil {
		t.Fatalf("decode frame %s: %v", raw, err)
	}
	if e.T == "pong" {
		t.Fatalf("bad ping was ponged: %s", raw)
	}
	if e.T != "error" || e.Code != "bad_frame" {
		t.Fatalf("frame = %s, want error/bad_frame", raw)
	}
}

// assertPong decodes raw and fails unless it is a pong echoing wantN.
func assertPong(t *testing.T, raw []byte, wantN int64) {
	t.Helper()
	var p struct {
		T string `json:"t"`
		N int64  `json:"n"`
	}
	if err := json.Unmarshal(raw, &p); err != nil {
		t.Fatalf("decode frame %s: %v", raw, err)
	}
	if p.T != "pong" || p.N != wantN {
		t.Fatalf("frame = %s, want pong n=%d", raw, wantN)
	}
}

// TestParsePingValidation pins the shared validator at the discriminator: a
// present, non-negative integer `n` is required. Missing (would silently decode
// to 0) and negative are rejected; extra fields are ignored.
func TestParsePingValidation(t *testing.T) {
	table := []struct {
		name  string
		raw   string
		wantN int64
		ok    bool
	}{
		{"missing n", `{"t":"ping"}`, 0, false},
		{"null n", `{"t":"ping","n":null}`, 0, false},
		{"negative n", `{"t":"ping","n":-1}`, 0, false},
		{"negative large", `{"t":"ping","n":-9007199254740993}`, 0, false},
		{"zero n", `{"t":"ping","n":0}`, 0, true},
		{"positive n", `{"t":"ping","n":42}`, 42, true},
		{"positive with extra fields", `{"t":"ping","n":7,"junk":true}`, 7, true},
		{"garbage json", `{"t":"ping","n":`, 0, false},
	}
	for _, tc := range table {
		t.Run(tc.name, func(t *testing.T) {
			n, ok := parsePing([]byte(tc.raw))
			if ok != tc.ok || (ok && n != tc.wantN) {
				t.Fatalf("parsePing(%s) = (%d,%v), want (%d,%v)", tc.raw, n, ok, tc.wantN, tc.ok)
			}
		})
	}
}

// fakeClockHarness wires a settable clock into the mailbox before any session
// starts, so lease-expiry is deterministic. Returns the atomic nanos pointer.
func fakeClockHarness(t *testing.T, deviceID string, base time.Time) (*sessionHarness, *int64) {
	t.Helper()
	h := newSessionHarness(t, deviceID)
	nanos := new(int64)
	atomic.StoreInt64(nanos, base.UnixNano())
	h.srv.mailbox.clock = func() time.Time { return time.Unix(0, atomic.LoadInt64(nanos)) }
	return h, nanos
}

// TestPingValidationSteadyState pins fix (1) on the steady-state input loop: a
// bad ping is a bad_frame that neither pongs NOR refreshes the foreground lease
// (the device stays away-gated); a valid ping pongs and refreshes.
func TestPingValidationSteadyState(t *testing.T) {
	const deviceID = "dev-af31fd290542"
	base := time.Unix(2_000_000, 0)
	h, nanos := fakeClockHarness(t, deviceID, base)

	conn := h.dial()
	defer conn.Close()
	writeRaw(t, conn, []byte(`{"t":"hello","v":2,"device_id":"`+deviceID+`","secret":"`+h.secret+`","resume_from":"0"}`))
	if typ, _ := exactType(readRaw(t, conn)); typ != "welcome" {
		t.Fatal("first frame must be welcome")
	}

	// Advance past the 60s lease: with no refresh the device is away.
	atomic.StoreInt64(nanos, base.Add(61*time.Second).UnixNano())
	if !h.srv.mailbox.deviceAway(deviceID) {
		t.Fatal("device should be away once the lease expires")
	}

	// Two bad pings (missing n, then negative n): each is a bad_frame, is not
	// ponged, and does NOT refresh the lease. Two stays under the abuse limit.
	for _, bad := range []string{`{"t":"ping"}`, `{"t":"ping","n":-5}`} {
		writeRaw(t, conn, []byte(bad))
		assertBadFrame(t, readRaw(t, conn))
		if !h.srv.mailbox.deviceAway(deviceID) {
			t.Fatalf("bad ping %s refreshed the presence lease", bad)
		}
	}

	// A valid ping pongs and refreshes the lease at the current (post-expiry)
	// clock, so the device is present again.
	writeRaw(t, conn, []byte(`{"t":"ping","n":5}`))
	assertPong(t, readRaw(t, conn), 5)
	if h.srv.mailbox.deviceAway(deviceID) {
		t.Fatal("valid ping did not refresh the presence lease")
	}
}

// TestPingValidationGapPhase pins fix (1) on the mailbox-gap input loop
// (waitGapAck): identical no-pong / no-refresh behavior for bad pings and a
// pong + refresh for a valid ping, all before the gap ack completes.
func TestPingValidationGapPhase(t *testing.T) {
	const deviceID = "dev-af31fd290542"
	base := time.Unix(2_000_000, 0)
	h, nanos := fakeClockHarness(t, deviceID, base)
	mb := h.srv.mailbox.disk.Devices[deviceID]
	mb.Floor, mb.Head, mb.Ack = "9007199254741050", "9007199254741050", "9007199254741050"

	conn := h.dial()
	defer conn.Close()
	// resume_from below floor -> welcome then mailbox_gap (gap phase).
	writeRaw(t, conn, []byte(`{"t":"hello","v":2,"device_id":"`+deviceID+`","secret":"`+h.secret+`","resume_from":"0"}`))
	if typ, _ := exactType(readRaw(t, conn)); typ != "welcome" {
		t.Fatal("first frame must be welcome")
	}
	if typ, _ := exactType(readRaw(t, conn)); typ != "mailbox_gap" {
		t.Fatal("second frame must be mailbox_gap")
	}

	atomic.StoreInt64(nanos, base.Add(61*time.Second).UnixNano())
	if !h.srv.mailbox.deviceAway(deviceID) {
		t.Fatal("device should be away once the lease expires")
	}

	for _, bad := range []string{`{"t":"ping","n":null}`, `{"t":"ping","n":-1}`} {
		writeRaw(t, conn, []byte(bad))
		assertBadFrame(t, readRaw(t, conn))
		if !h.srv.mailbox.deviceAway(deviceID) {
			t.Fatalf("gap-phase bad ping %s refreshed the presence lease", bad)
		}
	}

	writeRaw(t, conn, []byte(`{"t":"ping","n":9}`))
	assertPong(t, readRaw(t, conn), 9)
	if h.srv.mailbox.deviceAway(deviceID) {
		t.Fatal("gap-phase valid ping did not refresh the presence lease")
	}
}

// TestPingBadFrameAbuseCloses pins that a bad ping counts toward the shared
// 3-bad-frame abuse tally in BOTH phases: the third bad ping closes the session
// with 4002 "bad_frame". The gap-phase half is the core of fix (2): the gap
// loop now matches steady state instead of ignoring malformed pings.
func TestPingBadFrameAbuseCloses(t *testing.T) {
	const deviceID = "dev-af31fd290542"

	t.Run("steady state", func(t *testing.T) {
		h := newSessionHarness(t, deviceID)
		conn := h.dial()
		defer conn.Close()
		writeRaw(t, conn, []byte(`{"t":"hello","v":2,"device_id":"`+deviceID+`","secret":"`+h.secret+`","resume_from":"0"}`))
		if typ, _ := exactType(readRaw(t, conn)); typ != "welcome" {
			t.Fatal("first frame must be welcome")
		}
		for i := 0; i < 3; i++ {
			writeRaw(t, conn, []byte(`{"t":"ping"}`))
		}
		expectCloseCode(t, conn, 4002)
	})

	t.Run("gap phase", func(t *testing.T) {
		h := newSessionHarness(t, deviceID)
		mb := h.srv.mailbox.disk.Devices[deviceID]
		mb.Floor, mb.Head, mb.Ack = "9007199254741050", "9007199254741050", "9007199254741050"
		conn := h.dial()
		defer conn.Close()
		writeRaw(t, conn, []byte(`{"t":"hello","v":2,"device_id":"`+deviceID+`","secret":"`+h.secret+`","resume_from":"0"}`))
		if typ, _ := exactType(readRaw(t, conn)); typ != "welcome" {
			t.Fatal("first frame must be welcome")
		}
		if typ, _ := exactType(readRaw(t, conn)); typ != "mailbox_gap" {
			t.Fatal("second frame must be mailbox_gap")
		}
		for i := 0; i < 3; i++ {
			writeRaw(t, conn, []byte(`{"t":"ping","n":-1}`))
		}
		expectCloseCode(t, conn, 4002)
	})
}

// TestGapPhaseRestartAndDisconnectPropagate pins the other two outcomes of
// waitGapAck's new (sessionResult, bool) return: a second hello during the gap
// restarts the session (fresh welcome), and a disconnect during the gap tears
// the session down (its subscriber is removed).
func TestGapPhaseRestartAndDisconnectPropagate(t *testing.T) {
	const deviceID = "dev-af31fd290542"

	seedGap := func(h *sessionHarness) {
		mb := h.srv.mailbox.disk.Devices[deviceID]
		mb.Floor, mb.Head, mb.Ack = "9007199254741050", "9007199254741050", "9007199254741050"
	}
	enterGap := func(t *testing.T, h *sessionHarness) *websocket.Conn {
		conn := h.dial()
		writeRaw(t, conn, []byte(`{"t":"hello","v":2,"device_id":"`+deviceID+`","secret":"`+h.secret+`","resume_from":"0"}`))
		if typ, _ := exactType(readRaw(t, conn)); typ != "welcome" {
			t.Fatal("first frame must be welcome")
		}
		if typ, _ := exactType(readRaw(t, conn)); typ != "mailbox_gap" {
			t.Fatal("second frame must be mailbox_gap")
		}
		return conn
	}

	t.Run("restart via second hello", func(t *testing.T) {
		h := newSessionHarness(t, deviceID)
		seedGap(h)
		conn := enterGap(t, h)
		defer conn.Close()
		// A hello mid-gap must restart the session, not be treated as a bad frame.
		writeRaw(t, conn, []byte(`{"t":"hello","v":2,"device_id":"`+deviceID+`","secret":"`+h.secret+`","resume_from":"0"}`))
		if typ, _ := exactType(readRaw(t, conn)); typ != "welcome" {
			t.Fatal("second hello during gap did not restart the session")
		}
	})

	t.Run("disconnect tears down the session", func(t *testing.T) {
		h := newSessionHarness(t, deviceID)
		seedGap(h)
		conn := enterGap(t, h)
		_ = conn.Close()
		// The gap-phase disconnect must propagate out of waitGapAck so the
		// deferred unsubscribe runs and the subscriber is removed.
		waitUntil(t, 2*time.Second, func() bool {
			h.srv.mailbox.mu.Lock()
			defer h.srv.mailbox.mu.Unlock()
			return len(h.srv.mailbox.subs[deviceID]) == 0
		})
	})
}

// TestActivePushTargetAtomicSnapshot pins fix (3): the atomic push-target
// snapshot returns (token, currentRoom, true) only for a device that is active
// AND bound to the current room, and false after a room rotation/unbind or when
// the current room is unknown.
func TestActivePushTargetAtomicSnapshot(t *testing.T) {
	const (
		deviceID = "dev-af31fd290542"
		roomA    = "Ab3dEf6hIj8lMn0pQr2tUv"
		roomB    = "Zz9yXx7wVv5uTt3sRr1qPo"
		token    = "ExponentPushToken[atomic-target]"
	)
	s, err := OpenRelayStore(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	// seed mutates state under lock and persists, so the reloadLocked() inside
	// ActivePushTarget observes exactly what we set.
	seed := func(mutate func()) {
		s.mu.Lock()
		defer s.mu.Unlock()
		mutate()
		if err := s.saveLocked(); err != nil {
			t.Fatal(err)
		}
	}

	seed(func() {
		s.st.Rooms = map[string]RoomRecord{roomA: {ID: roomA}}
		s.st.CurrentRoom = roomA
		s.st.Devices = map[string]DeviceRecord{deviceID: {ID: deviceID, Room: roomA, State: DeviceActive, PushToken: token}}
	})

	// Active and bound to the current room -> ok, with the current room echoed.
	if tok, _, room, ok := s.ActivePushTarget(deviceID); !ok || tok != token || room != roomA {
		t.Fatalf("active+bound = (%q,%q,%v), want (%q,%q,true)", tok, room, ok, token, roomA)
	}

	// Unknown device -> not ok.
	if _, _, _, ok := s.ActivePushTarget("dev-000000000000"); ok {
		t.Fatal("unknown device should not be a push target")
	}

	// Room rotation: current room moves to B while the device stays bound to A.
	seed(func() {
		s.st.Rooms = map[string]RoomRecord{roomB: {ID: roomB}}
		s.st.CurrentRoom = roomB
	})
	if _, _, _, ok := s.ActivePushTarget(deviceID); ok {
		t.Fatal("device bound to the old room must not be a push target after rotation")
	}

	// Re-bind the device to the current room but leave it unbound (inactive).
	seed(func() {
		d := s.st.Devices[deviceID]
		d.Room = roomB
		d.State = DeviceUnbound
		s.st.Devices[deviceID] = d
	})
	if _, _, _, ok := s.ActivePushTarget(deviceID); ok {
		t.Fatal("unbound device must not be a push target")
	}

	// The device's own bound room going dead (a `relay revoke`) -> not ok. Under
	// the multi-device model push resolves via the device's own room (MD3), so a
	// deaded room, not a moved current_room, is what suppresses the target.
	seed(func() {
		d := s.st.Devices[deviceID]
		d.Room = roomB
		d.State = DeviceActive
		s.st.Devices[deviceID] = d
		r := s.st.Rooms[roomB]
		r.State = RoomDead
		s.st.Rooms[roomB] = r
	})
	if _, _, _, ok := s.ActivePushTarget(deviceID); ok {
		t.Fatal("a device whose bound room is dead must not yield a push target")
	}
}

// TestMaybePushSuppressedWhenNotActivePushTarget pins that maybePush's push /
// suppression follows exactly from ActivePushTarget's ok: an active+bound
// device pushes, and after a room rotation (device unbound) the same call
// suppresses the push.
func TestMaybePushSuppressedWhenNotActivePushTarget(t *testing.T) {
	const deviceID = "dev-af31fd290542"
	h := newSessionHarness(t, deviceID)
	token := "ExponentPushToken[maybe-push]"
	if err := h.srv.store.SetPush(deviceID, token, "ios"); err != nil {
		t.Fatal(err)
	}
	got := make(chan map[string]any, 4)
	expo := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var body map[string]any
		_ = json.NewDecoder(r.Body).Decode(&body)
		got <- body
		w.WriteHeader(http.StatusOK)
	}))
	defer expo.Close()
	h.srv.pushEndpoint = expo.URL

	item := MailboxItem{T: "msg", Payload: json.RawMessage(`{"t":"msg","id":"a-1","text":"ping me"}`)}

	// Active and bound: maybePush sends, carrying the current room.
	h.srv.maybePush(deviceID, item)
	select {
	case body := <-got:
		if body["to"] != token {
			t.Fatalf("push to = %v, want %s", body["to"], token)
		}
		data, _ := body["data"].(map[string]any)
		if data["room"] != h.room.ID {
			t.Fatalf("push data room = %v, want %s", data["room"], h.room.ID)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("active+bound device did not push")
	}

	// --rotate-all unbinds every existing device (an additive new-link would keep
	// this device live — that's the whole point of MD1). ActivePushTarget now
	// reports ok=false, so maybePush must suppress.
	if _, err := h.srv.store.RotateAll("ws://fixture.invalid", "pi", false); err != nil {
		t.Fatal(err)
	}
	if _, _, _, ok := h.srv.store.ActivePushTarget(deviceID); ok {
		t.Fatal("device should not be an active push target after rotation")
	}
	h.srv.maybePush(deviceID, item)
	select {
	case body := <-got:
		t.Fatalf("push fired after room rotation: %v", body)
	case <-time.After(200 * time.Millisecond):
	}
}
