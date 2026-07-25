package app

import (
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"io"
	"strings"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/transcript"
)

func newFleetServer(t *testing.T) *Server {
	t.Helper()
	cfg := testConfig(t)
	return NewServer(cfg, transcript.New(cfg.TranscriptFile))
}

func serveEdge(t *testing.T, s *Server, alias string) (FleetEdge, fleetHello) {
	t.Helper()
	edge, _, err := s.fleetStore.Link(fleetTestRelay, alias)
	if err != nil {
		t.Fatalf("link: %v", err)
	}
	return edge, fleetHello{T: "hello", V: ProtocolVersion, DeviceID: "flt-" + edge.EdgeID, Secret: edge.Secret}
}

// runFleetSession drives serveFleetSession synchronously: feed is pre-loaded plus
// a terminating EOF, so the handler processes every frame then returns.
func runFleetSession(t *testing.T, s *Server, edge FleetEdge, hello fleetHello, feed ...[]byte) (fleetResult, [][]byte) {
	t.Helper()
	inputs := make(chan sessionInput, len(feed)+2)
	for _, f := range feed {
		inputs <- sessionInput{raw: f}
	}
	inputs <- sessionInput{err: io.EOF}
	var frames [][]byte
	write := func(b []byte) error {
		frames = append(frames, append([]byte(nil), b...))
		return nil
	}
	res := s.serveFleetSession(context.Background(), edge.EdgeID, hello, inputs, write)
	return res, frames
}

func fmsgRaw(cid, text, kind string) []byte {
	m := map[string]any{"t": "fleet_msg", "cid": cid, "text": text}
	if kind != "" {
		m["kind"] = kind
	}
	b, _ := json.Marshal(m)
	return b
}

func frameOfType(frames [][]byte, typ string) map[string]any {
	for _, f := range frames {
		var m map[string]any
		if json.Unmarshal(f, &m) == nil && m["t"] == typ {
			return m
		}
	}
	return nil
}

func TestFleetWelcomeAndValidMsgJournaled(t *testing.T) {
	s := newFleetServer(t)
	if _, _, err := s.store.SeedIdentityName("TestBox"); err != nil {
		t.Fatalf("seed identity: %v", err)
	}
	edge, hello := serveEdge(t, s, "alpha")

	res, frames := runFleetSession(t, s, edge, hello, fmsgRaw("cid-abcdefgh1234", "hi peer", "brief"))
	if res.closeCode != 0 {
		t.Fatalf("clean EOF should not set a close code: %+v", res)
	}
	w := frameOfType(frames, "welcome_fleet")
	if w == nil {
		t.Fatalf("no welcome_fleet frame; got %d frames", len(frames))
	}
	if w["edge_id"] != edge.EdgeID {
		t.Fatalf("welcome_fleet edge_id=%v want %s", w["edge_id"], edge.EdgeID)
	}
	if w["box"] != "TestBox" {
		t.Fatalf("welcome_fleet box=%v want TestBox", w["box"])
	}
	// key_fp is a required, full RFC 7638 thumbprint (43-char base64url sha256).
	fp, _ := w["key_fp"].(string)
	if len(fp) != 43 {
		t.Fatalf("welcome_fleet key_fp not a full thumbprint: %q", fp)
	}
	if int(w["v"].(float64)) != ProtocolVersion {
		t.Fatalf("welcome_fleet v=%v", w["v"])
	}
	// Frozen journal entry (B4): v=1, monotonic seq, dir, complete frame incl from.
	entries, err := s.fleetStore.JournalEntries(edge.EdgeID)
	if err != nil || len(entries) != 1 {
		t.Fatalf("expected 1 journal entry, got %d (err %v)", len(entries), err)
	}
	e := entries[0]
	if e.V != 1 || e.Seq != 1 || e.Dir != "in" {
		t.Fatalf("journal entry envelope wrong: %+v", e)
	}
	var frame map[string]any
	if err := json.Unmarshal(e.Frame, &frame); err != nil {
		t.Fatalf("decode journal frame: %v", err)
	}
	if frame["text"] != "hi peer" || frame["cid"] != "cid-abcdefgh1234" || frame["t"] != "fleet_msg" {
		t.Fatalf("journal frame wrong: %+v", frame)
	}
	if _, ok := frame["from"]; !ok {
		t.Fatalf("journal frame missing from{}: %+v", frame)
	}
}

