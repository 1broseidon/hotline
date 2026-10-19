# Error handling and reachable panics — 0.40 audit

Read-only audit of `375322cc` on `release/0.40.0`, 2026-10-17, task-105.
Recommendations below await review; no runtime, wire, or tape changes are made here.

## Findings

**1. High priority, small/medium effort: desktop startup turns recoverable failures into panics.**
`crates/hotline-app/src/lib.rs:351`, `crates/hotline-app/src/lib.rs:355`,
`crates/hotline-app/src/lib.rs:359`, and `crates/hotline-app/src/lib.rs:570`
expect Desk opening, Remote opening, loopback binding, and Tauri building to succeed.
These are four fallible operations, not four invariants. In particular,
`crates/hotline-core/src/remote/mod.rs:371` decodes `remote.json` with `?`:
a truncated JSON file becomes an ordinary I/O error, then the desktop panics.
`crates/hotline-core/src/wire/mod.rs:927` binds a real socket and can fail under
descriptor exhaustion or OS restrictions. Disk permissions, credential-store
failures and shell/window setup can likewise prevent startup.
These paths are source-traced, not destructive tests against a running desk.
Return a startup error to one outer handler, with a short message such as
“Hotline couldn't start” and selectable, sanitized details. Exit unsuccessfully;
do not silently start an empty room or regenerate unreadable settings.

**2. Medium priority, small effort: model discovery's HTTP constructor is fallible.**
`crates/hotline-core/src/providers/discovery.rs:95` expects a reqwest client to
build because its configuration is valid. Valid timeout constants do not prove
the TLS backend/system resolver can initialize. The pinned reqwest 0.13.4
`ClientBuilder::build` documents those failures in its local source.
`crates/hotline-core/src/providers/discovery.rs:219`,
`crates/hotline-core/src/providers/discovery.rs:249`, and
`crates/hotline-core/src/providers/discovery.rs:293` construct it while listing
models; a setup failure unwinds that task instead of returning the existing
discovery refusal. This is an environment-dependent failure, not evidence that
an arbitrary model-list response panics. Replace `Default` with a fallible
constructor and propagate a short discovery error; keep the previous list.

**3. Medium priority, small effort: CLI OS setup assumes success.**
`crates/hotline-cli/src/door.rs:173` expects Tokio runtime creation;
`crates/hotline-cli/src/serve.rs:271` and `crates/hotline-cli/src/serve.rs:272`
expect signal registration; `crates/hotline-cli/src/serve.rs:282` expects the
OS entropy source. These four `expect`s describe desired operating conditions,
not guarantees. Resource exhaustion and OS failures must return errors to the
CLI's unsuccessful exit path. Never replace a failed token generation with a
constant or weak fallback. The signal setup should fail startup explicitly,
rather than panic after the listener has already been started.

**4. Medium priority, small batches: lock poisoning has two incompatible treatments.**
There are **58 production `.lock().unwrap()` sites**, 53 in core's `remote/`
and five in `crates/hotline-app/src/transfer.rs:46`,
`crates/hotline-app/src/transfer.rs:58`, `crates/hotline-app/src/transfer.rs:183`,
`crates/hotline-app/src/transfer.rs:206`, and `crates/hotline-app/src/transfer.rs:227`.
Examples in core are `crates/hotline-core/src/remote/mod.rs:455`,
`crates/hotline-core/src/remote/relay.rs:208`,
`crates/hotline-core/src/remote/bridge.rs:217`, and
`crates/hotline-core/src/remote/admission.rs:195`.
The last is in `Permit::drop`: a second panic while unwinding can abort the
process, and unwinding before `released.cancel()` also defeats cleanup.
This is conditional propagation after an earlier panic; no independent
unauthenticated poison trigger was demonstrated.
The existing house pattern is a local `lock` helper using
`unwrap_or_else(PoisonError::into_inner)` at
`crates/hotline-core/src/session/mod.rs:5474`; credentials and the log already
use the same recovery directly (`crates/hotline-core/src/log/mod.rs:212`).
Use that one acquisition pattern consistently, including destructors, after
checking each protected mutation. Recovering the guard does not repair a
half-updated authorization record: cancel/revalidate affected state before
admitting further work if its invariants could have been interrupted. Do not
invent a poisoned-lock UI error at every call site or blindly resume grants.

