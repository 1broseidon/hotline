# @1broseidon/hotline-claude-sdk-v2

The hotline Claude Agent SDK harness, 0.2.0 rebuild. Built clean beside the 0.1
harness (`harness/claude-sdk`); the child identity stays `claude-sdk`, so the Go
side selects it exactly as before.

`hotline up` spawns this node child via `HOTLINE_CLAUDE_SDK_ENTRY`. It drives a
long-lived Claude Agent SDK session, injecting inbound `<channel>` envelopes as
user turns and proxying hotline's tools into the session over an in-process MCP
server.

## What 0.2.0 adds

- **Turn ledger + delivery guarantee (M1).** Every inbound envelope is
  uuid-stamped and tracked through the SDK turn that consumes it. An operator
  turn that ends without a `mcp__hotline__reply` call has its buffered text
  forwarded by a fallback lane, so an answer can never silently stay in the box.
  Schedule / notify / fleet turns are excluded (no monologue leakage).
- **claude-sdk instruction profile (M1).** Its own uncapped profile — the reply
  contract, a preset neutralizer that names the headless topology, and a
  Task-tool delegation doctrine — plus a per-turn reply-contract reminder via
  `UserPromptSubmit`. No pi Agent-tool doctrine.
- **Auth containment (M2).** A dead credential is classified, the last operator
  is notified once, and the supervisor is told to cold-loop (10-minute backoff)
  instead of respawning forever in silence.

## Running a box under 0.2.0

```sh
export HOTLINE_CLAUDE_SDK_ENTRY="$(pwd)/dist/index.js"   # from harness/claude-sdk-v2
hotline up --harness claude-sdk
```

## Knobs (0.2.0)

- `HOTLINE_SDK_FALLBACK` — the M1 delivery lane; default on, disabled by
  `0|false|off|no`. A conversation stays *awaiting-delivery* across turns until a
  successful reply lands, so continuation turns are forwarded too (deduped per
  conversation).
- `HOTLINE_SDK_ENFORCE` — M1.1 reply enforcement; `stop-hook` (default) installs
  a `Stop` hook that blocks a turn from ending while it still owes the operator a
  reply (≤2 blocks/turn, then the fallback lane delivers), `off` removes it.
  Independent of `HOTLINE_SDK_FALLBACK`.
- `HOTLINE_SDK_ATTRIBUTION` — `echo` (default) or `pull-window` (the
  conservative attribution fallback if a runtime smoke shows the SDK drops the
  client-stamped uuid; boot log states which is live).
- The existing `HOTLINE_SDK_*` family (`MODEL`, `EFFORT`, `MAX_TURNS`,
  `SETTING_SOURCES`, `LOG`, `*_TIMEOUT_MS`) is unchanged.

## Build

```sh
npm run build     # tsc + write the dist/.hotline-harness lockstep marker
npm test
```
