---
id: task-185
title: Computer settings say 'applies after restart' for memory and CPUs
column: todo
position: 11
priority: high
tags:
  - computer
  - ui
parentId: epic-14
createdAt: "2026-10-18T23:40:55.776Z"
---

## Description
Slice of task-27. computer/container.rs:143: CPU and memory only apply when the container is next created, but the settings imply they take effect now. Say 'Applies the next time the computer starts' under those controls, and show a 'Restart to apply' action when they differ from the running container. Live update (docker or podman update) and Apple container home persistence stay in task-27.

Split from task-27, which keeps the rest and stays deferred.