**Inventory and counting method.**
The primary scope is all 195 Rust files under the three crates' `src/`,
including ordinary binary targets and all platform branches. Exclude `tests/`,
`tests.rs`, `*_tests.rs`, and every item whose `cfg` is false with `test=false`
(including `cfg(all(test, unix))`). Preserve unknown platform/feature predicates;
`cfg(not(test))` remains production. Do not truncate a file at its first test
module: production functions follow one in `crates/hotline-core/src/wire/mod.rs:2187`.
Parse with tree-sitter 0.25.2 / tree-sitter-rust 0.24.2, discard excluded AST subtrees,
then count method calls named exactly `unwrap`/`expect`, macro invocations named
`panic`/`unreachable`, and index expressions. Scan macro token trees separately:
they contain four of the `expect` sites and **46 additional index expressions**.
Manually check postfix bracket uses there, excluding JSON array literals,
attributes, strings, comments, array types and patterns. Count source sites,
not executions or macro expansion multiplicity; nested `v["a"][0]` counts twice.
`unwrap_or`, `unwrap_or_else`, `get`, assertions, implicit library panics,
allocation failure, and arithmetic overflow are outside these syntax totals.

| Shipping source | `unwrap()` | `expect()` | `panic!` | `unreachable!` | Indexing | Total |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| hotline-core/src | 72 | 85 | 1 | 5 | 533 | 696 |
| hotline-app/src | 6 | 5 | 0 | 0 | 8 | 19 |
| hotline-cli/src | 6 | 4 | 0 | 0 | 21 | 31 |
| **Total** | **84** | **94** | **1** | **5** | **562** | **746** |

Of the 184 non-index sites, 58 are lock poisoning, nine are the reachable
fallible-setup assertions above, and 117 have an internal invariant. The 562
index expressions were screened for bounds, UTF-8 boundaries, receiver type,
and construction provenance. No unguarded input-triggered index panic was
established. This is static review, not exhaustive fuzzing or a proof that
dependencies cannot panic. OS-specific paths were read, not run on every OS.

For reconciliation, the following groups partition all 696 core sites. A row
includes both `foo.rs` and `foo/`; zero-site modules are omitted. I = invariant
or guarded access; L = lock poisoning; E = fallible external setup.

| Core group | unwrap | expect | panic | unreachable | index | Classification |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| bin | 1 | 0 | 0 | 0 | 0 | I: literal MCP echo schema |
| capabilities | 0 | 0 | 0 | 0 | 2 | I: settings defaults |
| computer | 0 | 4 | 0 | 0 | 21 | I: response construction, piped child, JSON reads |
| credentials | 0 | 0 | 0 | 0 | 1 | I: bounded chunk range |
| desk | 0 | 0 | 0 | 0 | 2 | I: settings defaults |
| driver | 0 | 4 | 0 | 1 | 20 | I: owned enum/state, bounded output, immutable env |
| imagegen | 0 | 0 | 0 | 0 | 32 | I: JSON reads, fixed avatar arrays, sniff guards |
| images | 0 | 0 | 0 | 0 | 3 | I: fixed RGBA channels |
| import | 0 | 1 | 0 | 0 | 0 | I: typed persona serialization |
| link_preview | 4 | 0 | 0 | 0 | 5 | I: fixed regex and guaranteed captures |
| linked | 0 | 0 | 0 | 0 | 5 | I: JSON reads/typed record writes |
| log | 0 | 1 | 0 | 0 | 16 | I: literal sidecar, guarded record writes |
| mcp | 0 | 5 | 0 | 0 | 17 | I: normalized objects, checked OAuth resource, ASCII slug |
| models | 0 | 1 | 0 | 0 | 9 | I: bundled catalog, guarded one-element array |
| paths | 0 | 0 | 0 | 0 | 7 | I: byte/character guards |
| providers | 0 | 4 | 0 | 0 | 10 | E: discovery constructor; I: other three expects/accesses |
| push | 0 | 0 | 0 | 0 | 5 | I: literal JSON objects |
| remote | 63 | 2 | 0 | 0 | 51 | L: 53 unwraps; I: remaining 10 unwraps/two expects/accesses |
| room | 0 | 1 | 0 | 1 | 0 | I: typed jobs and object-only callers |
| sent | 1 | 0 | 0 | 1 | 13 | I: joined paths, narrowed format, SVG/name bounds |
| session | 1 | 10 | 0 | 1 | 90 | I: held queue/state, typed writes, checked indices |
| skills | 0 | 0 | 1 | 0 | 2 | I: bundled skills and line-derived offsets |
| store | 0 | 0 | 0 | 0 | 8 | I: matched event/segment indices |
| thread | 0 | 1 | 0 | 0 | 0 | I: typed Link serialization |
| tools | 0 | 1 | 0 | 0 | 9 | I: completed wait, bounded lines/private folder indices |
| vault | 0 | 3 | 0 | 0 | 11 | I: generated paths, normalized objects, typed credentials |
| voice | 0 | 23 | 0 | 0 | 127 | I: guarded consumers, model resolution, fixed regex/audio ranges |
| websearch | 0 | 22 | 0 | 0 | 40 | I: fixed regex/captures, bounded matching and outcome indices |
| wire | 2 | 2 | 0 | 1 | 27 | I: object guards, stream-bearing threads, narrowed commands |

