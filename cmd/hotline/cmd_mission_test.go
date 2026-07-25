package main

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// missionEnv points the CLI at a fresh state dir so config.MissionControl
// resolves <dir>/mc, and clears the MC envs a live shell might carry.
func missionEnv(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", dir)
	t.Setenv("HOTLINE_PROVIDERS", "")
	t.Setenv("HOTLINE_MC_DIR", "")
	t.Setenv("HOTLINE_MISSION_CONTROL", "")
	t.Setenv("HOTLINE_MC_INDEX_BUDGET", "")
	t.Setenv("HOTLINE_MC_CONTEXT_CAP", "")
	return dir
}

func runMission(t *testing.T, args ...string) (int, string, string) {
	t.Helper()
	var out, errb bytes.Buffer
	code := cmdMission("", args, &out, &errb)
	return code, out.String(), errb.String()
}

func TestCmdMissionRoundTrip(t *testing.T) {
	dir := missionEnv(t)

	// update creates a thread and syncs the index.
	if code, out, errb := runMission(t, "update", "--thread", "relay-cors",
		"--summary", "CORS fix for web redeem", "--next", "verify header on prod"); code != exitAccepted {
		t.Fatalf("update = %d err=%s", code, errb)
	} else if !strings.Contains(out, "relay-cors") {
		t.Errorf("update confirmation missing slug: %s", out)
	}

	// note appends to the thread log.
	if code, _, errb := runMission(t, "note", "--thread", "relay-cors", "--text", "patched relay handler"); code != exitAccepted {
		t.Fatalf("note(thread) = %d err=%s", code, errb)
	}
	// note to standing notes (no thread).
	if code, _, errb := runMission(t, "note", "--text", "deploys go through make ship"); code != exitAccepted {
		t.Fatalf("note(standing) = %d err=%s", code, errb)
	}

	// handoff with a CLI-only trigger value.
	if code, _, errb := runMission(t, "handoff", "--state", "half wired", "--next", "wire redial", "--trigger", "boundary"); code != exitAccepted {
		t.Fatalf("handoff = %d err=%s", code, errb)
	}
	ho, err := os.ReadFile(filepath.Join(dir, "mc", "handoff.md"))
	if err != nil {
		t.Fatalf("handoff.md: %v", err)
	}
	if !strings.Contains(string(ho), "trigger: boundary") {
		t.Errorf("handoff should record the CLI trigger, got:\n%s", ho)
	}

	// show <slug> prints the thread file.
	if code, out, errb := runMission(t, "show", "relay-cors"); code != exitAccepted {
		t.Fatalf("show slug = %d err=%s", code, errb)
	} else if !strings.Contains(out, "patched relay handler") || !strings.Contains(out, "slug: relay-cors") {
		t.Errorf("show slug missing content: %s", out)
	}

	// show (no arg) prints the index and the handoff.
	if code, out, _ := runMission(t, "show"); code != exitAccepted {
		t.Fatalf("show = %d", code)
	} else if !strings.Contains(out, "relay-cors") || !strings.Contains(out, "handoff.md") {
		t.Errorf("show index missing thread/handoff: %s", out)
	}

	// archive closes it.
	if code, _, errb := runMission(t, "archive", "--thread", "relay-cors", "--outcome", "shipped"); code != exitAccepted {
		t.Fatalf("archive = %d err=%s", code, errb)
	}
	if _, err := os.Stat(filepath.Join(dir, "mc", "threads", "relay-cors.md")); !os.IsNotExist(err) {
		t.Error("archived thread should leave threads/")
	}
	if _, err := os.Stat(filepath.Join(dir, "mc", "archive", "relay-cors.md")); err != nil {
		t.Errorf("archived thread should land in archive/: %v", err)
	}
}

func TestCmdMissionValidation(t *testing.T) {
	missionEnv(t)
	cases := []struct {
		name string
		args []string
		want int
	}{
		{"no action", nil, exitUsage},
		{"bad action", []string{"frobnicate"}, exitUsage},
		{"unknown flag", []string{"note", "--text", "x", "--wat", "y"}, exitUsage},
		{"dangling value", []string{"note", "--text"}, exitUsage},
		{"bad trigger", []string{"handoff", "--state", "s", "--next", "n", "--trigger", "sideways"}, exitUsage},
		{"note without text", []string{"note"}, exitRejected}, // store-level require
		{"archive unknown thread", []string{"archive", "--thread", "nope", "--outcome", "x"}, exitRejected},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if code, _, _ := runMission(t, tc.args...); code != tc.want {
				t.Errorf("cmdMission(%v) = %d, want %d", tc.args, code, tc.want)
			}
		})
	}
}

func TestCmdMissionShowEmpty(t *testing.T) {
	missionEnv(t)
	// Nothing seeded yet: show must not error.
	if code, out, _ := runMission(t, "show"); code != exitAccepted {
		t.Fatalf("show empty = %d", code)
	} else if !strings.Contains(out, "empty") {
		t.Errorf("show empty message: %s", out)
	}
}

