package supervise

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"time"
)

// File names inside the supervisor state dir. Liveness truth is the held
// flock on lockName (immune to pid reuse and stale JSON — the same primitive
// access.Mutate and the poller-slot claim rely on); state.json is detail.
const (
	stateName   = "state.json"
	lockName    = "lock"
	requestName = "restart.request"

	// SupervisorLogName receives the detached supervisor's own stdout/stderr
	// (its event lines). HarnessLogName receives the harness's pty output,
	// size-rotated by RotatingWriter.
	SupervisorLogName = "supervisor.log"
	HarnessLogName    = "harness.log"
)

// EnvDir is the environment variable the supervisor sets on the harness so
// the hotline MCP child (a grandchild) knows where to write restart requests
// — it gates registration of the restart tool.
const EnvDir = "HOTLINE_SUPERVISOR_DIR"

// Dir returns the supervisor state directory under the shared state root.
func Dir(stateRoot string) string { return filepath.Join(stateRoot, "supervisor") }

// Supervisor phases recorded in state.json.
const (
	PhaseRunning = "running" // harness up
	PhaseBackoff = "backoff" // harness down, waiting to restart
	PhaseStopped = "stopped" // supervisor exited cleanly
)

// State is the supervisor's persisted status document, written with the
// tmp+rename discipline the rest of the state dir uses. It is advisory
// detail for `hotline status` and `hotline down`; Running() is the truth.
type State struct {
	PID              int      `json:"pid"`
	Phase            string   `json:"phase"`
	StartedAt        string   `json:"startedAt"`
	HarnessPID       int      `json:"harnessPid,omitempty"`
	HarnessStartedAt string   `json:"harnessStartedAt,omitempty"`
	Restarts         int      `json:"restarts"`
	LastExit         string   `json:"lastExit,omitempty"`
	NextRestartAt    string   `json:"nextRestartAt,omitempty"`
	Argv             []string `json:"argv,omitempty"`
	WorkDir          string   `json:"workDir,omitempty"`
}

// WriteState atomically writes state.json (tmp 0600 + rename).
func WriteState(dir string, st *State) error {
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return err
	}
	data, err := json.MarshalIndent(st, "", "  ")
	if err != nil {
		return err
	}
	data = append(data, '\n')
	path := filepath.Join(dir, stateName)
	tmp := fmt.Sprintf("%s.%d.tmp", path, os.Getpid())
	if err := os.WriteFile(tmp, data, 0o600); err != nil {
		return err
	}
	if err := os.Rename(tmp, path); err != nil {
		_ = os.Remove(tmp)
		return err
	}
	return nil
}

// ReadState reads state.json. A missing file returns (nil, nil).
func ReadState(dir string) (*State, error) {
	raw, err := os.ReadFile(filepath.Join(dir, stateName))
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}
	var st State
	if err := json.Unmarshal(raw, &st); err != nil {
		return nil, fmt.Errorf("parsing %s: %w", stateName, err)
	}
	return &st, nil
}

// AcquireLock takes the supervisor singleton: an exclusive non-blocking flock
// on the lock file, held until release is called (in practice, for the
// supervisor's lifetime — the held lock IS the liveness signal). A second
// supervisor on the same state root fails here instead of double-spawning
// harnesses.
func AcquireLock(dir string) (release func(), err error) {
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return nil, err
	}
	lockPath := filepath.Join(dir, lockName)
	lf, err := os.OpenFile(lockPath, os.O_CREATE|os.O_RDWR, 0o600)
	if err != nil {
		return nil, err
	}
	if err := syscall.Flock(int(lf.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		lf.Close()
		return nil, fmt.Errorf("locking %s: %w", lockPath, err)
	}
	return func() {
		_ = syscall.Flock(int(lf.Fd()), syscall.LOCK_UN)
		lf.Close()
	}, nil
}

// Running reports whether a supervisor currently holds the lock for dir.
func Running(dir string) bool {
	lf, err := os.Open(filepath.Join(dir, lockName))
	if err != nil {
		return false
	}
	defer lf.Close()
	if err := syscall.Flock(int(lf.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		return true // held elsewhere
	}
	_ = syscall.Flock(int(lf.Fd()), syscall.LOCK_UN)
	return false
}

// RequestRestart asks a running supervisor to bounce the harness by writing
// the restart.request control file (tmp+rename; the supervisor's poll loop
// consumes it). reason is flattened to one bounded line — it is only ever
// logged, never executed. Callers: the restart MCP tool (chat path) and the
// supervisor's own SIGHUP handler.
func RequestRestart(dir, reason string) error {
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return err
	}
	reason = sanitizeReason(reason)
	body := fmt.Sprintf("%s %s\n", time.Now().UTC().Format(time.RFC3339), reason)
	path := filepath.Join(dir, requestName)
	tmp := fmt.Sprintf("%s.%d.tmp", path, os.Getpid())
	if err := os.WriteFile(tmp, []byte(body), 0o600); err != nil {
		return err
	}
	if err := os.Rename(tmp, path); err != nil {
		_ = os.Remove(tmp)
		return err
	}
	return nil
}

// consumeRestart removes a pending restart request, returning its reason and
// whether one existed. Remove-then-read would race the writer's rename, so it
// reads first; the rename-based writer guarantees the read never sees a
// partial file.
func consumeRestart(dir string) (reason string, ok bool) {
	path := filepath.Join(dir, requestName)
	raw, err := os.ReadFile(path)
	if err != nil {
		return "", false
	}
	_ = os.Remove(path)
	line := strings.TrimSpace(string(raw))
	if _, rest, found := strings.Cut(line, " "); found {
		return rest, true
	}
	return line, true
}

// sanitizeReason flattens a restart reason to a single bounded line: it comes
// from chat (a prompt-injection surface) and is written into logs.
func sanitizeReason(s string) string {
	s = strings.Join(strings.Fields(s), " ")
	if len(s) > 200 {
		s = s[:200]
	}
	if s == "" {
		s = "(no reason given)"
	}
	return s
}
