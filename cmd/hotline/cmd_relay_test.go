package main

import (
	"bytes"
	"os"
	"strings"
	"testing"

	"github.com/1broseidon/hotline/internal/app"
	"github.com/1broseidon/hotline/internal/config"
)

func TestCmdRelayMintsLinkAndStartsAppProvider(t *testing.T) {
	t.Setenv("HOTLINE_STATE_DIR", t.TempDir())
	t.Setenv("HOTLINE_RENDEZVOUS_URL", "ws://127.0.0.1:9876/")
	old := runRelayProcess
	defer func() { runRelayProcess = old }()
	called := false
	runRelayProcess = func(bot string) error {
		called = true
		if bot != "" {
			t.Fatalf("bot = %q", bot)
		}
		return nil
	}
	var out, diagnostics bytes.Buffer
	if err := cmdRelay("", nil, &out, &diagnostics); err != nil {
		t.Fatal(err)
	}
	if !called {
		t.Fatal("relay provider process was not started")
	}
	if out.Len() != 0 {
		t.Fatalf("default relay wrote protocol-breaking stdout: %q", out.String())
	}
	if !strings.Contains(diagnostics.String(), "hotline://pair?") || !strings.Contains(diagnostics.String(), "relay starting") {
		t.Fatalf("diagnostics missing URI/start line: %q", diagnostics.String())
	}
	cfg, _ := config.LoadApp("")
	store, err := app.OpenRelayStore(cfg.StateDir)
	if err != nil {
		t.Fatal(err)
	}
	room, ok := store.CurrentRoom()
	if !ok || room.URL != "ws://127.0.0.1:9876" {
		t.Fatalf("room = %+v ok=%v", room, ok)
	}
}

func TestCmdRelayKeepsExistingRoomWithNoActiveDevices(t *testing.T) {
	t.Setenv("HOTLINE_STATE_DIR", t.TempDir())
	cfg, err := config.LoadApp("")
	if err != nil {
		t.Fatal(err)
	}
	if err := cfg.EnsureDirs(); err != nil {
		t.Fatal(err)
	}
	store, err := app.OpenRelayStore(cfg.StateDir)
	if err != nil {
		t.Fatal(err)
	}
	link, err := store.MintLink(defaultRendezvousURL, "pi")
	if err != nil {
		t.Fatal(err)
	}

	old := runRelayProcess
	defer func() { runRelayProcess = old }()
	runRelayProcess = func(string) error { return nil }
	var diagnostics bytes.Buffer
	if err := cmdRelay("", nil, &bytes.Buffer{}, &diagnostics); err != nil {
		t.Fatal(err)
	}
	current, ok := store.CurrentRoom()
	if !ok || current.ID != link.Room {
		t.Fatalf("room rotated on restart: got %+v ok=%v, want %s", current, ok, link.Room)
	}
	if strings.Contains(diagnostics.String(), "hotline://pair?") {
		t.Fatalf("restart printed a replacement pairing link: %q", diagnostics.String())
	}
}

func TestCmdRelayStatusRevokeAndNewLink(t *testing.T) {
	t.Setenv("HOTLINE_STATE_DIR", t.TempDir())
	cfg, err := config.LoadApp("")
	if err != nil {
		t.Fatal(err)
	}
	if err := cfg.EnsureDirs(); err != nil {
		t.Fatal(err)
	}
	store, _ := app.OpenRelayStore(cfg.StateDir)
	link, _ := store.MintLink(defaultRendezvousURL, "pi")
	if result, _, err := store.VerifyAndLink(link.Room, "dev-af31fd290542", link.Secret); err != nil || result != app.VerifyActive {
		t.Fatalf("link: result=%v err=%v", result, err)
	}

	var out bytes.Buffer
	if err := cmdRelay("", []string{"status"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatal(err)
	}
	status := out.String()
	if !strings.Contains(status, "dev-af31fd290542 (active)") || strings.Contains(status, link.Secret) || strings.Contains(status, "secret_hash") {
		t.Fatalf("unsafe/wrong status: %q", status)
	}
	out.Reset()
	if err := cmdRelay("", []string{"revoke", "dev-af31"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out.String(), "Revoked dev-af31fd290542") {
		t.Fatalf("revoke output = %q", out.String())
	}
	out.Reset()
	if err := cmdRelay("", []string{"new-link"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out.String(), "hotline://pair?") {
		t.Fatalf("new-link output = %q", out.String())
	}
}

func TestRelayRendezvousConfigPrecedence(t *testing.T) {
	dir := t.TempDir()
	envFile := dir + "/.env"
	if err := os.WriteFile(envFile, []byte("HOTLINE_RENDEZVOUS_URL=wss://from-file.example\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if got := relayRendezvous(envFile); got != "wss://from-file.example" {
		t.Fatalf("file value = %q", got)
	}
	t.Setenv("HOTLINE_RENDEZVOUS_URL", "wss://from-env.example")
	if got := relayRendezvous(envFile); got != "wss://from-env.example" {
		t.Fatalf("env value = %q", got)
	}
}
