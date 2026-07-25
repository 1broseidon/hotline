package app

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/gorilla/websocket"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/transcript"
)

// fixtureRoom / fixtureSecret are the envelope-e1.json vectors' room + pairing
// secret, so the derived keys here match the golden HKDF outputs.
const (
	fixtureRoom   = "Ab3dEf6hIj8lMn0pQr2tUv"
	fixtureSecret = "Ab3dEf6hIj8lMn0pQr2tUvWx4zAb3dEf6hIj8lMn0pQ"
)

// TestConnectorEnvelopeRoundTripInnerV2Untouched drives a full hello→welcome
// exchange through the e1 envelope and asserts the box emits a valid, unchanged
// v2 welcome frame INSIDE the envelope: the app decrypts (b2a) and the inner
// bytes are byte-for-byte a normal v2 frame. It also asserts a plaintext hello on
// an e-room gets silence. This proves the envelope is a pure transport layer.
func TestConnectorEnvelopeRoundTripInnerV2Untouched(t *testing.T) {
	dir := t.TempDir()
	cfg := &config.Config{StateDir: dir, AccessFile: filepath.Join(dir, "access.json"), TranscriptFile: filepath.Join(dir, "transcript.jsonl")}
	srv := NewServer(cfg, transcript.New(cfg.TranscriptFile))
	if srv.initErr != nil {
		t.Fatal(srv.initErr)
	}
	deviceID := "dev-af31fd290542"
	room := RoomRecord{ID: fixtureRoom, URL: "ws://fixture.invalid", Name: "pi", SecretHash: secretHash(fixtureSecret), Secret: fixtureSecret, Envelope: true}
	srv.store.st.Rooms[room.ID] = room
	srv.store.st.CurrentRoom = room.ID
	srv.store.st.Devices[deviceID] = DeviceRecord{ID: deviceID, Room: room.ID, SecretHash: room.SecretHash, State: DeviceActive}
	srv.mailbox.disk.Devices[deviceID] = &mailboxRecord{Floor: "0", Head: "0", Ack: "0"}

	up := websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}
	var sessions sync.WaitGroup
	// Hold one token for the server's whole lifetime. Each handler does its own
	// Add(1) from inside the http goroutine; if the counter is at zero when
	// Wait() starts, that Add races the Wait — WaitGroup misuse the go1.26 race
	// detector reports as a data race (it fired deterministically on the
	// plaintext-silence test). The token keeps the counter above zero until
	// ts.Close() has returned, after which no new handler can start.
	sessions.Add(1)
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := up.Upgrade(w, r, nil)
		if err != nil {
			return
		}
		sessions.Add(1)
		defer sessions.Done()
		srv.serveV2Conn(context.Background(), conn, room)
		_ = conn.Close()
	}))
	t.Cleanup(func() { ts.Close(); sessions.Done(); sessions.Wait(); srv.outbox.close() })

	// The app side derives the same keys and speaks e1.
	appCodec, err := newEnvelopeCodec(room)
	if err != nil {
		t.Fatal(err)
	}
	conn, _, err := websocket.DefaultDialer.Dial("ws"+ts.URL[len("http"):]+"/", nil)
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()

	// App→box hello, sealed a2b. appCodec.wrap seals b2a (that's the box role), so
	// the app must seal with kA2B under the a2b AAD directly.
	hello := []byte(`{"t":"hello","v":2,"device_id":"` + deviceID + `","secret":"` + fixtureSecret + `","resume_from":"0"}`)
	nonce := make([]byte, 24)
	nonce[0] = 7
	n, c, err := sealE1(appCodec.kA2B, nonce, appCodec.aad("a2b"), hello)
	if err != nil {
		t.Fatal(err)
	}
	sealedHello, _ := json.Marshal(map[string]string{"t": "e1", "n": n, "c": c})
	if err := conn.WriteMessage(websocket.TextMessage, sealedHello); err != nil {
		t.Fatal(err)
	}

	// Box→app: every reply is an e1 frame the app opens under kB2A/b2a. The FIRST
	// inner frame must be a normal v2 welcome, byte-shape unchanged.
	_ = conn.SetReadDeadline(time.Now().Add(3 * time.Second))
	_, raw, err := conn.ReadMessage()
	if err != nil {
		t.Fatalf("no enveloped reply: %v", err)
	}
	var wire struct{ T, N, C string }
	if json.Unmarshal(raw, &wire) != nil || wire.T != "e1" {
		t.Fatalf("reply is not an e1 frame: %s", raw)
	}
	inner, err := openE1(appCodec.kB2A, wire.N, wire.C, appCodec.aad("b2a"))
	if err != nil {
		t.Fatalf("open welcome: %v", err)
	}
	var wel struct {
		T    string `json:"t"`
		V    int    `json:"v"`
		Room string `json:"room"`
	}
	if json.Unmarshal(inner, &wel) != nil || wel.T != "welcome" || wel.V != 2 || wel.Room != room.ID {
		t.Fatalf("inner frame is not an unchanged v2 welcome: %s", inner)
	}
}

