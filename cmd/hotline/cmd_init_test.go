package main

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func readMCP(t *testing.T, dir string) map[string]any {
	t.Helper()
	data, err := os.ReadFile(filepath.Join(dir, ".mcp.json"))
	if err != nil {
		t.Fatal(err)
	}
	var m map[string]any
	if err := json.Unmarshal(data, &m); err != nil {
		t.Fatalf("output not valid JSON: %v\n%s", err, data)
	}
	return m
}

func hotlineEntry(t *testing.T, m map[string]any, name string) map[string]any {
	t.Helper()
	servers, ok := m["mcpServers"].(map[string]any)
	if !ok {
		t.Fatalf("no mcpServers in %v", m)
	}
	entry, ok := servers[name].(map[string]any)
	if !ok {
		t.Fatalf("no %s entry in %v", name, servers)
	}
	return entry
}

func TestInitCreatesFresh(t *testing.T) {
	dir := t.TempDir()
	var out bytes.Buffer
	if err := cmdInit("", []string{"--mcp-json"}, dir, &out); err != nil {
		t.Fatalf("init: %v", err)
	}
	entry := hotlineEntry(t, readMCP(t, dir), "hotline")
	if entry["command"] != "hotline" {
		t.Errorf("command = %v", entry["command"])
	}
	args, _ := entry["args"].([]any)
	if len(args) != 1 || args[0] != "run" {
		t.Errorf("args = %v, want [run]", args)
	}
	if !strings.Contains(out.String(), "hotline start") {
		t.Error("missing next-step hint")
	}
}

func TestInitPreservesOtherServersAndKeys(t *testing.T) {
	dir := t.TempDir()
	existing := `{
  "mcpServers": {
    "other": {"command": "other-bin", "args": ["--x"]}
  },
  "unrelatedTopLevel": {"keep": true}
}`
	if err := os.WriteFile(filepath.Join(dir, ".mcp.json"), []byte(existing), 0o644); err != nil {
		t.Fatal(err)
	}
	var out bytes.Buffer
	if err := cmdInit("", []string{"--mcp-json", "--providers", "telegram,signal"}, dir, &out); err != nil {
		t.Fatalf("init: %v", err)
	}
	m := readMCP(t, dir)
	if _, ok := m["unrelatedTopLevel"]; !ok {
		t.Error("unrelated top-level key dropped")
	}
	servers := m["mcpServers"].(map[string]any)
	if _, ok := servers["other"]; !ok {
		t.Error("other server dropped")
	}
	entry := hotlineEntry(t, m, "hotline")
	env, _ := entry["env"].(map[string]any)
	if env["HOTLINE_PROVIDERS"] != "telegram,signal" {
		t.Errorf("env = %v", env)
	}
}

func TestInitUpdatesExistingHotlineEntry(t *testing.T) {
	dir := t.TempDir()
	existing := `{
  "mcpServers": {
    "my-channel": {"command": "hotline", "args": ["run", "--bot", "old"], "env": {"CUSTOM": "keep"}}
  }
}`
	if err := os.WriteFile(filepath.Join(dir, ".mcp.json"), []byte(existing), 0o644); err != nil {
		t.Fatal(err)
	}
	var out bytes.Buffer
	if err := cmdInit("work", []string{"--mcp-json"}, dir, &out); err != nil {
		t.Fatalf("init: %v", err)
	}
	m := readMCP(t, dir)
	servers := m["mcpServers"].(map[string]any)
	if len(servers) != 1 {
		t.Errorf("expected the existing entry updated in place, got %v", servers)
	}
	entry := hotlineEntry(t, m, "my-channel")
	args, _ := entry["args"].([]any)
	want := []any{"run", "--bot", "work"}
	if len(args) != 3 || args[0] != want[0] || args[1] != want[1] || args[2] != want[2] {
		t.Errorf("args = %v, want %v", args, want)
	}
	env, _ := entry["env"].(map[string]any)
	if env["CUSTOM"] != "keep" {
		t.Errorf("custom env key dropped: %v", env)
	}
}

func TestInitMalformedJSONErrorsWithoutClobber(t *testing.T) {
	dir := t.TempDir()
	bad := `{ not json`
	path := filepath.Join(dir, ".mcp.json")
	if err := os.WriteFile(path, []byte(bad), 0o644); err != nil {
		t.Fatal(err)
	}
	var out bytes.Buffer
	err := cmdInit("", []string{"--mcp-json"}, dir, &out)
	if err == nil || !strings.Contains(err.Error(), "not valid JSON") {
		t.Fatalf("want clear JSON error, got %v", err)
	}
	data, _ := os.ReadFile(path)
	if string(data) != bad {
		t.Error("malformed file was clobbered")
	}
}

func TestInitVoiceWritesOnceNeverOverwrites(t *testing.T) {
	dir := t.TempDir()
	var out bytes.Buffer
	if err := cmdInit("", []string{"--mcp-json", "--voice"}, dir, &out); err != nil {
		t.Fatalf("init: %v", err)
	}
	voicePath := filepath.Join(dir, "HOTLINE.md")
	if _, err := os.Stat(voicePath); err != nil {
		t.Fatal("HOTLINE.md not written")
	}
	custom := "my custom voice\n"
	if err := os.WriteFile(voicePath, []byte(custom), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := cmdInit("", []string{"--mcp-json", "--voice"}, dir, &out); err != nil {
		t.Fatalf("second init: %v", err)
	}
	data, _ := os.ReadFile(voicePath)
	if string(data) != custom {
		t.Error("existing HOTLINE.md was overwritten")
	}
}
