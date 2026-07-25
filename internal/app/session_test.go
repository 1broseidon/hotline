package app

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/gorilla/websocket"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/transcript"
)

type goldenFixture struct {
	Frames []struct {
		Dir   string          `json:"dir"`
		Frame json.RawMessage `json:"frame"`
	} `json:"frames"`
	Cases []struct {
		Name   string `json:"name"`
		URI    string `json:"uri"`
		Expect struct {
			OK      bool   `json:"ok"`
			Reason  string `json:"reason"`
			Pairing struct {
				URL    string `json:"url"`
				Room   string `json:"room"`
				Secret string `json:"secret"`
				Name   string `json:"name"`
			} `json:"pairing"`
		} `json:"expect"`
	} `json:"cases"`
}

func loadGolden(t *testing.T, name string) goldenFixture {
	t.Helper()
	path := filepath.Join("..", "..", "protocol", "v2", "fixtures", name)
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var f goldenFixture
	if err := json.Unmarshal(data, &f); err != nil {
		t.Fatal(err)
	}
	return f
}

func TestGoldenFixturesExactTypeCoverage(t *testing.T) {
	for _, name := range []string{"hello-welcome-drain.json", "resume-dedup.json", "gap-reset.json", "send-echo.json", "cursor-ahead-and-errors.json", "presence.json"} {
		t.Run(name, func(t *testing.T) {
			f := loadGolden(t, name)
			if len(f.Frames) == 0 {
				t.Fatal("fixture has no frames")
			}
			for i, row := range f.Frames {
				var obj map[string]json.RawMessage
				if err := json.Unmarshal(row.Frame, &obj); err != nil {
					t.Fatalf("frame %d: %v", i, err)
				}
				_, lower := obj["t"]
				got, exact := exactType(row.Frame)
				if lower && (!exact || got == "") {
					t.Fatalf("frame %d exact t not decoded", i)
				}
				if !lower && exact {
					t.Fatalf("frame %d without lowercase t decoded as %q", i, got)
				}
			}
		})
	}
}

func TestGoldenPairURI(t *testing.T) {
	f := loadGolden(t, "pair-uri.json")
	for _, tc := range f.Cases {
		t.Run(tc.Name, func(t *testing.T) {
			u, err := url.Parse(tc.URI)
			if err != nil {
				t.Fatal(err)
			}
			q := u.Query()
			valid := u.Scheme == "hotline" && u.Host == "pair" && q.Get("v") == "1" && roomIDRE.MatchString(q.Get("r")) && len(q.Get("s")) == 43 && q.Get("u") != ""
			if valid != tc.Expect.OK {
				t.Fatalf("valid=%v want %v", valid, tc.Expect.OK)
			}
			if tc.Expect.OK {
				generated := PairingURI(tc.Expect.Pairing.URL, tc.Expect.Pairing.Room, tc.Expect.Pairing.Secret, tc.Expect.Pairing.Name)
				got, _ := url.Parse(generated)
				if got.Query().Get("future") != "" || got.Query().Get("u") != tc.Expect.Pairing.URL || got.Query().Get("r") != tc.Expect.Pairing.Room || got.Query().Get("s") != tc.Expect.Pairing.Secret || got.Query().Get("n") != tc.Expect.Pairing.Name {
					t.Fatalf("generated URI = %s", generated)
				}
			}
		})
	}
}

type sessionHarness struct {
	t      *testing.T
	srv    *Server
	room   RoomRecord
	secret string
	http   *httptest.Server
}

func newSessionHarness(t *testing.T, deviceID string) *sessionHarness {
	t.Helper()
	dir := t.TempDir()
	cfg := &config.Config{StateDir: dir, AccessFile: filepath.Join(dir, "access.json"), TranscriptFile: filepath.Join(dir, "transcript.jsonl")}
	srv := NewServer(cfg, transcript.New(cfg.TranscriptFile))
	if srv.initErr != nil {
		t.Fatal(srv.initErr)
	}
	room := RoomRecord{ID: "Ab3dEf6hIj8lMn0pQr2tUv", URL: "ws://fixture.invalid", Name: "pi", SecretHash: secretHash("Ab3dEf6hIj8lMn0pQr2tUvWx4zAb3dEf6hIj8lMn0pQ")}
	secret := "Ab3dEf6hIj8lMn0pQr2tUvWx4zAb3dEf6hIj8lMn0pQ"
	srv.store.st.Rooms[room.ID] = room
	srv.store.st.CurrentRoom = room.ID
	if deviceID != "" {
		srv.store.st.Devices[deviceID] = DeviceRecord{ID: deviceID, Room: room.ID, SecretHash: room.SecretHash, State: DeviceActive}
		srv.mailbox.disk.Devices[deviceID] = &mailboxRecord{Floor: "0", Head: "0", Ack: "0"}
	}
	up := websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}
	// sessions tracks the serveV2Conn goroutines: httptest.Server.Close does
	// not wait for hijacked websocket handlers, so without the Wait a session
	// goroutine can still be persisting state (mailbox ack saves, etc.) while
	// t.TempDir's RemoveAll runs — a "directory not empty" teardown flake.
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
	h := &sessionHarness{t: t, srv: srv, room: room, secret: secret, http: ts}
	t.Cleanup(func() { ts.Close(); sessions.Done(); sessions.Wait(); srv.outbox.close() })
	return h
}

func (h *sessionHarness) dial() *websocket.Conn {
	h.t.Helper()
	wsURL := "ws" + h.http.URL[len("http"):] + "/"
	conn, _, err := websocket.DefaultDialer.Dial(wsURL, nil)
	if err != nil {
		h.t.Fatal(err)
	}
	return conn
}

func writeRaw(t *testing.T, conn *websocket.Conn, raw []byte) {
	t.Helper()
	if err := conn.WriteMessage(websocket.TextMessage, raw); err != nil {
		t.Fatal(err)
	}
}

