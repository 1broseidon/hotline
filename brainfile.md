---
schema: https://brainfile.md/v2/board.json
title: Hotline
agent:
  instructions:
    - Task files are individual .md files in board/
    - Completed tasks are in logs/
    - Preserve all IDs
    - Make minimal changes
columns:
  - id: todo
    title: To Do
  - id: in-progress
    title: In Progress
---

# Hotline

The Hotline desk: the Rust core, the Tauri window, the CLI and the served desk. Phone work lives on the hotline-mobile board.

> Note: Completing a task moves it to `logs/` via `brainfile complete`.
