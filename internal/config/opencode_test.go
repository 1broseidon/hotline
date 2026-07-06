package config

import (
	"os"
	"path/filepath"
	"testing"
)

// withState points the config resolver at a fresh temp state dir and clears the
// OpenCode/harness env for the duration of the test.
func withState(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", dir)
	for _, k := range []string{
		"OPENCODE_SERVER_URL", "OPENCODE_SERVER_PASSWORD", "OPENCODE_SESSION", "HOTLINE_HARNESS",
	} {
		t.Setenv(k, "")
		os.Unsetenv(k)
	}
	return dir
}

func TestLoadOpenCodeDefaults(t *testing.T) {
	withState(t)
	c, err := LoadOpenCode()
	if err != nil {
		t.Fatal(err)
	}
	if c.ServerURL != DefaultOpenCodeServerURL {
		t.Fatalf("ServerURL %q, want default", c.ServerURL)
	}
	if c.Password != "" || c.Session != "" {
		t.Fatalf("unexpected password/session: %+v", c)
	}
}

func TestLoadOpenCodeFromEnv(t *testing.T) {
	withState(t)
	t.Setenv("OPENCODE_SERVER_URL", "http://box:5000/")
	t.Setenv("OPENCODE_SERVER_PASSWORD", "hunter2")
	t.Setenv("OPENCODE_SESSION", "ses_pin")
	c, err := LoadOpenCode()
	if err != nil {
		t.Fatal(err)
	}
	if c.ServerURL != "http://box:5000" { // trailing slash trimmed
		t.Fatalf("ServerURL %q", c.ServerURL)
	}
	if c.Password != "hunter2" || c.Session != "ses_pin" {
		t.Fatalf("got %+v", c)
	}
}

func TestLoadOpenCodeFromDotEnv(t *testing.T) {
	dir := withState(t)
	env := "OPENCODE_SERVER_URL=http://dotenv:9\nOPENCODE_SESSION=ses_dot\n"
	if err := os.WriteFile(filepath.Join(dir, ".env"), []byte(env), 0o600); err != nil {
		t.Fatal(err)
	}
	c, err := LoadOpenCode()
	if err != nil {
		t.Fatal(err)
	}
	if c.ServerURL != "http://dotenv:9" || c.Session != "ses_dot" {
		t.Fatalf("got %+v", c)
	}
}

func TestHarnessDefaultAndOverride(t *testing.T) {
	withState(t)
	if h, err := Harness(); err != nil || h != "claude" {
		t.Fatalf("default harness = %q, %v; want claude", h, err)
	}
	t.Setenv("HOTLINE_HARNESS", "opencode")
	if h, err := Harness(); err != nil || h != "opencode" {
		t.Fatalf("override harness = %q, %v; want opencode", h, err)
	}
	t.Setenv("HOTLINE_HARNESS", "CLAUDE") // case-insensitive
	if h, err := Harness(); err != nil || h != "claude" {
		t.Fatalf("uppercase = %q, %v; want claude", h, err)
	}
	t.Setenv("HOTLINE_HARNESS", "bogus")
	if _, err := Harness(); err == nil {
		t.Fatal("expected error for unknown harness")
	}
}
