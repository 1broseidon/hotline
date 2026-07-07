package main

import (
	"errors"
	"fmt"
	"io"
	"strconv"
	"strings"
	"time"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/notify"
)

// cmdSource is the operator surface over the notify capability-key registry
// (sources.json): add mints a key and prints it, list joins the registry to the
// spool's lifetime counters, revoke removes a key (instantly failing the gate,
// since every notify call reads the registry fresh). Keys are held by scripts;
// every human-facing surface shows the label.
func cmdSource(args []string, out io.Writer) error {
	if len(args) < 1 {
		return errors.New("usage: hotline source <add|list|revoke> [label]")
	}
	stateRoot, err := config.StateRoot()
	if err != nil {
		return err
	}
	path := notify.SourcesPath(stateRoot)

	switch args[0] {
	case "add":
		return sourceAdd(path, args[1:], out)
	case "list":
		return sourceList(path, notify.SpoolPath(stateRoot), out)
	case "revoke":
		if len(args) < 2 {
			return errors.New("usage: hotline source revoke <label>")
		}
		s, err := notify.RevokeSource(path, args[1])
		if err != nil {
			return err
		}
		fmt.Fprintf(out, "Revoked source %q.\n", s.Label)
		return nil
	default:
		return fmt.Errorf("unknown source command %q (add, list, revoke)", args[0])
	}
}

func sourceAdd(path string, args []string, out io.Writer) error {
	var label, capStr, chatID string
	var rate notify.Rate

	for i := 0; i < len(args); i++ {
		a := args[i]
		takeVal := func() (string, error) {
			if i+1 >= len(args) {
				return "", fmt.Errorf("%s needs a value", a)
			}
			i++
			return args[i], nil
		}
		switch {
		case a == "--cap":
			v, err := takeVal()
			if err != nil {
				return err
			}
			capStr = v
		case strings.HasPrefix(a, "--cap="):
			capStr = strings.TrimPrefix(a, "--cap=")
		case a == "--burst":
			v, err := takeVal()
			if err != nil {
				return err
			}
			if rate.Burst, err = strconv.Atoi(v); err != nil {
				return fmt.Errorf("--burst must be an integer: %w", err)
			}
		case a == "--refill-mins":
			v, err := takeVal()
			if err != nil {
				return err
			}
			if rate.RefillMins, err = strconv.Atoi(v); err != nil {
				return fmt.Errorf("--refill-mins must be an integer: %w", err)
			}
		case a == "--chat-id":
			v, err := takeVal()
			if err != nil {
				return err
			}
			chatID = v
		case strings.HasPrefix(a, "--"):
			return fmt.Errorf("unknown flag %q", a)
		default:
			if label != "" {
				return fmt.Errorf("unexpected argument %q (one label only)", a)
			}
			label = a
		}
	}
	if label == "" {
		return errors.New("usage: hotline source add <label> [--cap urgent|normal|low] [--burst N] [--refill-mins M] [--chat-id ID]")
	}
	cap, err := notify.ParseLevel(capStr)
	if err != nil {
		return fmt.Errorf("--cap: %w", err)
	}
	s, err := notify.AddSource(path, label, cap, rate, chatID, time.Now())
	if err != nil {
		return err
	}
	fmt.Fprintf(out, "Added source %q (level cap: %s).\n", s.Label, s.LevelCap)
	fmt.Fprintf(out, "Key (store it; scripts pass it as --source): %s\n", s.Key)
	fmt.Fprintf(out, "Wire a script to it, e.g.:\n")
	fmt.Fprintf(out, "  tail -1 backup.log | hotline notify --source $KEY --level low\n")
	fmt.Fprintf(out, "(Re-show it any time with `hotline source list`.)\n")
	return nil
}

func sourceList(sourcesPath, spoolPath string, out io.Writer) error {
	reg, err := notify.LoadRegistry(sourcesPath)
	if err != nil {
		return err
	}
	sp, err := notify.LoadSpool(spoolPath)
	if err != nil {
		return err
	}
	if reg.QuietHours != "" {
		fmt.Fprintf(out, "quiet hours: %s\n", reg.QuietHours)
	}
	fmt.Fprintf(out, "%d source(s)\n", len(reg.Sources))
	for _, s := range reg.Sources {
		rate := "default (burst 5, refill 5m)"
		if s.Rate.Burst > 0 || s.Rate.RefillMins > 0 {
			rate = fmt.Sprintf("burst %d, refill %dm", s.Rate.Burst, s.Rate.RefillMins)
		}
		fmt.Fprintf(out, "  - %-16s cap %-6s  %s\n", s.Label, s.LevelCap, rate)
		fmt.Fprintf(out, "      key %s\n", s.Key)
		if st := sp.State[s.Label]; st != nil {
			fmt.Fprintf(out, "      delivered %d  suppressed %d  last-seen %s\n",
				st.Delivered, st.Suppressed, localTimeOrDash(st.LastSeen))
		}
	}
	return nil
}
