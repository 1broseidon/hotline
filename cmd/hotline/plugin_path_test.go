package main

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// fakeClaudeRunner stubs the claude CLI shell-out and records invocations.
// It also points the user-settings and allowlist seams at empty temp files so
// tests never read the developer's real ~/.claude state.
func fakeClaudeRunner(t *testing.T) *[][]string {
	t.Helper()
	var calls [][]string
	origRun := runClaude
	runClaude = func(dir string, args ...string) (string, error) {
		calls = append(calls, args)
		return "ok", nil
	}
	empty := t.TempDir()
	origUser, origState, origManaged := userSettingsFile, claudeStateFile, managedSettingsPath
	userSettingsFile = func() string { return filepath.Join(empty, "settings.json") }
	claudeStateFile = func() string { return filepath.Join(empty, ".claude.json") }
	managedSettingsPath = filepath.Join(empty, "managed-settings.json")
	t.Cleanup(func() {
		runClaude, userSettingsFile, claudeStateFile, managedSettingsPath = origRun, origUser, origState, origManaged
	})
	return &calls
}

func writeProjectSettings(t *testing.T, dir string, content string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Join(dir, ".claude"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, ".claude", "settings.json"), []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

func TestInitPluginInstallsMarketplaceAndPlugin(t *testing.T) {
	calls := fakeClaudeRunner(t)
	dir := t.TempDir()
	var out bytes.Buffer
	if err := cmdInit("", nil, dir, &out); err != nil {
		t.Fatalf("init: %v", err)
	}
	if len(*calls) != 2 {
		t.Fatalf("want 2 claude invocations, got %v", *calls)
	}
	if got := strings.Join((*calls)[0], " "); got != "plugin marketplace add 1broseidon/hotline" {
		t.Errorf("first call = %q", got)
	}
	if got := strings.Join((*calls)[1], " "); got != "plugin install hotline@hotline -s project" {
		t.Errorf("second call = %q", got)
	}
	if !strings.Contains(out.String(), "hotline start") {
		t.Error("missing next-step hint")
	}
}

func TestInitPluginSkipsInstallWhenEnabled(t *testing.T) {
	calls := fakeClaudeRunner(t)
	dir := t.TempDir()
	writeProjectSettings(t, dir, `{"enabledPlugins": {"hotline@hotline": true}}`)
	var out bytes.Buffer
	if err := cmdInit("", nil, dir, &out); err != nil {
		t.Fatalf("init: %v", err)
	}
	if len(*calls) != 0 {
		t.Errorf("expected no claude invocations, got %v", *calls)
	}
	if !strings.Contains(out.String(), "already enabled") {
		t.Errorf("missing already-enabled notice: %s", out.String())
	}
}

func TestInitPluginWritesEnvBlock(t *testing.T) {
	fakeClaudeRunner(t)
	dir := t.TempDir()
	writeProjectSettings(t, dir, `{"enabledPlugins": {"hotline@hotline": true}, "permissions": {"allow": ["WebFetch"]}}`)
	var out bytes.Buffer
	if err := cmdInit("work", []string{"--providers", "telegram,signal"}, dir, &out); err != nil {
		t.Fatalf("init: %v", err)
	}
	data, err := os.ReadFile(filepath.Join(dir, ".claude", "settings.json"))
	if err != nil {
		t.Fatal(err)
	}
	var root map[string]any
	if err := json.Unmarshal(data, &root); err != nil {
		t.Fatalf("settings not valid JSON: %v\n%s", err, data)
	}
	env, _ := root["env"].(map[string]any)
	if env["HOTLINE_PROVIDERS"] != "telegram,signal" {
		t.Errorf("HOTLINE_PROVIDERS = %v", env["HOTLINE_PROVIDERS"])
	}
	if env["HOTLINE_BOT"] != "work" {
		t.Errorf("HOTLINE_BOT = %v", env["HOTLINE_BOT"])
	}
	if _, ok := root["permissions"]; !ok {
		t.Error("unrelated settings key dropped")
	}
	if _, ok := root["enabledPlugins"]; !ok {
		t.Error("enabledPlugins dropped")
	}
}

func TestInitPluginMalformedSettingsErrors(t *testing.T) {
	fakeClaudeRunner(t)
	dir := t.TempDir()
	writeProjectSettings(t, dir, `{ not json`)
	var out bytes.Buffer
	err := cmdInit("", []string{"--providers", "telegram"}, dir, &out)
	if err == nil || !strings.Contains(err.Error(), "not valid JSON") {
		t.Fatalf("want clear JSON error, got %v", err)
	}
	data, _ := os.ReadFile(filepath.Join(dir, ".claude", "settings.json"))
	if string(data) != `{ not json` {
		t.Error("malformed settings file was clobbered")
	}
}

func TestChannelAllowlisted(t *testing.T) {
	fakeClaudeRunner(t)
	if channelAllowlisted() {
		t.Error("empty state should not be allowlisted")
	}

	// Cached Anthropic allowlist includes hotline.
	if err := os.WriteFile(claudeStateFile(), []byte(`{"cachedGrowthBookFeatures": {"tengu_harbor_ledger": [
		{"marketplace": "claude-plugins-official", "plugin": "telegram"},
		{"marketplace": "hotline", "plugin": "hotline"}
	]}}`), 0o644); err != nil {
		t.Fatal(err)
	}
	if !channelAllowlisted() {
		t.Error("ledger entry should allowlist hotline")
	}

	// Managed org settings replace the ledger.
	if err := os.WriteFile(managedSettingsPath, []byte(`{"allowedChannelPlugins": []}`), 0o644); err != nil {
		t.Fatal(err)
	}
	if channelAllowlisted() {
		t.Error("org allowlist without hotline should win over the ledger")
	}
	if err := os.WriteFile(managedSettingsPath, []byte(`{"allowedChannelPlugins": [{"marketplace": "hotline", "plugin": "hotline"}]}`), 0o644); err != nil {
		t.Fatal(err)
	}
	if !channelAllowlisted() {
		t.Error("org allowlist with hotline should pass")
	}
}
