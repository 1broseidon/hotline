package main

import (
	"bytes"
	"strings"
	"testing"

	"github.com/1broseidon/hotline/internal/app"
	"github.com/1broseidon/hotline/internal/config"
)

// TestCmdRelayNewLinkIsAdditive proves the default `relay new-link` mints a new
// room WITHOUT unbinding the existing device (the MD1 default-to-add UX), while
// `--rotate-all` reproduces the old destructive rotation.
func TestCmdRelayNewLinkIsAdditive(t *testing.T) {
	t.Setenv("HOTLINE_STATE_DIR", t.TempDir())
	t.Setenv("HOTLINE_RENDEZVOUS_URL", "ws://127.0.0.1:9876/")
	cfg, err := config.LoadApp("")
	if err != nil {
		t.Fatal(err)
	}
	if err := cfg.EnsureDirs(); err != nil {
		t.Fatal(err)
	}
	store, _ := app.OpenRelayStore(cfg.StateDir)
	linkA, _ := store.MintLink("ws://127.0.0.1:9876/", "pi")
	if res, _, err := store.VerifyAndLink(linkA.Room, "dev-aaaaaa111111", linkA.Secret); err != nil || res != app.VerifyActive {
		t.Fatalf("bind A: res=%v err=%v", res, err)
	}

	// Default new-link: additive. Device on A stays active; two rooms served.
	var out bytes.Buffer
	if err := cmdRelay("", []string{"new-link"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out.String(), "hotline://pair?") {
		t.Fatalf("new-link output = %q", out.String())
	}
	store2, _ := app.OpenRelayStore(cfg.StateDir)
	if d, ok := store2.Device("dev-aaaaaa111111"); !ok || d.State != app.DeviceActive {
		t.Fatalf("additive new-link unbound the existing device: %+v", d)
	}
	if n := len(store2.ServedRooms()); n != 2 {
		t.Fatalf("served rooms after additive new-link = %d, want 2", n)
	}

	// --rotate-all: destructive. Device unbinds; single served room.
	out.Reset()
	if err := cmdRelay("", []string{"new-link", "--rotate-all"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatal(err)
	}
	store3, _ := app.OpenRelayStore(cfg.StateDir)
	if d, _ := store3.Device("dev-aaaaaa111111"); d.State == app.DeviceActive {
		t.Fatal("--rotate-all did not unbind the device")
	}
	if n := len(store3.ServedRooms()); n != 1 {
		t.Fatalf("served rooms after --rotate-all = %d, want 1", n)
	}
}

// TestCmdRelayStatusShowsSlots proves `relay status` reports the per-slot line.
func TestCmdRelayStatusShowsSlots(t *testing.T) {
	t.Setenv("HOTLINE_STATE_DIR", t.TempDir())
	cfg, err := config.LoadApp("")
	if err != nil {
		t.Fatal(err)
	}
	if err := cfg.EnsureDirs(); err != nil {
		t.Fatal(err)
	}
	store, _ := app.OpenRelayStore(cfg.StateDir)
	store.MintLink("wss://relay.hotline.dev", "pi")
	store.MintLink("wss://relay.hotline.dev", "pi")

	var out bytes.Buffer
	if err := cmdRelay("", []string{"status"}, &out, &bytes.Buffer{}); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out.String(), "slots:       2/") {
		t.Fatalf("status missing slots line: %q", out.String())
	}
}
