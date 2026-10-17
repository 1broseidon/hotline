# 0.40 audits: index and the Phase 2 plan

Task-109, epic-12, 2026-10-17. Phase 1 was six read-only audits of
`release/0.40.0`. This file merges them into one ordered plan for George to
approve. Nothing in it is filed or applied yet. The six reports are unchanged;
where one of them is wrong about the code, the correction is under
[Contradictions](#contradictions) and the report stays as written.

## Reports

- [structure.md](structure.md) (task-103): Rust module layout. 11 files are
  over a 1,500-line production budget, two 7k-line test files are the merge hot
  spots, 16 `mod.rs` sit beside 13 `foo.rs` pairs, and `contract` is not a
  leaf. It proposes 21 move-only tasks and no new crates.
- [dead-code-and-deps.md](dead-code-and-deps.md) (task-104): dependency
  advisories (ZIP extraction, rustls, two npm families), missing license
  metadata, three dead implementations, 22 value and 14 type exports that
  should be private, and every one of the 10 `dead_code` suppressions
  accounted for. No direct dependency is unused. It proposes 18 tasks, with
  the gates last.
- [errors.md](errors.md) (task-105): 746 panic sites in shipping source.
  Nine are reachable fallible setup: desktop startup, the discovery client,
  and CLI runtime, signal and entropy setup. 58 are lock poisoning, handled
  two incompatible ways. The rest are guarded invariants. Search cooldowns
  branch on message text, and several UI errors show raw diagnostics. It
  proposes 9 tasks.
- [lints.md](lints.md) (task-106): pedantic, nursery and restriction lints
  measured lint by lint, a proposed `[workspace.lints]` table, the 9
  `too_many_arguments` allows (all removable by grouping arguments that
  already travel together) and the 13 `result_large_err` allows. fmt and the
  edition need nothing. It proposes 7 tasks.
- [window.md](window.md) (task-107): 52 effect dependency-list hits,
  a 780-line `App()`, three components of 1,800–2,000 lines, dead exports, and
  Biome rather than ESLint, which cannot parse TypeScript 7. It proposes 14
  tasks.
- [tests-and-docs.md](tests-and-docs.md) (task-108): four proofs cited in
  `docs/security.md` that no longer exist under those names. The remote
  harness depends on the host's network, nothing uses a paused clock, and the
  test `Client` exists eight times. Docs drift lines are given with file:line.
  It proposes 18 tasks.

## Phase 2 order

The six reports propose 87 tasks. Merged and deduplicated, they become tasks
1–56 (plus 22a) and the docs tasks D-1 to D-6.

**How the order was set.**

1. Security fixes come first.
2. Then dead code goes, so nothing dead is moved.
3. Then move-only renames and splits, so blame survives and the fixes after
   them land on the final paths without conflicts.
4. Then lint, error, test and window fixes in the moved files.
5. Docs come after the code they describe.
6. Last come the gates: the `[lints]` table, Biome, and the scanners in
   `make check`. Each is added only once the code already passes it.

**Running in parallel.** Within a group, a task whose "Collides with" names
no open task can run alongside the others. Every task is one landable commit,
or a short series where it says so, and ends with `make check`.

**Size.** XS is one mechanical edit. S is under a day. M is one to three
landable commits. L is more.

### A. Security

These come first, ahead of all quality work.

| # | Task | Source | Files | Collides with | Size |
|---|---|---|---|---|---|
| 1 | **security.md proves what it cites.** Re-point and re-word the four rows (verdicts below), and write the missing proof for `:1426`. George decides `:683` | tests-and-docs T3, task 1 | `docs/security.md:330`, `:683`, `:703`, `:1426`; a new test in `session/tests.rs` | 23 (`session/tests.rs` split): land first | S |
| 2 | **Patch rustls** from 0.23.45 to `>=0.23.46`, that crate only. cargo-deny no longer reports RUSTSEC-2026-0346 to 0349 | dead-code F2, task 1 | `Cargo.lock` | 3, 4, 5 (lockfile): land one at a time | XS |
| 3 | **Patch ACP ZIP extraction (F1).** This is a capability change; see below | dead-code F1, task 2; also lints `case_sensitive_file_extension_comparisons` at `install.rs:112` and dead-code F8 (`zip` features) | `crates/hotline-core/Cargo.toml:42`, `Cargo.lock`, `crates/hotline-core/src/driver/acp/install.rs` and its tests, `docs/security.md` (new row) | 2, 4, 5 (lockfile); 11 (Cargo.toml) | M |
| 4 | **Tauri updater ZIP path.** Upgrade the owning plugin to a release with patched extraction, or file an upstream blocker with the IDs. Do not force ZIP 9 under a ZIP 4 API | dead-code F1, task 3 | `crates/hotline-app/Cargo.toml`, `Cargo.lock` | 2, 3, 5 (lockfile) | M |
| 5 | **sherpa build ZIP path.** Move to a patched upstream build release and refresh the archive hashes. Builds still never use an unverified download | dead-code F1, task 4 | `crates/hotline-core/Cargo.toml` (sherpa), `scripts/fetch-speech-engine`, `scripts/speech-engine.sha256`, `Cargo.lock` | 2, 3, 4 (lockfile) | M |
| 6 | **Patch source-map-js** to `>=1.2.2` within the parent ranges (GHSA-68fv-2mgg-jv7q) | dead-code F3, task 5 | `ui/bun.lock` | 7, 12 (lockfile) | S |
| 7 | **Ship a Mermaid bundle with KaTeX fixed** (GHSA-238p-pmpm-9mq7). Check the minified asset itself, because a lockfile override does not reach the copy embedded in `mermaid.min.js`. Keep classic-script loading, strict mode and plain labels | dead-code F3, task 6 | `ui/package.json`, `ui/bun.lock`, `ui/src/components/Mermaid.tsx` only if its API moved | 6, 12 (lockfile) | M |

**Task 1: the four missing proofs.** Each verdict below was checked
against `git log -S` and the current test.

| Row | Verdict | What happened, and the fix |
|---|---|---|
| `security.md:330` `chatgpt_images_status_requires_owner_selection_without_checking_entitlement` | **Stale index; the claim is now false.** | Commit 3175176d (2026-10-01, "a subscription draws first") made ChatGPT images automatic. The test was renamed `chatgpt_images_status_is_automatic_without_checking_entitlement` (`wire/tests.rs:5416`). It still proves the companion denial through the real handler, and that there is no entitlement check. "Proves selected status" no longer describes anything. Re-word the row to describe automatic use when signed in. |
| `security.md:683` `teammate_discovery_never_derives_public_fields_from_private_instructions` | **Stale index over an unrecorded reach change.** | Commit 98ee7021 (2026-09-25) gave `list_teammates` state, activity and `workingOn`. In the same commit the test lost its leak assertions: a secret-shaped `goal` must not appear, and `goal` must not be listed. It became `teammate_discovery_never_derives_the_technical_fields_of_a_persona_record` (`mcp/server.rs:1415`). `working_on_falls_back_to_the_goal_until_a_chapter_is_titled` now asserts that a colleague's goal *is* shown. So the row claims something the code deliberately stopped doing, and `docs/security.md` never recorded the change. **George decides:** either accept that the goal is visible to colleagues, and re-word the row (five fields, never `cwd`, `mcpPolicy`, `reach`, `node`, `backendId`) with residual risk "a secret typed into a goal reaches colleagues"; or restore the old rule as a capability fix, with its own test. |
| `security.md:703` `a_computer_from_before_secrets_is_named_only_when_something_was_granted` | **Stale index; the behaviour changed on purpose.** | Commit eccf4cc9 (2026-09-23, "no narrated machinery") made the tape silent either way. The test is `a_computer_from_before_secrets_leaves_the_conversation_alone` (`session/tests.rs:2626`). No reach was lost: this is a notice, not a grant. Re-word the clause ("a release from before secrets is told apart, and the conversation says nothing") and re-point the name. |
| `security.md:1426` `session::tests::only_the_reply_to_a_voice_handoff_gets_a_summarised_push` | **A capability with no proof.** tests-and-docs.md found "no near match"; there is one. | Commit a8c71102 (2026-10-01) renamed it `short_voice_handoff_replies_keep_their_text_without_an_extra_model` (`session/tests.rs:6769`). It now asserts that all three pushes are identical and that no narration happens. The voice-provenance branch that rewrites a push through a paid summary (`voice/mod.rs:2575`, `!from_voice`) is still live, but no test can tell a voice reply from a typed one any more. Write the proof: a reply long enough to summarise is rewritten only when it came from a voice handoff, and a typed one with the same text is pushed as written. |

**Task 3 is a capability change** under `docs/security.md`. It changes what an
ACP registry install can write on the host. `docs/security.md` has no row
for registry installs today, so the PR adds one:

- **Default.** An agent from the ACP registry is downloaded only when a
  session starts it. It is unpacked into a scratch folder beside its install
  folder and renamed into place (`install.rs:27`).
- **Grant source.** The person's choice of an ACP backend for a teammate.
  The archive URL and optional `sha256` come from the registry. When the
  registry gives no digest, nothing is checked (`install.rs:39`).
- **Enforcement points.** All in `unpack` (`install.rs:103`), outside the
  model:
  - The ZIP version is one the fixes cover.
  - A symlink entry, an entry whose path leaves `into`, and set-id or
    world-writable modes are refused, for zip and tar alike.
  - The command must be a regular file under the install folder
    (`install.rs:58`).
- **Tests.** Through `fetch`:
  - Denial: a symlink escaping the scratch folder, a symlink followed by a
    write through it, `../` entries, and a set-id mode. Each installs
    nothing and leaves no scratch.
  - Allowed: the existing `a_tarball_is_unpacked_into_place…`,
    `a_zip_without_modes_still_starts`, `a_bare_binary_is_put_where_the_command_says`,
    `a_checksum_mismatch_installs_nothing`.
- **Residual risk.**
  - With no published digest, the archive is trusted as served.
  - The installed agent runs with the host's permissions (the accepted risk
    at `security.md:106`).
  - Tasks 4 and 5 own the updater and sherpa copies of ZIP.
