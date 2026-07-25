package main

import (
	"bytes"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/lifecycle"
	"github.com/1broseidon/hotline/internal/supervise"
)

// writeMCPJSON writes a raw .mcp.json with a hotline entry carrying env, the
// exact layout Claude spawns the channel server from.
func writeMCPJSON(t *testing.T, dir string, env map[string]string) string {
	t.Helper()
	var pairs []string
	for k, v := range env {
		pairs = append(pairs, fmt.Sprintf("%q: %q", k, v))
	}
	body := fmt.Sprintf(`{
  "mcpServers": {
    "hotline": {
      "command": "/usr/local/bin/hotline",
      "args": ["run"],
      "env": {%s}
    }
  }
}`, strings.Join(pairs, ", "))
	path := filepath.Join(dir, ".mcp.json")
	if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestMCPServerEntryReturnsEnvAndBasenameCommand(t *testing.T) {
	dir := t.TempDir()
	body := `{
  "mcpServers": {
    "ada": {
      "command": "/home/x/go/bin/hotline",
      "args": ["run", "--bot", "Ada"],
      "env": {"HOTLINE_MC_DIR": "/custom/mc"}
    }
  }
}`
	path := filepath.Join(dir, ".mcp.json")
	if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
	name, env, botName, botSet, found, err := mcpServerEntry(path)
	if err != nil {
		t.Fatal(err)
	}
	if !found || name != "ada" {
		t.Fatalf("mcpServerEntry = %q found=%v, want ada/true (absolute hotline command must match by basename)", name, found)
	}
	if !botSet || botName != "Ada" {
		t.Fatalf("mcpServerEntry bot = %q set=%v, want Ada/true", botName, botSet)
	}
	if env["HOTLINE_MC_DIR"] != "/custom/mc" {
		t.Fatalf("env = %v, want HOTLINE_MC_DIR=/custom/mc", env)
	}
}

func TestMCPServerBotArgsMirrorCLIParsing(t *testing.T) {
	cases := []struct {
		name       string
		args       string
		env        string
		wantBot    string
		wantBotSet bool
	}{
		{name: "explicit empty beats env", args: `["run","--bot="]`, env: `,"env":{"HOTLINE_BOT":"Ambient"}`, wantBotSet: true},
		{name: "passthrough bot ignored", args: `["run","--","--bot","Ada"]`, wantBotSet: false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			dir := t.TempDir()
			body := fmt.Sprintf(`{"mcpServers":{"hotline":{"command":"hotline","args":%s%s}}}`, tc.args, tc.env)
			path := filepath.Join(dir, ".mcp.json")
			if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
				t.Fatal(err)
			}
			_, _, botName, botSet, found, err := mcpServerEntry(path)
			if err != nil {
				t.Fatal(err)
			}
			if !found || botSet != tc.wantBotSet || botName != tc.wantBot {
				t.Fatalf("bot=%q set=%v found=%v, want %q/%v/true", botName, botSet, found, tc.wantBot, tc.wantBotSet)
			}
		})
	}
}

func TestMCPEnvLegacyBotDoesNotOverrideAmbientPrimaryBot(t *testing.T) {
	t.Setenv("HOTLINE_BOT", "Bob")
	t.Setenv("TELE_GO_BOT", "Old")
	dir := t.TempDir()
	path := writeMCPJSON(t, dir, map[string]string{"TELE_GO_BOT": "Ada"})
	identity, err := adoptMCPServerEnv(path, &bytes.Buffer{})
	if err != nil {
		t.Fatal(err)
	}
	if !identity.Found || identity.BotName != "Bob" {
		t.Fatalf("effective bot = %+v, want ambient HOTLINE_BOT Bob", identity)
	}
}

func TestAdoptMCPServerEnvRefusesInvalidEnvironmentValue(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ".mcp.json")
	body := `{"mcpServers":{"hotline":{"command":"hotline","env":{"HOTLINE_STATE_DIR":"bad\u0000path"}}}}`
	if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := adoptMCPServerEnv(path, &bytes.Buffer{}); err == nil || !strings.Contains(err.Error(), "adopting HOTLINE_STATE_DIR") {
		t.Fatalf("invalid env adoption error = %v", err)
	}
}

