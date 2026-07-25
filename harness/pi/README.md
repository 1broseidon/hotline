# @1broseidon/hotline-pi

The **hotline channel extension for Pi**. It lets you run [hotline](https://github.com/1broseidon/hotline)
on top of a [Pi](https://pi.dev) session, so you can drive your Telegram / Discord / Signal
bot straight from Pi. Pi is one of hotline's four harnesses, next to Claude Code, claude-sdk, and OpenCode.

The extension spawns `hotline run` as a child process, bridges its stdio to the Pi
session, exposes hotline's tools (`reply`, `react`, `edit_message`, `download_attachment`,
`publish`, …) as first-class Pi tools, and injects inbound messages as user turns. There is
no MCP anywhere on the Pi side — the bridge is a small hand-rolled JSONL JSON-RPC client
with zero runtime dependencies.

The same package also ships a **starter agent pack** and the **`hotline-setup` skill**
(see below). The skill loads with the channel; the agent pack is content for the
[`pi-subagents`](https://github.com/tintinweb/pi-subagents) plugin (see the loadout below).

## Install

**You need the `hotline` binary on PATH first.**

```sh
curl -fsSL https://hotline.dev/install.sh | sh   # or: brew install 1broseidon/tap/hotline
hotline setup                                    # paste your bot token
```

Then install the extension into Pi from a checkout of this repo (writes to user settings,
so it works in headless `pi --mode rpc` with no trust prompt):

```sh
git clone https://github.com/1broseidon/hotline
cd hotline
pi install ./harness/pi
```

The extension is not published to npm — hotline distributes one signed binary and
everything else is built or installed from source.

Run Pi and text your bot:

```sh
pi
# text the bot from Telegram, then approve the pairing from your terminal:
hotline pair <code>
```

Or let the bundled skill walk you through it end to end (binary, token, pairing, agent
pack, smoke test):

```
run the hotline-setup skill
```

## Recommended loadout

The stock channel works out of the box. For the best experience, add the pieces we run
ourselves:

**Async subagents, via [`pi-subagents`](https://github.com/tintinweb/pi-subagents).**
This package ships only the channel bridge; async delegation comes from the plugin. It gives
you Claude Code style background delegation: the bot keeps answering you while workers run,
and posts each result when it lands, through its `Agent` tool.

```sh
pi install npm:@tintinweb/pi-subagents
```

One thing to set: add `run_in_background: true` to each agent's frontmatter in
`~/.pi/agent/agents/*.md`. The plugin runs agents in the foreground by default, and a
foreground call blocks the turn. This line makes delegation background per agent, with no
model discretion needed. The starter pack (below) already ships with it set.

```sh
hotline up --foreground -- -e ./src/index.ts --provider zai --model glm-5.2:high
```

Restart the session after editing agent files; agents are snapshotted at start. Your
existing agent Markdown files work under the plugin as they are (it keys the name off the
filename, same format).

**Web search, via `@ollama/pi-web-search`** (optional). Gives researcher-type agents a real
search tool:

```sh
pi install npm:@ollama/pi-web-search
```

### Operator mode (`hotline up`)

Set `HOTLINE_HARNESS=pi` in hotline's `.env` and run `hotline up`. The supervisor launches
`pi --mode rpc --session-id <stable>` and this extension takes it from there — the child,
the pollers, the scheduler, loops, and restart-on-crash all carry over from the other
harnesses. Pin the provider/model there too (see the knob vars below).

> Heads up: on pi, hotline runs its tools without a per-action permission prompt to your
> phone. `hotline up` prints the same warning. Run it only where you are comfortable with that.

## How it works

- **`session_start`** spawns `hotline run` (with `HOTLINE_HARNESS=pi` and full env
  passthrough, including `HOTLINE_SUPERVISOR_DIR`), runs the MCP initialize handshake over
  its stdio, lists its tools, and registers each with `pi.registerTool` (raw JSON Schema
  passed through verbatim). The uncapped instruction block from the initialize result is
  appended to the system prompt each turn via `before_agent_start`.
- **Inbound** `notifications/claude/channel` frames carry a pre-rendered `<channel …>`
  envelope; the extension forwards the text to `pi.sendUserMessage` (steering when the
  agent is mid-stream), which fires a turn.
- **Outbound** tool calls are proxied to the child as `tools/call`.
- **`session_shutdown`** kills the child. A child that crashes is respawned with capped
  exponential backoff; a missing binary or a poller-slot conflict (another session already
  polling the same bot) is surfaced loudly and not retried.
- **Mission Control compaction insurance** is disk-first. Pi 0.80.6 does not let a
  `session_before_compact` listener extend the built-in summarizer instructions, so pi-owned
  compactions rely on the agent-authored handoff or the extension's mechanical disk fallback.
  Cap-triggered recompaction does receive the handoff-summary instruction through the supported
  direct `ctx.compact({ customInstructions })` path.

All extension logging goes to stderr (and, if `HOTLINE_PI_LOG` is set, an append-only
file) — never stdout, which in `--mode rpc` is the JSONL event stream.

## Subagents

Async delegation is provided by the [`pi-subagents`](https://github.com/tintinweb/pi-subagents)
plugin (see the loadout above), not this package. This package ships a **starter agent pack**
as content for it: plain agent-definition Markdown the plugin loads by filename.

### Starter agent pack

Four starter agents ship with the package:

- **architect** — read-only design decisions (system design, API boundaries, tradeoffs); proposes designs for implementer to execute.
- **researcher** — web research, returns a tight cited summary.
- **scout** — read-only local recon (find files, read code, summarize configs).
- **implementer** — real changes (edits, refactors, builds), reports what it did.

Each is a Markdown file with frontmatter (`name`, `description`, optional `tools`, `model`,
and `run_in_background: true` so the plugin delegates in the background). Install them (the
`hotline-setup` skill does this for you, or run it directly):

```sh
harness/pi/skills/hotline-setup/scripts/install-agents.sh                    # all four
harness/pi/skills/hotline-setup/scripts/install-agents.sh researcher scout   # a subset
```

It copies to `~/.pi/agent/agents/`, never overwrites your own agents (a conflicting name
lands as `hotline-<name>` instead), and is safe to re-run. The templates pin no model, so
each agent inherits pi's default; the skill can pin one per agent.

> Note: the plugin snapshots the installed agents when pi starts. Agents installed
> mid-session are picked up after a restart.

## The `hotline-setup` skill

The package ships a Pi skill that walks an operator through setup one step at a time:
binary install, bot token, extension health, the starter agent pack, pairing, a smoke
test, and optional always-on mode. Invoke it from a Pi session with `run the hotline-setup
skill`. It checks each step's state first, so it is safe to re-run mid-way.

### Environment

| Var | Effect |
|-----|--------|
| `HOTLINE_BIN` | Override the `hotline` binary path (default: `hotline` on PATH). |
| `HOTLINE_PI_LOG` | Absolute path to also append extension logs to. |
| `HOTLINE_HARNESS` | Forced to `pi` for the child (selects hotline's pi branch). |
| `HOTLINE_SUPERVISOR_DIR` | Passed through from `hotline up`; enables the `restart` tool. |
| `HOTLINE_PI_PROVIDER` | `hotline up`: pin the pi provider (a `-- --provider` passthrough still wins). |
| `HOTLINE_PI_MODEL` | `hotline up`: pin the pi model (a `-- --model` passthrough still wins). |
| `HOTLINE_PI_THINKING` | `hotline up`: pin the pi thinking level (a `-- --thinking` passthrough still wins). |
| `HOTLINE_PI_MODELS` | `hotline up`: comma-separated patterns scoping Ctrl+P model cycling (`--models`; globs like `anthropic/*` and `*sonnet*` allowed, plus a `:level` suffix). A `-- --models` passthrough still wins, and the box re-exports whichever won — this extension reads it back to enumerate the SAME list for the app's model row, since pi's resolved cycling list is not on the extension surface. Unset: the app is offered every model with auth configured. |
| `HOTLINE_PI_SESSION` | `hotline up`: override the stable `--session-id` used for memory across restarts. |
| `HOTLINE_MISSION_CONTROL` | Mission Control memory: unset or `1` enables it; `0` disables it. |
| `HOTLINE_MC_DIR` | Override the Mission Control state directory (default: `<box-root>/mc`; named boxes use `<state-root>/bots/<name>/mc`). |
| `HOTLINE_MC_INDEX_BUDGET` | Byte cap for the injected Mission Control index (default: `4096`). |
| `HOTLINE_MC_CONTEXT_CAP` | Requested soft token cap for early handoff and compaction. It must exceed pi's effective `compaction.keepRecentTokens`; unsafe values are warned and raised to `keepRecentTokens + 4096` (default pi settings make the minimum effective cap `24096`). |

## Security note

One deliberate posture choice worth knowing:

- **The pi harness runs tools unguarded.** There is no per-action permission relay to your
  phone yet (unlike the Claude Code harness). Run the bot only where that is acceptable.

## Development

```sh
npm run typecheck     # tsc --noEmit
npm test              # JSONL client framing + inbound delivery ladder
npm run gen:goldens   # regenerate test/goldens.json from the real hotline binary
bash test/smoke.sh    # integration vs real pi --mode rpc + the fake child
```

`test/fake-hotline.mjs` speaks exactly the JSON-RPC frames the Go child emits (initialize
caps, verbatim tool schemas, the `<channel …>` envelope golden), captured in
`test/goldens.json` from the real binary, so the client is tested against the true wire
shape without a bot token.