- **Revocability.** An install is a file on disk, not a lease. Removing the
  backend removes the reason to run it, and pruning (`install.rs:156`) keeps
  only the current release.

### B. Delete dead code

Dead code goes before anything moves.

| # | Task | Source | Files | Collides with | Size |
|---|---|---|---|---|---|
| 8 | **Rust dead code.** Delete `paths::resolve_command` and its docs (keep `resolve_command_in`). Drop the redundant `#[allow(dead_code)]` on `Job`'s field, which `terminate` reads. Check, and delete, the never-read collection at `desks.rs:660` | dead-code F5, F6, tasks 8 and 9; lints `collection_is_never_read`, `dead_code` | `crates/hotline-core/src/paths.rs:183`, `process_windows.rs:38`, `crates/hotline-app/src/desks.rs:660` | none | XS |
| 9 | **Narrow `lib.rs`.** Make the nine modules no outside crate names `pub(crate)`, and delete whatever the compiler then reports as unused. Moved up from structure's step 19 (C10) | structure F12, task 19 | `crates/hotline-core/src/lib.rs:14-48`, plus whatever the compiler reports | 16 (contract leaf) touches `thread`, `spending`, `linked`: land 9 first | S–M |
| 10 | **Window dead code.** Delete `BookIcon`, `currentCall` and `voiceOf`, which have no use beyond their definition (C1). Drop `export` from the other value exports in dead-code F7 that have a use inside their own file (`windowTitle` among them), and from the 14 types. Keep `cadence.ts`, `pacing.ts`, `reveal.ts` and `Segment`, which Hotline Mobile copies (C2). Re-run knip | dead-code F5, F7, tasks 8 and 10; window F4, tasks 1 and 2 | `ui/src/icons.tsx:197`, `voice/call.ts:32,155,160,1029`, `useFor.ts:38-127`, `Transcript.tsx:1310,2034`, `Rail.tsx:508`, `Skills.tsx:15`, `wire.ts:71-310`, and the rest of the list in dead-code F7 | 33 (window splits) and 54 (knip gate): land first | S |
| 11 | **Dependency surface.** Rig without `derive` (explicitly `agent`, `reqwest`, `rustls`). Drop `Win32_System_Memory` | dead-code F8, tasks 11 and 12 | `Cargo.toml:10`, `crates/hotline-core/Cargo.toml:114`, `Cargo.lock` | 2–5 (lockfile, core Cargo.toml) | S |
| 12 | **Deduplicate `@tauri-apps/api`** onto the tested direct pin | dead-code F9, task 13 | `ui/bun.lock:687` | 6, 7 | S |
| 13 | **Declare crate licenses.** Add an SPDX expression in `[workspace.package]` and inherit it in all three crates | dead-code F4, task 7 | `Cargo.toml`, three `Cargo.toml`s | 52 (lints table, same section) | XS |

