// Package supervise implements hotline's always-on supervisor (`hotline up`):
// it owns the harness process (Claude Code on a supervisor-allocated pty, or
// `opencode serve` on plain pipes), restarts it on exit with exponential
// backoff, honors restart requests from the control file (written by the
// restart MCP tool or SIGHUP), and persists its status to the state dir for
// `hotline status` / `hotline down`.
//
// The supervisor composes with internal/lifecycle rather than duplicating it:
// it never touches the hotline MCP child directly. Killing the harness's
// process group ends the harness; hotline then exits through its existing
// stdin-EOF / orphan-watchdog shutdown paths, and a hard kill is recovered by
// the stale-poller claim on the next boot.
package supervise

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"time"
)

// ErrUnsupported is returned by StartOnPTY and StartPiped on platforms
// without the unix spawn machinery (pty allocation for the claude path,
// Setsid process groups and group signalling for both).
var ErrUnsupported = errors.New("hotline up is supported on linux and macOS only")

// Harness is one running harness process as the supervisor sees it. ExitDesc
// is valid once Done is closed. Terminate/Kill signal the whole process
// group, so the harness's own children (the hotline MCP server, its tunnels)
// go down with it.
type Harness interface {
	Pid() int
	Done() <-chan struct{}
	ExitDesc() string
	Terminate()
	Kill()
}

// Supervisor runs the start → watch → backoff → restart loop. Start, now, and
// sleep are seams so tests inject fake harnesses and never really wait
// (scheduler_test.go style).
type Supervisor struct {
	Dir     string                                     // supervisor state dir
	Start   func(ctx context.Context) (Harness, error) // spawns one harness
	Backoff *Backoff
	Grace   time.Duration // SIGTERM → SIGKILL escalation window
	Poll    time.Duration // restart.request poll interval
	Log     io.Writer     // event log (stderr; the detached parent points stderr at supervisor.log)
	Argv    []string      // recorded in state.json for status display
	WorkDir string        // recorded in state.json

	now   func() time.Time
	sleep func(ctx context.Context, d time.Duration) bool // false when ctx ended first
}

// New builds a Supervisor with production defaults: 2s→10m backoff resetting
// after 5m of healthy uptime, a 10s kill grace, and a 2s control-file poll.
func New(dir string, start func(ctx context.Context) (Harness, error)) *Supervisor {
	return &Supervisor{
		Dir:     dir,
		Start:   start,
		Backoff: &Backoff{Initial: 2 * time.Second, Max: 10 * time.Minute, Healthy: 5 * time.Minute},
		Grace:   10 * time.Second,
		Poll:    2 * time.Second,
		Log:     os.Stderr,
		now:     time.Now,
		sleep:   sleepCtx,
	}
}

func sleepCtx(ctx context.Context, d time.Duration) bool {
	t := time.NewTimer(d)
	defer t.Stop()
	select {
	case <-ctx.Done():
		return false
	case <-t.C:
		return true
	}
}

func (s *Supervisor) logf(format string, args ...any) {
	fmt.Fprintf(s.Log, "hotline: %s %s\n", s.now().Format("2006-01-02 15:04:05"), fmt.Sprintf(format, args...))
}

// watch outcomes.
type outcome int

const (
	harnessExited outcome = iota
	restartRequested
	shutdownRequested
)

