---
id: task-247
title: release-cache runs only where its cache is saved
column: in-progress
position: 0
priority: high
tags:
  - ci
  - build
parentId: epic-23
createdAt: "2026-10-21T02:18:27.195Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T02:18:39.412Z"
  until: review
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T02:18:42.121Z"
assignee: codex
updatedAt: "2026-10-21T02:18:57.900Z"
---

## Description
Done when:
- The release-cache matrix in ci.yml runs on pushes to main (and dispatches on main) only, where save-if is true. A dispatch on a release branch runs none of it.
- desktop-release still restores a warm release-<target> cache after a main push (shown by a cache hit in a dry run, or by its log).
- The PR records the runner minutes a release-branch dispatch used before and after.

## Log
- 2026-10-21T02:20:04.449Z: [codex] Guard added: release-cache requires refs/heads/main and push/workflow_dispatch; save-if and desktop-release shared keys unchanged. PR evidence: baseline release/0.42.0 dispatch https://github.com/1broseidon/hotline/actions/runs/39522820228 used 61.95 release-cache runner-minutes (Intel Mac 19.07, ARM Mac 15.35, Windows 15.03, Linux 12.50; sum of job completed_at-started_at, no billing multipliers). After: 0 release-cache minutes by local job-condition evaluation, not an executed hosted run; seven event/ref cases pass. Baseline whole workflow 141.90 minutes; holding other jobs fixed gives 79.95 minutes, a projection only. desktop-release https://github.com/1broseidon/hotline/actions/runs/39375382651 logs restore-key hits for release-x86_64-apple-darwin ...fc985acc-60d651fe and release-x86_64-pc-windows-msvc ...e70cdc14-52398585, matching main push run https://github.com/1broseidon/hotline/actions/runs/39208610138. No push or new hosted dispatch performed.
