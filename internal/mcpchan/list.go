package mcpchan

import (
	"fmt"
	"strings"

	"github.com/1broseidon/hotline/internal/loop"
	"github.com/1broseidon/hotline/internal/notify"
	"github.com/1broseidon/hotline/internal/schedule"
)

// listSchedulesSchema / listLoopsSchema are the verbatim InputSchemas for the
// read-only introspection tools. Both take no input: an object with no
// properties, so any arguments the client sends are ignored.
const (
	listSchedulesSchema = `{"type":"object","properties":{}}`
	listLoopsSchema     = `{"type":"object","properties":{}}`
)

// handleListSchedules renders every persisted schedule as a compact plaintext
// summary. It reads the same schedules.json the schedule tool writes. Failures
// use the house "list_schedules failed: …" prefix and return isErr=true.
func handleListSchedules(path string) (string, bool) {
	d, err := schedule.Load(path)
	if err != nil {
		return "list_schedules failed: " + err.Error(), true
	}
	if len(d.Schedules) == 0 {
		return "No schedules set.", false
	}
	lines := make([]string, 0, len(d.Schedules)+1)
	lines = append(lines, fmt.Sprintf("%d schedule(s):", len(d.Schedules)))
	for _, sc := range d.Schedules {
		state := "active"
		if sc.Paused {
			state = "paused"
		}
		lines = append(lines, fmt.Sprintf("%s  %s  %s  next %s  last %s  %s chat %s  — %s",
			sc.ID, state, schedule.Describe(sc.Recurrence), fireTimeOrNever(sc.NextFire),
			fireTimeOrNever(sc.LastFired), sc.Source, sc.ChatID, firstLineRunes(sc.Prompt, 80)))
	}
	return strings.Join(lines, "\n"), false
}

// handleListLoops renders every registered loop, mirroring the labels the
// hotline CLI's loop list uses. It reads loops.json under stateRoot. Failures
// use the house "list_loops failed: …" prefix and return isErr=true.
func handleListLoops(stateRoot string) (string, bool) {
	d, err := loop.Load(loop.Path(stateRoot))
	if err != nil {
		return "list_loops failed: " + err.Error(), true
	}
	if len(d.Loops) == 0 {
		return "No loops configured.", false
	}
	lines := make([]string, 0, 2*len(d.Loops)+1)
	lines = append(lines, fmt.Sprintf("%d loop(s):", len(d.Loops)))
	for _, l := range d.Loops {
		approval := "approved"
		if !l.Approved {
			approval = "pending"
		}
		state := "active"
		if l.Paused {
			state = "paused"
		}
		lines = append(lines, fmt.Sprintf("%s  %s  %s  every %s  last run %s  exit %d  runs %d",
			l.Label, approval, state, l.Every, fireTimeOrNever(l.LastRunAt), l.LastExit, l.Runs))
		route := "script-owned"
		if l.NotifyLLM {
			route = fmt.Sprintf("notify source %q level %s", l.Source, loopLevelOrDefault(l.Level))
		}
		lines = append(lines, fmt.Sprintf("    %s; %s", route, firstLineRunes(l.Cmd, 80)))
	}
	return strings.Join(lines, "\n"), false
}

// fireTimeOrNever renders a stored RFC3339 UTC instant in local time, or
// "never" when it was never set (empty). It reuses localFireTime so the
// display matches the schedule tool's own list.
func fireTimeOrNever(rfc string) string {
	if rfc == "" {
		return "never"
	}
	return localFireTime(rfc)
}

// firstLineRunes collapses s to its first line, then truncates to at most n
// runes with an ellipsis when cut — mirroring the CLI's firstLine helper.
func firstLineRunes(s string, n int) string {
	if i := strings.IndexByte(s, '\n'); i >= 0 {
		s = s[:i]
	}
	return truncateRunes(s, n)
}

// loopLevelOrDefault renders a loop's notify level, defaulting an empty/invalid
// value the way the CLI's levelOrDefault does (empty → normal).
func loopLevelOrDefault(s string) string {
	l, err := notify.ParseLevel(s)
	if err != nil {
		return s
	}
	return string(l)
}
