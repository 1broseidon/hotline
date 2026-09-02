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
  session/               Session, the funnel, quiet, chapters, the scheduler, the ledger
  driver/                Toad Agent on Rig, and the ACP child with its registry
  mcp/                   the client of granted servers, and Toad's own teammate tools
  tools/                 workspace tools on cap-std, shell command
  desk.rs                the room, vault and log behind the wire
  import.rs              copies an existing Toad data directory
  wire/                  the door: seats, commands, subscriptions
  bin/toad-import.rs     the importer as a binary
  bin/toad-mcp-echo.rs   a one-tool stdio server the MCP harnesses spawn
crates/toad-core/tests/  headless proofs driving the real core over the wire
crates/toad-desktop/     the Tauri 2 shell: plugins, the menu, the door
ui/                      the React window, built against the generated contract
docs/                    what is
```

The core is a library. The shell binds its door on a loopback port, injects
the port and a token into the page, and opens a window on it. There is no
child process for the room. Opening a `Desk` opens the room, which settles
every tape (expired permission cards, a compact, the search index) and
starts the idle chapter sweep and the scheduler's clock before anything is
served.

## Running it

```bash
make dev        # the Tauri shell, Vite with hot reload, on .toad-dev
make check      # cargo fmt --check, clippy -D warnings, cargo test, the window's typecheck
make verify     # the headless harnesses, driving the real core over the wire
```

`make dev` exports `TOAD_DATA_DIR` to `.toad-dev` in the checkout, then
runs `cargo tauri dev` from `crates/toad-desktop`, where `tauri.conf.json`
is. That starts Vite for the window on port 5174 and refuses any other
port, so a port already in use is a mistake rather than a second
instance. The shell generates a one-launch token, binds the door on
`127.0.0.1` with an ephemeral port, and sets `window.__toadDesk` to
`{platform, origin, token}` before the page loads.

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
not in the shell. The menu bar is this process's: its items emit
`toad://menu` and the window handles them. Capabilities for the main
window are `crates/toad-desktop/capabilities/default.json`.

| plugin | what the page uses it for |
| --- | --- |
| `window-state` | size, place, maximised, restored on launch |
| `notification` | a toast when a teammate finishes or blocks while the window is not focused |
| `dialog` | the folder picker, and "Remove …? Their conversation goes too." |
| `opener` | open a link, reveal a path in the file manager |
| `clipboard-manager` | write the clipboard |

The menu (Ctrl, not Cmd — the window's own listener is Ctrl on every
platform): Settings `Ctrl+,`, Search `Ctrl+F`, New Teammate `Ctrl+N`,
Teammate `Ctrl+I`, Teammate 1–9 `Ctrl+1`…`Ctrl+9`. On macOS the title bar
is overlay so the rail header can sit on the traffic-light centre line.

Settings and New Teammate are panes: they replace the conversation, they
are not a card over it. Settings has a left index — General, Keys, Tools,
Import — and a selected teammate is one more row in that index, not a
second window. Search is a drawer over the conversation that is already
on screen. The chrome strip drags the window; a double-click maximises.

## The data directory

`TOAD_DATA_DIR` overrides the data directory when it is set and not blank.
`make dev` points it at `.toad-dev` in the checkout so nothing done in a
dev instance reaches real data. `.toad-dev` is gitignored.

Without the override, the directory is the platform's application-support
path: `~/Library/Application Support/Toad` on macOS, `%APPDATA%\Toad` on
Windows, and `${XDG_DATA_HOME:-~/.local/share}/toad` on Linux.

Tests use temporary directories of their own. Never point a test, a
harness, or `TOAD_DATA_DIR` at a real Toad data directory.

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

## The harness

The house proof is a headless harness that starts the real core and drives
it over the wire. Each file under `crates/toad-core/tests/` opens a `Desk`
on a temporary directory, binds a `Door` with a token only the harness
knows, and speaks WebSocket JSON the way the window does:
`ws://127.0.0.1:<port>/ws?token=…`.

| file | what it proves |
| --- | --- |
| `desk.rs` | a teammate over the wire; a Toad Agent turn with a real key; an ACP child turn |
| `mcp.rs` | Toad as an MCP client: a granted echo server, a policy of none, a server that will not start |
| `schedule.rs` | a job created, listed, silenced and cancelled, remembered on the room stream |

`desk.rs` has three tests. The first creates a teammate, watches the
roster view, lists models, adds a credential, and deletes the teammate —
no provider key required. The second starts a Toad Agent turn with a real
key, has it read a file, and checks the tape. It is skipped unless
`TOAD_HARNESS_ANTHROPIC_KEY` is set. The third is the same proof for the
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
is opened from a copy so SQLite cannot leave `-wal`/`-shm` beside it,
tapes are copied, secrets are read out of the old vault. A teammate
already in the roster, a tape that already exists here, and a setting
already set are left alone, so running it twice is the same as running it
once.

Success prints a JSON report (`teammates`, `tapes`, `settings`, `keys`,
`skipped`) and exits 0. A failure prints the error and exits 1. Wrong
arguments print `usage: toad-import <from> <to>` and exit 2.

The same importer is the `room.import` command on the wire. Imported
teammates keep their working directories under the old data directory's
`workspaces/`, by design: the workspace is the teammate's project, not
something the importer copies. Backend ids are mapped onto this tree's
registry (the previous Toad's DEFAULT table — `pi`, `cursor`, `opencode`,
`gemini`, `claude-acp`, `codex-acp` — already matches the hand-taught
ids here). An id this build has no harness for is kept as written, and
the report notes it so the session can refuse that start in a sentence.
