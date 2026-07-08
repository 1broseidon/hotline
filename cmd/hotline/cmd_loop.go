package main

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"strconv"
	"strings"
	"time"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/loop"
	"github.com/1broseidon/hotline/internal/notify"
)

type exitCodeError int

func (e exitCodeError) Error() string { return fmt.Sprintf("exit status %d", int(e)) }
func (e exitCodeError) Code() int     { return int(e) }

// cmdLoop is the operator surface over loops.json: add/list/remove/pause/
// resume/logs/run. Loop creation is CLI-first because the command is local
// operator code, not chat-authored agent text.
func cmdLoop(args []string, out, errout io.Writer) error {
	if len(args) < 1 {
		return errors.New("usage: hotline loop <add|list|remove|pause|resume|logs|run> [args]")
	}
	stateRoot, err := config.StateRoot()
	if err != nil {
		return err
	}
	path := loop.Path(stateRoot)

	needLabel := func() (string, error) {
		if len(args) < 2 {
			return "", fmt.Errorf("usage: hotline loop %s <label>", args[0])
		}
		return args[1], nil
	}

	switch args[0] {
	case "add":
		return loopAdd(stateRoot, path, args[1:], out)
	case "list":
		return loopList(path, out)
	case "remove":
		label, err := needLabel()
		if err != nil {
			return err
		}
		l, err := loop.Remove(path, label)
		if err != nil {
			return err
		}
		fmt.Fprintf(out, "Removed loop %q.\n", l.Label)
		return nil
	case "pause":
		label, err := needLabel()
		if err != nil {
			return err
		}
		l, err := loop.SetPaused(path, label, true)
		if err != nil {
			return err
		}
		fmt.Fprintf(out, "Paused loop %q.\n", l.Label)
		return nil
	case "resume":
		label, err := needLabel()
		if err != nil {
			return err
		}
		l, err := loop.SetPaused(path, label, false)
		if err != nil {
			return err
		}
		fmt.Fprintf(out, "Resumed loop %q.\n", l.Label)
		return nil
	case "logs":
		label, err := needLabel()
		if err != nil {
			return err
		}
		return loopLogs(stateRoot, label, args[2:], out)
	case "run":
		label, err := needLabel()
		if err != nil {
			return err
		}
		return loopRun(stateRoot, label, args[2:], out, errout)
	default:
		return fmt.Errorf("unknown loop command %q (add, list, remove, pause, resume, logs, run)", args[0])
	}
}

func loopAdd(stateRoot, path string, args []string, out io.Writer) error {
	var l loop.Loop
	var sourceSet bool

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
		case a == "--every":
			v, err := takeVal()
			if err != nil {
				return err
			}
			l.Every = v
		case strings.HasPrefix(a, "--every="):
			l.Every = strings.TrimPrefix(a, "--every=")
		case a == "--cmd":
			v, err := takeVal()
			if err != nil {
				return err
			}
			l.Cmd = v
		case strings.HasPrefix(a, "--cmd="):
			l.Cmd = strings.TrimPrefix(a, "--cmd=")
		case a == "--notify-llm":
			l.NotifyLLM = true
		case a == "--sink":
			v, err := takeVal()
			if err != nil {
				return err
			}
			l.Sink = v
		case strings.HasPrefix(a, "--sink="):
			l.Sink = strings.TrimPrefix(a, "--sink=")
		case a == "--source":
			v, err := takeVal()
			if err != nil {
				return err
			}
			l.Source, sourceSet = v, true
		case strings.HasPrefix(a, "--source="):
			l.Source, sourceSet = strings.TrimPrefix(a, "--source="), true
		case a == "--level":
			v, err := takeVal()
			if err != nil {
				return err
			}
			l.Level = v
		case strings.HasPrefix(a, "--level="):
			l.Level = strings.TrimPrefix(a, "--level=")
		case a == "--timeout":
			v, err := takeVal()
			if err != nil {
				return err
			}
			l.Timeout = v
		case strings.HasPrefix(a, "--timeout="):
			l.Timeout = strings.TrimPrefix(a, "--timeout=")
		case strings.HasPrefix(a, "--"):
			return fmt.Errorf("unknown flag %q", a)
		default:
			if l.Label != "" {
				return fmt.Errorf("unexpected argument %q (one label only)", a)
			}
			l.Label = a
		}
	}
	if l.Label == "" || l.Every == "" || l.Cmd == "" {
		return errors.New("usage: hotline loop add <label> --every <dur> --cmd \"<shell>\" [--notify-llm] [--sink notify] [--source <notify-label>] [--level urgent|normal|low] [--timeout <dur>]")
	}
	if _, err := notify.ParseLevel(l.Level); err != nil {
		return fmt.Errorf("--level: %w", err)
	}
	sourcesPath := notify.SourcesPath(stateRoot)
	autoSource := ""
	if sourceSet {
		if err := requireSourceLabel(sourcesPath, l.Source); err != nil {
			return err
		}
	} else if l.NotifyLLM {
		src, err := notify.AddSource(sourcesPath, l.Label, notify.LevelNormal, notify.Rate{}, "", time.Now())
		if err != nil {
			return fmt.Errorf("auto-adding notify source %q: %w", l.Label, err)
		}
		l.Source = src.Label
		autoSource = src.Label
		fmt.Fprintf(out, "Added notify source %q for loop %q.\n", src.Label, l.Label)
		fmt.Fprintf(out, "Key (kept in sources.json; loops.json stores only the label): %s\n", src.Key)
	}

	stored, err := loop.Add(path, l, time.Now())
	if err != nil {
		if autoSource != "" {
			_, _ = notify.RevokeSource(sourcesPath, autoSource)
		}
		return err
	}
	fmt.Fprintf(out, "Added loop %q every %s.\n", stored.Label, stored.Every)
	if stored.NotifyLLM {
		fmt.Fprintf(out, "Non-empty stdout routes to notify source %q at level %s.\n", stored.Source, levelOrDefault(stored.Level))
	}
	fmt.Fprintf(out, "State dir: %s\n", loop.StateDir(stateRoot, stored.Label))
	return nil
}

