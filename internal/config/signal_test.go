package config

import (
	"path/filepath"
	"testing"
)

func TestLoadSignalDefaultInstance(t *testing.T) {
	base := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", base)
	t.Setenv("SIGNAL_ACCOUNT", "+15550001111")

	c, err := LoadSignal("")
	if err != nil {
		t.Fatal(err)
	}
	if c.StateDir != filepath.Join(base, "signal") {
		t.Fatalf("state dir %q", c.StateDir)
	}
	if c.SignalAccount != "+15550001111" {
		t.Fatalf("account %q", c.SignalAccount)
	}
	if c.SignalDaemonURL != DefaultSignalDaemonURL {
		t.Fatalf("daemon url %q, want default", c.SignalDaemonURL)
	}
}

func TestLoadSignalNamedInstance(t *testing.T) {
	base := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", base)
	t.Setenv("SIGNAL_ACCOUNT", "+15550001111")
	t.Setenv("SIGNAL_ACCOUNT_WORK", "+15550002222")
	t.Setenv("SIGNAL_DAEMON_URL_WORK", "http://127.0.0.1:9090/")

	c, err := LoadSignal("work")
	if err != nil {
		t.Fatal(err)
	}
	if c.StateDir != filepath.Join(base, "signal", "instances", "work") {
		t.Fatalf("state dir %q", c.StateDir)
	}
	if c.SignalAccount != "+15550002222" {
		t.Fatalf("account %q", c.SignalAccount)
	}
	if c.SignalDaemonURL != "http://127.0.0.1:9090" {
		t.Fatalf("daemon url %q, want trailing slash trimmed", c.SignalDaemonURL)
	}
}

func TestLoadSignalRejectsBadInstance(t *testing.T) {
	if _, err := LoadSignal("../evil"); err == nil {
		t.Fatal("bad instance accepted")
	}
}
