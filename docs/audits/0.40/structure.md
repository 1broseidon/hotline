# Structure audit: Rust modules and layout (0.40)

Task-103, epic-12. Read-only: this file is the only change. Measured on
`release/0.40.0` at 375322cc, 2026-10-17, on an M-series Mac.

**What was measured.** Line counts per file, split into production lines and
lines after the file's `mod tests` (`wc -l`, awk). The module graph is built by
hand: every `crate::<module>` path in each top-level module's production code,
with test files and in-file test modules left out. Build times come from
`cargo check` and `cargo test --no-run` in this worktree's own target
directory, using `--timings` for the cold run. Churn is `git log --since=2026-09-01`
per file (1,153 commits in the tree in that time).

**The shape today.** `hotline-core` has 202 files and 179k lines, with
47.8k of those in test files. `hotline-app` has 4.6k lines and
`hotline-cli` has 2.8k. The four largest modules are `session/` (34.4k
lines, 12.5k of them tests), `voice/` (24.1k, 6.3k tests), `driver/`
(15.9k) and `remote/` (13.2k, 6.4k tests).

| Module graph (production code) | |
|---|---|
| Hub | `session` reaches 26 of the 37 other top-level modules; `computer` (77 paths) and `contract` (54) most |
| Composition root | `desk` reaches 18 modules and nothing reaches it, apart from `voice` tests |
| Leaves | `credentials`, `fence`, `images`, `process_windows`, `room_lock`, `spending` |
| Mutual pairs (24) | contract↔{imagegen, remote, thread, wire}; session↔{driver, imagegen, mcp, models, thread, vault, voice, wire}; driver↔{mcp, providers, tools, voice}; mcp↔{room, vault, wire}; voice↔{desk(test), models, wire}; remote↔wire; providers↔vault |

**Build times.** A crate split's only payoff is build time, so this is the baseline:

| Run | Time |
|---|---|
| `hotline-core` lib check, cold, its own unit only (`--timings`) | 16.6 s |
| lib check after editing a function body (`session/mod.rs`) | 3.3 s |
| lib check after adding a `pub fn` to `contract.rs` | 5.6 s |
| `cargo test -p hotline-core --lib --no-run`, incremental, same edits | 9.2–13.8 s |
| `cargo test -p hotline-core --no-run` (lib and 13 harness binaries), incremental | 12.3 s |
| `cargo test -p hotline-core --lib --no-run`, everything cold | 142 s (mostly dependencies) |

## Findings

Ranked by severity, then by effort (S is under a day, M is one to three
landable tasks, L is more). Severity reflects how much the problem costs the
agents changing this tree. Many agents work this tree in parallel worktrees,
so a file that every branch touches is a merge-conflict magnet, and that
cost comes on top of navigation.

### Decisions this audit makes

- **Module convention: Rust 2018 `foo.rs` + `foo/`.** It is already the
  majority shape for nested modules, used by 13 directories (`driver/acp`,
  `driver/rig`, `session/peers`, `vault`, `providers`, `import`,
  `credentials`, `sent`, `tools/shell`, `websearch/rank`, `voice/speech/xai`,
  `voice/realtime/live`, ...). Sixteen directories still use `mod.rs`. With
  `foo.rs` every tab has a distinct name, where today ten tabs are all called
  `mod.rs`. The one exception is `crates/*/tests/common/mod.rs`: that is
  Cargo's idiom, because `tests/common.rs` would be built as a test binary of
  its own.
- **Test layout.**
  - Tests stay in-file in `#[cfg(test)] mod tests { }` while they are under
    400 lines and smaller than the code they test. That covers about 100 of
    today's 114 in-file modules.
  - Past that, they move to a sibling `foo/tests.rs`, declared as
    `#[cfg(test)] mod tests;`.
  - A `tests.rs` over 2,000 lines becomes `foo/tests.rs` (shared fixtures
    only) plus `foo/tests/<topic>.rs`.
  - No `#[path]`, and no `_tests.rs` suffix.
  - Integration harnesses stay at `crates/*/tests/<area>.rs`, one binary per
    area.
- **File-size budget.**
  - Production code: aim for 1,000 lines per file; 1,500 is the line that
    triggers a split.
  - Test code: 2,000 lines per file.
  - Functions: 250 lines.
  - Eleven files go over the production budget today. Their splits are in the
    findings below.

