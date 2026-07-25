#!/usr/bin/env sh
# hotline FB13 — Claude Code job-card adapter (done).
#
# Wire as a PostToolUse hook matching the subagent tool. When a (foreground)
# subagent returns, this closes its slot in the batch's rollup card. Once every
# subagent in the batch is terminal the box flips the card to its final state
# (err if any subagent erred, else ok).
#
# Correlates to the start hook by tool_use_id (the same cookie). Background
# subagents (whose PostToolUse fires at dispatch, not completion) are closed by
# the box's lease reaper / restart sweep instead — see README.md. Requires `jq`
# and `hotline` on PATH. Never fails the tool call.
set -eu

input=$(cat)
emit() { printf '%s' "$input" | jq -r "$1" 2>/dev/null || true; }

tool=$(emit '.tool_name // ""')
case "$tool" in
  Task|Agent) ;;
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

# Map an errored subagent result to state=err when the payload exposes it.
state=$(emit 'if (.tool_response.is_error // .tool_response.isError // false) then "err" else "ok" end')
[ -n "$state" ] || state=ok

hotline job done --cookie "$cookie" --batch "$batch" --state "$state" >/dev/null 2>&1 || true
exit 0