Outside `src/`, the non-test build/example targets add nine unwraps, 15 expects,
three panics, no unreachable macros, and nine indices: **36 additional sites**.
Thus all workspace Rust excluding tests totals **93 / 109 / 4 / 5 / 571 = 782**
in the same column order. `crates/hotline-app/build.rs:10` and
`crates/hotline-app/build.rs:67` depend on Cargo-provided variables;
`crates/hotline-app/build.rs:71`, `crates/hotline-app/build.rs:94`, and
`crates/hotline-app/build.rs:122` deliberately stop a failed compiler/toolchain
build. Example assertions at `crates/hotline-core/examples/image_check.rs:21`
and `crates/hotline-core/examples/local_speech_check.rs:22` stop manual scratch
checks on filesystem/provider errors. These are externally fallible too, but
not desktop/session crashes; improve diagnostics separately if these utilities
become supported user commands. No build/example assertions enter shipping totals.

**5. Low priority: review the allowances by cause, not by blanket removal.**
The baseline contains **13**, not 12, `allow(clippy::result_large_err)` attributes:
seven production attributes and six test-only attributes. None changes error
semantics or justifies replacing a typed error with `String`.

| Location | Cause | Recommendation |
| --- | --- | --- |
| `crates/hotline-core/src/driver/rig/turn.rs:43` | `ChatMeter::reserve` returns owned `Failure` | Consider boxed Failure with the other four Rig sites below |
| `crates/hotline-core/src/driver/rig/turn.rs:141` | turn runner returns `Failure` | Same representation decision |
| `crates/hotline-core/src/driver/rig/turn.rs:194` | inner loop returns `Failure` | Same representation decision |
| `crates/hotline-core/src/driver/rig/turn.rs:689` | recovery returns `Failure` | Same; retain independent too-many-arguments rationale |
| `crates/hotline-core/src/driver/rig.rs:718` | `Turn::run` returns `Failure` | Same representation decision |
| `crates/hotline-core/src/wire/mod.rs:1021` | tungstenite handshake callback returns `ErrorResponse` | Keep narrow allow: upstream callback fixes the type |
| `crates/hotline-core/src/remote/bridge.rs:236` | same callback constraint | Keep documented allow |
| `crates/hotline-core/src/remote/tests.rs:1553` | test handshake rejection | Keep test-only allow |
| `crates/hotline-core/src/voice/realtime/socket/tests.rs:29` | mock handshake callback | Keep test-only allow |
| `crates/hotline-core/src/voice/realtime/socket/tests.rs:342` | mock handshake callback | Keep test-only allow |
| `crates/hotline-core/src/voice/realtime/socket/tests.rs:484` | mock handshake callback | Keep test-only allow |
| `crates/hotline-core/src/voice/speech/xai/tests.rs:213` | mock handshake callback | Keep test-only allow |
| `crates/hotline-core/src/voice/speech/xai/tests.rs:426` | mock handshake callback | Keep test-only allow |

`Failure` at `crates/hotline-core/src/driver/failure.rs:40` carries several
strings, static title/summary/phase, status, retry information, sign-in metadata,
and execution uncertainty. It is useful structured data. A single
`Box<Failure>` convention across the five Rig functions could remove their
allowances without changing serialized notices; it adds allocation on failure.
There is no measured performance problem here. Avoid boxing each individual
field or inventing a workspace-wide Result alias just to quiet this lint.

