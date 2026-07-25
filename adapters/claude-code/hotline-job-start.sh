#!/usr/bin/env sh
# hotline FB13 — Claude Code job-card adapter (start).
#
# Wire as a PreToolUse hook matching the subagent tool. When Claude Code
# dispatches a subagent, this opens a rollup job card on your phone: the card is
# keyed on the batch (this session), so N subagents fanned out from one turn show
# as ONE card ("3/5 done"), not five.
#
# Reads the hook payload as JSON on stdin (Claude Code's hook contract) and
# shells out to `hotline job`, which durably enqueues the intent — the box's job
# dispatcher drives the actual card. Every failure is swallowed: a job card must
# never break a tool call. Requires `jq` and `hotline` on PATH.
set -eu

input=$(cat)
emit() { printf '%s' "$input" | jq -r "$1" 2>/dev/null || true; }

tool=$(emit '.tool_name // ""')
case "$tool" in
  Task|Agent) ;;            # the subagent dispatch tool (name varies by build)
  *) exit 0 ;;
esac

# Background dispatches return immediately (PostToolUse fires at launch, not
# completion), so hook-carding them would show instant false "done"s. The
# orchestrator cards background agents' real lifecycle via the CLI itself.
bg=$(emit '.tool_input.run_in_background // false')
[ "$bg" = "true" ] && exit 0

cookie=$(emit '.tool_use_id // ""')
[ -n "$cookie" ] || exit 0
batch=$(emit '.session_id // ""')
title=$(emit '.tool_input.description // .tool_input.subagent_type // "Subagent"')

hotline job start --cookie "$cookie" --batch "$batch" --title "$title" >/dev/null 2>&1 || true
exit 0
