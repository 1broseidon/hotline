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
updatedAt: "2026-10-17T07:47:18.837Z"
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
---

## Description
Docs and packaging: a relay on a Raspberry Pi. Last: it describes what task-99 proved works end to end.

- docs-site: "Run a relay" under Server (`docs-site/src/content/docs/server/relay.md`, added to the sidebar). Covers the aarch64 hotline-server build, a systemd unit for `--capabilities relay` (packaging/), pairing a desktop as relay (`hotline pair --link --role relay`), choosing it under Settings › Remote › Relay, the measured footprint (from docs/serve.md, with its platform stated), and switching a relay to a full server later.
- docs/serve.md: capabilities and roles, as shipped. docs/security.md: check the relay role's matrix row and test index match the tests that exist (task-91, task-99).
- CHANGELOG line.
