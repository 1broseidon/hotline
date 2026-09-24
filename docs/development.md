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
- [Bun](https://bun.sh), for the window's install, typecheck, Vite, and
  production build.
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
crates/hotline-core/src/
  contract.rs            the wire's types (serde + ts-rs)
  paths.rs               the data directory's layout
  log/                   streams: append, fold, segments, subscribe
  store/                 FTS5 index, chapter list, roster previews
  room.rs                the roster fold, settings, and schedules
  vault.rs               secrets beside the room stream
  session/               Session, the funnel, quiet, chapters, the scheduler, the ledger, peer threads
  driver/                Hotline Agent on Rig, and the ACP child with its registry
  models.rs              the providers Hotline Agent reaches, and the model catalogue they serve
  mcp/                   the client of granted servers, and Hotline's own teammate tools
  computer/              runtime detection and one container per teammate
  tools/                 workspace tools on cap-std, shell command
  desk.rs                the room, vault and log behind the wire
  import.rs              copies an existing Hotline data directory
  import/                the previous edition's roster and records, read-only
  wire/                  the door: seats, commands, subscriptions
  bin/hotline-import.rs     the importer as a binary
  bin/hotline-mcp-echo.rs   a one-tool stdio server the MCP harnesses spawn
  bin/hotline-models-sync.rs rewrites models.json from models.dev
crates/hotline-core/models.json  the model catalogue: a filtered snapshot of models.dev
crates/hotline-core/tests/  headless proofs driving the real core over the wire
crates/hotline-app/     the Tauri 2 shell: plugins, the menu, the door
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
make dev        # the Tauri shell, Vite with hot reload, on .hotline-dev
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

`make dev` exports `HOTLINE_DATA_DIR` to `.hotline-dev` in the checkout, then
runs `cargo tauri dev` from `crates/hotline-app`, where `tauri.conf.json`
is. That starts Vite for the window on port 5174 and refuses any other
port, so a port already in use is a mistake rather than a second
instance. The shell generates a one-launch token, binds the door on
`127.0.0.1` with an ephemeral port, and sets `window.__hotlineDesk` to
`{platform, origin, token, version, dataDir}` before the page loads.

`make check` is, in order: `bun install --frozen-lockfile` and `bun run
typecheck` in `ui/`; the Python release-manifest tests; `cargo fmt --all --check`; `cargo clippy --workspace
--all-targets -- -D warnings`; `cargo test --workspace`. A window that
does not compile is a broken build however green the Rust is.

`make verify` is `cargo test --workspace --test '*'`: the integration
tests under `crates/hotline-core/tests/`, not the unit tests that live beside
the code. `make check` already runs those unit tests as part of
`cargo test --workspace`. Two of those tests talk to a real agent and are
skipped unless you set the env they name — see [The harness](#the-harness).

## Application updates

Settings → Updates checks GitHub every six hours while a packaged Hotline is
running, with a first check after 20 seconds when due. **Check now** bypasses
that interval. The last attempt and available version are stored in
`<data dir>/updater.json`; restarting Hotline preserves the interval. A new
installed version starts a fresh check. Failures keep the previous offer.
Release notes are plain text, and cached offers never authorize installation:
Hotline rechecks the trusted endpoint and requires the version the user reviewed.

**Download, install and restart** uses `tauri-plugin-updater` 2.11.0, following
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
keychain. Run `cargo test -p hotline-core native_store_roundtrip -- --ignored`
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

The macOS menu (Ctrl, not Cmd — the window's own listener is Ctrl on
every platform): Settings `Ctrl+,`, Search `Ctrl+F`, New Teammate
`Ctrl+N`, Teammate `Ctrl+I`, Teammate 1–9 `Ctrl+1`…`Ctrl+9`. The App
menu is Settings, About, Quit. Help opens Keyboard shortcuts, About Hotline,
and Hotline on GitHub. Where there is no menu bar, the same three help items
sit under the More button beside Settings at the foot of the rail, and
the chords are the window's own.

The app icon is `assets/hotline-tile.svg`: the mark from `assets/hotline-mark.svg`
on a dark rounded tile, in the page's own colours. `make icons` runs
`cargo tauri icon` on it and writes every size the bundler wants into
`crates/hotline-app/icons/`, then drops the Android and iOS sets it also
produces; the PNGs are tracked so a checkout builds without the CLI's
rasteriser. The first PNG in `tauri.conf.json`'s icon list is the window's
own icon on Linux, which is why the 128px one leads it: X drops an icon
larger than a quarter megabyte, and 256px is a few bytes over. The same
target also renders the tray from the small drawing, `assets/hotline-mark-small.svg`,
whose wider cut between receiver and eyes survives at that size:
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
conversation, not a card over it. While Settings is open the rail is its
sections — General, Providers, Tools, Computer, Updates, Import — with a back key in its band
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

Tests use temporary directories of their own. Never point a test, a
harness, or `HOTLINE_DATA_DIR` at a real Hotline data directory.

One desk runs per data directory. Before the room opens, a release build
takes an exclusive lock on `desk.lock` there and writes, beside it, the
loopback port it answers on to `desk.wake`. A second launch finds the lock
held, asks the running desk to show its window, and exits once that desk
says it has. Two desks on one room would both fire its schedules, both
answer its phones and both write its streams. The system releases the lock
however the process ends. A desk that holds the lock without answering is
waited on for twenty seconds, because a restart after an update starts the
new process while the old one is still leaving, and a desk that is still
opening answers only once its window exists. After that the launch exits
and the room stays with the desk that has it. A desk on another data
directory is another room and runs alongside. A development build (`make
dev`, `cargo tauri dev`, any debug build) takes no lock, so a checkout runs
next to the installed app. The code is `crates/hotline-app/src/instance.rs`.

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
`COMPUTER_VERSION` in `crates/hotline-core/src/computer/mod.rs`, and the
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

Types are defined once in `crates/hotline-core/src/contract.rs` (serde +
ts-rs). `cargo test -p hotline-core` runs the export tests and writes
`ui/src/generated/contract.ts`. `.cargo/config.toml` points ts-rs at that
directory and tells it a 64-bit integer is a JavaScript `number`, because
every number on this wire arrives as `JSON.parse` produces it.

A filtered run (`cargo test session`) writes only the types it matched
and leaves a file the window cannot compile against. Run
`cargo test -p hotline-core` unfiltered before committing that file; `make
check` catches it. Never run a filtered test as the last run.

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
A connected sign-in lists its models live, so mirror `CHATGPT_MODELS` from
the Codex model list when you sync.

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

The `mcp/oauth.rs` unit harness runs a loopback protected-resource and
authorization server through metadata discovery, DCR, PKCE callback, token
refresh and authenticated MCP calls. It also covers restart reuse, denied and
wrong-state callbacks, registration gaps, concurrent refresh, URL binding,
sign-out and the protected vault boundary. No real provider account is used by
the test suite.

`desk.rs` has four tests. The first creates a teammate, watches the
roster view, lists models, adds a credential, and deletes the teammate —
no provider key required. The second lists a peer thread, subscribes to
it, and marks a message read. The third starts a Hotline Agent turn with a
real key, has it read a file, and checks the tape. It is skipped unless
`HOTLINE_HARNESS_ANTHROPIC_KEY` is set. The fourth is the same proof for the
other kind of agent: set `HOTLINE_HARNESS_ACP` to a backend id this machine
can run (`cursor`, say) and it drives a real harness as a child.

```bash
HOTLINE_HARNESS_ANTHROPIC_KEY=… make verify
HOTLINE_HARNESS_ACP=cursor make verify
```

A harness that wants state in a stream asks the core over the wire, or
writes the file before the core starts. The core is the only process that
appends to a stream.

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