func TestCmdMissionHookSessionStart(t *testing.T) {
	missionEnv(t)
	// Seed a thread so the index render is non-empty.
	runMission(t, "update", "--thread", "ping-tuning", "--summary", "idle ping", "--next", "watch battery")

	code, out, errb := runMission(t, "hook", "session-start")
	if code != exitAccepted {
		t.Fatalf("hook session-start = %d err=%s", code, errb)
	}
	var payload struct {
		HookSpecificOutput struct {
			HookEventName     string `json:"hookEventName"`
			AdditionalContext string `json:"additionalContext"`
		} `json:"hookSpecificOutput"`
	}
	if err := json.Unmarshal([]byte(out), &payload); err != nil {
		t.Fatalf("hook output not JSON: %v\n%s", err, out)
	}
	if payload.HookSpecificOutput.HookEventName != "SessionStart" {
		t.Errorf("event name = %q", payload.HookSpecificOutput.HookEventName)
	}
	ctx := payload.HookSpecificOutput.AdditionalContext
	if !strings.Contains(ctx, "ping-tuning") || !strings.Contains(ctx, "last handoff") {
		t.Errorf("session-start context missing index/wake-up: %s", ctx)
	}
}

func TestCmdMissionHookPreCompact(t *testing.T) {
	dir := missionEnv(t)
	code, out, _ := runMission(t, "hook", "pre-compact")
	if code != exitAccepted {
		t.Fatalf("hook pre-compact = %d", code)
	}
	// PreCompact's additionalContext is NOT honored by Claude Code, so the hook
	// must emit no JSON side effects — nothing for CC to misparse.
	if strings.Contains(out, "hookSpecificOutput") || strings.Contains(out, "additionalContext") {
		t.Errorf("pre-compact must not emit hook JSON (CC drops it), got: %s", out)
	}
	// Instead it writes a mechanical pre-compact handoff to the store, which the
	// next SessionStart (source "compact") re-injects.
	ho, err := os.ReadFile(filepath.Join(dir, "mc", "handoff.md"))
	if err != nil {
		t.Fatalf("pre-compact should write handoff.md: %v", err)
	}
	if !strings.Contains(string(ho), "trigger: pre-compact") {
		t.Errorf("handoff should record trigger: pre-compact, got:\n%s", ho)
	}
	if !strings.Contains(string(ho), "compaction imminent") {
		t.Errorf("handoff should carry the compaction-imminent state, got:\n%s", ho)
	}
}

// TestCmdMissionHookSessionStartRidesHandoff pins that after a pre-compact
// handoff is written, the SessionStart hook injects it prominently (spec §4:
// SessionStart's additionalContext IS honored and fires after compaction).
func TestCmdMissionHookSessionStartRidesHandoff(t *testing.T) {
	missionEnv(t)
	runMission(t, "hook", "pre-compact")
	_, out, _ := runMission(t, "hook", "session-start")
	if !strings.Contains(out, "compaction imminent") {
		t.Errorf("session-start should ride the latest handoff excerpt, got: %s", out)
	}
}

// TestCmdMissionHookDisabledNoOp pins P2-A: with HOTLINE_MISSION_CONTROL=0 the
// Claude Code hook subcommands become clean no-ops — exit 0, empty output, and
// no writes to the store (the opt-out must not be silently defeated while the
// hooks stay installed in settings.json).
func TestCmdMissionHookDisabledNoOp(t *testing.T) {
	dir := missionEnv(t)
	t.Setenv("HOTLINE_MISSION_CONTROL", "0")

	// pre-compact must not write a handoff.
	if code, out, errb := runMission(t, "hook", "pre-compact"); code != exitAccepted {
		t.Fatalf("hook pre-compact (disabled) = %d err=%s", code, errb)
	} else if strings.TrimSpace(out) != "" {
		t.Errorf("disabled pre-compact should emit nothing, got: %s", out)
	}
	if _, err := os.Stat(filepath.Join(dir, "mc", "handoff.md")); !os.IsNotExist(err) {
		t.Errorf("disabled pre-compact must not write handoff.md (err=%v)", err)
	}

	// session-start must inject nothing.
	if code, out, errb := runMission(t, "hook", "session-start"); code != exitAccepted {
		t.Fatalf("hook session-start (disabled) = %d err=%s", code, errb)
	} else if strings.TrimSpace(out) != "" {
		t.Errorf("disabled session-start should emit nothing, got: %s", out)
	}
	// The mc dir must not have been created by either hook.
	if _, err := os.Stat(filepath.Join(dir, "mc")); !os.IsNotExist(err) {
		t.Errorf("disabled hooks must not create the mc dir (err=%v)", err)
	}
}

// TestCmdMissionOperatorVerbsIgnoreDisabled pins that the operator/automation
// write verbs keep working with HOTLINE_MISSION_CONTROL=0 — only the CC hooks
// gate on Enabled (spec: the operator can always read/write the files directly).
func TestCmdMissionOperatorVerbsIgnoreDisabled(t *testing.T) {
	dir := missionEnv(t)
	t.Setenv("HOTLINE_MISSION_CONTROL", "0")

	if code, _, errb := runMission(t, "handoff", "--state", "s", "--next", "n"); code != exitAccepted {
		t.Fatalf("handoff (disabled) = %d err=%s", code, errb)
	}
	if _, err := os.Stat(filepath.Join(dir, "mc", "handoff.md")); err != nil {
		t.Errorf("operator handoff must write even when MC is disabled: %v", err)
	}
}

func TestCmdMissionHookUsage(t *testing.T) {
	missionEnv(t)
	if code, _, _ := runMission(t, "hook"); code != exitUsage {
		t.Errorf("hook with no event should be usage")
	}
	if code, _, _ := runMission(t, "hook", "bogus"); code != exitUsage {
		t.Errorf("hook with bad event should be usage")
	}
}
