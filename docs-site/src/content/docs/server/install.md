---
title: Install on a server
description: The binary, a service account, and a systemd unit.
---

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