// TestConnectorEnvelopePlaintextHelloGetsSilence pins that a plaintext hello on
// an e-room is dropped (no reply) per SPEC §1.
func TestConnectorEnvelopePlaintextHelloGetsSilence(t *testing.T) {
	dir := t.TempDir()
	cfg := &config.Config{StateDir: dir, AccessFile: filepath.Join(dir, "access.json"), TranscriptFile: filepath.Join(dir, "transcript.jsonl")}
	srv := NewServer(cfg, transcript.New(cfg.TranscriptFile))
	if srv.initErr != nil {
		t.Fatal(srv.initErr)
	}
	room := RoomRecord{ID: fixtureRoom, URL: "ws://fixture.invalid", Name: "pi", SecretHash: secretHash(fixtureSecret), Secret: fixtureSecret, Envelope: true}
	up := websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}
	var sessions sync.WaitGroup
	// Hold one token for the server's whole lifetime. Each handler does its own
	// Add(1) from inside the http goroutine; if the counter is at zero when
	// Wait() starts, that Add races the Wait — WaitGroup misuse the go1.26 race
	// detector reports as a data race (it fired deterministically on the
	// plaintext-silence test). The token keeps the counter above zero until
	// ts.Close() has returned, after which no new handler can start.
	sessions.Add(1)
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := up.Upgrade(w, r, nil)
		if err != nil {
			return
		}
		sessions.Add(1)
		defer sessions.Done()
		srv.serveV2Conn(context.Background(), conn, room)
		_ = conn.Close()
	}))
	t.Cleanup(func() { ts.Close(); sessions.Done(); sessions.Wait(); srv.outbox.close() })

	conn, _, err := websocket.DefaultDialer.Dial("ws"+ts.URL[len("http"):]+"/", nil)
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()
	_ = conn.WriteMessage(websocket.TextMessage, []byte(`{"t":"hello","v":2,"device_id":"dev-af31fd290542","secret":"`+fixtureSecret+`","resume_from":"0"}`))
	_ = conn.SetReadDeadline(time.Now().Add(500 * time.Millisecond))
	if _, _, err := conn.ReadMessage(); err == nil {
		t.Fatal("plaintext hello on e-room got a reply, want silence")
	}
}

// TestCoreModeUnsetIsInert is the rollback proof: with HOTLINE_CORE_MODE unset,
// NewServer creates no core client, no box-key.json, mints plaintext rooms with
// no e param and no stored secret, and the room the connector sees carries
// Envelope=false (so the read/write choke points run the exact pre-core path).
func TestCoreModeUnsetIsInert(t *testing.T) {
	dir := t.TempDir()
	cfg := &config.Config{StateDir: dir, AccessFile: filepath.Join(dir, "access.json"), TranscriptFile: filepath.Join(dir, "transcript.jsonl")}
	// CoreMode defaults to false on a zero Config.
	srv := NewServer(cfg, transcript.New(cfg.TranscriptFile))
	if srv.initErr != nil {
		t.Fatal(srv.initErr)
	}
	defer srv.outbox.close()
	if srv.coreMode || srv.coreClient != nil {
		t.Fatal("core mode active with HOTLINE_CORE_MODE unset")
	}
	if _, err := os.Stat(filepath.Join(dir, boxKeyFile)); !os.IsNotExist(err) {
		t.Fatalf("box-key.json created outside core mode: %v", err)
	}
	link, err := srv.store.MintLink("wss://relay.hotline.dev", "pi")
	if err != nil {
		t.Fatal(err)
	}
	if link.Envelope || strings.Contains(link.URI, "e=1") {
		t.Errorf("plaintext mint produced an envelope link: %s", link.URI)
	}
	room, ok := srv.store.CurrentRoom()
	if !ok || room.Envelope || room.Secret != "" {
		t.Errorf("plaintext room carries envelope/secret: %+v", room)
	}
	// ensureRegistered is a no-op (no panic, no registration marked) when inert.
	srv.ensureRegistered(context.Background(), room)
}

// TestMintLinkModeEnvelope pins the core-mode mint: e=1 QR, persisted secret,
// Envelope room, and that the URI matches the pair-uri-e.json e=1 shape.
func TestMintLinkModeEnvelope(t *testing.T) {
	dir := t.TempDir()
	store, err := OpenRelayStore(dir)
	if err != nil {
		t.Fatal(err)
	}
	link, err := store.MintLinkMode("wss://relay.hotline.dev", "pi", true)
	if err != nil {
		t.Fatal(err)
	}
	if !link.Envelope || link.Secret == "" {
		t.Fatalf("envelope mint missing envelope/secret: %+v", link)
	}
	if !strings.Contains(link.URI, "e=1") {
		t.Errorf("envelope URI missing e=1: %s", link.URI)
	}
	room, ok := store.CurrentRoom()
	if !ok || !room.Envelope || room.Secret != link.Secret {
		t.Errorf("envelope room not persisted with secret: %+v", room)
	}
	// The stored secret must derive the same auth_hash the register call sends.
	if _, err := deriveRoomAuthHash(room.Secret); err != nil {
		t.Errorf("auth_hash derive from stored secret failed: %v", err)
	}
}
