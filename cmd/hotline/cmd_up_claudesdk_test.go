package main

import (
	"bytes"
	"encoding/json"
	"io"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"syscall"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/supervise"
)

// claudeSDKTestEntry writes a fake built harness entry (dist/index.js) plus
// the .hotline-harness lockstep marker the build stamps, and exports
// HOTLINE_CLAUDE_SDK_ENTRY at it.
func claudeSDKTestEntry(t *testing.T) string {
	t.Helper()
	return claudeSDKTestEntryMarker(t, "harness=claude-sdk\nchild-harness=claude-sdk\n")
}

// claudeSDKTestEntryMarker is claudeSDKTestEntry with a caller-chosen marker
// body; an empty body writes no marker at all (a pre-marker dist).
func claudeSDKTestEntryMarker(t *testing.T, marker string) string {
	t.Helper()
	dir := t.TempDir()
	entry := filepath.Join(dir, "index.js")
	if err := os.WriteFile(entry, []byte("// fake harness entry\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if marker != "" {
		if err := os.WriteFile(filepath.Join(dir, ".hotline-harness"), []byte(marker), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	t.Setenv("HOTLINE_CLAUDE_SDK_ENTRY", entry)
	return entry
}

func envValue(env []string, key string) (string, bool) {
	for _, e := range env {
		if strings.HasPrefix(e, key+"=") {
			return e[len(key)+1:], true
		}
	}
	return "", false
}

// TestUpClaudeSDKSpawnsNodeHarness: HOTLINE_HARNESS=claude-sdk in the real env
// selects the mode even when the shared .env says pi (real env wins in
// config.Harness), spawns `node <entry> <passthrough…>` on the stdin-open
// piped seam with HOTLINE_BIN pinned to this executable, no HOTLINE_YOLO
// without --yolo, and reserves the box first-class as harness=claude-sdk
// (owner.json) — no pi aliasing anywhere.
func TestUpClaudeSDKSpawnsNodeHarness(t *testing.T) {
	baseRoot := upTestState(t)
	// .env says pi; the real environment must win for claude-sdk.
	if err := os.WriteFile(filepath.Join(baseRoot, ".env"), []byte("HOTLINE_HARNESS=pi\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	t.Setenv("HOTLINE_HARNESS", "claude-sdk")
	stubBinary(t, "node")
	entry := claudeSDKTestEntry(t)

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

	// syncBuffer, not bytes.Buffer: this test asserts on errOut WHILE cmdUp is
	// still running in its own goroutine (the supervisor keeps logging until
	// SIGTERM), so an unguarded bytes.Buffer is a real read/write data race —
	// go1.26's -race build failed here deterministically.
	var out, errOut syncBuffer
	done := make(chan error, 1)
	go func() {
		done <- cmdUp("Ada", false, nil, []string{"--verbose"}, t.TempDir(), &out, &errOut)
	}()

	var got spawn
	select {
	case got = <-spawned:
	case <-time.After(5 * time.Second):
		t.Fatal("claude-sdk harness was not spawned")
	}
	boxRoot := filepath.Join(baseRoot, "bots", "Ada")

	// argv: node <entry> <passthrough…>
	if len(got.argv) < 2 || !strings.HasSuffix(got.argv[0], "node") || got.argv[1] != entry {
		t.Fatalf("argv = %v, want [node %s …]", got.argv, entry)
	}
	if got.argv[len(got.argv)-1] != "--verbose" {
		t.Errorf("passthrough not appended: %v", got.argv)
	}

	// env: HOTLINE_BIN pinned to this executable, no HOTLINE_YOLO without --yolo.
	exe, _ := os.Executable()
	if v, ok := envValue(got.env, "HOTLINE_BIN"); !ok || v != exe {
		t.Errorf("HOTLINE_BIN = %q (present=%v), want %q", v, ok, exe)
	}
	if _, ok := envValue(got.env, "HOTLINE_YOLO"); ok {
		t.Error("HOTLINE_YOLO set without --yolo")
	}
	if _, ok := envValue(got.env, supervise.EnvDir); !ok {
		t.Errorf("%s not exported to the harness", supervise.EnvDir)
	}
	// First-class identity: HOTLINE_HARNESS rides into the node harness as
	// claude-sdk (no scrub, no pi cosplay); index.ts re-sets the same value on
	// the run child it spawns.
	if v, ok := envValue(got.env, "HOTLINE_HARNESS"); !ok || v != "claude-sdk" {
		t.Errorf("HOTLINE_HARNESS = %q (present=%v), want claude-sdk", v, ok)
	}

	// The box reservation is first-class: harness=claude-sdk.
	metaRaw, err := os.ReadFile(filepath.Join(boxRoot, ".hotline", "owner.json"))
	if err != nil {
		t.Fatalf("reading owner.json: %v", err)
	}
	var meta struct {
		Harness string `json:"harness"`
	}
	if err := json.Unmarshal(metaRaw, &meta); err != nil {
		t.Fatal(err)
	}
	if meta.Harness != "claude-sdk" {
		t.Errorf("reservation harness = %q, want %q", meta.Harness, "claude-sdk")
	}

	// Loud unguarded warning, pi-style.
	if !strings.Contains(errOut.String(), "harness=claude-sdk") || !strings.Contains(errOut.String(), "bypassPermissions") {
		t.Errorf("missing unguarded warning in stderr: %q", errOut.String())
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
		t.Fatal("cmdUp did not exit after SIGTERM")
	}
}

// TestUpClaudeSDKYoloSetsEnvMarker: --yolo is accepted (no pi-style error) and
// only adds the HOTLINE_YOLO=1 marker.
func TestUpClaudeSDKYoloSetsEnvMarker(t *testing.T) {
	upTestState(t)
	t.Setenv("HOTLINE_HARNESS", "claude-sdk")
	stubBinary(t, "node")
	claudeSDKTestEntry(t)

	spawned := make(chan []string, 1)
	orig := startHarnessPipedStdinOpen
	startHarnessPipedStdinOpen = func(argv []string, dir string, env []string, logw io.Writer) (supervise.Harness, error) {
		select {
		case spawned <- env:
		default:
		}
		return &fakeUpHarness{done: make(chan struct{})}, nil
	}
	t.Cleanup(func() { startHarnessPipedStdinOpen = orig })

	var out, errOut bytes.Buffer
	done := make(chan error, 1)
	go func() {
		done <- cmdUp("Ada", false, []string{"--yolo"}, nil, t.TempDir(), &out, &errOut)
	}()

	var env []string
	select {
	case env = <-spawned:
	case <-time.After(5 * time.Second):
		t.Fatal("claude-sdk harness was not spawned")
	}
	if v, ok := envValue(env, "HOTLINE_YOLO"); !ok || v != "1" {
		t.Errorf("HOTLINE_YOLO = %q (present=%v), want 1", v, ok)
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
		t.Fatal("cmdUp did not exit after SIGTERM")
	}
}

// TestUpClaudeSDKMissingEntryErrors: no HOTLINE_CLAUDE_SDK_ENTRY → a loud
// error carrying the build instructions, before anything spawns.
func TestUpClaudeSDKMissingEntryErrors(t *testing.T) {
	upTestState(t)
	t.Setenv("HOTLINE_HARNESS", "claude-sdk")
	t.Setenv("HOTLINE_CLAUDE_SDK_ENTRY", "")
	stubBinary(t, "node")

	var out, errOut bytes.Buffer
	err := cmdUp("Ada", false, nil, nil, t.TempDir(), &out, &errOut)
	if err == nil {
		t.Fatal("expected error for missing HOTLINE_CLAUDE_SDK_ENTRY")
	}
	if !strings.Contains(err.Error(), "npm install && npm run build") {
		t.Errorf("error should carry build instructions, got: %v", err)
	}
}

// TestUpClaudeSDKMissingNodeErrors: node absent from PATH fails the preflight.
func TestUpClaudeSDKMissingNodeErrors(t *testing.T) {
	upTestState(t)
	t.Setenv("HOTLINE_HARNESS", "claude-sdk")
	claudeSDKTestEntry(t)
	t.Setenv("PATH", t.TempDir()) // no node here

	var out, errOut bytes.Buffer
	err := cmdUp("Ada", false, nil, nil, t.TempDir(), &out, &errOut)
	if err == nil || !strings.Contains(err.Error(), "node") {
		t.Fatalf("expected node-not-found error, got: %v", err)
	}
}

// TestUpClaudeSDKStaleDistNoMarker: an entry without the .hotline-harness
// marker (a dist built before the first-class harness change) is refused at
// preflight with rebuild instructions — never a late ownership refusal.
func TestUpClaudeSDKStaleDistNoMarker(t *testing.T) {
	upTestState(t)
	t.Setenv("HOTLINE_HARNESS", "claude-sdk")
	stubBinary(t, "node")
	claudeSDKTestEntryMarker(t, "")

	var out, errOut bytes.Buffer
	err := cmdUp("Ada", false, nil, nil, t.TempDir(), &out, &errOut)
	if err == nil {
		t.Fatal("expected stale-dist error for a markerless entry")
	}
	if !strings.Contains(err.Error(), "stale claude-sdk build") || !strings.Contains(err.Error(), "npm run build") {
		t.Errorf("error should name the stale build and the fix, got: %v", err)
	}
}

// TestUpClaudeSDKStaleDistPiChild: a marker recording child-harness=pi (a
// cosplay-era dist) is refused with the same one-line fix.
func TestUpClaudeSDKStaleDistPiChild(t *testing.T) {
	upTestState(t)
	t.Setenv("HOTLINE_HARNESS", "claude-sdk")
	stubBinary(t, "node")
	claudeSDKTestEntryMarker(t, "harness=claude-sdk\nchild-harness=pi\n")

	var out, errOut bytes.Buffer
	err := cmdUp("Ada", false, nil, nil, t.TempDir(), &out, &errOut)
	if err == nil {
		t.Fatal("expected stale-dist error for a pi-child marker")
	}
	if !strings.Contains(err.Error(), "harness=pi") || !strings.Contains(err.Error(), "npm run build") {
		t.Errorf("error should name the pi child identity and the fix, got: %v", err)
	}
}

// TestUpClaudeSDKKnobPlumbing: .env-only SDK knobs are resolved by up and
// re-exported into the node harness env as the canonical HOTLINE_SDK_*
// values (including the deprecated HOTLINE_CLAUDE_SDK_MODEL fallback), which
// is how the TS harness — real-env-only — sees them.
func TestUpClaudeSDKKnobPlumbing(t *testing.T) {
	baseRoot := upTestState(t)
	t.Setenv("HOTLINE_HARNESS", "claude-sdk")
	for _, k := range []string{"HOTLINE_SDK_MODEL", "HOTLINE_SDK_EFFORT", "HOTLINE_SDK_MAX_TURNS", "HOTLINE_CLAUDE_SDK_MODEL"} {
		t.Setenv(k, "")
		if err := os.Unsetenv(k); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(filepath.Join(baseRoot, ".env"),
		[]byte("HOTLINE_CLAUDE_SDK_MODEL=claude-opus-4-8\nHOTLINE_SDK_EFFORT=xhigh\nHOTLINE_SDK_MAX_TURNS=40\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	stubBinary(t, "node")
	claudeSDKTestEntry(t)

	spawned := make(chan []string, 1)
	orig := startHarnessPipedStdinOpen
	startHarnessPipedStdinOpen = func(argv []string, dir string, env []string, logw io.Writer) (supervise.Harness, error) {
		select {
		case spawned <- env:
		default:
		}
		return &fakeUpHarness{done: make(chan struct{})}, nil
	}
	t.Cleanup(func() { startHarnessPipedStdinOpen = orig })

	var out, errOut bytes.Buffer
	done := make(chan error, 1)
	go func() {
		done <- cmdUp("Ada", false, nil, nil, t.TempDir(), &out, &errOut)
	}()

	var env []string
	select {
	case env = <-spawned:
	case err := <-done:
		t.Fatalf("cmdUp returned before spawning: %v", err)
	case <-time.After(5 * time.Second):
		t.Fatal("claude-sdk harness was not spawned")
	}
	if v, ok := envValue(env, "HOTLINE_SDK_MODEL"); !ok || v != "claude-opus-4-8" {
		t.Errorf("HOTLINE_SDK_MODEL = %q (present=%v), want legacy-resolved claude-opus-4-8", v, ok)
	}
	if v, ok := envValue(env, "HOTLINE_SDK_EFFORT"); !ok || v != "xhigh" {
		t.Errorf("HOTLINE_SDK_EFFORT = %q (present=%v), want xhigh", v, ok)
	}
	if v, ok := envValue(env, "HOTLINE_SDK_MAX_TURNS"); !ok || v != "40" {
		t.Errorf("HOTLINE_SDK_MAX_TURNS = %q (present=%v), want 40", v, ok)
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
		t.Fatal("cmdUp did not exit after SIGTERM")
	}
}

// TestUpClaudeSDKBadKnobFailsLaunch: an invalid HOTLINE_SDK_EFFORT fails
// `hotline up` loudly at launch (preflight), before anything spawns.
func TestUpClaudeSDKBadKnobFailsLaunch(t *testing.T) {
	upTestState(t)
	t.Setenv("HOTLINE_HARNESS", "claude-sdk")
	t.Setenv("HOTLINE_SDK_EFFORT", "xtreme")
	stubBinary(t, "node")
	claudeSDKTestEntry(t)

	var out, errOut bytes.Buffer
	err := cmdUp("Ada", false, nil, nil, t.TempDir(), &out, &errOut)
	if err == nil || !strings.Contains(err.Error(), "HOTLINE_SDK_EFFORT") {
		t.Fatalf("expected loud HOTLINE_SDK_EFFORT error at launch, got: %v", err)
	}
}

// TestUpClaudeSDKFromDotEnv: harness=claude-sdk is a first-class
// config.Harness value, so selecting it ONLY via the shared .env (no real
// env) works like every other harness.
func TestUpClaudeSDKFromDotEnv(t *testing.T) {
	baseRoot := upTestState(t)
	// upTestState leaves HOTLINE_HARNESS set-but-empty, which wins over the
	// .env in mergedEnv. Register the restore via t.Setenv, then truly unset
	// so only the .env speaks.
	t.Setenv("HOTLINE_HARNESS", "")
	if err := os.Unsetenv("HOTLINE_HARNESS"); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(baseRoot, ".env"), []byte("HOTLINE_HARNESS=claude-sdk\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	stubBinary(t, "node")
	claudeSDKTestEntry(t)

	spawned := make(chan []string, 1)
	orig := startHarnessPipedStdinOpen
	startHarnessPipedStdinOpen = func(argv []string, dir string, env []string, logw io.Writer) (supervise.Harness, error) {
		select {
		case spawned <- argv:
		default:
		}
		return &fakeUpHarness{done: make(chan struct{})}, nil
	}
	t.Cleanup(func() { startHarnessPipedStdinOpen = orig })

	var out, errOut bytes.Buffer
	done := make(chan error, 1)
	go func() {
		done <- cmdUp("Ada", false, nil, nil, t.TempDir(), &out, &errOut)
	}()

	select {
	case argv := <-spawned:
		if len(argv) < 2 || !strings.HasSuffix(argv[0], "node") {
			t.Errorf("argv = %v, want node entry spawn", argv)
		}
	case err := <-done:
		t.Fatalf("cmdUp returned before spawning: %v", err)
	case <-time.After(5 * time.Second):
		t.Fatal("claude-sdk harness was not spawned from .env selection")
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
		t.Fatal("cmdUp did not exit after SIGTERM")
	}
}

// TestUpClaudeSDKPersistedSettingsSurviveRestart proves the box half of the
// SDK-settings apply loop end-to-end at the supervisor seam: settings
// persisted through config.UpdateSDKEnv (the exact call the app's
// set_sdk_config control makes) survive a supervisor-driven harness restart
// (the same supervise.RequestRestart control file the handler writes) and
// reach the RESPAWNED child's environment as the canonical HOTLINE_SDK_*
// values — because cmd_up's claude-sdk start() re-runs LoadSDK on every
// spawn. This is the persist → restart → respawn → child-env leg of the
// apply loop; the harness_info restamp downstream is existing mcpchan
// behavior.
func TestUpClaudeSDKPersistedSettingsSurviveRestart(t *testing.T) {
	baseRoot := upTestState(t)
	t.Setenv("HOTLINE_HARNESS", "claude-sdk")
	for _, k := range []string{"HOTLINE_SDK_MODEL", "HOTLINE_SDK_EFFORT", "HOTLINE_SDK_MAX_TURNS", "HOTLINE_CLAUDE_SDK_MODEL"} {
		t.Setenv(k, "")
		if err := os.Unsetenv(k); err != nil {
			t.Fatal(err)
		}
	}
	// The box starts on opus/xhigh, from the shared .env.
	if err := os.WriteFile(filepath.Join(baseRoot, ".env"),
		[]byte("HOTLINE_SDK_MODEL=claude-opus-4-8\nHOTLINE_SDK_EFFORT=xhigh\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	stubBinary(t, "node")
	claudeSDKTestEntry(t)

	spawned := make(chan []string, 2)
	orig := startHarnessPipedStdinOpen
	startHarnessPipedStdinOpen = func(argv []string, dir string, env []string, logw io.Writer) (supervise.Harness, error) {
		select {
		case spawned <- env:
		default:
		}
		return &fakeUpHarness{done: make(chan struct{})}, nil
	}
	t.Cleanup(func() { startHarnessPipedStdinOpen = orig })

	var out, errOut bytes.Buffer
	done := make(chan error, 1)
	go func() {
		done <- cmdUp("Ada", false, nil, nil, t.TempDir(), &out, &errOut)
	}()

	var env []string
	select {
	case env = <-spawned:
	case err := <-done:
		t.Fatalf("cmdUp returned before spawning: %v", err)
	case <-time.After(5 * time.Second):
		t.Fatal("first claude-sdk spawn missing")
	}
	if v, ok := envValue(env, "HOTLINE_SDK_MODEL"); !ok || v != "claude-opus-4-8" {
		t.Fatalf("first spawn HOTLINE_SDK_MODEL = %q (present=%v), want claude-opus-4-8", v, ok)
	}

	// The app-side change: persist new knobs the exact way handleSetSDKConfig
	// does, then bounce via the same restart control file.
	model, effort := "claude-sonnet-4-6", "high"
	if err := config.UpdateSDKEnv(&model, &effort); err != nil {
		t.Fatalf("UpdateSDKEnv: %v", err)
	}
	supDir := supervise.Dir(filepath.Join(baseRoot, "bots", "Ada"))
	if err := supervise.RequestRestart(supDir, "sdk config change from app (rid test)"); err != nil {
		t.Fatalf("RequestRestart: %v", err)
	}

	// The supervisor's 2s poll consumes the request, bounces the fake
	// harness, and respawns — the respawned env must carry the NEW knobs.
	select {
	case env = <-spawned:
	case err := <-done:
		t.Fatalf("cmdUp exited instead of respawning: %v", err)
	case <-time.After(10 * time.Second):
		t.Fatal("no respawn after restart request")
	}
	if v, ok := envValue(env, "HOTLINE_SDK_MODEL"); !ok || v != "claude-sonnet-4-6" {
		t.Errorf("respawn HOTLINE_SDK_MODEL = %q (present=%v), want claude-sonnet-4-6", v, ok)
	}
	if v, ok := envValue(env, "HOTLINE_SDK_EFFORT"); !ok || v != "high" {
		t.Errorf("respawn HOTLINE_SDK_EFFORT = %q (present=%v), want high", v, ok)
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
		t.Fatal("cmdUp did not exit after SIGTERM")
	}
}

// syncBuffer is a tiny concurrency-safe buffer for the tests that read cmdUp's
// output while cmdUp is still writing it from another goroutine.
type syncBuffer struct {
	mu sync.Mutex
	b  strings.Builder
}

func (s *syncBuffer) Write(p []byte) (int, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.b.Write(p)
}

func (s *syncBuffer) String() string {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.b.String()
}
