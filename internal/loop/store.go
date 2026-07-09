// Package loop implements hotline's script loop registry and runner: a loop is
// a shell command on an interval, with optional routing of non-empty stdout into
// the existing notify gate.
package loop

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"time"
)

const (
	maxLoops       = 50
	defaultTimeout = 120 * time.Second
	minEvery       = 10 * time.Second
)

// ErrNotFound is returned when no loop matches a label.
var ErrNotFound = errors.New("no loop with that label")

// Loop is one persisted script loop. Source is a notify source label, never a
// capability key.
type Loop struct {
	Label          string `json:"label"`
	Every          string `json:"every"`
	Cmd            string `json:"cmd"`
	NotifyLLM      bool   `json:"notifyLlm"`
	Sink           string `json:"sink,omitempty"`
	Source         string `json:"source,omitempty"`
	Level          string `json:"level,omitempty"`
	Timeout        string `json:"timeout,omitempty"`
	Paused         bool   `json:"paused,omitempty"`
	CreatedAt      string `json:"createdAt"`
	LastRunAt      string `json:"lastRunAt,omitempty"`
	LastExit       int    `json:"lastExit,omitempty"`
	LastDurationMs int64  `json:"lastDurationMs,omitempty"`
	Runs           int64  `json:"runs,omitempty"`
	Approved       bool   `json:"approved"`
}

// Doc is the full persisted loops.json document.
type Doc struct {
	Loops []Loop `json:"loops"`
}

// Path returns the registry path under the state root.
func Path(stateRoot string) string { return filepath.Join(stateRoot, "loops.json") }

// Dir returns the directory that holds loop logs and per-loop state dirs.
func Dir(stateRoot string) string { return filepath.Join(stateRoot, "loops") }

// StateDir returns one loop's durable private scratch directory.
func StateDir(stateRoot, label string) string { return filepath.Join(Dir(stateRoot), label, "state") }

// LogPath returns one loop's size-rotated run log path.
func LogPath(stateRoot, label string) string { return filepath.Join(Dir(stateRoot), label+".log") }

// Load reads loops.json. Missing file → empty Doc. Corrupt file → moved aside
// to path+".corrupt", empty Doc returned. (schedule.Load pattern.)
func Load(path string) (*Doc, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return &Doc{Loops: []Loop{}}, nil
		}
		return nil, err
	}
	var d Doc
	if err := json.Unmarshal(raw, &d); err != nil {
		_ = os.Rename(path, path+".corrupt")
		return &Doc{Loops: []Loop{}}, nil
	}
	d.normalize()
	return &d, nil
}

// Save atomically writes loops.json (tmp file 0600 + rename).
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

// Mutate is a flock(LOCK_EX)-guarded read-modify-write on loops.json via
// path+".lock", so the supervisor runner and CLI subcommands never race.
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

func (d *Doc) normalize() {
	if d.Loops == nil {
		d.Loops = []Loop{}
	}
}

// UnmarshalJSON keeps pre-approval-gate loop files compatible: an omitted
// approved field means "approved" for loops created before the gate existed.
func (l *Loop) UnmarshalJSON(data []byte) error {
	type alias Loop
	var raw map[string]json.RawMessage
	if err := json.Unmarshal(data, &raw); err != nil {
		return err
	}
	aux := alias{Approved: true}
	if err := json.Unmarshal(data, &aux); err != nil {
		return err
	}
	if _, ok := raw["approved"]; !ok {
		aux.Approved = true
	}
	*l = Loop(aux)
	return nil
}

type addConfig struct {
	gated      bool
	stateRoot  string
	preApprove bool
	notify     bool
}

// AddOption configures loop creation. Existing direct Add callers remain
// legacy-approved; CLI/MCP setup paths pass WithApprovalGate.
type AddOption func(*addConfig)

// WithApprovalGate makes Add apply the posture-aware gate. preApprove is the
// trusted operator CLI -y path; agents never pass it.
func WithApprovalGate(stateRoot string, preApprove bool) AddOption {
	return func(c *addConfig) {
		c.gated = true
		c.stateRoot = stateRoot
		c.preApprove = preApprove
		c.notify = true
	}
}

// Add validates l, enforces maxLoops, appends it, and returns the stored copy.
func Add(path string, l Loop, now time.Time, opts ...AddOption) (Loop, error) {
	var cfg addConfig
	for _, opt := range opts {
		opt(&cfg)
	}
	if err := normalizeLoop(&l); err != nil {
		return Loop{}, err
	}
	yolo := false
	if cfg.gated {
		var err error
		yolo, err = YoloEnabled(cfg.stateRoot)
		if err != nil {
			return Loop{}, err
		}
		l.Approved = cfg.preApprove || yolo
	} else {
		l.Approved = true
	}

	var stored Loop
	err := Mutate(path, func(d *Doc) error {
		if len(d.Loops) >= maxLoops {
			return fmt.Errorf("too many loops (max %d)", maxLoops)
		}
		for _, existing := range d.Loops {
			if existing.Label == l.Label {
				return fmt.Errorf("loop %q already exists", l.Label)
			}
		}
		l.CreatedAt = now.UTC().Format(time.RFC3339)
		d.Loops = append(d.Loops, l)
		stored = l
		return nil
	})
	if err != nil {
		return Loop{}, err
	}
	if cfg.gated && cfg.notify && !cfg.preApprove {
		_ = notifyOperator(cfg.stateRoot, stored, yolo, now)
	}
	return stored, nil
}

