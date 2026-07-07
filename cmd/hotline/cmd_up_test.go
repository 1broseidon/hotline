package main

import (
	"bytes"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"syscall"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/supervise"
)

func upTestState(t *testing.T) string {
	t.Helper()
	dir := setupTestState(t)
	t.Setenv("HOTLINE_PROVIDERS", "")
	t.Setenv("HOTLINE_BOT", "")
	t.Setenv("TELE_GO_BOT", "")
	t.Setenv("HOTLINE_HARNESS", "")
	return dir
}

// stubBinary puts an executable stub named name on PATH so LookPath
// preflights pass without the real binary.
func stubBinary(t *testing.T, name string) {
	t.Helper()
	binDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(binDir, name), []byte("#!/bin/sh\nexit 0\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", binDir+string(os.PathListSeparator)+os.Getenv("PATH"))
}

// fakeUpHarness is a cooperative supervise.Harness for cmd-level tests: it
// stays up until Terminate/Kill.
type fakeUpHarness struct {
	done chan struct{}
	once sync.Once
}

func (h *fakeUpHarness) Pid() int              { return 4242 }
func (h *fakeUpHarness) Done() <-chan struct{} { return h.done }
func (h *fakeUpHarness) ExitDesc() string      { return "signal: terminated" }
func (h *fakeUpHarness) Terminate()            { h.once.Do(func() { close(h.done) }) }
func (h *fakeUpHarness) Kill()                 { h.once.Do(func() { close(h.done) }) }

