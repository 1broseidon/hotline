package schedule

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"time"
)

// maxSchedules caps the document size, mirroring maxPending's flood guard.
const maxSchedules = 50

// maxPromptLen bounds one stored prompt (matches access.MaxChunkLimit's 4096).
const maxPromptLen = 4096

// ErrNotFound is returned when no schedule matches an id (or unique prefix).
var ErrNotFound = errors.New("no schedule with that id")

// Load reads schedules.json. Missing file → empty Doc. Corrupt file → moved
// aside to path+".corrupt", empty Doc returned. (access.go pattern, with
// Defaults() replaced by an empty Doc.)
func Load(path string) (*Doc, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return &Doc{Schedules: []Schedule{}}, nil
		}
		return nil, err
	}
	var d Doc
	if err := json.Unmarshal(raw, &d); err != nil {
		// Corrupt — preserve it for forensics and start fresh.
		_ = os.Rename(path, path+".corrupt")
		return &Doc{Schedules: []Schedule{}}, nil
	}
	d.normalize()
	return &d, nil
}

// Save atomically writes schedules.json (tmp file 0600 + rename).
func Save(d *Doc, path string) error {
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

// Mutate is a flock(LOCK_EX)-guarded read-modify-write on schedules.json via
// path+".lock", so the ticker, the MCP tool, and CLI subcommands never race.
func Mutate(path string, fn func(*Doc) error) error {
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

	d, err := Load(path)
	if err != nil {
		return err
	}
	if err := fn(d); err != nil {
		return err
	}
	return Save(d, path)
}

// normalize replaces a nil Schedules slice with an empty one so callers can
// rely on a non-nil slice regardless of what the on-disk document omitted.
func (d *Doc) normalize() {
	if d.Schedules == nil {
		d.Schedules = []Schedule{}
	}
}

// Add validates s, enforces maxSchedules, assigns a fresh unique ID and
// CreatedAt, appends, and returns the stored copy. Runs entirely under Mutate.
func Add(path string, s Schedule, now time.Time) (Schedule, error) {
	if err := s.Recurrence.Validate(); err != nil {
		return Schedule{}, err
	}
	if strings.TrimSpace(s.Prompt) == "" {
		return Schedule{}, fmt.Errorf("prompt is required")
	}
	if len(s.Prompt) > maxPromptLen {
		return Schedule{}, fmt.Errorf("prompt too long (%d > %d)", len(s.Prompt), maxPromptLen)
	}
	if strings.TrimSpace(s.ChatID) == "" {
		return Schedule{}, fmt.Errorf("chat_id is required")
	}
	if _, err := time.Parse(time.RFC3339, s.NextFire); err != nil {
		return Schedule{}, fmt.Errorf("nextFire must be RFC3339: %w", err)
	}

	var stored Schedule
	err := Mutate(path, func(d *Doc) error {
		if len(d.Schedules) >= maxSchedules {
			return fmt.Errorf("too many schedules (max %d)", maxSchedules)
		}
		taken := func(id string) bool {
			for _, sc := range d.Schedules {
				if sc.ID == id {
					return true
				}
			}
			return false
		}
		s.ID = newID(taken)
		s.CreatedAt = now.UTC().Format(time.RFC3339)
		d.Schedules = append(d.Schedules, s)
		stored = s
		return nil
	})
	if err != nil {
		return Schedule{}, err
	}
	return stored, nil
}

// Remove deletes the schedule matching id (exact, else unique prefix) and
// returns it.
func Remove(path, id string) (Schedule, error) {
	var removed Schedule
	err := Mutate(path, func(d *Doc) error {
		i, err := findIndex(d, id)
		if err != nil {
			return err
		}
		removed = d.Schedules[i]
		d.Schedules = append(d.Schedules[:i], d.Schedules[i+1:]...)
		return nil
	})
	if err != nil {
		return Schedule{}, err
	}
	return removed, nil
}

// SetPaused pauses or resumes a schedule. Resuming a recurring schedule
// recomputes NextFire from now via First (so a long-paused daily doesn't
// instantly fire a stale catch-up); resuming a once keeps its stored time.
// Pausing never touches NextFire.
func SetPaused(path, id string, paused bool, now time.Time, loc *time.Location) (Schedule, error) {
	var updated Schedule
	err := Mutate(path, func(d *Doc) error {
		i, err := findIndex(d, id)
		if err != nil {
			return err
		}
		sc := d.Schedules[i]
		if !paused && sc.Recurrence.Kind != KindOnce {
			next, err := First(sc.Recurrence, time.Time{}, now, loc)
			if err != nil {
				return err
			}
			sc.NextFire = next.UTC().Format(time.RFC3339)
		}
		sc.Paused = paused
		d.Schedules[i] = sc
		updated = sc
		return nil
	})
	if err != nil {
		return Schedule{}, err
	}
	return updated, nil
}

// newID returns a fresh 6-hex id (crypto/rand, 3 bytes — NewPairingCode's
// recipe) that is not already taken; loops on the astronomically-rare
// collision. Called only under Mutate's lock, so uniqueness holds across
// concurrent creators.
func newID(taken func(string) bool) string {
	for {
		var b [3]byte
		_, _ = rand.Read(b[:])
		id := hex.EncodeToString(b[:])
		if !taken(id) {
			return id
		}
	}
}

// findIndex resolves id → index with exact match, else unique-prefix rules.
// 0 matches → ErrNotFound; >1 → ambiguous error listing the candidates.
func findIndex(d *Doc, id string) (int, error) {
	for i, sc := range d.Schedules {
		if sc.ID == id {
			return i, nil
		}
	}
	var candidates []int
	var ids []string
	for i, sc := range d.Schedules {
		if strings.HasPrefix(sc.ID, id) {
			candidates = append(candidates, i)
			ids = append(ids, sc.ID)
		}
	}
	switch len(candidates) {
	case 1:
		return candidates[0], nil
	case 0:
		return -1, fmt.Errorf("%w: %q", ErrNotFound, id)
	default:
		return -1, fmt.Errorf("ambiguous id %q: matches %s", id, strings.Join(ids, ", "))
	}
}
