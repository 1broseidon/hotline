package main

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func readSettings(t *testing.T, dir string) map[string]any {
	t.Helper()
	data, err := os.ReadFile(filepath.Join(dir, ".claude", "settings.json"))
	if err != nil {
		t.Fatalf("settings.json: %v", err)
	}
	var m map[string]any
	if err := json.Unmarshal(data, &m); err != nil {
		t.Fatalf("settings.json not JSON: %v\n%s", err, data)
	}
	return m
}

// hookCommandsFor returns every command string installed under a hook event.
func hookCommandsFor(t *testing.T, root map[string]any, event string) []string {
	t.Helper()
	hooks, _ := root["hooks"].(map[string]any)
	matchers, _ := hooks[event].([]any)
	var out []string
	for _, m := range matchers {
		mm, _ := m.(map[string]any)
		inner, _ := mm["hooks"].([]any)
		for _, h := range inner {
			hm, _ := h.(map[string]any)
			if cmd, ok := hm["command"].(string); ok {
				out = append(out, cmd)
			}
		}
	}
	return out
}

func TestMergeProjectSettingsHooksGolden(t *testing.T) {
	dir := t.TempDir()
	added, err := mergeProjectSettingsHooks(dir)
	if err != nil {
		t.Fatal(err)
	}
	if len(added) != 2 || added[0] != "SessionStart" || added[1] != "PreCompact" {
		t.Fatalf("added = %v, want [SessionStart PreCompact]", added)
	}
	root := readSettings(t, dir)
	if got := hookCommandsFor(t, root, "SessionStart"); len(got) != 1 || got[0] != "hotline mission hook session-start" {
		t.Errorf("SessionStart commands = %v", got)
	}
	if got := hookCommandsFor(t, root, "PreCompact"); len(got) != 1 || got[0] != "hotline mission hook pre-compact" {
		t.Errorf("PreCompact commands = %v", got)
	}
}

func TestMergeProjectSettingsHooksIdempotent(t *testing.T) {
	dir := t.TempDir()
	if _, err := mergeProjectSettingsHooks(dir); err != nil {
		t.Fatal(err)
	}
	// Second run must add nothing and not duplicate.
	added, err := mergeProjectSettingsHooks(dir)
	if err != nil {
		t.Fatal(err)
	}
	if len(added) != 0 {
		t.Fatalf("second run added %v, want none", added)
	}
	root := readSettings(t, dir)
	if got := hookCommandsFor(t, root, "SessionStart"); len(got) != 1 {
		t.Errorf("SessionStart duplicated: %v", got)
	}
	if got := hookCommandsFor(t, root, "PreCompact"); len(got) != 1 {
		t.Errorf("PreCompact duplicated: %v", got)
	}
}

func TestMergeProjectSettingsHooksPreservesExisting(t *testing.T) {
	dir := t.TempDir()
	if err := os.MkdirAll(filepath.Join(dir, ".claude"), 0o755); err != nil {
		t.Fatal(err)
	}
	existing := `{
  "env": {"FOO": "bar"},
  "hooks": {
    "SessionStart": [
      {"hooks": [{"type": "command", "command": "echo hi"}]}
    ]
  }
}`
	if err := os.WriteFile(filepath.Join(dir, ".claude", "settings.json"), []byte(existing), 0o644); err != nil {
		t.Fatal(err)
	}
	added, err := mergeProjectSettingsHooks(dir)
	if err != nil {
		t.Fatal(err)
	}
	// SessionStart already exists (but without our command) → our command is
	// appended alongside; PreCompact is new.
	if len(added) != 2 {
		t.Fatalf("added = %v", added)
	}
	root := readSettings(t, dir)
	if env, _ := root["env"].(map[string]any); env["FOO"] != "bar" {
		t.Error("unrelated env key not preserved")
	}
	ss := hookCommandsFor(t, root, "SessionStart")
	if len(ss) != 2 || ss[0] != "echo hi" || ss[1] != "hotline mission hook session-start" {
		t.Errorf("SessionStart commands = %v, want [echo hi, hotline mission hook session-start]", ss)
	}
}

func TestInstallMissionHooksGating(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", t.TempDir())
	t.Setenv("HOTLINE_MC_DIR", "")

	// MC off → no hooks written.
	t.Setenv("HOTLINE_MISSION_CONTROL", "0")
	var out bytes.Buffer
	if err := installMissionHooks(dir, &out); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(dir, ".claude", "settings.json")); !os.IsNotExist(err) {
		t.Error("MC off should not write settings.json")
	}

	// MC on → hooks written.
	t.Setenv("HOTLINE_MISSION_CONTROL", "1")
	if err := installMissionHooks(dir, &out); err != nil {
		t.Fatal(err)
	}
	root := readSettings(t, dir)
	if len(hookCommandsFor(t, root, "PreCompact")) != 1 {
		t.Error("MC on should install the PreCompact hook")
	}
}