### F1. `session/mod.rs` is 5,577 production lines, 4,009 of them one `impl Room` — High · M

Evidence: `crates/hotline-core/src/session/mod.rs:678` opens the
`impl Room` block, which runs to `:4687`. The file has had 95 commits since
2026-09-01, more than any other file. The session module already has a good
idiom for this: 17 submodules each hold an `impl Room` for one concern
(`session/sides.rs:554`, `session/peers.rs:550`, ...). `mod.rs` just never
followed it. It also builds `Room` through five telescoping constructors
(`session/mod.rs:679,692,710,719,729`).

The table lists what moves. Line ranges are today's.

| New file | Moves |
|---|---|
| `session.rs` (renamed from `mod.rs`) | Module docs and `mod` lines; `ProviderAuth`, `ProviderKeys`, `Agents`, `DeskAgents`, `FrontReach` (144–345); `Room` and its constructors and accessors (572–945, 3352–3446, 3884, 4462–4675); `lock`, `new_id`, `now_ms`, `timed*` (5474–5577). About 1,100 lines |
| `session/start.rs` | `start`, `start_gate`, `capability_*`, `current_session`, `start_now` (881–1080); `invalidate`, `revoke_session`, `forget`, `stop*`, `reattach*` (2026–2250) |
| `session/computer.rs` | `computer_at_start` through `attach_computer_when_idle`, `computer_in_a_thread`, `computer_setup_status`, `grant_computer`, `finish_grant` (1080–1435); `computer_capacity` through `sweep_computers` (3446–3884); `ComputerSwap`, `ComputerSetup`, `Driving`, `ComputerAtStart` and their helpers (5183–5292) |
| `session/secrets.rs` | `hand_secrets` through `secrets_changed` (1435–2026); `PasskeyArming` (553); `watch_passkey_arming` (4743); `describe_secret` (5166) |
| `session/prompt.rs` | `in_this_chapter`, `prompt*`, `deliver_into`, `handoff_queued`, `deliver_identified`, `nudge`, `retry`, `say`, `mark_read`, `react`, `open_link`, `dispatch`, `cancel` (2250–2668); `said`, `voiced`, `fold_said`, `event_of`, `PendingTool`, `stamped`, `through_quiet`, `stamp_scheduled`, `scheduled_wire_text` (4827–5166) |
| `session/driver_info.rs` | `watch_driver_info`, `watch_wakes`, `watch_unprompted`, `refresh_session_models`, `apply_driver_info`, `set_model`, `set_config`, `set_mode` (2668–2861); `follow_model_changes` (4769) |
| `session/asking.rs` | `answer_permission`, `request_human` through `human_card` (2861–3352); `HumanWait`, `HumanAnswered`, `human_outcome` (4687–4743) |
| `session/chapters.rs` (exists; 605 production lines) | Adds `start_fresh_chapter` through `went_quiet` (3896–4290) and `sweep_idle_chapters` (4791) |
| `session/preamble.rs` | `preamble`, `reach_sentence`, `folders_sentence`, `skills_index`, `CLOCK` (5292–5474, 5519) |

The five constructors become one `Room::new` that takes a small options
struct, or they stay as they are. Decide this in the `start.rs` task, not
earlier.

### F2. `driver/acp.rs` is 3,183 production lines and 2,637 in-file test lines — High · M

Evidence: `crates/hotline-core/src/driver/acp.rs:3184` opens `mod tests {`.
The file's 31 tests share six scripted fake agents
(`scripted_agent` :3400, `recovery_agent` :3597, `subagent_agent` :3896,
`holding_agent` :4111, `steering_agent` :5320, `autonomous_agent` :5635).
`acp.rs:30` declares a sibling `auth_tests.rs` from the parent rather than
from `acp/auth.rs`.

