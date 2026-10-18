---
id: task-171
title: "P2-56 (F): The license gate"
column: review
position: 60
priority: high
tags:
  - phase2
  - gates
parentId: epic-13
createdAt: "2026-10-17T22:41:12.690Z"
relatedFiles:
  - deny.toml
  - Makefile
  - THIRD-PARTY-NOTICES.md
updatedAt: "2026-10-18T10:28:32.548Z"
contract:
  status: delivered
  validation:
    commands:
      - grep -q '^\[licenses\]' deny.toml
      - make check
      - make precheck
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T10:14:38.357Z"
    reworkCount: 1
    deliveredAt: "2026-10-18T10:28:32.548Z"
    duration: 834
dependsOn:
  - task-170
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T10:14:25.574Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T10:14:28.874Z"
assignee: claude
---

## Description
Phase 2 group F (Gates), plan row 56 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**The license gate.** A reviewed cargo-deny allow list, and a UI license and notice inventory covering fonts, bundled distributions and the native speech assets

- Source: dead-code F4, task 18
- Files: `deny.toml`, the notice inventory
- Collides with: 13, 55
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T10:28:32.061Z: [claude] Done on branch dispatch/task-171 (1a880b46): reviewed deny.toml [licenses] allow list (every entry in use, commented), scripts/window_licenses.py (+test) failing on any unreviewed license in the window's production closure, both run by a new `make licenses` in `make check`; THIRD-PARTY-NOTICES.md inventories crates, window packages, IBM Plex (OFL-1.1), mermaid.min.js's bundled deps and the sherpa-onnx archive's libraries. make check and make precheck pass. Findings for George: elkjs (EPL-2.0) ships inside mermaid.min.js and the audit missed it; the sherpa archive holds eSpeak NG (GPL-3.0-or-later), but macOS arm64 binaries have no eSpeak symbols (Linux/Windows unchecked); the notices file is not in the app bundle. Left unwired for task-170 (its work is uncommitted in its worktree): deny.toml is created by both (add/add conflict: keep 170's header and [advisories], add this [licenses]); both Makefiles pin DENY_VERSION 0.20.2 (fold `licenses` into 170's `deps`, one pin, `cargo deny --locked check advisories licenses`); CI runs scripts/window_licenses.py in the check job, but cargo-deny licenses needs 170's dependencies job (change its step to `check advisories licenses`).
