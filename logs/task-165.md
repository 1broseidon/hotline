---
id: task-165
title: "P2-D-5 (E): One module map,"
priority: high
tags:
  - phase2
  - docs
parentId: epic-13
createdAt: "2026-10-17T22:41:09.515Z"
relatedFiles:
  - docs/development.md
  - docs/design.md
updatedAt: "2026-10-18T06:35:19.766Z"
contract:
  status: delivered
  validation:
    commands:
      - "! grep -n 'defined once in contract.rs' docs/development.md docs/design.md"
      - git diff --quiet $(git merge-base HEAD release/0.40.0) -- crates ui
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T06:27:54.228Z"
    reworkCount: 0
    deliveredAt: "2026-10-18T06:34:59.771Z"
    duration: 426
dependsOn:
  - task-164
  - task-129
  - task-131
  - task-132
  - task-133
  - task-136
  - task-138
  - task-139
  - task-140
  - task-141
  - task-142
  - task-172
  - task-173
  - task-174
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:25.543Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T06:27:44.860Z"
    ended: "2026-10-18T06:35:19.766Z"
    outcome: done
assignee: claude
completedAt: "2026-10-18T06:35:19.348Z"
---

## Description
Phase 2 group E (Docs), plan row D-5 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**One module map,** in `docs/development.md`, matching the tree after C, with `design.md` linking to it. Correct the "defined once in contract.rs" line

- Source: structure F11, task 21
- Files: `docs/development.md:45-72,396`, `docs/design.md:634-660`
- Collides with: all of C (land last); D-4 (same file)
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T06:35:03.889Z: [claude] Done on dispatch/task-165, commit 597f1417. Docs only. development.md "Layout" is now the only module map. It lists every top-level entry in crates/hotline-core/src after C, adding voice/, websearch/, imagegen/, providers/, credentials/, thread/, sent/, spending, push, pricing, capabilities, images, thumbs, fence, link_preview, room_lock, log_file and process_windows, plus crates/hotline-cli. design.md "Module map" is now a link to development.md#layout. The "defined once" line was wrong in both files. development.md "The generated contract" now says each wire type is defined once in Rust: most in contract/, which is a leaf; the rest beside remote.rs, remote/{client,relay,v2}.rs, linked.rs and import.rs. design.md §2 says the same and links there. make precheck: 6 of 7 legs passed on the first run. The windows leg failed on a network timeout (URLError), then passed when rerun alone. Left alone because it is outside this row: design.md:137 still says "George runs the old app daily until Phase 1 replaces it" (D-4 left this one open too), and a tracked bindings/contract.ts at the repo root looks stale. D-6 (standards.md) can go ahead: I did not write the module or test layout rules into development.md.
- 2026-10-18T06:35:18.933Z: Landed on release/0.40.0
