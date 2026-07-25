package main

import (
	"bytes"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/1broseidon/hotline/internal/supervise"
)

// writeSettingsEnv writes a .claude/<name> with the given env block, the layout
// `hotline init` uses to record a plugin-launched box's HOTLINE_* identity.
func writeSettingsEnv(t *testing.T, dir, name string, env map[string]string) string {
	t.Helper()
	var pairs []string
	for k, v := range env {
		pairs = append(pairs, fmt.Sprintf("%q: %q", k, v))
	}
	body := fmt.Sprintf(`{"env": {%s}}`, strings.Join(pairs, ", "))
	claudeDir := filepath.Join(dir, ".claude")
	if err := os.MkdirAll(claudeDir, 0o755); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(claudeDir, name)
	if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
	return path
}

// cleanShell strips every ambient box selector and points the default state root
// at a hermetic temp dir, so a resolved-but-unset box lands on a known default.
func cleanShell(t *testing.T) string {
	t.Helper()
	for _, k := range []string{
		"HOTLINE_STATE_DIR", "TELE_GO_STATE_DIR", "TELEGRAM_STATE_DIR",
		"HOTLINE_PROVIDERS", "HOTLINE_BOT", "TELE_GO_BOT",
	} {
		t.Setenv(k, "")
	}
	xdg := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", xdg)
	t.Setenv("HOME", t.TempDir()) // legacy-state migration stays a no-op
	return filepath.Join(xdg, "hotline")
}

func TestProjectBoxDeclaration(t *testing.T) {
	tests := []struct {
		name         string
		setup        func(t *testing.T, dir string)
		queryNested  bool // query from dir/sub/pkg to exercise the parent walk
		wantFound    bool
		wantStateDir string // relative to dir, "" for none
		wantSource   string // basename of the config file, "" when not found
	}{
		{
			name: "mcp.json declares state dir",
			setup: func(t *testing.T, dir string) {
				writeMCPJSON(t, dir, map[string]string{"HOTLINE_STATE_DIR": filepath.Join(dir, "box")})
			},
			wantFound:    true,
			wantStateDir: "box",
			wantSource:   ".mcp.json",
		},
		{
			name: "plugin settings declares state dir",
			setup: func(t *testing.T, dir string) {
				writeSettingsEnv(t, dir, "settings.json", map[string]string{
					"HOTLINE_PROVIDERS": "app",
					"HOTLINE_STATE_DIR": filepath.Join(dir, "box"),
				})
			},
			wantFound:    true,
			wantStateDir: "box",
			wantSource:   "settings.json",
		},
		{
			name: "settings.local.json is consulted too",
			setup: func(t *testing.T, dir string) {
				writeSettingsEnv(t, dir, "settings.local.json", map[string]string{
					"HOTLINE_STATE_DIR": filepath.Join(dir, "box"),
				})
			},
			wantFound:    true,
			wantStateDir: "box",
			wantSource:   "settings.local.json",
		},
		{
			name: "mcp.json wins over plugin settings",
			setup: func(t *testing.T, dir string) {
				writeMCPJSON(t, dir, map[string]string{"HOTLINE_STATE_DIR": filepath.Join(dir, "mcpbox")})
				writeSettingsEnv(t, dir, "settings.json", map[string]string{"HOTLINE_STATE_DIR": filepath.Join(dir, "pluginbox")})
			},
			wantFound:    true,
			wantStateDir: "mcpbox",
			wantSource:   ".mcp.json",
		},
		{
			name: "non-hotline settings is ignored",
			setup: func(t *testing.T, dir string) {
				writeSettingsEnv(t, dir, "settings.json", map[string]string{"CLAUDE_CODE_AUTO_COMPACT_WINDOW": "500000"})
			},
			wantFound: false,
		},
		{
			name:      "no project config",
			setup:     func(t *testing.T, dir string) {},
			wantFound: false,
		},
		{
			name: "parent-directory config found from a subdir",
			setup: func(t *testing.T, dir string) {
				writeSettingsEnv(t, dir, "settings.json", map[string]string{"HOTLINE_STATE_DIR": filepath.Join(dir, "box")})
			},
			queryNested:  true,
			wantFound:    true,
			wantStateDir: "box",
			wantSource:   "settings.json",
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			dir := t.TempDir()
			tc.setup(t, dir)
			query := dir
			if tc.queryNested {
				query = filepath.Join(dir, "sub", "pkg")
				if err := os.MkdirAll(query, 0o755); err != nil {
					t.Fatal(err)
				}
			}
			decl, err := projectBoxDeclaration(query)
			if err != nil {
				t.Fatalf("projectBoxDeclaration: %v", err)
			}
			if decl.Found != tc.wantFound {
				t.Fatalf("found = %v, want %v (%+v)", decl.Found, tc.wantFound, decl)
			}
			if !tc.wantFound {
				return
			}
			wantState := ""
			if tc.wantStateDir != "" {
				wantState = filepath.Join(dir, tc.wantStateDir)
			}
			if decl.StateDir != wantState {
				t.Errorf("state dir = %q, want %q", decl.StateDir, wantState)
			}
			if filepath.Base(decl.Source) != tc.wantSource {
				t.Errorf("source = %q, want basename %q", decl.Source, tc.wantSource)
			}
		})
	}
}

