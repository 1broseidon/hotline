package main

import (
	"bytes"
	"strings"
	"testing"

	"github.com/1broseidon/hotline/internal/notify"
)

func TestCmdSourceUsage(t *testing.T) {
	notifyState(t)
	var out bytes.Buffer
	if err := cmdSource(nil, &out); err == nil {
		t.Error("no args should error")
	}
	if err := cmdSource([]string{"bogus"}, &out); err == nil {
		t.Error("unknown subcommand should error")
	}
	if err := cmdSource([]string{"add"}, &out); err == nil {
		t.Error("add without label should error")
	}
	if err := cmdSource([]string{"revoke"}, &out); err == nil {
		t.Error("revoke without label should error")
	}
}

func TestCmdSourceAddPrintsKeyOnce(t *testing.T) {
	dir := notifyState(t)
	var out bytes.Buffer
	if err := cmdSource([]string{"add", "email-sentry", "--cap", "urgent"}, &out); err != nil {
		t.Fatalf("add: %v", err)
	}
	// The registry now holds exactly one source with an urgent cap.
	reg, _ := notify.LoadRegistry(notify.SourcesPath(dir))
	if len(reg.Sources) != 1 || reg.Sources[0].Label != "email-sentry" || reg.Sources[0].LevelCap != notify.LevelUrgent {
		t.Fatalf("registry wrong: %+v", reg.Sources)
	}
	// The minted key is printed exactly once, prominently.
	key := reg.Sources[0].Key
	if c := strings.Count(out.String(), key); c != 1 {
		t.Errorf("key printed %d times, want 1; output=%q", c, out.String())
	}

	// Duplicate label rejected.
	if err := cmdSource([]string{"add", "email-sentry"}, &out); err == nil {
		t.Error("duplicate label should error")
	}
}

func TestCmdSourceAddRateOverride(t *testing.T) {
	dir := notifyState(t)
	var out bytes.Buffer
	if err := cmdSource([]string{"add", "noisy", "--burst", "10", "--refill-mins", "2", "--chat-id", "42"}, &out); err != nil {
		t.Fatalf("add: %v", err)
	}
	reg, _ := notify.LoadRegistry(notify.SourcesPath(dir))
	s := reg.Sources[0]
	if s.Rate.Burst != 10 || s.Rate.RefillMins != 2 || s.ChatID != "42" {
		t.Errorf("rate/chat override not stored: %+v", s)
	}
	// Bad numeric flag is a usage error.
	if err := cmdSource([]string{"add", "x", "--burst", "notint"}, &out); err == nil {
		t.Error("non-integer --burst should error")
	}
}

func TestCmdSourceListAndRevoke(t *testing.T) {
	dir := notifyState(t)
	var out bytes.Buffer
	if err := cmdSource([]string{"add", "a"}, &out); err != nil {
		t.Fatal(err)
	}
	if err := cmdSource([]string{"add", "b"}, &out); err != nil {
		t.Fatal(err)
	}

	out.Reset()
	if err := cmdSource([]string{"list"}, &out); err != nil {
		t.Fatalf("list: %v", err)
	}
	if !strings.Contains(out.String(), "2 source(s)") || !strings.Contains(out.String(), "a") || !strings.Contains(out.String(), "b") {
		t.Errorf("list output = %q", out.String())
	}

	if err := cmdSource([]string{"revoke", "a"}, &out); err != nil {
		t.Fatalf("revoke: %v", err)
	}
	reg, _ := notify.LoadRegistry(notify.SourcesPath(dir))
	if len(reg.Sources) != 1 || reg.Sources[0].Label != "b" {
		t.Errorf("after revoke: %+v", reg.Sources)
	}
	if err := cmdSource([]string{"revoke", "nope"}, &out); err == nil {
		t.Error("revoking missing label should error")
	}
}