func TestAdoptMCPServerEnvAppliesHotlineKeysOnly(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("HOTLINE_MC_DIR", "/old/mc")
	t.Setenv("HOTLINE_PROVIDERS", "telegram:Old")
	t.Setenv("PATH_LIKE", "keep")
	path := writeMCPJSON(t, dir, map[string]string{
		"HOTLINE_MC_DIR":        "/new/mc",
		"HOTLINE_PROVIDERS":     "telegram:Ada",
		"PATH_LIKE":             "clobber",
		lifecycle.OwnerLeaseEnv: "bogus",
		supervise.EnvDir:        "/bogus",
	})
	var errOut bytes.Buffer
	if _, err := adoptMCPServerEnv(path, &errOut); err != nil {
		t.Fatal(err)
	}

	if got := os.Getenv("HOTLINE_MC_DIR"); got != "/new/mc" {
		t.Errorf("HOTLINE_MC_DIR = %q, want /new/mc (the .mcp.json value wins inside the child, so up must resolve with it)", got)
	}
	if got := os.Getenv("HOTLINE_PROVIDERS"); got != "telegram:Ada" {
		t.Errorf("HOTLINE_PROVIDERS = %q, want telegram:Ada", got)
	}
	if got := os.Getenv("PATH_LIKE"); got != "keep" {
		t.Errorf("non-HOTLINE key adopted: PATH_LIKE = %q", got)
	}
	if got := os.Getenv(lifecycle.OwnerLeaseEnv); got == "bogus" {
		t.Error("internal lease marker must never be adopted from .mcp.json")
	}
	if !strings.Contains(errOut.String(), "HOTLINE_MC_DIR") {
		t.Error("expected an override notice for HOTLINE_MC_DIR")
	}
	if !strings.Contains(errOut.String(), lifecycle.OwnerLeaseEnv) {
		t.Error("expected a warning about the ignored internal lease key")
	}
}

func TestProjectScopedCommandCoverage(t *testing.T) {
	for _, cmd := range []string{
		"up", "start", "down", "status", "tail", "pair", "deny", "revoke",
		"schedule", "loop", "notify", "job", "mission", "source", "relay",
	} {
		if !projectScopedCommand(cmd) {
			t.Errorf("%s must adopt project MCP identity", cmd)
		}
	}
	for _, cmd := range []string{"run", "setup", "init", "version", "--help", "unknown"} {
		if projectScopedCommand(cmd) {
			t.Errorf("%s must not adopt project MCP identity", cmd)
		}
	}
}

func TestProjectScopedCommandsRefuseUnclearMCPIdentity(t *testing.T) {
	cases := map[string]string{
		"malformed":                `{"mcpServers":`,
		"misnamed command":         `{"mcpServers":{"hotline":{"command":"not-hotline"}}}`,
		"multiple hotline entries": `{"mcpServers":{"one":{"command":"hotline"},"two":{"command":"/usr/local/bin/hotline"}}}`,
	}
	for name, body := range cases {
		t.Run(name, func(t *testing.T) {
			setupTestState(t)
			projectDir := t.TempDir()
			if err := os.WriteFile(filepath.Join(projectDir, ".mcp.json"), []byte(body), 0o600); err != nil {
				t.Fatal(err)
			}
			if _, _, _, _, _, err := resolveInvocation([]string{"down"}, projectDir, &bytes.Buffer{}); err == nil {
				t.Fatal("box-scoped down accepted unclear MCP identity")
			}
		})
	}
}

