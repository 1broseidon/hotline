// Package transcript provides a durable, append-only JSONL log of the channel's
// conversation — every inbound message relayed to Claude and every outbound
// reply. Telegram has no history API, and a Claude Code session restarts or
// compacts its context over time; the transcript is what lets the assistant
// recall the thread across those resets. It lives in the shared per-token state
// dir so the single conversation stays coherent regardless of which Claude Code
// session currently holds the channel.
package transcript

import (
	"encoding/json"
	"os"
	"sync"
	"time"
)

// Record is one line in the transcript. Empty optional fields are omitted.
type Record struct {
	TS        string `json:"ts"`
	Dir       string `json:"dir"` // "in" (from the user) | "out" (from Claude)
	ChatID    string `json:"chat_id,omitempty"`
	User      string `json:"user,omitempty"`
	UserID    string `json:"user_id,omitempty"`
	Kind      string `json:"kind,omitempty"` // text | photo | reaction | button | reply | ...
	MessageID string `json:"message_id,omitempty"`
	Text      string `json:"text"`
}

// Logger appends records to a JSONL file. It is safe for concurrent use: the
// inbound poll goroutine and the outbound tool-call goroutine can write at once.
type Logger struct {
	mu   sync.Mutex
	path string
	now  func() time.Time // injectable for tests
}

// New returns a Logger writing to path. A nil *Logger is a valid no-op, so
// callers can hold an optional logger without nil-checking every call site.
func New(path string) *Logger {
	return &Logger{path: path, now: time.Now}
}

// Append writes one record as a JSON line. It stamps TS (UTC) when unset. Errors
// are returned for the caller to log; a failed write never blocks the channel.
// A nil receiver is a no-op.
func (l *Logger) Append(r Record) error {
	if l == nil {
		return nil
	}
	if r.TS == "" {
		r.TS = l.now().UTC().Format("2006-01-02T15:04:05.000Z07:00")
	}
	line, err := json.Marshal(r)
	if err != nil {
		return err
	}
	line = append(line, '\n')

	l.mu.Lock()
	defer l.mu.Unlock()
	// Open per write: robust to external rotation/deletion, and at human texting
	// rates the cost is irrelevant. O_APPEND keeps lines from interleaving. 0600
	// because the file holds conversation content.
	f, err := os.OpenFile(l.path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600)
	if err != nil {
		return err
	}
	defer f.Close()
	_, err = f.Write(line)
	return err
}
