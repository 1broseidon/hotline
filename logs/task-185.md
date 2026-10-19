---
id: task-185
title: Computer settings say 'applies after restart' for memory and CPUs
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
updatedAt: "2026-10-19T00:33:23.983Z"
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
    ended: "2026-10-19T00:31:11.284Z"
    outcome: review
assignee: claude
completedAt: "2026-10-19T00:33:23.983Z"
---

## Description
Slice of task-27. computer/container.rs:143: CPU and memory only apply when the container is next created, but the settings imply they take effect now. Say 'Applies the next time the computer starts' under those controls, and show a 'Restart to apply' action when they differ from the running container. Live update (docker or podman update) and Apple container home persistence stay in task-27.

Split from task-27, which keeps the rest and stays deferred.

## Log
- 2026-10-19T00:31:06.874Z: [claude] Done on dispatch/task-185 (31e7cd61). The description's premise was half right: a stop and start reused the container, so new limits never applied. Now a container records the limits it was made with (HOTLINE_LIMITS env, next to HOTLINE_FOLDERS), and ensure_running makes it again when they differ. A running container from before this change is kept, and is made again the next time it starts from stopped, so upgrading never tears down a live desktop. ComputerStatus gains limitsChanged (a running computer with a record that differs). The new computer.restart command (desk-only, not for the phone) puts a swap in computer_swaps marked ready, so it lands between turns the way an update does; with no session it only stops. Under the sliders the pane says 'Applies the next time the computer starts.', and 'Restart to apply' shows when limitsChanged. Computer::status now takes Option<&Persona>. Tests: computer::tests a_computer_made_with_other_limits_is_made_again, session tests a_restart_makes_the_computer_again… and restarting_a_resting…, a seats test, and ui/tests/computer-restart.test.tsx. Green: fmt, clippy -D warnings, cargo test -p hotline-core (1892), the contract diff, make ui-check (433). Left for task-27: live docker/podman update and Apple home persistence (an Apple restart still loses its home, as an update does). The phone owner card has no restart.
- 2026-10-19T00:33:23.562Z: Landed on release/0.40.0
