# hotline job cards — Claude Code adapter (FB13)

Automatic job cards on your phone for the subagents Claude Code fans out. When a
turn dispatches subagents, hotline shows a live card that fills in as they
finish — one card per batch, not one per worker:

```
▸ Explore auth code            3/5 done  ▓▓▓▓▓▓░░░░
  running: audit token refresh path
```

The card is driven entirely by two hooks that shell out to `hotline job`. The
box's job dispatcher does the rest (correlation, batch rollup, restart recovery).

## How it works

`hotline job start|update|done --cookie <id>` durably enqueues an intent; the box
drains the queue every ~2s and drives the app-channel job card. Two facts make
it a rollup:

- **cookie** = the subagent's `tool_use_id`. Correlates its start with its done.
- **batch** = the Claude Code `session_id`. Every subagent in one session
  aggregates into ONE card: progress = finished/total, terminal when all are
  terminal (err if any subagent erred, else ok). A lone subagent renders as a
  plain single card.

Nothing is synchronous and nothing can break a tool call — every `hotline`
invocation is best-effort (`|| true`) and the box tolerates a missing start,
duplicate done, or a hook that never fires.

## Install

1. Copy the two scripts onto the box that runs Claude Code and make them
   executable:

   ```sh
   mkdir -p ~/.claude/hooks
   cp adapters/claude-code/hotline-job-*.sh ~/.claude/hooks/
   chmod +x ~/.claude/hooks/hotline-job-*.sh
   ```

   They need `jq` and `hotline` on `PATH`.

2. Add the hooks to your Claude Code `settings.json` (`~/.claude/settings.json`
   or `.claude/settings.json` in the repo). Merge into any existing `hooks`
   block — do not clobber it:

   ```json
   {
     "hooks": {
       "PreToolUse": [
         {
           "matcher": "Task",
           "hooks": [
             { "type": "command", "command": "$HOME/.claude/hooks/hotline-job-start.sh" }
           ]
         }
       ],
       "PostToolUse": [
         {
           "matcher": "Task",
           "hooks": [
             { "type": "command", "command": "$HOME/.claude/hooks/hotline-job-done.sh" }
           ]
         }
       ]
     }
   }
   ```

   `matcher` is the subagent dispatch tool's name. It is `Task` on stock Claude
   Code; some builds (and the Agent SDK) call it `Agent`. The scripts accept
   both, so if your build uses `Agent`, change the matcher string to
   `"Task|Agent"` (matchers are regular expressions).

3. Pair a device (`hotline relay`) so there is somewhere to show the card, and
   run your session under a hotline box as usual.

## What each hook does

| Hook | Event | Action |
|------|-------|--------|
| `hotline-job-start.sh` | `PreToolUse` [Task] | `hotline job start --cookie <tool_use_id> --batch <session_id> --title <description>` |
| `hotline-job-done.sh`  | `PostToolUse` [Task] | `hotline job done --cookie <tool_use_id> --batch <session_id> --state ok\|err` |

## Known gaps (backstopped by the box, not the hook)

- **Background subagents.** For a subagent launched to run in the background,
  `PostToolUse` fires at *dispatch*, not completion, so its done can land early.
  The card still resolves: the box's **lease reaper** closes any card that sits
  running past its lease (default 30 min) as `err "tracking lost"`.
- **User interrupt.** If a turn is interrupted, `PostToolUse` may not fire at
  all. Same backstop — the reaper closes it.
- **Box restart mid-flight.** The in-memory card registry dies with the process,
  but the card *message* persists. On the next boot the dispatcher sweeps every
  still-running card to `err "lost on restart"` from `activejobs.json`, so a card
  never freezes spinning forever.

`SubagentStop` looks like the natural background-close hook, but its payload
carries no `tool_use_id`, so it cannot be correlated back to the start cookie
without an assumption that is not yet settled. It is deliberately not wired here;
the reaper covers the gap instead.
