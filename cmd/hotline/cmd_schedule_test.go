package main

import (
	"path/filepath"
	"testing"
	"time"

	"github.com/1broseidon/hotline/internal/schedule"
)

// seedSchedules points the state root at a temp dir and writes a doc there,
// returning the schedules.json path.
func seedSchedules(t *testing.T, schedules ...schedule.Schedule) string {
	t.Helper()
	dir := t.TempDir()
	t.Setenv("HOTLINE_STATE_DIR", dir)
	path := filepath.Join(dir, "schedules.json")
	if err := schedule.Save(&schedule.Doc{Schedules: schedules}, path); err != nil {
		t.Fatal(err)
	}
	return path
}

func sampleDaily(id string) schedule.Schedule {
	return schedule.Schedule{
		ID: id, Prompt: "check the deploy", Source: "telegram", ChatID: "123",
		Recurrence: schedule.Recurrence{Kind: schedule.KindDaily, TimeOfDay: "09:00"},
		NextFire:   time.Now().Add(time.Hour).UTC().Format(time.RFC3339),
	}
}

func TestCmdScheduleUsage(t *testing.T) {
	seedSchedules(t)
	if err := cmdSchedule(nil); err == nil {
		t.Error("no args should error with usage")
	}
	if err := cmdSchedule([]string{"bogus"}); err == nil {
		t.Error("unknown subcommand should error")
	}
	for _, verb := range []string{"remove", "pause", "resume"} {
		if err := cmdSchedule([]string{verb}); err == nil {
			t.Errorf("%s without id should error", verb)
		}
	}
}

func TestCmdScheduleList(t *testing.T) {
	seedSchedules(t, sampleDaily("aaaaaa"), sampleDaily("bbbbbb"))
	if err := cmdSchedule([]string{"list"}); err != nil {
		t.Fatalf("list: %v", err)
	}
}

func TestCmdScheduleRemove(t *testing.T) {
	path := seedSchedules(t, sampleDaily("aaaaaa"), sampleDaily("bbbbbb"))
	if err := cmdSchedule([]string{"remove", "aaaaaa"}); err != nil {
		t.Fatalf("remove: %v", err)
	}
	d, _ := schedule.Load(path)
	if len(d.Schedules) != 1 || d.Schedules[0].ID != "bbbbbb" {
		t.Errorf("after remove: %+v", d.Schedules)
	}
	if err := cmdSchedule([]string{"remove", "zzzzzz"}); err == nil {
		t.Error("removing missing id should error")
	}
}

func TestCmdSchedulePauseResume(t *testing.T) {
	path := seedSchedules(t, sampleDaily("aaaaaa"))
	if err := cmdSchedule([]string{"pause", "aaaaaa"}); err != nil {
		t.Fatalf("pause: %v", err)
	}
	d, _ := schedule.Load(path)
	if !d.Schedules[0].Paused {
		t.Error("schedule should be paused")
	}
	if err := cmdSchedule([]string{"resume", "aaaaaa"}); err != nil {
		t.Fatalf("resume: %v", err)
	}
	d, _ = schedule.Load(path)
	if d.Schedules[0].Paused {
		t.Error("schedule should be resumed")
	}
	nf, _ := time.Parse(time.RFC3339, d.Schedules[0].NextFire)
	if !nf.After(time.Now()) {
		t.Errorf("resume should leave NextFire in the future, got %v", nf)
	}
}