func TestProjectMCPEnvRelayUsesUnambiguousNamedInstance(t *testing.T) {
	setupTestState(t)
	t.Setenv("HOTLINE_PROVIDERS", "telegram")
	projectDir := t.TempDir()
	writeMCPJSON(t, projectDir, map[string]string{"HOTLINE_PROVIDERS": "telegram:Ada"})
	botName, _, _, cmd, botExplicit, err := resolveInvocation([]string{"relay", "status"}, projectDir, &bytes.Buffer{})
	if err != nil {
		t.Fatal(err)
	}
	if cmd != "relay" || botName != "Ada" || botExplicit {
		t.Fatalf("relay resolved cmd=%q bot=%q explicit=%v, want relay/Ada/false", cmd, botName, botExplicit)
	}
	botName, _, _, _, botExplicit, err = resolveInvocation([]string{"relay", "status", "--bot="}, projectDir, &bytes.Buffer{})
	if err != nil {
		t.Fatal(err)
	}
	if botName != "" || !botExplicit {
		t.Fatalf("relay explicit default resolved bot=%q explicit=%v", botName, botExplicit)
	}

	ambiguousDir := t.TempDir()
	writeMCPJSON(t, ambiguousDir, map[string]string{"HOTLINE_PROVIDERS": "telegram:Ada,discord:Bob"})
	if _, _, _, _, _, err := resolveInvocation([]string{"relay", "status"}, ambiguousDir, &bytes.Buffer{}); err == nil || !strings.Contains(err.Error(), "multiple named instances") {
		t.Fatalf("ambiguous relay error = %v", err)
	}
}

