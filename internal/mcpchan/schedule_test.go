package mcpchan

import (
	"context"
	"encoding/json"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/schedule"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

func schedPath(t *testing.T) string {
	t.Helper()
	return filepath.Join(t.TempDir(), "schedules.json")
}

func TestScheduleSchemaValidJSON(t *testing.T) {
	var v map[string]any
	if err := json.Unmarshal([]byte(scheduleSchema), &v); err != nil {
		t.Fatalf("scheduleSchema is not valid JSON: %v", err)
	}
	if v["type"] != "object" {
		t.Errorf("type = %v, want object", v["type"])
	}
	req, _ := v["required"].([]any)
	if len(req) != 1 || req[0] != "action" {
		t.Errorf("required = %v, want [action]", req)
	}
}

func TestHandleScheduleCreateKinds(t *testing.T) {
	sources := []string{"telegram"}
	cases := []struct {
		name string
		in   ScheduleInput
	}{
		{"once", ScheduleInput{Action: "create", Prompt: "p", ChatID: "1", Repeat: "once", At: "2030-01-01T09:00"}},
		{"daily", ScheduleInput{Action: "create", Prompt: "p", ChatID: "1", Repeat: "daily", TimeOfDay: "09:00"}},
		{"weekly", ScheduleInput{Action: "create", Prompt: "p", ChatID: "1", Repeat: "weekly", TimeOfDay: "09:00", Weekday: "sunday"}},
		{"every_n_hours", ScheduleInput{Action: "create", Prompt: "p", ChatID: "1", Repeat: "every_n_hours", EveryN: 4}},
		{"every_n_days", ScheduleInput{Action: "create", Prompt: "p", ChatID: "1", Repeat: "every_n_days", EveryN: 3, TimeOfDay: "08:30"}},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			path := schedPath(t)
			msg, isErr := handleSchedule(c.in, path, sources)
			if isErr {
				t.Fatalf("create %s errored: %s", c.name, msg)
			}
			if !strings.HasPrefix(msg, "Scheduled ") {
				t.Errorf("unexpected success message: %q", msg)
			}
			d, _ := schedule.Load(path)
			if len(d.Schedules) != 1 {
				t.Fatalf("want 1 stored schedule, got %d", len(d.Schedules))
			}
			if d.Schedules[0].CreatedBy != "agent" {
				t.Errorf("createdBy = %q, want agent", d.Schedules[0].CreatedBy)
			}
			if d.Schedules[0].Source != "telegram" {
				t.Errorf("source = %q, want telegram", d.Schedules[0].Source)
			}
		})
	}
}

func TestHandleScheduleCreateValidation(t *testing.T) {
	path := schedPath(t)
	sources := []string{"telegram"}

	noPrompt := ScheduleInput{Action: "create", ChatID: "1", Repeat: "daily", TimeOfDay: "09:00"}
	if msg, isErr := handleSchedule(noPrompt, path, sources); !isErr || !strings.Contains(msg, "prompt") {
		t.Errorf("missing prompt: %q isErr=%v", msg, isErr)
	}
	noChat := ScheduleInput{Action: "create", Prompt: "p", Repeat: "daily", TimeOfDay: "09:00"}
	if msg, isErr := handleSchedule(noChat, path, sources); !isErr || !strings.Contains(msg, "chat_id") {
		t.Errorf("missing chat_id: %q isErr=%v", msg, isErr)
	}
	pastOnce := ScheduleInput{Action: "create", Prompt: "p", ChatID: "1", Repeat: "once", At: "2000-01-01T09:00"}
	if _, isErr := handleSchedule(pastOnce, path, sources); !isErr {
		t.Error("past once should error")
	}
	badRepeat := ScheduleInput{Action: "create", Prompt: "p", ChatID: "1", Repeat: "cron"}
	if _, isErr := handleSchedule(badRepeat, path, sources); !isErr {
		t.Error("bad repeat should error")
	}
}

func TestHandleScheduleSourceResolution(t *testing.T) {
	path := schedPath(t)
	base := ScheduleInput{Action: "create", Prompt: "p", ChatID: "1", Repeat: "daily", TimeOfDay: "09:00"}

	// Multi-source with no source -> error.
	if msg, isErr := handleSchedule(base, path, []string{"telegram", "discord"}); !isErr || !strings.Contains(msg, "multiple channels") {
		t.Errorf("multi-source no source: %q isErr=%v", msg, isErr)
	}
	// Single source with no source -> defaults.
	if msg, isErr := handleSchedule(base, schedPath(t), []string{"telegram"}); isErr {
		t.Errorf("single-source default should succeed: %q", msg)
	}
	// Unknown source -> error listing configured.
	withUnknown := base
	withUnknown.Source = "signal"
	if msg, isErr := handleSchedule(withUnknown, schedPath(t), []string{"telegram", "discord"}); !isErr || !strings.Contains(msg, "unknown source") {
		t.Errorf("unknown source: %q isErr=%v", msg, isErr)
	}
}

