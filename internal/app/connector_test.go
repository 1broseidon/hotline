package app

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/gorilla/websocket"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/transcript"
)

type acceptedRelayConn struct {
	path string
	conn *websocket.Conn
}

func TestConnectorHopsToExternallyReplacedRoom(t *testing.T) {
	accepted := make(chan acceptedRelayConn, 4)
	upgrader := websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}
	pipe := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := upgrader.Upgrade(w, r, nil)
		if err != nil {
			return
		}
		accepted <- acceptedRelayConn{path: r.URL.Path, conn: conn}
	}))
	defer pipe.Close()
	pipeURL := "ws" + pipe.URL[len("http"):]

	stateDir := t.TempDir()
	cfg := &config.Config{
		StateDir:       stateDir,
		AccessFile:     filepath.Join(stateDir, "access.json"),
		TranscriptFile: filepath.Join(stateDir, "transcript.jsonl"),
	}
	srv := NewServer(cfg, transcript.New(cfg.TranscriptFile))
	if srv.initErr != nil {
		t.Fatal(srv.initErr)
	}
	defer srv.outbox.close()
	roomA, err := srv.store.MintLink(pipeURL, "room-a")
	if err != nil {
		t.Fatal(err)
	}
	srv.onLinked = func(string) {}

	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { done <- srv.runRoomManager(ctx) }()
	var open []*websocket.Conn
	defer func() {
		cancel()
		for _, conn := range open {
			_ = conn.Close()
		}
		select {
		case err := <-done:
			if err != nil {
				t.Errorf("connector stopped: %v", err)
			}
		case <-time.After(2 * time.Second):
			t.Error("connector did not stop")
		}
	}()

	var first acceptedRelayConn
	select {
	case first = <-accepted:
		open = append(open, first.conn)
	case <-time.After(3 * time.Second):
		t.Fatal("connector did not dial room A")
	}
	if want := "/r/" + roomA.Room + "/c"; first.path != want {
		t.Fatalf("first dial path = %q, want %q", first.path, want)
	}

	externalStore, err := OpenRelayStore(stateDir)
	if err != nil {
		t.Fatal(err)
	}
	// --rotate-all replaces the whole served set: room A is reaped, room B served.
	roomB, err := externalStore.RotateAll(pipeURL, "room-b", false)
	if err != nil {
		t.Fatal(err)
	}

	var second acceptedRelayConn
	select {
	case second = <-accepted:
		open = append(open, second.conn)
	case <-time.After(5 * time.Second):
		t.Fatal("connector did not leave room A and dial room B")
	}
	if want := "/r/" + roomB.Room + "/c"; second.path != want {
		t.Fatalf("second dial path = %q, want %q", second.path, want)
	}
	_ = first.conn.SetReadDeadline(time.Now().Add(time.Second))
	if _, _, err := first.conn.ReadMessage(); err == nil {
		t.Fatal("room-A connection remained open")
	}

	hello := helloFrame{T: "hello", V: ProtocolVersion, DeviceID: "dev-abcdef123456", Secret: roomB.Secret, ResumeFrom: "0"}
	if err := second.conn.WriteJSON(hello); err != nil {
		t.Fatal(err)
	}
	_ = second.conn.SetReadDeadline(time.Now().Add(2 * time.Second))
	_, raw, err := second.conn.ReadMessage()
	if err != nil {
		t.Fatal(err)
	}
	var welcome struct {
		T    string `json:"t"`
		Room string `json:"room"`
	}
	if err := json.Unmarshal(raw, &welcome); err != nil {
		t.Fatal(err)
	}
	if welcome.T != "welcome" || welcome.Room != roomB.Room {
		t.Fatalf("room-B hello response = %s", raw)
	}
}

// TestConnectorRequestsHarnessRecycleOnRotation proves the box-side bug-1 fix:
// when the current room rotates out from under a running connector (a `relay
// new-link` from a separate process against the same state dir), the connector
// asks the supervisor to recycle the harness — once, for the new room, and never
// on the initial bind.
func TestConnectorRequestsHarnessRecycleOnRotation(t *testing.T) {
	upgrader := websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}
	accepted := make(chan *websocket.Conn, 4)
	pipe := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := upgrader.Upgrade(w, r, nil)
		if err != nil {
			return
		}
		accepted <- conn
	}))
	defer pipe.Close()
	pipeURL := "ws" + pipe.URL[len("http"):]

	stateDir := t.TempDir()
	cfg := &config.Config{
		StateDir:       stateDir,
		AccessFile:     filepath.Join(stateDir, "access.json"),
		TranscriptFile: filepath.Join(stateDir, "transcript.jsonl"),
	}
	srv := NewServer(cfg, transcript.New(cfg.TranscriptFile))
	if srv.initErr != nil {
		t.Fatal(srv.initErr)
	}
	defer srv.outbox.close()
	srv.onLinked = func(string) {}

	rotations := make(chan string, 4)
	srv.onRoomRotate = func(newRoomID string) { rotations <- newRoomID }

	if _, err := srv.store.MintLink(pipeURL, "room-a"); err != nil {
		t.Fatal(err)
	}

	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { done <- srv.runRoomManager(ctx) }()
	defer func() {
		cancel()
		select {
		case <-done:
		case <-time.After(2 * time.Second):
			t.Error("connector did not stop")
		}
	}()

	// Initial bind to room A must NOT be reported as a rotation.
	select {
	case c := <-accepted:
		_ = c.Close()
	case <-time.After(3 * time.Second):
		t.Fatal("connector did not dial room A")
	}
	select {
	case id := <-rotations:
		t.Fatalf("initial bind wrongly reported a rotation to %q", id)
	case <-time.After(200 * time.Millisecond):
	}

	// A separate store rotates the room, as `relay new-link` would from its own
	// process.
	external, err := OpenRelayStore(stateDir)
	if err != nil {
		t.Fatal(err)
	}
	roomB, err := external.RotateAll(pipeURL, "room-b", false)
	if err != nil {
		t.Fatal(err)
	}

	select {
	case id := <-rotations:
		if id != roomB.Room {
			t.Fatalf("recycle requested for %q, want new room %q", id, roomB.Room)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("connector did not request a harness recycle on rotation")
	}

	// Exactly one recycle for this rotation (drain any dialed conns meanwhile).
	go func() {
		for {
			select {
			case c := <-accepted:
				_ = c.Close()
			case <-ctx.Done():
				return
			}
		}
	}()
	select {
	case id := <-rotations:
		t.Fatalf("rotation reported more than once (extra %q)", id)
	case <-time.After(time.Second):
	}
}

// TestRequestHarnessRecycleWritesRestartRequest proves the default onRoomRotate
// files the same restart.request control file the supervisor consumes, and is an
// inert no-op when unsupervised (no supervisor dir).
func TestRequestHarnessRecycleWritesRestartRequest(t *testing.T) {
	supDir := t.TempDir()
	srv := &Server{supervisorDir: supDir}
	srv.requestHarnessRecycle("room-xyz")
	if _, err := os.Stat(filepath.Join(supDir, "restart.request")); err != nil {
		t.Fatalf("restart.request not written: %v", err)
	}

	// Unsupervised: no dir, no panic, nothing written.
	(&Server{supervisorDir: ""}).requestHarnessRecycle("room-xyz")
}