### C. Move-only renames and splits

There are no behaviour changes in this group. Land these in order: the
renames first, then the two merge hot spots.

| # | Task | Source | Files | Collides with | Size |
|---|---|---|---|---|---|
| 14 | **`foo.rs` + `foo/`.** `git mv` all 16 `mod.rs` files; `git diff -M` shows 100% similarity | structure F9, task 1 | the 16 `mod.rs` | everything under `src/`: land with no other Rust branch open | S |
| 15 | **One test-file naming style.** `chatgpt_tests.rs` → `chatgpt/tests.rs`, and the `#[path]` goes. `auth_tests.rs` moves under `acp/auth`. `remote/tests/*_tests.rs` → `<topic>.rs` | structure F9, task 2 | `imagegen/chatgpt*`, `driver/acp/auth_tests.rs`, `remote/tests/*` | 25, 37, 38 | XS |
| 16 | **`contract` becomes a leaf.** Move the types it borrows into it, with no re-export left behind. `contract.ts` stays byte-identical | structure F3, F10, task 3 | `contract.rs`, `thread`, `remote`, `linked`, `imagegen`, `spending`, and their callers | 17 | S |
| 17 | **Split `contract.rs`** into the nine submodules, re-exported. `contract.ts` stays byte-identical | structure F3, task 4 | `contract.rs` → `contract/*.rs` | 16 | M |
| 18–22 | **`session/mod.rs` in five parts:** preamble and asking; secrets; computer; prompt and driver_info; start and chapters | structure F1, tasks 5–9 | `session.rs`, `session/{preamble,asking,secrets,computer,prompt,driver_info,start,chapters}.rs` | 23; 40 (expects in `session/mod.rs:813,2630`) | M |
| 22a | **`Room` constructors:** one `Room::new` with options, or leave them. Its own commit after 22, not inside a move (C16) | structure F1 | `session.rs`, `session/start.rs` | 22 | S |
| 23 | **`session/tests.rs`** → fixtures plus `session/tests/*.rs`, 131 tests before and after | structure F6, task 10 | `session/tests.rs` | 1 (new test: land 1 first), 45 | M |
| 24 | **`wire/tests.rs`** → fixtures plus `wire/tests/*.rs`, 126 tests | structure F6, task 11 | `wire/tests.rs` | 37, 45 | M |
| 25 | **`remote/tests.rs`** topic split. Pulled forward from structure's step 18 so that the network fixes (37, 38) can follow | structure F6, task 18 (part) | `remote/tests.rs` | 15, 37, 38 | M |
| 26 | **`driver/acp.rs` tests** → `acp/tests.rs` plus `acp/tests/*.rs` | structure F2, task 12 | `driver/acp.rs` | 27 | M |
| 27 | **`driver/acp.rs` code** → `oauth_proxy.rs`, `live.rs`, `translate.rs`, `session.rs` | structure F2, task 13 | `driver/acp.rs` | 3 (`acp/install.rs` is untouched, but land 3 first) | M |
| 28 | **`voice/mod.rs`** → `call.rs`, `input.rs`, `turn.rs`, `synthesis.rs` | structure F4, task 14 | `voice.rs`, `voice/*` | 41 (`start_call`, `speak_sentence`), 40 | S |
| 29 | **`wire/mod.rs`** → `room_handle.rs`, `seat.rs`, `door.rs`, `socket.rs`, `subscribe.rs`, `phone.rs` | structure F5, task 15 | `wire.rs`, `wire/*` | 41 (`subscribe`, `stream_events`), 42 (`wire/mod.rs:1021`) | S |
| 30 | **`wire/commands.rs`:** `run` only dispatches, and the arms go to `commands/{persona,settings,models}.rs` | structure F7, task 16 | `wire/commands.rs` | 41 (`mobile_persona_create`) | S |
| 31 | **The six other over-budget files**, one commit each: `session/peers.rs`, `tools/workspace.rs`, `session/sides.rs`, `computer/mod.rs`, `driver/rig.rs`, `driver/rig/turn.rs` (its tests, plus the `run_inner` phases with the retry rule kept in one place) | structure F8, task 17 | those files | 41 (`rig/turn.rs:689`), 42 (rig `Failure` sites), 47 (`turn.rs` sleeps) | M |
| 32 | **The remaining big test files:** `voice/tests.rs`, `tests/voice.rs`, `tests/desk.rs` | structure F6, task 18 | those files | 43, 44, 45 | M |
| 33 | **Window splits, move-only, each re-exported from its old path.** First `App()` → `useNavigation`, `useSeen`, `useWindowHotkeys`. Then `Settings.tsx` → `components/settings/`, `Transcript.tsx` (folds to `transcriptFolds.ts` first), and `Teammate.tsx`. One landing each | window F2, F3, tasks 9–12 | `ui/src/App.tsx`, `components/{Settings,Transcript,Teammate}.tsx` | 10 (land first); 48, 49 (fixes follow) | L |