// readRaw reads the next frame, SKIPPING agent_state transients: since ERRATA
// E7 every caught-up session receives an unconditional snapshot, which would
// otherwise interleave with the exact frame sequences these goldens pin. The
// snapshot itself is pinned by TestSessionSendsAgentStateSnapshotAfterDrain
// (which uses readRawAny).
func readRaw(t *testing.T, conn *websocket.Conn) []byte {
	t.Helper()
	for {
		raw := readRawAny(t, conn)
		if typ, ok := exactType(raw); ok && typ == "agent_state" {
			continue
		}
		return raw
	}
}

func readRawAny(t *testing.T, conn *websocket.Conn) []byte {
	t.Helper()
	_ = conn.SetReadDeadline(time.Now().Add(3 * time.Second))
	_, raw, err := conn.ReadMessage()
	if err != nil {
		t.Fatal(err)
	}
	return raw
}

func assertJSONEqual(t *testing.T, got, want []byte) {
	t.Helper()
	var g, w any
	if err := json.Unmarshal(got, &g); err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(want, &w); err != nil {
		t.Fatal(err)
	}
	gb, _ := json.Marshal(g)
	wb, _ := json.Marshal(w)
	if string(gb) != string(wb) {
		t.Fatalf("JSON mismatch\n got %s\nwant %s", gb, wb)
	}
}

func assertStampedMailboxItemEqual(t *testing.T, got, want []byte) {
	t.Helper()
	var item MailboxItem
	if err := json.Unmarshal(got, &item); err != nil {
		t.Fatal(err)
	}
	if _, err := time.Parse(time.RFC3339Nano, item.CreatedAt); err != nil {
		t.Fatalf("created_at = %q: %v", item.CreatedAt, err)
	}
	item.CreatedAt = "" // the older optional-field fixture remains a valid baseline
	assertJSONEqual(t, mustMarshal(item), want)
}

func TestGoldenHelloWelcomeDrainAndResumeDedup(t *testing.T) {
	f := loadGolden(t, "hello-welcome-drain.json")
	h := newSessionHarness(t, "dev-af31fd290542")
	mb := h.srv.mailbox.disk.Devices["dev-af31fd290542"]
	mb.Floor, mb.Head, mb.Ack = "9007199254741001", "9007199254741007", "9007199254741001"
	for _, idx := range []int{2, 3} {
		var item MailboxItem
		if err := json.Unmarshal(f.Frames[idx].Frame, &item); err != nil {
			t.Fatal(err)
		}
		mb.Items = append(mb.Items, item)
	}

	conn := h.dial()
	defer conn.Close()
	writeRaw(t, conn, f.Frames[0].Frame)
	assertJSONEqual(t, readRaw(t, conn), f.Frames[1].Frame)
	assertJSONEqual(t, readRaw(t, conn), f.Frames[2].Frame)
	assertJSONEqual(t, readRaw(t, conn), f.Frames[3].Frame)
	writeRaw(t, conn, f.Frames[4].Frame)
	writeRaw(t, conn, f.Frames[5].Frame)
	assertJSONEqual(t, readRaw(t, conn), f.Frames[6].Frame)
	var live MailboxItem
	if err := json.Unmarshal(f.Frames[7].Frame, &live); err != nil {
		t.Fatal(err)
	}
	if _, err := h.srv.mailbox.enqueue("dev-af31fd290542", live.J, live.Payload); err != nil {
		t.Fatal(err)
	}
	assertStampedMailboxItemEqual(t, readRaw(t, conn), f.Frames[7].Frame)
	writeRaw(t, conn, f.Frames[8].Frame)

	// Kill after apply but before ack: a separate crash fixture mailbox keeps the
	// old durable cursor and therefore re-delivers the same deterministic id.
	resume := loadGolden(t, "resume-dedup.json")
	h2 := newSessionHarness(t, "dev-af31fd290542")
	mb2 := h2.srv.mailbox.disk.Devices["dev-af31fd290542"]
	mb2.Floor, mb2.Head, mb2.Ack = "9007199254741001", "9007199254741007", "9007199254741001"
	for _, idx := range []int{2, 3} {
		var seeded MailboxItem
		_ = json.Unmarshal(resume.Frames[idx].Frame, &seeded)
		mb2.Items = append(mb2.Items, seeded)
	}
	conn2 := h2.dial()
	writeRaw(t, conn2, resume.Frames[0].Frame)
	_ = readRaw(t, conn2)
	first := readRaw(t, conn2)
	_ = conn2.Close() // applied, not acked
	conn3 := h2.dial()
	defer conn3.Close()
	writeRaw(t, conn3, resume.Frames[0].Frame)
	_ = readRaw(t, conn3)
	second := readRaw(t, conn3)
	var firstItem, secondItem MailboxItem
	_ = json.Unmarshal(first, &firstItem)
	_ = json.Unmarshal(second, &secondItem)
	if firstItem.ID != "env-j812-daf31fd290542" || secondItem.ID != firstItem.ID {
		t.Fatalf("redelivered ids = %s, %s", firstItem.ID, secondItem.ID)
	}
}

func TestGoldenGapReset(t *testing.T) {
	f := loadGolden(t, "gap-reset.json")
	h := newSessionHarness(t, "dev-af31fd290542")
	mb := h.srv.mailbox.disk.Devices["dev-af31fd290542"]
	mb.Floor, mb.Head, mb.Ack = "9007199254741050", "9007199254741052", "9007199254741050"
	for _, idx := range []int{4, 5} {
		var item MailboxItem
		_ = json.Unmarshal(f.Frames[idx].Frame, &item)
		mb.Items = append(mb.Items, item)
	}
	conn := h.dial()
	defer conn.Close()
	writeRaw(t, conn, f.Frames[0].Frame)
	assertJSONEqual(t, readRaw(t, conn), f.Frames[1].Frame)
	assertJSONEqual(t, readRaw(t, conn), f.Frames[2].Frame)
	writeRaw(t, conn, f.Frames[3].Frame)
	assertJSONEqual(t, readRaw(t, conn), f.Frames[4].Frame)
	assertJSONEqual(t, readRaw(t, conn), f.Frames[5].Frame)
	writeRaw(t, conn, f.Frames[6].Frame)
}