| New file | Moves |
|---|---|
| `driver/acp.rs` | Docs; constants; `StartBounds`; the process-group registry `end_every_agent` and `end_group` (136–164); `ChildAgent` and `new`/`with_mcp`/`kill_child` (311–505); `impl Driver for ChildAgent` (1253–1635). About 900 lines |
| `driver/acp/oauth_proxy.rs` | `OAuthProxy` through `proxy_error` (931–1246): the capability-checked loopback proxy, which deserves its own file for security review |
| `driver/acp/live.rs` | `Live`, `Unprompted`, `Session`, `OpenMessage`, `ToolLine`, `Scribe`, `Pen` and their impls (505–931) |
| `driver/acp/translate.rs` | `receive`, `between_turns`, `watch_unprompted`, `translate`, `translate_in`, `ask_permission`, `describe_request`, the `finished`…`usage_of` mappers (2379–2821), `steer_lane`, `operator_*`, `text_block` (3078–3171) |
| `driver/acp/session.rs` | The second `impl ChildAgent` (1635–2205: `spawn`, `open_session`, `prepare_oauth_proxies`, `declared_servers`, `publish_ledger`, `adopt*`, `apply_configs`); `connect`, `RawSessionUpdate`, `Announced`, `initialize_request` (2205–2379); `capabilities_of`, `Disposition`, `apply_configs_to_session`, `choices_of` (2833–2995) |
| `driver/acp/tests.rs` | The six fake agents and the shared helpers |
| `driver/acp/tests/{turns,oauth_proxy,session_new,config,steering,unprompted,auth}.rs` | Turns, subagents, parking and checkpoints (3200, 3674–4420); the proxy (3222); `session/new`, folders, callbacks and `AGENTS.md` (4420–5268); disposition and config (4885–5320); steering (5320–5635); unprompted work (5635–end); `auth_tests.rs` → `tests/auth.rs` |

### F3. `contract.rs` is 4,118 production lines, and is not a leaf — High · M

Evidence: `crates/hotline-core/src/contract.rs` has 153 `TS` derives and
94 commits since 2026-09-01. Despite the module's name, it imports from five
other modules: `use crate::thread::{ThreadId, ThreadKind}`
(`contract.rs:19`), `pub use crate::imagegen::ImageSettings` (`:24`),
`pub use crate::spending::{...}` (`:25`), `crate::thread::End` (`:1488`,
`:3911`), `crate::remote::DeviceRole` (`:3074`) and
`crate::linked::LinkedTeammate` (`:3107`). Twenty-two more exported types
live in nine other files (`spending.rs:18`, `linked.rs`, `thread/mod.rs`,
`import.rs`, `remote/{mod,client,relay,v2}.rs`, `imagegen/mod.rs`). That
contradicts `docs/development.md:396`, which says types are "defined once in
`crates/hotline-core/src/contract.rs`". The file already has eight
`// ---` section banners, which mark where it splits.

Proposed layout: `contract.rs` keeps the docs and `pub use <sub>::*;`, so
every `crate::contract::X` path still resolves. It gains these submodules:

- `contract/persona.rs` (27–391, 743–783): avatars, `Persona`, reach,
  policies, the tool ledger, web search, `PersonaDraft`.
- `contract/computer.rs` (391–743): computer, browsers, cookies, shared
  secrets, passkeys, releases.
- `contract/credentials.rs` (783–921).
- `contract/session.rs` (921–1146, and Backends and Skills from 2286–2401).
- `contract/transcript.rs` (1146–2049): `TranscriptEvent` and its parts,
  which are the tape's byte-compatible shape (AGENTS.md rule 4). The
  structs move unchanged.
- `contract/views.rs` (2049–2286): views and pushes.
- `contract/payloads.rs` (2401–2954): mobile chunks, voice, capabilities,
  spending.
- `contract/command.rs` (2954–3871): the `Command` enum, whole.
- `contract/replies.rs` (3871–4118).
- The tests (4119–end) go to `contract/tests.rs`.

**Check before landing either contract task.** All of these types write to
one `contract.ts`. ts-rs may order its output by declaration site. Both
tasks must show `git diff --exit-code ui/src/generated/contract.ts` clean
after an unfiltered `cargo test -p hotline-core`.

### F4. `voice/mod.rs` is 3,021 production lines, 2,234 of them one `impl Calls` — High · S

Evidence: `crates/hotline-core/src/voice/mod.rs:492` opens
`impl Calls {`, which runs to `:2726`. The file has had 51 commits since
2026-09-01. The new names below avoid the existing `spoken.rs`, `replies.rs`
and `record.rs`.

