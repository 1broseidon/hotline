package main

import (
	"bytes"
	"errors"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"syscall"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/lifecycle"
	"github.com/1broseidon/hotline/internal/supervise"
)

func upTestState(t *testing.T) string {
	t.Helper()
	dir := setupTestState(t)
	t.Setenv("HOTLINE_PROVIDERS", "")
	t.Setenv("HOTLINE_BOT", "")
	t.Setenv("TELE_GO_BOT", "")
	t.Setenv("HOTLINE_HARNESS", "")
	// Force the passive (non-tty) attach decision so the claude path always
	// routes through the startHarness seam, even when `go test` runs from a live
	// terminal — otherwise NewAttacher would attach the real pty and time out
	// waiting for the fake spawn (finding 7).
	orig := newAttacher
	newAttacher = func(_, _ *os.File, _ io.Writer, _ func()) (terminalAttacher, error) {
		return passiveAttacher{}, nil
	}
	t.Cleanup(func() { newAttacher = orig })
	return dir
}

// passiveAttacher is the forced non-tty attach decision for command tests:
// TTY() is false, so cmdUp never calls Start and falls back to the startHarness
// seam (and StopOnCleanExit stays off, matching the log-only claude path).
type passiveAttacher struct{}

func (passiveAttacher) TTY() bool { return false }
func (passiveAttacher) Start([]string, string, []string) (supervise.Harness, error) {
	return nil, errors.New("passiveAttacher.Start must not be called (TTY() is false)")
}
func (passiveAttacher) Close() error { return nil }

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

// TestUpSupervisesOpenCodeHarness: foreground-default `up` with
// HOTLINE_HARNESS=opencode supervises `opencode serve` on the piped (no-pty)
// spawn path — port and
// hostname derived from OPENCODE_SERVER_URL (the same source the hotline MCP
// child reads, so daemon and client agree), passthrough appended verbatim,
// and HOTLINE_SUPERVISOR_DIR exported so the restart tool registers in the
// session opencode spawns.
func TestUpSupervisesOpenCodeHarness(t *testing.T) {
	baseRoot := upTestState(t)
	t.Setenv("HOTLINE_HARNESS", "opencode")
	t.Setenv("OPENCODE_SERVER_URL", "http://127.0.0.1:4777")
	t.Setenv(lifecycle.OwnerLeaseEnv, "stale-inherited-lease")
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
		done <- cmdUp("Ada", true, nil, []string{"--log-level", "DEBUG"}, t.TempDir(), &out, &errOut)
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
	wantSupervisorDir := filepath.Join(baseRoot, "bots", "Ada", "supervisor")
	envSeen := false
	ownerLease := ""
	ownerLeaseCount := 0
	for _, kv := range got.env {
		if kv == supervise.EnvDir+"="+wantSupervisorDir {
			envSeen = true
		}
		if strings.HasPrefix(kv, lifecycle.OwnerLeaseEnv+"=") {
			ownerLease = strings.TrimPrefix(kv, lifecycle.OwnerLeaseEnv+"=")
			ownerLeaseCount++
		}
	}
	if !envSeen {
		t.Errorf("harness env lacks %s=%s — named supervisor state is on the wrong root", supervise.EnvDir, wantSupervisorDir)
	}
	if ownerLease == "" || ownerLease == "stale-inherited-lease" || ownerLeaseCount != 1 {
		t.Errorf("harness env %s = %q (%d entries), want one fresh supervisor lease", lifecycle.OwnerLeaseEnv, ownerLease, ownerLeaseCount)
	}
}