// Run supervises until ctx is cancelled (`hotline down` / SIGTERM / SIGINT).
// It restarts the harness on ANY exit, clean or not — an always-on agent that
// exits cleanly at 3am is still down at 9am; `hotline down` is the
// intentional way to stop. It returns nil on a clean shutdown; state.json
// write failures are logged, never fatal.
func (s *Supervisor) Run(ctx context.Context) error {
	st := &State{
		PID:       os.Getpid(),
		StartedAt: s.rfcNow(),
		Argv:      s.Argv,
		WorkDir:   s.WorkDir,
	}
	s.logf("supervisor started (pid %d)", st.PID)

	for {
		// Absorb any pending restart request before spawning: one left over
		// from a previous supervisor's life, or one filed while the harness
		// was already down, must not bounce the fresh harness we are about to
		// start — a start IS the requested restart.
		if reason, pending := consumeRestart(s.Dir); pending {
			s.logf("absorbing restart request (%s): harness is starting anyway", reason)
		}
		h, err := s.Start(ctx)
		if err != nil {
			s.logf("harness spawn failed: %v", err)
			st.LastExit = "spawn failed: " + err.Error()
			if !s.backoffWait(ctx, st, s.Backoff.Next(0)) {
				return s.finalize(st)
			}
			continue
		}
		started := s.now()
		st.Phase, st.HarnessPID, st.HarnessStartedAt, st.NextRestartAt = PhaseRunning, h.Pid(), s.rfcNow(), ""
		s.writeState(st)
		s.logf("harness running (pid %d, start #%d)", h.Pid(), st.Restarts+1)

		out, reason := s.watch(ctx, h)
		switch out {
		case shutdownRequested:
			s.logf("shutting down; stopping harness (pid %d)", h.Pid())
			s.stopHarness(h)
			return s.finalize(st)
		case restartRequested:
			s.logf("restart requested (%s); bouncing harness (pid %d)", reason, h.Pid())
			s.stopHarness(h)
			s.Backoff.Reset()
			st.Restarts++
			st.LastExit = "restart requested: " + reason
			continue
		default: // harnessExited
			uptime := s.now().Sub(started)
			st.Restarts++
			st.LastExit = fmt.Sprintf("%s after %s", h.ExitDesc(), uptime.Round(time.Second))
			s.logf("harness exited: %s", st.LastExit)
			if !s.backoffWait(ctx, st, s.Backoff.Next(uptime)) {
				return s.finalize(st)
			}
		}
	}
}

// watch blocks until the harness exits, a restart is requested via the
// control file, or ctx ends.
func (s *Supervisor) watch(ctx context.Context, h Harness) (outcome, string) {
	t := time.NewTicker(s.Poll)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return shutdownRequested, ""
		case <-h.Done():
			return harnessExited, ""
		case <-t.C:
			if reason, ok := consumeRestart(s.Dir); ok {
				return restartRequested, reason
			}
		}
	}
}

// stopHarness terminates the harness's process group, escalating to SIGKILL
// after Grace. It returns once the harness has exited (or, defensively, after
// a second Grace past the kill).
func (s *Supervisor) stopHarness(h Harness) {
	h.Terminate()
	select {
	case <-h.Done():
		return
	case <-time.After(s.Grace):
	}
	s.logf("harness (pid %d) ignored SIGTERM after %s; killing", h.Pid(), s.Grace)
	h.Kill()
	select {
	case <-h.Done():
	case <-time.After(s.Grace):
		s.logf("harness (pid %d) still not reaped after SIGKILL; abandoning", h.Pid())
	}
}

// backoffWait records the backoff phase and sleeps d. It returns false when
// ctx ended first (shutdown wins over a pending restart).
func (s *Supervisor) backoffWait(ctx context.Context, st *State, d time.Duration) bool {
	st.Phase, st.HarnessPID, st.NextRestartAt = PhaseBackoff, 0, s.now().Add(d).UTC().Format(time.RFC3339)
	s.writeState(st)
	s.logf("restarting in %s", d)
	return s.sleep(ctx, d)
}

func (s *Supervisor) finalize(st *State) error {
	st.Phase, st.HarnessPID, st.NextRestartAt = PhaseStopped, 0, ""
	s.writeState(st)
	s.logf("supervisor stopped")
	return nil
}

func (s *Supervisor) writeState(st *State) {
	if err := WriteState(s.Dir, st); err != nil {
		s.logf("writing state.json failed: %v", err)
	}
}

func (s *Supervisor) rfcNow() string { return s.now().UTC().Format(time.RFC3339) }