| New file | Moves |
|---|---|
| `voice.rs` | Docs, `mod` lines, `Services`, `CallSpeech`, `FrontSpeech`, `Origin`, `Call`, `Reply`, `Work`, `Calls` (79–492); `new`, speech models, `transcribe`, `status*`, `pipeline_status` (494–728); the public accessors (2511–2726). About 1,000 lines |
| `voice/call.rs` | `start*`, `start_call`, `thinking_again`, `subscribe`, `change`, `end`, `finish`, `hold`, `interrupt` (728–1046) |
| `voice/input.rs` | `utterance`, `text`, `queue_text`, `queue_utterance`, `resolve_speech`, `speech_for`, `connected`, `audio` (1046–1386); `validate_audio`, `mp4_duration` (2881–3021) |
| `voice/turn.rs` | `emit`, `system_line`, `run`, `hear`, `respond`, `hand_off`, `summary`, `narrate`, `answer`, `unspoken`, `rewrite`, `keep*`, `count`, `writer`, `front_*`, `record`, `pay` (1386–2154) |
| `voice/synthesis.rs` | `synthesize`, `say`, `say_reply`, `speaking`, `speak_sentence`, `synthesize_stream`, `clip` (2154–2511); `calling`, `begin_answer`, `speech_ready`, `goodbye`, `own_voice`, `sentences`, `take_sentences`, `spoken_lengths` (2730–2881) |

### F5. `wire/mod.rs` is 2,517 production lines with six concerns — High · S

Evidence: the whole file is production code. Its tests are the sibling
declared at `crates/hotline-core/src/wire/mod.rs:61`. The file has had 80
commits since 2026-09-01. `RoomHandle` (`:82`) is a 590-line trait of
defaulted methods.

| New file | Moves |
|---|---|
| `wire.rs` | Docs, `mod` lines, shared helpers (`same_secret`, `send`, `reply_to`) |
| `wire/room_handle.rs` | `RoomHandle` (62–669) |
| `wire/seat.rs` | `Seat`, `federation_command`, `phone_thread_command`, `phone_may_read` (669–901) |
| `wire/door.rs` | `Door`, `Behind`, `accept_again`, `serve`, `refuse`, the handshake (901–1195) |
| `wire/socket.rs` | `seated*`, lanes, `answer`, `command`, `read_command`, `reply`, `Refused`, `decline` (1195–1904) |
| `wire/subscribe.rs` | `subscribe`, `stream_events`, `delta_scope`, `delta_for` (1904–2050, 2234–2414) |
| `wire/phone.rs` | `public_snapshot`, `link_as_sent`, `phone_frame`, `phone_event`, `window_start`, `snapshot_for_seat`, `phone_budget`, `snapshot_shaped` (2050–2234) |
| `wire/roster.rs` (exists) | Adds `roster_entry` and the tail (2414–2517) |

### F6. The two 7k-line test files are the tree's merge hot spots — High · M

Evidence:

- `crates/hotline-core/src/session/tests.rs` is 7,183 lines with 131 tests,
  and has had 76 commits since 2026-09-01.
- `crates/hotline-core/src/wire/tests.rs` is 6,933 lines with 126 tests,
  and has had 81 commits in the same period.
- Nearly every behaviour change appends to one of these two files, so
  parallel branches collide at their ends.
- `session/mod.rs:5578` declares the fixtures `pub(crate) mod tests`
  because other modules' tests import them. Their path must stay
  `crate::session::tests`.

**`session/tests.rs`** keeps `Scripted`, `Fake`, `DeskKeys` and the helpers
(`room`, `tape`, `markers`, `kinds`, `settled`, `spoken_turn`; lines 1–612).
The tests move to `session/tests/`:

- `turns.rs`: 613–1416, plus 6620–6733 and 7164.
- `preamble.rs`: 1416–1655, plus the stamps at 7107–7164.
- `chat.rs`: 1647–2057, covering pacing, thinking, the phone report and
  typed or call replies.