// TestUpSupervisesOpenCodeHarness: with HOTLINE_HARNESS=opencode, `up`
// supervises `opencode serve` on the piped (no-pty) spawn path — port and
// hostname derived from OPENCODE_SERVER_URL (the same source the hotline MCP
// child reads, so daemon and client agree), passthrough appended verbatim,
// and HOTLINE_SUPERVISOR_DIR exported so the restart tool registers in the
// session opencode spawns.
func TestUpSupervisesOpenCodeHarness(t *testing.T) {
	upTestState(t)
	t.Setenv("HOTLINE_HARNESS", "opencode")
	t.Setenv("OPENCODE_SERVER_URL", "http://127.0.0.1:4777")
	stubBinary(t, "opencode")

	type spawn struct{ argv, env []string }
	spawned := make(chan spawn, 1)
	orig := startHarnessPiped
	startHarnessPiped = func(argv []string, dir string, env []string, logw io.Writer) (supervise.Harness, error) {
		select {
		case spawned <- spawn{argv, env}:
		default:
		}
		return &fakeUpHarness{done: make(chan struct{})}, nil
	}
	t.Cleanup(func() { startHarnessPiped = orig })

	var out, errOut bytes.Buffer
	done := make(chan error, 1)
	go func() {
		done <- cmdUp("", []string{"--foreground"}, []string{"--log-level", "DEBUG"}, t.TempDir(), &out, &errOut)
	}()

	var got spawn
	select {
	case got = <-spawned:
	case <-time.After(5 * time.Second):
		t.Fatal("opencode harness was not spawned")
	}
	// Stop the foreground supervisor the way `hotline down` would. cmdUp's
	// signal handler is registered before the loop that spawned, so this is
	// race-free.
	if err := syscall.Kill(os.Getpid(), syscall.SIGTERM); err != nil {
		t.Fatal(err)
	}
	select {
	case err := <-done:
		if err != nil {
			t.Fatalf("cmdUp: %v", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("cmdUp did not stop on SIGTERM")
	}

	if len(got.argv) == 0 || filepath.Base(got.argv[0]) != "opencode" {
		t.Fatalf("argv = %v, want resolved opencode binary first", got.argv)
	}
	want := []string{"serve", "--port", "4777", "--hostname", "127.0.0.1", "--log-level", "DEBUG"}
	if !reflect.DeepEqual(got.argv[1:], want) {
		t.Errorf("argv[1:] = %v, want %v", got.argv[1:], want)
	}
	envSeen := false
	for _, kv := range got.env {
		if strings.HasPrefix(kv, supervise.EnvDir+"=") {
			envSeen = true
		}
	}
	if !envSeen {
		t.Errorf("harness env lacks %s — the restart tool would never register", supervise.EnvDir)
	}
}

// TestUpRejectsYoloOnOpenCode: --yolo is claude's
// --dangerously-skip-permissions; opencode's permission policy lives in
// opencode.json, so the flag must error, not be silently ignored.
func TestUpRejectsYoloOnOpenCode(t *testing.T) {
	upTestState(t)
	t.Setenv("HOTLINE_HARNESS", "opencode")
	var out, errOut bytes.Buffer
	err := cmdUp("", []string{"--yolo"}, nil, t.TempDir(), &out, &errOut)
	if err == nil || !strings.Contains(err.Error(), "--yolo") || !strings.Contains(err.Error(), "permission") {
		t.Fatalf("err = %v, want a --yolo/permission-block explanation", err)
	}
}

// TestUpRequiresOpenCodeOnPath mirrors the claude preflight.
func TestUpRequiresOpenCodeOnPath(t *testing.T) {
	upTestState(t)
	t.Setenv("HOTLINE_HARNESS", "opencode")
	t.Setenv("PATH", t.TempDir()) // nothing on it
	var out, errOut bytes.Buffer
	err := cmdUp("", nil, nil, t.TempDir(), &out, &errOut)
	if err == nil || !strings.Contains(err.Error(), "opencode not found") {
		t.Fatalf("err = %v, want opencode-not-found", err)
	}
}

// TestOpencodeServeAddr: the bind address is derived from the same URL the
// SSE client dials, including the scheme-default port when none is explicit.
func TestOpencodeServeAddr(t *testing.T) {
	cases := []struct {
		url        string
		host, port string
		wantErr    bool
	}{
		{url: "http://127.0.0.1:4096", host: "127.0.0.1", port: "4096"},
		{url: "http://0.0.0.0:5000", host: "0.0.0.0", port: "5000"},
		{url: "http://localhost", host: "localhost", port: "80"},
		{url: "https://oc.internal", host: "oc.internal", port: "443"},
		{url: "http://", wantErr: true},
	}
	for _, c := range cases {
		host, port, err := opencodeServeAddr(c.url)
		if c.wantErr {
			if err == nil {
				t.Errorf("%s: err = nil, want error", c.url)
			}
			continue
		}
		if err != nil || host != c.host || port != c.port {
			t.Errorf("%s: got (%s, %s, %v), want (%s, %s)", c.url, host, port, err, c.host, c.port)
		}
	}
}

// TestUpRefusesWhenAlreadyRunning: the flock singleton stops a second
// supervisor (which would double-spawn harnesses and fight over the poller
// slot).
func TestUpRefusesWhenAlreadyRunning(t *testing.T) {
	dir := upTestState(t)
	fakeClaude(t) // puts a stub claude on PATH for the preflight
	supDir := supervise.Dir(dir)
	release, err := supervise.AcquireLock(supDir)
	if err != nil {
		t.Fatal(err)
	}
	defer release()

	var out, errOut bytes.Buffer
	if err := cmdUp("", nil, nil, t.TempDir(), &out, &errOut); err == nil || !strings.Contains(err.Error(), "already running") {
		t.Fatalf("detached up: err = %v, want already-running", err)
	}
	if err := cmdUp("", []string{"--foreground"}, nil, t.TempDir(), &out, &errOut); err == nil || !strings.Contains(err.Error(), "already running") {
		t.Fatalf("foreground up: err = %v, want already-running", err)
	}
}

// TestUpRequiresClaudeOnPath mirrors start's preflight.
func TestUpRequiresClaudeOnPath(t *testing.T) {
	upTestState(t)
	t.Setenv("PATH", t.TempDir()) // nothing on it
	var out, errOut bytes.Buffer
	err := cmdUp("", nil, nil, t.TempDir(), &out, &errOut)
	if err == nil || !strings.Contains(err.Error(), "claude not found") {
		t.Fatalf("err = %v, want claude-not-found", err)
	}
}

// TestDownWhenNotRunning is a friendly no-op, not an error.
func TestDownWhenNotRunning(t *testing.T) {
	upTestState(t)
	var out bytes.Buffer
	if err := cmdDown(&out); err != nil {
		t.Fatalf("down: %v", err)
	}
	if !strings.Contains(out.String(), "not running") {
		t.Errorf("output = %q, want not-running notice", out.String())
	}
}