### D. Fixes in the moved files

These change behaviour or code shape, each in small commits. Error fixes
change failure behaviour, which needs George's approval (errors.md).

| # | Task | Source | Files | Collides with | Size |
|---|---|---|---|---|---|
| 34 | **One boot that returns errors.** A core function that both `hotline serve` and the app call: room lock, backend, Desk, Remote, Door, and one token minter that never falls back to weak entropy. The desktop's four startup `expect`s and the CLI's runtime, signal and entropy `expect`s become errors at one outer handler, with an unsuccessful exit. The relay-only, file-store and drain differences stay as arguments (C13) | structure F13, task 20; errors F1, F3, tasks 1 and 3 | `crates/hotline-app/src/lib.rs:66,307-362,570`, `crates/hotline-cli/src/serve.rs:161-282`, `door.rs:173`, `desk.rs` | none after 14 | M |
| 35 | **Discovery client is fallible.** Remove `Default`, propagate the error, and keep the previous list | errors F2, task 2 | `providers/discovery.rs:95,219,249,293` | 39 (`future_not_send` at `:182`) | S |
| 36 | **Lock recovery.** Use the one `PoisonError::into_inner` idiom for `Permit::drop` and admission first, then relay, then Remote and bridge, then the five `transfer.rs` sites. One domain per commit, with partial mutations reviewed | errors F4, tasks 4 and 5 | `remote/{admission,relay,bridge}.rs`, `remote.rs`, `crates/hotline-app/src/transfer.rs` | 39, 51 (`significant_drop_in_scrutinee` lens) | M |
| 37 | **One `ANSWER` for the remote tests,** at every wait for something that must arrive | tests-and-docs T2, task 2 | `remote/tests.rs`, `remote/tests/*.rs` | 15, 25 (land first) | S |
| 38 | **The remote harness connects over loopback.** The interface test keeps real addresses. Then let `Remote` and `ServeOptions` take a bound listener in tests, ending the free-port race | tests-and-docs T1, T11, task 3 | `remote/tests.rs:62-163`, `remote/network.rs`, `crates/hotline-cli/tests/{serve,relay,pair}.rs` | 25, 37 | M |
| 39 | **Lint autofix slice.** The fifteen "[1]" lints from lints.md, one commit per lint, followed by an unfiltered `cargo test -p hotline-core`. Prove each slice clean with a one-off `cargo clippy -- -W clippy::<lint>`; the table comes later (C4) | lints, task 2 | about 700 sites workspace-wide | every split in C: land after 33 | M |
| 40 | **By-hand lints and reasoned invariants.** The "[2]" lints and the one- to five-hit pedantic picks, reading `suspicious_operation_groupings`, `missing_fields_in_debug` and `unchecked_time_subtraction` first. Then replace the 26 non-lock `unwrap`s with reasoned `expect`s, and give the empty `unreachable!`s the narrowing that justifies them | lints, task 3; errors F5, task 6 | the hit sites; the clusters in the errors.md "Leave alone" table | 39 | M |
| 41 | **Argument groups.** Nine `too_many_arguments` refactors, one per function chain, with `make verify` for wire, voice and remote | lints, task 5 | `wire/subscribe.rs` (was `wire/mod.rs:1903,2270`), `voice/{call,synthesis}.rs`, `wire/commands/persona.rs`, `session/exchanges.rs:176`, `remote.rs:737`, `driver/rig/turn.rs:689` | 28–31 (land first) | M |
| 42 | **`result_large_err`: all 13 become `#[expect(…, reason)]`.** Five name the structured `Failure`, eight name tungstenite's fixed callback type. Boxing `Failure` is dropped unless George wants it (C6) | errors F5, task 9; lints, task 6 | `driver/rig/turn.rs:43,141,194,689`, `driver/rig.rs:718`, `wire` (was `mod.rs:1021`), `remote/bridge.rs:236`, six test sites | 31, 41 | S |
| 43 | **Paused clock** for the 30 s viewer-token and 10 s xAI deadline tests (`tokio` `test-util` as a dev feature). The core lib binary should drop below 10 s | tests-and-docs T5, task 4 | `Cargo.toml:16`, `crates/hotline-core/Cargo.toml:14`, `remote/tests/viewer_token.rs`, `voice/speech/xai/tests.rs:819` | 15 | S |
| 44 | **Honest tests.** Opt-in tests use `#[ignore = "needs …"]` instead of returning early. Elapsed-time ceilings become orderings or wide gaps. A missing `python3` or `perl` fails by name | tests-and-docs T6, T7, T10, tasks 5, 6, 9 | `tests/desk.rs:926,1041,1195`, `tests/local_speech.rs:428`, `tools/shell.rs:585-595,968`, the six T6 sites, `tests/processes.rs:28,385`, `tests/voice.rs:2826`, `tools/shell/macos.rs:476` | 32 | S |
| 45 | **Shared test plumbing.** One bounded test `Client` in `tests/common/mod.rs`, then one file per commit for the seven copies. The CLI tests get a `tests/common`. Add one `until(what, cond)` with one deadline, then replace the T8 sleep-then-assert sites module by module | tests-and-docs T4, T8, T9, T14, tasks 7, 8, 18 | `crates/hotline-core/tests/*.rs`, `tests/common/{mod,stalled}.rs`, `crates/hotline-cli/tests/*`, and the T8/T9 sites | 23, 24, 32 (land first) | L |
| 46 | **Search failures get a kind.** A local enum with status and retry-after picks the cooldown; the rendered text stays at the edge | errors F6, task 7 | `websearch/mod.rs:108,193,687` and the adapters | 39 | S |
| 47 | **Updater and settings errors use Refusal details.** First "Update check failed" with a disclosure, then provider connection, skills, secrets, and the ErrorCard retry. Labels stay within decision-2 | errors F7, task 8 | `crates/hotline-app/src/updater.rs:272`, `ui/src/components/{UpdatesSection,ConnectProvider,Skills,Secrets,ErrorCard}.tsx`, `ui/src/wire.ts:525` | 10 (`Skills.tsx`), 33 | S |
| 48 | **Window fixes that Biome needs first.** Triage the 52 `useExhaustiveDependencies` hits, one file per commit and `App.tsx` first, with a failing test before each real stale closure. Fix `stepText.ts:14` (`biome-ignore` with a reason), the two `useIterableCallbackReturn` and the two `useExponentiationOperator` hits. Give the icon wrapper `aria-hidden` (37 hits) | window F1, F6, tasks 8, 13, and the fixes in 7 | `ui/src/App.tsx`, `Settings.tsx`, `ConnectProvider.tsx`, `Welcome.tsx`, `icons.tsx`, `stepText.ts`, `voice/call.ts`, `dictation.ts` | 33 (land first) | M |
| 49 | **Window plumbing.** `storage.ts` for the two seen-record copies. `tests/dom.ts` for the 22 hand-built happy-dom windows. Add `tests` to `tsconfig` `include`, plus `noImplicitReturns` and `noPropertyAccessFromIndexSignature`, fixing what surfaces | window F5, F7, F9, tasks 3, 4, 6 | `ui/src/App.tsx:912-935`, `Teammate.tsx:1814-1839`, `ui/tests/*`, `ui/tsconfig.json` | 33 | M |
| 50 | **a11y pass:** the 9 `noAutofocus`, 8 `useSemanticElements` and 8 `noStaticElementInteractions` hits, as separate small UI changes | window F6, task 14 | the hit sites | 33, 48 | M |
| 51 | **Audit lenses, with no table entry.** Read the 23 `significant_drop_in_scrutinee` and 114 `large_futures` hits, and the `exit` and `mem_forget` sites. Adopt a lint only if a bug class turns up | lints, task 7 | read-only, then fixes as found | 36 | S |

