package main

import (
	"fmt"
	"io"
	"strconv"
	"strings"
	"time"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/jobspool"
)

// cmdJob is the automatic-job-card ingress: `hotline job start|update|done
// --cookie <id> [--agent <id>] [--batch <id>] [--title …] [--detail …]
// [--progress f] [--state ok|err|cancelled] [--notify …] [--chat …]`. A harness
// hook calls it around a subagent dispatch; the CLI durably enqueues the intent
// and the box's JobDispatcher drives the live card. It returns a process exit
// code so a hook can log outcomes, mirroring `hotline notify`.
//
// --agent carries the harness's completion-side id. On an update it BINDS that
// id to the cookie; on a done it may stand in for the cookie entirely, which is
// how asynchronous work gets closed — the event that reports a background
// subagent's completion knows its agent id and not the tool_use_id the card was
// opened under.
//
// `hotline job list` is the operator's view of the pending spool.
func cmdJob(botName string, args []string, stdout, stderr io.Writer) int {
	if len(args) >= 1 && args[0] == "list" {
		return jobList(botName, stdout, stderr)
	}
	if len(args) < 1 {
		return usageErr(stderr, "hotline job start|update|done --cookie <id> [flags]")
	}
	action := args[0]
	switch action {
	case "start", "update", "done":
	default:
		return usageErr(stderr, fmt.Sprintf("unknown job action %q (start, update, done, list)", action))
	}

	in := jobspool.Intent{Action: action}
	rest := args[1:]
	for i := 0; i < len(rest); i++ {
		a := rest[i]
		val := func() (string, bool) {
			if eq := strings.IndexByte(a, '='); strings.HasPrefix(a, "--") && eq >= 0 {
				return a[eq+1:], true
			}
			if i+1 >= len(rest) {
				return "", false
			}
			i++
			return rest[i], true
		}
		key := a
		if eq := strings.IndexByte(a, '='); eq >= 0 {
			key = a[:eq]
		}
		switch key {
		case "--cookie":
			v, ok := val()
			if !ok {
				return usageErr(stderr, "--cookie needs a value")
			}
			in.Cookie = v
		case "--agent":
			v, ok := val()
			if !ok {
				return usageErr(stderr, "--agent needs a value")
			}
			in.Agent = v
		case "--batch":
			v, ok := val()
			if !ok {
				return usageErr(stderr, "--batch needs a value")
			}
			in.Batch = v
		case "--title":
			v, ok := val()
			if !ok {
				return usageErr(stderr, "--title needs a value")
			}
			in.Title = v
		case "--detail":
			v, ok := val()
			if !ok {
				return usageErr(stderr, "--detail needs a value")
			}
			in.Detail = v
		case "--notify":
			v, ok := val()
			if !ok {
				return usageErr(stderr, "--notify needs a value")
			}
			in.Notify = v
		case "--chat":
			v, ok := val()
			if !ok {
				return usageErr(stderr, "--chat needs a value")
			}
			in.ChatID = v
		case "--state":
			v, ok := val()
			if !ok {
				return usageErr(stderr, "--state needs a value")
			}
			in.State = v
		case "--progress":
			v, ok := val()
			if !ok {
				return usageErr(stderr, "--progress needs a value")
			}
			f, err := strconv.ParseFloat(strings.TrimSpace(v), 64)
			if err != nil || f < 0 || f > 1 {
				return usageErr(stderr, "--progress must be a number within 0..1")
			}
			in.Progress = &f
		default:
			return usageErr(stderr, fmt.Sprintf("unknown flag %q", a))
		}
	}

	// A job is addressed by --cookie (the dispatch-side id) or, for a completion
	// event that only knows the harness's agent id, by --agent alone. An --agent
	// with no prior binding is dropped by the dispatcher, never carded.
	if strings.TrimSpace(in.Cookie) == "" && strings.TrimSpace(in.Agent) == "" {
		return usageErr(stderr, "--cookie is required (the harness's tool_use_id), or --agent for a completion addressed by agent id")
	}
	if strings.TrimSpace(in.Cookie) == "" && action != "done" {
		return usageErr(stderr, fmt.Sprintf("--agent addresses a done, not a %s; %s needs --cookie", action, action))
	}
	if action == "start" && strings.TrimSpace(in.Title) == "" {
		return usageErr(stderr, "start requires --title")
	}
	if action == "done" {
		switch in.State {
		case "", "ok", "err", "cancelled":
			// empty defaults to ok in the dispatcher
		default:
			return usageErr(stderr, "--state must be ok, err, or cancelled")
		}
	}

	boxRoot, err := config.BoxRoot(botName)
	if err != nil {
		fmt.Fprintf(stderr, "hotline: %v\n", err)
		return exitInternal
	}
	out, err := jobspool.Enqueue(jobspool.SpoolPath(boxRoot), in, time.Now())
	if err != nil {
		fmt.Fprintf(stderr, "hotline: %v\n", err)
		return exitInternal
	}
	switch out {
	case jobspool.SpoolFull:
		fmt.Fprintln(stdout, "rejected: job spool full")
		return exitRejected
	default:
		key := "cookie=" + in.Cookie
		if in.Cookie == "" {
			key = "agent=" + in.Agent
		}
		fmt.Fprintf(stdout, "accepted (%s %s)\n", action, key)
		return exitAccepted
	}
}

func jobList(botName string, stdout, stderr io.Writer) int {
	boxRoot, err := config.BoxRoot(botName)
	if err != nil {
		fmt.Fprintf(stderr, "hotline: %v\n", err)
		return exitInternal
	}
	sp, err := jobspool.LoadSpool(jobspool.SpoolPath(boxRoot))
	if err != nil {
		fmt.Fprintf(stderr, "hotline: %v\n", err)
		return exitInternal
	}
	fmt.Fprintf(stdout, "%d pending job intent(s)\n", len(sp.Pending))
	for _, e := range sp.Pending {
		batch := e.Batch
		if batch == "" {
			batch = "(self)"
		}
		fmt.Fprintf(stdout, "  - #%d %-6s cookie=%s batch=%s %s\n", e.Seq, e.Action, e.Cookie, batch, firstLine(e.Title+" "+e.Detail, 60))
	}
	ac, err := jobspool.LoadActive(jobspool.ActivePath(boxRoot))
	if err == nil && len(ac.Batches) > 0 {
		fmt.Fprintf(stdout, "%d active card(s):\n", len(ac.Batches))
		for _, b := range ac.Batches {
			total, term := 0, 0
			for _, j := range b.Jobs {
				total++
				if j.State != "" && j.State != "running" {
					term++
				}
			}
			fmt.Fprintf(stdout, "  - %s job=%s %d/%d done  %s\n", b.Batch, b.JobID, term, total, firstLine(b.Title, 50))
		}
	}
	return exitAccepted
}
