# Tests and docs drift — 0.40 audit

Read-only audit of `f2532066` on `release/0.40.0`, 2026-10-17, task-108.
Nothing outside this file changes here; every mismatch below is listed, not fixed.

How it was measured: one full `RUSTC_BOOTSTRAP=1 cargo test --workspace --
-Z unstable-options --report-time` run on an M-series Mac with
`HOTLINE_DATA_DIR` pointed at a scratch directory (debug build, 1 m 28 s to
build). It passed: about 2,000 tests, 11 ignored (all with a reason), 97.6 s
wall. Flaky patterns are read from the source, not reproduced; a pattern is
listed when it fails under the same conditions the two 2026-10-17 fixes
(`a1cd577d`, `1859ec74`) failed under: suite-wide load, or a host network
the test did not choose. Doc claims were checked against the code by grep, by
diffing every `Command` rename in `crates/hotline-core/src/contract.rs` against
the docs, and by resolving every relative link in `README.md`, `docs/` and
`docs-site/src/content`.

## Findings

Ranked by severity, then effort. "Small" is under an hour and one file or
two; "medium" is a day; "large" is a change to shared test plumbing.

### Tests

**T1. High, medium: every remote harness test reaches the desk through the host's first real address.**
`crates/hotline-core/src/remote/tests.rs:115` is `self.remote.status().endpoint.unwrap()`,
and the endpoint is the first address `network::addresses()` keeps:
`crates/hotline-core/src/remote/network.rs:8` refuses loopback, and the
default-route interface is put first. Under a full-tunnel VPN that is the
utun address, exactly the address `1859ec74` found unreachable; offline or
loopback-only (a CI container) the `unwrap` panics. Every `Harness` socket
goes through it (`remote/tests.rs:147`, `:163`, `:1673`), so the whole
`remote::tests` tree (33 `Harness::new()` in `tests.rs` alone, plus
`relay_tests.rs`, `admission_tests.rs`, `link_tests.rs`, `viewer_*_tests.rs`)
is network-dependent. The CLI harnesses do the same with `"host": "all"`
(`crates/hotline-cli/tests/serve.rs:268`, `crates/hotline-cli/tests/relay.rs:164`).
Fix: a test-only loopback endpoint for the harness; keep real-interface
coverage in the one test that is about interfaces.

**T2. High, small: the 15 s `ANSWER` from `a1cd577d` covers one module; its siblings still wait 1–3 s.**
`ANSWER` is private to `mod sealed_network` (`remote/tests.rs:1640`). The same
TLS-plus-Noise round trips elsewhere wait 1–3 s for something that must
arrive: `remote/tests/relay_tests.rs:33`, `:99`, `:168`, `:364`, `:458`,
`:615`; `remote/tests/admission_tests.rs:118`, `:152`, `:314`, `:461`, `:620`
(and 1 s at `:381`, `:509`, `:519`); `remote/tests.rs:823`, `:1868`, `:2214`,
`:2988`; `remote/tests/viewer_file_tests.rs:453`. Outside `remote/`, the same
shape sits in `driver/rig/turn.rs:1006`, `:1268`, `:1821`, `:2099`;
`voice/speech/xai/tests.rs:762`, `:805`, `:888`; `wire/tests.rs:1449`,
`:4416`. Fix: hoist `ANSWER` to `remote/tests.rs` as `pub(super)` and use it
wherever the test expects arrival; keep short timeouts only where the test
asserts that nothing arrives.

**T3. High, small: four tests `docs/security.md` cites as proof do not exist.**
security.md is the index of the tests that prove a capability (AGENTS.md), so
a dead name is a missing proof:
- `docs/security.md:330` `chatgpt_images_status_requires_owner_selection_without_checking_entitlement`;
  the test is now `chatgpt_images_status_is_automatic_without_checking_entitlement`
  (`crates/hotline-core/src/wire/tests.rs:5416`), and the claim beside it
  ("proves selected status") is now wrong too.
