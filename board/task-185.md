---
id: task-185
title: Computer settings say 'applies after restart' for memory and CPUs
column: review
position: 11
priority: high
tags:
  - computer
  - ui
parentId: epic-14
createdAt: "2026-10-18T23:40:55.776Z"
relatedFiles:
  - crates/hotline-core/src/computer/container.rs
  - ui/src/components/settings
  - ui/src/generated/contract.ts
updatedAt: "2026-10-19T00:31:01.866Z"
contract:
  status: delivered
  validation:
    commands:
      - cargo fmt --all --check
      - cargo clippy --workspace --all-targets -- -D warnings
      - cargo test -p hotline-core
      - git diff --exit-code ui/src/generated/contract.ts
      - make ui-check
  metrics:
    readyAt: "2026-10-18T23:53:55.554Z"
    pickedUpAt: "2026-10-19T00:16:08.580Z"
    reworkCount: 0
    deliveredAt: "2026-10-19T00:31:01.866Z"
    duration: 893
dependsOn:
  - task-180
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-19T00:15:54.638Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59290
    at: "2026-10-19T00:15:57.030Z"
assignee: claude
---

## Description
Slice of task-27. computer/container.rs:143: CPU and memory only apply when the container is next created, but the settings imply they take effect now. Say 'Applies the next time the computer starts' under those controls, and show a 'Restart to apply' action when they differ from the running container. Live update (docker or podman update) and Apple container home persistence stay in task-27.

Split from task-27, which keeps the rest and stays deferred.