func TestHandleScheduleListEmptyAndPopulated(t *testing.T) {
	path := schedPath(t)
	if msg, isErr := handleSchedule(ScheduleInput{Action: "list"}, path, nil); isErr || msg != "No schedules." {
		t.Errorf("empty list: %q isErr=%v", msg, isErr)
	}
	// Populate two.
	handleSchedule(ScheduleInput{Action: "create", Prompt: "aaa", ChatID: "1", Repeat: "daily", TimeOfDay: "09:00"}, path, []string{"telegram"})
	handleSchedule(ScheduleInput{Action: "create", Prompt: "bbb", ChatID: "2", Repeat: "once", At: "2030-01-01T09:00"}, path, []string{"telegram"})
	msg, isErr := handleSchedule(ScheduleInput{Action: "list"}, path, nil)
	if isErr {
		t.Fatalf("list errored: %s", msg)
	}
	if !strings.HasPrefix(msg, "2 schedule(s):") {
		t.Errorf("list header wrong: %q", msg)
	}
	if !strings.Contains(msg, "aaa") || !strings.Contains(msg, "bbb") {
		t.Errorf("list missing prompts: %q", msg)
	}
}

func TestHandleScheduleCancel(t *testing.T) {
	path := schedPath(t)
	handleSchedule(ScheduleInput{Action: "create", Prompt: "p", ChatID: "1", Repeat: "daily", TimeOfDay: "09:00"}, path, []string{"telegram"})
	d, _ := schedule.Load(path)
	id := d.Schedules[0].ID

	// Cancel by unique prefix.
	msg, isErr := handleSchedule(ScheduleInput{Action: "cancel", ID: id[:3]}, path, nil)
	if isErr || !strings.HasPrefix(msg, "Cancelled schedule ") {
		t.Errorf("cancel by prefix: %q isErr=%v", msg, isErr)
	}
	d, _ = schedule.Load(path)
	if len(d.Schedules) != 0 {
		t.Errorf("schedule not removed")
	}
	// Cancel missing.
	if _, isErr := handleSchedule(ScheduleInput{Action: "cancel", ID: "zzzzzz"}, path, nil); !isErr {
		t.Error("cancel missing should error")
	}
	// Cancel with no id.
	if _, isErr := handleSchedule(ScheduleInput{Action: "cancel"}, path, nil); !isErr {
		t.Error("cancel without id should error")
	}
}

func TestHandleScheduleUnknownAction(t *testing.T) {
	if msg, isErr := handleSchedule(ScheduleInput{Action: "frobnicate"}, schedPath(t), nil); !isErr || !strings.Contains(msg, "unknown action") {
		t.Errorf("unknown action: %q isErr=%v", msg, isErr)
	}
}

// TestScheduleToolRegistration verifies the tool appears (with the verbatim
// schema) only when a non-empty schedulesPath is configured.
func TestScheduleToolRegistration(t *testing.T) {
	toolNames := func(schedulesPath string) map[string]string {
		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		server := NewServer(&fakeToolSet{}, false, "/state/transcript.jsonl", []string{"telegram"}, "", "", schedulesPath, "")
		st, ct := mcp.NewInMemoryTransports()
		if _, err := server.Connect(ctx, st, nil); err != nil {
			t.Fatal(err)
		}
		client := mcp.NewClient(&mcp.Implementation{Name: "t", Version: "0"}, nil)
		session, err := client.Connect(ctx, ct, nil)
		if err != nil {
			t.Fatal(err)
		}
		defer session.Close()
		lr, err := session.ListTools(ctx, nil)
		if err != nil {
			t.Fatal(err)
		}
		out := map[string]string{}
		for _, tool := range lr.Tools {
			b, _ := json.Marshal(tool.InputSchema)
			out[tool.Name] = string(b)
		}
		return out
	}

	with := toolNames("/state/schedules.json")
	if got, ok := with["schedule"]; !ok {
		t.Error("schedule tool missing when schedulesPath is set")
	} else if !jsonEqual(t, []byte(scheduleSchema), json.RawMessage(got)) {
		t.Errorf("schedule tool schema mismatch: %s", got)
	}

	without := toolNames("")
	if _, ok := without["schedule"]; ok {
		t.Error("schedule tool should be absent when schedulesPath is empty")
	}
}
