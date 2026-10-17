---
schema: https://brainfile.md/v2/board.json
title: Hotline
agent:
  instructions:
    - Task files are individual .md files in board/
    - Completed tasks are in logs/
    - Preserve all IDs
    - Make minimal changes
related:
  mobile: ../hotline-mobile
columns:
  - id: todo
    title: To do
  - id: in-progress
    title: In progress
  - id: review
    title: Review
  - id: backlog
    title: Backlog
  - id: done
    title: Done
    completionColumn: true
---

# Hotline

The Hotline desk: the Rust core, the Tauri window, the CLI and the served desk. Phone work lives on the hotline-mobile board.

**Will do** is the backlog we mean to ship, ordered by priority, with a release tag (`0.36.1`, `0.37.0`) once it is planned. **Won't do** holds what we decided against or parked, each with a note saying why; move it back if that changes. Done work leaves the board through `brainfile complete`.

> Note: Completing a task moves it to `logs/` via `brainfile complete`.
