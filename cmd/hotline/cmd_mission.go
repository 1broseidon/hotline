package main

import (
	"encoding/json"
	"fmt"
	"io"
	"strings"

	"github.com/1broseidon/hotline/internal/config"
	"github.com/1broseidon/hotline/internal/mc"
)

// cmdMission is the operator/automation face of Mission Control: a direct-write
// twin of the `mission` MCP tool (spec §3), writing the same files through the
// same internal/mc package under the same flock — no spool, because MC files are
// passive disk state with no daemon owner. It backs three callers: the operator
// (`show`, and hand-authored notes/handoffs), the pi extension's mechanical
// auto-handoff, and Claude Code's SessionStart/PreCompact hooks (`hook`).
//
//	hotline mission note    --text "…" [--thread slug]
//	hotline mission update  --thread slug [--status …] [--summary …] [--next …] [--text …]
//	hotline mission handoff --state "…" --next "…" [--beware "…"] [--trigger …]
//	hotline mission archive --thread slug --outcome "…"
//	hotline mission show    [slug]
//	hotline mission hook    session-start|pre-compact   (Claude Code hook payloads)
//
// It returns a process exit code so a hook/script can branch on the outcome,
// mirroring `hotline notify` / `hotline job`.
func cmdMission(botName string, args []string, stdout, stderr io.Writer) int {
	if len(args) < 1 {
		return usageErr(stderr, "hotline mission note|update|handoff|archive|show|hook [flags]")
	}
	action := args[0]
	rest := args[1:]

	store, err := missionStore(botName)
	if err != nil {
		fmt.Fprintf(stderr, "hotline: %v\n", err)
		return exitInternal
	}

	switch action {
	case "show":
		return missionShow(store, rest, stdout, stderr)
	case "hook":
		return missionHook(store, botName, rest, stdout, stderr)
	case "note", "update", "handoff", "archive":
		// write verbs, handled below
	default:
		return usageErr(stderr, fmt.Sprintf("unknown mission action %q (note, update, handoff, archive, show, hook)", action))
	}

	in := mc.Input{Action: action}
	fl, code := parseMissionFlags(rest, &in, stderr)
	if code != 0 {
		return code
	}
	if fl {
		return exitUsage // parseMissionFlags already printed
	}

	if action == "handoff" && in.Trigger != "" && !validTrigger(in.Trigger) {
		return usageErr(stderr, `--trigger must be one of manual, pre-compact, boundary, auto`)
	}

	msg, isErr := store.Apply(in)
	if isErr {
		fmt.Fprintln(stderr, msg)
		return exitRejected
	}
	fmt.Fprintln(stdout, msg)
	return exitAccepted
}

// missionStore resolves the Mission Control store for the CLI. The directory is
// resolved independent of whether MC is enabled for any harness — the operator
// can always read/write the files directly.
func missionStore(botName string) (*mc.Store, error) {
	mcCfg, err := config.MissionControlForBox("", botName)
	if err != nil {
		return nil, err
	}
	return mc.NewStore(mcCfg.Dir), nil
}

func validTrigger(v string) bool {
	switch v {
	case "manual", "pre-compact", "boundary", "auto":
		return true
	}
	return false
}

// parseMissionFlags fills in from --k v / --k=v flags (the cmd_job idiom). It
// returns (usagePrinted, exitCode): a non-zero exitCode is a hard error already
// reported; usagePrinted true means a flag-level usage error was printed.
func parseMissionFlags(args []string, in *mc.Input, stderr io.Writer) (bool, int) {
	for i := 0; i < len(args); i++ {
		a := args[i]
		key := a
		if eq := strings.IndexByte(a, '='); eq >= 0 && strings.HasPrefix(a, "--") {
			key = a[:eq]
		}
		val := func() (string, bool) {
			if eq := strings.IndexByte(a, '='); strings.HasPrefix(a, "--") && eq >= 0 {
				return a[eq+1:], true
			}
			if i+1 >= len(args) {
				return "", false
			}
			i++
			return args[i], true
		}
		need := func(dst *string) bool {
			v, ok := val()
			if !ok {
				usageErr(stderr, fmt.Sprintf("%s needs a value", key))
				return false
			}
			*dst = v
			return true
		}
		var ok bool
		switch key {
		case "--thread":
			ok = need(&in.Thread)
		case "--text":
			ok = need(&in.Text)
		case "--status":
			ok = need(&in.Status)
		case "--summary":
			ok = need(&in.Summary)
		case "--next":
			ok = need(&in.Next)
		case "--state":
			ok = need(&in.State)
		case "--beware":
			ok = need(&in.Beware)
		case "--outcome":
			ok = need(&in.Outcome)
		case "--trigger":
			ok = need(&in.Trigger)
		default:
			usageErr(stderr, fmt.Sprintf("unknown flag %q", a))
			return true, 0
		}
		if !ok {
			return true, 0
		}
	}
	return false, 0
}

