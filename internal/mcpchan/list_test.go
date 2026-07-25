package mcpchan

import (
	"encoding/json"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/loop"
)

func TestListSchemasValidJSON(t *testing.T) {
	for name, schema := range map[string]string{
		"list_schedules": listSchedulesSchema,
		"list_loops":     listLoopsSchema,
	} {
		var v map[string]any
		if err := json.Unmarshal([]byte(schema), &v); err != nil {
			t.Fatalf("%s schema is not valid JSON: %v", name, err)
		}
		if v["type"] != "object" {
			t.Errorf("%s type = %v, want object", name, v["type"])
		}
		if _, ok := v["required"]; ok {
			t.Errorf("%s should have no required properties", name)
		}
	}
}

func TestHandleListSchedulesEmpty(t *testing.T) {
	if msg, isErr := handleListSchedules(schedPath(t)); isErr || msg != "No schedules set." {
		t.Errorf("empty list: %q isErr=%v", msg, isErr)
	}
}

func TestHandleListSchedulesPopulated(t *testing.T) {
	path := schedPath(t)
	// Two schedules created through the create handler.
	handleSchedule(ScheduleInput{Action: "create", Prompt: "morning brief\nsecond line", ChatID: "1", Repeat: "daily", TimeOfDay: "09:00"}, path, []string{"telegram"})
	handleSchedule(ScheduleInput{Action: "create", Prompt: "one-off", ChatID: "2", Repeat: "once", At: "2030-01-01T09:00"}, path, []string{"telegram"})

	msg, isErr := handleListSchedules(path)
	if isErr {
		t.Fatalf("list errored: %s", msg)
	}
	if !strings.HasPrefix(msg, "2 schedule(s):") {
		t.Errorf("header wrong: %q", msg)
	}
	// Recurrence, chat routing, and prompt appear.
	for _, want := range []string{"daily at 09:00", "chat 1", "morning brief", "one-off"} {
		if !strings.Contains(msg, want) {
			t.Errorf("output missing %q: %q", want, msg)
		}
	}
	// A freshly created schedule has never fired.
	if !strings.Contains(msg, "last never") {
		t.Errorf("unfired schedule should render 'last never': %q", msg)
	}
	// Multi-line prompt collapses to its first line.
	if strings.Contains(msg, "second line") {
		t.Errorf("prompt should collapse to first line: %q", msg)
	}
	// Active by default.
	if !strings.Contains(msg, "active") {
		t.Errorf("schedule should render active: %q", msg)
	}
}

func TestHandleListLoopsEmpty(t *testing.T) {
	stateRoot := t.TempDir()
	if msg, isErr := handleListLoops(stateRoot); isErr || msg != "No loops configured." {
		t.Errorf("empty list: %q isErr=%v", msg, isErr)
	}
}

func TestHandleListLoopsPopulated(t *testing.T) {
	stateRoot := t.TempDir()
	// The fixture explicitly exercises a pending loop, not the caller's ambient
	// no-approval posture.
	t.Setenv("HOTLINE_YOLO", "0")
	t.Setenv("HOTLINE_HARNESS", "claude")
	now := time.Now()
	// Approved, active, script-owned; recorded one run with exit 2.
	if _, err := loop.Add(loop.Path(stateRoot), loop.Loop{Label: "watcher", Every: "1m", Cmd: "true\nignored"}, now); err != nil {
		t.Fatal(err)
	}
	if err := loop.RecordRun(loop.Path(stateRoot), "watcher", now, 2, time.Second); err != nil {
		t.Fatal(err)
	}
	// Pending + paused loop that routes to notify. The approval gate (non-yolo,
	// not pre-approved) leaves it pending, unlike the legacy Add above.
	if _, err := loop.Add(loop.Path(stateRoot), loop.Loop{Label: "poller", Every: "5m", Cmd: "poll.sh", NotifyLLM: true, Source: "poller", Level: "urgent"}, now, loop.WithApprovalGate(stateRoot, false)); err != nil {
		t.Fatal(err)
	}
	if _, err := loop.SetPaused(loop.Path(stateRoot), "poller", true); err != nil {
		t.Fatal(err)
	}

	msg, isErr := handleListLoops(stateRoot)
	if isErr {
		t.Fatalf("list errored: %s", msg)
	}
	if !strings.HasPrefix(msg, "2 loop(s):") {
		t.Errorf("header wrong: %q", msg)
	}
	for _, want := range []string{
		"watcher", "approved", "active", "exit 2", "runs 1", "script-owned",
		"poller", "pending", "paused", `notify source "poller" level urgent`,
	} {
		if !strings.Contains(msg, want) {
			t.Errorf("output missing %q: %q", want, msg)
		}
	}
	// The unrun (well, poller) loop should show 'last run never'.
	if !strings.Contains(msg, "last run never") {
		t.Errorf("unrun loop should render 'last run never': %q", msg)
	}
	// Multi-line cmd collapses to its first line.
	if strings.Contains(msg, "ignored") {
		t.Errorf("cmd should collapse to first line: %q", msg)
	}
}

// TestListToolsRegisteredWithScheduleSurface: the read-only list tools appear
// exactly when the schedule surface is wired in (non-empty schedulesPath), the
// same gate their create counterparts sit behind.
func TestListToolsRegisteredWithScheduleSurface(t *testing.T) {
	with, _ := listToolNames(t, NewServer(&fakeToolSet{}, false, "/state/transcript.jsonl", []string{"telegram"}, "", "", filepath.Join(t.TempDir(), "schedules.json"), ""))
	for _, want := range []string{"list_schedules", "list_loops"} {
		if !with[want] {
			t.Fatalf("%s tool missing; tools=%v", want, with)
		}
	}
	without, _ := listToolNames(t, NewServer(&fakeToolSet{}, false, "/state/transcript.jsonl", []string{"telegram"}, "", "", "", ""))
	for _, name := range []string{"list_schedules", "list_loops"} {
		if without[name] {
			t.Errorf("%s tool should be absent when schedulesPath is empty", name)
		}
	}
}