- `subagents.rs`: 2057–2420.
- `computer.rs`: 2420–3723 and 6733.
- `scheduled.rs`: 3723–4418.
- `chapters.rs`: 4418–5038.
- `cards.rs`: 5038–5892.
- `runs.rs` and `sent_files.rs`: the existing inline `mod runs` (5892) and
  `mod sent_files` (6337).
- `voice.rs`: 6769–7107.

**`wire/tests.rs`** keeps `CoreHandle`, `Quiet`, `NoAgents`, `NoKeys` and
the door helpers (lines 1–983). The tests move to `wire/tests/`:

- `door.rs`: 983–1072 and 3009–3109.
- `roster.rs`: 1072–1670.
- `personas.rs`: 1670–1786, 2504–2537 and 3464–3804.
- `deltas.rs`: 1786–1985.
- `seats.rs`: 1985–2352, 2537–2719, 2944–3009 and 4120–4435.
- `files.rs`: 2352–2504.
- `schedules.rs`: 2719–2944.
- `models.rs`: 3109–3464.
- `settings.rs`: 3804–4120.
- `mobile_computer.rs`: 4435–4677.
- `owner_handler.rs`: 4677–5093.
- `capabilities.rs`: 5093–5820 and 6607–6678.
- `threads.rs`: 5820–6486.
- `windows.rs`: 6486–6607.
- `linked.rs`: 6678–end.

`remote/tests.rs` (3,406), `voice/tests.rs` (2,988), `tests/voice.rs` (2,958)
and `tests/desk.rs` (2,772) also exceed the test budget, but have half the
churn. They come last.

### F7. `wire/commands.rs::run` is one 801-line function — Medium · S

Evidence: `crates/hotline-core/src/wire/commands.rs:64` is
`pub(crate) async fn run(` and runs to line 865. It is a single `match` over
`Command`. The file is 1,782 production lines. Its siblings `wire/threads.rs`,
`wire/schedules.rs` and `wire/files.rs` already show the right shape, with
one area per file. Split it so that `run` only dispatches, at about 200
lines, and arms call into area files:

- `wire/commands/persona.rs`: the persona and `mobile_persona_*` functions
  (885–1478).
- `wire/commands/settings.rs`: `web_search_status`, `update_settings`,
  `server_ids` and `forget_removed_servers` (1478–1554).
- `wire/commands/models.rs`: `set_model`, `set_config` and the
  `write_last_model` helpers (1554–1772).

### F8. Six more files over the production budget — Medium · S each

- **`session/peers.rs`, 1,971 production lines**
  (`crates/hotline-core/src/session/peers.rs:550` opens `impl Room`, which
  runs to `:1482`).
  - `peers.rs` keeps `Peers`, the types and the free helpers (69–550,
    1484–1971).
  - `peers/delivery.rs` takes `deliver*`, `ask`, `checked`, `exchange`,
    `authorize_collaboration`, `allow_sender` and
    `write_collaboration_decision` (551–935).
  - `peers/sessions.rs` takes `answer_peer_permission`,
    `drop_peer_sessions`, `peer_threads`, `recover_exchanges`,
    `quiet_peers`, `peer_session` and `append_thread` (935–1482).
- **`tools/workspace.rs`, 1,659 production and 748 test lines**
  (`crates/hotline-core/src/tools/workspace.rs:88` opens `impl Workspace`,
  which runs to `:1024`).
  - `Workspace` and the path helpers stay together (88–1224). They are the
    confinement boundary, and a reviewer should read them in one file.
  - The Rig `Tool` impls `ListDirectory` through `EditFile` (1224–1659) move
    to `tools/workspace/rig_tools.rs`.
  - The tests move to `workspace/tests.rs`.
- **`session/sides.rs`, 1,640 production and 1,077 test lines**
  (`crates/hotline-core/src/session/sides.rs:554` opens `impl Room`).
  - `sides/ending.rs` takes `finish_side`, `park_side`, `archive_parked`,
    `remember_session`, `name_side`, `mark_side`, `side_summary`, `stored_*`
    and `last_words` (1276–1640).
  - The tests move to `sides/tests.rs`.
- **`computer/mod.rs`, 1,619 production and 661 test lines**
  (`crates/hotline-core/src/computer/mod.rs:235` opens `impl Computer`).
  - `computer/container.rs` takes the container-CLI layer:
    `pick_runtime`, `create_args`, the volume, cpu, memory and pids
    helpers, `mount_args`, `pull`, `PullTally`, `inspect*`, `parse_*`,
    `run` and `wait_healthy` (779–1497).
  - The tests and `fixtures` (1496) move out.
