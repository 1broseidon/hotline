package config

import (
	"os"
	"path/filepath"
	"testing"
)

// fakeHome points every state path at a temp dir: the legacy dir resolves
// under HOME/.claude/channels/tele-go and the new default under
// HOME/.config/hotline. No test here ever touches the real home.
func fakeHome(t *testing.T) (home, oldDir, newDir string) {
	t.Helper()
	home = t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("XDG_CONFIG_HOME", "")
	t.Setenv("HOTLINE_STATE_DIR", "")
	t.Setenv("TELE_GO_STATE_DIR", "")
	t.Setenv("TELEGRAM_STATE_DIR", "")
	return home, filepath.Join(home, ".claude", "channels", "tele-go"), filepath.Join(home, ".config", "hotline")
}

func seedOldState(t *testing.T, oldDir string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Join(oldDir, "inbox"), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(oldDir, ".env"), []byte("TELEGRAM_BOT_TOKEN=tok\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(oldDir, "inbox", "p.json"), []byte("{}"), 0o600); err != nil {
		t.Fatal(err)
	}
}

func TestStateDirEnvOverrideBeatsMigration(t *testing.T) {
	_, oldDir, newDir := fakeHome(t)
	seedOldState(t, oldDir)
	t.Setenv("HOTLINE_STATE_DIR", "/explicit")
	got, err := resolveStateDir()
	if err != nil {
		t.Fatal(err)
	}
	if got != "/explicit" {
		t.Fatalf("state dir = %q, want /explicit", got)
	}
	if _, err := os.Stat(newDir); !os.IsNotExist(err) {
		t.Fatalf("env override must not trigger migration; new dir stat err = %v", err)
	}
}

func TestMigrateHappyPath(t *testing.T) {
	_, oldDir, newDir := fakeHome(t)
	seedOldState(t, oldDir)

	got, err := resolveStateDir()
	if err != nil {
		t.Fatal(err)
	}
	if got != newDir {
		t.Fatalf("state dir = %q, want %q", got, newDir)
	}

	// Copied content.
	b, err := os.ReadFile(filepath.Join(newDir, ".env"))
	if err != nil || string(b) != "TELEGRAM_BOT_TOKEN=tok\n" {
		t.Fatalf(".env copy = %q, %v", b, err)
	}
	if _, err := os.Stat(filepath.Join(newDir, "inbox", "p.json")); err != nil {
		t.Fatalf("nested file not copied: %v", err)
	}

	// Perms preserved.
	if fi, _ := os.Stat(newDir); fi.Mode().Perm() != 0o700 {
		t.Errorf("new dir perms = %o, want 0700", fi.Mode().Perm())
	}
	if fi, _ := os.Stat(filepath.Join(newDir, "inbox")); fi.Mode().Perm() != 0o700 {
		t.Errorf("inbox perms = %o, want 0700", fi.Mode().Perm())
	}
	if fi, _ := os.Stat(filepath.Join(newDir, ".env")); fi.Mode().Perm() != 0o600 {
		t.Errorf(".env perms = %o, want 0600", fi.Mode().Perm())
	}

	// Old dir left in place, intact.
	if _, err := os.Stat(filepath.Join(oldDir, ".env")); err != nil {
		t.Fatalf("old dir must be left in place: %v", err)
	}

	// No staging leftovers.
	if _, err := os.Stat(newDir + ".migrating"); !os.IsNotExist(err) {
		t.Errorf("staging dir left behind: %v", err)
	}
}

func TestMigrateBothExistNoClobber(t *testing.T) {
	_, oldDir, newDir := fakeHome(t)
	seedOldState(t, oldDir)
	if err := os.MkdirAll(newDir, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(newDir, ".env"), []byte("NEW=1\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	got, err := resolveStateDir()
	if err != nil {
		t.Fatal(err)
	}
	if got != newDir {
		t.Fatalf("state dir = %q, want %q", got, newDir)
	}
	b, err := os.ReadFile(filepath.Join(newDir, ".env"))
	if err != nil || string(b) != "NEW=1\n" {
		t.Fatalf("existing new state clobbered: %q, %v", b, err)
	}
	if _, err := os.Stat(filepath.Join(newDir, "inbox")); !os.IsNotExist(err) {
		t.Fatalf("re-copy into existing new dir: %v", err)
	}
}

func TestMigrateMissingOldNoOp(t *testing.T) {
	_, _, newDir := fakeHome(t)
	got, err := resolveStateDir()
	if err != nil {
		t.Fatal(err)
	}
	if got != newDir {
		t.Fatalf("state dir = %q, want %q", got, newDir)
	}
	if _, err := os.Stat(newDir); !os.IsNotExist(err) {
		t.Fatalf("no-op migration must not create the new dir: %v", err)
	}
}