// TestUpSupervisesPiHarness: with HOTLINE_HARNESS=pi, `up` supervises
// `pi --mode rpc --session-id <stable>` on the stdin-held-open spawn path
// (pi reads JSONL from stdin and treats EOF as shutdown), passthrough appended
// verbatim, and HOTLINE_SUPERVISOR_DIR exported so the restart tool registers
// in the hotline child pi spawns.
func TestUpSupervisesPiHarness(t *testing.T) {
	upTestState(t)
	t.Setenv("HOTLINE_HARNESS", "pi")
	t.Setenv("HOTLINE_PI_SESSION", "")
	stubBinary(t, "pi")

	type spawn struct{ argv, env []string }
	spawned := make(chan spawn, 1)
	orig := startHarnessPipedStdinOpen
	startHarnessPipedStdinOpen = func(argv []string, dir string, env []string, logw io.Writer) (supervise.Harness, error) {
		select {
		case spawned <- spawn{argv, env}:
		default:
		}
		return &fakeUpHarness{done: make(chan struct{})}, nil
	}
	t.Cleanup(func() { startHarnessPipedStdinOpen = orig })

	var out, errOut bytes.Buffer
	done := make(chan error, 1)
	go func() {
		done <- cmdUp("", false, []string{"--foreground"}, []string{"--verbose"}, t.TempDir(), &out, &errOut)
	}()

	var got spawn
	select {
	case got = <-spawned:
	case <-time.After(5 * time.Second):
		t.Fatal("pi harness was not spawned")
	}
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

	if len(got.argv) == 0 || filepath.Base(got.argv[0]) != "pi" {
		t.Fatalf("argv = %v, want resolved pi binary first", got.argv)
	}
	want := []string{"--mode", "rpc", "--session-id", "hotline", "--verbose"}
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
	if notice := errOut.String(); !strings.Contains(notice, "--foreground is deprecated") {
		t.Errorf("deprecated --foreground notice missing: %q", notice)
	}
}

// TestUpHarnessFlagOverridesEnv: `--harness pi` wins over HOTLINE_HARNESS=claude
// in the real env. Proof the flag reached both the harness resolution AND the
// yolo/background guard: the pi (stdin-open) spawn path is taken instead of
// claude's pty path, and the launch line attributes the choice to --harness.
func TestUpHarnessFlagOverridesEnv(t *testing.T) {
	upTestState(t)
	t.Setenv("HOTLINE_HARNESS", "claude") // env says claude; the flag must win
	t.Setenv("HOTLINE_PI_SESSION", "")
	stubBinary(t, "pi")

	type spawn struct{ argv, env []string }
	spawned := make(chan spawn, 1)
	orig := startHarnessPipedStdinOpen
	startHarnessPipedStdinOpen = func(argv []string, dir string, env []string, logw io.Writer) (supervise.Harness, error) {
		select {
		case spawned <- spawn{argv, env}:
		default:
		}
		return &fakeUpHarness{done: make(chan struct{})}, nil
	}
	t.Cleanup(func() { startHarnessPipedStdinOpen = orig })

	var out, errOut bytes.Buffer
	done := make(chan error, 1)
	go func() {
		done <- cmdUp("", false, []string{"--harness", "pi"}, nil, t.TempDir(), &out, &errOut)
	}()

	var got spawn
	select {
	case got = <-spawned:
	case <-time.After(5 * time.Second):
		t.Fatal("pi harness was not spawned — the --harness flag did not override the env")
	}
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

	if len(got.argv) == 0 || filepath.Base(got.argv[0]) != "pi" {
		t.Fatalf("argv = %v, want the pi binary — the flag-selected harness was not supervised", got.argv)
	}
	if notice := errOut.String(); !strings.Contains(notice, "harness pi (from --harness)") {
		t.Errorf("launch line = %q, want `harness pi (from --harness)` attribution", notice)
	}
}

// TestUpRejectsInvalidHarnessFlag: a typo'd --harness fails loudly at launch,
// sharing config.NormalizeHarness's four-value switch.
func TestUpRejectsInvalidHarnessFlag(t *testing.T) {
	upTestState(t)
	var out, errOut bytes.Buffer
	err := cmdUp("", false, []string{"--harness", "bogus"}, nil, t.TempDir(), &out, &errOut)
	if err == nil || !strings.Contains(err.Error(), "unknown harness") {
		t.Fatalf("err = %v, want unknown-harness", err)
	}
}

// TestUpRejectsYoloOnPi: --yolo is claude's --dangerously-skip-permissions;
// Pi has no permission prompts at all, so the flag must error, not pretend to
// change anything.
func TestUpRejectsYoloOnPi(t *testing.T) {
	upTestState(t)
	t.Setenv("HOTLINE_HARNESS", "pi")
	var out, errOut bytes.Buffer
	err := cmdUp("", false, []string{"--yolo"}, nil, t.TempDir(), &out, &errOut)
	if err == nil || !strings.Contains(err.Error(), "--yolo") || !strings.Contains(err.Error(), "nothing to skip") {
		t.Fatalf("err = %v, want a --yolo/nothing-to-skip explanation", err)
	}
}

