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

func readAllowList(t *testing.T, dir string) []string {
	t.Helper()
	data, err := os.ReadFile(filepath.Join(dir, ".claude", "settings.json"))
	if err != nil {
		t.Fatal(err)
	}
	var root map[string]any
	if err := json.Unmarshal(data, &root); err != nil {
		t.Fatal(err)
	}
	perms, _ := root["permissions"].(map[string]any)
	raw, _ := perms["allow"].([]any)
	var out []string
	for _, a := range raw {
		if s, ok := a.(string); ok {
			out = append(out, s)
		}
	}
	return out
}

func TestMergeProjectSettingsAllow(t *testing.T) {
	// Fresh project: all safe tools get added.
	dir := t.TempDir()
	added, err := mergeProjectSettingsAllow(dir, safeAutoAllowTools)
	if err != nil {
		t.Fatalf("merge: %v", err)
	}
	if len(added) != len(safeAutoAllowTools) {
		t.Fatalf("expected all %d added, got %v", len(safeAutoAllowTools), added)
	}
	got := readAllowList(t, dir)
	for _, tool := range safeAutoAllowTools {
		if !contains(got, tool) {
			t.Errorf("missing %q in allow list %v", tool, got)
		}
	}

	// Idempotent: a second merge adds nothing and does not duplicate.
	added, err = mergeProjectSettingsAllow(dir, safeAutoAllowTools)
	if err != nil {
		t.Fatalf("merge2: %v", err)
	}
	if len(added) != 0 {
		t.Errorf("expected no new adds, got %v", added)
	}
	if again := readAllowList(t, dir); len(again) != len(got) {
		t.Errorf("allow list changed on re-merge: %v -> %v", got, again)
	}

	// Existing user permissions + other keys are preserved; existing allow kept.
	dir2 := t.TempDir()
	writeProjectSettings(t, dir2, `{"env":{"HOTLINE_BOT":"mybot"},"permissions":{"allow":["Bash(git diff:*)"],"deny":["Read(./secrets/**)"]}}`)
	if _, err := mergeProjectSettingsAllow(dir2, safeAutoAllowTools); err != nil {
		t.Fatalf("merge3: %v", err)
	}
	got2 := readAllowList(t, dir2)
	if !contains(got2, "Bash(git diff:*)") {
		t.Errorf("dropped pre-existing allow entry: %v", got2)
	}
	if !contains(got2, "Read") {
		t.Errorf("did not add safe tool alongside existing: %v", got2)
	}
	data, _ := os.ReadFile(filepath.Join(dir2, ".claude", "settings.json"))
	if !strings.Contains(string(data), `"HOTLINE_BOT"`) || !strings.Contains(string(data), `"deny"`) {
		t.Errorf("merge clobbered unrelated keys: %s", data)
	}
}

func contains(xs []string, want string) bool {
	for _, x := range xs {
		if x == want {
			return true
		}
	}
	return false
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
