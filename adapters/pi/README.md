# hotline job cards — Pi adapter (BUILT)

> Built. The box side (`hotline job` CLI + dispatcher) and this Pi adapter both
> ship. Lives inside the existing Pi channel extension — no separate process.
> Implementation: `harness/pi/src/jobcards.ts` + wiring in
> `harness/pi/src/index.ts` (repo `mcp/hotline`, branch `feat/app-channel`).

## Event source

Pi's subagents come from `@tintinweb/pi-subagents`, a separate user-installed pi
extension that emits subagent lifecycle events on the shared `pi.events` bus. The
hotline Pi extension subscribes there. Event names + payload shapes were verified
against **@tintinweb/pi-subagents@0.14.1** (`dist/index.js`):

| Bus event | Payload (fields we read) | Src | Job intent |
|-----------|--------------------------|-----|-----------|
| `subagents:started`   | `{id, type, description}` | index.js:385 | `job start --cookie <id> --batch <session id> --title <description\|type>` |
| `subagents:compacted` | `{id, reason, …}`         | index.js:392 | `job update --cookie <id> --detail "compacting context (<reason>)"` |
| `subagents:completed` | `{id, status, …}`         | index.js:354 | `job done --cookie <id> --batch <session id> --state ok` |
| `subagents:failed`    | `{id, status, …}`         | index.js:351 | `job done --cookie <id> --batch <session id> --state err\|cancelled` |

Notes from verification:

- **There is no `subagents:progress` event.** `subagents:compacted` is the only
  mid-run signal on the bus, so it is routed to `job update` (doubles as a
  heartbeat that keeps the card warm against the box lease reaper). Optional.
- `subagents:failed` fires for status `error | stopped | aborted` (index.js:351).
  The adapter maps `aborted`/`stopped` → `cancelled` and everything else → `err`
  (the box CLI accepts `ok|err|cancelled`).
- All terminal statuses — including abort — fire on this bus, so there is no
  interrupt gap to backstop (unlike Claude Code).

## Correlation

- **cookie** = the subagent record id (`data.id`). One card per subagent.
- **batch** = the pi **session id** (`ctx.sessionManager.getSessionId()`).
  pi-subagents *does* have a group-join primitive, but it keeps its group id
  (`record.groupId` / internal `batch-N`) **off** the emitted lifecycle payloads,
  so the session id is the stable per-run fallback. `--batch` is **always** passed
  explicitly on start AND done — `hotline job done` without `--batch` fails to
  resolve the cookie to its rollup and mis-cards (box bug, 2026-07-16).

## Contract

Same job-file contract as every adapter: shell out to `hotline job`, one call per
lifecycle transition, keyed on a stable `--cookie`. Routing through the CLI (vs
the in-memory `job` MCP tool) buys durability and restart self-heal for free and
keeps one code path across harnesses.

Best-effort by construction: every call is a **detached** spawn with stdio
discarded, its `error` swallowed and exit code ignored, so a failed or missing
`hotline` binary never disturbs the Pi run. The `hotline` binary is resolved from
PATH at runtime, overridable via `HOTLINE_BIN`.

## Scope / limitations

- Only fires for subagents dispatched through **@tintinweb/pi-subagents** (its
  `Agent` tool with `run_in_background`). The hotline-vendored `subagent` tool
  (`harness/pi/src/subagent.ts`) spawns child pi processes directly and does
  **not** emit bus events, so it is not carded by this adapter.
- Wired only on the top-level bot session (depth 0). A depth>0 worker returns
  before the channel boots, and its own subagents run in a separate pi process
  on a separate bus — so there is no double-carding.
- Opt out with `HOTLINE_AUTO_JOBS=0` (also `false`/`off`/`no`).
- Because batch = session id, every subagent in one bot session rolls up into one
  card. A tighter per-fan-out batch key would need pi-subagents to emit its group
  id on the lifecycle payloads (it currently does not).

## Tests

`harness/pi/test/run-unit.mjs` Part 6 covers the event→CLI mapping end to end:
pure argv builders + title/state mappers, plus a fake `pi.events` bus and an
injectable `spawn` asserting the emitted argv for each lifecycle event, the
"ignore events with no id", the "throwing spawn never escapes", and dispose.