**6. Medium priority where control flow depends on text: typed errors pay off selectively.**
The best immediate candidate is search. `Failure` already exists at
`crates/hotline-core/src/websearch/mod.rs:108`, but
`crates/hotline-core/src/websearch/mod.rs:193` computes cooldowns by finding
“rate limited”, “credits exhausted”, “timed out”, or a rendered status in its
message; `crates/hotline-core/src/websearch/mod.rs:687` again classifies that text
for display. A `FailureKind` plus status/retry-after would keep retry policy
independent of copy changes, while rendering at the tool boundary as today.
Inspect every adapter when doing this: provider refusal text must stay scrubbed.

Useful existing types to preserve are `ToolError`/`ToolErrorKind`
(`crates/hotline-core/src/tools/mod.rs:22`), MCP `CallError`
(`crates/hotline-mcp/src/tool.rs:97`), remote `OpenError`
(`crates/hotline-core/src/remote/client.rs:490`), `ImageError`
(`crates/hotline-core/src/imagegen/mod.rs:117`), and `SpeechError`
(`crates/hotline-core/src/voice/speech/mod.rs:67`). They distinguish refusal,
revocation, transport loss, unsupported input, or retry/fallback conditions.
In particular a lost remote connection must not become a revoked grant.
Computer `Refusal::TooOld` versus `Failed(String)` at
`crates/hotline-core/src/computer/secrets.rs:25` is similarly purposeful:
the former offers an update, while the latter retains a diagnostic.

`Result<_, String>` remains common at the driver/session/wire/Tauri boundaries:
for example `crates/hotline-core/src/wire/commands.rs:1238`,
`crates/hotline-core/src/session/jobs.rs:296`,
`crates/hotline-app/src/transfer.rs:115`, and
`crates/hotline-cli/src/update.rs:11`. It is adequate for a terminal validation
sentence with no branching caller. Where errors still need retry, status,
source chaining, or a separate human label, keep a domain type inside the
module and convert once at the existing boundary. Startup/discovery errors
are good small opportunities; replacing every String at once would spread
through drivers and wire handlers without a corresponding benefit.
Keep `io::Result` in log/credential storage where callers examine `ErrorKind`.

**7. Medium priority, small UI batches: short error labels are not universal.**
The main turn error path already follows `docs/design.md`'s notice format:
`crates/hotline-core/src/driver/failure.rs:262` assigns short titles such as
“Provider rate limit”; `ui/src/components/ErrorCard.tsx:96` puts diagnostics
behind “Error details”. Its legacy fallback at
`ui/src/components/ErrorCard.tsx:41` also hides raw text. Preserve sanitization,
the title/summary envelope, and the unchanged JSONL event shape.

Several command/settings errors bypass that treatment. The Rust updater
includes the underlying error in its sentence at
`crates/hotline-app/src/updater.rs:272`; the window uses it directly as the
message at `ui/src/components/UpdatesSection.tsx:81`.
`ui/src/components/ConnectProvider.tsx:66` captures raw Error text and
`ui/src/components/ConnectProvider.tsx:224` passes it to `Refusal` without details.
Similar direct rendering is at `ui/src/components/Skills.tsx:201` and
`ui/src/components/Secrets.tsx:247`. Wire errors become plain Error messages
at `ui/src/wire.ts:525`, so their callers cannot infer a short stable label.
Even `ErrorCard` renders a failed retry's raw message inline at
`ui/src/components/ErrorCard.tsx:92` (the line containing `{again}`).
The `Refusal` component supports disclosure already (`ui/src/ui/Refusal.tsx:22`);
follow the existing example at `ui/src/components/McpPasteBack.tsx:54`.
Use a few-word action label (“Update check failed”, “Connection failed”) and
sanitized detail behind a working disclosure. Do not truncate diagnostics or
put them into tooltips. Decision-2's 2–3 words, five maximum applies to tooltips
and short labels, not to all explanatory prose; a brief recovery sentence is
still useful. No change to decision-1 or decision-2 is needed.

## Leave alone

