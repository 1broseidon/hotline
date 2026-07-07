package main

import (
	"fmt"
	"io"
	"strings"
	"time"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/notify"
)

// CLI exit codes for the `hotline notify` send path — the script-facing contract.
const (
	exitAccepted = 0 // durably enqueued (also: duplicate coalesced)
	exitInternal = 1 // I/O, lock failure
	exitUsage    = 2 // missing --source, bad --level, empty message
	exitQueued   = 3 // valid but held for quiet hours
	exitRejected = 4 // unknown/revoked key, or rate-limit suppressed
)

// cmdNotify is the event-ingress surface: `hotline notify --source <key>
// [--level L] [message|-]` enqueues one machine event through the gate; `hotline
// notify list` is the operator's spool view. It returns the process exit code —
// the send path distinguishes accepted/queued/rejected so cron jobs and the
// email sentry can log outcomes. stdin is first-class (the email-sentry
// integration is a pipe); a positional message wins over stdin when both exist.
func cmdNotify(args []string, stdin io.Reader, stdout, stderr io.Writer) int {
	if len(args) >= 1 && args[0] == "list" {
		return notifyList(stdout, stderr)
	}
	return notifySend(args, stdin, stdout, stderr)
}

func notifySend(args []string, stdin io.Reader, stdout, stderr io.Writer) int {
	var source, levelStr string
	var positional []string

	endOfFlags := false
	for i := 0; i < len(args); i++ {
		a := args[i]
		if endOfFlags {
			positional = append(positional, a)
			continue
		}
		switch {
		case a == "--":
			// POSIX end-of-options: everything after is positional, even if it
			// starts with "--" (lets a script pass a message that looks like a flag).
			endOfFlags = true
		case a == "--source":
			if i+1 >= len(args) {
				return usageErr(stderr, "--source needs a value")
			}
			source = args[i+1]
			i++
		case strings.HasPrefix(a, "--source="):
			source = strings.TrimPrefix(a, "--source=")
		case a == "--level":
			if i+1 >= len(args) {
				return usageErr(stderr, "--level needs a value")
			}
			levelStr = args[i+1]
			i++
		case strings.HasPrefix(a, "--level="):
			levelStr = strings.TrimPrefix(a, "--level=")
		case a == "-":
			// Explicit stdin marker: consumed, not a positional. Absence of a
			// positional already triggers the stdin read below, so this is advisory.
		case strings.HasPrefix(a, "--"):
			return usageErr(stderr, fmt.Sprintf("unknown flag %q", a))
		default:
			positional = append(positional, a)
		}
	}

	if strings.TrimSpace(source) == "" {
		return usageErr(stderr, "hotline notify --source <key> [--level urgent|normal|low] [message]")
	}
	level, err := notify.ParseLevel(levelStr)
	if err != nil {
		return usageErr(stderr, err.Error())
	}

	// Message: a positional arg wins over stdin (predictable for scripts that
	// pass "$MSG" explicitly while inheriting a pipe); otherwise read stdin.
	var msg string
	if len(positional) > 0 {
		msg = strings.Join(positional, " ")
	} else {
		b, err := io.ReadAll(io.LimitReader(stdin, notify.MaxStdinBytes()))
		if err != nil {
			fmt.Fprintf(stderr, "hotline: reading stdin: %v\n", err)
			return exitInternal
		}
		msg = string(b)
	}
	msg = strings.TrimRight(msg, "\r\n") // trailing newline is cosmetic
	if strings.TrimSpace(msg) == "" {
		return usageErr(stderr, "message is required (positional arg or piped stdin)")
	}

	stateRoot, err := config.StateRoot()
	if err != nil {
		fmt.Fprintf(stderr, "hotline: %v\n", err)
		return exitInternal
	}
	reg, err := notify.LoadRegistry(notify.SourcesPath(stateRoot))
	if err != nil {
		fmt.Fprintf(stderr, "hotline: %v\n", err)
		return exitInternal
	}
	out, err := notify.Enqueue(notify.SpoolPath(stateRoot), notify.RejectsPath(stateRoot), reg, source, level, msg, time.Now())
	if err != nil {
		fmt.Fprintf(stderr, "hotline: %v\n", err)
		return exitInternal
	}

	line, code := notifyOutcome(out)
	fmt.Fprintln(stdout, line)
	return code
}

// notifyOutcome maps a gate Outcome to its stdout line and exit code.
func notifyOutcome(out notify.Outcome) (string, int) {
	switch out.Status {
	case notify.Accepted:
		if out.Clamped {
			return fmt.Sprintf("accepted (level clamped to %s)", out.ClampedTo), exitAccepted
		}
		return "accepted", exitAccepted
	case notify.Duplicate:
		return fmt.Sprintf("accepted (duplicate ×%d coalesced)", out.Count), exitAccepted
	case notify.Queued:
		return fmt.Sprintf("queued until %s (quiet hours)", out.QueuedUntil), exitQueued
	case notify.RejectedUnknown:
		return "rejected: unknown or revoked source key", exitRejected
	case notify.RejectedRate:
		return fmt.Sprintf("rejected: rate limited (%d suppressed since %s)", out.Suppressed, out.SuppressedSince), exitRejected
	case notify.RejectedSpoolFull:
		return "rejected: spool full", exitRejected
	default:
		return "internal error", exitInternal
	}
}

// notifyList prints the operator's spool view: pending/queued entries and
// per-source counters.
func notifyList(stdout, stderr io.Writer) int {
	stateRoot, err := config.StateRoot()
	if err != nil {
		fmt.Fprintf(stderr, "hotline: %v\n", err)
		return exitInternal
	}
	sp, err := notify.LoadSpool(notify.SpoolPath(stateRoot))
	if err != nil {
		fmt.Fprintf(stderr, "hotline: %v\n", err)
		return exitInternal
	}
	fmt.Fprintf(stdout, "%d pending notify(ies)\n", len(sp.Pending))
	for _, e := range sp.Pending {
		flag := ""
		if e.Status == "queued" {
			flag = " [queued]"
		}
		count := ""
		if e.Count > 1 {
			count = fmt.Sprintf(" ×%d", e.Count)
		}
		fmt.Fprintf(stdout, "  - %s  %-16s %-6s%s%s  %s\n", e.ID, e.Label, e.Level, count, flag, firstLine(e.Message, 80))
	}
	if len(sp.State) > 0 {
		fmt.Fprintln(stdout, "counters:")
		for label, st := range sp.State {
			fmt.Fprintf(stdout, "  - %-16s delivered %d  suppressed %d  last-seen %s\n",
				label, st.Delivered, st.Suppressed, localTimeOrDash(st.LastSeen))
		}
	}
	return exitAccepted
}

func usageErr(stderr io.Writer, msg string) int {
	fmt.Fprintf(stderr, "usage: %s\n", msg)
	return exitUsage
}

// localTimeOrDash renders a stored RFC3339 instant in local time, or "-" if empty.
func localTimeOrDash(rfc string) string {
	if rfc == "" {
		return "-"
	}
	return localTime(rfc)
}
