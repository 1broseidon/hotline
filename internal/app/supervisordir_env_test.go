package app

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/supervise"
	"github.com/1broseidon/hotline/internal/transcript"
)

// TestMain scrubs HOTLINE_SUPERVISOR_DIR for the entire internal/app test
// binary. NewServer now sources the supervisor dir from cfg.SupervisorDir
// (populated only by config.LoadApp at the CLI boundary), so tests already
// cannot leak — but a supervised operator box exports HOTLINE_SUPERVISOR_DIR
// into every descendant shell, and this belt-and-braces unset guarantees that
// even a future accidental os.Getenv(supervise.EnvDir) inside this package
// can't file a real restart.request into the live supervisor dir and bounce
// the operator's session. See config.Config.SupervisorDir.
func TestMain(m *testing.M) {
	os.Unsetenv(supervise.EnvDir)
	os.Exit(m.Run())
}

// TestNewServerIgnoresAmbientSupervisorEnv proves the leak is closed: a Server
// built from a default/test config has an empty supervisorDir even when
// HOTLINE_SUPERVISOR_DIR is set in the process environment. Regression guard
// for the bug where `go test ./internal/app/` on a supervised box filed a real
// "relay room rotated" restart.request into the live supervisor dir.
func TestNewServerIgnoresAmbientSupervisorEnv(t *testing.T) {
	// Point the ambient env at a decoy dir. If NewServer still read the env
	// (the old bug), supervisorDir would pick this up.
	decoy := t.TempDir()
	t.Setenv(supervise.EnvDir, decoy)

	dir := t.TempDir()
	cfg := &config.Config{
		StateDir:       dir,
		AccessFile:     filepath.Join(dir, "access.json"),
		TranscriptFile: filepath.Join(dir, "transcript.jsonl"),
	}
	srv := NewServer(cfg, transcript.New(cfg.TranscriptFile))
	if srv.initErr != nil {
		t.Fatal(srv.initErr)
	}
	if srv.supervisorDir != "" {
		t.Fatalf("supervisorDir = %q; want empty — NewServer must read cfg.SupervisorDir, never the ambient env", srv.supervisorDir)
	}

	// Belt and braces: a config that DID carry the dir (as LoadApp would set
	// it) is honored — production behavior is unchanged.
	cfg.SupervisorDir = decoy
	srv2 := NewServer(cfg, transcript.New(cfg.TranscriptFile))
	if srv2.initErr != nil {
		t.Fatal(srv2.initErr)
	}
	if srv2.supervisorDir != decoy {
		t.Fatalf("supervisorDir = %q; want %q — NewServer must honor cfg.SupervisorDir", srv2.supervisorDir, decoy)
	}
}
