package jobspool

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"syscall"
	"time"
	"unicode/utf8"
)

// LoadSpool reads spool.json. Missing → empty doc. Corrupt → moved aside to
// path+".corrupt", empty doc returned. (notify.LoadSpool pattern.)
func LoadSpool(path string) (*SpoolDoc, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			d := &SpoolDoc{}
			d.normalize()
			return d, nil
		}
		return nil, err
	}
	var d SpoolDoc
	if err := json.Unmarshal(raw, &d); err != nil {
		_ = os.Rename(path, path+".corrupt")
		d := &SpoolDoc{}
		d.normalize()
		return d, nil
	}
	d.normalize()
	return &d, nil
}

// SaveSpool atomically writes spool.json (tmp file 0600 + rename).
func SaveSpool(d *SpoolDoc, path string) error {
	d.normalize()
	return atomicWriteJSON(path, d)
}

// MutateSpool is a flock(LOCK_EX)-guarded read-modify-write on spool.json via
// path+".lock", so the CLI append and the daemon's drain never race (the same
// idiom notify/schedules use).
func MutateSpool(path string, fn func(*SpoolDoc) error) error {
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

	d, err := LoadSpool(path)
	if err != nil {
		return err
	}
	if err := fn(d); err != nil {
		return err
	}
	return SaveSpool(d, path)
}

// EnqueueOutcome is what Enqueue reports back to the CLI for its exit line.
type EnqueueOutcome int

const (
	Enqueued  EnqueueOutcome = iota // durably appended
	SpoolFull                       // at capacity — backpressure
)

// Enqueue appends one validated intent to the spool under the flock. action is
// start|update|done; the caller has already validated the shape. Returns
// SpoolFull (never an error) when the spool is at capacity, so a crashlooping
// hook degrades rather than failing the harness.
func Enqueue(spoolPath string, in Intent, now time.Time) (EnqueueOutcome, error) {
	in.Title = clampField(in.Title)
	in.Detail = clampField(in.Detail)
	in.Notify = clampField(in.Notify)
	if in.ChatID == "" {
		in.ChatID = "app"
	}
	in.At = now.UTC().Format(time.RFC3339)
	var out EnqueueOutcome
	err := MutateSpool(spoolPath, func(d *SpoolDoc) error {
		if len(d.Pending) >= maxPending {
			out = SpoolFull
			return nil
		}
		d.Seq++
		in.Seq = d.Seq
		d.Pending = append(d.Pending, in)
		out = Enqueued
		return nil
	})
	return out, err
}

// LoadActive reads activejobs.json (missing → empty, corrupt → aside + empty).
func LoadActive(path string) (*ActiveDoc, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			d := &ActiveDoc{}
			d.normalize()
			return d, nil
		}
		return nil, err
	}
	var d ActiveDoc
	if err := json.Unmarshal(raw, &d); err != nil {
		_ = os.Rename(path, path+".corrupt")
		d := &ActiveDoc{}
		d.normalize()
		return d, nil
	}
	d.normalize()
	return &d, nil
}

// SaveActive atomically writes activejobs.json. Sole writer is the dispatcher, so
// no lock is needed — the atomic rename guards a concurrent reader (e.g. an
// operator `cat`).
func SaveActive(d *ActiveDoc, path string) error {
	d.normalize()
	return atomicWriteJSON(path, d)
}

func atomicWriteJSON(path string, v any) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	data, err := json.MarshalIndent(v, "", "  ")
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

// clampField truncates a text field to maxField at a UTF-8 boundary.
func clampField(s string) string {
	if len(s) <= maxField {
		return s
	}
	cut := 0
	for i, r := range s {
		size := utf8.RuneLen(r)
		if i+size > maxField {
			break
		}
		cut = i + size
	}
	return s[:cut]
}