// missionShow is the operator view: no slug prints INDEX.md (plus the current
// handoff and the live thread list); a slug prints that one thread's file.
func missionShow(store *mc.Store, args []string, stdout, stderr io.Writer) int {
	if len(args) >= 1 && !strings.HasPrefix(args[0], "-") {
		slug := args[0]
		body, ok, err := store.ReadThread(slug)
		if err != nil {
			fmt.Fprintf(stderr, "hotline: %v\n", err)
			return exitInternal
		}
		if !ok {
			slugs, _ := store.ListThreadSlugs()
			active := "none"
			if len(slugs) > 0 {
				active = strings.Join(slugs, ", ")
			}
			fmt.Fprintf(stderr, "unknown thread %q — active: %s\n", slug, active)
			return exitRejected
		}
		fmt.Fprint(stdout, body)
		return exitAccepted
	}

	idx, ok, err := store.ReadIndex()
	if err != nil {
		fmt.Fprintf(stderr, "hotline: %v\n", err)
		return exitInternal
	}
	if !ok {
		fmt.Fprintln(stdout, "mission control is empty (no mc/ store yet — it seeds on the next box start)")
		return exitAccepted
	}
	fmt.Fprint(stdout, idx)
	if ho, has, _ := store.ReadHandoff(); has {
		fmt.Fprintf(stdout, "\n--- handoff.md ---\n%s", ho)
	}
	return exitAccepted
}

// missionHook renders the Claude Code hooks (resolved call #3). The two events
// use different vehicles because Claude Code's hook contract honors them
// differently:
//
//   - session-start: SessionStart's additionalContext IS honored (and also fires
//     after compaction with source "compact"), so we emit hookSpecificOutput JSON
//     carrying the mc index + latest handoff + a "check the last handoff before
//     responding" wake-up. This is where the disk handoff re-enters context.
//   - pre-compact: PreCompact honors ONLY a top-level decision/reason — its
//     additionalContext is silently dropped, so emitting it is a no-op that never
//     reaches the summary. Instead we use the disk vehicle: write a mechanical
//     "compaction imminent" handoff to the mc store (trigger: pre-compact), which
//     the next SessionStart (source "compact") then injects. The CC-produced
//     summary itself carries the conversation; this handoff is metadata only. We
//     emit no JSON — nothing for CC to misparse — and always exit 0.
//
// Both are best-effort — a hook failure must never block the session.
func missionHook(store *mc.Store, botName string, args []string, stdout, stderr io.Writer) int {
	if len(args) < 1 {
		return usageErr(stderr, "hotline mission hook session-start|pre-compact")
	}
	// P2-A: honor the opt-out. The CC hooks stay installed in settings.json even
	// when MC is disabled, so they must become clean no-ops (exit 0, no output, no
	// writes, no dir creation) rather than silently defeating HOTLINE_MISSION_CONTROL=0.
	// The operator verbs (note/update/handoff/archive/show) intentionally ignore
	// this gate — they run before this branch and can always touch the files.
	if mcCfg, err := config.MissionControlForBox("claude", botName); err == nil && !mcCfg.Enabled {
		return exitAccepted
	}
	switch args[0] {
	case "session-start", "SessionStart":
		return emitHookContext(stdout, "SessionStart", sessionStartContext(store, botName))
	case "pre-compact", "PreCompact":
		return missionPreCompactHook(store)
	default:
		return usageErr(stderr, fmt.Sprintf("unknown hook %q (session-start, pre-compact)", args[0]))
	}
}

// preCompactState / preCompactNext are the mechanical "compaction imminent"
// handoff the PreCompact hook writes to disk. It is deliberately metadata-shaped:
// the Claude Code summary carries the actual conversation, and the next
// SessionStart (source "compact") re-injects this so the fresh window knows a
// compaction just happened and where to look.
const (
	preCompactState = "compaction imminent — Claude Code is summarizing this session now; its summary carries the conversation. If anything durable wasn't filed, it may be thinner after this."
	preCompactNext  = "re-read the mission-control index and the thread you left open, then continue where the summary leaves off"
)

// missionPreCompactHook writes the mechanical pre-compact handoff and emits
// nothing (PreCompact's additionalContext is not honored, so JSON here would be
// a silent no-op at best and misparsed at worst). Always exits 0: a hook must
// never block compaction, so even a store write failure is swallowed.
func missionPreCompactHook(store *mc.Store) int {
	_, _ = store.Apply(mc.Input{
		Action:  "handoff",
		State:   preCompactState,
		Next:    preCompactNext,
		Trigger: "pre-compact",
	})
	return exitAccepted
}

// sessionStartContext builds the SessionStart additionalContext: the rendered
// mc index followed by the wake-up instruction. Empty when the store has no
// content (unseeded box).
func sessionStartContext(store *mc.Store, botName string) string {
	mcCfg, err := config.MissionControlForBox("claude", botName)
	budget := config.DefaultMCIndexBudget
	if err == nil && mcCfg.IndexBudget > 0 {
		budget = mcCfg.IndexBudget
	}
	idx := store.RenderIndex(budget)
	instr := "This is your mission-control memory across sessions. Check the last handoff and the thread you left open before responding; file updates with the mission tool as you work."
	if strings.TrimSpace(idx) == "" {
		return instr
	}
	return idx + "\n\n" + instr
}

// emitHookContext writes the Claude Code hook JSON contract: hookSpecificOutput
// with the event name and additionalContext. Empty context still emits valid
// JSON so the hook always succeeds.
func emitHookContext(stdout io.Writer, event, context string) int {
	payload := map[string]any{
		"hookSpecificOutput": map[string]any{
			"hookEventName":     event,
			"additionalContext": context,
		},
	}
	b, err := json.Marshal(payload)
	if err != nil {
		return exitInternal
	}
	fmt.Fprintln(stdout, string(b))
	return exitAccepted
}