func TestProjectMCPBotConflictRefusesUp(t *testing.T) {
	upTestState(t)
	projectDir := t.TempDir()
	body := `{"mcpServers":{"hotline":{"command":"hotline","args":["run","--bot","Ada"]}}}`
	if err := os.WriteFile(filepath.Join(projectDir, ".mcp.json"), []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
	var out, errOut bytes.Buffer
	err := cmdUp("Bob", true, nil, nil, projectDir, &out, &errOut)
	if err == nil || !strings.Contains(err.Error(), "conflicts") || !strings.Contains(err.Error(), "Ada") || !strings.Contains(err.Error(), "Bob") {
		t.Fatalf("up bot conflict error = %v", err)
	}
	err = cmdUp("", true, nil, nil, projectDir, &out, &errOut)
	if err == nil || !strings.Contains(err.Error(), "conflicts") || !strings.Contains(err.Error(), "Ada") {
		t.Fatalf("up explicit-default conflict error = %v", err)
	}
}

func TestProjectMCPEnvDownAndStatusTargetNamedBox(t *testing.T) {
	baseRoot := setupTestState(t)
	t.Setenv("HOTLINE_PROVIDERS", "telegram")
	t.Setenv("HOTLINE_BOT", "")
	os.Unsetenv("HOTLINE_BOT")
	t.Setenv("TELE_GO_BOT", "")
	os.Unsetenv("TELE_GO_BOT")
	t.Setenv("HOTLINE_MC_DIR", "")
	os.Unsetenv("HOTLINE_MC_DIR")

	projectDir := t.TempDir()
	customMC := filepath.Join(baseRoot, "mc-ada")
	writeMCPJSON(t, projectDir, map[string]string{
		"HOTLINE_PROVIDERS": "telegram:Ada",
		"HOTLINE_MC_DIR":    customMC,
	})
	nestedDir := filepath.Join(projectDir, "sub", "package")
	if err := os.MkdirAll(nestedDir, 0o700); err != nil {
		t.Fatal(err)
	}

	// Keep only the base supervisor lock live. A wrongly resolved down would
	// enter its state-read/signal path; the named box correctly reports idle.
	releaseBase, err := supervise.AcquireLock(supervise.Dir(baseRoot))
	if err != nil {
		t.Fatal(err)
	}
	defer releaseBase()

	botName, providerSel, args, cmd, botExplicit, resolveErr := resolveInvocation([]string{"down"}, nestedDir, &bytes.Buffer{})
	if resolveErr != nil {
		t.Fatal(resolveErr)
	}
	if cmd != "down" || len(args) != 1 {
		t.Fatalf("resolved down invocation = bot %q provider %q args %v cmd %q", botName, providerSel, args, cmd)
	}
	if got := os.Getenv("HOTLINE_MC_DIR"); got != customMC {
		t.Fatalf("HOTLINE_MC_DIR = %q, want project value %q", got, customMC)
	}
	namedRoot := filepath.Join(baseRoot, "bots", "Ada")
	var downOut, downErr bytes.Buffer
	if err := cmdDown(botName, botExplicit, args[1:], nestedDir, &downOut, &downErr); err != nil {
		t.Fatalf("project-scoped down touched the live base supervisor: %v", err)
	}
	if !strings.Contains(downOut.String(), "not running") {
		t.Fatalf("named-box down output = %q, want not-running", downOut.String())
	}

	botName, providerSel, _, cmd, _, resolveErr = resolveInvocation([]string{"status"}, nestedDir, &bytes.Buffer{})
	if resolveErr != nil {
		t.Fatal(resolveErr)
	}
	if cmd != "status" {
		t.Fatalf("resolved status command = %q", cmd)
	}
	var statusOut, statusErr bytes.Buffer
	if err := cmdStatus(providerSel, botName, nestedDir, &statusOut, &statusErr); err != nil {
		t.Fatal(err)
	}
	wantState := "state dir:   " + namedRoot + "\n"
	if !strings.Contains(statusOut.String(), wantState) {
		t.Fatalf("status did not read named provider state; want %q in:\n%s", wantState, statusOut.String())
	}
	if strings.Contains(statusOut.String(), "state dir:   "+baseRoot+"\n") {
		t.Fatalf("status crossed into base provider state:\n%s", statusOut.String())
	}
	if !strings.Contains(statusOut.String(), "supervisor:  not running") {
		t.Fatalf("status inspected the live base supervisor instead of named box:\n%s", statusOut.String())
	}
}

func TestProjectMCPEnvImplicitStatusRequiresConfiguredTelegram(t *testing.T) {
	setupTestState(t)
	t.Setenv("HOTLINE_PROVIDERS", "telegram")
	projectDir := t.TempDir()
	writeMCPJSON(t, projectDir, map[string]string{
		"HOTLINE_PROVIDERS": "discord:Ada",
		"HOTLINE_BOT":       "Ada",
	})
	botName, providerSel, _, _, _, err := resolveInvocation([]string{"status"}, projectDir, &bytes.Buffer{})
	if err != nil {
		t.Fatal(err)
	}
	if err := cmdStatus(providerSel, botName, projectDir, &bytes.Buffer{}, &bytes.Buffer{}); err == nil || !strings.Contains(err.Error(), "telegram is not configured") {
		t.Fatalf("implicit status without Telegram error = %v", err)
	}
}

func TestProjectMCPEnvStatusRefusesAmbientBotConflict(t *testing.T) {
	setupTestState(t)
	t.Setenv("HOTLINE_PROVIDERS", "telegram")
	t.Setenv("HOTLINE_BOT", "Bob")
	projectDir := t.TempDir()
	writeMCPJSON(t, projectDir, map[string]string{"HOTLINE_PROVIDERS": "telegram:Ada"})

	botName, providerSel, _, _, _, err := resolveInvocation([]string{"status"}, projectDir, &bytes.Buffer{})
	if err != nil {
		t.Fatal(err)
	}
	if botName != "Bob" {
		t.Fatalf("ambient bot = %q, want Bob before conflict validation", botName)
	}
	if err := cmdStatus(providerSel, botName, projectDir, &bytes.Buffer{}, &bytes.Buffer{}); err == nil || !strings.Contains(err.Error(), "conflicts") {
		t.Fatalf("status ambient/provider conflict error = %v", err)
	}
}

func TestProjectMCPBotArgFallbackAndExplicitOverride(t *testing.T) {
	setupTestState(t)
	t.Setenv("HOTLINE_BOT", "")
	os.Unsetenv("HOTLINE_BOT")
	projectDir := t.TempDir()
	body := `{"mcpServers":{"hotline":{"command":"hotline","args":["run","--bot","Ada"]}}}`
	if err := os.WriteFile(filepath.Join(projectDir, ".mcp.json"), []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}

	botName, _, _, _, botExplicit, err := resolveInvocation([]string{"status"}, projectDir, &bytes.Buffer{})
	if err != nil {
		t.Fatal(err)
	}
	if botName != "Ada" || botExplicit {
		t.Fatalf("adopted MCP --bot = %q explicit=%v, want Ada/false", botName, botExplicit)
	}
	botName, _, _, _, botExplicit, err = resolveInvocation([]string{"status", "--bot", "Bob"}, projectDir, &bytes.Buffer{})
	if err != nil {
		t.Fatal(err)
	}
	if botName != "Bob" || !botExplicit {
		t.Fatalf("explicit CLI --bot = %q explicit=%v, want Bob/true", botName, botExplicit)
	}
	botName, _, _, _, botExplicit, err = resolveInvocation([]string{"status", "--bot="}, projectDir, &bytes.Buffer{})
	if err != nil {
		t.Fatal(err)
	}
	if botName != "" || !botExplicit {
		t.Fatalf("explicit empty CLI --bot = %q explicit=%v, want default/true", botName, botExplicit)
	}
}

// TestUpClaudeMCPEnvChildClaimsOwnership is the regression test for the
// 2026-07-19 live failure: with HOTLINE_MC_DIR present only in .mcp.json, the
// supervisor reserved one box identity while the Claude-spawned `hotline run`
// child resolved another; ClaimBox refused ("live supervisor reservation does
// not match this runtime"), so the telegram channel was never served and
// bot.pid never written. Under `up` the supervisor must adopt the .mcp.json
// identity env so the child's claim — the gate the poller and pid write sit
// behind — succeeds, while remaining the single reservation for the box.
func TestUpClaudeMCPEnvChildClaimsOwnership(t *testing.T) {
	upTestState(t)
	t.Setenv("HOTLINE_MC_DIR", "")
	os.Unsetenv("HOTLINE_MC_DIR")
	clearAnthropicEnv(t)
	stubBinary(t, "claude")
	fakeClaudeRunner(t)

	workDir := t.TempDir()
	customMC := filepath.Join(t.TempDir(), "mc-custom")
	writeMCPJSON(t, workDir, map[string]string{"HOTLINE_MC_DIR": customMC})

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
		done <- cmdUp("", false, []string{"--foreground"}, nil, workDir, &out, &errOut)
	}()

	var got spawn
	select {
	case got = <-spawned:
	case <-time.After(5 * time.Second):
		t.Fatal("claude harness was not spawned")
	}

	// The supervised claude env must carry the adopted identity plus the lease.
	var lease string
	joined := strings.Join(got.env, "\n")
	if !strings.Contains(joined, "HOTLINE_MC_DIR="+customMC) {
		t.Errorf("supervised claude env lost the adopted HOTLINE_MC_DIR; env:\n%s", joined)
	}
	for _, e := range got.env {
		if strings.HasPrefix(e, lifecycle.OwnerLeaseEnv+"=") {
			lease = strings.TrimPrefix(e, lifecycle.OwnerLeaseEnv+"=")
		}
	}
	if lease == "" {
		t.Fatal("supervised claude env carries no owner lease")
	}

	// Simulate the .mcp.json-spawned `hotline run` child: same identity env
	// (Claude merges the entry env over the inherited env; adoption already
	// applied it to this process), inherited lease. The claim must succeed —
	// this exact call refused before the fix.
	box, err := config.ResolveBox("")
	if err != nil {
		t.Fatal(err)
	}
	mcCfg, err := config.MissionControlForRoot("claude", box.Root)
	if err != nil {
		t.Fatal(err)
	}
	if mcCfg.Enabled && mcCfg.Dir != customMC {
		t.Fatalf("child MC dir = %q, want %q", mcCfg.Dir, customMC)
	}
	spec, err := boxOwnerSpec(box, "claude", workDir, mcCfg)
	if err != nil {
		t.Fatal(err)
	}
	childLease, err := lifecycle.ClaimBox(spec, lease)
	if err != nil {
		t.Fatalf("channel child ClaimBox under the supervisor lease: %v (this is the up+claude+.mcp.json channel-serving regression)", err)
	}

	// Single-consumer invariant: a second channel runtime for the same box must
	// refuse while the first claim is live.
	if _, err := lifecycle.ClaimBox(spec, lease); err == nil {
		t.Error("second concurrent channel claim succeeded; the single-consumer guard is gone")
	} else if !strings.Contains(err.Error(), lifecycle.OwnerConflictMarker) {
		t.Errorf("second claim error %v, want %s", err, lifecycle.OwnerConflictMarker)
	}
	childLease.Release()

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
}