type captureSink struct {
	mu    sync.Mutex
	calls []string
	metas []map[string]string
}

func (s *captureSink) SendChannel(_ context.Context, content string, meta map[string]string) error {
	s.mu.Lock()
	s.calls = append(s.calls, content)
	copied := make(map[string]string, len(meta))
	for key, value := range meta {
		copied[key] = value
	}
	s.metas = append(s.metas, copied)
	s.mu.Unlock()
	return nil
}
func (*captureSink) SendVerdict(context.Context, string, string) error { return nil }

func TestGoldenSendEchoCIDIdempotency(t *testing.T) {
	f := loadGolden(t, "send-echo.json")
	h := newSessionHarness(t, "dev-af31fd290542")
	// The fixture pins the echo at seq 815. The session's unconditional
	// post-drain agent_state snapshot (E7) consumes one transient seq first,
	// so seed one lower.
	h.srv.outbox.seq = 813
	mb := h.srv.mailbox.disk.Devices["dev-af31fd290542"]
	mb.Floor, mb.Head, mb.Ack = "9007199254741008", "9007199254741008", "9007199254741008"
	sink := &captureSink{}
	h.srv.bindSink(sink)
	conn := h.dial()
	defer conn.Close()
	hello := mustMarshal(map[string]any{"t": "hello", "v": 2, "device_id": "dev-af31fd290542", "secret": h.secret, "resume_from": mb.Head})
	writeRaw(t, conn, hello)
	_ = readRaw(t, conn)
	writeRaw(t, conn, f.Frames[0].Frame)
	assertStampedMailboxItemEqual(t, readRaw(t, conn), f.Frames[1].Frame)
	writeRaw(t, conn, f.Frames[3].Frame) // duplicate: silent
	writeRaw(t, conn, f.Frames[4].Frame)
	assertStampedMailboxItemEqual(t, readRaw(t, conn), f.Frames[5].Frame)
	sink.mu.Lock()
	defer sink.mu.Unlock()
	if len(sink.calls) != 2 {
		t.Fatalf("sink calls = %v", sink.calls)
	}
}

func TestSessionReplayCarriesOriginalCreatedAt(t *testing.T) {
	const deviceID = "dev-af31fd290542"
	original := time.Date(2026, 7, 17, 20, 15, 23, 456789000, time.UTC)
	h := newSessionHarness(t, deviceID)
	h.srv.outbox.clock = func() time.Time { return original }
	// Pin the mailbox clock too. expireLocked trims items older than
	// mailboxRetention against m.clock(); with the default time.Now the fixed
	// `original` instant silently crossed the retention boundary on
	// 2026-07-24 and the box replied with a mailbox_gap instead of the item,
	// which unmarshals into MailboxItem as an empty created_at. Keep BOTH
	// clocks on the same fixed instant so this test never depends on wall
	// time again — do not "simplify" original to time.Now()-relative, that
	// only resets the fuse.
	h.srv.mailbox.clock = func() time.Time { return original.Add(time.Minute) }
	h.srv.emit(func(seq uint64) []byte {
		return msgFrame(seq, fmt.Sprintf("a-%d", seq), "timestamped replay", nil, "", nil, nil)
	})

	readReplay := func() MailboxItem {
		conn := h.dial()
		defer conn.Close()
		writeRaw(t, conn, mustMarshal(map[string]any{
			"t": "hello", "v": ProtocolVersion, "device_id": deviceID,
			"secret": h.secret, "resume_from": "0",
		}))
		if typ, _ := exactType(readRaw(t, conn)); typ != "welcome" {
			t.Fatalf("first replay frame = %q, want welcome", typ)
		}
		var item MailboxItem
		if err := json.Unmarshal(readRaw(t, conn), &item); err != nil {
			t.Fatal(err)
		}
		return item
	}

	first := readReplay()
	second := readReplay()
	want := original.Format(time.RFC3339Nano)
	if first.CreatedAt != want || second.CreatedAt != want {
		t.Fatalf("replay created_at = %q, %q; want original %q", first.CreatedAt, second.CreatedAt, want)
	}
}

