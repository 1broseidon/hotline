---
id: epic-1
title: Hotline runs headlessly on any server, controlled from the phone
type: epic
column: todo
position: 2
priority: medium
tags:
  - bro-16
createdAt: "2026-10-11T21:04:10.322Z"
description: |-
  Run the desk on any Linux box you can SSH into (a tailnet VM, a DigitalOcean droplet with a public IP, or behind an edge proxy on a real domain), and operate it from the phone. **The only way to link a phone is to SSH in and scan the QR that** `hotline pair` **prints.** No network endpoint issues, refreshes or recovers a credential.

  The final spec was agreed on 26 Sep 2026 (Clementine's proposal, merging this plan with the BRO-117 audit). Everything below is decided. The previous text is in the issue history.

  ## Shape

  One `hotline` binary (new `crates/hotline-cli`) runs Desk + Remote + Door under systemd with `User=hotline` and `HOTLINE_DATA_DIR=/var/lib/hotline`. There is no new server and no web UI. The CLI is a thin client of the Door on loopback, which it finds through `door.json`.

  ```
  hotline serve --store file --listen <ip:port> [--public-url wss://…] [--tls-cert … --tls-key …]
  hotline pair [--companion]     # 2-min single-use window, prints the QR, waits for the claim
  hotline status | devices | revoke <device>
  hotline wire <command> < request.json   # dev escape hatch, JSON on stdin only
  ```

  ## Decisions

  * **Secrets:** an explicit `FileStore` (`--store file`), with no fallback from the keychain. Files are 0600 in a 0700 directory, replaced atomically; symlinks are refused; errors are distinguished as missing, corrupt or unavailable. A `store.json` marker makes a backend mismatch refuse to start instead of regenerating the identity. There is no app-level encryption in v1: the operator's disk encryption is the boundary, and `docs/security.md` records that.
  * **Reach:** the operator names one listen address. There's no wildcard default and no ephemeral port fallback. At boot the desk waits for the address and never widens. Tailscale, a public IP and an edge proxy are install recipes, not code paths.
  * **TLS:** self-signed (the default) or a real certificate. The phone no longer pins the certificate.
  * **Sealed channel:** the desk has an X25519 identity key, and the QR carries its public half, the public URL and a single-use secret. Every connection runs a Noise IK handshake inside the WebSocket before any wire frame, using the desk's identity and a per-phone device key, then ChaCha20-Poly1305. Proxies and edges only ever see ciphertext. The bearer token stops travelling on the wire. `protocolVersion` 2; the desktop app moves to it too, and old pinned pairings keep working until the phone pairs again.
  * **Internet-facing:** without an open pairing window, only the handshake listens, and it answers only known device keys; everyone else is closed silently. The claim route exists only while `pair` runs. A served desk has no 6-digit route. Per-IP and half-open handshake limits apply.
  * **Owner seat:** a QR pairing makes an owner; `--companion` makes a restricted grant. The role is stored on the grant and published in the hello. Existing grants migrate to owner once. Companions lose teammate create, edit and delete. The owner seat is a fixed set on top of the phone seat (providers, credentials, welcome, schedules cancel/quiet, devices, desk.status, computer on/off) and never becomes `Seat::Desk`.
  * **Restart:** SIGTERM closes admission, drains for 30 s, persists accepted-but-unstarted prompts to `<data>/pending/`, and cancels what's still running with an "interrupted by restart" marker. One-shot schedules are tombstoned when they start. On boot, pending prompts are re-queued once. Nothing is replayed after SIGKILL, and nothing claims exactly-once.
  * **Phone-only scope:** Hotline Agent with built-in providers (API keys; ChatGPT, Copilot and xAI device sign-in). Claude Code and Codex come via BRO-132. OpenRouter is API-key only.

  ## Slices

  1. BRO-113**:** host and lifecycle: CLI, the core lock everywhere, FileStore, systemd, the restart semantics, a first cut of `status`, and Linux tarballs.
  2. BRO-114**:** trust: the sealed channel, listen/public-url/TLS modes, Remote on the wire, QR-only `pair`, the owner role. BRO-116**:** first turn: Connections on the phone, welcome, honest `backends.list`, and `deskVersion`/`role` in the hello.
  3. BRO-115**:** health. **BRO-135:** daily operation.

  ## Cut

  `desk.folders`/clone, `hotline logs`, `apply hotline.toml`, self-update, ACME inside the desk, a web UI, the fleet, reach/tools/MCP/secrets editing from the phone, OpenRouter/MCP OAuth, Face ID gating, Android, and any way to link other than the QR.

  Migrated from Linear BRO-16: https://linear.app/1broseidon/issue/BRO-16/hotline-runs-headlessly-on-any-server-controlled-from-the-phone (2026-10-11).

  ## Status (2026-10-11 review)

  Children done: BRO-113, BRO-114, BRO-117, BRO-127, BRO-132, BRO-139, BRO-140, BRO-151, BRO-152 (0.26–0.30). Left: BRO-115, BRO-135. BRO-116's leftovers were merged into BRO-160; BRO-138 was canceled.
updatedAt: "2026-10-11T21:04:56.188Z"
---
