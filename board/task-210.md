---
id: task-210
title: Timings after stage 2, in ADR 0007
column: review
position: 11
priority: medium
tags:
  - crate-split
  - measure
assignee: claude
relatedFiles:
  - docs/adr/0007-split-hotline-core.md
  - docs/standards.md
  - docs/security.md
  - AGENTS.md
parentId: epic-18
createdAt: "2026-10-19T14:07:42.521Z"
dependsOn:
  - task-209
updatedAt: "2026-10-19T16:52:43.361Z"
contract:
  status: delivered
  deliverables:
    - type: docs
      path: docs/adr/0007-split-hotline-core.md
      description: stage 2 timings, the stopping-rule verdict on step 3, and the whole-epic check
  validation:
    commands:
      - test -z "$(git diff --name-only $(git merge-base HEAD release/0.41.0) HEAD -- . ':!docs')"
      - grep -q 'hotline-mcp' docs/adr/0007-split-hotline-core.md
      - grep -q 'hotline-room\|room crate' docs/adr/0007-split-hotline-core.md
      - "! grep -rnE --exclude-dir=adr 'crates/hotline-core/src/(room|linked|session/ledger|wire/room_handle|mcp/oauth|mcp/tool)\\.rs' docs AGENTS.md"
      - "! grep -qE '^hotline-core[[:space:]]*=' crates/hotline-mcp/Cargo.toml"
      - make check
      - git diff --exit-code 9e2c01bc -- ui/src/generated/contract.ts
      - make precheck
  constraints:
    - "Docs only: no change outside docs/"
    - Timings taken on the same machine and conditions at each of the three commits
  metrics:
    readyAt: "2026-10-19T14:10:20.086Z"
    pickedUpAt: "2026-10-19T15:35:56.671Z"
    reworkCount: 0
    deliveredAt: "2026-10-19T16:52:43.361Z"
    duration: 4607
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T15:35:43.429Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-19T15:35:45.996Z"
---

## Description
The last child: time stage 2 and check the epic whole against its "done when". Docs only.

Timings: re-run ADR 0007's commands at three commits on this machine, under the same conditions, each in its own worktree: the release/0.41.0 commit before the epic (9e2c01bc, to re-baseline), the merge of task-208 (the room crate), and the merge of task-209 (the MCP client). Add the rows beside the earlier ones: cold and incremental build and test build, the core's front-end time, and what got faster and what didn't. Say whether the numbers support going on to the voice engines (step 3), as the ADR's stopping rule asks.

The whole: on the tip of release/0.41.0, confirm and state in the ADR that (1) hotline-room and hotline-mcp are workspace crates and neither depends on hotline-core; (2) contract.ts is byte-identical to 9e2c01bc's after an unfiltered test run; (3) docs/standards.md's module map, docs/security.md and AGENTS.md name the new paths, not the old (grep for room.rs, linked.rs, session/ledger.rs, wire/room_handle.rs, mcp/oauth.rs, mcp/tool.rs under crates/hotline-core in docs/ and AGENTS.md); (4) make check and make precheck pass. For epic-17 (voice) to start on top: its task-202 names crates/hotline-core/src/room.rs and task-207 names crates/hotline-core/src/voice/spoken.rs, both moved by task-208; make sure ADR 0007's old→new path list covers every file epic-17's children name that moved. If any of (1)-(4) fails, do not fix code here: say what failed in a note on this task.

## Log
- 2026-10-19T15:05:08.482Z: [claude] Lead: task-208 took no timings, so measure all three points yourself: 9e2c01bc, the task-208 merge on release/0.41.0, and the task-209 merge. task-208 also moved voice/settings.rs and synthesis's sentence helpers into hotline-room (beyond the plan), and left the core reaching every moved piece through `pub use` re-exports; the epic check should list which of those re-exports remain and why.
- 2026-10-19T15:35:42.490Z: [claude] Lead: the three commits are 9e2c01bc (before the epic), 66c2aa54 (merge of task-208) and a40e7745 (merge of task-209); task-211 landed between them at 8188b43d and counts with the room step. task-209's cold test build (74.5 s) was taken under load 7-13: re-take every row with nothing else running (check `uptime` before each run and give the load in the ADR), and run the three commits one after another, not in parallel. task-209 left the core reaching the client and normalize_servers through re-exports at crate::mcp (callers in import.rs, wire settings, vault/launch.rs). That is allowed while callers remain: say so in the ADR as what's left, don't change code. Run make check and make precheck in the foreground until they finish before you end your turn, then commit.