Keep the following invariant clusters. Where an `unwrap`, empty
`unreachable!()`, or vague expect message remains, make the reason explicit
when touching that code; do not fabricate an external error for an impossible
state. These are concrete local proofs, not “the input usually looks right”.

An additional assertion sweep found one production `assert!`, outside the
requested syntax totals: `crates/hotline-core/src/paths.rs:51` rejects an empty
file component. Keep it: roster and import readers filter empty ids at
`crates/hotline-room/src/room.rs:216` and
`crates/hotline-core/src/import/personas.rs:66`; tape readers/writers reject
them at `crates/hotline-core/src/log/tape.rs:24` and
`crates/hotline-core/src/log/tape.rs:73`. Avatar reads check a living teammate
before constructing its path (`crates/hotline-core/src/wire/commands.rs:581`).
New callers of the public path helpers must retain those boundary checks.

| Cluster and evidence | Why it cannot fail under the current callers |
| --- | --- |
| Literal schemas/objects: `crates/hotline-core/src/bin/hotline-mcp-echo.rs:24`, `crates/hotline-core/src/log/thread.rs:84`, `crates/hotline-core/src/mcp/server.rs:149` | Built with object-shaped literals immediately above; use “the literal tool schema is an object” rather than unwrap |
| Typed serialization: `crates/hotline-core/src/import.rs:466`, `crates/hotline-core/src/wire/commands.rs:1248`, `crates/hotline-core/src/thread/link.rs:158`, `crates/hotline-core/src/vault.rs:1034`, `crates/hotline-core/src/voice/speech/local/install.rs:448` | Fixed serializable structs/string-keyed fields, not arbitrary deserialization; `voice/dispatcher.rs:962` serializes a string and should say so instead of “text” |
| Bundled resources: `crates/hotline-core/src/models.rs:305`, `crates/hotline-core/src/skills/mod.rs:55` | Compiled-in files, validated by their tests; cannot be replaced by user disk content at runtime |
| Fixed regex/captures: `crates/hotline-core/src/link_preview.rs:196`, `crates/hotline-core/src/websearch/when.rs:31`, `crates/hotline-core/src/voice/spoken.rs:88`, `crates/hotline-core/src/voice/spoken.rs:151` | Literal patterns and mandatory matched groups; replace preview's four unwraps with an expect explaining the fixed pattern |
| Narrowed enums: `crates/hotline-core/src/driver/rig/images.rs:143`, `crates/hotline-core/src/sent/generated.rs:48`, `crates/hotline-core/src/session/jobs.rs:524`, `crates/hotline-core/src/wire/mod.rs:1630`, `crates/hotline-room/src/room.rs:188` | Prior matches/filters restrict variants; all room_event callers supply object literals/typed records. Explain the narrowing in the two empty unreachable messages |
| Response builders: `crates/hotline-core/src/computer/gate.rs:322`, `crates/hotline-core/src/computer/gate.rs:378`, `crates/hotline-core/src/remote/server.rs:45`, `crates/hotline-core/src/remote/bridge.rs:260` | Typed status and fixed valid headers; the body is not parsed as a header. Replace raw unwraps with that reason |
| Normalized MCP state: `crates/hotline-core/src/mcp/mod.rs:246`, `crates/hotline-core/src/mcp/mod.rs:787`, `crates/hotline-mcp/src/oauth.rs:284`, `crates/hotline-core/src/driver/acp.rs:2024` | Normalization/object checks and connected-proxy filtering precede access; proxy entries are not concurrently removed on this path |
| Guarded HTTP state: `crates/hotline-core/src/providers/discovery.rs:140`, `crates/hotline-core/src/providers/xai.rs:56`, `crates/hotline-core/src/providers/xai.rs:334` | Valid response/status already obtained; distinct from fallible client construction |
| Remote ownership: `crates/hotline-core/src/remote/admission.rs:172`, `crates/hotline-core/src/remote/relay.rs:217`, `crates/hotline-core/src/remote/v2.rs:356`, `crates/hotline-core/src/remote/server.rs:483`, `crates/hotline-core/src/remote/channel.rs:58` | Live permit entry, paired host/budget, checked pairing window, once-taken seat, or nonempty queue; same lock/exclusive owner spans the check and use |
| Session ownership: `crates/hotline-core/src/session/dm.rs:168`, `crates/hotline-core/src/session/dm.rs:475`, `crates/hotline-core/src/session/jobs.rs:522`, `crates/hotline-core/src/session/exchanges.rs:1256`, `crates/hotline-core/src/session/sides.rs:310` | Turn driver consumes each hold once; indices/ids come from the same held collection; OnceLock-backed driver stays installed |
| Session work lease: `crates/hotline-core/src/session/mod.rs:2630`, `crates/hotline-core/src/session/mod.rs:813` | Dispatch callers retain a read lease, preventing restart's exclusive write lease; preserve the lease across new callers |
| Voice state: `crates/hotline-core/src/voice/mod.rs:1376`, `crates/hotline-core/src/voice/mod.rs:1460`, `crates/hotline-core/src/voice/mod.rs:1521`, `crates/hotline-core/src/voice/realtime/live.rs:825` | Accepted input and single work-item consumer; async select branch is polled only when its option is present. Replace “guarded” with the actual branch condition |
| Resolved voice model/tool: `crates/hotline-core/src/voice/dispatcher.rs:444`, `crates/hotline-core/src/voice/dispatcher.rs:924`, `crates/hotline-core/src/voice/settings.rs:69` | Resolver constructs provider/model; built-in tool exists; each enum's static voice list is nonempty |
| Piped processes/completed wait: `crates/hotline-core/src/computer/mod.rs:1145`, `crates/hotline-core/src/computer/mod.rs:1146`, `crates/hotline-core/src/tools/shell.rs:330` | Spawn explicitly requests pipes; successful uninterrupted wait sets output before it is taken |
| Never-closed decoder semaphore: `crates/hotline-core/src/driver/rig/images.rs:30`, `crates/hotline-core/src/driver/rig/images.rs:74` | Static semaphore has no close caller; ToolOutput replacement retains its nonempty content invariant |
| Joined/constant paths: `crates/hotline-core/src/remote/attachments.rs:81`, `crates/hotline-core/src/sent.rs:321`, `crates/hotline-core/src/session/avatar.rs:247`, `crates/hotline-core/src/vault/launch.rs:80`, `crates/hotline-cli/src/update.rs:299`, `crates/hotline-cli/src/update.rs:638`, `crates/hotline-app/src/updater.rs:224` | Child file names are joined to a directory; Linux updater target derives from its absolute BINARY constant. Give the six CLI parent unwraps that reason |
| Validated addresses/targets: `crates/hotline-core/src/remote/network.rs:57`, `crates/hotline-core/src/remote/viewer_files.rs:190`, `crates/hotline-core/src/remote/served.rs:71`, `crates/hotline-app/src/updater.rs:275` | Host validated/enumerated as IP, private loopback URL, served-only callers, or immutable target checked earlier |

