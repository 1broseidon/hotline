---
id: task-248
title: Faster linkers on Windows and Linux
column: in-progress
position: 0
priority: high
tags:
  - ci
  - build
parentId: epic-23
createdAt: "2026-10-21T02:18:27.672Z"
dependsOn:
  - task-247
updatedAt: "2026-10-21T03:41:33.565Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T03:40:47.579Z"
  until: review
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T03:40:48.803Z"
assignee: codex
---

## Description
Done when:
- Windows MSVC targets link with rust-lld (lld-link) through .cargo/config.toml, keeping +crt-static. Linux links with mold or lld in CI and in the documented local setup. macOS keeps Apple's linker.
- CI passes on all three OS, and a release build of each target still produces a working binary (the release-cache jobs on a main-like run, and a desktop-release dry run on the branch).
- The PR records before/after link-heavy timings: CI's Windows and Linux test steps, and cargo test --no-run locally on Linux (the QA computer).

## Log
- 2026-10-21T03:43:05.999Z: [codex] Implementing bundled rust-lld/lld-link on Windows with crt-static unchanged and explicit -fuse-ld=lld on both shipped GNU/Linux targets. Stable x86_64 Linux already defaults to LLD, so no speedup is assumed. CI/release/container installs cover ARM and self-hosted runners; task-247 release-cache guard/keys untouched. Branch cannot run new hosted CI without a push (explicitly forbidden); QA host details requested. make check running.