func requireSourceLabel(path, label string) error {
	label = strings.TrimSpace(label)
	if label == "" {
		return fmt.Errorf("--source needs a notify source label")
	}
	reg, err := notify.LoadRegistry(path)
	if err != nil {
		return err
	}
	if _, ok := reg.FindByLabel(label); !ok {
		return fmt.Errorf("notify source %q not found (create it with `hotline source add %s`)", label, label)
	}
	return nil
}

func loopList(path string, out io.Writer) error {
	d, err := loop.Load(path)
	if err != nil {
		return err
	}
	fmt.Fprintf(out, "%d loop(s)\n", len(d.Loops))
	for _, l := range d.Loops {
		flag := ""
		if l.Paused {
			flag = " [paused]"
		}
		fmt.Fprintf(out, "  - %-16s every %-8s last %s exit %-4d runs %-4d%s\n",
			l.Label, l.Every, localTimeOrDash(l.LastRunAt), l.LastExit, l.Runs, flag)
		route := "script-owned"
		if l.NotifyLLM {
			route = fmt.Sprintf("notify source %q level %s", l.Source, levelOrDefault(l.Level))
		}
		fmt.Fprintf(out, "      %s; timeout %s; %s\n", route, l.Timeout, firstLine(l.Cmd, 100))
	}
	return nil
}

func loopLogs(stateRoot, label string, args []string, out io.Writer) error {
	n := 80
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
		case a == "-n":
			v, err := takeVal()
			if err != nil {
				return err
			}
			nn, err := strconv.Atoi(v)
			if err != nil || nn < 0 {
				return fmt.Errorf("-n must be a non-negative integer")
			}
			n = nn
		case strings.HasPrefix(a, "-n="):
			nn, err := strconv.Atoi(strings.TrimPrefix(a, "-n="))
			if err != nil || nn < 0 {
				return fmt.Errorf("-n must be a non-negative integer")
			}
			n = nn
		default:
			return fmt.Errorf("unknown logs argument %q", a)
		}
	}
	data, err := os.ReadFile(loop.LogPath(stateRoot, label))
	if err != nil {
		if os.IsNotExist(err) {
			return fmt.Errorf("no log for loop %q yet", label)
		}
		return err
	}
	lines := bytes.SplitAfter(data, []byte("\n"))
	if len(lines) > 0 && len(lines[len(lines)-1]) == 0 {
		lines = lines[:len(lines)-1]
	}
	if n < len(lines) {
		lines = lines[len(lines)-n:]
	}
	for _, line := range lines {
		if _, err := out.Write(line); err != nil {
			return err
		}
	}
	return nil
}

func loopRun(stateRoot, label string, args []string, out, errout io.Writer) error {
	for _, a := range args {
		if a != "--once" {
			return fmt.Errorf("unknown run argument %q", a)
		}
	}
	d, err := loop.Load(loop.Path(stateRoot))
	if err != nil {
		return err
	}
	var l loop.Loop
	found := false
	for _, candidate := range d.Loops {
		if candidate.Label == label {
			l, found = candidate, true
			break
		}
	}
	if !found {
		return fmt.Errorf("%w: %q", loop.ErrNotFound, label)
	}
	if l.Paused {
		return fmt.Errorf("loop %q is paused", label)
	}
	r := loop.NewRunner(stateRoot)
	r.Log = errout
	res, err := r.RunOnce(context.Background(), l)
	if err != nil {
		return err
	}
	if res.Skipped {
		fmt.Fprintf(out, "loop %q skipped: still running\n", label)
		return exitCodeError(99)
	}
	fmt.Fprintf(out, "loop %q exit=%d duration=%s\n", label, res.ExitCode, res.Duration.Round(time.Millisecond))
	if res.ExitCode != 0 {
		return exitCodeError(res.ExitCode)
	}
	return nil
}

func levelOrDefault(s string) string {
	l, err := notify.ParseLevel(s)
	if err != nil {
		return s
	}
	return string(l)
}
