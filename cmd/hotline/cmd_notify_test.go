package main

import (
	"bytes"
	"strings"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/notify"
)

// notifyState points the state root at a temp dir and returns it.
func notifyState(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", dir)
	return dir
}

// addSource registers a source and returns its key.
func addSource(t *testing.T, dir, label string, cap notify.Level) string {
	t.Helper()
	s, err := notify.AddSource(notify.SourcesPath(dir), label, cap, notify.Rate{}, "", time.Now())
	if err != nil {
		t.Fatal(err)
	}
	return s.Key
}

func TestNotifyOutcomeMapping(t *testing.T) {
	cases := []struct {
		out      notify.Outcome
		wantLine string
		wantCode int
	}{
		{notify.Outcome{Status: notify.Accepted}, "accepted", 0},
		{notify.Outcome{Status: notify.Accepted, Clamped: true, ClampedTo: notify.LevelNormal}, "accepted (level clamped to normal)", 0},
		{notify.Outcome{Status: notify.Duplicate, Count: 3}, "accepted (duplicate ×3 coalesced)", 0},
		{notify.Outcome{Status: notify.Queued, QueuedUntil: "07:00"}, "queued until 07:00 (quiet hours)", 3},
		{notify.Outcome{Status: notify.RejectedUnknown}, "rejected: unknown or revoked source key", 4},
		{notify.Outcome{Status: notify.RejectedRate, Suppressed: 5, SuppressedSince: "08:59"}, "rejected: rate limited (5 suppressed since 08:59)", 4},
		{notify.Outcome{Status: notify.RejectedSpoolFull}, "rejected: spool full", 4},
	}
	for _, c := range cases {
		line, code := notifyOutcome(c.out)
		if line != c.wantLine || code != c.wantCode {
			t.Errorf("notifyOutcome(%+v) = %q,%d; want %q,%d", c.out.Status, line, code, c.wantLine, c.wantCode)
		}
	}
}

func TestCmdNotifyUsageErrors(t *testing.T) {
	notifyState(t)
	var out, errb bytes.Buffer

	// Missing --source.
	if code := cmdNotify([]string{"hello"}, strings.NewReader(""), &out, &errb); code != 2 {
		t.Errorf("missing --source: exit %d, want 2", code)
	}
	// Bad --level.
	if code := cmdNotify([]string{"--source", "k", "--level", "boom", "msg"}, strings.NewReader(""), &out, &errb); code != 2 {
		t.Errorf("bad --level: exit %d, want 2", code)
	}
	// Empty message (no positional, empty stdin).
	if code := cmdNotify([]string{"--source", "k"}, strings.NewReader("   \n"), &out, &errb); code != 2 {
		t.Errorf("empty message: exit %d, want 2", code)
	}
	// Unknown flag.
	if code := cmdNotify([]string{"--source", "k", "--bogus"}, strings.NewReader(""), &out, &errb); code != 2 {
		t.Errorf("unknown flag: exit %d, want 2", code)
	}
}

func TestCmdNotifyAcceptedFromStdin(t *testing.T) {
	dir := notifyState(t)
	key := addSource(t, dir, "backups", notify.LevelNormal)

	var out, errb bytes.Buffer
	code := cmdNotify([]string{"--source", key, "--level", "low"}, strings.NewReader("backup finished\n"), &out, &errb)
	if code != 0 {
		t.Fatalf("exit %d, want 0; stderr=%q", code, errb.String())
	}
	if strings.TrimSpace(out.String()) != "accepted" {
		t.Errorf("stdout = %q, want accepted", out.String())
	}
	sp, _ := notify.LoadSpool(notify.SpoolPath(dir))
	if len(sp.Pending) != 1 || sp.Pending[0].Message != "backup finished" || sp.Pending[0].Level != notify.LevelLow {
		t.Errorf("spool entry wrong: %+v", sp.Pending)
	}
}

func TestCmdNotifyPositionalWinsOverStdin(t *testing.T) {
	dir := notifyState(t)
	key := addSource(t, dir, "s", notify.LevelNormal)

	var out, errb bytes.Buffer
	code := cmdNotify([]string{"--source", key, "POSITIONAL"}, strings.NewReader("PIPED"), &out, &errb)
	if code != 0 {
		t.Fatalf("exit %d, want 0", code)
	}
	sp, _ := notify.LoadSpool(notify.SpoolPath(dir))
	if len(sp.Pending) != 1 || sp.Pending[0].Message != "POSITIONAL" {
		t.Errorf("positional should win over stdin, got %+v", sp.Pending)
	}
}

func TestCmdNotifyDoubleDashEndsFlags(t *testing.T) {
	dir := notifyState(t)
	key := addSource(t, dir, "s", notify.LevelNormal)

	var out, errb bytes.Buffer
	code := cmdNotify([]string{"--source", key, "--", "--looks-like-a-flag"}, strings.NewReader(""), &out, &errb)
	if code != 0 {
		t.Fatalf("exit %d, want 0; stderr=%q", code, errb.String())
	}
	sp, _ := notify.LoadSpool(notify.SpoolPath(dir))
	if len(sp.Pending) != 1 || sp.Pending[0].Message != "--looks-like-a-flag" {
		t.Errorf("bare -- should end flags, treating rest as positional; got %+v", sp.Pending)
	}
}

func TestCmdNotifyUnknownKey(t *testing.T) {
	notifyState(t)
	var out, errb bytes.Buffer
	code := cmdNotify([]string{"--source", "9f2c6a1e-dead-beef-cafe-2d5b8e1f4a6c", "hi"}, strings.NewReader(""), &out, &errb)
	if code != 4 {
		t.Fatalf("unknown key: exit %d, want 4", code)
	}
	if !strings.Contains(out.String(), "unknown or revoked") {
		t.Errorf("stdout = %q", out.String())
	}
}

func TestCmdNotifyList(t *testing.T) {
	dir := notifyState(t)
	key := addSource(t, dir, "s", notify.LevelNormal)
	var out, errb bytes.Buffer
	if code := cmdNotify([]string{"--source", key, "an event"}, strings.NewReader(""), &out, &errb); code != 0 {
		t.Fatalf("enqueue exit %d", code)
	}
	out.Reset()
	if code := cmdNotify([]string{"list"}, strings.NewReader(""), &out, &errb); code != 0 {
		t.Fatalf("list exit %d, want 0", code)
	}
	if !strings.Contains(out.String(), "1 pending") || !strings.Contains(out.String(), "s") {
		t.Errorf("list output = %q", out.String())
	}
}
