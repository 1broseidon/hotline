package app

import (
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"time"
)

// connectorLogFile is the persistent connector-lifecycle log under the state
// dir. The box's relay /c dial / connect / disconnect / backoff events, and the
// WebSocket close code on every /c drop, historically went ONLY to os.Stderr.
// Under the supervised pi/claude harness the box runs as a child whose stderr is
// consumed into the harness's terminal (a tmux pane) and never persisted, so a
// field flap — e.g. the 2026-07-17 incident in which a client reconnect set one
// box's per-room /c to relay.hotline.dev churning while another box's held — left
// no on-disk trace to attribute (frame-rate 1013 self-close vs 4001 replaced vs a
// harness-recycle restart are indistinguishable after the fact). This log
// records those events to a file so the next occurrence is diagnosable.
const connectorLogFile = "connector.log"

// connLogger appends timestamped connector-lifecycle lines to a file. It opens
// the file per write — connector events are infrequent (one per reconnect, not
// per frame) — so it never holds an fd open and never loses buffered data on a
// crash. A nil *connLogger is a valid no-op, and every error is swallowed:
// instrumentation must never perturb the box's serving path.
type connLogger struct {
	mu   sync.Mutex
	path string
}

// openConnLogger returns a connLogger writing under stateDir. It never fails the
// box: an empty stateDir yields nil (writes no-op) and an unwritable path is
// discovered lazily at write time and swallowed — the connector's pre-existing
// os.Stderr lines are untouched either way.
func openConnLogger(stateDir string) *connLogger {
	if stateDir == "" {
		return nil
	}
	return &connLogger{path: filepath.Join(stateDir, connectorLogFile)}
}

// logf appends one UTC-timestamped line. Best-effort and nil-safe.
func (c *connLogger) logf(format string, args ...any) {
	if c == nil || c.path == "" {
		return
	}
	line := time.Now().UTC().Format("2006-01-02T15:04:05.000Z") + " " + fmt.Sprintf(format, args...) + "\n"
	c.mu.Lock()
	defer c.mu.Unlock()
	f, err := os.OpenFile(c.path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600)
	if err != nil {
		return
	}
	_, _ = f.WriteString(line)
	_ = f.Close()
}