// Remove deletes the loop matching label and returns it.
func Remove(path, label string) (Loop, error) {
	var removed Loop
	err := Mutate(path, func(d *Doc) error {
		i, err := findIndex(d, label)
		if err != nil {
			return err
		}
		removed = d.Loops[i]
		d.Loops = append(d.Loops[:i], d.Loops[i+1:]...)
		return nil
	})
	if err != nil {
		return Loop{}, err
	}
	return removed, nil
}

// SetPaused pauses or resumes a loop.
func SetPaused(path, label string, paused bool) (Loop, error) {
	var updated Loop
	err := Mutate(path, func(d *Doc) error {
		i, err := findIndex(d, label)
		if err != nil {
			return err
		}
		l := d.Loops[i]
		l.Paused = paused
		d.Loops[i] = l
		updated = l
		return nil
	})
	if err != nil {
		return Loop{}, err
	}
	return updated, nil
}

// Approve flips a pending loop live.
func Approve(path, label string) (Loop, error) {
	var updated Loop
	err := Mutate(path, func(d *Doc) error {
		i, err := findIndex(d, label)
		if err != nil {
			return err
		}
		l := d.Loops[i]
		l.Approved = true
		d.Loops[i] = l
		updated = l
		return nil
	})
	if err != nil {
		return Loop{}, err
	}
	return updated, nil
}

// RecordRun updates advisory status for label if the loop still exists.
func RecordRun(path, label string, now time.Time, exit int, dur time.Duration) error {
	return Mutate(path, func(d *Doc) error {
		for i := range d.Loops {
			if d.Loops[i].Label == label {
				d.Loops[i].LastRunAt = now.UTC().Format(time.RFC3339)
				d.Loops[i].LastExit = exit
				d.Loops[i].LastDurationMs = dur.Milliseconds()
				d.Loops[i].Runs++
				return nil
			}
		}
		return nil
	})
}

func findIndex(d *Doc, label string) (int, error) {
	for i, l := range d.Loops {
		if l.Label == label {
			return i, nil
		}
	}
	return -1, fmt.Errorf("%w: %q", ErrNotFound, label)
}

func normalizeLoop(l *Loop) error {
	l.Label = strings.TrimSpace(l.Label)
	l.Every = strings.TrimSpace(l.Every)
	l.Cmd = strings.TrimSpace(l.Cmd)
	l.Sink = strings.TrimSpace(l.Sink)
	l.Source = strings.TrimSpace(l.Source)
	l.Level = strings.TrimSpace(l.Level)
	l.Timeout = strings.TrimSpace(l.Timeout)

	if l.Label == "" {
		return fmt.Errorf("label is required")
	}
	if strings.Contains(l.Label, "/") || strings.Contains(l.Label, string(filepath.Separator)) || l.Label == "." || l.Label == ".." {
		return fmt.Errorf("label %q is not safe for a state directory", l.Label)
	}
	if l.Every == "" {
		return fmt.Errorf("every is required")
	}
	every, err := time.ParseDuration(l.Every)
	if err != nil {
		return fmt.Errorf("every must be a duration: %w", err)
	}
	if every < minEvery {
		every = minEvery
	}
	l.Every = every.String()
	if l.Cmd == "" {
		return fmt.Errorf("cmd is required")
	}
	if l.Sink == "" {
		l.Sink = "notify"
	}
	if l.Sink != "notify" {
		return fmt.Errorf("unsupported sink %q (v1 supports notify)", l.Sink)
	}
	if l.Timeout == "" {
		l.Timeout = defaultTimeout.String()
	} else {
		timeout, err := time.ParseDuration(l.Timeout)
		if err != nil {
			return fmt.Errorf("timeout must be a duration: %w", err)
		}
		if timeout <= 0 {
			return fmt.Errorf("timeout must be positive")
		}
		l.Timeout = timeout.String()
	}
	return nil
}

// EveryDuration returns the parsed interval, after applying the v1 floor.
func (l Loop) EveryDuration() time.Duration {
	d, err := time.ParseDuration(l.Every)
	if err != nil || d < minEvery {
		return minEvery
	}
	return d
}

// TimeoutDuration returns the parsed timeout, falling back to the v1 default.
func (l Loop) TimeoutDuration() time.Duration {
	d, err := time.ParseDuration(l.Timeout)
	if err != nil || d <= 0 {
		return defaultTimeout
	}
	return d
}
