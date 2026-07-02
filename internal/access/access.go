// Package access implements the channel's access-control model: DM policy
// (pairing / allowlist / disabled), per-group policy with mention-gating, and
// the pairing lifecycle. State lives in access.json and is re-read on every
// inbound message so the operator can edit it live.
package access

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"syscall"
)

// MaxChunkLimit is Telegram's hard per-message character cap.
const MaxChunkLimit = 4096

// GroupPolicy controls delivery from a single supergroup.
type GroupPolicy struct {
	RequireMention bool     `json:"requireMention"`
	AllowFrom      []string `json:"allowFrom,omitempty"`
}

// Pending is a not-yet-approved pairing request, keyed by a 6-hex code.
type Pending struct {
	SenderID  string `json:"senderId"`
	ChatID    string `json:"chatId"`
	CreatedAt string `json:"createdAt"`
	ExpiresAt string `json:"expiresAt"`
	Replies   int    `json:"replies"`
}

// Access is the full persisted access-control document.
type Access struct {
	DMPolicy        string                 `json:"dmPolicy"`
	AllowFrom       []string               `json:"allowFrom"`
	Groups          map[string]GroupPolicy `json:"groups"`
	Pending         map[string]Pending     `json:"pending,omitempty"`
	MentionPatterns []string               `json:"mentionPatterns,omitempty"`
	AckReaction     string                 `json:"ackReaction,omitempty"`
	ReplyToMode     string                 `json:"replyToMode,omitempty"`
	TextChunkLimit  int                    `json:"textChunkLimit,omitempty"`
	ChunkMode       string                 `json:"chunkMode,omitempty"`
	// BubbleMode controls how a multi-bubble reply is delivered: "paced"
	// (default) inserts a typing indicator and a length-scaled pause between
	// bubbles for a human texting cadence; "instant" sends them back-to-back.
	BubbleMode string `json:"bubbleMode,omitempty"`
}

// Defaults returns a fresh Access with the default pairing policy.
func Defaults() *Access {
	return &Access{
		DMPolicy:       "pairing",
		AllowFrom:      []string{},
		Groups:         map[string]GroupPolicy{},
		Pending:        map[string]Pending{},
		ReplyToMode:    "first",
		TextChunkLimit: MaxChunkLimit,
		ChunkMode:      "newline",
		BubbleMode:     "paced",
	}
}

// Load reads access.json. A missing file yields Defaults(). Field defaults are
// applied and TextChunkLimit is clamped to 1..4096. A corrupt file is moved
// aside and Defaults() is returned.
func Load(path string) (*Access, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return Defaults(), nil
		}
		return nil, err
	}
	var a Access
	if err := json.Unmarshal(raw, &a); err != nil {
		// Corrupt — preserve it for forensics and start fresh.
		_ = os.Rename(path, path+".corrupt")
		return Defaults(), nil
	}
	a.normalize()
	return &a, nil
}

// Save atomically writes access.json (tmp file 0600 + rename).
func Save(a *Access, path string) error {
	a.normalize()
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	data, err := json.MarshalIndent(a, "", "  ")
	if err != nil {
		return err
	}
	data = append(data, '\n')
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, data, 0o600); err != nil {
		return err
	}
	if err := os.Rename(tmp, path); err != nil {
		_ = os.Remove(tmp)
		return err
	}
	return nil
}

// Mutate performs a flock(LOCK_EX)-guarded read-modify-write on access.json so
// concurrent processes (poller + pair/deny subcommands) never lose updates.
func Mutate(path string, fn func(*Access) error) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	lockPath := path + ".lock"
	lf, err := os.OpenFile(lockPath, os.O_CREATE|os.O_RDWR, 0o600)
	if err != nil {
		return err
	}
	defer lf.Close()

	if err := syscall.Flock(int(lf.Fd()), syscall.LOCK_EX); err != nil {
		return fmt.Errorf("locking %s: %w", lockPath, err)
	}
	defer syscall.Flock(int(lf.Fd()), syscall.LOCK_UN)

	a, err := Load(path)
	if err != nil {
		return err
	}
	if err := fn(a); err != nil {
		return err
	}
	return Save(a, path)
}

// normalize applies field defaults and clamps so callers can rely on sane
// values regardless of what the on-disk document omitted.
func (a *Access) normalize() {
	if a.DMPolicy == "" {
		a.DMPolicy = "pairing"
	}
	if a.AllowFrom == nil {
		a.AllowFrom = []string{}
	}
	if a.Groups == nil {
		a.Groups = map[string]GroupPolicy{}
	}
	if a.Pending == nil {
		a.Pending = map[string]Pending{}
	}
	if a.ReplyToMode == "" {
		a.ReplyToMode = "first"
	}
	if a.ChunkMode == "" {
		a.ChunkMode = "newline"
	}
	if a.BubbleMode == "" {
		a.BubbleMode = "paced"
	}
	if a.TextChunkLimit == 0 {
		a.TextChunkLimit = MaxChunkLimit
	}
	if a.TextChunkLimit < 1 {
		a.TextChunkLimit = 1
	}
	if a.TextChunkLimit > MaxChunkLimit {
		a.TextChunkLimit = MaxChunkLimit
	}
}

// ErrNotPending is returned by ApprovePairing / DenyPairing when no pending
// entry matches the supplied code.
var ErrNotPending = errors.New("no pending pairing for that code")

// ErrSenderNotAllowed is returned by RevokeSender when no allowFrom entry
// matches the supplied sender id (or a unique prefix of one).
var ErrSenderNotAllowed = errors.New("sender not in allowFrom")
