---
id: task-95
title: "Docs and packaging: Run a relay on a Raspberry Pi"
column: todo
position: 9
priority: high
tags:
  - relay
  - docs
parentId: epic-11
createdAt: "2026-10-17T05:37:03.487Z"
dependsOn:
  - task-99
updatedAt: "2026-10-17T09:15:31.958Z"
relatedFiles:
  - docs-site/src/content/docs/server/relay.md
  - docs-site/src/content/docs/server/index.md
  - docs-site/astro.config.mjs
  - packaging/hotline-relay.service
  - packaging/hotline.service
  - docs/serve.md
  - docs/security.md
  - CHANGELOG.md
contract:
  status: ready
  validation:
    commands:
      - test -f docs-site/src/content/docs/server/relay.md
      - grep -q -- '--capabilities relay' packaging/*.service
      - cd docs-site && bun install --frozen-lockfile && bun run build
      - grep -qi 'relay' CHANGELOG.md
  metrics:
    readyAt: "2026-10-17T07:47:18.837Z"
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-17T09:15:31.841Z"
  until: done
  tries: 0
assignee: claude
---

## Description
Docs and packaging: a relay on a Raspberry Pi. Last: it describes what task-99 proved works end to end. Use the names in docs/adr/0006-relay-only-servers.md.

- docs-site: "Run a relay" under Server (`docs-site/src/content/docs/server/relay.md`, added to the sidebar). Covers the aarch64 hotline-server build, a systemd unit for `--capabilities relay` (packaging/), pairing a desktop as relay (`hotline pair` on a relay-only server; `hotline pair --relay` on one that also runs agents), choosing it under Settings › Remote › Relay, the offer to drop an owner grant to relay, the measured footprint (from docs/serve.md, with its platform stated), and switching a relay to a full server later (`serve.json`, pairings kept, agents set up as a new server).
- docs/serve.md: capabilities and roles, as shipped. docs/security.md: check the relay role's matrix row and test index match the tests that exist (task-91, task-92, task-98, task-99).
- CHANGELOG line.

## Log
- 2026-10-17T09:07:50.245Z: [claude] [lead] From task-100: the real CLI flag is `hotline pair --relay --link` (task-93's UI hint said `--role relay`, now fixed); check every command you document against `hotline pair --help`. Also describe pasting a relay link into "Add a server": it says "Added X as a relay" and chooses it only when no relay is chosen yet, else points to Settings › Remote › Relay. List task-94, task-100 tests alongside 91/92/98/99 when checking security.md's test index.
- 2026-10-17T09:15:28.825Z: [claude] [lead] From task-99 (1be2eca9, crates/hotline-cli/tests/relay.rs): on a relay-only server /v2, /v2/computer and /v2/link have no route (404 before Noise); role_not_permitted applies only once the server also runs agents. Document it that way, and add the relay.rs harness to security.md's test index. No aarch64/Pi footprint was measured: state the platform the docs/serve.md figure comes from and say the Pi figure is unconfirmed; do not invent one.
