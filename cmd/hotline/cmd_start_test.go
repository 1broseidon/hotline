package main

import (
	"bytes"
	"errors"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
)

// fakeClaude puts a stub claude binary on PATH and captures execProcess calls
// instead of launching anything.
func fakeClaude(t *testing.T) (gotArgv *[]string, gotEnv *[]string) {
	t.Helper()
	binDir := t.TempDir()
	stub := filepath.Join(binDir, "claude")
	if err := os.WriteFile(stub, []byte("#!/bin/sh\nexit 0\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", binDir+string(os.PathListSeparator)+os.Getenv("PATH"))

	var argv, env []string
	orig := execProcess
	execProcess = func(bin string, a []string, e []string) error {
		argv, env = a, e
		return nil
	}
	t.Cleanup(func() { execProcess = orig })
	return &argv, &env
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

func TestStartWarnsWhenNothingConfigured(t *testing.T) {
	startTestState(t)
	fakeClaudeRunner(t)
	argv, _ := fakeClaude(t)
	var out, errOut bytes.Buffer
	if err := cmdStart("", nil, nil, t.TempDir(), &out, &errOut); err != nil {
		t.Fatalf("start: %v", err)
	}
	if strings.Join(*argv, " ") != "claude" {
		t.Errorf("argv = %v, want plain claude", *argv)
	}
	if !strings.Contains(errOut.String(), "hotline init") {
		t.Error("missing not-set-up warning")
	}
}

func TestStartPluginPathAllowlisted(t *testing.T) {
	startTestState(t)
	fakeClaudeRunner(t)
	argv, _ := fakeClaude(t)
	dir := t.TempDir()
	writeProjectSettings(t, dir, `{"enabledPlugins": {"hotline@hotline": true}}`)
	if err := os.WriteFile(claudeStateFile(), []byte(`{"cachedGrowthBookFeatures": {"tengu_harbor_ledger": [{"marketplace": "hotline", "plugin": "hotline"}]}}`), 0o644); err != nil {
		t.Fatal(err)
	}
	var out, errOut bytes.Buffer
	if err := cmdStart("", nil, []string{"--continue"}, dir, &out, &errOut); err != nil {
		t.Fatalf("start: %v", err)
	}
	want := "claude --channels plugin:hotline@hotline --continue"
	if strings.Join(*argv, " ") != want {
		t.Errorf("argv = %v, want %q", *argv, want)
	}
	if strings.Contains(errOut.String(), "dev-channel") {
		t.Errorf("spurious dev-channel notice: %s", errOut.String())
	}
}

func TestStartPluginPathNotAllowlistedFallsBack(t *testing.T) {
	startTestState(t)
	fakeClaudeRunner(t)
	argv, _ := fakeClaude(t)
	dir := t.TempDir()
	writeProjectSettings(t, dir, `{"enabledPlugins": {"hotline@hotline": true}}`)
	var out, errOut bytes.Buffer
	if err := cmdStart("", nil, nil, dir, &out, &errOut); err != nil {
		t.Fatalf("start: %v", err)
	}
	want := "claude --dangerously-load-development-channels plugin:hotline@hotline"
	if strings.Join(*argv, " ") != want {
		t.Errorf("argv = %v, want %q", *argv, want)
	}
	if !strings.Contains(errOut.String(), "approved channels list") {
		t.Errorf("missing allowlist notice: %s", errOut.String())
	}
}

func TestStartRawMCPJSONWinsOverPlugin(t *testing.T) {
	startTestState(t)
	fakeClaudeRunner(t)
	argv, _ := fakeClaude(t)
	dir := t.TempDir()
	writeProjectSettings(t, dir, `{"enabledPlugins": {"hotline@hotline": true}}`)
	mcp := `{"mcpServers": {"hotline": {"command": "hotline", "args": ["run"]}}}`
	if err := os.WriteFile(filepath.Join(dir, ".mcp.json"), []byte(mcp), 0o644); err != nil {
		t.Fatal(err)
	}
	var out, errOut bytes.Buffer
	if err := cmdStart("", nil, nil, dir, &out, &errOut); err != nil {
		t.Fatalf("start: %v", err)
	}
	want := "claude --dangerously-load-development-channels server:hotline"
	if strings.Join(*argv, " ") != want {
		t.Errorf("argv = %v, want %q", *argv, want)
	}
	if !strings.Contains(errOut.String(), "raw .mcp.json") {
		t.Errorf("missing raw-path notice: %s", errOut.String())
	}
}

func TestStartPassthroughAndEnv(t *testing.T) {
	startTestState(t)
	fakeClaudeRunner(t)
	argv, env := fakeClaude(t)
	var out, errOut bytes.Buffer
	err := cmdStart("work", []string{"--providers", "telegram:work"}, []string{"--continue"}, t.TempDir(), &out, &errOut)
	if err != nil {
		t.Fatalf("start: %v", err)
	}
	if (*argv)[len(*argv)-1] != "--continue" {
		t.Errorf("passthrough missing: %v", *argv)
	}
	joined := strings.Join(*env, "\n")
	if !strings.Contains(joined, "HOTLINE_PROVIDERS=telegram:work") {
		t.Error("HOTLINE_PROVIDERS not exported")
	}
	if !strings.Contains(joined, "HOTLINE_BOT=work") {
		t.Error("HOTLINE_BOT not exported")
	}
}

func TestStartYoloExportsPosture(t *testing.T) {
	startTestState(t)
	fakeClaudeRunner(t)
	argv, env := fakeClaude(t)
	var out, errOut bytes.Buffer
	if err := cmdStart("", []string{"--yolo"}, nil, t.TempDir(), &out, &errOut); err != nil {
		t.Fatalf("start: %v", err)
	}
	if !slices.Contains(*argv, "--dangerously-skip-permissions") {
		t.Errorf("argv missing yolo flag: %v", *argv)
	}
	if !strings.Contains(strings.Join(*env, "\n"), "HOTLINE_YOLO=1") {
		t.Errorf("env missing HOTLINE_YOLO: %v", *env)
	}
}

func TestStartReadsServerNameFromMCPJSON(t *testing.T) {
	startTestState(t)
	fakeClaudeRunner(t)
	argv, _ := fakeClaude(t)
	dir := t.TempDir()
	mcp := `{"mcpServers": {"my-channel": {"command": "hotline", "args": ["run"]}}}`
	if err := os.WriteFile(filepath.Join(dir, ".mcp.json"), []byte(mcp), 0o644); err != nil {
		t.Fatal(err)
	}
	var out, errOut bytes.Buffer
	if err := cmdStart("", nil, nil, dir, &out, &errOut); err != nil {
		t.Fatalf("start: %v", err)
	}
	if (*argv)[2] != "server:my-channel" {
		t.Errorf("argv = %v, want server:my-channel", *argv)
	}
	if strings.Contains(errOut.String(), "no .mcp.json") {
		t.Error("spurious .mcp.json warning")
	}
}

func TestStartBlocksWithoutClaude(t *testing.T) {
	startTestState(t)
	t.Setenv("PATH", t.TempDir()) // empty PATH: no claude
	var out, errOut bytes.Buffer
	err := cmdStart("", nil, nil, t.TempDir(), &out, &errOut)
	if err == nil || !strings.Contains(err.Error(), "claude not found") {
		t.Errorf("want missing-binary error, got %v", err)
	}
}

func TestStartWarnsMissingTokenAndSignal(t *testing.T) {
	startTestState(t)
	t.Setenv("TELEGRAM_BOT_TOKEN", "")
	t.Setenv("SIGNAL_ACCOUNT", "+15551234567")
	fakeClaudeRunner(t)
	fakeClaude(t)
	origCheck := signalCheck
	signalCheck = func(url string) error { return errors.New("connection refused") }
	t.Cleanup(func() { signalCheck = origCheck })

	var out, errOut bytes.Buffer
	err := cmdStart("", []string{"--providers", "telegram,signal"}, nil, t.TempDir(), &out, &errOut)
	if err != nil {
		t.Fatalf("start should warn, not block: %v", err)
	}
	if !strings.Contains(errOut.String(), "no telegram token") {
		t.Errorf("missing token warning, got: %s", errOut.String())
	}
	if !strings.Contains(errOut.String(), "signal daemon not reachable") {
		t.Errorf("missing signal warning, got: %s", errOut.String())
	}
}

func TestSplitPassthrough(t *testing.T) {
	head, tail := splitPassthrough([]string{"start", "--providers", "telegram", "--", "--continue", "--bot", "x"})
	if len(head) != 3 || len(tail) != 3 || tail[0] != "--continue" {
		t.Errorf("head=%v tail=%v", head, tail)
	}
	head, tail = splitPassthrough([]string{"status"})
	if len(head) != 1 || tail != nil {
		t.Errorf("head=%v tail=%v", head, tail)
	}
}