func TestProjectBoxMismatch(t *testing.T) {
	t.Run("plugin box differs from default -> mismatch", func(t *testing.T) {
		defBase := cleanShell(t)
		dir := t.TempDir()
		otherBox := t.TempDir()
		writeSettingsEnv(t, dir, "settings.json", map[string]string{"HOTLINE_STATE_DIR": otherBox})

		decl, base, mismatch, err := projectBoxMismatch(dir)
		if err != nil {
			t.Fatal(err)
		}
		if !mismatch {
			t.Fatalf("mismatch = false, want true (default %s, project %s)", defBase, otherBox)
		}
		if decl.StateDir != otherBox {
			t.Errorf("declared state dir = %q, want %q", decl.StateDir, otherBox)
		}
		if absClean(base) != absClean(defBase) {
			t.Errorf("resolved base = %q, want default %q", base, defBase)
		}
	})

	t.Run("shell state dir set -> no mismatch", func(t *testing.T) {
		cleanShell(t)
		t.Setenv("HOTLINE_STATE_DIR", t.TempDir()) // operator (or adopted .mcp.json) pinned a box
		dir := t.TempDir()
		writeSettingsEnv(t, dir, "settings.json", map[string]string{"HOTLINE_STATE_DIR": t.TempDir()})

		if _, _, mismatch, err := projectBoxMismatch(dir); err != nil || mismatch {
			t.Fatalf("mismatch = %v err = %v, want false/nil when the shell pins a box", mismatch, err)
		}
	})

	t.Run("project declares the default box -> no mismatch", func(t *testing.T) {
		defBase := cleanShell(t)
		dir := t.TempDir()
		writeSettingsEnv(t, dir, "settings.json", map[string]string{"HOTLINE_STATE_DIR": defBase})

		if _, _, mismatch, err := projectBoxMismatch(dir); err != nil || mismatch {
			t.Fatalf("mismatch = %v err = %v, want false/nil when the project names the default box", mismatch, err)
		}
	})

	t.Run("no project config -> no mismatch", func(t *testing.T) {
		cleanShell(t)
		if _, _, mismatch, err := projectBoxMismatch(t.TempDir()); err != nil || mismatch {
			t.Fatalf("mismatch = %v err = %v, want false/nil with no project config", mismatch, err)
		}
	})
}

// TestDownRefusesCrossBoxFromProjectDir is the 2026-07-21 incident, reproduced:
// `hotline down` from a plugin-configured box's project dir, with that box's
// HOTLINE_STATE_DIR absent from the shell, must refuse rather than SIGTERM the
// default box.
func TestDownRefusesCrossBoxFromProjectDir(t *testing.T) {
	defBase := cleanShell(t)
	dir := t.TempDir()
	otherBox := t.TempDir()
	writeSettingsEnv(t, dir, "settings.json", map[string]string{
		"HOTLINE_PROVIDERS": "app",
		"HOTLINE_STATE_DIR": otherBox,
	})

	var out, errOut bytes.Buffer
	err := cmdDown("", false, nil, dir, &out, &errOut)
	if err == nil {
		t.Fatalf("down did not refuse; output = %q", out.String())
	}
	msg := err.Error()
	for _, want := range []string{"refusing to stop the wrong box", otherBox, defBase, "--state-dir", "--force", "HOTLINE_STATE_DIR="} {
		if !strings.Contains(msg, want) {
			t.Errorf("refusal missing %q\n%s", want, msg)
		}
	}
	if out.Len() != 0 {
		t.Errorf("refused down must not print a target announcement, got %q", out.String())
	}
}

