package app

import (
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/gorilla/websocket"
)

// TestConnLoggerAppendsTimestampedLines: each logf call appends exactly one
// UTC-timestamped line carrying the formatted message.
func TestConnLoggerAppendsTimestampedLines(t *testing.T) {
	dir := t.TempDir()
	c := openConnLogger(dir)
	if c == nil {
		t.Fatal("openConnLogger returned nil for a valid dir")
	}
	c.logf("room=%s /c connected", "Ab3dEf6h")
	c.logf("room=%s /c closed code=%d reason=%q", "Ab3dEf6h", 1013, "frame rate exceeded")

	raw, err := os.ReadFile(filepath.Join(dir, connectorLogFile))
	if err != nil {
		t.Fatalf("read connector.log: %v", err)
	}
	lines := strings.Split(strings.TrimRight(string(raw), "\n"), "\n")
	if len(lines) != 2 {
		t.Fatalf("got %d lines, want 2: %q", len(lines), string(raw))
	}
	if !strings.Contains(lines[0], "room=Ab3dEf6h /c connected") {
		t.Fatalf("line 0 missing message: %q", lines[0])
	}
	if !strings.Contains(lines[1], "code=1013") || !strings.Contains(lines[1], "frame rate exceeded") {
		t.Fatalf("line 1 missing close-code detail: %q", lines[1])
	}
	// Each line must start with an RFC3339-ish UTC timestamp (parseable prefix).
	for i, ln := range lines {
		ts := strings.SplitN(ln, " ", 2)[0]
		if _, err := time.Parse("2006-01-02T15:04:05.000Z", ts); err != nil {
			t.Fatalf("line %d has no valid timestamp prefix %q: %v", i, ts, err)
		}
	}
}

// TestConnLoggerNilSafe: a nil logger and an empty-stateDir logger must both be
// silent no-ops — instrumentation can never panic the box.
func TestConnLoggerNilSafe(t *testing.T) {
	var c *connLogger
	c.logf("must not panic %d", 1) // nil receiver

	if got := openConnLogger(""); got != nil {
		t.Fatalf("openConnLogger(\"\") = %v, want nil", got)
	}
	openConnLogger("").logf("also must not panic") // nil from empty dir
}

// TestServeV2ConnLogsCloseCode is the field-repro instrumentation regression:
// when the relay closes the box's /c with a specific code (here 1013 "frame
// rate exceeded", the documented flood-flap close, and 4001 "replaced"), that
// code lands in connector.log so a bounce is attributable. The harness's
// httptest server hands each upgraded socket to serveV2Conn as the /c pipe, so
// closing the dialed end drives the reader's close-code path.
func TestServeV2ConnLogsCloseCode(t *testing.T) {
	for _, tc := range []struct {
		name   string
		code   int
		reason string
	}{
		{"frame_rate_1013", 1013, "frame rate exceeded"},
		{"replaced_4001", 4001, "replaced"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			h := newSessionHarness(t, "")
			logPath := h.srv.connLog.path

			conn := h.dial()
			// Close the /c end with the target code before any hello; the box's
			// reader observes it as a *websocket.CloseError and logs it.
			_ = conn.WriteControl(
				websocket.CloseMessage,
				websocket.FormatCloseMessage(tc.code, tc.reason),
				time.Now().Add(time.Second),
			)
			_ = conn.Close()

			want := "code=" + strconv.Itoa(tc.code)
			deadline := time.Now().Add(3 * time.Second)
			for {
				raw, _ := os.ReadFile(logPath)
				if strings.Contains(string(raw), want) && strings.Contains(string(raw), "/c closed") {
					break
				}
				if time.Now().After(deadline) {
					t.Fatalf("connector.log never recorded %q; got:\n%s", want, string(raw))
				}
				time.Sleep(20 * time.Millisecond)
			}
		})
	}
}
