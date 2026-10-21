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