func TestDownGuardBypasses(t *testing.T) {
	// Each escape hatch must skip the guard and fall through to normal resolution
	// (here: the target box simply isn't running, so down is a friendly no-op).
	newIncidentDir := func(t *testing.T) string {
		dir := t.TempDir()
		writeSettingsEnv(t, dir, "settings.json", map[string]string{"HOTLINE_STATE_DIR": t.TempDir()})
		return dir
	}

	t.Run("--state-dir targets a box root directly", func(t *testing.T) {
		cleanShell(t)
		dir := newIncidentDir(t)
		target := t.TempDir()
		var out, errOut bytes.Buffer
		if err := cmdDown("", false, []string{"--state-dir", target}, dir, &out, &errOut); err != nil {
			t.Fatalf("down --state-dir refused/failed: %v", err)
		}
		if !strings.Contains(out.String(), "not running") || !strings.Contains(out.String(), target) {
			t.Fatalf("output = %q, want not-running notice naming %s", out.String(), target)
		}
	})

	t.Run("--force stops the resolved box", func(t *testing.T) {
		cleanShell(t)
		dir := newIncidentDir(t)
		var out, errOut bytes.Buffer
		if err := cmdDown("", false, []string{"--force"}, dir, &out, &errOut); err != nil {
			t.Fatalf("down --force refused/failed: %v", err)
		}
		if !strings.Contains(out.String(), "not running") {
			t.Fatalf("output = %q, want not-running notice", out.String())
		}
	})

	t.Run("explicit --bot is a deliberate selection", func(t *testing.T) {
		cleanShell(t)
		dir := newIncidentDir(t)
		var out, errOut bytes.Buffer
		if err := cmdDown("Ada", true, nil, dir, &out, &errOut); err != nil {
			t.Fatalf("down --bot refused/failed: %v", err)
		}
		if !strings.Contains(out.String(), "not running") {
			t.Fatalf("output = %q, want not-running notice", out.String())
		}
	})
}

// TestDownNoProjectConfigUsesEnvBox: with no project config the pre-existing
// env-based resolution stands (a bare directory is not a cross-box hazard).
func TestDownNoProjectConfigUsesEnvBox(t *testing.T) {
	cleanShell(t)
	var out, errOut bytes.Buffer
	if err := cmdDown("", false, nil, t.TempDir(), &out, &errOut); err != nil {
		t.Fatalf("down with no project config: %v", err)
	}
	if !strings.Contains(out.String(), "not running") {
		t.Fatalf("output = %q, want not-running notice", out.String())
	}
}

func TestStatusNotesCrossBoxMismatch(t *testing.T) {
	setupTestState(t)                 // gives a real base with .env so status can load
	t.Setenv("HOTLINE_STATE_DIR", "") // simulate a shell that dropped the box's env
	t.Setenv("TELE_GO_STATE_DIR", "")
	t.Setenv("TELEGRAM_STATE_DIR", "")
	xdg := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", xdg)
	t.Setenv("HOME", t.TempDir())

	dir := t.TempDir()
	otherBox := t.TempDir()
	writeSettingsEnv(t, dir, "settings.json", map[string]string{"HOTLINE_STATE_DIR": otherBox})

	var out, errOut bytes.Buffer
	// status resolves the (empty) default box; the read itself is a plain success.
	if err := cmdStatus("", "", dir, &out, &errOut); err != nil {
		t.Fatalf("status: %v", err)
	}
	if !strings.Contains(errOut.String(), "note:") || !strings.Contains(errOut.String(), otherBox) {
		t.Fatalf("status note missing; stderr = %q", errOut.String())
	}
}

func TestAnnounceDownTarget(t *testing.T) {
	tests := []struct {
		name  string
		st    supervise.State
		wants []string
	}{
		{
			name:  "running harness",
			st:    supervise.State{PID: 4242, HarnessPID: 4250, WorkDir: "/home/op/proj", Argv: []string{"pi", "--mode", "rpc"}},
			wants: []string{"stopping box mybox", "box root:       /var/box", "launched from:  /home/op/proj", "running:        pi --mode rpc", "supervisor pid: 4242", "harness pid:    4250"},
		},
		{
			name:  "harness down in backoff",
			st:    supervise.State{PID: 99},
			wants: []string{"supervisor pid: 99", "harness pid:    (down; supervisor retrying)"},
		},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			var buf bytes.Buffer
			announceDownTarget(&buf, "mybox", "/var/box", &tc.st)
			for _, w := range tc.wants {
				if !strings.Contains(buf.String(), w) {
					t.Errorf("announcement missing %q\n%s", w, buf.String())
				}
			}
		})
	}
}