**Indexing is not a single failure class.**
Read-only `serde_json::Value` indexing returns Null for absent/wrong-shaped
members: for example `crates/hotline-core/src/imagegen/google.rs:84` and
`crates/hotline-core/src/imagegen/openai.rs:158` subsequently validate the data.
Do not convert these reads mechanically to error handling at each bracket.
Mutable Value indexing can panic on a scalar, but reviewed writes start with
typed/literal objects or prove a named member exists first. For example
`crates/hotline-core/src/wire/mod.rs:2133` is guarded by `get("dataUrl")`;
`crates/hotline-core/src/log/mod.rs:505` only mutates a checkpoint after reading
its named backendId. Array/scalar records cannot pass those named-field checks.
Map indexing differs: `settings["images"]` at
`crates/hotline-core/src/capabilities.rs:33` can panic on a missing key, but
`crates/hotline-room/src/room.rs:52` always seeds that key and spending defaults.
The ACP env map access at `crates/hotline-core/src/driver/acp.rs:2010` uses keys
from that same immutable map.

Bounds/UTF-8 clusters also have explicit proofs: ASCII slugs before truncation
(`crates/hotline-mcp/src/tool.rs:91`), ASCII-only generated image stems
(`crates/hotline-core/src/session/generate.rs:315`), brace-find character offsets
(`crates/hotline-core/src/session/chapters.rs:286`), boundary-adjusted output
elision (`crates/hotline-core/src/driver/rig.rs:836`), hexadecimal byte checks
before percent decoding (`crates/hotline-core/src/paths.rs:287`), same-vector
enumeration (`crates/hotline-core/src/websearch/mod.rs:592`), private immutable
folder indices (`crates/hotline-core/src/tools/workspace.rs:436`), segment-sized
parallel vectors (`crates/hotline-core/src/store/previews.rs:61`), and RGBA channel
limits (`crates/hotline-core/src/images.rs:125`). Audio uses short-circuit length
checks and checked chunk ends before slices/fixed-size conversions
(`crates/hotline-core/src/voice/mod.rs:2892`,
`crates/hotline-core/src/voice/mod.rs:2905`). Keep these local guards beside
the access; “four bytes” expects could better say “the checked slice is four bytes”.