func TestSessionGenericAttachmentUploadReachesSinkAndReplays(t *testing.T) {
	const (
		deviceID = "dev-af31fd290542"
		xfer     = "x-document-upload"
		cid      = "01J2ZK8Q0X6WYV9R3T5B7N4DOC"
		name     = "report.pdf"
		media    = "application/pdf"
	)
	data := []byte("%PDF-test")
	h := newSessionHarness(t, deviceID)
	sink := &captureSink{}
	h.srv.bindSink(sink)

	conn := h.dial()
	writeRaw(t, conn, mustMarshal(map[string]any{
		"t": "hello", "v": ProtocolVersion, "device_id": deviceID,
		"secret": h.secret, "resume_from": "0",
	}))
	if typ, _ := exactType(readRaw(t, conn)); typ != "welcome" {
		t.Fatalf("first frame = %q, want welcome", typ)
	}
	writeRaw(t, conn, mustMarshal(map[string]any{
		"t": "blob_begin", "xfer": xfer, "mime": media, "size": len(data), "chunks": 1,
	}))
	writeRaw(t, conn, mustMarshal(map[string]any{
		"t": "blob_chunk", "xfer": xfer, "i": 0, "data": base64.StdEncoding.EncodeToString(data),
	}))
	writeRaw(t, conn, mustMarshal(map[string]any{"t": "blob_end", "xfer": xfer}))
	writeRaw(t, conn, mustMarshal(map[string]any{
		"t": "device_send", "cid": cid,
		"payload": map[string]any{
			"t": "send.attachment", "xfer": xfer, "name": name, "mime": media, "size": len(data),
		},
	}))

	for _, wantType := range []string{"blob_begin", "blob_chunk", "blob_end"} {
		if typ, _ := exactType(readRaw(t, conn)); typ != wantType {
			t.Fatalf("attachment echo prelude = %q, want %q", typ, wantType)
		}
	}
	var item MailboxItem
	if err := json.Unmarshal(readRaw(t, conn), &item); err != nil {
		t.Fatal(err)
	}
	var echo struct {
		T    string  `json:"t"`
		CID  string  `json:"cid"`
		Kind string  `json:"kind"`
		File fileRef `json:"file"`
	}
	if err := json.Unmarshal(item.Payload, &echo); err != nil {
		t.Fatal(err)
	}
	if echo.T != "sent" || echo.CID != cid || echo.Kind != "attachment" ||
		echo.File.Name != name || echo.File.Mime != media || echo.File.Size != int64(len(data)) || echo.File.Xfer != xfer {
		t.Fatalf("attachment echo = %+v", echo)
	}

	sink.mu.Lock()
	calls := append([]string(nil), sink.calls...)
	metas := append([]map[string]string(nil), sink.metas...)
	sink.mu.Unlock()
	if len(calls) != 1 || len(metas) != 1 || metas[0]["attachment_file_id"] != xfer ||
		metas[0]["attachment_name"] != name || metas[0]["attachment_kind"] != "document" {
		t.Fatalf("sink calls=%v metas=%v", calls, metas)
	}
	if rec, ok := h.srv.blobs.resolve(xfer); !ok {
		t.Fatal("completed upload is not downloadable")
	} else if got, err := os.ReadFile(rec.Path); err != nil || string(got) != string(data) {
		t.Fatalf("downloadable upload = %q err=%v", got, err)
	}
	_ = conn.Close() // no ack: reconnect must replay both bytes and sent.file

	replay := h.dial()
	defer replay.Close()
	writeRaw(t, replay, mustMarshal(map[string]any{
		"t": "hello", "v": ProtocolVersion, "device_id": deviceID,
		"secret": h.secret, "resume_from": "0",
	}))
	_ = readRaw(t, replay) // welcome
	for _, wantType := range []string{"blob_begin", "blob_chunk", "blob_end"} {
		if typ, _ := exactType(readRaw(t, replay)); typ != wantType {
			t.Fatalf("replay attachment prelude = %q, want %q", typ, wantType)
		}
	}
	var replayed MailboxItem
	if err := json.Unmarshal(readRaw(t, replay), &replayed); err != nil {
		t.Fatal(err)
	}
	if string(replayed.Payload) != string(item.Payload) {
		t.Fatalf("replayed payload = %s, want %s", replayed.Payload, item.Payload)
	}
}

func TestDeviceSendReplayAfterCrashReusesDurableEcho(t *testing.T) {
	const deviceID = "dev-af31fd290542"
	original := time.Date(2026, 7, 17, 20, 16, 45, 123456000, time.UTC)
	h := newSessionHarness(t, deviceID)
	h.srv.outbox.clock = func() time.Time { return original }
	h.srv.store.mu.Lock()
	err := h.srv.store.saveLocked()
	h.srv.store.mu.Unlock()
	if err != nil {
		t.Fatal(err)
	}
	h.srv.mailbox.mu.Lock()
	err = h.srv.mailbox.saveLocked()
	h.srv.mailbox.mu.Unlock()
	if err != nil {
		t.Fatal(err)
	}
	sink := &captureSink{}
	h.srv.bindSink(sink)
	frame := deviceSendFrame{
		T:       "device_send",
		CID:     "01J2ZK8Q0X6WYV9R3T5B7N4E2M",
		Payload: json.RawMessage(`{"t":"send","text":"crash-safe"}`),
	}

	crashed := false
	h.srv.afterDeviceSendPersist = func() { panic("simulated crash") }
	func() {
		defer func() {
			if recover() != nil {
				crashed = true
			}
		}()
		_ = h.srv.handleDeviceSend(context.Background(), deviceID, frame)
	}()
	if !crashed {
		t.Fatal("crash seam did not run")
	}
	if got := len(h.srv.mailbox.disk.Devices[deviceID].Items); got != 0 {
		t.Fatalf("mailbox items before recovery = %d, want 0", got)
	}
	h.srv.outbox.close()

	restarted := NewServer(h.srv.cfg, transcript.New(h.srv.cfg.TranscriptFile))
	if restarted.initErr != nil {
		t.Fatal(restarted.initErr)
	}
	t.Cleanup(restarted.outbox.close)
	restarted.bindSink(sink)
	if err := restarted.handleDeviceSend(context.Background(), deviceID, frame); err != nil {
		t.Fatal(err)
	}

	sink.mu.Lock()
	calls := append([]string(nil), sink.calls...)
	sink.mu.Unlock()
	if len(calls) != 1 {
		t.Fatalf("sink calls after replay = %v", calls)
	}
	frames := restarted.outbox.framesAfter(0)
	if len(frames) != 1 {
		t.Fatalf("journal frames after replay = %d, want 1", len(frames))
	}
	wantCreatedAt := original.Format(time.RFC3339Nano)
	if frames[0].createdAt != wantCreatedAt {
		t.Fatalf("journal created_at after restart = %q, want %q", frames[0].createdAt, wantCreatedAt)
	}
	var echo struct {
		T   string `json:"t"`
		CID string `json:"cid"`
	}
	if err := json.Unmarshal(frames[0].data, &echo); err != nil || echo.T != "sent" || echo.CID != frame.CID {
		t.Fatalf("recovered echo = %s err=%v", frames[0].data, err)
	}
	items := restarted.mailbox.disk.Devices[deviceID].Items
	if len(items) != 1 || items[0].J != fmt.Sprint(frames[0].seq) || items[0].CreatedAt != wantCreatedAt {
		t.Fatalf("reconciled mailbox = %+v", items)
	}
}

