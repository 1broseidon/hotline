package mcpchan

import (
	"fmt"
	"slices"
	"strings"
	"time"

	"github.com/1broseidon/hotline/internal/schedule"
)

// scheduleSchema is the verbatim InputSchema for the schedule tool. source is
// declared here (optional) rather than injected by withSourceProperty because
// it only applies to action=create; the handler enforces it exactly like
// router.pick does for outbound tools.
const scheduleSchema = `{"type":"object","properties":{"action":{"type":"string","enum":["create","list","cancel"],"description":"create a scheduled task, list existing ones, or cancel one by id."},"prompt":{"type":"string","description":"create: the task, written to your future self — at fire time it is injected back to you as an inbound turn with full tool access. Say what to do and what to send the user. Max 4096 bytes."},"chat_id":{"type":"string","description":"create: the chat the fire is addressed to — use the chat_id of the conversation asking for it. It comes back in the fired turn's meta so you know where to reply."},"source":{"type":"string","description":"create: which channel the fire is addressed to — echo the source of the current conversation. Optional with one channel configured; required with several."},"repeat":{"type":"string","enum":["once","daily","weekly","every_n_hours","every_n_days"],"description":"create: how the schedule recurs. For interval kinds the first fire is one interval from now — if something should also happen right now, just do it now."},"at":{"type":"string","description":"create, repeat=once: when to fire. Relative: +duration from now (e.g. +2m, +45m, +1h30m — units h/m/s). Absolute: server-local YYYY-MM-DDTHH:MM (e.g. 2026-07-08T09:00) or RFC3339. Must be in the future."},"time_of_day":{"type":"string","description":"create: fire time, 24h HH:MM server-local (e.g. 09:00). Required for daily and weekly; optional for every_n_days."},"weekday":{"type":"string","enum":["monday","tuesday","wednesday","thursday","friday","saturday","sunday"],"description":"create, repeat=weekly: the day it fires."},"every_n":{"type":"integer","description":"create: interval count — hours for every_n_hours (1-720), days for every_n_days (1-365)."},"id":{"type":"string","description":"cancel: the schedule id (or unique prefix) from list."}},"required":["action"]}`

// ScheduleInput is the decoded argument set for the schedule tool.
type ScheduleInput struct {
	Action    string `json:"action"`
	Prompt    string `json:"prompt"`
	ChatID    string `json:"chat_id"`
	Source    string `json:"source"`
	Repeat    string `json:"repeat"`
	At        string `json:"at"`
	TimeOfDay string `json:"time_of_day"`
	Weekday   string `json:"weekday"`
	EveryN    int    `json:"every_n"`
	ID        string `json:"id"`
}

// handleSchedule implements the schedule tool: create, list, cancel. All
// failures use the house "schedule failed: …" prefix and return isErr=true.
func handleSchedule(in ScheduleInput, path string, sources []string) (string, bool) {
	switch in.Action {
	case "create":
		return scheduleCreate(in, path, sources)
	case "list":
		return scheduleList(path)
	case "cancel":
		return scheduleCancel(in, path)
	default:
		return fmt.Sprintf("schedule failed: unknown action %q (create, list, cancel)", in.Action), true
	}
}

func scheduleCreate(in ScheduleInput, path string, sources []string) (string, bool) {
	if strings.TrimSpace(in.Prompt) == "" {
		return "schedule failed: prompt is required", true
	}
	if strings.TrimSpace(in.ChatID) == "" {
		return "schedule failed: chat_id is required", true
	}
	src, errMsg := pickSource(in.Source, sources)
	if errMsg != "" {
		return errMsg, true
	}

	rec := schedule.Recurrence{
		Kind:      in.Repeat,
		TimeOfDay: in.TimeOfDay,
		Weekday:   in.Weekday,
		EveryN:    in.EveryN,
	}
	now := time.Now()
	var onceAt time.Time
	if rec.Kind == schedule.KindOnce {
		t, err := schedule.ParseOnceAt(in.At, now, time.Local)
		if err != nil {
			return "schedule failed: " + err.Error(), true
		}
		onceAt = t
	}

	next, err := schedule.First(rec, onceAt, now, time.Local)
	if err != nil {
		return "schedule failed: " + err.Error(), true
	}
	stored, err := schedule.Add(path, schedule.Schedule{
		Prompt:     in.Prompt,
		Source:     src,
		ChatID:     in.ChatID,
		CreatedBy:  "agent",
		Recurrence: rec,
		NextFire:   next.UTC().Format(time.RFC3339),
	}, now)
	if err != nil {
		return "schedule failed: " + err.Error(), true
	}
	return fmt.Sprintf("Scheduled %s — %s. Next fire: %s. Cancel anytime with schedule cancel id=%s.",
		stored.ID, schedule.Describe(rec), next.In(time.Local).Format("2006-01-02 15:04"), stored.ID), false
}

func scheduleList(path string) (string, bool) {
	d, err := schedule.Load(path)
	if err != nil {
		return "schedule failed: " + err.Error(), true
	}
	if len(d.Schedules) == 0 {
		return "No schedules.", false
	}
	lines := make([]string, 0, len(d.Schedules)+1)
	lines = append(lines, fmt.Sprintf("%d schedule(s):", len(d.Schedules)))
	for _, sc := range d.Schedules {
		paused := ""
		if sc.Paused {
			paused = "  [paused]"
		}
		lines = append(lines, fmt.Sprintf("%s  %s  next %s  %s chat %s%s  — %s",
			sc.ID, schedule.Describe(sc.Recurrence), localFireTime(sc.NextFire),
			sc.Source, sc.ChatID, paused, truncateRunes(sc.Prompt, 80)))
	}
	return strings.Join(lines, "\n"), false
}

func scheduleCancel(in ScheduleInput, path string) (string, bool) {
	if strings.TrimSpace(in.ID) == "" {
		return "schedule failed: id is required", true
	}
	sc, err := schedule.Remove(path, in.ID)
	if err != nil {
		return "schedule failed: " + err.Error(), true
	}
	return fmt.Sprintf("Cancelled schedule %s (%s).", sc.ID, schedule.Describe(sc.Recurrence)), false
}

// pickSource resolves the stored source for a create, mirroring router.pick:
// empty + one configured → default to it; empty + several → error; non-empty
// must be a configured source (when any are configured).
func pickSource(source string, sources []string) (resolved, errMsg string) {
	if source == "" {
		switch len(sources) {
		case 0:
			return "", ""
		case 1:
			return sources[0], ""
		default:
			return "", "schedule failed: multiple channels connected — pass source (one of: " + strings.Join(sources, ", ") + ")"
		}
	}
	if len(sources) > 0 && !slices.Contains(sources, source) {
		return "", fmt.Sprintf("schedule failed: unknown source %q (configured: %s)", source, strings.Join(sources, ", "))
	}
	return source, ""
}

// localFireTime renders a stored RFC3339 UTC instant in local time for display.
func localFireTime(rfc string) string {
	t, err := time.Parse(time.RFC3339, rfc)
	if err != nil {
		return rfc
	}
	return t.In(time.Local).Format("2006-01-02 15:04")
}

// truncateRunes shortens s to at most n runes, appending an ellipsis when cut.
func truncateRunes(s string, n int) string {
	r := []rune(s)
	if len(r) <= n {
		return s
	}
	return string(r[:n]) + "…"
}
