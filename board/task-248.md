---
id: task-248
title: Faster linkers on Windows and Linux
column: review
position: 0
priority: high
tags:
  - ci
  - build
parentId: epic-23
createdAt: "2026-10-21T02:18:27.672Z"
dependsOn:
  - task-247
updatedAt: "2026-10-21T03:55:32.043Z"
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
    ended: "2026-10-21T03:55:32.043Z"
    outcome: review
assignee: codex
decisions:
  - decision-8
---

## Description
Done when:
- Windows MSVC targets link with rust-lld (lld-link) through .cargo/config.toml, keeping +crt-static. Linux links with mold or lld in CI and in the documented local setup. macOS keeps Apple's linker.
- CI passes on all three OS, and a release build of each target still produces a working binary (the release-cache jobs on a main-like run, and a desktop-release dry run on the branch).
- The PR records before/after link-heavy timings: CI's Windows and Linux test steps, and cargo test --no-run locally on Linux (the QA computer).

## Log
- 2026-10-21T03:43:05.999Z: [codex] Implementing bundled rust-lld/lld-link on Windows with crt-static unchanged and explicit -fuse-ld=lld on both shipped GNU/Linux targets. Stable x86_64 Linux already defaults to LLD, so no speedup is assumed. CI/release/container installs cover ARM and self-hosted runners; task-247 release-cache guard/keys untouched. Branch cannot run new hosted CI without a push (explicitly forbidden); QA host details requested. make check running.
- 2026-10-21T03:55:17.008Z: [codex] Implemented on dispatch/task-248 (implementation 2006ccbf, follow-up evidence commit): Windows rust-lld/lld-link preserves +crt-static; both GNU/Linux targets explicitly use LLD with installs in CI, releases, isolation and precheck; macOS unchanged. task-247 guard/save-if/shared keys preserved. make check passed (2164/16); make precheck all 8 passed; ARM Ubuntu container strict clippy/workspace tests (2172/26) plus 13 opt-in tests passed after retrying a full shared disk with a fresh target volume. docs/audits/build/task-248.md contains hosted baseline Windows 896 s/Linux 547 s and PR-ready procedure. UNFINISHED acceptance evidence: no push permitted, so reviewer must push/run 3-OS CI, branch desktop-release dispatch and native package smoke tests, then main release-cache builds; QA SSH access unavailable, so before/after QA Linux timings still needed. No speedup claimed. Move to Review for person to inspect branch and arrange those runs.
- 2026-10-21T05:05:18.304Z: [claude] 2026-10-21 George: not worth it. Windows gain ~1 min, within run-to-run noise; Linux already links with LLD. Dropped; branch ci/lld-linkers left unmerged.