func TestGoldenCursorAheadExactCaseUnauthorizedRevokedAndBadFrame(t *testing.T) {
	f := loadGolden(t, "cursor-ahead-and-errors.json")
	h := newSessionHarness(t, "dev-af31fd290542")
	mb := h.srv.mailbox.disk.Devices["dev-af31fd290542"]
	mb.Floor, mb.Head, mb.Ack = "9007199254741001", "9007199254741007", "9007199254741001"

	conn := h.dial()
	writeRaw(t, conn, f.Frames[0].Frame)
	assertJSONEqual(t, readRaw(t, conn), f.Frames[1].Frame)
	writeRaw(t, conn, f.Frames[2].Frame)
	assertJSONEqual(t, readRaw(t, conn), f.Frames[3].Frame)
	_, _, err := conn.ReadMessage()
	if ce, ok := err.(*websocket.CloseError); !ok || ce.Code != 4003 {
		t.Fatalf("unauthorized close = %T %v", err, err)
	}

	h.srv.store.st.Devices["dev-revoked00001"] = DeviceRecord{ID: "dev-revoked00001", Room: h.room.ID, SecretHash: secretHash("Zz9yXx7wVv5uTt3sRr1qPoNn8mLl6kJj4iHh2gFf0eD"), State: DeviceBanned}
	conn = h.dial()
	writeRaw(t, conn, f.Frames[4].Frame)
	assertJSONEqual(t, readRaw(t, conn), f.Frames[5].Frame)
	_, _, err = conn.ReadMessage()
	if ce, ok := err.(*websocket.CloseError); !ok || ce.Code != 4003 {
		t.Fatalf("revoked close = %T %v", err, err)
	}

	conn = h.dial()
	defer conn.Close()
	writeRaw(t, conn, f.Frames[6].Frame) // uppercase T is dropped
	hello := mustMarshal(map[string]any{"t": "hello", "v": 2, "device_id": "dev-af31fd290542", "secret": h.secret, "resume_from": "9007199254741007"})
	writeRaw(t, conn, hello)
	if typ, _ := exactType(readRaw(t, conn)); typ != "welcome" {
		t.Fatalf("uppercase T mutated handshake")
	}
	writeRaw(t, conn, f.Frames[7].Frame)
	assertJSONEqual(t, readRaw(t, conn), f.Frames[8].Frame)
}

func TestServeV2ConnSecondHelloRestartsSession(t *testing.T) {
	const deviceID = "dev-af31fd290542"
	h := newSessionHarness(t, deviceID)
	conn := h.dial()
	defer conn.Close()

	hello := func(resumeFrom string) []byte {
		return mustMarshal(map[string]any{
			"t": "hello", "v": ProtocolVersion, "device_id": deviceID,
			"secret": h.secret, "resume_from": resumeFrom,
		})
	}
	writeRaw(t, conn, hello("0"))
	if typ, _ := exactType(readRaw(t, conn)); typ != "welcome" {
		t.Fatalf("first hello response type = %q, want welcome", typ)
	}

	first, err := h.srv.mailbox.enqueue(deviceID, "1", []byte(`{"t":"msg","seq":1,"id":"a-1","text":"first"}`))
	if err != nil {
		t.Fatal(err)
	}
	var delivered MailboxItem
	if raw := readRaw(t, conn); json.Unmarshal(raw, &delivered) != nil || delivered.M != first.M {
		t.Fatalf("first delivery = %s, want mailbox item %s", raw, first.M)
	}
	writeRaw(t, conn, mustMarshal(map[string]any{"t": "mailbox_ack", "m": first.M}))
	writeRaw(t, conn, mustMarshal(map[string]any{"t": "ping", "n": 1}))
	if typ, _ := exactType(readRaw(t, conn)); typ != "pong" {
		t.Fatalf("post-ack response type = %q, want pong", typ)
	}

	// Put an unannounced durable item behind the acknowledged cursor so only the
	// restarted session's fresh snapshot can drain it.
	h.srv.mailbox.mu.Lock()
	second := h.srv.mailbox.enqueueLocked(deviceID, "2", []byte(`{"t":"msg","seq":2,"id":"a-2","text":"second"}`), true)
	err = h.srv.mailbox.saveLocked()
	h.srv.mailbox.mu.Unlock()
	if err != nil {
		t.Fatal(err)
	}

	writeRaw(t, conn, hello(first.M))
	raw := readRaw(t, conn)
	if typ, _ := exactType(raw); typ != "welcome" {
		t.Fatalf("second hello response = %s, want fresh welcome", raw)
	}
	if typ, _ := exactType(raw); typ == "error" {
		t.Fatalf("second hello emitted an error: %s", raw)
	}
	if raw = readRaw(t, conn); json.Unmarshal(raw, &delivered) != nil || delivered.M != second.M {
		t.Fatalf("restart drain = %s, want mailbox item %s", raw, second.M)
	}

	h.srv.mailbox.mu.Lock()
	subscribers := len(h.srv.mailbox.subs[deviceID])
	h.srv.mailbox.mu.Unlock()
	if subscribers != 1 {
		t.Fatalf("live subscribers after restart = %d, want 1", subscribers)
	}
	third, err := h.srv.mailbox.enqueue(deviceID, "3", []byte(`{"t":"msg","seq":3,"id":"a-3","text":"third"}`))
	if err != nil {
		t.Fatal(err)
	}
	if raw = readRaw(t, conn); json.Unmarshal(raw, &delivered) != nil || delivered.M != third.M {
		t.Fatalf("post-restart live delivery = %s, want mailbox item %s", raw, third.M)
	}
}

