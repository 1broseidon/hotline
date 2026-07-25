package app

import (
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"sync"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/transcript"
)

// pathRecorder is a tiny concurrent-safe record of the request paths a test
// server saw, so a background maybePush goroutine can be observed deterministically.
type pathRecorder struct {
	mu    sync.Mutex
	paths []string
	hit   chan string
}

func newPathRecorder() *pathRecorder { return &pathRecorder{hit: make(chan string, 8)} }

func (r *pathRecorder) record(p string) {
	r.mu.Lock()
	r.paths = append(r.paths, p)
	r.mu.Unlock()
	select {
	case r.hit <- p:
	default:
	}
}

func (r *pathRecorder) count() int {
	r.mu.Lock()
	defer r.mu.Unlock()
	return len(r.paths)
}

// waitHit blocks until the next request lands (or fails the test on timeout).
func (r *pathRecorder) waitHit(t *testing.T) string {
	t.Helper()
	select {
	case p := <-r.hit:
		return p
	case <-time.After(2 * time.Second):
		t.Fatal("timed out waiting for a request")
		return ""
	}
}

// coreModeServer builds a core-mode Server whose coreClient points at coreURL and
// whose legacy Expo path points at expoURL, with one active device carrying an
// Expo push token in the current room. Returns the server, device id, and room id.
func coreModeServer(t *testing.T, coreURL, expoURL string) (*Server, string, string) {
	t.Helper()
	dir := t.TempDir()
	cfg := &config.Config{
		StateDir:       dir,
		AccessFile:     filepath.Join(dir, "access.json"),
		TranscriptFile: filepath.Join(dir, "transcript.jsonl"),
		CoreMode:       true,
		CoreURL:        coreURL,
	}
	srv := NewServer(cfg, transcript.New(cfg.TranscriptFile))
	if srv.initErr != nil {
		t.Fatal(srv.initErr)
	}
	t.Cleanup(func() { srv.outbox.close() })
	if srv.coreClient == nil {
		t.Fatal("core client not constructed in core mode")
	}
	srv.pushEndpoint = expoURL

	roomID := fixtureRoom
	deviceID := "dev-af31fd290542"
	room := RoomRecord{ID: roomID, URL: "ws://fixture.invalid", Name: "pi", SecretHash: secretHash(fixtureSecret), Secret: fixtureSecret, Envelope: true}
	srv.store.st.Rooms[roomID] = room
	srv.store.st.CurrentRoom = roomID
	srv.store.st.Devices[deviceID] = DeviceRecord{ID: deviceID, Room: roomID, SecretHash: room.SecretHash, State: DeviceActive}
	if err := srv.store.SetPush(deviceID, "ExponentPushToken[abc123]", "ios"); err != nil {
		t.Fatal(err)
	}
	return srv, deviceID, roomID
}

func pushItem() MailboxItem {
	return MailboxItem{Payload: []byte(`{"t":"msg","id":"a-1","text":"ping"}`)}
}

// TestMaybePushCoreRegisteredRoomWakes proves a core-REGISTERED room takes the
// signed wake-hint transport (B1: the wake path fires only for registered rooms).
func TestMaybePushCoreRegisteredRoomWakes(t *testing.T) {
	rec := newPathRecorder()
	core := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		rec.record(r.URL.Path)
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte(`{"pushed":true}`))
	}))
	defer core.Close()
	expo := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		rec.record("EXPO:" + r.URL.Path)
		w.WriteHeader(http.StatusOK)
	}))
	defer expo.Close()

	srv, deviceID, roomID := coreModeServer(t, core.URL, expo.URL)
	// Mark the room registered — ensureRegistered's success state.
	srv.regMu.Lock()
	srv.registered[roomID] = true
	srv.regMu.Unlock()

	srv.maybePush(deviceID, pushItem())
	got := rec.waitHit(t)
	if got != "/v1/rooms/"+roomID+"/wake" {
		t.Fatalf("registered room did not take the wake path, hit %q", got)
	}
}

// TestMaybePushPlaintextRoomInCoreModeUsesLegacyExpo proves an UNregistered
// (plaintext / secretless) room in core mode falls through to the legacy
// Expo-direct path instead of a wake that would 404 (B1 core fix).
func TestMaybePushPlaintextRoomInCoreModeUsesLegacyExpo(t *testing.T) {
	rec := newPathRecorder()
	core := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		rec.record("WAKE:" + r.URL.Path)
		w.WriteHeader(http.StatusNotFound)
		_, _ = w.Write([]byte(`{"code":"unknown_room"}`))
	}))
	defer core.Close()
	expo := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		rec.record("EXPO:" + r.URL.Path)
		w.WriteHeader(http.StatusOK)
	}))
	defer expo.Close()

	srv, deviceID, _ := coreModeServer(t, core.URL, expo.URL)
	// Room is NOT registered (default) — a plaintext room the core never knew.

	srv.maybePush(deviceID, pushItem())
	got := rec.waitHit(t)
	if got != "EXPO:/" {
		t.Fatalf("plaintext core-mode room did not fall back to legacy Expo, hit %q", got)
	}
}

