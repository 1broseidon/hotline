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

func TestNormalizeHarness(t *testing.T) {
	cases := map[string]string{
		"":           "claude",
		"claude":     "claude",
		"CLAUDE":     "claude",
		"  pi  ":     "pi",
		"opencode":   "opencode",
		"claude-sdk": "claude-sdk",
	}
	for in, want := range cases {
		got, err := NormalizeHarness(in)
		if err != nil || got != want {
			t.Errorf("NormalizeHarness(%q) = %q, %v; want %q", in, got, err, want)
		}
	}
	if _, err := NormalizeHarness("bogus"); err == nil {
		t.Error("NormalizeHarness(bogus) = nil error, want unknown-harness")
	}
}

// TestHarnessResolvedSource covers the launch-time attribution precedence: the
// real env wins over the state .env, which wins over the built-in default. (The
// --harness flag rides in via HOTLINE_HARNESS, so it surfaces here as the env
// source; up relabels it.)
func TestHarnessResolvedSource(t *testing.T) {
	dir := withState(t)

	// Default: nothing set.
	if h, src, err := HarnessResolved(); err != nil || h != "claude" || src != HarnessSourceDefault {
		t.Fatalf("default = (%q, %q, %v); want (claude, default)", h, src, err)
	}

	// State .env only.
	if err := os.WriteFile(filepath.Join(dir, ".env"), []byte("HOTLINE_HARNESS=pi\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if h, src, err := HarnessResolved(); err != nil || h != "pi" || src != HarnessSourceDotEnv {
		t.Fatalf("dotenv = (%q, %q, %v); want (pi, from state .env)", h, src, err)
	}

	// Real env wins over the state .env.
	t.Setenv("HOTLINE_HARNESS", "opencode")
	if h, src, err := HarnessResolved(); err != nil || h != "opencode" || src != HarnessSourceEnv {
		t.Fatalf("env = (%q, %q, %v); want (opencode, from HOTLINE_HARNESS)", h, src, err)
	}

	// A present-but-empty real env shadows the .env (mergedEnv precedence), so the
	// value is the claude default and the source is attributed as such.
	t.Setenv("HOTLINE_HARNESS", "")
	if h, src, err := HarnessResolved(); err != nil || h != "claude" || src != HarnessSourceDefault {
		t.Fatalf("empty-env = (%q, %q, %v); want (claude, default)", h, src, err)
	}
}
