---
title: Install on a server
description: The binary, a service account, and a systemd unit.
---

## Quick install

On a Linux server with systemd:

```sh
curl -fsSL https://hotline.dev/install | sh -s -- --server
```

It downloads the server build for your architecture, checks it against the
release's checksums, installs `hotline` to `/usr/local/bin`, creates the
`hotline` account and the unit below, and starts the desk. It asks for the
[listen address and public URL](#listen-address-and-tls), or takes them as
`--listen` and `--public-url`. Once the desk is up it shows a pairing QR, so
keep your phone handy.

The installer detects an existing install and reports its version and the
available version. It asks before updating when a terminal is attached;
`--yes` skips that question, as does running without a terminal. An install
already at the requested version is left alone unless you pass `--force`.
Pass `--listen` and `--public-url` again to change them (add `--force` when
keeping the same version); if the desk won't start with the new values,
the previous unit goes back.

## Update a server

For a server installed at `/usr/local/bin/hotline` with the `hotline` systemd
unit:

```sh
hotline update --check              # report the installed and available versions
sudo /usr/local/bin/hotline update  # install the latest server release
```

To choose a particular release, including an intentional downgrade:

```sh
sudo /usr/local/bin/hotline update --version X.Y.Z
```

The updater downloads the server archive for your architecture, verifies its
SHA-256 checksum, and replaces only the binary with an atomic rename. The
room, pairings and systemd unit stay untouched. If the unit is running, it
restarts and the updater checks that the desk comes back; a failed start
restores the previous binary and tries to restart it. A stopped unit stays
stopped. Without root, the command prints the exact `sudo` command to run;
`--check` never needs root and never changes the install.

Rollback restores the executable, not room-data changes a newer release may
have made. Keep an encrypted backup of the room, taken with the service stopped,
before upgrading or downgrading. If recovery also fails, the updater preserves
the old binary and prints its backup path; inspect `journalctl -u hotline`
before recovering manually.

This command is only for the Linux server binary. Desktop installs use the
app's updater or their package manager; `hotline update` does not overwrite
`.deb`, `.rpm`, AppImage, macOS app or Windows setup installations.

Older releases, including 0.26.0, do not have `hotline update`. For that first
upgrade, rerun the installer once:

```sh
curl -fsSL https://hotline.dev/install | sh -s -- --server
```

It keeps the existing room, pairings and unit. Use `hotline update` after that.
An older binary without `--version` may be reported as `unknown` when its
running desk's version is not readable; the installer still offers the upgrade.

## Manual install

The rest of this page is the same install done by hand.

## Get the binary

Each [release](https://github.com/1broseidon/hotline/releases/latest) from
0.26.0 carries `hotline-server_<version>_linux_x86_64.tar.gz` and
`hotline-server_<version>_linux_aarch64.tar.gz`, each with the `hotline`
binary and a systemd unit. To build it yourself instead:

```sh
cargo build --release -p hotline-cli   # target/release/hotline
```

## Install it as a service

```sh
tar xzf hotline-server_*_linux_x86_64.tar.gz
sudo install -m 0755 hotline-server_*/hotline /usr/local/bin/hotline
sudo useradd --system --create-home --home-dir /var/lib/hotline --shell /usr/sbin/nologin hotline
sudo install -m 0644 hotline-server_*/hotline.service /etc/systemd/system/hotline.service
sudo systemctl edit --full hotline     # set --listen and --public-url, below
sudo systemctl daemon-reload
sudo systemctl enable --now hotline
```

The unit runs as the `hotline` account with the room at
`/var/lib/hotline/room`. It sets `HOME`, `PATH` and `HOTLINE_DATA_DIR`
itself and reads no login shell, so add to its `PATH` what your teammates
need: a Node install for harnesses run through `npx`, for example.

## Listen address and TLS

`--listen` and `--public-url` are both required.

```sh
hotline serve --store file --listen 192.0.2.10:9443 --public-url https://desk.example:9443
```

- `--listen` is exactly one local IP and a fixed port. Wildcards (`0.0.0.0`),
  port zero and falling back to another address are refused. IPv6 goes in
  brackets: `--listen '[2001:db8::10]:9443'`. If the address isn't up yet at
  boot, the desk waits for it.
- `--public-url` is where the phone connects: an `https://` address with a
  DNS name or IP, or a TLS proxy in front of the desk. It goes into the
  pairing QR. Don't put credentials or a query in it.
- TLS is self-signed unless you pass `--tls-cert fullchain.pem --tls-key
  key.pem`, which the service account must be able to read. `--tls self`
  says the default out loud. The phone trusts the server's key, not its
  certificate, so rotating certificates never breaks a pairing.
- `--store file` keeps secrets as owner-only files under the room. Encrypt
  the disk, and its snapshots and backups. `native` uses the OS credential
  store, which a server usually doesn't have.

Open the port in your firewall for the phones that will connect, and nothing
else. Only a phone holding a pairing gets past the handshake; to anything
else the port offers nothing but TLS and a closed door.

## Check it's running

```sh
sudo -u hotline HOTLINE_DATA_DIR=/var/lib/hotline/room hotline status
```

A fresh room says **No teammate can run yet: connect a model provider**.
That is expected: [pair your phone](/docs/server/pair/), then
[connect a model](/docs/server/models/).