- `docs/security.md:683` `teammate_discovery_never_derives_public_fields_from_private_instructions`;
  nearest is `teammate_discovery_never_derives_the_technical_fields_of_a_persona_record`
  (`crates/hotline-core/src/mcp/server.rs:1415`).
- `docs/security.md:703` `a_computer_from_before_secrets_is_named_only_when_something_was_granted`;
  nearest is `a_computer_from_before_secrets_leaves_the_conversation_alone`
  (`crates/hotline-core/src/session/tests.rs:2626`).
- `docs/security.md:1426` `session::tests::only_the_reply_to_a_voice_handoff_gets_a_summarised_push`;
  no near match. Either the proof is gone or it was renamed past recognition.
Fix: rename the three, re-read each row's claim against the renamed test, and
find or write the fourth.

**T4. High, large: the WebSocket test `Client` exists eight times, and seven copies can hang forever.**
`crates/hotline-core/tests/desk.rs:194` (the base), `folders.rs:486` and
`steering.rs:638` (identical), `processes.rs:633` (97% the same),
`threads.rs:288` (adds an inbox dump on failure), `mcp.rs:67` and
`schedule.rs:37` (subsets), and `tests/common/stalled.rs:145`, the only one
with a bounded `call(…, patience)`. In every other copy `call()`/`read()` has
no deadline (`desk.rs:214-241`), so one lost reply hangs the test binary
rather than failing it. `tests/voice.rs:17-33` and `:1644` keep a fourth
style, an `until` that discards frames it doesn't match. Fix: move
`common/stalled.rs`'s Client to `tests/common/mod.rs`, add `threads.rs`'s
diagnostic and `desk.rs:258`'s `ok` assertion in `subscribe`, delete the rest.

**T5. Medium, small: the slowest tests wait on the real clock, and no test uses tokio's paused time.**
The top of the timing run:

| s | test | why |
|---|---|---|
| 30.3 | `remote::tests::viewer_token_tests::viewer_tokens_expire_thirty_seconds_after_minting` | `tokio::time::sleep(30 s)`, `remote/tests/viewer_token_tests.rs:172` |
| 12.1 | `acp_startup::a_stalled_agent_start_fails_visibly_and_holds_up_nobody_else` | waits out a real start deadline, `tests/acp_startup.rs:23-27` |
| 10.9 | `remote::tests::relay_tests::revoking_a_relay_host_releases_a_joined_visit_while_its_peer_is_not_reading` | fills a socket buffer until it blocks |
| 10.0 | `voice::speech::xai::tests::a_provider_that_never_becomes_ready_hits_the_connection_deadline` | the real 10 s `CONNECT_TIMEOUT`, `voice/speech/xai.rs:43`, `xai/tests.rs:819` |
| 7.6 | `session::exchanges::tests::pause::every_pause_is_a_new_decision_on_both_tapes_and_stop_settles_the_queue` | real pacing |
| 5.4 | `session::exchanges::tests::delayed_boot_recovery_preserves_a_new_long_running_handoff_and_its_real_result` | `sleep(5.3 s)` to cross the recovery deadline, `session/exchanges/tests.rs:676` |
| 5.3–5.4 | three `admission_tests` / `sealed_network` expiry tests | real pre-auth lifetimes |

The core library's unit binary takes 36.4 s, and the 30 s sleep alone is
most of it. A grep of `crates/` for `start_paused` and `time::pause` finds
nothing, and the workspace's tokio has no `test-util` feature
(`Cargo.toml:16`, `crates/hotline-core/Cargo.toml:14`). Fix: enable
`test-util` for dev and give the viewer-token and xai-deadline tests a paused
clock; together that is about 40 s of the suite's 98 s.

**T6. Medium, small: upper bounds on elapsed time.**
`crates/hotline-voice/src/speech.rs:421` already states the house
rule ("assert the floors and the order, never a ceiling"). These break it:
`providers/copilot.rs:800` (< 50 ms), `websearch/tests.rs:896` (< 400 ms) and
`:942` (< 600 ms), `voice/tests.rs:2713` (≤ 200 ms),
`crates/hotline-app/src/shell_path.rs:248` (< 1 s), and
`crates/hotline-core/tests/processes.rs:95` (< 2 s against a `sleep 2`
command, so the margin is one whole turn). Fix: widen the gap (a `sleep 30`
command) or assert order instead of time.