- **`driver/rig.rs`, 1,506 production and 1,154 test lines**.
  - `rig/build.rs` takes `effort_*`, `Reuse`, `anthropic_model`,
    `openrouter_model`, `agent_builder`, `copilot_responses` and
    `request_settings` (1008–1259).
  - `rig/history.rs` takes `images_to_model` through `flush` (799–1008).
  - `rig/tool_output.rs` takes `tell_person` through `describe_tool`
    (1277–1506).
  - The tests move to `rig/tests.rs`.
- **`driver/rig/turn.rs`**: within budget at 922 production lines, but it
  has 1,401 test lines, and `run_inner` is 469 lines
  (`crates/hotline-core/src/driver/rig/turn.rs:195`).
  - The tests move to `turn/tests.rs`.
  - `run_inner` splits into a streaming phase and a tool-dispatch phase. The
    retry-before-accept rule (`turn.rs:688`) must stay in one place.

### F9. Two module conventions and four test-file naming styles — Medium · S

Evidence:

- Module files: 16 `mod.rs` files (`session/`, `voice/`, `wire/`, `remote/`,
  `mcp/`, `computer/`, `tools/`, `store/`, `log/`, `thread/`, `skills/`,
  `imagegen/`, `websearch/`, `driver/`, `voice/speech/`,
  `voice/realtime/`) sit beside 13 `foo.rs` + `foo/` pairs.
- Test files come in four shapes:
  - `foo/tests.rs` (`session/peers/tests.rs`).
  - A `#[path]` attribute (`imagegen/chatgpt.rs:153`
    `#[path = "chatgpt_tests.rs"]`).
  - A parent declaring a child's tests (`driver/acp.rs:30` `mod auth_tests;`).
  - `remote/tests/*_tests.rs`, declared at `remote/tests.rs:2292` and
    `:3099–3103`.

The fix is the Decisions above, applied as pure renames.

### F10. The module graph has 24 mutual pairs, and `session` is the hub — Medium · M

Evidence: the table at the top. Cycles inside one crate compile fine. What
they cost is the option to extract anything later: `contract` reaches
`thread`, `remote`, `wire` (through a doc link only), `imagegen`, `linked`
and `spending` (F3), and `wire` and `remote` reach each other
(`remote` → `wire` 5 paths, `wire` → `remote` 1). The one cheap cut is
worth making now: move the wire types that `contract` borrows into
`contract`, which makes it a leaf. `ThreadId`, `ThreadKind`, `End`,
`DeviceRole`, `LinkedTeammate`, `LinkedDelivery`, `ImageSettings` and the
three spending types are all `TS` exports already. Leave the
`session`↔`driver` and `session`↔`voice` cycles alone (see Leave alone).

### F11. The module maps are stale, and there are two of them — Low · S

Evidence: `docs/development.md:45–72` and `docs/design.md:634–660` each
list the tree. Neither lists `voice/` (24k lines), `websearch/`,
`imagegen/`, `providers/`, `credentials/`, `spending.rs`, `push.rs` or
`sent.rs`, and only one lists `remote/`, `linked.rs` and `skills/`.
`docs/development.md:396` is wrong (F3). Keep one map, in
`development.md`, and have `design.md` link to it.

### F12. Every module is `pub`, and nine have no user outside the crate — Low · S

Evidence: `crates/hotline-core/src/lib.rs:14–48`. Nothing in `hotline-app`,
`hotline-cli`, `hotline-core/tests`, `examples` or `src/bin` names
`link_preview`, `linked`, `push`, `session`, `skills`, `spending`, `store`,
`thread` or `websearch`. ts-rs export needs no `pub` path. If they are made
`pub(crate)`, the compiler reports `pub` items nobody calls, which is free
dead-code detection. A crate boundary decided later would also start from
the true public surface.

### F13. The app and the CLI each write the desk's boot — Low · S

Evidence:

