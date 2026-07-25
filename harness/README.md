# Harnesses

hotline is the messaging channel for your coding agent. You bring the harness.
This folder is the map: which harness, what you get, where its wiring lives.

hotline drives four today: Claude Code, claude-sdk, OpenCode, and Pi — the four
values `HOTLINE_HARNESS` accepts (`internal/config`.`HarnessValues`); an unknown
one is rejected at startup. The texting experience is the same on all four. They
differ in how you wire them up, where their artifacts live, and how much they
can relay to your phone.

## What each harness gives you

| | Claude Code | claude-sdk | OpenCode | Pi |
|---|---|---|---|---|
| wire-up | `hotline init` (plugin) | build `claude-sdk/`, point `HOTLINE_CLAUDE_SDK_ENTRY` at its `dist/index.js` | `hotline init --harness opencode` | `pi install ./harness/pi` |
| two-way texting (bubbles, buttons, photos, reactions, edits) | yes | yes | yes | yes |
| always-on (`hotline up`, crash restart, lossless) | yes | yes | yes | yes |
| loops + schedules | yes | yes | yes | yes |
| permission relay to your phone | yes | no — runs with permissions bypassed, by design | no (coarse model) | not yet — runs unguarded; relay is the planned next step |
| subagents | built in (Task) | built in (Task) | built in | starter pack ships with the extension |
| voice override (HOTLINE.md) | yes | yes | yes | yes |
| headless / no TTY | pty under supervisor | yes | yes | yes |

The claude-sdk and Pi permission cells are honest and loud on purpose: neither
gates its tools on your phone today. A Pi session runs unguarded, and claude-sdk
runs the Agent SDK in `bypassPermissions`. `hotline up` prints the same warning.

[`PARITY.md`](./PARITY.md) is the capability-by-harness drift ledger and carries
the same four harnesses at much finer grain. If this table and PARITY.md ever
disagree, PARITY.md is right — it is the one a harness change is required to
update in the same commit.

## Claude Code

No folder here. Claude Code speaks the channel protocol natively, so its
shippable artifact is the plugin, not an extension in this tree. `hotline init`
wires it up.

- Plugin and marketplace root: [`plugins/hotline/`](../plugins) and
  `.claude-plugin/` at the repo root. These paths are load-bearing for the
  marketplace manifest and do not move.
- Protocol reference: [`site/docs/protocol.html`](../site/docs/protocol.html).

## claude-sdk

Claude again, but managed: a headless node harness that drives a Claude Agent
SDK session and owns the `hotline run` child itself, instead of Claude Code
loading hotline as an MCP server. One folder, [`claude-sdk/`](./claude-sdk), at
0.2.0 — the turn-ledger delivery guarantee, reply enforcement, and auth
containment.

There is no registry install and no PATH lookup — the harness is repo-local, so
you build it and name the entry point:

```
cd harness/claude-sdk && npm install && npm run build
export HOTLINE_CLAUDE_SDK_ENTRY=<repo>/harness/claude-sdk/dist/index.js
HOTLINE_HARNESS=claude-sdk hotline up
```

`hotline init --harness claude-sdk` prints those same steps. The Go side and the
TS harness ship identity changes in the same commit — see the lockstep rule at
the end of [`PARITY.md`](./PARITY.md).

## OpenCode

No folder here either. OpenCode's artifact is generated, not stored: a dedicated
`hotline` agent plus an `opencode.json` merge, both scaffolded by

```
hotline init --harness opencode
```

The generator lives in the Go binary (`cmd/hotline/cmd_init.go`); a folder here
would be an empty shrine. See the root README's OpenCode section for the wiring
it writes.

## Pi

Ships files, in [`pi/`](./pi).

```
pi install ./harness/pi
```

That one command puts the channel extension on disk, from a checkout of this
repo — the extension is not published to a registry, same as hotline itself.
`harness/pi/` is the whole package: the channel extension in `src/` (which
co-loads a `subagent` tool), a starter agent pack in `agents/`, the
`hotline-setup` skill in `skills/`, tests in `test/`, and its own README with the
full Pi quickstart. Install docs live in [`pi/README.md`](./pi/README.md).

## core/ is not a harness

[`core/`](./core) is the third subfolder and the odd one out: it is the shared
TS library — child management, JSONL JSON-RPC, inbound extraction, queue,
session, auth, logging — that claude-sdk and pi both build on.
Counting it as a harness gets you five; there are four. `harness/package.json`
is the npm workspace tying the three packages together for local development.
None of them is published: they all carry `"private": true` and are built from
source in this checkout.