func TestFleetJournalSeqMonotonic(t *testing.T) {
	s := newFleetServer(t)
	edge, hello := serveEdge(t, s, "alpha")
	runFleetSession(t, s, edge, hello,
		fmsgRaw("cid-one000000001", "one", "brief"),
		fmsgRaw("cid-two000000002", "two", "task"),
	)
	entries, _ := s.fleetStore.JournalEntries(edge.EdgeID)
	if len(entries) != 2 || entries[0].Seq != 1 || entries[1].Seq != 2 {
		t.Fatalf("journal seq not monotonic: %+v", entries)
	}
}

func TestFleetMsgTextCapDropped(t *testing.T) {
	s := newFleetServer(t)
	edge, hello := serveEdge(t, s, "alpha")
	big := strings.Repeat("x", fleetTextCap+1)

	res, frames := runFleetSession(t, s, edge, hello, fmsgRaw("cid-oversize0001", big, "brief"))
	if res.closeCode != 0 {
		t.Fatalf("oversize msg must not close the session: %+v", res)
	}
	if e := frameOfType(frames, "error"); e == nil || e["code"] != "text_too_large" {
		t.Fatalf("expected a text_too_large error, got %v", e)
	}
	entries, _ := s.fleetStore.JournalEntries(edge.EdgeID)
	if len(entries) != 0 {
		t.Fatalf("oversize msg was journaled (%d entries)", len(entries))
	}
}

func TestFleetFrameOverBoundRejected(t *testing.T) {
	s := newFleetServer(t)
	edge, hello := serveEdge(t, s, "alpha")
	huge := make([]byte, fleetMaxFrameBytes+1)
	for i := range huge {
		huge[i] = 'a'
	}
	res, frames := runFleetSession(t, s, edge, hello, huge)
	if res.closeCode != 4002 || res.closeReason != "protocol_error" {
		t.Fatalf("oversize frame not refused: %+v", res)
	}
	if frameOfType(frames, "error") == nil {
		t.Fatalf("expected an error frame for the oversize frame")
	}
}

func TestFleetMalformedFrameRejected(t *testing.T) {
	s := newFleetServer(t)
	edge, hello := serveEdge(t, s, "alpha")
	res, _ := runFleetSession(t, s, edge, hello, []byte(`{"nope":1}`))
	if res.closeCode != 4002 || res.closeReason != "protocol_error" {
		t.Fatalf("malformed frame not refused: %+v", res)
	}
}

func TestFleetBadFieldsRejected(t *testing.T) {
	s := newFleetServer(t)
	edge, hello := serveEdge(t, s, "alpha")
	if res, _ := runFleetSession(t, s, edge, hello, fmsgRaw("bad cid with spaces", "x", "brief")); res.closeCode != 4002 {
		t.Fatalf("bad cid not refused: %+v", res)
	}
	if res, _ := runFleetSession(t, s, edge, hello, fmsgRaw("cid-goodgoodgood", "x", "bogus")); res.closeCode != 4002 {
		t.Fatalf("bad kind not refused: %+v", res)
	}
}

func TestFleetAppProtocolFrameRejected(t *testing.T) {
	s := newFleetServer(t)
	edge, hello := serveEdge(t, s, "alpha")
	deviceSend := []byte(`{"t":"device_send","cid":"cid-app-frame-01","payload":{"t":"send","text":"pwn"}}`)

	res, frames := runFleetSession(t, s, edge, hello, deviceSend)
	if res.closeCode != 4002 || res.closeReason != "protocol_error" {
		t.Fatalf("app frame did not close with protocol_error: %+v", res)
	}
	if e := frameOfType(frames, "error"); e == nil || e["code"] != "protocol_error" {
		t.Fatalf("expected a protocol_error frame, got %v", e)
	}
	entries, _ := s.fleetStore.JournalEntries(edge.EdgeID)
	if len(entries) != 0 {
		t.Fatalf("app frame was journaled (%d entries)", len(entries))
	}
}