func TestServeV2ConnSecondHelloReverifiesRevokedDevice(t *testing.T) {
	const deviceID = "dev-af31fd290542"
	h := newSessionHarness(t, deviceID)
	conn := h.dial()
	defer conn.Close()
	hello := mustMarshal(map[string]any{
		"t": "hello", "v": ProtocolVersion, "device_id": deviceID,
		"secret": h.secret, "resume_from": "0",
	})

	writeRaw(t, conn, hello)
	if typ, _ := exactType(readRaw(t, conn)); typ != "welcome" {
		t.Fatalf("first hello response type = %q, want welcome", typ)
	}
	if _, err := h.srv.store.Revoke(deviceID); err != nil {
		t.Fatal(err)
	}
	writeRaw(t, conn, hello)
	var frame struct {
		T    string `json:"t"`
		Code string `json:"code"`
	}
	raw := readRaw(t, conn)
	if json.Unmarshal(raw, &frame) != nil || frame.T != "error" || frame.Code != "revoked" {
		t.Fatalf("second hello response = %s, want revoked error", raw)
	}
	_, _, err := conn.ReadMessage()
	if ce, ok := err.(*websocket.CloseError); !ok || ce.Code != 4003 {
		t.Fatalf("revoked close = %T %v", err, err)
	}
}

func TestPushIntentUsesSubscriberPresenceForEveryNotifiableItem(t *testing.T) {
	const deviceID = "dev-af31fd290542"
	h := newSessionHarness(t, deviceID)
	token := "ExponentPushToken[push-presence]"
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

	_, _, _, sub, err := h.srv.mailbox.stateAndSubscribe(deviceID)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := h.srv.mailbox.enqueue(deviceID, "1", []byte(`{"t":"msg","seq":1,"id":"a-1","text":"foreground"}`)); err != nil {
		t.Fatal(err)
	}
	h.srv.mailbox.unsubscribe(deviceID, sub)
	select {
	case body := <-got:
		t.Fatalf("push while subscribed = %v", body)
	case <-time.After(150 * time.Millisecond):
	}

	// The E10 push rule: msgs (element-only included) and reacts push; a text
	// edit pushes; an element-only edit (text == the synthesized fallback join)
	// and an empty-text edit do NOT.
	payloads := []struct {
		raw  string
		push bool
	}{
		{`{"t":"msg","seq":2,"id":"a-2","text":"away one"}`, true},
		{`{"t":"msg","seq":3,"id":"a-3","text":"away two"}`, true},
		{`{"t":"msg","seq":4,"id":"a-4","text":"away three"}`, true},
		{`{"t":"edit","seq":5,"id":"a-4","text":"away edit"}`, true},
		{`{"t":"edit","seq":6,"id":"a-4","text":"job x: done","elements":[{"el":"job","id":"el-1","fallback":"job x: done"}]}`, false},
		{`{"t":"msg","seq":7,"id":"a-5","text":"job y: running","elements":[{"el":"job","id":"el-2","fallback":"job y: running"}]}`, true},
		{`{"t":"react","seq":8,"msg_id":"a-4","emoji":"👍"}`, true},
	}
	wantPushes := 0
	for i, p := range payloads {
		if p.push {
			wantPushes++
		}
		if _, err := h.srv.mailbox.enqueue(deviceID, fmt.Sprint(i+2), []byte(p.raw)); err != nil {
			t.Fatal(err)
		}
	}
	for i := 0; i < wantPushes; i++ {
		select {
		case body := <-got:
			if body["to"] != token {
				t.Fatalf("push %d = %v", i, body)
			}
		case <-time.After(2 * time.Second):
			t.Fatalf("push intents = %d, want %d", i, wantPushes)
		}
	}
	select {
	case body := <-got:
		t.Fatalf("extra push = %v", body)
	case <-time.After(150 * time.Millisecond):
	}
}

func waitUntil(t *testing.T, timeout time.Duration, cond func() bool) {
	t.Helper()
	deadline := time.Now().Add(timeout)
	for {
		if cond() {
			return
		}
		if time.Now().After(deadline) {
			t.Fatal("condition not met before timeout")
		}
		time.Sleep(5 * time.Millisecond)
	}
}

func drainExpo(t *testing.T) (*httptest.Server, chan map[string]any) {
	t.Helper()
	got := make(chan map[string]any, 8)
	expo := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var body map[string]any
		_ = json.NewDecoder(r.Body).Decode(&body)
		got <- body
		w.WriteHeader(http.StatusOK)
	}))
	t.Cleanup(expo.Close)
	return expo, got
}

