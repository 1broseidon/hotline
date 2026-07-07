//go:build linux || darwin

package supervise

import (
	"bytes"
	"os"
	"strings"
	"sync"
	"testing"
	"time"
)

// syncBuf is a goroutine-safe buffer for the pty drain.
type syncBuf struct {
	mu sync.Mutex
	b  bytes.Buffer
}

func (s *syncBuf) Write(p []byte) (int, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.b.Write(p)
}

func (s *syncBuf) String() string {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.b.String()
}

func waitDone(t *testing.T, h Harness) {
	t.Helper()
	select {
	case <-h.Done():
	case <-time.After(5 * time.Second):
		t.Fatal("harness did not exit in time")
	}
}

// TestStartOnPTYGivesChildATTY is the reason the pty exists: interactive
// Claude Code requires a controlling terminal (with a non-tty stdin it drops
// into print mode and exits). Verify the spawned child really sees a tty and
// its output lands in the log writer.
func TestStartOnPTYGivesChildATTY(t *testing.T) {
	buf := &syncBuf{}
	h, err := StartOnPTY([]string{"/bin/sh", "-c", "test -t 0 && test -t 1 && echo ISATTY"}, t.TempDir(), os.Environ(), buf)
	if err != nil {
		t.Fatal(err)
	}
	waitDone(t, h)
	if got := h.ExitDesc(); got != "exit status 0" {
		t.Errorf("ExitDesc = %q, want exit status 0 (child saw no tty?)", got)
	}
	// The drain goroutine may lag the exit by a beat.
	deadline := time.Now().Add(2 * time.Second)
	for !strings.Contains(buf.String(), "ISATTY") {
		if time.Now().After(deadline) {
			t.Fatalf("pty output not captured: %q", buf.String())
		}
		time.Sleep(5 * time.Millisecond)
	}
}

// TestStartOnPTYTerminateEndsProcessGroup: Terminate signals the whole group,
// so a sleeping child (standing in for claude + its MCP children) goes down.
func TestStartOnPTYTerminateEndsProcessGroup(t *testing.T) {
	buf := &syncBuf{}
	h, err := StartOnPTY([]string{"/bin/sh", "-c", "sleep 30"}, t.TempDir(), os.Environ(), buf)
	if err != nil {
		t.Fatal(err)
	}
	h.Terminate()
	waitDone(t, h)
	if got := h.ExitDesc(); !strings.Contains(got, "terminated") && !strings.Contains(got, "hangup") {
		t.Errorf("ExitDesc = %q, want a termination signal", got)
	}
}

// TestStartOnPTYExitCodeSurfaces: a non-zero exit is described for the
// supervisor log breadcrumb.
func TestStartOnPTYExitCodeSurfaces(t *testing.T) {
	buf := &syncBuf{}
	h, err := StartOnPTY([]string{"/bin/sh", "-c", "exit 3"}, t.TempDir(), os.Environ(), buf)
	if err != nil {
		t.Fatal(err)
	}
	waitDone(t, h)
	if got := h.ExitDesc(); !strings.Contains(got, "exit status 3") {
		t.Errorf("ExitDesc = %q, want exit status 3", got)
	}
}

// TestStartPipedRunsWithoutATTY is the point of the piped path: a headless
// daemon (`opencode serve`) gets plain pipes — no terminal, empty stdin —
// with both output streams landing in the log writer.
func TestStartPipedRunsWithoutATTY(t *testing.T) {
	buf := &syncBuf{}
	h, err := StartPiped([]string{"/bin/sh", "-c", "test -t 0 || echo NOTTY; echo OUT; echo ERR >&2; test -z \"$(cat)\" && echo EMPTYSTDIN"}, t.TempDir(), os.Environ(), buf)
	if err != nil {
		t.Fatal(err)
	}
	waitDone(t, h)
	if got := h.ExitDesc(); got != "exit status 0" {
		t.Errorf("ExitDesc = %q, want exit status 0", got)
	}
	out := buf.String()
	for _, want := range []string{"NOTTY", "OUT", "ERR", "EMPTYSTDIN"} {
		if !strings.Contains(out, want) {
			t.Errorf("log output %q missing %q", out, want)
		}
	}
}

// TestStartPipedTerminateEndsProcessGroup: the piped child is still a
// session leader, so Terminate takes down the whole group (the daemon plus
// any MCP children it spawned).
func TestStartPipedTerminateEndsProcessGroup(t *testing.T) {
	buf := &syncBuf{}
	h, err := StartPiped([]string{"/bin/sh", "-c", "sleep 30"}, t.TempDir(), os.Environ(), buf)
	if err != nil {
		t.Fatal(err)
	}
	h.Terminate()
	waitDone(t, h)
	if got := h.ExitDesc(); !strings.Contains(got, "terminated") {
		t.Errorf("ExitDesc = %q, want a termination signal", got)
	}
}

// TestStartPipedExitCodeSurfaces mirrors the pty variant.
func TestStartPipedExitCodeSurfaces(t *testing.T) {
	buf := &syncBuf{}
	h, err := StartPiped([]string{"/bin/sh", "-c", "exit 3"}, t.TempDir(), os.Environ(), buf)
	if err != nil {
		t.Fatal(err)
	}
	waitDone(t, h)
	if got := h.ExitDesc(); !strings.Contains(got, "exit status 3") {
		t.Errorf("ExitDesc = %q, want exit status 3", got)
	}
}
