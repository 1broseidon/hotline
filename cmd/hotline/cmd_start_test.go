package main

import (
	"bytes"
	"errors"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/1broseidon/hotline/internal/config"
)

// clearAnthropicEnv removes the alternate-provider keys from the real process
// environment so a .env-configured value actually injects (real env wins).
func clearAnthropicEnv(t *testing.T) {
	t.Helper()
	for _, k := range config.AnthropicEnvKeys {
		t.Setenv(k, "") // register restore-on-cleanup
		os.Unsetenv(k)  // then clear for the test
	}
}

func startTestState(t *testing.T) string {
	t.Helper()
	dir := setupTestState(t)
	t.Setenv("HOTLINE_PROVIDERS", "")
	t.Setenv("HOTLINE_BOT", "")
	t.Setenv("TELE_GO_BOT", "")
	t.Setenv("TELEGRAM_BOT_TOKEN", goodToken)
	return dir
}

func TestStartRoutesToUpAndWarns(t *testing.T) {
	startTestState(t)
	wantErr := errors.New("routed")
	type call struct {
		bot, dir          string
		botExplicit       bool
		args, passthrough []string
		stdout, stderr    io.Writer
	}
	var got call
	orig := routeStartToUp
	routeStartToUp = func(bot string, botExplicit bool, args, passthrough []string, dir string, stdout, stderr io.Writer) error {
		got = call{bot: bot, botExplicit: botExplicit, dir: dir, args: args, passthrough: passthrough, stdout: stdout, stderr: stderr}
		return wantErr
	}
	t.Cleanup(func() { routeStartToUp = orig })

	var out, errOut bytes.Buffer
	args := []string{"--providers", "telegram:work", "--yolo"}
	passthrough := []string{"--continue"}
	err := cmdStart("work", true, args, passthrough, "/project", &out, &errOut)
	if !errors.Is(err, wantErr) {
		t.Fatalf("start error = %v, want routed error", err)
	}
	if got.bot != "work" || !got.botExplicit || got.dir != "/project" || !reflect.DeepEqual(got.args, args) || !reflect.DeepEqual(got.passthrough, passthrough) {
		t.Fatalf("routed call = %+v", got)
	}
	if got.stdout != &out || got.stderr != &errOut {
		t.Fatal("start did not preserve up's I/O streams")
	}
	if notice := errOut.String(); !strings.Contains(notice, "start") || !strings.Contains(notice, "deprecated") || !strings.Contains(notice, "hotline up") {
		t.Fatalf("deprecation notice = %q", notice)
	}
}

func TestChannelArgsWarnsWhenNothingConfigured(t *testing.T) {
	startTestState(t)
	fakeClaudeRunner(t)
	var errOut bytes.Buffer
	if got := channelArgs(t.TempDir(), &errOut); got != nil {
		t.Fatalf("channel args = %v, want none", got)
	}
	if !strings.Contains(errOut.String(), "hotline init") {
		t.Error("missing not-set-up warning")
	}
}

func TestChannelArgsPluginPathAllowlisted(t *testing.T) {
	startTestState(t)
	fakeClaudeRunner(t)
	dir := t.TempDir()
	writeProjectSettings(t, dir, `{"enabledPlugins": {"hotline@hotline": true}}`)
	if err := os.WriteFile(claudeStateFile(), []byte(`{"cachedGrowthBookFeatures": {"tengu_harbor_ledger": [{"marketplace": "hotline", "plugin": "hotline"}]}}`), 0o644); err != nil {
		t.Fatal(err)
	}
	var errOut bytes.Buffer
	got := channelArgs(dir, &errOut)
	want := []string{"--channels", "plugin:hotline@hotline"}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("channel args = %v, want %v", got, want)
	}
	if strings.Contains(errOut.String(), "dev-channel") {
		t.Errorf("spurious dev-channel notice: %s", errOut.String())
	}
}

func TestChannelArgsPluginPathNotAllowlistedFallsBack(t *testing.T) {
	startTestState(t)
	fakeClaudeRunner(t)
	dir := t.TempDir()
	writeProjectSettings(t, dir, `{"enabledPlugins": {"hotline@hotline": true}}`)
	var errOut bytes.Buffer
	got := channelArgs(dir, &errOut)
	want := []string{"--dangerously-load-development-channels", "plugin:hotline@hotline"}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("channel args = %v, want %v", got, want)
	}
	if !strings.Contains(errOut.String(), "approved channels list") {
		t.Errorf("missing allowlist notice: %s", errOut.String())
	}
}

func TestChannelArgsRawMCPJSONWinsOverPlugin(t *testing.T) {
	startTestState(t)
	fakeClaudeRunner(t)
	dir := t.TempDir()
	writeProjectSettings(t, dir, `{"enabledPlugins": {"hotline@hotline": true}}`)
	mcp := `{"mcpServers": {"hotline": {"command": "hotline", "args": ["run"]}}}`
	if err := os.WriteFile(filepath.Join(dir, ".mcp.json"), []byte(mcp), 0o644); err != nil {
		t.Fatal(err)
	}
	var errOut bytes.Buffer
	got := channelArgs(dir, &errOut)
	want := []string{"--dangerously-load-development-channels", "server:hotline"}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("channel args = %v, want %v", got, want)
	}
	if !strings.Contains(errOut.String(), "raw .mcp.json") {
		t.Errorf("missing raw-path notice: %s", errOut.String())
	}
}

func TestChannelArgsReadsServerNameFromMCPJSON(t *testing.T) {
	startTestState(t)
	fakeClaudeRunner(t)
	dir := t.TempDir()
	mcp := `{"mcpServers": {"my-channel": {"command": "hotline", "args": ["run"]}}}`
	if err := os.WriteFile(filepath.Join(dir, ".mcp.json"), []byte(mcp), 0o644); err != nil {
		t.Fatal(err)
	}
	var errOut bytes.Buffer
	got := channelArgs(dir, &errOut)
	if len(got) != 2 || got[1] != "server:my-channel" {
		t.Errorf("channel args = %v, want server:my-channel", got)
	}
}

func TestWarnMissingTokenAndSignal(t *testing.T) {
	startTestState(t)
	t.Setenv("TELEGRAM_BOT_TOKEN", "")
	t.Setenv("SIGNAL_ACCOUNT", "+15551234567")
	origCheck := signalCheck
	signalCheck = func(url string) error { return errors.New("connection refused") }
	t.Cleanup(func() { signalCheck = origCheck })

	var errOut bytes.Buffer
	t.Setenv("HOTLINE_PROVIDERS", "telegram,signal")
	warnMissingCreds("", &errOut)
	if !strings.Contains(errOut.String(), "no telegram token") {
		t.Errorf("missing token warning, got: %s", errOut.String())
	}
	if !strings.Contains(errOut.String(), "signal daemon not reachable") {
		t.Errorf("missing signal warning, got: %s", errOut.String())
	}
}

func TestSplitPassthrough(t *testing.T) {
	head, tail := splitPassthrough([]string{"up", "--providers", "telegram", "--", "--continue", "--bot", "x"})
	if len(head) != 3 || len(tail) != 3 || tail[0] != "--continue" {
		t.Errorf("head=%v tail=%v", head, tail)
	}
	head, tail = splitPassthrough([]string{"status"})
	if len(head) != 1 || tail != nil {
		t.Errorf("head=%v tail=%v", head, tail)
	}
}