// TestPresenceBackgroundFrameTriggersPushWithRoom exercises the full A-path on
// the wire: a foreground subscriber suppresses push; an explicit
// presence:background latches away so the next enqueue pushes; and the push
// data carries the current room (PART 2, foreground-banner routing).
func TestPresenceBackgroundFrameTriggersPushWithRoom(t *testing.T) {
	const deviceID = "dev-af31fd290542"
	h := newSessionHarness(t, deviceID)
	token := "ExponentPushToken[presence-bg]"
	if err := h.srv.store.SetPush(deviceID, token, "ios"); err != nil {
		t.Fatal(err)
	}
	expo, got := drainExpo(t)
	h.srv.pushEndpoint = expo.URL

	conn := h.dial()
	defer conn.Close()
	writeRaw(t, conn, []byte(`{"t":"hello","v":2,"device_id":"`+deviceID+`","secret":"`+h.secret+`","resume_from":"0"}`))
	if typ, _ := exactType(readRaw(t, conn)); typ != "welcome" {
		t.Fatal("first frame must be welcome")
	}
	// Foreground fresh subscriber: an enqueue is delivered live, no push.
	if _, err := h.srv.mailbox.enqueue(deviceID, "1", []byte(`{"t":"msg","seq":1,"id":"a-1","text":"live"}`)); err != nil {
		t.Fatal(err)
	}
	select {
	case body := <-got:
		t.Fatalf("push while foreground = %v", body)
	case <-time.After(150 * time.Millisecond):
	}

	// Explicit background latch → the next enqueue pushes.
	writeRaw(t, conn, []byte(`{"t":"presence","state":"background"}`))
	waitUntil(t, 2*time.Second, func() bool { return h.srv.mailbox.deviceAway(deviceID) })
	if _, err := h.srv.mailbox.enqueue(deviceID, "2", []byte(`{"t":"msg","seq":2,"id":"a-2","text":"away"}`)); err != nil {
		t.Fatal(err)
	}
	select {
	case body := <-got:
		if body["to"] != token {
			t.Fatalf("push to = %v", body["to"])
		}
		data, _ := body["data"].(map[string]any)
		if data["url"] != "hotline://chat" {
			t.Fatalf("push data url = %v", data["url"])
		}
		if data["room"] != h.room.ID {
			t.Fatalf("push data room = %v, want %s", data["room"], h.room.ID)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("background presence did not produce a push")
	}

	// presence:foreground clears the latch → push suppressed again.
	writeRaw(t, conn, []byte(`{"t":"presence","state":"foreground"}`))
	waitUntil(t, 2*time.Second, func() bool { return !h.srv.mailbox.deviceAway(deviceID) })
	if _, err := h.srv.mailbox.enqueue(deviceID, "3", []byte(`{"t":"msg","seq":3,"id":"a-3","text":"back"}`)); err != nil {
		t.Fatal(err)
	}
	select {
	case body := <-got:
		t.Fatalf("push after foreground restore = %v", body)
	case <-time.After(150 * time.Millisecond):
	}
}

// TestHeartbeatTimeoutTriggersPush pins the box-side lease (B) with a fake
// clock: a foreground subscriber that stops pinging (an old app that never sends
// presence) is treated as away once the 60s lease expires.
func TestHeartbeatTimeoutTriggersPush(t *testing.T) {
	const deviceID = "dev-af31fd290542"
	h := newSessionHarness(t, deviceID)
	var nanos int64 = time.Unix(2_000_000, 0).UnixNano()
	// Set the clock before any session starts (the harness dials lazily), then
	// advance it atomically so the live session goroutine reads it race-free.
	h.srv.mailbox.clock = func() time.Time { return time.Unix(0, atomic.LoadInt64(&nanos)) }
	token := "ExponentPushToken[lease]"
	if err := h.srv.store.SetPush(deviceID, token, "ios"); err != nil {
		t.Fatal(err)
	}
	expo, got := drainExpo(t)
	h.srv.pushEndpoint = expo.URL

	conn := h.dial()
	defer conn.Close()
	writeRaw(t, conn, []byte(`{"t":"hello","v":2,"device_id":"`+deviceID+`","secret":"`+h.secret+`","resume_from":"0"}`))
	if typ, _ := exactType(readRaw(t, conn)); typ != "welcome" {
		t.Fatal("first frame must be welcome")
	}
	// Within the lease: no push.
	atomic.StoreInt64(&nanos, time.Unix(2_000_000+30, 0).UnixNano())
	if _, err := h.srv.mailbox.enqueue(deviceID, "1", []byte(`{"t":"msg","seq":1,"id":"a-1","text":"fresh"}`)); err != nil {
		t.Fatal(err)
	}
	select {
	case body := <-got:
		t.Fatalf("push within lease = %v", body)
	case <-time.After(150 * time.Millisecond):
	}
	// Past the 60s lease with no ping refresh: away → push.
	atomic.StoreInt64(&nanos, time.Unix(2_000_000+61, 0).UnixNano())
	if _, err := h.srv.mailbox.enqueue(deviceID, "2", []byte(`{"t":"msg","seq":2,"id":"a-2","text":"stale"}`)); err != nil {
		t.Fatal(err)
	}
	select {
	case body := <-got:
		if body["to"] != token {
			t.Fatalf("lease-expiry push to = %v", body["to"])
		}
	case <-time.After(2 * time.Second):
		t.Fatal("lease expiry did not produce a push")
	}
}

// TestPresenceDuringGapAccepted verifies presence is honored during the
// mailbox_gap ack wait without disturbing the gap ack/drain.
func TestPresenceDuringGapAccepted(t *testing.T) {
	const deviceID = "dev-af31fd290542"
	h := newSessionHarness(t, deviceID)
	token := "ExponentPushToken[gap]"
	if err := h.srv.store.SetPush(deviceID, token, "ios"); err != nil {
		t.Fatal(err)
	}
	expo, got := drainExpo(t)
	h.srv.pushEndpoint = expo.URL
	mb := h.srv.mailbox.disk.Devices[deviceID]
	mb.Floor, mb.Head, mb.Ack = "9007199254741050", "9007199254741050", "9007199254741050"

	conn := h.dial()
	defer conn.Close()
	// resume_from below floor → welcome then mailbox_gap.
	writeRaw(t, conn, []byte(`{"t":"hello","v":2,"device_id":"`+deviceID+`","secret":"`+h.secret+`","resume_from":"0"}`))
	if typ, _ := exactType(readRaw(t, conn)); typ != "welcome" {
		t.Fatal("first frame must be welcome")
	}
	if typ, _ := exactType(readRaw(t, conn)); typ != "mailbox_gap" {
		t.Fatal("second frame must be mailbox_gap")
	}
	// Presence during the gap phase: accepted, applied, no bad_frame.
	writeRaw(t, conn, []byte(`{"t":"presence","state":"background"}`))
	waitUntil(t, 2*time.Second, func() bool { return h.srv.mailbox.deviceAway(deviceID) })
	// Complete the gap with the floor ack; the session proceeds to steady state.
	writeRaw(t, conn, []byte(`{"t":"mailbox_ack","m":"9007199254741050"}`))
	// The subscriber remains background-latched (an ack does not refresh), so an
	// enqueue past head pushes.
	if _, err := h.srv.mailbox.enqueue(deviceID, "9007199254741051", []byte(`{"t":"msg","seq":1,"id":"a-1","text":"post-gap"}`)); err != nil {
		t.Fatal(err)
	}
	select {
	case body := <-got:
		if body["to"] != token {
			t.Fatalf("post-gap push to = %v", body["to"])
		}
	case <-time.After(2 * time.Second):
		t.Fatal("presence during gap was not honored (no push after gap)")
	}
}

// TestMalformedPresenceIsBadFrame pins malformed presence → bad_frame.
func TestMalformedPresenceIsBadFrame(t *testing.T) {
	const deviceID = "dev-af31fd290542"
	h := newSessionHarness(t, deviceID)
	conn := h.dial()
	defer conn.Close()
	writeRaw(t, conn, []byte(`{"t":"hello","v":2,"device_id":"`+deviceID+`","secret":"`+h.secret+`","resume_from":"0"}`))
	if typ, _ := exactType(readRaw(t, conn)); typ != "welcome" {
		t.Fatal("first frame must be welcome")
	}
	writeRaw(t, conn, []byte(`{"t":"presence","state":"sideways"}`))
	raw := readRaw(t, conn) // skips the post-drain agent_state snapshot
	var e struct {
		T    string `json:"t"`
		Code string `json:"code"`
	}
	if err := json.Unmarshal(raw, &e); err != nil || e.T != "error" || e.Code != "bad_frame" {
		t.Fatalf("malformed presence response = %s (err %v)", raw, err)
	}
}

func TestTypingDeliveredLiveButNotRetainedOrRedelivered(t *testing.T) {
	const deviceID = "dev-af31fd290542"
	h := newSessionHarness(t, deviceID)
	hello := mustMarshal(map[string]any{
		"t": "hello", "v": ProtocolVersion, "device_id": deviceID,
		"secret": h.secret, "resume_from": "0",
	})
	conn := h.dial()
	writeRaw(t, conn, hello)
	if typ, _ := exactType(readRaw(t, conn)); typ != "welcome" {
		t.Fatalf("hello response type = %q", typ)
	}
	h.srv.emitTransient(func(seq uint64) []byte { return typingFrame(seq, true) })
	raw := readRaw(t, conn)
	if typ, _ := exactType(raw); typ != "typing" {
		t.Fatalf("live transient = %s", raw)
	}
	_ = conn.Close()

	deadline := time.Now().Add(2 * time.Second)
	for {
		h.srv.mailbox.mu.Lock()
		subs := len(h.srv.mailbox.subs[deviceID])
		h.srv.mailbox.mu.Unlock()
		if subs == 0 {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("subscriber did not close")
		}
		time.Sleep(10 * time.Millisecond)
	}
	h.srv.emitTransient(func(seq uint64) []byte { return typingFrame(seq, false) })
	if frames := h.srv.outbox.framesAfter(0); len(frames) != 0 {
		t.Fatalf("typing retained in journal: %+v", frames)
	}
	if items := h.srv.mailbox.disk.Devices[deviceID].Items; len(items) != 0 {
		t.Fatalf("typing retained in mailbox: %+v", items)
	}

	reconnected := h.dial()
	defer reconnected.Close()
	writeRaw(t, reconnected, hello)
	if typ, _ := exactType(readRaw(t, reconnected)); typ != "welcome" {
		t.Fatalf("reconnect response type = %q", typ)
	}
	// The unconditional post-drain agent_state snapshot (E7) is expected; the
	// typing frame must NOT be.
	if typ, _ := exactType(readRawAny(t, reconnected)); typ != "agent_state" {
		t.Fatalf("expected the post-drain agent_state snapshot, got %q", typ)
	}
	_ = reconnected.SetReadDeadline(time.Now().Add(150 * time.Millisecond))
	if _, _, err := reconnected.ReadMessage(); err == nil {
		t.Fatal("reconnect drain redelivered a typing frame")
	}
}

func TestConnectorBackoffShape(t *testing.T) {
	if nextBackoff(time.Second, time.Minute) != 2*time.Second || nextBackoff(32*time.Second, time.Minute) != time.Minute || nextBackoff(time.Minute, time.Minute) != time.Minute {
		t.Fatal("backoff is not 1s -> 60s exponential")
	}
	for i := 0; i < 100; i++ {
		d := jitterBackoff(8*time.Second, time.Minute)
		if d < 8*time.Second || d > 12*time.Second {
			t.Fatalf("jitter = %s", d)
		}
	}
}

func ExamplePairingURI() {
	fmt.Println(PairingURI("ws://127.0.0.1:8787", "Ab3dEf6hIj8lMn0pQr2tUv", "Ab3dEf6hIj8lMn0pQr2tUvWx4zAb3dEf6hIj8lMn0pQ", "pi") != "")
	// Output: true
}

// TestSessionSendsAgentStateSnapshotAfterDrain pins ERRATA E7 on the real
// session path: once a device is caught up (welcome + drain), the box sends
// the current agent_state snapshot UNCONDITIONALLY — an idle box sends the
// empty {runs:[],schedules:[],loops:[]} so a stale client snapshot from before
// a restart is corrected.
func TestSessionSendsAgentStateSnapshotAfterDrain(t *testing.T) {
	const deviceID = "dev-af31fd290542"
	h := newSessionHarness(t, deviceID)
	conn := h.dial()
	defer conn.Close()
	writeRaw(t, conn, []byte(`{"t":"hello","v":2,"device_id":"`+deviceID+`","secret":"`+h.secret+`","resume_from":"0"}`))

	var welcome map[string]any
	if err := json.Unmarshal(readRawAny(t, conn), &welcome); err != nil || welcome["t"] != "welcome" {
		t.Fatalf("first frame = %v (err %v), want welcome", welcome, err)
	}
	var snap struct {
		T     string `json:"t"`
		State struct {
			Runs      []any `json:"runs"`
			Schedules []any `json:"schedules"`
			Loops     []any `json:"loops"`
		} `json:"state"`
	}
	raw := readRawAny(t, conn)
	if err := json.Unmarshal(raw, &snap); err != nil || snap.T != "agent_state" {
		t.Fatalf("post-drain frame = %s, want an agent_state snapshot", raw)
	}
	if snap.State.Runs == nil || snap.State.Schedules == nil || snap.State.Loops == nil {
		t.Fatalf("empty snapshot must carry empty ARRAYS, got %s", raw)
	}
	if len(snap.State.Runs)+len(snap.State.Schedules)+len(snap.State.Loops) != 0 {
		t.Fatalf("idle box should snapshot empty state, got %s", raw)
	}
}