// TestUpRequiresPiOnPath mirrors the claude/opencode preflight.
func TestUpRequiresPiOnPath(t *testing.T) {
	upTestState(t)
	t.Setenv("HOTLINE_HARNESS", "pi")
	t.Setenv("PATH", t.TempDir()) // nothing on it
	var out, errOut bytes.Buffer
	err := cmdUp("", false, nil, nil, t.TempDir(), &out, &errOut)
	if err == nil || !strings.Contains(err.Error(), "pi not found") {
		t.Fatalf("err = %v, want pi-not-found", err)
	}
}

// TestPiSessionID: the stable id is deterministic (no persistence needed) — bot
// name keyed, HOTLINE_PI_SESSION overriding.
func TestPiSessionID(t *testing.T) {
	t.Setenv("HOTLINE_PI_SESSION", "")
	if got := piSessionID(""); got != "hotline" {
		t.Errorf("piSessionID(\"\") = %q, want hotline", got)
	}
	if got := piSessionID("work"); got != "hotline-work" {
		t.Errorf("piSessionID(\"work\") = %q, want hotline-work", got)
	}
	t.Setenv("HOTLINE_PI_SESSION", "custom")
	if got := piSessionID("work"); got != "custom" {
		t.Errorf("HOTLINE_PI_SESSION override = %q, want custom", got)
	}
}

// unsetEnv removes key for the duration of the test and restores whatever it
// was at cleanup, so a .env-driven knob test is not defeated by a var the
// developer's shell happens to export.
func unsetEnv(t *testing.T, key string) {
	t.Helper()
	t.Setenv(key, "") // record the original for restoration
	os.Unsetenv(key)
}

// TestPiModelArgs is the knob-precedence matrix: an unset field contributes
// nothing, a set field becomes `--flag value` in fixed order, and a field the
// passthrough already carries is dropped so the explicit passthrough wins.
func TestPiModelArgs(t *testing.T) {
	tests := []struct {
		name        string
		knob        config.PiModel
		passthrough []string
		want        []string
	}{
		{
			name: "unset",
			knob: config.PiModel{},
			want: nil,
		},
		{
			name: "env only",
			knob: config.PiModel{Provider: "anthropic", Model: "sonnet", Thinking: "high"},
			want: []string{"--provider", "anthropic", "--model", "sonnet", "--thinking", "high"},
		},
		{
			name: "partial env",
			knob: config.PiModel{Model: "openai/gpt-4o"},
			want: []string{"--model", "openai/gpt-4o"},
		},
		{
			name:        "passthrough only",
			knob:        config.PiModel{},
			passthrough: []string{"--provider", "openai", "--model", "gpt-4o"},
			want:        nil,
		},
		{
			name:        "passthrough wins space form",
			knob:        config.PiModel{Provider: "anthropic", Model: "sonnet", Thinking: "high"},
			passthrough: []string{"--provider", "openai"},
			want:        []string{"--model", "sonnet", "--thinking", "high"},
		},
		{
			name:        "passthrough wins equals form",
			knob:        config.PiModel{Provider: "anthropic", Model: "sonnet"},
			passthrough: []string{"--model=gpt-4o"},
			want:        []string{"--provider", "anthropic"},
		},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			got := piModelArgs(tc.knob, tc.passthrough)
			if !reflect.DeepEqual(got, tc.want) {
				t.Errorf("piModelArgs(%+v, %v) = %v, want %v", tc.knob, tc.passthrough, got, tc.want)
			}
		})
	}
}

