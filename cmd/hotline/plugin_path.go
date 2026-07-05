package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
)

// The official plugin identity: the hotline plugin from the hotline
// marketplace (github.com/1broseidon/hotline).
const (
	marketplaceRepo = "1broseidon/hotline"
	pluginID        = "hotline@hotline"
	channelRef      = "plugin:hotline@hotline"
)

// runClaude is the seam through which init shells out to the claude CLI
// (marketplace add / plugin install). dir is the working directory — plugin
// install is project-scoped, so it matters. Swapped out in tests.
var runClaude = func(dir string, args ...string) (string, error) {
	cmd := exec.Command("claude", args...)
	cmd.Dir = dir
	out, err := cmd.CombinedOutput()
	return string(out), err
}

// managedSettingsPath is where Claude Code reads org-managed policy settings
// on Linux; overridable in tests.
var managedSettingsPath = "/etc/claude-code/managed-settings.json"

// claudeStateFile returns ~/.claude.json, Claude Code's local state file,
// which caches the remote channel-plugin allowlist. Overridable in tests.
var claudeStateFile = func() string {
	home, err := os.UserHomeDir()
	if err != nil {
		return ""
	}
	return filepath.Join(home, ".claude.json")
}

// userSettingsFile returns ~/.claude/settings.json; overridable in tests.
var userSettingsFile = func() string {
	home, err := os.UserHomeDir()
	if err != nil {
		return ""
	}
	return filepath.Join(home, ".claude", "settings.json")
}

// readJSONMap parses a JSON object file. A missing file returns an empty map;
// malformed JSON is an error so callers never write back garbage.
func readJSONMap(path string) (map[string]any, error) {
	data, err := os.ReadFile(path)
	if os.IsNotExist(err) {
		return map[string]any{}, nil
	}
	if err != nil {
		return nil, err
	}
	root := map[string]any{}
	if err := json.Unmarshal(data, &root); err != nil {
		return nil, fmt.Errorf("%s exists but is not valid JSON (%v); fix or remove it, nothing was changed", path, err)
	}
	return root, nil
}

// pluginEnabledIn reports whether enabledPlugins["hotline@hotline"] is true in
// the given settings file. Read errors count as "not enabled".
func pluginEnabledIn(path string) bool {
	root, err := readJSONMap(path)
	if err != nil {
		return false
	}
	enabled, _ := root["enabledPlugins"].(map[string]any)
	v, _ := enabled[pluginID].(bool)
	return v
}

// pluginPathActive reports whether the hotline plugin is enabled for the
// project at dir: project settings, project-local settings, or user settings.
func pluginPathActive(dir string) bool {
	return pluginEnabledIn(filepath.Join(dir, ".claude", "settings.json")) ||
		pluginEnabledIn(filepath.Join(dir, ".claude", "settings.local.json")) ||
		pluginEnabledIn(userSettingsFile())
}

// writeProjectSettingsEnv merges the given keys into the env block of
// dir/.claude/settings.json, preserving every other key. Claude Code applies
// this env to the session, and plugin-spawned MCP servers inherit it — this is
// how per-project HOTLINE_PROVIDERS reaches the plugin-shipped server.
func writeProjectSettingsEnv(dir string, kv map[string]string) error {
	if len(kv) == 0 {
		return nil
	}
	path := filepath.Join(dir, ".claude", "settings.json")
	root, err := readJSONMap(path)
	if err != nil {
		return err
	}
	env, _ := root["env"].(map[string]any)
	if env == nil {
		env = map[string]any{}
	}
	for k, v := range kv {
		env[k] = v
	}
	root["env"] = env
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetIndent("", "  ")
	if err := enc.Encode(root); err != nil {
		return err
	}
	return os.WriteFile(path, buf.Bytes(), 0o644)
}

// safeAutoAllowTools are read-only Claude Code tools that hotline pre-approves on
// init so a remote user — who can't reach the terminal to approve — isn't buzzed
// for routine navigation. Anything that writes or executes (Edit, Write, Bash, …)
// still prompts, so the permission gate keeps its value where it matters.
var safeAutoAllowTools = []string{"Read", "Grep", "Glob", "LS", "NotebookRead", "TodoWrite"}

// mergeProjectSettingsAllow adds tools to permissions.allow in .claude/settings.json
// additively: every existing allow entry and every other key is preserved, and a
// tool already present is never duplicated. It returns the tools it actually added
// (nil if all were already allowed) so the caller can tell the user, and never
// clobbers a malformed file (readJSONMap surfaces that as an error).
func mergeProjectSettingsAllow(dir string, tools []string) ([]string, error) {
	path := filepath.Join(dir, ".claude", "settings.json")
	root, err := readJSONMap(path)
	if err != nil {
		return nil, err
	}
	perms, _ := root["permissions"].(map[string]any)
	if perms == nil {
		perms = map[string]any{}
	}
	rawAllow, _ := perms["allow"].([]any)
	have := make(map[string]bool, len(rawAllow))
	allow := make([]any, 0, len(rawAllow)+len(tools))
	for _, a := range rawAllow {
		allow = append(allow, a)
		if s, ok := a.(string); ok {
			have[s] = true
		}
	}
	var added []string
	for _, tool := range tools {
		if have[tool] {
			continue
		}
		allow = append(allow, tool)
		added = append(added, tool)
	}
	if len(added) == 0 {
		return nil, nil
	}
	perms["allow"] = allow
	root["permissions"] = perms
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return nil, err
	}
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetIndent("", "  ")
	if err := enc.Encode(root); err != nil {
		return nil, err
	}
	if err := os.WriteFile(path, buf.Bytes(), 0o644); err != nil {
		return nil, err
	}
	return added, nil
}

// channelAllowlisted reports whether hotline@hotline is on Claude Code's
// approved channel-plugin allowlist, in which case the non-dangerous
// `--channels plugin:hotline@hotline` form registers the channel. Two
// sources, mirroring the CLI's own gate: an org's allowedChannelPlugins in
// managed settings, else the Anthropic allowlist cached in ~/.claude.json
// (cachedGrowthBookFeatures.tengu_harbor_ledger). Until hotline is approved,
// only the dev-channel flag registers the channel.
func channelAllowlisted() bool {
	if managed, err := readJSONMap(managedSettingsPath); err == nil {
		if list, ok := managed["allowedChannelPlugins"].([]any); ok {
			return allowlistHasHotline(list)
		}
	}
	state, err := readJSONMap(claudeStateFile())
	if err != nil {
		return false
	}
	features, _ := state["cachedGrowthBookFeatures"].(map[string]any)
	list, _ := features["tengu_harbor_ledger"].([]any)
	return allowlistHasHotline(list)
}

func allowlistHasHotline(list []any) bool {
	for _, e := range list {
		m, ok := e.(map[string]any)
		if !ok {
			continue
		}
		if m["plugin"] == "hotline" && m["marketplace"] == "hotline" {
			return true
		}
	}
	return false
}