- **Boot.** `crates/hotline-cli/src/serve.rs:161–268` and
  `crates/hotline-app/src/lib.rs:307–362` run the same order: take the room
  lock (`instance::claim`, which is a `RoomLock`), `claim_backend`, open the
  `Desk`, open the `Remote`, `desk.set_remote`, `remote.restore()`,
  `Door::bind` and spawn `door.run()`.
- **Shutdown.** Both end with `driver::acp::end_every_agent()`
  (`serve.rs:260`, `lib.rs:577`).
- **Tokens.** Both mint a 32-byte hex token with different crates:
  `serve.rs:280` uses `getrandom`+`hex`, and `lib.rs:66` uses `rand`.

Both shells are real callers, so a core function earns its place. The
differences must stay visible in its arguments: the CLI has relay-only and
file-store modes, and its shutdown drains with `stop_for_restart`
(`serve.rs:246`), which the app's exit does not.

The rest of the app is already where it belongs. Every module that is free
of Tauri is specific to the shell:

- `instance.rs` brings the running window forward.
- `shell_path.rs` fixes the PATH of a Finder or desktop launch.
- `linux_package.rs` runs pkexec for the updater.
- `desks.rs` holds a paired-desk registry with the Tauri commands over it,
  and the shell is its only caller. Its save is already atomic, staged and
  renamed (`desks.rs:518–524`).

The CLI holds no domain logic: `door.rs`, `room.rs` and `remote.rs` are the
client side of the loopback Door.

### F14. Crate boundaries: none now — decision · —

Weighed one boundary at a time:

- **`contract` / wire types.** Blocked today by the cycles in F3 and F10.
  The ts-rs `export_to` paths would also move, and the gain is nil: a
  `contract.rs` signature change already costs 5.6 s to check and 13.8 s to
  rebuild the tests. Revisit once F3's leaf task lands, and only if a second
  consumer appears, such as a wasm or remote client that should not link
  core.
- **`voice`.** It reaches 15 modules, including `session`, `driver`, `wire`,
  `room` and `models`. A crate would need trait seams for each of them,
  which is new machinery with no requirement behind it.
- **`remote`.** It is cyclic with `wire` and reaches `computer`. Same
  answer.
- **`tools`.** Cheap, at 4.6k lines with a single edge to `driver`
  (`CapabilityLease`, `tools/workspace.rs:3`), and pointless: it is a small
  fraction of a 16.6 s unit.

The inner loop is already incremental: a body edit takes 3.3 s to check and
12.3 s to rebuild every test binary. Cold builds are dominated by
dependencies (142 s). A split would also churn every `use` across 202 files
while parallel branches are open.

**Revisit trigger:** an incremental `cargo test -p hotline-core --no-run`
above 30 s, or a second consumer of the contract.

## Leave alone

- **The 13 integration harnesses as separate binaries.** The incremental
  rebuild of all of them is 12.3 s. Merging them into one binary would save
  link time nobody waits on, and would cost `cargo test --test voice`.
- **`tests/common/mod.rs`.** It is Cargo's idiom, and the one exception to
  the `foo.rs` rule.
- **The `Command` enum as one 917-line item** (`contract.rs:2954–3871`). An
  enum cannot be split across files. It is the wire's index, and reads best
  whole in `contract/command.rs`.
- **The shapes of `TranscriptEvent` and the segment layout.** Moving files is
  fine. Changing a field is not (AGENTS.md rule 4).
- **The `impl Room` spread across `session/*`.** One type whose behaviour is
  filed by concern is the right idiom here. F1 extends it rather than
  inventing per-concern types or traits.
- **The `session`↔`driver`, `session`↔`voice` and `mcp`↔`session` cycles.**
  They are the room's live parts calling each other. Breaking them needs
  trait seams with one implementation each, which is the indirection
  AGENTS.md argues against.
- **The ~100 in-file test modules under 400 lines.** They are within the
  test layout rule, so moving them is churn without benefit.
- **Files between 1,000 and 1,500 production lines** (`desk.rs` 1,358,
  `session/exchanges.rs` 1,353, `mcp/oauth.rs` 1,305, `mcp/server.rs` 1,237,
  `vault.rs` 1,205, `voice/realtime/live.rs` 1,086, `voice/dispatcher.rs`
  1,006). They are within budget. The 337-line `mcp/server.rs:570` `call`
  and the 264-line `session/exchanges.rs:743` `exchange_step` are the next
  candidates, if the function budget is enforced.
