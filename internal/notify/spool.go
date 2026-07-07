package notify

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"syscall"
	"time"
)

// LoadSpool reads spool.json. Missing file → empty doc. Corrupt file → moved
// aside to path+".corrupt", empty doc returned. (schedule.Load pattern.)
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
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	data, err := json.MarshalIndent(d, "", "  ")
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

// MutateSpool is a flock(LOCK_EX)-guarded read-modify-write on spool.json via
// path+".lock", so the CLI gate and the daemon's dispatcher never race — the
// same CLI-mutates-while-daemon-ticks concurrency schedules already live with.
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

// OutcomeStatus is the gate's decision, mapped by the CLI to an exit code.
type OutcomeStatus int

const (
	Accepted          OutcomeStatus = iota // durably enqueued as ready
	Duplicate                              // coalesced into an existing/recent identical event
	Queued                                 // valid but held for quiet hours
	RejectedUnknown                        // unknown or revoked source key
	RejectedRate                           // rate-limit suppressed
	RejectedSpoolFull                      // spool at capacity
)

// Outcome carries everything the CLI needs to print the right line and exit code.
type Outcome struct {
	Status          OutcomeStatus
	Label           string
	Level           Level
	Clamped         bool
	ClampedTo       Level
	Count           int    // for Duplicate
	QueuedUntil     string // "HH:MM", for Queued
	Suppressed      int    // for RejectedRate
	SuppressedSince string // "HH:MM", for RejectedRate
}

// Enqueue runs the full gate for one notify and durably records the result. The
// registry is read (fresh, unlocked — atomic rename makes that safe) by the
// caller and passed in; the whole check-and-record runs inside the spool's flock
// critical section so a crashlooping caller cannot race the bucket. Returns an
// error only for internal failures (I/O, lock, quiet-hours parse) → exit 1; the
// gate decision rides Outcome.
func Enqueue(spoolPath, rejectsPath string, reg *Registry, key string, level Level, rawMessage string, now time.Time) (Outcome, error) {
	// 1. key → source (constant-time over the registry).
	src, ok := reg.FindByKey(key)
	if !ok {
		appendReject(rejectsPath, key, len(rawMessage), now)
		return Outcome{Status: RejectedUnknown}, nil
	}
	cap := capOrDefault(src.LevelCap)
	// 2. level clamp.
	lvl, clamped := ClampLevel(level, cap)
	// 3. sanitize (once, at enqueue).
	msg := Sanitize(rawMessage)
	hash := hashMessage(msg)
	// Quiet-hours config is parsed before the lock: a bad value fails the call
	// (exit 1) rather than fail-open into a 3am buzz or fail-closed into a drop.
	qh, err := parseQuietHours(reg.QuietHours)
	if err != nil {
		return Outcome{}, err
	}
	burst, refillMins := effectiveRate(src.Rate)

	var out Outcome
	err = MutateSpool(spoolPath, func(d *SpoolDoc) error {
		st := d.stateFor(src.Label)

		// 4. dedup: identical message from this source within the window.
		if st.LastHash == hash && !parseTime(st.LastHashAt).IsZero() &&
			now.Sub(parseTime(st.LastHashAt)) < dedupWindow {
			st.LastHash = hash
			st.LastHashAt = rfc(now) // rolling window
			count := 1
			for i := range d.Pending {
				if d.Pending[i].Label == src.Label && d.Pending[i].Hash == hash {
					d.Pending[i].Count++
					d.Pending[i].LastAt = rfc(now)
					count = d.Pending[i].Count
					break
				}
			}
			out = Outcome{Status: Duplicate, Label: src.Label, Level: lvl, Count: count, Clamped: clamped, ClampedTo: cap}
			return nil
		}

		// 5. rate limit: token bucket. Urgent does NOT bypass — crashloop
		// protection holds unconditionally.
		tokens := refill(st.Tokens, parseTime(st.TokensAt), now, burst, refillMins)
		if tokens < 1 {
			st.Tokens = tokens
			st.TokensAt = rfc(now)
			st.Suppressed++
			if st.SuppressedSince == "" {
				st.SuppressedSince = rfc(now)
			}
			out = Outcome{Status: RejectedRate, Label: src.Label, Suppressed: st.Suppressed, SuppressedSince: hhmm(parseTime(st.SuppressedSince))}
			return nil
		}

		// Spool capacity backstop (rate limiting has almost certainly kicked in
		// first). Only entries that would be appended hit this.
		if len(d.Pending) >= maxPending {
			out = Outcome{Status: RejectedSpoolFull, Label: src.Label}
			return nil
		}

		// 6. quiet hours: normal/low queue; only urgent passes through.
		status := statusReady
		outStatus := Accepted
		if qh.contains(now) && lvl != LevelUrgent {
			status = statusQueued
			outStatus = Queued
		}

		// 7. spend a token, record the fingerprint, append the entry.
		st.Tokens = tokens - 1
		st.TokensAt = rfc(now)
		st.LastHash = hash
		st.LastHashAt = rfc(now)
		st.LastSeen = rfc(now)
		d.Pending = append(d.Pending, Entry{
			ID:      newEntryID(d),
			Label:   src.Label,
			Level:   lvl,
			Message: msg,
			Hash:    hash,
			Status:  status,
			Count:   1,
			Clamped: clamped,
			FirstAt: rfc(now),
			LastAt:  rfc(now),
		})
		out = Outcome{Status: outStatus, Label: src.Label, Level: lvl, Clamped: clamped, ClampedTo: cap}
		if outStatus == Queued {
			out.QueuedUntil = qh.endLabel()
		}
		return nil
	})
	if err != nil {
		return Outcome{}, err
	}
	return out, nil
}

// newEntryID returns a fresh 6-hex id not already present in the spool (schedule
// newID's recipe). Called only under the spool lock.
func newEntryID(d *SpoolDoc) string {
	taken := func(id string) bool {
		for _, e := range d.Pending {
			if e.ID == id {
				return true
			}
		}
		return false
	}
	for {
		var b [3]byte
		_, _ = rand.Read(b[:])
		id := hex.EncodeToString(b[:])
		if !taken(id) {
			return id
		}
	}
}

// appendReject records an unknown/revoked-key attempt. Best-effort: a failed
// write is not worth failing the call over. The key is truncated to an 8-char
// prefix (enough to find a stale script, useless to replay) and the message
// content is deliberately NOT logged (it is unauthenticated input).
func appendReject(path, key string, msgBytes int, now time.Time) {
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return
	}
	prefix := key
	if len(prefix) > 8 {
		prefix = prefix[:8]
	}
	line := fmt.Sprintf("%s key=%s (prefix) msg-bytes=%d\n", now.UTC().Format(time.RFC3339), prefix, msgBytes)
	f, err := os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600)
	if err != nil {
		return
	}
	defer f.Close()
	_, _ = f.WriteString(line)
}
