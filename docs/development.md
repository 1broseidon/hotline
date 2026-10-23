# Developing Hotline

What you need, how to run it, and where things are. The working contract
for changes — footguns, verification, taste — is [AGENTS.md](../AGENTS.md);
read that first. Why the tree is shaped this way is
[docs/design.md](design.md). The wire, the log and the session, as the
code implements them, are [wire.md](wire.md), [log.md](log.md) and
[sessions.md](sessions.md).

## Requirements

- [Rust](https://rustup.rs) stable. The workspace edition is 2024.
- The [Tauri CLI](https://v2.tauri.app/) as a cargo subcommand:
  `cargo install tauri-cli --version ^2`.
- Python 3, for release manifest validation in `make check`.
- cargo-machete 0.9.2 and cargo-deny 0.20.2, for the dependency gates in
  `make check`: `cargo install --locked cargo-machete@0.9.2 cargo-deny@0.20.2`.
  Another version stops `make check` with that line; the Makefile says why.
- [Bun](https://bun.sh), for the window's install, typecheck, Vite, and
  production build.
- On macOS, Xcode or its Command Line Tools: `build.rs` compiles the
  shell's on-device speech (`crates/hotline-app/macos/speech/`) with
  `xcrun swiftc`. Its `SpeechAnalyzer` path needs Swift 6.2 or newer
  (Xcode 26); an older compiler builds only the `SFSpeechRecognizer` path
  ([voice.md](voice.md#device-text-on-a-mac)).
- [CMake](https://cmake.org): the `opus` crate builds libopus from source
  for GPT-Live calls (`brew install cmake` on macOS; GitHub's runners have
  it).
- Python 3 and network access once per target, for the speech engine:
  `hotline-core` links sherpa-onnx's prebuilt static library, which the
  build takes only from `.speech-engine`
  (`SHERPA_ONNX_ARCHIVE_DIR` in `.cargo/config.toml`).
  `scripts/fetch-speech-engine` puts it there after checking its SHA-256
  against `scripts/speech-engine.sha256`; `make dev`, `check`, `build` and
  `verify` run it first, and so does every CI job that compiles Rust. A
  plain `cargo build` on a fresh checkout needs it run once by hand
  (`--target` for a cross build).
- On Linux, `libayatana-appindicator3` at runtime, for the tray, and its
  dev package (`libayatana-appindicator3-dev`) to bundle: the Tauri CLI
  finds the library through pkg-config before it writes the deb. A
  Homebrew `pkg-config` earlier on PATH searches only Homebrew's
  directories; point `PKG_CONFIG_PATH` at the system one
  (`/usr/lib/x86_64-linux-gnu/pkgconfig`) for `make build`.

`hotline-core` has no Tauri dependency. The shell crate is the only place
Rust that needs Tauri lives.

## Layout

```
crates/hotline-contract/src/  the wire's types (serde + ts-rs), a file per concern; a leaf crate
crates/hotline-support/src/   the utilities that hold no Hotline state; a leaf crate
  paths.rs                   the data directory's layout, and a linked teammate's id
  room_lock.rs               one writer per room
  credentials/               the OS credential store that holds the secret values
  images.rs                  what an attached or generated image is made into before anyone sees it
  thumbs.rs                  smaller copies of pictures for a phone
  fence.rs                   the one fence around anything quoted out of a conversation
  log_file.rs                where the app's own lines go when nobody reads its output
  process_windows.rs         a Windows child's kill-on-close job, and a sign-in console's job that outlives a normal exit
  private_dir.rs             a credential folder made private from its first instant, and the real-directory check before each use
crates/hotline-store/src/     what is kept on disk and rebuilt from it; on the two leaves and nothing else
  log/                       streams: append, fold, segments, subscribe; tapes and threads
  search.rs                  beside the tape: FTS5 search over every tape
  chapters.rs                a tape's chapter list
  previews.rs                a tape's last line, for the roster
  link.rs                    the line a thread leaves on its parent, where a thread stands, and the agent account its session ran on
  sent/                      files teammates send the person, and their readback
crates/hotline-tools/src/     a teammate's workspace and shell tools; on the two leaves and nothing else
  capability.rs              the session's lease every tool checks, and the epoch that revokes it
  workspace.rs               workspace tools on cap-std: the wall, granted folders, read, grep, edit
  shell.rs                   the shell command, confined: Seatbelt on macOS, bubblewrap on Linux
crates/hotline-websearch/src/ web search for every teammate, keyless by default, ranked and cleaned; on the contract alone
  exa.rs, parallel.rs, ...   the four providers, one file each
  clean.rs, rank.rs          error pages and chrome out, duplicates and mirrors merged
  query.rs, when.rs          what the query asked for: its language, its dates
  mock.rs                    a provider on localhost, for tests (the `test-support` feature)
crates/hotline-room/src/      the room as the code above the core sees it; on the leaves, the store and web search
  room.rs                    the roster fold, settings, and schedules
  mcp_servers.rs             the `mcpServers` setting, read the way the fold reads it
  agent_accounts.rs          the `agentAccounts` setting, read entry by entry
  linked.rs                  linked desks: their records, qualified ids, the roster that crosses
  room_handle.rs             RoomHandle, the seam the wire calls the live room through
  tool_ledger.rs             what tools a teammate got, from where, and why one is absent
  voice/                     voice's settings, its spend and the budget that gates it, spoken replies
  provider_auth.rs, ...      ProviderAuth, the secret and text helpers, the TLS default, the computer's home
crates/hotline-mcp/src/       Hotline as an MCP client; on the contract, the support, the tools and the room
  lib.rs                     the servers settings name, a teammate's grant, the live connections
  tool.rs                    one granted tool as Hotline Agent calls it, and its ledger row
  oauth.rs                   OAuth sign-in to an HTTP server, with what the vault keeps handed in
crates/hotline-voice/src/     calls and voice engines; the room is reached through CallRoom
  call_room.rs               the core's callbacks, with temporary operations labelled by removal slice
  lib.rs, call.rs, input.rs   call ownership, lifecycle and bounded input
  duplex.rs, front*.rs        live sound, the front's conversation and its leased tools
  dispatcher.rs              the legacy desk assistant and its metering
  turn.rs, synthesis.rs       turns, streamed replies and spoken clips
  record.rs, replies.rs       the call's record and spoken-reply counts
  speech.rs                  the Speech seam: a clip heard, a reply spoken, and the errors either gives
  speech/                    the providers' adapters, which one a call uses, the desk's own models
  realtime.rs                the Realtime seam: one live spoken conversation that renews itself
  realtime/                  Grok Voice and GPT Realtime over a socket, GPT-Live over WebRTC
crates/hotline-core/src/
  thread/                    the one name for a conversation, its store and its policy
  vault/                     credential metadata beside the room stream
  session/                   Session, the funnel, quiet, chapters, the scheduler, peer threads
  driver/                    Hotline Agent on Rig, and the ACP child with its registry and OAuth proxy
  accounts/                  an agent account: its agent's row, folder, env, status, sign-in, setup and switch
  models.rs                  the providers Hotline Agent reaches, and the model catalogue they serve
  providers/                 connection work Rig does not own; requests stay on Rig's clients
  capabilities.rs            which connected provider the owner can pick for each job
  pricing.rs                 what a model costs per token
  spending.rs                the spending ledger and the budget it enforces
  mcp/                       Hotline's own teammate tools, and the vault's side of the client
  skills.rs                  the catalog: built-in, gateway and workspace skills, copied into a workspace
  computer/                  runtime detection and one container per teammate
  voice/                     the room's CallRoom implementation and the vault's engine glue
  imagegen/                  making images over whichever connected provider the owner has
  link_preview.rs            a link's card, read from the page's head
  desk.rs                    the room, vault and log behind the wire
  boot.rs                    one boot, the app's and `hotline serve`'s, that returns its errors
  wire/                      the door: seats, commands, subscriptions, the phone and relay doors
  remote/                    Remote: the sealed listener, pairing, the relay, a linked desk's channel
  push.rs                    notifications to paired phones
  import.rs                  copies an existing Hotline data directory
  import/                    the previous edition's roster and records, read-only
  test_support.rs            what the unit tests of more than one module share
  bin/hotline-import.rs      the importer as a binary
  bin/hotline-mcp-echo.rs    a one-tool stdio server the MCP harnesses spawn
  bin/hotline-acp-fixture.rs a scripted ACP agent and its CLI, for the harnesses
  bin/hotline-models-sync.rs rewrites models.json from models.dev
crates/hotline-core/models.json  the model catalogue: a filtered snapshot of models.dev
crates/hotline-core/tests/   headless proofs driving the real core over the wire
crates/hotline-app/          the Tauri 2 shell: plugins, the menu, the door
crates/hotline-cli/          `hotline`: the desk with no window (`hotline serve`), and its operator commands
ui/                          the React window, built against the generated contract; ui/src/ui is the design system
assets/                      the mark and the app tile it sits on; `make icons` renders the icon set from them
docs/                        what is
```

The core is a library. The shell binds its door on a loopback port, injects
the port and a token into the page, and opens a window on it. There is no
child process for the room. Opening a `Desk` opens the room, which settles
every tape and every thread (expired permission cards, a compact, the
search index) and starts the idle chapter sweep and the scheduler's clock
before anything is served.

## Running it

```bash
make dev        # the Tauri shell, Vite with hot reload, on .hotline-dev
make check      # the window's typecheck, lint and tests, the dependency gates, the Python tests, cargo fmt, clippy -D warnings, cargo test
make verify     # the headless harnesses, driving the real core over the wire
make precheck   # what CI's runners would fail, caught here; see Before pushing
make build      # a release bundle under target/release/bundle (unsigned)
make icons      # every platform's app icon, and the tray marks, from assets/
```

A release is a tag `desktop-vX.Y.Z` whose version matches `Cargo.toml` and
`tauri.conf.json`; `.github/workflows/desktop-release.yml` builds each
target on its own runner, signs and notarizes the Mac bundles from the
repository secrets, and publishes one GitHub Release marked latest. A hand
run of the workflow builds and keeps artifacts only. `CHANGELOG.md` takes
an entry per version, and `docs/releases/<version>.md` its release notes.

`make dev` exports `HOTLINE_DATA_DIR` to `.hotline-dev` in the checkout, then
runs `cargo tauri dev` from `crates/hotline-app`, where `tauri.conf.json`
is. That starts Vite for the window on port 5174 and refuses any other
port, so a port already in use is a mistake rather than a second
instance. The shell generates a one-launch token, binds the door on
`127.0.0.1` with an ephemeral port, and sets `window.__hotlineDesk` to
`{platform, origin, token, version, dataDir}` before the page loads.

`make check` is, in order: `bun install --frozen-lockfile`; a contract
preflight that stops in one line when `ui/src/generated/contract.ts` is
partial (a filtered `cargo test` wrote it); `bun run typecheck`, `bun run
lint` (Biome, lint only), `bun run knip` (no unused export, file or
dependency; `ui/knip.json` and the Makefile say what is exempt and why) and
`bun run test` in `ui/`; the dependency gates, `make deps`: `cargo machete
--with-metadata` (no unused Cargo dependency), `cargo deny --locked check
advisories licenses` (no advisory against `Cargo.lock` that `deny.toml`
does not list with its review condition, and no crate under a license
nobody reviewed), `scripts/window_licenses.py` (the same for the window's
packages; [THIRD-PARTY-NOTICES.md](../THIRD-PARTY-NOTICES.md) says what was
reviewed) and `bun audit` in `ui/` (no advisory at all); the speech engine fetched and checked against its pinned hash;
`scripts/security-citations` (every test `docs/security.md` cites is a `fn` in
the file its row names); the Python tests under `scripts/` and
`site/tests/`; `cargo fmt --all --check`; `cargo clippy --workspace
--all-targets -- -D warnings`; `cargo test --workspace`. A window that
does not compile is a broken build however green the Rust is.

`make verify` is `cargo test --workspace --test '*'`: the integration
tests under `crates/hotline-core/tests/`, not the unit tests that live beside
the code. `make check` already runs those unit tests as part of
`cargo test --workspace`. The tests that talk to a real agent are skipped
unless you set the env they name — see [The harness](#the-harness).

Two more variables are for development only. `HOTLINE_EXTRA_DESKS` adds
desks to the window's list by endpoint: a JSON list of `{ "id", "name",
"origin", "token" }`, for example a second `hotline serve` on this machine.
A malformed entry is skipped and said. `HOTLINE_DEV_SIGN_IDENTITY` picks
the certificate `make dev` signs the debug binary with on macOS
(`scripts/cargo-dev-sign`); without it the first "Apple Development"
identity in the keychain is used, and with none the binary stays unsigned.

## Before pushing

```bash
make precheck        # before any pull request or release branch is pushed
make precheck-full   # before a release pull request
scripts/precheck windows ui   # only the legs named
```

`make check` is green on this Mac and says nothing about what only breaks
on a runner, which was every CI failure in 0.39.0: test helpers only
Unix uses, dead code on Windows; a window test that leaned on a `window`
another file left defined; a tool the runner's PATH found differently.
`scripts/precheck` runs one leg for each, prints a line per leg with
its time, and for a failure the tail of `target/precheck/<leg>.log` and
the command that reruns it. Every leg runs even after one fails. A tool a
leg needs and this Mac lacks is installed with brew, `cargo install` or
rustup, and announced first.

| Leg | What it runs | What it catches |
| --- | --- | --- |
| `fmt` | `cargo fmt --all --check` | unformatted Rust |
| `lockfile` | `cargo metadata --locked`, `bun install --frozen-lockfile` | a `Cargo.lock` or `bun.lock` that CI's `--locked` refuses |
| `actionlint` | `actionlint` (with shellcheck) | a workflow that reads what is not there, a `run:` script shellcheck faults |
| `citations` | `scripts/security-citations` | a test `docs/security.md` cites that is renamed or moved without its row |
| `contract` | every ts-rs export test, writing `contract.ts` from nothing | a committed contract that is stale, or that a filtered run truncated; the file you have is put back |
| `clippy` | `cargo clippy --workspace --all-targets --locked -- -D warnings` | the host's lints |
| `windows` | the same clippy for `x86_64-pc-windows-msvc`, through `cargo xwin` | Windows-only code, and code only Unix uses that is dead on Windows |
| `ui` | `bun run typecheck`, `bun run lint` and `bun run knip`, then `bun test ./<file>` for each file alone, then `bun run test` | a test file that passes only after another has set up a global for it |
| `linux` (full) | clippy and `cargo test --workspace --locked` in Ubuntu 24.04 | Linux-only code and its tests |
| `bare-path` (full) | the integration tests with PATH `/usr/bin:/bin:/usr/sbin:/sbin` and the toolchain | a test that needs a tool this shell put first (mise, uv, Homebrew's python) |
| `shuffle` (full) | every test in a random order, `--shuffle` | a test that passes only after another: a shared temp path, a global |

How the less obvious legs work:

- `windows` cross-compiles with `clang-cl` and `lld-link` (Homebrew's
  `llvm` and `lld`, which are keg-only, so the script puts them on PATH
  for that leg) and the MSVC CRT and SDK headers `cargo-xwin` downloads
  once into `~/Library/Caches/cargo-xwin`, accepting Microsoft's licence
  for them. `ninja` builds libopus. The C in ring, aws-lc-sys and libopus
  compiles, so the whole workspace checks and nothing is skipped. It
  links nothing: tests are compiled, not run.
- `ui`: bun runs every file in one process, in the order this machine
  lists them, so a global one file leaves behind is there for the files
  after it. A file that fails alone is named with its command.
- `linux` runs under Apple's `container` CLI. The image,
  `scripts/precheck-linux.Containerfile`, is the check job's packages on
  ubuntu-24.04 and this Mac's stable Rust, tagged by both, so changing
  either builds a new one. The checkout is mounted read-only; the build
  writes only to volumes (`hotline-precheck-*`): the target directory,
  cargo's registry, and the three gitignored folders tauri-build writes
  into the app crate. The tests run as an unprivileged user, as on the
  runner, under an init that reaps what they kill (a zombie looks alive
  to the tests that check a process group died), with every test binary
  run even after one fails. The container's masked `/proc` paths are
  cleared so bubblewrap can mount its own: the workspace shell's sandbox
  tests, ignored on Linux unless asked for, run here with `--ignored`, as
  in workspace-isolation.yml; ci.yml's Linux job, without the AppArmor
  grant, reports them ignored.
  The container is arm64, this Mac's own architecture, where the runner
  is x86_64: code that differs by architecture rather than by operating
  system is the one thing this leg does not see. `container volume rm`
  of the five volumes starts it cold.
- `bare-path` compiles with the normal PATH, then cargo runs each test
  binary through `scripts/precheck-runner-env`, which keeps cargo's
  variables, `HOME`, `USER`, `TMPDIR` and `LANG` and nothing else. This
  Mac has the Command Line Tools, so `/usr/bin/python3` works here; a
  runner with only Xcode, task-89's case, is not reproduced.
- `shuffle` sets `RUSTC_BOOTSTRAP=1` in the test runner, so stable's
  libtest takes `-Z unstable-options --shuffle` without the variable
  reaching cargo, which would rebuild every crate. Each binary prints its
  seed in `target/precheck/shuffle.log`; `-- -Z unstable-options
  --shuffle-seed <seed>` replays it. Doc tests are not shuffled.

Times on a Mac Studio with 18 cores, measured 2026-10-17. Warm means a
build of the same tree has run before; cold, an empty target directory
(and for `linux`, a new image), with crates already
downloaded and cargo-xwin's MSVC files already cached:

| | warm | cold |
| --- | --- | --- |
| `make precheck` | 15 s | 4 min |
| `linux` | 70 s | 5 min, the image included |
| `bare-path` | 35–45 s | a `cargo test` build first |
| `shuffle` | 75 s | a `cargo test` build first |
| `make precheck-full` | 3.5 min | about 12 min |

## Git hooks

```bash
make hooks   # git config core.hooksPath .githooks, once per clone
```

The hooks are plain `sh` in `.githooks/`, versioned with the code they
check; there is no hook manager. `make dev` and `make check` print a line
while they are off and never turn them on themselves. The setting lives in
the repository's shared config, so every worktree, a dispatched agent's
included, runs the hooks its own branch carries; a branch cut before
`.githooks/` existed runs none.

| Hook | What it runs | How long |
| --- | --- | --- |
| `pre-commit` | refuses a staged `*.p8`, `*.pem` or `auth.json`; `cargo fmt --all -- --check` when a `.rs` file is staged; when `contract.ts` is staged, refuses it if it lost more than a quarter of HEAD's lines or does not compile alone; `bun run typecheck` when anything under `ui/` is staged | 1–2 s |
| `pre-push` | `make precheck` ([Before pushing](#before-pushing)) | 15 s warm, 4 min cold |

The `contract.ts` check is for the filtered run's truncation
([The generated contract](#the-generated-contract)): a file missing types
either shrinks or names a type it no longer defines. The typecheck and
`cargo fmt` read the working tree, not the index, so a partly staged file
is checked as it stands on disk.

`git push --no-verify` skips pre-push, and the hook says so when it
fails; it is for an emergency, and the push then has only CI behind it.
There is no `pre-merge-commit`: a clean merge (`brainfile land`'s) joins
commits that each passed pre-commit, and a merge that stopped on a
conflict is finished with `git commit`, which runs pre-commit on
everything the merge brought in. `scripts/test_githooks.py`, in `make
check`, stages a clean change, unformatted Rust, a truncated contract, a
contract that does not compile and a key file in a clone, and runs
pre-commit on each.

## Application updates

Settings → Updates checks GitHub every six hours while a packaged Hotline is
running, with a first check after 20 seconds when due. **Check now** bypasses
that interval. The last attempt and available version are stored in
`<data dir>/updater.json`; restarting Hotline preserves the interval. A new
installed version starts a fresh check. Failures keep the previous offer.
Release notes are written for each version in `docs/releases/<version>.md`:
a headline and a few plain highlights for the person using Hotline. They are
what Settings → Updates and the update card offer, through the
conversation's markdown renderer, which never interprets HTML. The GitHub
Release shows them over the version's `CHANGELOG.md` section, the full
record, and a comparison with the previous tag (`scripts/release_notes.py`).
A tag missing either fails before anything is published. Cached offers never authorize installation:
Hotline rechecks the trusted endpoint and requires the version the user reviewed.

A version found while the window is open also shows once as a card in the
window's bottom-right corner (`ui/src/components/UpdateFloat.tsx`), whose
**Update…** opens Settings → Updates. Closing it puts that version away
(`hotline.updateDismissed` in the window's local storage) until a newer one
is found. A dev build never checks, so `VITE_PREVIEW_UPDATE=0.35.1 make dev`
pretends that version is out, for the card and Settings → Updates alike.

**Update and restart** uses `tauri-plugin-updater` 2.11.0, following
Prism's desktop updater. The plugin verifies the downloaded signature before
installation. A download can be cancelled; installation cannot be cancelled
from Hotline after the native installer starts. A Linux deb or rpm does not
go through the plugin's installer, which runs `pkexec` from PATH and then
falls back to `sudo`. Hotline's PATH is the login shell's, so a Homebrew
polkit's `pkexec`, which is not setuid, can shadow the system one. The last
`sudo` fallback then asks for a password on the desktop session's own
terminal, where nobody can answer, and every later `sudo` queues behind it.
`crates/hotline-app/src/linux_package.rs` instead runs `/usr/bin/pkexec`
with `/usr/bin/dpkg -i` or `/usr/bin/rpm -U`, all by absolute path. It has
no sudo fallback and gives up after ten minutes. A dismissed password prompt
reads as cancelled. Any other failure carries the installer's last line.
Failed download, verification or installation leaves
Hotline running and releases the room so work can continue. Successful installation
hands restart to Tauri. If the OS cannot relaunch it, reopen Hotline normally.

The room refuses installation while a turn, queued message, peer request,
session start or chapter handoff is running. Once idle, it holds a restart
lease through download and installation, blocking new work and syncing the
room, tape and thread files. Due schedules stay on disk for the next launch,
or resume after cancellation/failure. Installation replaces application files;
it does not migrate, reset or overwrite the data directory, vault, discovered
models, manual model entries or conversations.

The configured targets are macOS aarch64/x86_64 app bundles, Linux x86_64
AppImage/deb/rpm, and a Windows x86_64 per-user NSIS installer. Explicit manifest keys include the installer suffix, so a
missing deb is an error, never an AppImage fallback. Other packages and
architectures open the release page instead. Development builds, including
`make dev`, neither check nor install. Headless tests use Tauri's mock runtime,
a separate test signing key, and loopback HTTP fixtures; they never install.
The first release containing this updater must itself be installed manually
on copies that predate it.

The public signing key and HTTPS endpoint live in
`crates/hotline-app/tauri.conf.json`. The endpoint currently points directly
to GitHub's `releases/latest/download/latest.json`; a future hosting change can
serve the same contract from another configured HTTPS endpoint. There is no
runtime URL or signing-key control in the window.

CI requires `TAURI_SIGNING_PRIVATE_KEY` and optionally
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` in repository secrets. This key is separate
from Apple signing credentials. Keep an independent private backup: replacing
the public key in a future build does not teach already installed copies to
trust the replacement. Never commit the private key or include it in artifacts.
`make build` keeps ordinary local bundling available without a signing key;
release CI explicitly enables `createUpdaterArtifacts` and requires signatures.

Each workflow build collects packages and signatures under predictable names.
After all four build targets pass their checks, `scripts/updater_manifest.py` verifies that
every expected package/signature exists and writes `updater-X.Y.Z.json` and
`latest.json`. Notes come from the draft GitHub release. The release becomes
latest only after all assets are uploaded. Manual workflow runs produce signed
workflow artifacts without publishing. Published versions cannot be rebuilt;
ship a new stable `desktop-vX.Y.Z` tag. Windows updater signatures use the
same required updater key. Authenticode publisher signing is deferred; an
unsigned installer can still trigger Windows reputation warnings.

Each build runs formatting, Clippy, the workspace tests, UI type checking and
manifest tests. Windows and macOS also test a disposable native credential entry.
Local core harnesses inject in-memory stores; they never use the operator's
keychain. Run `cargo test -p hotline-support native_store_roundtrip -- --ignored`
to explicitly test the native store with a disposable entry. Linux build hosts
need `libdbus-1-dev`, and desktop credential use needs an unlocked Secret
Service session. The credential format and Rig OAuth exception are described
in [the vault](log.md#the-vault).

## The window

The shell is `crates/hotline-app`. Plugins remember the window's place,
post toasts, pick folders, open links, and write the clipboard; the
judgement for those lives in the page (`ui/src/native.ts`, `ui/src/notify.ts`),
not in the shell. The window has no system frame on Linux and Windows:
the page draws its own top strip (`ui/src/ui/Titlebar.tsx`) with the
window's title, the mark, and its minimize, maximize and close, and every
band drags. On Linux the window is transparent and the page rounds its own
corners to the pane radius, squared off while maximised; Windows rounds a
top-level window itself. On macOS the frame stays, with the title bar overlay so the rail header
can sit on the traffic-light centre line, and the menu bar is this
process's: its items emit `hotline://menu` and the window handles them.
Capabilities for the main window are
`crates/hotline-app/capabilities/default.json`.

| plugin | what the page uses it for |
| --- | --- |
| `window-state` | size, place, maximised, restored on launch |
| `notification` | a toast when a teammate finishes or blocks while the window is not focused |
| `dialog` | the folder picker, and "Remove …? Their conversation goes too." |
| `opener` | open a link, reveal a path in the file manager |
| `clipboard-manager` | write the clipboard |
| `updater` | signed updates, through desktop commands with an idle-room guard |
| `global-shortcut` | Dictate and Conversation from any app, set in Settings › General ([voice.md](voice.md#shortcuts-from-any-app)) |

Closing the window hides it; the process, the teammates and the schedules
stay. The tray is how the person gets the window back and how they actually
quit. There is no setting for this: a teammate mid-build that dies because
someone closed a window is the failure this exists to stop. The menu is
two items, Open Hotline and Quit Hotline, with a separator between them. On
Windows a left click on the icon opens the window; on macOS a left click
shows the menu, the platform's convention; on Linux Tauri 2 does not emit
tray clicks (AppIndicator), so the menu is the way back there. On macOS,
clicking the dock icon of a running app with no visible window brings the
window back; the App menu's Quit still exits. Linux needs
`libayatana-appindicator3` at runtime. macOS and Windows are built,
unproven until run there.

The macOS menu (Cmd, as the window's own listener is on a Mac; Ctrl
elsewhere): Settings `Cmd+,`, Search `Cmd+F`, New Teammate `Cmd+N`, Team
`Cmd+B`, Teammate `Cmd+I`, Teammate 1–9 `Cmd+1`…`Cmd+9`. The App
menu is Settings, About, Quit. Help opens Keyboard shortcuts, About Hotline,
and Hotline on GitHub. Where there is no menu bar, the same three help items
sit under the More button beside Settings at the foot of the rail, and
the chords are the window's own. Keyboard shortcuts lists every chord in
`ui/src/chords.ts`, and with them the two shortcuts that work from any app,
Dictate and Conversation, at the keys set for them in Settings › General.

The app icon is `assets/hotline-tile.svg`: the mark from `assets/hotline-mark.svg`
on a dark rounded tile, in the page's own colours. `make icons` runs
`cargo tauri icon` on it and writes every size the bundler wants into
`crates/hotline-app/icons/`, then drops the Android and iOS sets it also
produces; the PNGs are tracked so a checkout builds without the CLI's
rasteriser. The first PNG in `tauri.conf.json`'s icon list is the window's
own icon on Linux, which is why the 128px one leads it: X drops an icon
larger than a quarter megabyte, and 256px is a few bytes over. The same
target also renders the tray from the plain toad, `assets/hotline-mark-plain.svg`,
the mark without its handset, as the titlebar wears it:
`icons/tray.png` is the mark at 32×32 in the accent on transparent, for
Linux and Windows; `icons/tray-template.png` is the mark at 44×44 in solid
black on transparent, for macOS as a template image. In the page the same
drawing is `ui/src/ui/HotlineMark.tsx`, from the receiver geometry in
`ui/src/ui/receiver.ts`.

The page is `ui/`. The design system is written down in `ui/design.md`
and spelled as tokens in `ui/src/tokens.css` — five planes of one cool
hue, IBM Plex Sans and Mono, a 4px grid, six type sizes, hairlines of one
device pixel, and green as the one signal colour; `ui/src/ui` holds its
components (Avatar, Band, Menu). A colour or a face is always a token,
never a literal in a component. Settings, New
Teammate, Keyboard shortcuts and About are panes that replace the
conversation, not a card over it. Up to three teammates can be pinned
(`persona.pin`, stored as the room setting `pinnedTeammates` and carried on the
roster as `pin`): they leave the list and sit at the top of the rail as larger
faces with the name under each, stacked above a hairline in the faces-only
rail, with the same status, unread and busy marks. They reorder by dragging, or
Move left / right in the row's context menu, and the Teammate 1–9 chords
count pins first. While Settings is open the rail is its
sections (General, Desks, Providers, Tools, Skills, Computer,
Secrets, Remote, Updates and Import, with Keyboard shortcuts and About at its
foot), with a back key in its band
where the team's plus was, and the pane shows one section at a time. The
teammate inspector sits beside the
conversation; search is a popover that hangs under the band over the
conversation already on screen. Band is the chrome strip: it drags the
window, and a double-click maximises.

## The data directory

`HOTLINE_DATA_DIR` overrides the data directory when it is set and not blank.
`make dev` points it at `.hotline-dev` in the checkout so nothing done in a
dev instance reaches real data. `.hotline-dev` is gitignored.

Without the override, the directory is the platform's application-support
path: `~/Library/Application Support/Hotline` on macOS, `%APPDATA%\Hotline` on
Windows, and `${XDG_DATA_HOME:-~/.local/share}/hotline` on Linux.

The vault is `<data dir>/vault/`: `secrets.json` for pasted API keys,
`vault/logins/<id>/` for login tokens (the files Rig
writes, pre-created owner-only), Grok's atomically replaced OAuth tokens,
OpenRouter's acquired key, and discovered
model ids for each connection, and `vault/mcp/<server>.json` for an HTTP
MCP server's protected OAuth registration and tokens. MCP records are bound
to the configured server URL and authorization issuer; they never enter
settings, streams, tapes or agent descriptors.

`<data dir>/agent-accounts/<uuid>/` is one agent account's own config
folder, private from its creation (`hotline_support::private_dir`). The
agent writes its sign-in, settings and history there, and removing the
account in Settings leaves the folder.

Tests use temporary directories of their own. Never point a test, a
harness, or `HOTLINE_DATA_DIR` at a real Hotline data directory.

The app's own output — `[startup]` lines, an agent that would not start, a
panic — goes to `<data dir>/logs/hotline.log` whenever its stderr is not a
terminal, which is every launch from the Finder, the Start menu or a desktop
launcher, where it would otherwise be lost (`hotline_core::log_file`). The
process's stdout and stderr are pointed at the file itself, so everything
printed with `eprintln!` and the panic hook's message land there, and nothing
is buffered in the process. The file is capped at 5 MB: it is moved to
`hotline.log.1` at launch and every five minutes once it reaches that, and
only that one previous file is kept. Started from a terminal, as `make dev`
and running the bundle's binary by hand do, the output stays in the terminal
and no log is written. Nothing secret is printed, so nothing secret is logged:
keep it that way, and never `eprintln!` a key, a token or a request body.
`hotline serve` logs to its journal.

One desk runs per data directory. Before the room opens, every process
that writes it — the app, `hotline serve`, `hotline-import` — takes an
exclusive lock on `desk.lock` there (`hotline_support::room_lock`), in every
build; a file system that cannot lock is an error, not an unguarded room.
The app also writes, beside it, the loopback port it answers on to
`desk.wake`: a second launch finds the lock held, asks the running desk to
show its window, and exits once that desk says it has. Two desks on one
room would both fire its schedules, both answer its phones and both write
its streams. The system releases the lock however the process ends, and the
file is never removed. A desk that holds the lock without answering is
waited on for twenty seconds, because a restart after an update starts the
new process while the old one is still leaving, and a desk that is still
opening answers only once its window exists. After that the launch exits
and the room stays with the desk that has it. A desk on another data
directory is another room and runs alongside, which is how `make dev` runs
next to the installed app: it uses `.hotline-dev`. The app's half is
`crates/hotline-app/src/instance.rs`.

A room records its secret store in `store.json` — `native` for the app,
`file` for `hotline serve --store file` — and a desk started on the other
store refuses it (`credentials::claim_backend`). A room from before the
marker, with a vault, is a keychain room.

## The computer

A teammate with `computer.enabled` gets a container Hotline starts on session
start. The runtimes Hotline looks for, in detection order then ranked
rootless-available first:

| id | CLI | where |
| --- | --- | --- |
| `docker` | `docker` | Linux and macOS |
| `podman` | `podman` | Linux and macOS |
| `container` | Apple `container` | macOS only |

On macOS, desktop startup restores `PATH` from the user's interactive login
shell before starting the core. This covers shell-managed Node installations
and ACP harnesses as well as Docker credential helpers inherited by child
processes. Shell startup has a five-second limit; failure retains the inherited
path and adds the standard Homebrew, local CLI, and Docker Desktop directories.

Detection is `version` (available or a reason) then `info` (rootless).
Binaries are resolved on `PATH` plus `/usr/local/bin`, `/opt/homebrew/bin`
and `~/.local/bin`, because a packaged Mac app's GUI PATH is the bare
system one. The user's pick is the room setting `computerRuntime`; absent
means the first available. The image is the teammate's or the room's pin,
or else the newest published release at or above the floor
`COMPUTER_VERSION` in `crates/hotline-core/src/computer.rs`, and the
floor itself when nothing newer is known ([computer.md](computer.md)), at
`ghcr.io/1broseidon/hotline-computer:<release>`. Never `latest`. Tests use a fake runtime script in a temp dir; they do not talk
to a real daemon.

The create line is `--cap-drop=ALL`, `--security-opt no-new-privileges`,
`--pids-limit` and `--memory` from `persona.computer` (1024 and 4g when
absent), `--shm-size 1g`, the loopback port for 8787, the token in the
environment and the host's `TZ`, then the mounts: the named volumes
`hotline-home-<persona id>:/home/agent`, `hotline-nix-glibc:/nix` and
`hotline-src-<persona id>:/home/agent/src` (Docker and Podman only), the
teammate's `persona.computer.mounts` as `host:path[:ro]`, and the room's
cwd at `/home/agent/workspace`. `mount_args` refuses a host folder that
does not exist and a container path that overlaps the workspace, the
scratch or the store.

## The generated contract

Each wire type is defined once in Rust, with serde and ts-rs, in
`crates/hotline-contract`, a file per concern, each re-exported from its
`lib.rs`. The crate depends on no other Hotline crate, and `hotline-core`
re-exports it as `contract`, so `hotline_core::contract::X` names every one;
the types that belong to Remote, linked desks and the importer are
re-exported again from those modules. `cargo test -p hotline-contract` runs
the export tests and writes `ui/src/generated/contract.ts`.
`.cargo/config.toml` points ts-rs at that directory and tells it a 64-bit
integer is a JavaScript `number`, because every number on this wire arrives
as `JSON.parse` produces it.

ts-rs starts the file afresh the first time a test process writes it, so
every exported type has to live in the one crate: a type exported from
`hotline-core` would erase the contract crate's types, or be erased by them.
A filtered run (`cargo test session`) writes only the types it matched
and leaves a file the window cannot compile against. Run
`cargo test -p hotline-contract` unfiltered before committing that file;
`make check` and the pre-commit hook catch it. Never run a filtered test as
the last run.

## The model catalogue

Hotline Agent's picker is not hand-written. `crates/hotline-core/models.json` is
a snapshot of [models.dev](https://models.dev), the catalogue opencode and
pi draw theirs from, cut down to the providers `models::WIRING` names and
the models a coding agent can use: ones that call tools, answer in text
only, and are not deprecated. The core reads it at start with
`include_str!`; a test refuses a snapshot whose providers are not exactly
the wired ones. A model's `reasoning_options` becomes `efforts`: only the
`effort` type is read (a `toggle` or `budget_tokens` option is ignored),
in the order models.dev lists the values.

To refresh it:

```bash
cargo run -p hotline-core --bin hotline-models-sync        # fetch, filter, rewrite
cargo run -p hotline-core --bin hotline-models-sync -- --from api.json   # from a saved copy
```

It prints what each provider gained and lost against the snapshot it was
built with, then writes the file. Read that diff, run `make check`, commit.
The metadata snapshot lives in git. Runtime provider discovery supplies
current model IDs independently, and exact catalogue matches enrich those
IDs. A new ID is usable without a snapshot update. Hotline does not download
models.dev at runtime. Persisted discovery and manual additions live with
connection data, so installing a new release cannot replace them.

To add a provider: one `Wiring` line in `models.rs`, one `Client` arm in
`driver/rig.rs` naming the Rig client that speaks to it, and a sync. The
key form and picker use the trusted provider identity from wiring and the bundled catalogue.

`openai-codex` is a synthesized provider. models.dev has no ChatGPT
subscription row, so the sync copies the listed models off `openai` and
clears their per-token price. A subscription has no per-token price; an id
`openai` lacks is an error from the sync, so the list cannot drift
silently. That list is only the fallback until a ChatGPT sign-in refreshes.
A connected sign-in lists its models live, and an id it lists that
`CHATGPT_MODELS` lacks is read from `openai` the same way, price cleared
(`models::model_of`), so a model ChatGPT adds has its efforts and limits
before the next sync. Still mirror `CHATGPT_MODELS` from the Codex model list
when you sync: it is what a picker offers before anyone signs in.

Ollama Local is the other exception: its catalogue row has no fixed models.
Rig discovers the installed ids from the chosen server on connection and
refresh. Ollama Cloud has a bundled models.dev list as a fallback, replaced
by discovery once available. `providers.rs` contains the connection work
Rig does not supply (OpenRouter PKCE and URL validation); `providers/xai.rs`
adds Grok's device sign-in and a refresh wrapper around Rig's HTTP client.
The device protocol follows [Pi's xAI implementation](https://github.com/earendil-works/pi/blob/main/packages/ai/src/auth/oauth/xai.ts)
and the [Grok CLI authentication guide](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-pager/docs/user-guide/02-authentication.md).
The public device client id is not a secret; requests identify Hotline as their
referrer. OAuth response bodies never enter user-facing errors. Model requests
still go through Rig. `credentialKinds` lists all connection methods a
provider offers, while each saved credential retains its own singular kind.

Active provider connections can refresh models through native Rig listing
clients. The validated list lives in `vault/logins/<credential-id>/discovery.json`;
`manual-models.json` holds separate user additions. Legacy `models.json` ID
lists remain readable. A valid discovery file takes precedence; neither
file is overwritten by installing a new application bundle. Live model names
and limits take precedence over exact bundled matches; the catalogue fills
missing fields, including efforts, capabilities and pricing. Provider identity
still comes from trusted wiring, not live `owned_by` fields or display names.

The `openai-compatible` catalogue row is also empty. Each custom connection
supplies its own model IDs, optionally discovered with Rig's model-listing
client, and gets a stable `custom-<uuid>` provider id. `providers/custom.rs`
uses a Rig provider builder solely to allow keyless servers; both inference
protocols retain Rig's native request, tool-call and streaming implementation.
Custom API keys are endpoint-bound records in the private credential's
`auth.json`, while the API choice and model IDs are ordinary metadata.

## The harness

The house proof is a headless harness that starts the real core and drives
it over the wire. Each file under `crates/hotline-core/tests/` opens a `Desk`
on a temporary directory, binds a `Door` with a token only the harness
knows, and speaks WebSocket JSON the way the window does:
`ws://127.0.0.1:<port>/ws?token=…`.

| file | what it proves |
| --- | --- |
| `desk.rs` | a teammate over the wire; a peer thread listed, streamed and marked read; a Hotline Agent turn with a real key; an ACP child turn |
| `mcp.rs` | Hotline as an MCP client and as the server of a teammate's own tools: a granted echo server, Hotline's seven tools, a policy of none, a server that will not start, a non-string env, a vanished server, a stdio process group |
| `schedule.rs` | a job created, listed, silenced and cancelled, remembered on the room stream; a loop carries `every` and has no `when` |

The OAuth unit harness in `crates/hotline-core/src/mcp/tests.rs` runs a
loopback protected-resource and authorization server, through the vault, through metadata discovery, DCR, PKCE callback, token
refresh and authenticated MCP calls. It also covers restart reuse, denied and
wrong-state callbacks, registration gaps, concurrent refresh, URL binding,
sign-out and the protected vault boundary. No real provider account is used by
the test suite.

Most of `desk.rs` needs no provider key: it creates a teammate, watches
the roster view, lists models, adds a credential, lists and marks a peer
thread, and so on. The tests that need a real agent are ignored, and run
with `--ignored`, failing by name when their env is missing. One starts a
Hotline Agent turn with a real key in `HOTLINE_HARNESS_ANTHROPIC_KEY`, has it
read a file, and checks the tape. The others are the same proof for the
other kind of agent: set `HOTLINE_HARNESS_ACP` to a backend id this machine
can run (`cursor`, say) and they drive a real harness as a child, including
a line sent mid-turn.

```bash
HOTLINE_HARNESS_ANTHROPIC_KEY=… cargo test -p hotline-core --test integration -- --ignored a_turn_with_a_real_key
HOTLINE_HARNESS_ACP=cursor cargo test -p hotline-core --test integration -- --ignored real_acp_harness
```

A harness that wants state in a stream asks the core over the wire, or
writes the file before the core starts. The core is the only process that
appends to a stream.

A test waits on the thing it is waiting for, never on a sleep it hopes is
long enough: `common::until(what, cond)` in a harness, and
`test_support::until(what, cond)` in a unit test, each giving up after one
generous minute with `what` in the failure. A loaded runner only makes them
slower, so a test asserts what happened and in what order, never how fast.

## hotline-import

```bash
cargo run -p hotline-core --bin hotline-import -- <from> <to>
```

Reads the previous edition's layout at `<from>` and writes this tree's
streams and vault at `<to>`. The source is never written: `store.sqlite`
is copied (the database and its `-wal`, never the `-shm`) into a private
temporary directory so SQLite cannot leave sidecars beside a directory it
must not write, and a copy that does not check out is an error rather
than a smaller roster. Tapes are copied, secrets are read out of the old
vault. A teammate already in the roster, a tape that already exists
here, and a setting already set (a tombstone is not a set setting) are
left alone, so running it twice is the same as running it once.
`mcpServers` comes over when the shape is one this tree accepts; an
entry it cannot read is named on `notes` and the rest of the list still
lands. A key this Hotline does not have is skipped with a row, not dropped
in silence.

Success prints a JSON report (`teammates`, `tapes`, `threads`,
`schedules`, `settings`, `keys`, `skipped`, `notes`) and exits 0. A
failure prints the error and exits 1. Wrong arguments print
`usage: hotline-import <from> <to>` and exit 2. The two paths must not
resolve to the same directory, and `<to>` must not live inside `<from>`.

The same importer is the `room.import` command on the wire. Imported
teammates keep their working directories under the old data directory's
`workspaces/`, by design: the workspace is the teammate's project, not
something the importer copies. Backend ids are mapped onto this tree's
registry (the previous edition's DEFAULT table — `pi`, `cursor`, `opencode`,
`gemini`, `claude-acp`, `codex-acp` — already matches the hand-taught
ids here). An id this build has no harness for is kept as written, and
the report notes it so the session can refuse that start in a sentence.
A search index that cannot be written is the same kind of note: the
index is rebuildable, and the next start will catch up.
