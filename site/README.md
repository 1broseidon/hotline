# hotline.dev

The landing page: one static file, `public/index.html`, carrying the launch
film as its hero, plus `og.png`, `favicon.svg`, `_headers`, `robots.txt` and
`sitemap.xml`. The docs are built into `public/docs/` from `../docs-site`
and are not tracked. Download links resolve against the latest GitHub
release at load time, so a release needs no change here.

```bash
make site-deploy   # from the repo root: builds the docs, deploys Worker `hotline-site`
```

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
listen address and public URL when no terminal is attached. Changing settings
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
