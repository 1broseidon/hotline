---
id: task-101
title: "Desktop: a relay's serves is read from its ready frame, so a relay switched to agents,relay is not shown as relay-only"
column: todo
position: 2
description: |-
  Left by task-98/99: relays.json keeps a relay's `serves` as pairing (or the owner→relay move) said it, and nothing refreshes it. task-99 proved standing in never reads it, so this is display only: after `hotline serve --capabilities agents,relay`, the Relay picker still labels the server "Relay only".

  Pick the smaller fix and say why in the commit: either (a) the core's stand-in keeps the ready frame's `serves` and `remote.relays`/status reports that (the core must not write relays.json: the shell is its only writer, task-98), or (b) the UI stops claiming more than it knows, e.g. labels by how this desktop holds it ("Relay" grant) rather than by what the server runs. Delete whatever the chosen path replaces.

  Tests: if (a), extend crates/hotline-cli/tests/relay.rs step 6 to show the new serves; if (b), ui/tests/relay-picker.test.tsx.
priority: low
tags:
  - relay
  - ui
relatedFiles:
  - crates/hotline-core/src/remote/relay.rs
  - crates/hotline-app/src/desks.rs
  - ui/src/components/RemoteSection.tsx
  - ui/tests/relay-picker.test.tsx
parentId: epic-11
contract:
  status: ready
  validation:
    commands:
      - cargo clippy --workspace --all-targets -- -D warnings
      - cd ui && bun install --frozen-lockfile && bun run typecheck && bun run test
  metrics:
    readyAt: "2026-10-17T09:15:29.082Z"
createdAt: "2026-10-17T09:15:29.087Z"
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-17T09:15:32.466Z"
  until: done
  tries: 0
assignee: claude
updatedAt: "2026-10-17T09:15:32.593Z"
---

## Description
Left by task-98/99: relays.json keeps a relay's `serves` as pairing (or the owner→relay move) said it, and nothing refreshes it. task-99 proved standing in never reads it, so this is display only: after `hotline serve --capabilities agents,relay`, the Relay picker still labels the server "Relay only".

Pick the smaller fix and say why in the commit: either (a) the core's stand-in keeps the ready frame's `serves` and `remote.relays`/status reports that (the core must not write relays.json: the shell is its only writer, task-98), or (b) the UI stops claiming more than it knows, e.g. labels by how this desktop holds it ("Relay" grant) rather than by what the server runs. Delete whatever the chosen path replaces.

Tests: if (a), extend crates/hotline-cli/tests/relay.rs step 6 to show the new serves; if (b), ui/tests/relay-picker.test.tsx.
