package notify

import (
	"crypto/rand"
	"crypto/subtle"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"time"
)

// ErrSourceNotFound is returned when no source matches a label.
var ErrSourceNotFound = errors.New("no source with that label")

// LoadRegistry reads sources.json. Missing file → empty Registry. Corrupt file →
// moved aside to path+".corrupt", empty Registry returned. (schedule.Load pattern.)
func LoadRegistry(path string) (*Registry, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return &Registry{Sources: []Source{}}, nil
		}
		return nil, err
	}
	var r Registry
	if err := json.Unmarshal(raw, &r); err != nil {
		_ = os.Rename(path, path+".corrupt")
		return &Registry{Sources: []Source{}}, nil
	}
	if r.Sources == nil {
		r.Sources = []Source{}
	}
	return &r, nil
}

// SaveRegistry atomically writes sources.json (tmp file 0600 + rename).
func SaveRegistry(r *Registry, path string) error {
	if r.Sources == nil {
		r.Sources = []Source{}
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	data, err := json.MarshalIndent(r, "", "  ")
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

// MutateRegistry is a flock(LOCK_EX)-guarded read-modify-write on sources.json
// via path+".lock", so concurrent source add/revoke never race.
func MutateRegistry(path string, fn func(*Registry) error) error {
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

	r, err := LoadRegistry(path)
	if err != nil {
		return err
	}
	if err := fn(r); err != nil {
		return err
	}
	return SaveRegistry(r, path)
}

// AddSource mints a fresh capability key for a new label and appends it. Labels
// are unique (they are the provenance handle); cap defaults to normal so urgent
// must be granted deliberately. Runs entirely under MutateRegistry.
func AddSource(path, label string, cap Level, rate Rate, chatID string, now time.Time) (Source, error) {
	label = strings.TrimSpace(label)
	if label == "" {
		return Source{}, fmt.Errorf("label is required")
	}
	cap = capOrDefault(cap)

	var stored Source
	err := MutateRegistry(path, func(r *Registry) error {
		for _, s := range r.Sources {
			if s.Label == label {
				return fmt.Errorf("source %q already exists", label)
			}
		}
		s := Source{
			Label:     label,
			Key:       newKey(),
			LevelCap:  cap,
			Rate:      rate,
			ChatID:    strings.TrimSpace(chatID),
			CreatedAt: now.UTC().Format(time.RFC3339),
		}
		r.Sources = append(r.Sources, s)
		stored = s
		return nil
	})
	if err != nil {
		return Source{}, err
	}
	return stored, nil
}

// RevokeSource removes the source matching label (exact) and returns it. There
// are no tombstones — the audit trail is the transcript plus rejects.log. A
// revoked key immediately fails the gate because every CLI call reads the
// registry fresh.
func RevokeSource(path, label string) (Source, error) {
	var removed Source
	err := MutateRegistry(path, func(r *Registry) error {
		for i, s := range r.Sources {
			if s.Label == label {
				removed = s
				r.Sources = append(r.Sources[:i], r.Sources[i+1:]...)
				return nil
			}
		}
		return fmt.Errorf("%w: %q", ErrSourceNotFound, label)
	})
	if err != nil {
		return Source{}, err
	}
	return removed, nil
}

// newKey returns 128 bits from crypto/rand formatted as a canonical UUIDv4
// string (version/variant bits set manually — the exact shape a bearer token
// needs, zero new dependencies).
func newKey() string {
	var b [16]byte
	_, _ = rand.Read(b[:])
	b[6] = (b[6] & 0x0f) | 0x40 // version 4
	b[8] = (b[8] & 0x3f) | 0x80 // variant 10
	return fmt.Sprintf("%x-%x-%x-%x-%x", b[0:4], b[4:6], b[6:8], b[8:10], b[10:16])
}

// constantTimeEqual compares two keys without a data-dependent early exit.
func constantTimeEqual(a, b string) bool {
	ab, bb := []byte(a), []byte(b)
	if len(ab) != len(bb) {
		return false
	}
	return subtle.ConstantTimeCompare(ab, bb) == 1
}