**T7. Medium, small: env-gated tests return early and report as passed.**
`crates/hotline-core/tests/desk.rs:926` (`HOTLINE_HARNESS_ANTHROPIC_KEY`),
`desk.rs:1041` and `:1195` (`HOTLINE_HARNESS_ACP`),
`tests/local_speech.rs:428` (`HOTLINE_TEST_SPEECH_MODEL`), the sandbox probe
`tools/shell.rs:585-595` (about ten callers) and its copy at
`tests/desk.rs:1273-1282`, and `tools/shell.rs:968`, which `continue`s past
each missing toolchain and so checks nothing where none is installed. The
other eleven opt-in tests use `#[ignore = "…"]` and show as ignored. Fix: the
same `#[ignore = "needs …"]`, run with `--ignored` where the env is set.

**T8. Medium, medium: sleep-then-assert that something happened.**
These fail under load: `computer/gate.rs:560` and `:766` (600 ms against a
400 ms lease), `session/jobs.rs:1140` (500 ms for `echo done`),
`session/sides.rs:2706` (30 ms), `voice/tests.rs:837` and `:868` ("let the
worker start", 30 ms), `driver/rig/turn.rs:2097` and `:2195` (100 ms; if the
turn isn't in backoff yet, the test silently exercises another path),
`tests/voice.rs:1031`, `:1168`, `:2456`, and `session/sides.rs:2064-2073` (5 ms
gaps so timestamps differ). Fix: wait on the signal (state, Notify, file)
with a deadline-based `until`.

**T9. Medium, medium: poll loops with tight fixed budgets, and nine poll helpers.**
`session/peers/tests.rs:41-53` polls 100 × 1 ms; `session/tests.rs:5963`,
`:6087`, `:6270`, `:6330` poll 500 × 2 ms; about 25 loops poll 200 × 10 ms
(`session/tests.rs:567`, `:2062`, `:4602`; `driver/acp.rs:4408`, `:5537`).
The helpers disagree on patience: `until` is 5 s in
`session/exchanges/tests.rs:34`, 2.5 s in `session/threads.rs:309`, 10 s in
`session/links/tests.rs:139` and 30 s in `remote/tests/link_tests.rs:106`,
beside `settled`, `until_awaiting`, `until_state`, `until_written` and
`heard`. Fix: one `test_support::until(what, cond)` with one generous deadline,
in `link_tests.rs:106`'s named form.

**T10. Medium, small: harness tests need `python3` or `perl` and don't say so.**
`crates/hotline-core/tests/processes.rs:28` and `tests/voice.rs:2826` start
`python3 -m http.server`; `tests/processes.rs:385` uses perl. Missing, they
time out after 15 s with a message about the server, not the binary. And
`tools/shell/macos.rs:476` asserts a python3 command *fails* inside the
sandbox, which it also does when python3 isn't installed. Fix: a Rust
listener, or check for the binary and fail with its name.

**T11. Medium, medium: the free-port race.**
Eleven sites bind port 0, read the port, drop the listener and rebind later:
`remote/tests.rs:62-68` (every `Harness`), `remote/tests/relay_tests.rs:12`,
`tests/processes.rs:387`, `tests/voice.rs:2818`,
`crates/hotline-cli/tests/serve.rs:33`, `:187`, `:265`, `relay.rs:120`,
`:164`, `pair.rs:60`, `:243`. It is deliberate: port 0 in
`remote/network.rs:66-69` tries `DEFAULT_PORT` 8788 first, the user's real
Hotline port, which a test must never take. No test binds a literal port.
Fix: let `Remote` and `ServeOptions` take a bound `TcpListener` in tests.

**T12. Low, small: one PATH mutation for a whole test binary.**
`crates/hotline-core/tests/common/mod.rs:67` `set_var("PATH", …)` (the only
`set_var` in test code) gives every test in `processes.rs` and `voice.rs` the
fake `open`, and leaks a kept temp dir (`common/mod.rs:55`). It is sound
under std's lock, as its comment says, but it is a global fake. Fix: pass
the opener through the desk's config.

**T13. Low, medium: scratch directories by pid or clock, never removed.**
About twenty `scratch()` copies name a temp dir by test name and pid
(`wire/tests.rs:874`, `room.rs:618`, `log/tape.rs:104`, `vault.rs:1220`,
`store/search.rs:864`, `tests/desk.rs:171`, `tests/schedule.rs:19`) or by
clock (`driver/acp.rs:3354`, `mcp/server.rs:1304`, `session/tests.rs:458`,
`tests/mcp.rs:45`). None collides today; all leak. Jobs in tests write
oversized output into the shared system temp dir by job id
(`session/jobs.rs:866`, `driver/rig.rs:2053`, `driver/rig/turn.rs:982`), and
`voice/realtime/live/tests.rs:161` uses a fixed `hotline-no-login`. Fix:
`tempfile::TempDir`, already a dependency.

**T14. Low, small: "start a desk and its door" and "serve a fake model" are inlined twenty times.**
`Door::bind` plus `tokio::spawn(door.run())` is written out in
`acp_shutdown.rs:25`, `acp_startup.rs:41`, `steering.rs:59`, `threads.rs:51`,
`folders.rs:131`, `processes.rs:465` and seven places in `voice.rs`, beside
three helpers (`desk.rs:170-192`, `mcp.rs:44`, `schedule.rs:18`) and nine
`const TOKEN`s. A fake model server on `127.0.0.1:0` is copied about twenty
times (`common/stalled.rs:139` is the one to keep). The CLI tests have no
`tests/common`: `free_address`, `desktop()`, `stop()` and `serve()` are copied
between `crates/hotline-cli/tests/serve.rs` and `relay.rs`.

**T15. Low, small: tests that check less than they say.**
`session/ledger.rs:342` (`…_is_a_no_op`) only calls `mark_verified` and
proves no panic, not no change. About thirty sleep-then-assert-nothing tests
(`session/tests.rs:1334`, `session/exchanges/tests.rs:231`,
`tests/processes.rs:51`, `tools/shell.rs:636`) pass vacuously under load;
`tools/shell.rs:636` also calls `std::thread::sleep` inside a tokio test.
Twenty-nine `yield_now`/`spin_loop` loops (`admission_tests.rs:120`,
`session/tests.rs:1058`) burn CPU the parallel suite needs.

### Docs

**D1. High, small: `docs-site/src/content/docs/server/operate.md:55-58` tells a server owner to update by hand.**
It says to replace `/usr/local/bin/hotline` and restart. `hotline update`
exists (`crates/hotline-cli/src/update.rs`), the installer prints "update
with: sudo hotline update" (`site/public/install:455`), and
`server/install.md:60-82` documents it. Fix: point to `sudo hotline update`.

**D2. High, small: `docs/wire.md` misstates four seat rules.**
- `docs/wire.md:941-943`: the phone "never opens the room stream or a run";
  `crates/hotline-core/src/wire/mod.rs:836-849` lets it open `Run` and
  `Side`, and `wire.md:818` itself says any thread kind but a call.
- `docs/wire.md:150`: `peers.answer_permission` "Owner seat only"; the local
  desk may too (`wire/mod.rs:698`).
- `docs/wire.md:250-255`: `unavailable` is a sentence naming what is
  missing; to a companion it is swapped for "Not available on this desk. Its
  owner can see why." unless it is one of `PLAIN_REASONS` (`wire/mod.rs:660-662`, `:1704-1717`).
- `docs/wire.md:132`: `session.set_config` carries no phone rule; a phone may
  set only an effort config (`wire/mod.rs:1680-1691`).
Minor: `wire.md:136`, `:158` and `:819` say "every seat", which the link
seat (`ping` and `federation.*` only, `wire/mod.rs:699`) is not.

**D3. Medium, medium: `docs/wire.md` omits about forty commands and several wire features.**
Not named in any doc: `agent.auth.start|poll|input|cancel`, `avatar.generate`,
`computer.update`, `link.preview`, `mobile.attachment`, `mobile.persona_access`
(and its hello capability `personaAccess`, `wire/mod.rs:795`),
`skills.list|add|remove|offer`, `websearch.status|set_enabled|set_key`
(`crates/hotline-core/src/contract.rs:3194-3689`). Named in prose with no
table row: `mobile.prompt`, `mobile.push_register`, `tape.page` (and its
400/2000/40 limits, `wire/mod.rs:2152-2155`), `teammates.exchange_resume|stop`,
`models.manual_set`, `credential.connect_local|custom_save|custom_models|login_cancel`,
`remote.relay`, `process.stop|output`, the seven `files.*` and
`computer.cookies.push` (only in serve.md). Also missing: the `{"call": id}`
target in the target list (`wire.md:707-719`, `contract.rs:3944`), the
subscription `window` field and the phone's eight-subscription cap
(`wire/mod.rs:1451`, `:1927`), that a tape snapshot is the last 400 lines
(200 / 512 KiB for a phone), not "that tape's fold" (`wire.md:714`,
`wire/mod.rs:2150`, `:2181-2183`), the hello capabilities `lean`,
`thumbnails`, `turnRetry` (`wire.md:959`, `wire/mod.rs:1792-1802`) and
`voiceFront` (`wire.md:1252`, `wire/mod.rs:818`), and the desk-only
`computer_pull` delta (`wire.md:731`, `contract.rs:2264`).

**D4. Medium, small: `docs/wire.md` param and field lists are short.**
`wire.md:135` `file.read` lacks `index?`, `size?` and makes `offset` required
(`contract.rs:3160-3172`); `wire.md:136` `avatar.read` lacks `size?`
(`contract.rs:3176-3186`); `wire.md:1268` `voice.call_start` lacks `mode?`
and `inputRate?` (`contract.rs:2961-2977`); `wire.md:791-793` `ThreadSummary`
lacks `withName?` (`contract.rs:3897`); `wire.md:840-865` `RosterEntry` lacks
`waiting`, `drawing`, `sides`.

**D5. Medium, small: the desk's account is stated three ways.**
`docs/serve.md:38-40` says the unit runs the desk as `hotline` and a CLI you
signed in to as yourself is invisible to it; `docs/serve.md:61-66` and
`docs-site/.../server/install.md:21-27` say the installer runs it as you
(`site/public/install:76-103`, `pick_user`). `server/troubleshooting.md:9-10`,
`:47-49` and `server/computers.md:20-21` tell the reader to sign harnesses in
as `hotline`. Fix: "the account the desk runs as", with the
`systemctl show -p User --value hotline` line.

**D6. Medium, small: `docs/development.md` miscounts its own harness.**
`docs/development.md:499` "`desk.rs` has four tests"; it has 23.
`docs/development.md:116` "Two of those tests talk to a real agent"; three are
env-gated (`tests/desk.rs:926`, `:1041`, `:1195`).
`docs/development.md:108-110` and `AGENTS.md:55` list what `make check` runs;
`Makefile:7-12` also fetches the speech engine, runs `bun run test` (via
`ui-check`) and both Python suites (`scripts/`, `site/tests`).

**D7. Medium, small: `docs/serve.md:494-495` documents the old `client::pair` signature.**
It "returns `PairedDesk { desk_id, name, url, desk_key, relay }`"; the free
function returns `Result<Paired, String>`, `Paired` being `Desk(PairedDesk)`
or `Relay(PairedRelay)`, and `PairedDesk` also has `public` and
`relay_declined` (`crates/hotline-core/src/remote/client.rs:23-37`, `:76`, `:99`).

**D8. Medium, small: `docs/threads.md` names code that is gone.**
`docs/threads.md:687` `Exchange::from_thread` does not exist (`Exchange` is
private at `voice/record.rs:124`, with no such function);
`docs/threads.md:297` keeps the queue loop in `run_side_turns`, which
`Room::run_queue` replaced (`session/turns.rs:285`), as `threads.md:466` says.
`docs/sessions.md:1478-1483` says the window draws only `side` and
`subagent` events of a thread; the window sends `threads2` now
(`docs/threads.md:690`) and gets `link`.

**D9. Low, small: ADR and release-note status.**
`docs/adr/0005-front-is-a-full-agent.md:3` still says "landing over epic-7";
epic-7 completed 2026-10-16 and 0.39.0 shipped it. `docs/adr/0005-front-is-a-full-agent.md:25`
credits "0.38.1", a version with no tag, no CHANGELOG entry and no
`docs/releases/0.38.1.md`. `docs/releases/` has no 0.40.0 note yet and
`CHANGELOG.md:9` holds 0.40's work under Unreleased, which is right for an
unreleased branch: a `docs/releases/0.40.0.md` is owed before
`desktop-v0.40.0` is tagged (`docs/development.md:126`).
`docs/adr/0006-relay-only-servers.md:3` "landing over epic-11" is accurate
until 0.40 ships.

**D10. Low, small: docs-site labels and lines that don't match the window or the CLI.**
`docs-site/src/content/docs/phone/pair.md:21` "**Revoke access**"; the button
is `Revoke` (`ui/src/components/RemoteSection.tsx:329`).
`server/relay.md:108-109` "marked **Relay only**"; the picker says `Relay`
(`RemoteSection.tsx:283`). `server/relay.md:13` shows
`hotline serve --capabilities relay` as if runnable; `--store`, `--listen`
and `--public-url` are required (`crates/hotline-cli/src/main.rs:99-105`).
`server/relay.md:84` runs `hotline pair --link` without the
`sudo -u hotline HOTLINE_DATA_DIR=…` form `relay.md:72` uses and
`crates/hotline-cli/src/room.rs:522-565` demands. `server/operate.md:14-15`
lists `status` output without `data`, `runs` or the relay-only form
(`crates/hotline-cli/src/door.rs:190-194`, `:236-244`); `server/operate.md:43-50`
omits `serve.json` and `pending.json` (`docs/serve.md:481-490` has both).

**D11. Low, small: undocumented surface.**
`hotline update [--check] [--version X.Y.Z]` and `hotline help` are in no
file under `docs/` (`crates/hotline-cli/src/main.rs:33`, `:153`), only the
docs site. `HOTLINE_EXTRA_DESKS` (`crates/hotline-app/src/desks.rs:280`) and
`HOTLINE_DEV_SIGN_IDENTITY` (`scripts/cargo-dev-sign:36`) are read and
documented nowhere; the live-test switches (`HOTLINE_LIVE_XAI_KEY`,
`HOTLINE_TEST_CHATGPT_DIR`, `HOTLINE_COOKIE_TEST_BROWSER`, …) only in source.

## Leave alone

- **The eleven `#[ignore = "…"]` tests.** Each one reaches a real browser, OS
  keychain, public network, live provider or paid account, and each one gives
  its reason. That is the right way to keep an opt-in test; T7 moves the
  early-return tests onto the same pattern.
- **Sleep-then-assert-nothing (T15) as a class.** "Nothing happens for a
  while" has no signal to wait on; until a paused clock is in (T5), a sleep
  is the honest form. Fix the vacuous ones only where a paused clock fits.
- **Port 0 trying 8788 first (`remote/network.rs:66`).** Product behaviour: a
  user's desk keeps its port across restarts. The tests' dance around it is
  T11's to fix, not this.
- **`acp_startup`'s 12 s and the relay revocation's 11 s.** They prove a real
  deadline against a real stalled child and a real full socket buffer; a
  paused clock doesn't move a child process or a kernel buffer.
- **Doc links.** Every relative link in `README.md`, `docs/`, `docs/adr/`,
  `docs/releases/` and the docs site resolves, and every docs-site anchor has
  its heading. The `github.com/1broseidon` vs `1Broseidon` case difference
  (`server/install.md:38`) breaks nothing.
- **CLI commands in docs.** Every `hotline …` line in `docs/` and the docs
  site parses as `crates/hotline-cli/src/main.rs` expects, apart from D1 and D10.
- **The window's contract.** `ui/src/generated/contract.ts` carries exactly
  the `Command` set of `contract.rs`; this run left it unchanged.
- **Names in docs that are marked historical** (`TapeStore` at
  `docs/threads.md:597`, `RunWork` at `:625`, `threads.jsonl` in an open
  question at `:705`) and `NotAllowedError`, a WebAuthn DOM error.
- **The `security.md` rows citing `src/secrets.rs`, `src/passkeys.rs`,
  `src/browser.rs`, `tests/contract.rs`.** They are in the Hotline Computer
  repository, and the doc says so.

## Phase 2 tasks

Each is small, lands alone and passes `make check`. Tests first, since T3 and
T1 are what the suite claims and where it breaks.

1. **security.md proves what it cites (T3).** Rename the three moved tests in
   `docs/security.md:330`, `:683`, `:703`, re-word each row to the test's
   claim, and find or write the proof for `:1426`.
2. **One `ANSWER` for the remote tests (T2).** Hoist the constant to
   `remote/tests.rs`, `pub(super)`, and use it at every arrival wait listed
   in T2 under `remote/`.
3. **The remote harness connects over loopback (T1).** A test-only endpoint
   on `Harness`; the interface test at `remote/tests.rs:961` keeps real
   addresses and refuses only addresses that answered.
4. **A paused clock for the 30 s and 10 s waits (T5).** `tokio` `test-util`
   as a dev feature; `start_paused` on the viewer-token and xai-deadline
   tests. Check: the core lib binary drops below 10 s.
5. **Opt-in tests say they are skipped (T7).** `#[ignore = "needs …"]` on
   the five early-return tests, and the shell toolchain test fails when it
   checked nothing.
6. **No ceilings on elapsed time (T6).** The six bounds in T6, each widened
   or turned into an ordering assertion.
7. **One bounded test Client (T4).** Move `common/stalled.rs`'s Client into
   `tests/common/mod.rs` and switch `desk.rs`; one file per follow-up commit
   for the other six.
8. **The CLI tests share `tests/common` (T14).** `free_address`, `desktop`,
   `stop`, `serve` from `crates/hotline-cli/tests/serve.rs`.
9. **Missing interpreters fail by name (T10).** `processes.rs` and `voice.rs`
   check for `python3`/`perl`; the macOS sandbox test checks python3 exists
   before asserting it is refused.
10. **wire.md's seat rules match `wire/mod.rs` (D2).** The four rules and the
    "every seat" wording.
11. **wire.md lists every command (D3, D4).** Rows for the forty, the `call`
    target, `window`, the subscription cap, snapshot windows, capabilities,
    `computer_pull`, and the short param lists.
12. **The docs site updates a server with `hotline update` (D1, D10).**
    `server/operate.md` Update, status and file table; the relay and phone
    label fixes.
13. **The desk's account is stated once (D5).** `docs/serve.md:38-40` and the
    three docs-site pages.
14. **development.md counts its harness (D6).** Drop the test counts, list
    what `make check` runs, in `docs/development.md` and `AGENTS.md:55`.
15. **serve.md and threads.md name live code (D7, D8).** `Paired`, the
    `run_queue` note, `Exchange::from_thread`, and sessions.md's `threads2`.
16. **ADR 0005 status (D9).** "Accepted (0.39)", and the 0.38.1 credit read
    as 0.39.0 or explained.
17. **Document the CLI and env leftovers (D11).** `hotline update`/`help` in
    `docs/serve.md`, `HOTLINE_EXTRA_DESKS` and `HOTLINE_DEV_SIGN_IDENTITY` in
    `docs/development.md`.
18. **Signals, not sleeps (T8, T9).** One `test_support::until(what, cond)`
    with one deadline; then the T8 sites, one module per commit.