// TestWake404DoesNotKillFuturePushes proves a wake that 404s (or any non-410
// failure) does NOT drop the device token, so the next message still wakes the
// same room (B1: a transient/unknown-room reply is not terminal).
func TestWake404DoesNotKillFuturePushes(t *testing.T) {
	rec := newPathRecorder()
	core := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		rec.record(r.URL.Path)
		w.WriteHeader(http.StatusNotFound)
		_, _ = w.Write([]byte(`{"code":"unknown_room"}`))
	}))
	defer core.Close()

	srv, deviceID, roomID := coreModeServer(t, core.URL, "")
	srv.regMu.Lock()
	srv.registered[roomID] = true
	srv.regMu.Unlock()

	// First wake 404s.
	srv.sendWakeHint(deviceID, roomID, "")
	// The token must survive a 404 (only a 410 token_invalid drops it).
	if _, _, _, ok := srv.store.ActivePushTarget(deviceID); !ok {
		t.Fatal("wake 404 dropped the push token; future pushes for this room are dead")
	}
	// A second wake still fires for the same room.
	srv.sendWakeHint(deviceID, roomID, "")
	if n := rec.count(); n != 2 {
		t.Fatalf("expected 2 wake attempts, got %d", n)
	}
}

// TestCoreModeIgnoresPushEndpoint proves HOTLINE_PUSH_ENDPOINT is ignored in core
// mode (SPEC §5, S1): even with a gateway endpoint configured, the gateway is off,
// the app's Expo token is accepted (not APNs-hex validated), and a registered
// room still takes the wake path rather than the gateway.
func TestCoreModeIgnoresPushEndpoint(t *testing.T) {
	rec := newPathRecorder()
	core := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		rec.record(r.URL.Path)
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte(`{"pushed":true}`))
	}))
	defer core.Close()

	dir := t.TempDir()
	cfg := &config.Config{
		StateDir:        dir,
		AccessFile:      filepath.Join(dir, "access.json"),
		TranscriptFile:  filepath.Join(dir, "transcript.jsonl"),
		AppPushEndpoint: "https://gateway.hotline.dev", // set alongside core mode
		CoreMode:        true,
		CoreURL:         core.URL,
	}
	srv := NewServer(cfg, transcript.New(cfg.TranscriptFile))
	if srv.initErr != nil {
		t.Fatal(srv.initErr)
	}
	t.Cleanup(func() { srv.outbox.close() })

	// Gateway must be OFF in core mode even though the endpoint (and signer) exist.
	if srv.gatewayEnabled() {
		t.Fatal("gateway enabled in core mode; HOTLINE_PUSH_ENDPOINT not ignored")
	}
	// The app's Expo token must be accepted, not rejected by APNs-hex validation.
	if !srv.acceptPushToken("ExponentPushToken[abc123]") {
		t.Fatal("Expo token rejected in core mode; token acceptance not treating endpoint as unset")
	}
	if srv.acceptPushToken("deadbeef") {
		t.Fatal("APNs-hex token accepted in core mode; gateway validation still active")
	}

	// A registered room still wakes (endpoint ignored, not gateway-pushed).
	roomID := fixtureRoom
	deviceID := "dev-af31fd290542"
	room := RoomRecord{ID: roomID, URL: "ws://fixture.invalid", Name: "pi", SecretHash: secretHash(fixtureSecret), Secret: fixtureSecret, Envelope: true}
	srv.store.st.Rooms[roomID] = room
	srv.store.st.CurrentRoom = roomID
	srv.store.st.Devices[deviceID] = DeviceRecord{ID: deviceID, Room: roomID, SecretHash: room.SecretHash, State: DeviceActive}
	if err := srv.store.SetPush(deviceID, "ExponentPushToken[abc123]", "ios"); err != nil {
		t.Fatal(err)
	}
	srv.regMu.Lock()
	srv.registered[roomID] = true
	srv.regMu.Unlock()

	srv.maybePush(deviceID, pushItem())
	got := rec.waitHit(t)
	if got != "/v1/rooms/"+roomID+"/wake" {
		t.Fatalf("registered room in core mode did not wake, hit %q", got)
	}
}
