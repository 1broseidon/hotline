---
id: task-117
title: "P2-5 (A): sherpa build ZIP path"
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
updatedAt: "2026-10-18T01:37:58.142Z"
contract:
  status: delivered
  validation:
    commands:
      - python3 scripts/fetch-speech-engine
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T01:25:22.099Z"
    reworkCount: 1
    deliveredAt: "2026-10-18T01:35:14.482Z"
    duration: 592
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
completedAt: "2026-10-18T01:37:58.142Z"
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
- 2026-10-18T01:27:48.754Z: Not landed: Claude Code is still working on task-117
- 2026-10-18T01:35:14.037Z: [claude] Done on dispatch/task-117 (d4703abf). No sherpa-onnx-sys release on crates.io takes ZIP 9: 1.13.8 is the latest and its build-dep is zip ^2. Upstream merged the fix on 2026-10-17 (k2-fsa/sherpa-onnx#4059, commit 1a9d234a), changing only that crate's Cargo.toml, zip "2" to "9". Its build.rs is byte-identical to v1.13.8. So the root Cargo.toml has a [patch.crates-io] that points sherpa-onnx-sys at that git rev, which Cargo.lock pins by commit. The version is still 1.13.8, so the archives and scripts/speech-engine.sha256 do not change: there were no hashes to refresh. fetch-speech-engine re-downloaded the osx-arm64 archive and its hash matched. SHERPA_ONNX_ARCHIVE_DIR still means the build never downloads anything itself. Lockfile: zip 2.4.2 is gone (only zip 9.0.5 is left), along with xz2/lzma-sys/bzip2 0.5. zip 9's defaults add aes 0.9/zstd 0.14/lzma-rust2/ppmd-rust as build-only deps, and zstd-sys moves 2.0.16 to 2.1.2, shared with arrow's zstd 0.13. cargo deny advisories now reports no vulnerabilities, only the 9 unmaintained crates. I also fixed docs/security.md:246, which wrongly said TLS was the only check on the speech archive. make check and make precheck pass (7/7, Windows included). For later: when sherpa-onnx publishes a release with ZIP 9 (1.13.9+), drop the [patch], bump the =1.13.8 pin in hotline-core and refresh the five hashes from that release. Task 55's dependency gate should list this patch as a review condition. Note that the iOS .xcframework.zip branch is the only caller of ZIP in that build script, and Hotline never takes it.
- 2026-10-18T01:37:57.770Z: Landed on release/0.40.0
