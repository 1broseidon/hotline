package app

import (
	"crypto/ecdsa"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"io"
	"math/big"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"
	"unicode/utf8"
)

// wakeCapture records the exact signed wake request the box sent to the core so
// a test can inspect the body (preview presence/truncation) and verify the
// signature actually covers the preview snippet.
type wakeCapture struct {
	mu        sync.Mutex
	body      []byte
	method    string
	path      string
	timestamp string
	nonce     string
	signature string
	hit       chan struct{}
}

func newWakeCapture() *wakeCapture { return &wakeCapture{hit: make(chan struct{}, 1)} }

func (c *wakeCapture) handler() http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		b, _ := io.ReadAll(r.Body)
		c.mu.Lock()
		c.body = b
		c.method = r.Method
		c.path = r.URL.Path
		c.timestamp = r.Header.Get("X-Hotline-Timestamp")
		c.nonce = r.Header.Get("X-Hotline-Nonce")
		c.signature = r.Header.Get("X-Hotline-Signature")
		c.mu.Unlock()
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte(`{"pushed":true}`))
		select {
		case c.hit <- struct{}{}:
		default:
		}
	}
}

func (c *wakeCapture) wait(t *testing.T) {
	t.Helper()
	select {
	case <-c.hit:
	case <-time.After(2 * time.Second):
		t.Fatal("timed out waiting for the wake request")
	}
}

func previewServer(t *testing.T, clear bool, cap *wakeCapture) (*Server, string, string) {
	t.Helper()
	core := httptest.NewServer(cap.handler())
	t.Cleanup(core.Close)
	srv, deviceID, roomID := coreModeServer(t, core.URL, "")
	srv.pushPreviewClear = clear
	srv.regMu.Lock()
	srv.registered[roomID] = true
	srv.regMu.Unlock()
	return srv, deviceID, roomID
}

func textItem(text string) MailboxItem {
	payload, _ := json.Marshal(map[string]any{"t": "msg", "id": "a-1", "text": text})
	return MailboxItem{Payload: payload}
}

// decodeWakeBody unmarshals the captured wake body into a generic map.
func (c *wakeCapture) decode(t *testing.T) map[string]any {
	t.Helper()
	c.mu.Lock()
	defer c.mu.Unlock()
	var m map[string]any
	if err := json.Unmarshal(c.body, &m); err != nil {
		t.Fatalf("wake body is not JSON: %v (%s)", err, c.body)
	}
	return m
}

// TestWakePreviewPresentWhenEnabled proves HOTLINE_PUSH_PREVIEW=clear puts the
// message plaintext in the wake hint's "preview" field.
func TestWakePreviewPresentWhenEnabled(t *testing.T) {
	cap := newWakeCapture()
	srv, deviceID, _ := previewServer(t, true, cap)
	srv.maybePush(deviceID, textItem("yo from the phone"))
	cap.wait(t)
	m := cap.decode(t)
	if m["preview"] != "yo from the phone" {
		t.Fatalf("preview = %v, want %q", m["preview"], "yo from the phone")
	}
	// preview_c stays present and null (mutually exclusive with preview).
	if pc, ok := m["preview_c"]; !ok || pc != nil {
		t.Fatalf("preview_c must be present and null, got %v (present=%v)", pc, ok)
	}
}

// TestWakePreviewAbsentWhenDisabled proves the default (knob unset) wake hint
// carries NO preview and is byte-identical to the generic form.
func TestWakePreviewAbsentWhenDisabled(t *testing.T) {
	cap := newWakeCapture()
	srv, deviceID, _ := previewServer(t, false, cap)
	srv.maybePush(deviceID, textItem("yo from the phone"))
	cap.wait(t)
	m := cap.decode(t)
	if _, ok := m["preview"]; ok {
		t.Fatalf("preview must be absent when the knob is off, got %v", m["preview"])
	}
	// Wire-identical to the generic hint the pre-feature code produced.
	cap.mu.Lock()
	got := string(cap.body)
	cap.mu.Unlock()
	want := `{"device_id":"dev-af31fd290542","kind":"message","preview_c":null}`
	if got != want {
		t.Fatalf("disabled-knob wake body not wire-identical:\n got %s\nwant %s", got, want)
	}
}