### E. Docs

All are docs-only. They can run in parallel with C and D unless a collision
is named.

| # | Task | Source | Files | Collides with | Size |
|---|---|---|---|---|---|
| D-1 | **wire.md matches the code.** The four seat rules and "every seat"; rows for the forty missing commands; the `call` target, `window`, the subscription cap, snapshot windows, capabilities, `computer_pull`; the short param lists | tests-and-docs D2–D4, tasks 10 and 11 | `docs/wire.md` | 29 (cites `wire/mod.rs` lines): cite paths after 29 lands | M |
| D-2 | **Server docs.** `hotline update` in operate.md; status and the file table; relay and phone labels; the desk's account stated once; the `Paired` signature; `hotline update` and `help` in serve.md | tests-and-docs D1, D5, D7, D10, D11 (CLI part), tasks 12, 13, 15 (serve.md part), 17 (CLI part) | `docs-site/src/content/docs/server/*`, `phone/pair.md`, `docs/serve.md` | none | S |
| D-3 | **threads.md and sessions.md name live code** (`run_queue`, `Exchange::from_thread`, `threads2`); ADR 0005 becomes "Accepted (0.39)" with the 0.38.1 credit explained | tests-and-docs D8, D9, tasks 15 (threads part), 16 | `docs/threads.md`, `docs/sessions.md`, `docs/adr/0005-front-is-a-full-agent.md` | none | XS |
| D-4 | **Present-tense wording and what `make check` runs.** Fix `lib.rs:5`, `Makefile:42` and `design.md:663` (history stays labelled as history). Remove the test counts from development.md and list what `make check` really runs there and in `AGENTS.md:55`. Document `HOTLINE_EXTRA_DESKS` and `HOTLINE_DEV_SIGN_IDENTITY` | dead-code F10, task 14; tests-and-docs D6, D11, tasks 14 and 17 (env part) | `crates/hotline-core/src/lib.rs:5`, `Makefile:42`, `docs/design.md:663`, `docs/development.md:108-116,499`, `AGENTS.md:55` | 9 (`lib.rs`) | XS |
| D-5 | **One module map,** in `docs/development.md`, matching the tree after C, with `design.md` linking to it. Correct the "defined once in contract.rs" line | structure F11, task 21 | `docs/development.md:45-72,396`, `docs/design.md:634-660` | all of C (land last); D-4 (same file) | S |
| D-6 | **`docs/standards.md`** from the [Standards](#standards) section below, once George approves it | this file | `docs/standards.md` (new) | D-5 | S |

`docs/releases/0.40.0.md` is owed before `desktop-v0.40.0` is tagged
(tests-and-docs D9). That belongs to the release, not to Phase 2.

### F. Gates

Each gate lands last, once the code already passes it.

| # | Task | Source | Files | Collides with | Size |
|---|---|---|---|---|---|
| 52 | **The `[workspace.lints]` table in one change.** It holds every lint that 39–42 made clean, the zero-hit denies, and `allow_attributes_without_reason`. Add `rust-version`, and `[lints] workspace = true` in all three crates. `-D warnings` stays in the Makefile | lints, tasks 1 and 4 (C4, C5) | `Cargo.toml`, three crate manifests | 13 | S |
| 53 | **Biome for lint only.** `biome.json` with the window.md rule set, formatter off, a11y at warn until 50 lands, and a `lint` step in `make ui-check` | window F6, task 7 | `ui/biome.json`, `ui/package.json`, `Makefile` | 48 | S |
| 54 | **The window's gates.** Pinned knip with both HTML entries, tests, generated types, and the mobile-copy exports and `computerViewer.css` exempt. A contract preflight that fails in one sentence when `contract.ts` is partial | dead-code F4, task 16; window F10, task 5 | `ui/knip.json`, `ui/package.json`, `Makefile` | 10 | S |
| 55 | **The dependency gates.** Pinned machete, with the `tauri-build` false positive explained. cargo-deny advisories and `bun audit`, with unresolved upstream IDs (from 4, 5, and the unmaintained GTK and D-Bus crates) listed with a review condition | dead-code F4, tasks 15 and 17 | `deny.toml`, `Makefile`, `.github/workflows/ci.yml` | 2–7 | S |
| 56 | **The license gate.** A reviewed cargo-deny allow list, and a UI license and notice inventory covering fonts, bundled distributions and the native speech assets | dead-code F4, task 18 | `deny.toml`, the notice inventory | 13, 55 | S |

## Contradictions

Each entry gives the conflict and a recommended resolution.

- **C1. Which window exports are dead.** window.md F4 says 30 values "go".
  dead-code F7 says most are used inside their own file, so only the
  `export` goes. A grep of `ui/src` and `ui/tests` agrees with dead-code:
  every listed value has a second occurrence, except `BookIcon`,
  `currentCall` and `voiceOf`. dead-code lists `voiceOf` (`useFor.ts:94`) as
  "make private", but nothing calls it, so it is dead too. **Resolution:**
  delete those three and unexport the rest (task 10).
- **C2. Mobile-shared files.** window.md deletes `beatMs`, `PACE_WITHIN_MS`,
  `revealed`, `revealSegments`, `Segment` and possibly `pacing.ts` and
  `reveal.ts`. dead-code keeps them. Each file opens "Shared with Hotline
  Mobile: the phone copies this file with `npm run sync:contract`".
  **Resolution:** keep them, and exempt them in knip (task 54). Mobile is
  outside this epic.
- **C3. Is `contract.ts` tracked?** window.md F10 says it is gitignored.
  `git ls-files ui/src/generated` lists it, and `git check-ignore` does not
  match it. AGENTS.md rule 3 and structure.md's `git diff --exit-code`
  check are right. **Resolution:** drop window.md's "say so in ui docs" item.
  The partial-file preflight is still worth having (task 54).
- **C4. When the `[lints]` table goes in.** lints.md installs it first,
  with firing lints set to `allow` and each slice flipping one. The epic's
  brief says the table goes on last. **Resolution:** follow the brief. Each
  slice proves itself clean with a one-off `cargo clippy -W`, and the table
  lands once (task 52). Fix commits then never conflict over the same
  Cargo.toml lines. The cost is a short window in which a fixed lint can
  regress unnoticed.
- **C5. `allow` to `expect` for `dead_code`.**
  - lints.md turns `process_windows.rs:38` into `#[expect]`. dead-code shows
    the field is read at `:43`. An `expect` there would be unfulfilled and
    warn. **Resolution:** delete the allow (task 8).
  - lints.md would delete `import/records.rs:39`'s field. dead-code keeps it,
    because the tests assert it (`:344-347`) and it documents the legacy row.
    Tests reading it means a plain `expect` is unfulfilled under `cfg(test)`.
    **Resolution:** use `#[cfg_attr(not(test), expect(dead_code, reason = …))]`.
  - The `tests/common` allows cannot be `expect`s, because each test binary
    uses a different subset. **Resolution:** the table adopts
    `allow_attributes_without_reason`, not `allow_attributes`. Prefer
    `expect` by convention (see Standards).
  - The counts disagree: the brief said 5, lints.md says 6 while listing 7,
    and dead-code counts 10 (7 plus 3 `cfg_attr`). dead-code's inventory is
    the complete one.
- **C6. `result_large_err`.** errors.md keeps the tungstenite `allow`s
  narrow, and makes boxing `Failure` optional and lowest priority. lints.md
  boxes `Failure` "if it is the cause" and turns the rest into `expect`s.
  The lint only fires above 128 bytes, so the measurement lints.md asks for
  is already answered. errors.md finds no performance problem, and boxing
  adds an allocation. **Resolution:** all 13 become
  `#[expect(clippy::result_large_err, reason = …)]` (task 42), and `Failure`
  stays as it is unless George prefers the box.
- **C7. Archive names and case.** lints.md says
  `case_sensitive_file_extension_comparisons` at `install.rs:112` means
  "archive names from a remote are matched case-sensitively today". They are
  not: `unpack` lowercases the name first (`install.rs:104-108`). This is a
  false positive. **Resolution:** silence it in task 3, which rewrites that
  function anyway, and do not count it as a bug.
- **C8. App.tsx order inside window.md.** Task 8 (triage the dependency
  lists) comes before task 9 (split `App()`). Finding 2, and the brief's
  move-first rule, say split first. **Resolution:** split first (task 33),
  then triage (task 48).
- **C9. Tests first, or renames first.** tests-and-docs says "tests first";
  structure says "renames first". **Resolution:** the security proofs
  (task 1) go first, then dead code, then the renames and splits. Test
  plumbing fixes follow the split of the files they touch (37, 38, 45). Fixes
  in files C does not split (43, 44) can run alongside C.
- **C10. Narrowing `lib.rs`.** structure.md puts it at step 19, after the
  splits. The brief says dead code goes before files move. It touches only
  `lib.rs` and the items the compiler then reports. **Resolution:** move it
  to task 9.
- **C11. `security.md:1426` has a near match.** tests-and-docs says "no near
  match". The test was renamed in a8c71102, and its assertions no longer
  separate voice from typed replies (task 1 table). **Resolution:** write
  the discriminating proof.
- **C12. `security.md:683` is a policy change, not a rename.** tests-and-docs
  offers the renamed test as the nearest match. The rename went with a
  deliberate exposure of the goal that `security.md` never recorded.
  **Resolution:** George decides (task 1).
- **C13. Boot shared versus boot fixed.** structure F13 unifies the two boot
  sequences. errors F1 and F3 make each one fallible. Done separately, the
  error work is done twice. **Resolution:** one task (34).
- **C14. `unreachable_pub` and the library question.** lints.md defers
  `unreachable_pub` until "is `hotline-core` a library?" is answered.
  structure F12 answers it: the app, the CLI and the harnesses use its
  `pub` items, and nine modules have no outside user. **Resolution:**
  narrow those nine (task 9), and leave `unreachable_pub` off.
  `redundant_pub_crate` stays off too, since `pub(crate)` is used as
  documentation.
- **C15. A non-move inside a move.** structure's task 9 decides the five
  `Room` constructors inside a move-only task. **Resolution:** make it its
  own commit (22a).
- **C16. Version.** `[workspace.package] version` is `0.39.0` on the 0.40
  branch (lints.md). This is outside Phase 2: the release process sets it.

## Standards

This is what `docs/standards.md` should say (task D-6), drawn from the
reports' conventions.

**Module layout**

- Rust 2018 `foo.rs` beside `foo/`, and never `mod.rs`. The one exception is
  `crates/*/tests/common/mod.rs`, Cargo's idiom.
- `#[path]` is not used.
- One type's behaviour may be filed by concern across sibling files, the way
  `impl Room` is spread over `session/*`. Do not invent per-concern traits or
  types for that.
- `contract` is a leaf: it imports nothing from the crate. ts-rs types
  elsewhere are fine, as long as `contract.ts` stays byte-identical under an
  unfiltered `cargo test -p hotline-core`.
- A module is `pub` only when the app, the CLI, the harnesses or examples
  name it. Otherwise it is `pub(crate)`.
- No new crates. Revisit when an incremental
  `cargo test -p hotline-core --no-run` passes 30 s, or the contract gets a
  second consumer.
- Do not break the `session`↔`driver`, `session`↔`voice` and `mcp`↔`session`
  cycles with single-implementation traits.

**Test layout**

- Tests stay in-file in `mod tests` while they are under 400 lines and
  smaller than the code they test.
- Past that, they move to a sibling `foo/tests.rs`.
- Past 2,000 lines, `foo/tests.rs` keeps the shared fixtures only, and the
  tests move to `foo/tests/<topic>.rs`.
- There is no `_tests.rs` suffix. Integration harnesses are
  `crates/*/tests/<area>.rs`, one binary per area, with shared helpers in
  `tests/common`: one bounded `Client`, one desk-and-door helper, one fake
  model server.
- Wait on a signal through one `until(what, cond)` with one generous
  deadline. A test asserts floors and order, never an elapsed-time ceiling.
- Time a test controls runs on tokio's paused clock. A real wait is for a
  real child process or a real kernel buffer.
- Tests reach the desk over loopback. They never take a host interface or
  port 8788; they get a bound listener instead.
- Scratch space is `tempfile::TempDir`.
- An opt-in test is `#[ignore = "needs …"]`, never an early return. A
  missing tool fails by name.
- `docs/security.md` cites only tests that exist. Renaming a cited test, or
  changing its assertions, means updating the row in the same commit.
- In the window, `ui/tests/` stays flat. `.ts` is pure and `.tsx` uses the
  DOM, through one `tests/dom.ts`, and the tests are typechecked.

**File budget**

- Production code: aim for 1,000 lines per file; 1,500 lines triggers a
  split.
- Test files: 2,000 lines.
- Functions: 250 lines. `too_many_lines` is an audit lens, not a lint.
- The window has no budget of its own in the reports. My proposal is to use
  the same numbers: `App()` at 780 lines and three 1,800–2,000-line
  components are this audit's findings. `voice/call.ts`'s `Call` class is the
  stated exception.

**Error and panic policy**

- Fallible external setup returns an error to one outer handler and exits
  unsuccessfully. That means disk, sockets, the TLS and HTTP client, the
  runtime, signals, entropy and the window shell. It never starts an empty
  room, regenerates unreadable settings, or falls back to weak entropy.
- An impossible state is an `expect` that names why it holds, not a bare
  `unwrap`. `unreachable!` names the narrowing that justifies it. Keep the
  guard beside the access, and add no generic safe-index helper.
- A lock is taken with `unwrap_or_else(PoisonError::into_inner)` through the
  module's `lock` helper, in destructors too. Once recovered, state that an
  interrupted mutation could break, such as an authorization record, is
  revalidated or cancelled before more work is admitted.
- A typed error is kept wherever a caller branches on it: retry, status,
  fallback, or revoked versus lost. `String` is for a terminal sentence at a
  boundary, and `io::Result` where callers read `ErrorKind`. There is no
  global error framework and no workspace `Result` alias.
- A person sees a few-word action label with sanitized detail behind a
  working disclosure. Diagnostics never go in tooltips (decision-2). Notice
  shapes on the tape stay byte-compatible.
- `serde_json::Value` reads may index. A mutable or map index needs its key
  proven first. Build scripts and examples may panic.

**Lint policy**

- Levels live in `[workspace.lints]`, and each crate declares
  `[lints] workspace = true`. `make check` keeps `-D warnings`.
  `rust-version` is set.
- rustfmt runs on defaults, with no config file.
- Clippy's defaults, plus chosen pedantic and restriction lints, adopted one
  by one, never as whole groups. lints.md's "not worth it" list stays off.
  Blanket `unwrap_used`, `expect_used`, `indexing_slicing` and the print
  lints are review lenses, not lints.
- A suppression is `#[expect(lint, reason = "…")]`, so that it fails when
  the reason goes away. `#[allow(…, reason = "…")]` is only for code compiled
  into several binaries that use different parts of it, or code whose use
  depends on `cfg`. Every suppression has a reason.
- `too_many_arguments` is fixed by grouping values that already travel
  together, not by adding a layer.
- Generated output is never lint-fixed by hand.
- In the window, Biome is lint only, with no formatter, using window.md's
  rule set. `useExhaustiveDependencies` is an error. An effect that re-runs
  on purpose says so in a comment. ESLint waits until typescript-eslint
  supports TypeScript 7.
- A tool joins `make check` only once the tree passes it. An upstream
  exception is listed by ID with a review condition.

**Dependencies** (from dead-code-and-deps.md; not in the brief's list, but
the gates need it)

- Each crate declares its SPDX license.
- Advisories are fixed on each owning path. A major version is never forced
  under an API that cannot take it, and a lockfile override is not proof
  for a bundled copy.
- A heavy dependency stays while it does a job nothing lighter does.
