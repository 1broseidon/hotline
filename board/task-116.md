---
id: task-116
title: "P2-4 (A): Tauri updater ZIP path"
column: review
position: 0
priority: high
tags:
  - phase2
  - security
parentId: epic-13
createdAt: "2026-10-17T22:40:44.032Z"
relatedFiles:
  - crates/hotline-app/Cargo.toml
  - Cargo.lock
updatedAt: "2026-10-18T01:18:31.604Z"
contract:
  status: delivered
  validation:
    commands:
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T01:09:20.398Z"
    reworkCount: 1
    deliveredAt: "2026-10-18T01:18:31.604Z"
    duration: 551
dependsOn:
  - task-115
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:37.210Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59932
    at: "2026-10-18T01:08:55.025Z"
assignee: claude
---

## Description
Phase 2 group A (Security), plan row 4 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Tauri updater ZIP path.** Upgrade the owning plugin to a release with patched extraction, or file an upstream blocker with the IDs. Do not force ZIP 9 under a ZIP 4 API

- Source: dead-code F1, task 3
- Files: `crates/hotline-app/Cargo.toml`, `Cargo.lock`
- Collides with: 2, 3, 5 (lockfile)
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T01:10:58.757Z: The claim on task-116 (desktop on Georges-Mac-Studio, pid 50333) was not given back as review: task-116 is claimed by cli (pid 59932) on Georges-Mac-Studio
- 2026-10-18T01:11:02.107Z: Not landed: Claude Code is still working on task-116
- 2026-10-18T01:18:31.082Z: [claude] Done on dispatch/task-116 (27e3bc2a). No tauri-plugin-updater release takes patched ZIP. Every stable release through 2.14.0, and 3.0.0-alpha.3, still has zip ^4 behind its default `zip` feature, and ZIP 4 has no backport: RUSTSEC-2026-0367/0368/0369 (GHSA-vv5m-jxxh-4627, GHSA-4v9h-p7m9-rh26, GHSA-4684-h2q9-2vcp) are patched only in >=9. So instead of upgrading, hotline-app now uses the updater with default-features=false plus rustls-tls and system-proxy. Without `zip`, the plugin drops only its Windows .zip branch (extract() under ZIP 4) and its Linux .AppImage.tar.gz branch. Hotline publishes neither: Windows gets the bare NSIS setup.exe and Linux the bare AppImage, and the macOS .app.tar.gz reader does not depend on the feature. Cargo.lock loses zip 4.6.1 and nothing else changes. cargo deny now reports the three ZIP IDs only against zip 2.4.2, which comes through sherpa's build and is row 5 / the next task. make check and make precheck pass (7/7, Windows leg included). Not done: I filed no upstream issue, because that is outward-facing and needs George's go-ahead. If wanted, file it at tauri-apps/plugins-workspace asking tauri-plugin-updater to move to zip >=9 with those three IDs. Two risks for later: if someone switches the release to v1Compatible (zipped) updater artifacts, Windows updates will fail as an unknown installer; and the updater is still pinned at =2.11.0.
