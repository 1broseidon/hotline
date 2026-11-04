# hotline.dev

The landing page: one static file, `public/index.html`, shown in still
captures of the app, plus `og.png`, `favicon.svg`, `_headers`, `robots.txt`
and `sitemap.xml`. The docs are built into `public/docs/` from `../docs-site`
and are not tracked. Download links resolve against the latest GitHub
release at load time, so a release needs no change here.

```bash
make site-deploy   # from the repo root: builds the docs, deploys Worker `hotline-site`
```

## Captures

The desk on the page is captured by `showcase/capture.mjs`, at 2x on any
display: headless Chromium draws the window at a device scale of 2, so the
screen in front of you plays no part. Each capture is framed as the Mac draws
the window: rounded corners, a hairline edge, and the traffic lights where the
app seats them.

```bash
make dev                                   # serves the window on :5174; leave it running
cargo build -p hotline-cli                 # the `hotline` the capture serves the room with
cd site/showcase && bun install
node capture.mjs --room ~/path/to/room     # every shot in shots.json, into public/screens/
```

`--room` defaults to `.hotline-dev`, `--hotline` to `target/debug/hotline`
and `--ui` to `http://localhost:5174/`. Use a `hotline` and a window from the
same tree, or the window may refuse the desk. Chromium is Playwright's own:
`bunx playwright-core install chromium` if it has none.

The room is never opened. The script copies its streams to a temporary
directory and cuts every tape and thread at `cut` in `shots.json`, so
whatever happened after the story the page tells stays out. In the copy each
teammate works in the temporary directory and has no computer, because a
teammate's container is named for the teammate and the copy's would be the
room's own. The copy's desk is stopped and the directory removed when the
shots are done. A shot names the teammate, the window size, how far back from
the end of the tape it scrolls (`fromEnd`, in CSS pixels), and the width it is
written at. It may also name:

- `open`, the text of a thread line to open beside the tape;
- `clicks`, what to press next, in order (a menu by its key's label, an entry
  in it by its text);
- `clip`, an element to cut the shot to, with offsets, which leaves it a piece
  without a window frame;
- `hide`, keys to leave out, such as a model whose provider key stayed in the
  room's vault;
- `computer`, to show each teammate's computer switched on as the room has it,
  where the copy turned it off;
- `voice`, to show the voice controls (the mic, the call key beside it and the
  call key in the rail). The copy has no voice provider, so these are a stand-in:
  the script answers the window's `voice.status` itself and says this Mac hears
  speech.

A shot of `"kind": "browser"` is a teammate's desk browser as its pane shows
it: macOS's 28px title bar, Hotline's bar (the window's own `browser.html`,
told the teammate, the address and that the teammate is driving) and the page
at 1280 by 800, as the desk browser opens it. The page is a file in the room,
named by `page`, such as a site a teammate built in its workspace.

A still that is not the desk, such as a teammate's computer, is framed with a
title bar instead:

```bash
node capture.mjs frame computer.png ../public/screens/computer-window.webp --title "Goldie's computer"
```

The computer still on the page is `public/screens/computer-run.webp` from
commit `a0312a70`.

## Installers

Both installers report an existing installation and the available version before
installing. An exact version match exits without downloading the package unless
forced. A terminal gets a **y/N** confirmation; Enter declines. Explicit consent
or no attached terminal preserves unattended installation. Force does not imply
consent. Unknown versions are reported as `unknown`, not treated as absent.

```sh
curl -fsSL https://hotline.dev/install | sh
curl -fsSL https://hotline.dev/install | sh -s -- --version 0.26.0 --yes --force
curl -fsSL https://hotline.dev/install | sh -s -- --server --yes \
  --listen 192.0.2.10:9443 --public-url https://desk.example:9443
```

`--yes` only skips installation confirmation: a fresh server still needs its
listen address and public URL when no terminal is attached. `--user NAME` runs
a first-install desk as that user, with their home and PATH, so it sees their
`claude` and `codex`; run under `sudo` it defaults to the person who ran sudo,
and `--user hotline` keeps the separate service account. An upgrade never
changes the user, and `--user` on an installed desk is refused. Changing settings
at the same version needs `--force`. A subsequent server update should use
`hotline update --check` or `sudo hotline update`, not bootstrap the service again.
The server release asset is a `.tar.gz` containing the binary and unit, not a
standalone download.

On Windows the normal one-liner is unchanged. Use PowerShell's single-dash
parameters on a downloaded script or a script block when options are needed:

```powershell
irm https://hotline.dev/install.ps1 | iex
& ([scriptblock]::Create((irm https://hotline.dev/install.ps1))) -Version 0.26.0 -Yes -Force
```

Detection uses package metadata on Linux, bundle metadata on macOS, and per-user
uninstall registration (or executable metadata) on Windows. An older AppImage
may have no readable version. New script-installed AppImages record their version
beside the file, bound to its SHA-256 checksum; a binary replaced by the app's
updater makes that marker stale and its version unknown. Old server binaries may lack `--version`: readable `door.json`
metadata is used only when its PID's executable matches the installed binary;
otherwise the version is unknown. Detection never starts the desktop app or
asks for elevation just to discover a version.

### Server updates and recovery

`hotline update [--check] [--version X.Y.Z]` is for the script-installed Linux
server at `/usr/local/bin/hotline`. Checking is read-only and needs no root;
replacing the binary needs root. Package-owned executables, desktop apps and
AppImages must use their package manager or app updater instead. An explicit
version can select an older release; check compatibility before downgrading.

The updater verifies the archive checksum and atomically replaces the server
binary. It restarts and checks an active service; a stopped service stays stopped.
A failed replacement or restart restores the previous binary and attempts to
recover the service. If recovery fails too, retain the backup path reported by
the updater and inspect `journalctl -u hotline` before trying again. The rollback
is of the binary, not room data: keep an encrypted room backup, taken with the
service stopped, before upgrading or downgrading. The installer itself only
restores the previous unit on a failed settings change; it does not provide the
updater's binary rollback. Older releases need one installer upgrade before the
`hotline update` command is available.

## Isolated installer tests

From the repository root:

```sh
sh -n site/public/install
shellcheck site/public/install
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s site/tests -v
pwsh -NoProfile -File site/tests/test_install.ps1
pwsh -NoProfile -File site/tests/test_install_entrypoints.ps1
```

Tests replace network, privilege, package and service operations with fixtures.
They do not install Hotline or touch a live desk. The PowerShell test also parses
the production script and needs no Pester dependency.
