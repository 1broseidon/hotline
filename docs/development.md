# Developing Toad

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
- [Bun](https://bun.sh), for the window's install, typecheck, Vite, and
  production build.
- On Linux, `libayatana-appindicator3` at runtime, for the tray, and its
  dev package (`libayatana-appindicator3-dev`) to bundle: the Tauri CLI
  finds the library through pkg-config before it writes the deb. A
  Homebrew `pkg-config` earlier on PATH searches only Homebrew's
  directories; point `PKG_CONFIG_PATH` at the system one
  (`/usr/lib/x86_64-linux-gnu/pkgconfig`) for `make build`.

`toad-core` has no Tauri dependency. The shell crate is the only place
Rust that needs Tauri lives.

## Layout

```
crates/toad-core/src/
  contract.rs            the wire's types (serde + ts-rs)
  paths.rs               the data directory's layout
  log/                   streams: append, fold, segments, subscribe
  store/                 FTS5 index, chapter list, roster previews
  room.rs                the roster fold, settings, and schedules
  vault.rs               secrets beside the room stream
  session/               Session, the funnel, quiet, chapters, the scheduler, the ledger, peer threads
  driver/                Toad Agent on Rig, and the ACP child with its registry
  models.rs              the providers Toad Agent reaches, and the model catalogue they serve
  mcp/                   the client of granted servers, and Toad's own teammate tools
  computer/              runtime detection and one container per teammate
  tools/                 workspace tools on cap-std, shell command
  desk.rs                the room, vault and log behind the wire
  import.rs              copies an existing Toad data directory
  import/                the previous Toad's roster and records, read-only
  wire/                  the door: seats, commands, subscriptions
  bin/toad-import.rs     the importer as a binary
  bin/toad-mcp-echo.rs   a one-tool stdio server the MCP harnesses spawn
  bin/toad-models-sync.rs rewrites models.json from models.dev
crates/toad-core/models.json  the model catalogue: a filtered snapshot of models.dev
crates/toad-core/tests/  headless proofs driving the real core over the wire
crates/toad-desktop/     the Tauri 2 shell: plugins, the menu, the door
ui/                      the React window, built against the generated contract; ui/src/ui is the design system
assets/                  the mark and the app tile it sits on; `make icons` renders the icon set from them
docs/                    what is
```

The core is a library. The shell binds its door on a loopback port, injects
the port and a token into the page, and opens a window on it. There is no
child process for the room. Opening a `Desk` opens the room, which settles
every tape and every thread (expired permission cards, a compact, the
search index) and starts the idle chapter sweep and the scheduler's clock
before anything is served.

## Running it

```bash
make dev        # the Tauri shell, Vite with hot reload, on .toad-dev
make check      # cargo fmt --check, clippy -D warnings, cargo test, the window's typecheck
make verify     # the headless harnesses, driving the real core over the wire
make build      # a release bundle under target/release/bundle (unsigned)
make icons      # every platform's app icon, and the tray marks, from assets/
```

A release is a tag `desktop-vX.Y.Z` whose version matches `Cargo.toml` and
`tauri.conf.json`; `.github/workflows/desktop-release.yml` builds each
target on its own runner, signs and notarizes the Mac bundles from the
repository secrets, and publishes one GitHub Release marked latest. A hand
run of the workflow builds and keeps artifacts only. `CHANGELOG.md` takes
an entry per version.

`make dev` exports `TOAD_DATA_DIR` to `.toad-dev` in the checkout, then
runs `cargo tauri dev` from `crates/toad-desktop`, where `tauri.conf.json`
is. That starts Vite for the window on port 5174 and refuses any other
port, so a port already in use is a mistake rather than a second
instance. The shell generates a one-launch token, binds the door on
`127.0.0.1` with an ephemeral port, and sets `window.__toadDesk` to
`{platform, origin, token, version, dataDir}` before the page loads.

`make check` is, in order: `bun install --frozen-lockfile` and `bun run
typecheck` in `ui/`; `cargo fmt --all --check`; `cargo clippy --workspace
--all-targets -- -D warnings`; `cargo test --workspace`. A window that
does not compile is a broken build however green the Rust is.

`make verify` is `cargo test --workspace --test '*'`: the integration
tests under `crates/toad-core/tests/`, not the unit tests that live beside
the code. `make check` already runs those unit tests as part of
`cargo test --workspace`. Two of those tests talk to a real agent and are
skipped unless you set the env they name — see [The harness](#the-harness).

## The window

The shell is `crates/toad-desktop`. Plugins remember the window's place,
post toasts, pick folders, open links, and write the clipboard; the
judgement for those lives in the page (`ui/src/native.ts`, `ui/src/notify.ts`),
not in the shell. The window has no system frame on Linux and Windows:
the page draws its own top strip (`ui/src/ui/Titlebar.tsx`) with the
window's title, the mark, and its minimize, maximize and close, and every
band drags. On Linux the window is transparent and the page rounds its own
corners to the pane radius, squared off while maximised; Windows rounds a
top-level window itself. On macOS the frame stays, with the title bar overlay so the rail header
can sit on the traffic-light centre line, and the menu bar is this
process's: its items emit `toad://menu` and the window handles them.
Capabilities for the main window are
`crates/toad-desktop/capabilities/default.json`.

| plugin | what the page uses it for |
| --- | --- |
| `window-state` | size, place, maximised, restored on launch |
| `notification` | a toast when a teammate finishes or blocks while the window is not focused |
| `dialog` | the folder picker, and "Remove …? Their conversation goes too." |
| `opener` | open a link, reveal a path in the file manager |
| `clipboard-manager` | write the clipboard |

Closing the window hides it; the process, the teammates and the schedules
stay. The tray is how the person gets the window back and how they actually
quit. There is no setting for this: a teammate mid-build that dies because
someone closed a window is the failure this exists to stop. The menu is
two items, Open Toad and Quit Toad, with a separator between them. On
Windows a left click on the icon opens the window; on macOS a left click
shows the menu, the platform's convention; on Linux Tauri 2 does not emit
tray clicks (AppIndicator), so the menu is the way back there. On macOS,
clicking the dock icon of a running app with no visible window brings the
window back; the App menu's Quit still exits. Linux needs
`libayatana-appindicator3` at runtime. macOS and Windows are built,
unproven until run there.

The macOS menu (Ctrl, not Cmd — the window's own listener is Ctrl on
every platform): Settings `Ctrl+,`, Search `Ctrl+F`, New Teammate
`Ctrl+N`, Teammate `Ctrl+I`, Teammate 1–9 `Ctrl+1`…`Ctrl+9`. The App
menu is Settings, About, Quit. Help opens Keyboard shortcuts, About Toad,
and Toad on GitHub. Where there is no menu bar, the same three help items
sit under the More button beside Settings at the foot of the rail, and
the chords are the window's own.

The app icon is `assets/toad-tile.svg`: the mark from `assets/toad-mark.svg`
on a dark rounded tile, in the page's own colours. `make icons` runs
`cargo tauri icon` on it and writes every size the bundler wants into
`crates/toad-desktop/icons/`, then drops the Android and iOS sets it also
produces; the PNGs are tracked so a checkout builds without the CLI's
rasteriser. The first PNG in `tauri.conf.json`'s icon list is the window's
own icon on Linux, which is why the 128px one leads it: X drops an icon
larger than a quarter megabyte, and 256px is a few bytes over. The same
target also renders the tray: `icons/tray.png` is the mark at 32×32 in the
accent on transparent, for Linux and Windows; `icons/tray-template.png` is
the mark at 44×44 in solid black on transparent, for macOS as a template
image. In the page the same drawing is `ui/src/ui/ToadMark.tsx`.

The page is `ui/`. The design system is written down in `ui/design.md`
and spelled as tokens in `ui/src/tokens.css` — five planes of one cool
hue, IBM Plex Sans and Mono, a 4px grid, six type sizes, hairlines of one
device pixel, and green as the one signal colour; `ui/src/ui` holds its
components (Avatar, Band, Menu). A colour or a face is always a token,
never a literal in a component. Settings, New
Teammate, Keyboard shortcuts and About are panes that replace the
conversation, not a card over it. While Settings is open the rail is its
sections — General, Providers, Tools, Import — with a back key in its band
where the team's plus was, and the pane shows one section at a time. The
teammate inspector sits beside the
conversation; search is a popover that hangs under the band over the
conversation already on screen. Band is the chrome strip: it drags the
window, and a double-click maximises.

## The data directory

`TOAD_DATA_DIR` overrides the data directory when it is set and not blank.
`make dev` points it at `.toad-dev` in the checkout so nothing done in a
dev instance reaches real data. `.toad-dev` is gitignored.

Without the override, the directory is the platform's application-support
path: `~/Library/Application Support/Toad` on macOS, `%APPDATA%\Toad` on
Windows, and `${XDG_DATA_HOME:-~/.local/share}/toad` on Linux.

The vault is `<data dir>/vault/`: `secrets.json` for pasted API keys,
`vault/logins/<id>/` for a subscription login's tokens (the files Rig
writes, pre-created owner-only), and `vault/mcp/<server>.json` for an HTTP
MCP server's protected OAuth registration and tokens. MCP records are bound
to the configured server URL and authorization issuer; they never enter
settings, streams, tapes or agent descriptors.

Tests use temporary directories of their own. Never point a test, a
harness, or `TOAD_DATA_DIR` at a real Toad data directory.

## The computer

A teammate with `computer.enabled` gets a container Toad starts on session
start. The runtimes Toad looks for, in detection order then ranked
rootless-available first:

| id | CLI | where |
| --- | --- | --- |
| `docker` | `docker` | Linux and macOS |
| `podman` | `podman` | Linux and macOS |
| `container` | Apple `container` | macOS only |

Detection is `version` (available or a reason) then `info` (rootless).
Binaries are resolved on `PATH` plus `/usr/local/bin`, `/opt/homebrew/bin`
and `~/.local/bin`, because a packaged Mac app's GUI PATH is the bare
system one. The user's pick is the room setting `computerRuntime`; absent
means the first available. The image is `persona.computer.image` or the
pin `COMPUTER_VERSION` in `crates/toad-core/src/computer/mod.rs`, currently
`0.3.0`, at `ghcr.io/1broseidon/toad-computer:<COMPUTER_VERSION>`. Never
`latest`. Tests use a fake runtime script in a temp dir; they do not talk
to a real daemon.

The create line is `--cap-drop=ALL`, `--security-opt no-new-privileges`,
`--pids-limit` and `--memory` from `persona.computer` (512 and 2g when
absent), `--shm-size 1g`, the loopback port for 8787, the token in the
environment, then the mounts: the named volumes `toad-nix:/nix` and
`toad-src-<persona id>:/home/agent/src` (Docker and Podman only), the
teammate's `persona.computer.mounts` as `host:path[:ro]`, and the room's
cwd at `/home/agent/workspace`. `mount_args` refuses a host folder that
does not exist and a container path that overlaps one of those three.

## The generated contract

Types are defined once in `crates/toad-core/src/contract.rs` (serde +
ts-rs). `cargo test -p toad-core` runs the export tests and writes
`ui/src/generated/contract.ts`. `.cargo/config.toml` points ts-rs at that
directory and tells it a 64-bit integer is a JavaScript `number`, because
every number on this wire arrives as `JSON.parse` produces it.

A filtered run (`cargo test session`) writes only the types it matched
and leaves a file the window cannot compile against. Run
`cargo test -p toad-core` unfiltered before committing that file; `make
check` catches it. Never run a filtered test as the last run.

## The model catalogue

Toad Agent's picker is not hand-written. `crates/toad-core/models.json` is
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
cargo run -p toad-core --bin toad-models-sync        # fetch, filter, rewrite
cargo run -p toad-core --bin toad-models-sync -- --from api.json   # from a saved copy
```

It prints what each provider gained and lost against the snapshot it was
built with, then writes the file. Read that diff, run `make check`, commit.
The snapshot lives in git rather than being fetched at run time because a
catalogue is behaviour — names, prices, limits — and a release should mean
the same thing on every machine that runs it.

To add a provider: one `Wiring` line in `models.rs`, one `Client` arm in
`driver/rig.rs` naming the Rig client that speaks to it, and a sync. The
key form and the picker learn the name from the catalogue.

`openai-codex` is the one hand-written provider. models.dev has no ChatGPT
subscription row, so the sync copies the listed models off `openai` and
clears their per-token price. A subscription has no per-token price; an id
`openai` lacks is an error from the sync, so the list cannot drift
silently.

## The harness

The house proof is a headless harness that starts the real core and drives
it over the wire. Each file under `crates/toad-core/tests/` opens a `Desk`
on a temporary directory, binds a `Door` with a token only the harness
knows, and speaks WebSocket JSON the way the window does:
`ws://127.0.0.1:<port>/ws?token=…`.

| file | what it proves |
| --- | --- |
| `desk.rs` | a teammate over the wire; a peer thread listed, streamed and marked read; a Toad Agent turn with a real key; an ACP child turn |
| `mcp.rs` | Toad as an MCP client and as the server of a teammate's own tools: a granted echo server, Toad's seven tools, a policy of none, a server that will not start, a non-string env, a vanished server, a stdio process group |
| `schedule.rs` | a job created, listed, silenced and cancelled, remembered on the room stream; a loop carries `every` and has no `when` |

The `mcp/oauth.rs` unit harness runs a loopback protected-resource and
authorization server through metadata discovery, DCR, PKCE callback, token
refresh and authenticated MCP calls. It also covers restart reuse, denied and
wrong-state callbacks, registration gaps, concurrent refresh, URL binding,
sign-out and the protected vault boundary. No real provider account is used by
the test suite.

`desk.rs` has four tests. The first creates a teammate, watches the
roster view, lists models, adds a credential, and deletes the teammate —
no provider key required. The second lists a peer thread, subscribes to
it, and marks a message read. The third starts a Toad Agent turn with a
real key, has it read a file, and checks the tape. It is skipped unless
`TOAD_HARNESS_ANTHROPIC_KEY` is set. The fourth is the same proof for the
other kind of agent: set `TOAD_HARNESS_ACP` to a backend id this machine
can run (`cursor`, say) and it drives a real harness as a child.

```bash
TOAD_HARNESS_ANTHROPIC_KEY=… make verify
TOAD_HARNESS_ACP=cursor make verify
```

A harness that wants state in a stream asks the core over the wire, or
writes the file before the core starts. The core is the only process that
appends to a stream.

## toad-import

```bash
cargo run -p toad-core --bin toad-import -- <from> <to>
```

Reads the previous Toad's layout at `<from>` and writes this tree's
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
lands. A key this Toad does not have is skipped with a row, not dropped
in silence.

Success prints a JSON report (`teammates`, `tapes`, `threads`,
`schedules`, `settings`, `keys`, `skipped`, `notes`) and exits 0. A
failure prints the error and exits 1. Wrong arguments print
`usage: toad-import <from> <to>` and exit 2. The two paths must not
resolve to the same directory, and `<to>` must not live inside `<from>`.

The same importer is the `room.import` command on the wire. Imported
teammates keep their working directories under the old data directory's
`workspaces/`, by design: the workspace is the teammate's project, not
something the importer copies. Backend ids are mapped onto this tree's
registry (the previous Toad's DEFAULT table — `pi`, `cursor`, `opencode`,
`gemini`, `claude-acp`, `codex-acp` — already matches the hand-taught
ids here). An id this build has no harness for is kept as written, and
the report notes it so the session can refuse that start in a sentence.
A search index that cannot be written is the same kind of note: the
index is rebuildable, and the next start will catch up.
