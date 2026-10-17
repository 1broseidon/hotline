---
id: task-117
title: "P2-5 (A): sherpa build ZIP path"
column: todo
position: 6
priority: high
tags:
  - phase2
  - security
parentId: epic-13
createdAt: "2026-10-17T22:40:44.541Z"
---

## Description
Phase 2 group A (Security), plan row 5 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**sherpa build ZIP path.** Move to a patched upstream build release and refresh the archive hashes. Builds still never use an unverified download

- Source: dead-code F1, task 4
- Files: `crates/hotline-core/Cargo.toml` (sherpa), `scripts/fetch-speech-engine`, `scripts/speech-engine.sha256`, `Cargo.lock`
- Collides with: 2, 3, 4 (lockfile)
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
