---
id: task-247
title: release-cache runs only where its cache is saved
column: backlog
position: 47
priority: high
tags:
  - ci
  - build
parentId: epic-23
createdAt: "2026-10-21T02:18:27.195Z"
---

## Description
Done when:
- The release-cache matrix in ci.yml runs on pushes to main (and dispatches on main) only, where save-if is true. A dispatch on a release branch runs none of it.
- desktop-release still restores a warm release-<target> cache after a main push (shown by a cache hit in a dry run, or by its log).
- The PR records the runner minutes a release-branch dispatch used before and after.