- **`hotline-cli/src/update.rs` beside `hotline-app/src/updater.rs`.** They
  are two install mechanisms, a script-installed server and the Tauri
  plugin. All they share is a five-line semver comparison.
- **`hotline-app/src/desks.rs` in the shell.** It is free of Tauri, but the
  shell is its only caller. Move it when the CLI pairs as a client, and not
  before.
- **New crates.** See F14.

## Phase 2 tasks

Each task is one commit that builds and passes `make check`, moves code
without changing behaviour, and can land alone. Land them in this order: the
renames first, so later splits land on the new paths. Within a group the
order is free.

1. **Adopt `foo.rs` + `foo/`.** `git mv` the 16 `mod.rs` files to `foo.rs`.
   This is pure renames: `git diff -M --stat` shows 100% similarity, and no
   content changes. (F9)
2. **One test-file naming style.**
   - `imagegen/chatgpt_tests.rs` becomes `imagegen/chatgpt/tests.rs`, and
     the `#[path]` goes.
   - `driver/acp/auth_tests.rs` moves under the module it tests.
   - `remote/tests/*_tests.rs` become `remote/tests/<topic>.rs`.
   (F9)
3. **Make `contract` a leaf.** Define `ThreadId`, `ThreadKind`, `End`,
   `DeviceRole`, `LinkedTeammate`, `LinkedDelivery`, `ImageSettings` and the
   spending types in `contract.rs`, and update their callers; leave no
   re-export at the old path. `contract.ts` must be byte-identical. (F3, F10)
4. **Split `contract.rs`** into the nine submodules in F3, re-exported from
   `contract.rs`. `contract.ts` must be byte-identical. (F3)
5. **`session/mod.rs`, part 1:** `preamble.rs` and `asking.rs`. (F1)
6. **`session/mod.rs`, part 2:** `secrets.rs`. (F1)
7. **`session/mod.rs`, part 3:** `computer.rs`. (F1)
8. **`session/mod.rs`, part 4:** `prompt.rs` and `driver_info.rs`. (F1)
9. **`session/mod.rs`, part 5:** `start.rs`, the chapter methods into
   `chapters.rs`, and the decision on the constructors. (F1)
10. **`session/tests.rs`** into fixtures plus `session/tests/*.rs`, as in F6.
    The test count before and after must match (131). (F6)
11. **`wire/tests.rs`** into fixtures plus `wire/tests/*.rs`, as in F6
    (126 tests). (F6)
12. **`driver/acp.rs`, tests:** `acp/tests.rs` plus `acp/tests/*.rs`. (F2)
13. **`driver/acp.rs`, code:** `oauth_proxy.rs`, `live.rs`, `translate.rs`
    and `session.rs`. (F2)
14. **`voice/mod.rs`:** `call.rs`, `input.rs`, `turn.rs` and `synthesis.rs`.
    (F4)
15. **`wire/mod.rs`:** `room_handle.rs`, `seat.rs`, `door.rs`, `socket.rs`,
    `subscribe.rs` and `phone.rs`. (F5)
16. **`wire/commands.rs`:** `run` becomes dispatch only, and the arms move to
    `commands/{persona,settings,models}.rs`. (F7)
17. **One task each** for `session/peers.rs`, `tools/workspace.rs`,
    `session/sides.rs`, `computer/mod.rs`, `driver/rig.rs`, and
    `driver/rig/turn.rs` (its tests plus `run_inner`). (F8)
18. **Topic splits of the remaining large test files:** `remote/tests.rs`,
    `voice/tests.rs`, `tests/voice.rs` and `tests/desk.rs`. (F6)
19. **Narrow `lib.rs`.** Make the nine unused modules `pub(crate)`, and
    delete what the compiler then reports as unused. (F12)
20. **One boot function.** Give `desk` a core function that both
    `hotline serve` and the app call, plus one token minting function. The
    relay-only, file-store and drain differences stay as arguments. (F13)
21. **One module map** in `docs/development.md`, matching the tree after
    tasks 1–17, with `design.md` linking to it. Correct the "defined once"
    line. (F11)
