package main

import (
	"errors"
	"fmt"
	"path/filepath"
	"time"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/schedule"
)

// cmdSchedule is the operator surface over schedules.json: list, remove,
// pause, resume. Creation is chat-first (the MCP schedule tool); there is
// deliberately no CLI add in v1.
//
// The running daemon re-reads schedules.json under the flock on every tick, so
// these mutations apply live without a restart — the same live-edit philosophy
// as access.json. Worst case a pause lands one tick late; the flock guarantees
// no lost update either way.
func cmdSchedule(args []string) error {
	if len(args) < 1 {
		return errors.New("usage: hotline schedule <list|remove|pause|resume> [id]")
	}
	stateRoot, err := config.StateRoot()
	if err != nil {
		return err
	}
	path := filepath.Join(stateRoot, "schedules.json")

	needID := func() (string, error) {
		if len(args) < 2 {
			return "", fmt.Errorf("usage: hotline schedule %s <id>", args[0])
		}
		return args[1], nil
	}

	switch args[0] {
	case "list":
		d, err := schedule.Load(path)
		if err != nil {
			return err
		}
		fmt.Printf("%d schedule(s)\n", len(d.Schedules))
		for _, s := range d.Schedules {
			flag := ""
			if s.Paused {
				flag = " [paused]"
			}
			fmt.Printf("  - %s  %-26s next %s  %s chat %s%s\n", s.ID,
				schedule.Describe(s.Recurrence), localTime(s.NextFire), s.Source, s.ChatID, flag)
			fmt.Printf("      %s\n", firstLine(s.Prompt, 100))
		}
		return nil
	case "remove":
		id, err := needID()
		if err != nil {
			return err
		}
		s, err := schedule.Remove(path, id)
		if err != nil {
			return err
		}
		fmt.Printf("Removed schedule %s (%s).\n", s.ID, schedule.Describe(s.Recurrence))
		return nil
	case "pause":
		id, err := needID()
		if err != nil {
			return err
		}
		s, err := schedule.SetPaused(path, id, true, time.Now(), time.Local)
		if err != nil {
			return err
		}
		fmt.Printf("Paused schedule %s.\n", s.ID)
		return nil
	case "resume":
		id, err := needID()
		if err != nil {
			return err
		}
		s, err := schedule.SetPaused(path, id, false, time.Now(), time.Local)
		if err != nil {
			return err
		}
		fmt.Printf("Resumed schedule %s. Next fire: %s.\n", s.ID, localTime(s.NextFire))
		return nil
	default:
		return fmt.Errorf("unknown schedule command %q (list, remove, pause, resume)", args[0])
	}
}

// localTime renders a stored RFC3339 UTC instant in local time for display.
func localTime(rfc string) string {
	t, err := time.Parse(time.RFC3339, rfc)
	if err != nil {
		return rfc
	}
	return t.In(time.Local).Format("2006-01-02 15:04")
}

// firstLine truncates prompt display to one line of at most n runes, collapsing
// a multi-line prompt to its first line.
func firstLine(s string, n int) string {
	for i, r := range s {
		if r == '\n' {
			s = s[:i]
			break
		}
	}
	rs := []rune(s)
	if len(rs) > n {
		return string(rs[:n]) + "…"
	}
	return s
}