Preserve the typed error classes and existing error-card sanitization. Preserve
byte-compatible transcript notices; do not add a parallel wire error protocol
or global error framework during this cleanup. A compact allow for an upstream
fixed callback type is clearer than converting useful errors to strings.

## Phase 2 tasks

Each proposal is independently landable, ends with `make check`, and changes
no public wire or tape shape. The panic fixes do change failure behavior and
need George's approval after this read-only phase. Keep them separate from
move-only file splits and coordinate allowance/lint work with task-106.

1. **Return desktop startup failures.** Replace the four app startup expects
   with one error boundary, preserving failure details and an unsuccessful exit.
   Prove invalid `remote.json`, inaccessible startup storage, failed bind, and
   failed shell setup do not unwind or reset saved state. Inject failures at
   narrow setup seams; no live instance or real data directory is needed.
2. **Make discovery client construction fallible.** Remove its `Default`
   implementation, propagate constructor errors at all three call-site families,
   and retain previous model lists. Add an injected client-build failure and
   extend the existing discovery/wire proof so the request finishes as an error.
3. **Return CLI setup errors.** Make runtime, signals, and token creation
   fallible, registering termination handling before admitting serve work.
   Verify nonzero exits and no advertised listener/token on injected failure;
   preserve entropy strength and existing orderly shutdown behavior.
4. **Normalize admission/relay lock recovery.** First cover `Permit::drop`
   and admission acquisition, then relay state in a separate small commit.
   Use the existing guard-recovery idiom, prove cleanup signals even after
   poisoning, and retain allowed/denied/revoked admission tests through the real
   handler. Review partial mutations before deciding recovery is valid.
5. **Normalize remaining remote and transfer locks.** Apply the same reviewed
   pattern to Remote state/bridge and the five transfer sites, in separate
   domain-sized commits. Prove poisoning does not unwind cleanup, and tokens,
   revocation, pending uploads and file grants retain their denial behavior.
6. **Explain retained invariants mechanically.** Replace the 26 non-lock
   unwraps with reasoned expects, and improve empty/vague unreachable/expect
   messages using the table above. Keep fixed regexes, constructors, narrowing
   guards and index access behavior. No generic safe-index abstraction.
7. **Give search failures a kind.** Carry adapter status/retry-after and
   scrubbed details, select cooldowns by a local enum, and render the existing
   tool text at the edge. Test unchanged cooldown behavior when diagnostic
   wording changes, including quota, rate limit, timeout and server errors.
8. **Use Refusal details for updater errors.** Supply “Update check failed” or
   the matching action label, move diagnostics into the existing disclosure,
   and test expand/collapse plus preserved retry controls. Then make a separate
   small pass over provider connection/skills/secrets and ErrorCard retry errors.
   Keep tooltips within decision-2 and every displayed action functional.
9. **Optional, lowest priority: box Rig failures once.** If desired for lint
   policy, change the five internal Result error representations together and
   remove their five allowances. Prove notice serialization, retry policy,
   sign-in actions and tools-may-have-run remain identical. Leave the eight
   tungstenite callback allowances (two production, six tests) alone.

Validation: `make check` passed on macOS: formatting, Clippy with warnings
denied, 2,024 Rust tests (11 ignored), 420 UI tests, UI typechecking, and the
9 script / 44 site Python tests. The full core run left the generated contract
unchanged. The syntax inventory and macro supplements reconcile with the
group totals, and all 162 file:line references were checked for valid locations.
Contract checks cover at least
60 lines, exactly the three requested level-two headings, Rust file:line
evidence, and a branch diff containing only this report.
