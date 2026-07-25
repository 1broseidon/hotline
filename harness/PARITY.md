# Harness parity matrix

CONTRIBUTING: this table is the drift ledger for hotline's harnesses. Any
change to a harness capability — adding a knob, moving plumbing into or out of
`@1broseidon/hotline-harness-core`, changing spawn/session/inbound behavior —
MUST update its row **in the same commit**. `harness/core`'s test run enforces
the mechanical half (`scripts/check-parity.mjs`): the pi copies of the shared
modules must not reappear, and every core module must keep a row here.

Cells: `core` = provided by harness-core, `native` = harness's own
implementation, `n/a` = does not apply, `TODO` = known gap. The `claude-sdk`
column describes the single package at `harness/claude-sdk` (0.2.0) — the
0.1 implementation and its `claude-sdk-v2` staging directory were collapsed
into it; there is one claude-sdk harness and it is this one.

| capability | `claude` (TUI) | `claude-sdk` | `pi` | `opencode` |
|---|---|---|---|---|
| child spawn/respawn/backoff (child) | core¹ | core | core | n/a — `opencode serve` is supervisor-spawned |
| JSONL JSON-RPC framing (jsonrpc) | n/a — MCP loader | core | core | n/a — HTTP+SSE |
| inbound injection (inbound) | native channel notif | core queue → SDK stream; uuid-stamped + envelope parsed at enqueue → turn ledger | core content → pi.sendUserMessage | SSE adapter |
| fleet inbound framing (trust marker / directive) | content-borne² | content-borne² | content-borne² | content-borne² |
| delivery guarantee (fallback lane) | native channel binding | native (turn ledger attributes each turn; a conversation stays awaiting-delivery across epochs until a SUCCESSFUL reply lands, so continuation turns are protected too; an uncovered operator turn's buffered text is forwarded via reply, deduped per conversation — `HOTLINE_SDK_FALLBACK`, default on) | n/a — reply is primary voice | n/a |
| reply enforcement (stop hook) | native (contract as segment #1 + channel binding) | native (Stop hook blocks a turn's end while it still owes the operator a reply — same `activeUncoveredGroups()` predicate the lane uses; ≤2 blocks/turn, then the fallback lane delivers — `HOTLINE_SDK_ENFORCE=stop-hook\|off`, default stop-hook, independent of `HOTLINE_SDK_FALLBACK`) | n/a — reply is primary voice | n/a |
| async queue (queue) | n/a | core (AsyncQueue feeds `query()` streaming input) | n/a — pi delivers via deliver.ts ladder | n/a |
| permission relay | native | off-by-design (bypassPermissions) | off-by-design | opencode.json policy |
| session continuity (session) | native CC session | core session file + `resume` | `--session-id` (deterministic, no file) | OPENCODE_SESSION |
| auth resolution report (auth) | native login | core (`resolveAuth`, report-only) | n/a — pi provider auth | n/a |
| auth containment | native (login prompt) | native (authwatch: classify auth_status/assistant-error/throw; notify last operator once; escalate at 3 consecutive failures via exit 5 + `auth.fatal` marker → supervisor pins backoff at Max until a successful init clears it) | n/a — pi provider auth | n/a |
| logging (log) | native | core `createLog("hotline-sdk", "HOTLINE_SDK_LOG")` | core `createLog("hotline-pi", "HOTLINE_PI_LOG")` | native |
| model knob | TUI-picked | `HOTLINE_SDK_MODEL` | `HOTLINE_PI_{PROVIDER,MODEL,THINKING}` | opencode.json |
| effort knob | n/a | `HOTLINE_SDK_EFFORT` | `--thinking` | n/a |
| max turns | n/a | `HOTLINE_SDK_MAX_TURNS` | n/a | n/a |
| wire metadata harness/model/effort | kind only | kind+model+effort (+fallback_count, the lane counter) | kind+model-if-knob | kind only |
| instruction budget | capped voice | uncapped, claude-sdk profile (reply contract → preset neutralizer → Task-tool doctrine; reply contract re-injected per turn via UserPromptSubmit) | uncapped | uncapped |
| `up --background` | refused — tmux | allowed | allowed | allowed |
| job cards | native (hotline CLI) | native (jobcards.ts, task_started/task_updated → hotline job) | native (jobcards.ts auto-wire) | TODO |
| mission control | native (mc tools) | native (missionControl.ts: nudge + cap loop; CLI-auto compaction gets a mechanical PreCompact handoff, not a model turn — PreCompact is uncancellable) | native (missionControl.ts handoff loop) | TODO |
| model catalog (app model row) | n/a — TUI-picked | native (catalog.ts, supportedModels() → harness_catalog; no scope knob) | native (modelcatalog.ts, HOTLINE_PI_MODELS scope → harness_catalog) | TODO |
| restart tool | native (supervisor dir) | native (supervisor dir) | native (supervisor dir) | native (supervisor dir) |

¹ the claude TUI child is the supervisor's pty spawn (`cmd_up.go`), not
harness-core's ChildManager; "core" here marks that respawn/backoff exists,
owned by the Go supervisor.

² no harness owns this and none may: the box prepends the preamble to the
CONTENT at its single injection point (`internal/app/fleetinject.go`,
`injectInbound`), so every harness sees the identical text and none can drop,
weaken, or forge it. Two preambles exist and they are mutually exclusive per
frame — `fleetTrustMarker` (untrusted peer data — the default for every frame on
every edge) and the F1 `fleetDirectiveMarker` (orchestrator directive), used only
when the OPERATOR granted authority on that edge (`hotline fleet grant`, bound to
the peer's pinned key, TTL-able, revocable) AND the frame's kind is in the closed
down vocabulary `task|cancel|status_req`. Harnesses must never re-frame, strip, or
synthesize either marker.

## Lockstep rule (claude-sdk)

The Go side (`config.Harness()`, `cmd_up.go`, `run_claudesdk.go`) and the TS
harness (`harness/claude-sdk`) ship identity changes in the same commit, and a
restart is always preceded by BOTH `go build` and `npm run build` (from
`harness/`). The `dist/.hotline-harness` marker written by the claude-sdk build
lets `hotline up` refuse a stale dist instead of failing later with an
ownership mismatch.
