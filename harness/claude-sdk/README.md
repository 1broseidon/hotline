# hotline claude-sdk harness

A **managed edition** of the hotline claude harness, built on the Claude Agent
SDK (`@anthropic-ai/claude-agent-sdk`). It owns the two-way channel loop in our
own code — no `--dangerously-load-development-channels`, no pty, no consent UI.
Shaped like the pi harness: a headless piped node process under `hotline up`
that:

1. spawns one `hotline run` child itself (`HOTLINE_HARNESS=claude-sdk` — the Go
   side's first-class injected-harness branch, `run_claudesdk.go`),
2. receives `notifications/claude/channel` pushes and injects each pre-rendered
   `<channel …>` envelope as a user turn into a long-lived SDK session
   (streaming input mode; the SDK queues turns that arrive mid-response),
3. exposes the child's tools to the agent through an in-process MCP proxy named
   `hotline`, so tool names stay `mcp__hotline__reply` etc.

Why not point the SDK's `mcpServers` at `hotline run` directly: the SDK's MCP
client drops custom server→client notifications (the inbound path), and
`hotline run` holds an exclusive box lease so a second instance cannot run.

The child-management, JSONL JSON-RPC, queue, session, auth, and logging
plumbing is `@1broseidon/hotline-harness-core`, shared with the pi extension —
see `harness/PARITY.md` for the capability matrix and the lockstep rule.

## Delivery guarantee, enforcement, auth containment

The harness is built around a **turn ledger**: every inbound envelope is
uuid-stamped and tracked through the SDK turn that consumes it, so the harness
knows whether a turn actually answered the channel.

- **Delivery guarantee (M1).** An operator turn that ends without a
  `mcp__hotline__reply` call has its buffered assistant text forwarded by a
  fallback lane, so an answer can never silently stay in the box. A conversation
  `(source, chat_id)` stays *awaiting-delivery* across turns until a SUCCESSFUL
  reply lands, so continuation turns are protected too; the lane dedups per
  conversation, and forwards nothing (logging `ambiguous-continuation`) rather
  than cross-talk when it cannot tell which awaiting conversation a turn belongs
  to. Schedule / notify / fleet / internal turns are excluded — no monologue
  leakage. `fallback_count` rides `harness_info` as the lane counter.
- **Reply enforcement (M1.1).** A `Stop` hook is the locked door in front of
  that net: while the ending turn still owes the operator a reply it blocks the
  stop and tells the model to call `mcp__hotline__reply` now — the same
  `activeUncoveredGroups()` predicate the lane uses — capped at two blocks per
  turn, after which the turn ends and the fallback lane delivers. Every settle
  logs one line per outcome (`fired` / `reply-satisfied` / `miss why=…` /
  `blocked-by-hook`).
- **Instruction profile.** Its own uncapped profile — the reply contract, a
  preset neutralizer that names the headless topology, and a Task-tool
  delegation doctrine — plus a per-turn reply-contract reminder injected via
  `UserPromptSubmit`. No pi Agent-tool doctrine.
- **Auth containment (M2).** A dead credential is classified
  (`auth_status` / assistant error / first-turn throw); on the third consecutive
  failure the last operator is notified once through the still-healthy channel
  child, an `auth.fatal` marker is written, and the process exits **5** so the
  supervisor cold-loops (10-minute backoff) instead of respawning forever in
  silence. A successful init clears it.

## Configuration (per-box knobs)

Set in the real environment or the shared state-dir `.env` (real env wins per
key; `hotline up` resolves both and re-exports the result, and re-resolves on
every respawn — edit `.env` + `kill -HUP` the supervisor to apply):

| env | meaning | values |
|---|---|---|
| `HOTLINE_SDK_MODEL` | Agent SDK `Options.model` | model id/alias (e.g. `claude-opus-4-8`); empty = SDK default |
| `HOTLINE_SDK_EFFORT` | thinking depth → `maxThinkingTokens` | `low` (4096) \| `medium` (8192) \| `high` (16384) \| `xhigh` (32768) \| `max` (63999), or a positive integer (raw token budget) |
| `HOTLINE_SDK_MAX_TURNS` | Agent SDK `Options.maxTurns` | positive integer; unset = unlimited |
| `HOTLINE_SDK_FALLBACK` | the M1 delivery lane | default on; disabled by `0` \| `false` \| `off` \| `no` |
| `HOTLINE_SDK_ENFORCE` | M1.1 reply enforcement | `stop-hook` (default) \| `off`; independent of `HOTLINE_SDK_FALLBACK` |
| `HOTLINE_SDK_ATTRIBUTION` | ledger attribution mode | `echo` (default) \| `pull-window`, the conservative fallback if a runtime smoke shows the SDK drops the client-stamped uuid; the boot log states which is live |

Bad values fail `hotline up` loudly at launch. `HOTLINE_CLAUDE_SDK_MODEL` (the
prototype's model env) still works as a deprecated fallback when
`HOTLINE_SDK_MODEL` is unset, with a warning; migrate when convenient.

**`maxTurns` caveat:** with streaming input, hitting the cap ends the query
stream → the harness exits 1 → the supervisor respawns and resumes the session.
It is a safety valve, not a rate limiter; default unset.

harness.log shows the configured knobs (`sdk options: model=… effort=…
maxTurns=…` at boot), the lane posture (`lane: attribution=… fallback=…
enforce=…`), and the truth line once the SDK resolves the session
(`sdk session: model=<resolved> session=<id>`).

## Auth (experimental, pluggable)

The SDK resolves credentials itself; the harness only logs which source wins:
`ANTHROPIC_API_KEY` → `CLAUDE_CODE_OAUTH_TOKEN` → `ANTHROPIC_AUTH_TOKEN` →
stored `claude login` under `HOME`. For Max, either inherit a box
`claude login` via HOME or export `CLAUDE_CODE_OAUTH_TOKEN` (from
`claude setup-token`); treat both as experimental — official programmatic Max
auth is postponed upstream. Keys pinned in the shared `.env` are applied by
`hotline up` via the Anthropic allowlist; the real environment wins per key.

## Build / run

```bash
# Go binary (worktree root)
go build -o ./hotline ./cmd/hotline

# harnesses (npm workspace at harness/ — builds core first, stamps the marker)
cd harness && npm install && npm run build && npm test -w core -w claude-sdk && cd ..

# throwaway box (never the live one)
export HOTLINE_STATE_DIR=$HOME/.config/hotline-sdk-test
./hotline setup && ./hotline pair

# run (harness selection works from the real env or the shared .env,
# like every harness)
export HOTLINE_HARNESS=claude-sdk
export HOTLINE_CLAUDE_SDK_ENTRY=$PWD/harness/claude-sdk/dist/index.js
export ANTHROPIC_API_KEY=sk-ant-…   # or rely on a stored claude login
./hotline up
```

**Lockstep rule** (see PARITY.md): the Go binary and `dist/` ship identity
changes together — always `go build` **and** `npm run build` before a restart.
The build stamps `dist/.hotline-harness`; `hotline up` refuses a stale dist
(missing marker, or one whose child identity disagrees) with a one-line
rebuild fix instead of a confusing ownership refusal at claim time.

The session runs with `bypassPermissions` — every tool runs unguarded, like
`claude --dangerously-skip-permissions`. There is no operator terminal, so a
permission prompt would freeze the session. See SECURITY.md.

## Behavior notes

- **Exit codes:** 0 clean shutdown, 1 stream ended/failed unexpectedly
  (supervisor respawns), 3 fatal child condition (binary missing, box ownership
  conflict), 4 child never became ready within the startup timeout, 5 auth-fatal
  (M2 — credentials dead three times running; the supervisor cold-loops).
- **Session continuity:** the SDK session id is persisted to
  `$HOTLINE_SUPERVISOR_DIR/claude-sdk-session.json` (atomic write) and passed
  as `resume` on respawn; the child's replayCatchup re-delivers messages that
  arrived while the harness was down. A failed resume clears the id and retries
  once without it, handing the doomed attempt's leased turns back to the queue
  and reverting the ledger so they are re-delivered rather than dropped.
- **Child supervision:** `hotline run` is respawned with capped exponential
  backoff; binary-missing and box-ownership conflicts are fatal (exit 3).
  While the child is down, tool calls return an isError "channel is down"
  result to the model.
- **Crash policy:** no inner restart loop — the process exits non-zero and the
  Go supervisor respawns it.
- **Transcript:** `hotline run` logs both directions to transcript.jsonl
  exactly as for every other harness.
- **Other env knobs:** `HOTLINE_SDK_LOG` (append log file),
  `HOTLINE_SDK_HANDSHAKE_TIMEOUT_MS` / `HOTLINE_SDK_CALL_TIMEOUT_MS` (child RPC
  deadlines, defaults 15s / 60s).
- **Job cards:** `task_started`/`task_updated` stream messages for Task-tool
  subagents are bridged to `hotline job start|update|done` (jobcards.ts) —
  live start→done status tiles in the app channel, keyed on the task id.
  `HOTLINE_AUTO_JOBS=0|false|off|no` opts out (shared with pi, which
  documents the knob in its own README).
- **Model catalog:** on child-ready and once per SDK session init,
  `Query.supportedModels()` is mapped to the `harness_catalog` notification
  (catalog.ts) so the app's model row reflects this box's live model list —
  no scope knob (unlike pi's `HOTLINE_PI_MODELS`), every row is offered. A row
  whose id is an alias is labelled with the generation it resolves to.
- **Mission control:** missionControl.ts runs the nudge + context-cap handoff
  loop; a CLI-auto compaction gets a mechanical `PreCompact` handoff rather than
  a model turn, because `PreCompact` is uncancellable.

## Tests

`npm test` runs a hermetic node:test suite (no network, no SDK session): the
knob mapping (options), proxy passthrough (over an in-memory MCP transport),
envelope parsing, the turn ledger and fallback lane, the Stop hook, the auth
watch, the instruction contract, job cards, mission control, the sdk_apply
handler, and a ChildManager integration against the shared fake child
(`../core/test/fake-hotline.mjs`). The generic plumbing suites — JSONL
framing, queue, envelope extraction, session persistence, auth precedence,
spawn-error tables — live in `harness/core`. The live SDK path is validated
manually per the run loop above.