// TestUpInjectsPiModelKnob: the .env knob keys land as pi flags in the
// supervised argv, before the passthrough args.
func TestUpInjectsPiModelKnob(t *testing.T) {
	dir := upTestState(t)
	t.Setenv("HOTLINE_HARNESS", "pi")
	t.Setenv("HOTLINE_PI_SESSION", "")
	for _, k := range []string{"HOTLINE_PI_PROVIDER", "HOTLINE_PI_MODEL", "HOTLINE_PI_THINKING"} {
		unsetEnv(t, k)
	}
	content := "HOTLINE_PI_PROVIDER=anthropic\nHOTLINE_PI_MODEL=sonnet\nHOTLINE_PI_THINKING=high\n"
	if err := os.WriteFile(filepath.Join(dir, ".env"), []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
	stubBinary(t, "pi")

	got := runUpCapturePi(t, []string{"--verbose"})
	want := []string{"--mode", "rpc", "--session-id", "hotline", "--provider", "anthropic", "--model", "sonnet", "--thinking", "high", "--verbose"}
	if !reflect.DeepEqual(got.argv[1:], want) {
		t.Errorf("argv[1:] = %v, want %v", got.argv[1:], want)
	}
}

// TestUpPiKnobPassthroughWins: an explicit `-- --provider X` overrides the .env
// provider knob and never double-applies the flag.
func TestUpPiKnobPassthroughWins(t *testing.T) {
	dir := upTestState(t)
	t.Setenv("HOTLINE_HARNESS", "pi")
	t.Setenv("HOTLINE_PI_SESSION", "")
	for _, k := range []string{"HOTLINE_PI_PROVIDER", "HOTLINE_PI_MODEL", "HOTLINE_PI_THINKING"} {
		unsetEnv(t, k)
	}
	content := "HOTLINE_PI_PROVIDER=envprov\nHOTLINE_PI_MODEL=sonnet\n"
	if err := os.WriteFile(filepath.Join(dir, ".env"), []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
	stubBinary(t, "pi")

	got := runUpCapturePi(t, []string{"--provider", "cliprov"})
	// The .env provider is dropped (passthrough carries --provider); the model
	// knob still applies. Passthrough --provider comes last, verbatim.
	want := []string{"--mode", "rpc", "--session-id", "hotline", "--model", "sonnet", "--provider", "cliprov"}
	if !reflect.DeepEqual(got.argv[1:], want) {
		t.Errorf("argv[1:] = %v, want %v", got.argv[1:], want)
	}
	provCount := 0
	for _, a := range got.argv {
		if a == "--provider" {
			provCount++
		}
	}
	if provCount != 1 {
		t.Errorf("--provider appears %d times, want exactly 1 (no double-apply)", provCount)
	}
	for _, a := range got.argv {
		if a == "envprov" {
			t.Errorf("the .env provider leaked into argv despite passthrough override: %v", got.argv)
		}
	}
}

// runUpCapturePi runs cmdUp on the pi path, captures the single spawned argv,
// and stops the supervisor. It centralizes the spawn-capture dance the pi knob
// tests share.
func runUpCapturePi(t *testing.T, passthrough []string) upSpawn {
	t.Helper()
	spawned := make(chan upSpawn, 1)
	orig := startHarnessPipedStdinOpen
	startHarnessPipedStdinOpen = func(argv []string, dir string, env []string, logw io.Writer) (supervise.Harness, error) {
		select {
		case spawned <- upSpawn{argv, env}:
		default:
		}
		return &fakeUpHarness{done: make(chan struct{})}, nil
	}
	t.Cleanup(func() { startHarnessPipedStdinOpen = orig })

	var out, errOut bytes.Buffer
	done := make(chan error, 1)
	go func() {
		done <- cmdUp("", false, []string{"--foreground"}, passthrough, t.TempDir(), &out, &errOut)
	}()

	var got upSpawn
	select {
	case got = <-spawned:
	case <-time.After(5 * time.Second):
		t.Fatal("pi harness was not spawned")
	}
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
	if len(got.argv) == 0 || filepath.Base(got.argv[0]) != "pi" {
		t.Fatalf("argv = %v, want resolved pi binary first", got.argv)
	}
	return got
}

type upSpawn struct{ argv, env []string }

// TestUpInjectsAnthropicProviderClaude: on the claude path, `up` folds the
// allowlisted .env keys into the env handed to the supervised harness, alongside
// HOTLINE_SUPERVISOR_DIR — without disturbing the opencode branch.
func TestUpInjectsAnthropicProviderClaude(t *testing.T) {
	dir := upTestState(t)
	clearAnthropicEnv(t)
	content := "ANTHROPIC_BASE_URL=https://alt.example/v1\nANTHROPIC_MODEL=alt-model\n"
	if err := os.WriteFile(filepath.Join(dir, ".env"), []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
	stubBinary(t, "claude")
	fakeClaudeRunner(t) // isolate channelArgs' reads of ~/.claude state

	type spawn struct{ argv, env []string }
	spawned := make(chan spawn, 1)
	orig := startHarness
	startHarness = func(argv []string, dir string, env []string, logw io.Writer) (supervise.Harness, error) {
		select {
		case spawned <- spawn{argv, env}:
		default:
		}
		return &fakeUpHarness{done: make(chan struct{})}, nil
	}
	t.Cleanup(func() { startHarness = orig })

	var out, errOut bytes.Buffer
	done := make(chan error, 1)
	go func() {
		done <- cmdUp("", false, []string{"--foreground"}, nil, t.TempDir(), &out, &errOut)
	}()

	var got spawn
	select {
	case got = <-spawned:
	case <-time.After(5 * time.Second):
		t.Fatal("claude harness was not spawned")
	}
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

	joined := strings.Join(got.env, "\n")
	if !strings.Contains(joined, "ANTHROPIC_BASE_URL=https://alt.example/v1") {
		t.Error("ANTHROPIC_BASE_URL not injected into the supervised claude env")
	}
	if !strings.Contains(joined, "ANTHROPIC_MODEL=alt-model") {
		t.Error("ANTHROPIC_MODEL not injected into the supervised claude env")
	}
	if !strings.Contains(joined, supervise.EnvDir+"=") {
		t.Errorf("supervised env lost %s while injecting the provider", supervise.EnvDir)
	}
}

// ttyAttacher is a fake active attacher: TTY() is true (so cmdUp takes the
// attached-claude path — StopOnCleanExit on, supervisor logs routed to
// supervisor.log) without needing a real pty or terminal.
type ttyAttacher struct{}

func (ttyAttacher) TTY() bool { return true }
func (ttyAttacher) Start([]string, string, []string) (supervise.Harness, error) {
	return &fakeUpHarness{done: make(chan struct{})}, nil
}
func (ttyAttacher) Close() error { return nil }

// TestUpAttachedRoutesSupervisorLogToFile pins finding #3: while the attached
// claude TUI owns the terminal, supervisor diagnostics must go to
// supervisor/supervisor.log, NOT stderr (which the full-screen TUI owns). The
// pre-raw transition line still goes to stderr so the operator knows where the
// logs went.
func TestUpAttachedRoutesSupervisorLogToFile(t *testing.T) {
	baseRoot := upTestState(t)
	clearAnthropicEnv(t)
	stubBinary(t, "claude")
	fakeClaudeRunner(t) // isolate channelArgs' reads of ~/.claude state

	// Force the attached path through the seam (upTestState set a passive fake;
	// this overrides it for this test — its cleanup restores the original).
	newAttacher = func(_, _ *os.File, _ io.Writer, _ func()) (terminalAttacher, error) {
		return ttyAttacher{}, nil
	}

	var out, errOut bytes.Buffer
	done := make(chan error, 1)
	go func() {
		done <- cmdUp("Ada", true, nil, nil, t.TempDir(), &out, &errOut)
	}()

	supLogPath := filepath.Join(baseRoot, "bots", "Ada", "supervisor", "supervisor.log")
	deadline := time.Now().Add(5 * time.Second)
	for {
		if b, _ := os.ReadFile(supLogPath); strings.Contains(string(b), "harness running") {
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("supervisor.log never got the diagnostic line; file=%q stderr=%q", supLogPath, errOut.String())
		}
		time.Sleep(5 * time.Millisecond)
	}

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

	// The supervisor's diagnostic lines went to the file, not stderr.
	if strings.Contains(errOut.String(), "harness running") {
		t.Errorf("supervisor diagnostics leaked to stderr while the TUI owned the terminal:\n%s", errOut.String())
	}
	logBytes, err := os.ReadFile(supLogPath)
	if err != nil {
		t.Fatalf("reading supervisor.log: %v", err)
	}
	if !strings.Contains(string(logBytes), "supervisor started") || !strings.Contains(string(logBytes), "harness running") {
		t.Errorf("supervisor.log missing diagnostics: %q", string(logBytes))
	}
	// The operator still learns where the logs went (transition line on stderr).
	if !strings.Contains(errOut.String(), "supervisor diagnostics now stream to") {
		t.Errorf("missing transition line on stderr: %q", errOut.String())
	}
}

// TestUpRejectsYoloOnOpenCode: --yolo is claude's
// --dangerously-skip-permissions; opencode's permission policy lives in
// opencode.json, so the flag must error, not be silently ignored.
func TestUpRejectsYoloOnOpenCode(t *testing.T) {
	upTestState(t)
	t.Setenv("HOTLINE_HARNESS", "opencode")
	var out, errOut bytes.Buffer
	err := cmdUp("", false, []string{"--yolo"}, nil, t.TempDir(), &out, &errOut)
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
	err := cmdUp("", false, nil, nil, t.TempDir(), &out, &errOut)
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

func TestUpBackgroundClaudeRefusesWithTmuxGuidance(t *testing.T) {
	for _, flag := range []string{"--background", "-d"} {
		t.Run(flag, func(t *testing.T) {
			upTestState(t)
			t.Setenv("HOTLINE_HARNESS", "claude")

			called := false
			orig := detachUpSupervisor
			detachUpSupervisor = func(string, bool, []string, string, io.Writer) error {
				called = true
				return nil
			}
			t.Cleanup(func() { detachUpSupervisor = orig })

			var out, errOut bytes.Buffer
			err := cmdUp("", false, []string{flag}, nil, t.TempDir(), &out, &errOut)
			if err == nil {
				t.Fatalf("up %s succeeded for Claude background", flag)
			}
			for _, want := range []string{
				"development-channel consent cannot be answered detached",
				"tmux new -s hotline -- hotline up",
				"Ctrl-b d",
				"tmux attach -t hotline",
			} {
				if !strings.Contains(err.Error(), want) {
					t.Errorf("up %s error %q missing %q", flag, err, want)
				}
			}
			if called {
				t.Fatalf("up %s called detached supervisor for Claude", flag)
			}
		})
	}
}

func TestUpBackgroundAliasesDetachHeadlessHarnesses(t *testing.T) {
	for _, harness := range []string{"opencode", "pi"} {
		for _, flag := range []string{"--background", "-d"} {
			t.Run(harness+"/"+flag, func(t *testing.T) {
				upTestState(t)
				t.Setenv("HOTLINE_HARNESS", harness)
				stubBinary(t, harness)

				called := false
				orig := detachUpSupervisor
				detachUpSupervisor = func(supDir string, yolo bool, passthrough []string, dir string, stdout io.Writer) error {
					called = true
					return nil
				}
				t.Cleanup(func() { detachUpSupervisor = orig })

				var out, errOut bytes.Buffer
				if err := cmdUp("", false, []string{flag}, nil, t.TempDir(), &out, &errOut); err != nil {
					t.Fatalf("up %s %s: %v", harness, flag, err)
				}
				if !called {
					t.Fatalf("up %s %s did not route to detached supervisor", harness, flag)
				}
			})
		}
	}
}

// TestUpRefusesWhenAlreadyRunning: the flock singleton stops a second
// supervisor (which would double-spawn harnesses and fight over the poller
// slot).
func TestUpRefusesWhenAlreadyRunning(t *testing.T) {
	dir := upTestState(t)
	t.Setenv("HOTLINE_HARNESS", "opencode")
	stubBinary(t, "opencode")
	supDir := supervise.Dir(dir)
	release, err := supervise.AcquireLock(supDir)
	if err != nil {
		t.Fatal(err)
	}
	defer release()

	var out, errOut bytes.Buffer
	if err := cmdUp("", false, nil, nil, t.TempDir(), &out, &errOut); err == nil || !strings.Contains(err.Error(), "already running") {
		t.Fatalf("foreground-default up: err = %v, want already-running", err)
	}
	if err := cmdUp("", false, []string{"--background"}, nil, t.TempDir(), &out, &errOut); err == nil || !strings.Contains(err.Error(), "already running") {
		t.Fatalf("background up: err = %v, want already-running", err)
	}
}

// TestUpRequiresClaudeOnPath mirrors start's preflight.
func TestUpRequiresClaudeOnPath(t *testing.T) {
	upTestState(t)
	t.Setenv("PATH", t.TempDir()) // nothing on it
	var out, errOut bytes.Buffer
	err := cmdUp("", false, nil, nil, t.TempDir(), &out, &errOut)
	if err == nil || !strings.Contains(err.Error(), "claude not found") {
		t.Fatalf("err = %v, want claude-not-found", err)
	}
}

// TestDownWhenNotRunning is a friendly no-op, not an error.
func TestDownWhenNotRunning(t *testing.T) {
	upTestState(t)
	var out, errOut bytes.Buffer
	if err := cmdDown("", false, nil, t.TempDir(), &out, &errOut); err != nil {
		t.Fatalf("down: %v", err)
	}
	if !strings.Contains(out.String(), "not running") {
		t.Errorf("output = %q, want not-running notice", out.String())
	}
}
