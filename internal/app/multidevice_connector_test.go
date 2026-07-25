package app

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/gorilla/websocket"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/transcript"
)

// TestRoomManagerServesReapsAndNeverRecyclesOnAddOrRevoke drives the MD2 room
// manager against the existing fake-relay harness: two additively-minted rooms
// are served concurrently, a third room added while running is served within one
// poll with NO harness recycle, and revoking a device deads its room so ONLY
// that room's loop is reaped (its session closes with 4003) — again no recycle.
func TestRoomManagerServesReapsAndNeverRecyclesOnAddOrRevoke(t *testing.T) {
	upgrader := websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}
	accepted := make(chan acceptedRelayConn, 8)
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
	srv.onLinked = func(string) {}

	rotations := make(chan string, 4)
	srv.onRoomRotate = func(id string) { rotations <- id }

	// Two rooms minted ADDITIVELY — both must be served concurrently.
	linkA, err := srv.store.MintLink(pipeURL, "room-a")
	if err != nil {
		t.Fatal(err)
	}
	linkB, err := srv.store.MintLink(pipeURL, "room-b")
	if err != nil {
		t.Fatal(err)
	}

	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { done <- srv.runRoomManager(ctx) }()
	open := map[string]*websocket.Conn{}
	defer func() {
		cancel()
		select {
		case <-done:
		case <-time.After(2 * time.Second):
			t.Error("room manager did not stop")
		}
	}()

	pathA := "/r/" + linkA.Room + "/c"
	pathB := "/r/" + linkB.Room + "/c"

	// Collect the two concurrent dials.
	for len(open) < 2 {
		select {
		case c := <-accepted:
			open[c.path] = c.conn
		case <-time.After(4 * time.Second):
			t.Fatalf("expected both rooms dialed, got %d: %v", len(open), keys(open))
		}
	}
	if open[pathA] == nil || open[pathB] == nil {
		t.Fatalf("both rooms not served concurrently: %v", keys(open))
	}
	// An additive two-room serve must NOT be read as a rotation.
	assertNoRotation(t, rotations, "concurrent additive serve")

	// Add a third room while running -> served within one poll, still no recycle.
	external, err := OpenRelayStore(stateDir)
	if err != nil {
		t.Fatal(err)
	}
	linkC, err := external.MintLink(pipeURL, "room-c")
	if err != nil {
		t.Fatal(err)
	}
	pathC := "/r/" + linkC.Room + "/c"
	select {
	case c := <-accepted:
		if c.path != pathC {
			t.Fatalf("third dial path = %q, want %q", c.path, pathC)
		}
		open[c.path] = c.conn
	case <-time.After(4 * time.Second):
		t.Fatal("added room C was not served within the poll window")
	}
	assertNoRotation(t, rotations, "live add of room C")

	// Bind a device on room A, then revoke it: room A goes dead, ONLY room A's
	// loop is reaped (its session closes), room B stays live, and no recycle.
	const devA = "dev-aaaaaa111111"
	hello := helloFrame{T: "hello", V: ProtocolVersion, DeviceID: devA, Secret: linkA.Secret, ResumeFrom: "0"}
	if err := open[pathA].WriteJSON(hello); err != nil {
		t.Fatal(err)
	}
	_ = open[pathA].SetReadDeadline(time.Now().Add(3 * time.Second))
	if _, raw, err := open[pathA].ReadMessage(); err != nil {
		t.Fatalf("no welcome on room A: %v", err)
	} else if !strings.Contains(string(raw), `"welcome"`) {
		t.Fatalf("room A first frame not welcome: %s", raw)
	}

	if _, err := external.Revoke(devA); err != nil {
		t.Fatal(err)
	}

	// Room A's session must end (a 4003 "revoked" frame and/or a socket close).
	if !readUntilClosedOrRevoked(open[pathA]) {
		t.Fatal("room A session did not close after revoke")
	}
	// Room B stays live: no close frame within a short window.
	_ = open[pathB].SetReadDeadline(time.Now().Add(500 * time.Millisecond))
	if _, _, err := open[pathB].ReadMessage(); err == nil {
		t.Fatal("room B unexpectedly received a frame/close after A was revoked")
	} else if websocket.IsCloseError(err, 4003) || websocket.IsUnexpectedCloseError(err) {
		t.Fatalf("room B socket closed after A revoke: %v", err)
	}
	assertNoRotation(t, rotations, "single-room revoke reap")

	// Drain any late dials so shutdown is clean.
	go func() {
		for {
			select {
			case <-accepted:
			case <-ctx.Done():
				return
			}
		}
	}()
}

func assertNoRotation(t *testing.T, rotations <-chan string, phase string) {
	t.Helper()
	select {
	case id := <-rotations:
		t.Fatalf("%s wrongly filed a harness recycle for %q", phase, id)
	case <-time.After(250 * time.Millisecond):
	}
}

// readUntilClosedOrRevoked reads until the socket closes or a "revoked" error
// frame arrives, returning true in either case (both are valid revoke UX).
func readUntilClosedOrRevoked(c *websocket.Conn) bool {
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		_ = c.SetReadDeadline(time.Now().Add(3 * time.Second))
		_, raw, err := c.ReadMessage()
		if err != nil {
			return true
		}
		var f struct {
			T    string `json:"t"`
			Code string `json:"code"`
		}
		if json.Unmarshal(raw, &f) == nil && f.T == "error" && f.Code == "revoked" {
			return true
		}
	}
	return false
}

func keys(m map[string]*websocket.Conn) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	return out
}