func TestFleetUnauthorizedHelloRejected(t *testing.T) {
	s := newFleetServer(t)
	edge, hello := serveEdge(t, s, "alpha")
	hello.Secret = strings.Repeat("z", 43)

	res, frames := runFleetSession(t, s, edge, hello)
	if res.closeCode != 4003 || res.closeReason != "unauthorized" {
		t.Fatalf("bad-secret hello not rejected: %+v", res)
	}
	if frameOfType(frames, "welcome_fleet") != nil {
		t.Fatalf("welcome_fleet leaked to an unauthorized hello")
	}
}

// TestFleetRmDuringLiveSessionTerminates proves B2: a `fleet rm` that lands after
// the hello terminates the live session on the next frame — the per-frame
// LiveEdge re-check refuses to journal and closes with revoked.
func TestFleetRmDuringLiveSessionTerminates(t *testing.T) {
	s := newFleetServer(t)
	edge, hello := serveEdge(t, s, "alpha")

	inputs := make(chan sessionInput)
	done := make(chan fleetResult, 1)
	go func() {
		done <- s.serveFleetSession(context.Background(), edge.EdgeID, hello, inputs, func([]byte) error { return nil })
	}()

	inputs <- sessionInput{raw: fmsgRaw("cid-live00000001", "before rm", "brief")}
	waitUntil(t, 2*time.Second, func() bool {
		e, _ := s.fleetStore.JournalEntries(edge.EdgeID)
		return len(e) == 1
	})

	if _, err := s.fleetStore.Remove(edge.EdgeID); err != nil {
		t.Fatalf("remove: %v", err)
	}

	inputs <- sessionInput{raw: fmsgRaw("cid-live00000002", "after rm", "brief")}
	res := <-done
	if res.closeCode != 4003 || res.closeReason != "revoked" {
		t.Fatalf("session did not terminate after rm: %+v", res)
	}
	entries, _ := s.fleetStore.JournalEntries(edge.EdgeID)
	if len(entries) != 1 {
		t.Fatalf("post-rm frame was journaled: %d entries", len(entries))
	}
}

func TestFleetPlaintextHelloRejectedByEnvelope(t *testing.T) {
	s := newFleetServer(t)
	edge, _ := serveEdge(t, s, "alpha")
	codec, err := newEnvelopeCodec(edge.roomRecordFor())
	if err != nil {
		t.Fatalf("codec: %v", err)
	}
	plaintextHello := []byte(`{"t":"hello","v":2,"device_id":"flt-abcdefgh","secret":"s"}`)
	if _, err := codec.unwrap(plaintextHello); err == nil {
		t.Fatalf("plaintext hello was accepted on an envelope fleet room")
	}
}

func TestFleetReadHelloRejectsNonHello(t *testing.T) {
	s := newFleetServer(t)
	inputs := make(chan sessionInput, 2)
	inputs <- sessionInput{raw: []byte(`{"t":"ping","n":1}`)}
	var frames [][]byte
	write := func(b []byte) error { frames = append(frames, b); return nil }
	if _, ok := s.readFleetHello(context.Background(), "test", inputs, write); ok {
		t.Fatalf("readFleetHello accepted a non-hello first frame")
	}
	if e := frameOfType(frames, "error"); e == nil || e["code"] != "protocol_error" {
		t.Fatalf("expected protocol_error, got %v", e)
	}
}

func TestJWKThumbprintP256RFC7638(t *testing.T) {
	// RFC 7638 canonicalization: members lexicographically ordered crv,kty,x,y
	// with no whitespace, SHA-256, base64url (no pad).
	sum := sha256.Sum256([]byte(`{"crv":"P-256","kty":"EC","x":"abc","y":"def"}`))
	want := base64.RawURLEncoding.EncodeToString(sum[:])
	got := jwkThumbprintP256("abc", "def")
	if got != want {
		t.Fatalf("thumbprint = %q want %q", got, want)
	}
	if len(got) != 43 {
		t.Fatalf("thumbprint not 43 chars (full sha256): %q", got)
	}
}
