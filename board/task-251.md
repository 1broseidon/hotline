---
id: task-251
title: "Local dev loop: linker setup and continuous check, documented"
column: backlog
position: 51
priority: high
tags:
  - ci
  - build
parentId: epic-23
createdAt: "2026-10-21T02:18:29.110Z"
---

## Description
Done when:
- docs/development.md says how to set up the faster linker on each OS, and recommends bacon (with a bacon.toml in the repo running check, clippy and the tests of the crate being edited) for continuous check.
- The ADR 0007 measurements are re-run after the changes above and appended.
