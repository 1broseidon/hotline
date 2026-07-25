package config

import (
	"os"
	"path/filepath"
	"testing"
)

// withStateDir points the state-dir resolver at a temp dir with the given .env
// contents, and restores the environment on cleanup.
func withStateDir(t *testing.T, env string) string {
	t.Helper()
	dir := t.TempDir()
	if env != "" {
		if err := os.WriteFile(filepath.Join(dir, ".env"), []byte(env), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	t.Setenv("HOTLINE_STATE_DIR", dir)
	t.Setenv("HOTLINE_PROVIDERS", "")
	t.Setenv("HOTLINE_BOT", "")
	t.Setenv("TELE_GO_BOT", "")
	return dir
}

func TestMissionControlEnabledDefaults(t *testing.T) {
	withStateDir(t, "")
	// Ensure the switch env is not inherited from the caller's shell.
	t.Setenv("HOTLINE_MISSION_CONTROL", "")

	for _, tc := range []struct {
		harness string
		want    bool
	}{
		{"claude", true}, // resolved call #3: full MC on CC — unset ⇒ ON everywhere
		{"pi", true},
		{"opencode", true},
	} {
		c, err := MissionControl(tc.harness)
		if err != nil {
			t.Fatal(err)
		}
		if c.Enabled != tc.want {
			t.Errorf("harness %s: Enabled = %v, want %v", tc.harness, c.Enabled, tc.want)
		}
	}
}

func TestMissionControlExplicitOverrides(t *testing.T) {
	withStateDir(t, "")
	t.Setenv("HOTLINE_MISSION_CONTROL", "0")
	if c, _ := MissionControl("pi"); c.Enabled {
		t.Error("=0 must disable even for pi")
	}
	t.Setenv("HOTLINE_MISSION_CONTROL", "1")
	if c, _ := MissionControl("claude"); !c.Enabled {
		t.Error("=1 must enable even for claude")
	}
}

func TestMissionControlNamedBoxAndExplicitDirOverride(t *testing.T) {
	dir := withStateDir(t, "")
	t.Setenv("HOTLINE_PROVIDERS", "telegram:Ada")
	t.Setenv("HOTLINE_BOT", "")
	t.Setenv("TELE_GO_BOT", "")
	t.Setenv("HOTLINE_MC_DIR", "")

	c, err := MissionControlForBox("pi", "")
	if err != nil {
		t.Fatal(err)
	}
	if want := filepath.Join(dir, "bots", "Ada", "mc"); c.Dir != want {
		t.Fatalf("named box MC dir = %q, want %q", c.Dir, want)
	}

	// The compatibility wrapper resolves the environment-selected box.
	t.Setenv("HOTLINE_PROVIDERS", "")
	t.Setenv("HOTLINE_BOT", "Ada")
	c, err = MissionControl("pi")
	if err != nil {
		t.Fatal(err)
	}
	if want := filepath.Join(dir, "bots", "Ada", "mc"); c.Dir != want {
		t.Fatalf("env-selected wrapper MC dir = %q, want %q", c.Dir, want)
	}

	// HOTLINE_MC_DIR remains a direct override even for a named box. Because it
	// needs no inferred default, it also keeps working independently of provider
	// path resolution, as it did before boxes existed.
	t.Setenv("HOTLINE_MC_DIR", "/custom/mc")
	t.Setenv("HOTLINE_PROVIDERS", "not/a/provider")
	c, err = MissionControlForBox("pi", "Ada")
	if err != nil {
		t.Fatal(err)
	}
	if c.Dir != "/custom/mc" {
		t.Fatalf("explicit MC dir = %q, want /custom/mc", c.Dir)
	}
}

func TestMissionControlDirOverridePrecedenceUnchanged(t *testing.T) {
	withStateDir(t, "HOTLINE_MC_DIR=/from/dotenv\n")
	// Register cleanup for any inherited value, then make the key genuinely
	// absent so mergedEnv exercises the shared .env fallback.
	t.Setenv("HOTLINE_MC_DIR", "")
	os.Unsetenv("HOTLINE_MC_DIR")

	c, err := MissionControlForBox("pi", "Ada")
	if err != nil {
		t.Fatal(err)
	}
	if c.Dir != "/from/dotenv" {
		t.Fatalf("dotenv MC dir = %q, want /from/dotenv", c.Dir)
	}

	t.Setenv("HOTLINE_MC_DIR", "/from/process")
	c, err = MissionControlForBox("pi", "Ada")
	if err != nil {
		t.Fatal(err)
	}
	if c.Dir != "/from/process" {
		t.Fatalf("process MC dir = %q, want /from/process", c.Dir)
	}
}

func TestMissionControlDirAndBudget(t *testing.T) {
	dir := withStateDir(t, "")
	t.Setenv("HOTLINE_MISSION_CONTROL", "")
	t.Setenv("HOTLINE_MC_DIR", "")
	t.Setenv("HOTLINE_MC_INDEX_BUDGET", "")
	t.Setenv("HOTLINE_MC_CONTEXT_CAP", "")

	c, err := MissionControl("pi")
	if err != nil {
		t.Fatal(err)
	}
	if c.Dir != filepath.Join(dir, "mc") {
		t.Errorf("default Dir = %s, want %s", c.Dir, filepath.Join(dir, "mc"))
	}
	if c.IndexBudget != DefaultMCIndexBudget {
		t.Errorf("default budget = %d, want %d", c.IndexBudget, DefaultMCIndexBudget)
	}
	if c.ContextCap != 0 {
		t.Errorf("default context cap = %d, want 0", c.ContextCap)
	}

	t.Setenv("HOTLINE_MC_DIR", "/custom/mc")
	t.Setenv("HOTLINE_MC_INDEX_BUDGET", "2048")
	t.Setenv("HOTLINE_MC_CONTEXT_CAP", "120000")
	c, _ = MissionControl("pi")
	if c.Dir != "/custom/mc" || c.IndexBudget != 2048 || c.ContextCap != 120000 {
		t.Errorf("overrides not applied: %+v", c)
	}

	// Invalid numerics fall back to defaults.
	t.Setenv("HOTLINE_MC_INDEX_BUDGET", "nope")
	t.Setenv("HOTLINE_MC_CONTEXT_CAP", "-5")
	c, _ = MissionControl("pi")
	if c.IndexBudget != DefaultMCIndexBudget || c.ContextCap != 0 {
		t.Errorf("invalid numerics not defaulted: %+v", c)
	}
}
