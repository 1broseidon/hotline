---
id: task-117
title: "P2-5 (A): sherpa build ZIP path"
column: in-progress
position: 6
priority: high
tags:
  - phase2
  - security
parentId: epic-13
createdAt: "2026-10-17T22:40:44.541Z"
relatedFiles:
  - crates/hotline-core/Cargo.toml
  - scripts/fetch-speech-engine
  - scripts/speech-engine.sha256
  - Cargo.lock
updatedAt: "2026-10-18T01:27:07.576Z"
contract:
  status: in_progress
  validation:
    commands:
      - python3 scripts/fetch-speech-engine
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T01:25:22.099Z"
    reworkCount: 1
dependsOn:
  - task-116
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:38.328Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-18T01:24:36.226Z"
    ended: "2026-10-18T01:27:07.576Z"
    outcome: review
assignee: claude
---

## Description
Phase 2 group A (Security), plan row 5 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**sherpa build ZIP path.** Move to a patched upstream build release and refresh the archive hashes. Builds still never use an unverified download

- Source: dead-code F1, task 4
- Files: `crates/hotline-core/Cargo.toml` (sherpa), `scripts/fetch-speech-engine`, `scripts/speech-engine.sha256`, `Cargo.lock`
- Collides with: 2, 3, 4 (lockfile)
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T01:27:03.196Z: [claude] A second session (opus, not the dispatch claim) stepped back without changing anything: at 01:26Z the claimed worker (PID 25177) was already editing this worktree and running make check. Research agrees with its approach. No sherpa-onnx-sys release after 1.13.8 exists. 1.13.7 and later need zip ^2, used only for the iOS xcframework.zip. Downgrading to 1.13.6 would bring back the Moonshine silent-audio hallucination that 1.13.8 fixed. Upstream commit 1a9d234ab9 (#4059, 2026-10-17) changes only the sys crate's zip line, from 2 to 9, so a [patch.crates-io] pin to it keeps the v1.13.8 archives and their pinned hashes.