// TestWakePreviewTruncatedRuneSafe proves an over-long message is clipped to 140
// runes (rune-safe) before it leaves the box.
func TestWakePreviewTruncatedRuneSafe(t *testing.T) {
	cap := newWakeCapture()
	srv, deviceID, _ := previewServer(t, true, cap)
	long := strings.Repeat("🎉", 200)
	srv.maybePush(deviceID, textItem(long))
	cap.wait(t)
	m := cap.decode(t)
	prev, _ := m["preview"].(string)
	want := strings.Repeat("🎉", pushBodyMax) + "…"
	if prev != want {
		t.Fatalf("truncated preview mismatch: got %d runes, want %d", utf8.RuneCountInString(prev), utf8.RuneCountInString(want))
	}
	if utf8.RuneCountInString(prev) != pushBodyMax+1 { // +1 for the ellipsis
		t.Fatalf("preview rune count = %d, want %d", utf8.RuneCountInString(prev), pushBodyMax+1)
	}
}

// TestWakePreviewEmptyTextNoPreview proves an element-only / empty-text message
// carries no preview even with the knob on (falls back to generic).
func TestWakePreviewEmptyTextNoPreview(t *testing.T) {
	cap := newWakeCapture()
	srv, deviceID, _ := previewServer(t, true, cap)
	srv.maybePush(deviceID, textItem("   "))
	cap.wait(t)
	m := cap.decode(t)
	if _, ok := m["preview"]; ok {
		t.Fatalf("empty text must yield no preview, got %v", m["preview"])
	}
}

// TestWakePreviewCoveredBySignature proves the box signature is computed over the
// exact body bytes that carry the preview: re-verifying the captured signature
// against the box public key over the canonical string built from the received
// body succeeds, and the body contains the preview.
func TestWakePreviewCoveredBySignature(t *testing.T) {
	cap := newWakeCapture()
	srv, deviceID, _ := previewServer(t, true, cap)
	srv.maybePush(deviceID, textItem("secret snippet"))
	cap.wait(t)

	cap.mu.Lock()
	body, method, path, ts, nonce, sig := cap.body, cap.method, cap.path, cap.timestamp, cap.nonce, cap.signature
	cap.mu.Unlock()

	if !strings.Contains(string(body), `"preview":"secret snippet"`) {
		t.Fatalf("captured body does not carry the preview: %s", body)
	}

	// Rebuild the canonical string from the RECEIVED body and verify the ECDSA
	// signature with the box public key. A pass means the preview bytes are
	// bound by the signature (any tamper changes the body hash → verify fails).
	canon := canonicalString(method, path, ts, nonce, bodyHashBase64URL(body))
	if !verifyLowSForTest(t, &srv.coreClient.priv.PublicKey, []byte(canon), sig) {
		t.Fatal("signature did not verify over the canonical string covering the preview")
	}

	// Negative control: flip the preview and verification must fail.
	tampered := strings.Replace(string(body), "secret snippet", "tampered snippet", 1)
	canonBad := canonicalString(method, path, ts, nonce, bodyHashBase64URL([]byte(tampered)))
	if verifyLowSForTest(t, &srv.coreClient.priv.PublicKey, []byte(canonBad), sig) {
		t.Fatal("signature verified over a tampered preview body; preview is not signed")
	}
}

// verifyLowSForTest verifies a base64url r||s ECDSA-P256/SHA-256 signature over
// data against pub (mirrors the core's WebCrypto verify; low-S is a signer
// obligation, so this test verifier does not re-reject high-S).
func verifyLowSForTest(t *testing.T, pub *ecdsa.PublicKey, data []byte, b64sig string) bool {
	t.Helper()
	raw, err := base64.RawURLEncoding.DecodeString(b64sig)
	if err != nil || len(raw) != 64 {
		t.Fatalf("bad signature encoding: %v len=%d", err, len(raw))
	}
	sum := sha256.Sum256(data)
	r := new(big.Int).SetBytes(raw[:32])
	s := new(big.Int).SetBytes(raw[32:])
	return ecdsa.Verify(pub, sum[:], r, s)
}
