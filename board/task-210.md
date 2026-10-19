---
id: task-210
title: Timings after stage 2, in ADR 0007
column: in-progress
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
updatedAt: "2026-10-19T16:53:17.902Z"
contract:
  status: in_progress
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
    pickedUpAt: "2026-10-19T16:53:17.902Z"
    reworkCount: 1
    deliveredAt: "2026-10-19T16:52:43.361Z"
    duration: 4607
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T15:35:43.429Z"
  until: done
  tries: 2
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-19T16:53:01.511Z"
---

## Description
The last child: time stage 2 and check the epic whole against its "done when". Docs only.

Timings: re-run ADR 0007's commands at three commits on this machine, under the same conditions, each in its own worktree: the release/0.41.0 commit before the epic (9e2c01bc, to re-baseline), the merge of task-208 (the room crate), and the merge of task-209 (the MCP client). Add the rows beside the earlier ones: cold and incremental build and test build, the core's front-end time, and what got faster and what didn't. Say whether the numbers support going on to the voice engines (step 3), as the ADR's stopping rule asks.

The whole: on the tip of release/0.41.0, confirm and state in the ADR that (1) hotline-room and hotline-mcp are workspace crates and neither depends on hotline-core; (2) contract.ts is byte-identical to 9e2c01bc's after an unfiltered test run; (3) docs/standards.md's module map, docs/security.md and AGENTS.md name the new paths, not the old (grep for room.rs, linked.rs, session/ledger.rs, wire/room_handle.rs, mcp/oauth.rs, mcp/tool.rs under crates/hotline-core in docs/ and AGENTS.md); (4) make check and make precheck pass. For epic-17 (voice) to start on top: its task-202 names crates/hotline-core/src/room.rs and task-207 names crates/hotline-core/src/voice/spoken.rs, both moved by task-208; make sure ADR 0007's old→new path list covers every file epic-17's children name that moved. If any of (1)-(4) fails, do not fix code here: say what failed in a note on this task.

## Log
- 2026-10-19T15:05:08.482Z: [claude] Lead: task-208 took no timings, so measure all three points yourself: 9e2c01bc, the task-208 merge on release/0.41.0, and the task-209 merge. task-208 also moved voice/settings.rs and synthesis's sentence helpers into hotline-room (beyond the plan), and left the core reaching every moved piece through `pub use` re-exports; the epic check should list which of those re-exports remain and why.
- 2026-10-19T15:35:42.490Z: [claude] Lead: the three commits are 9e2c01bc (before the epic), 66c2aa54 (merge of task-208) and a40e7745 (merge of task-209); task-211 landed between them at 8188b43d and counts with the room step. task-209's cold test build (74.5 s) was taken under load 7-13: re-take every row with nothing else running (check `uptime` before each run and give the load in the ADR), and run the three commits one after another, not in parallel. task-209 left the core reaching the client and normalize_servers through re-exports at crate::mcp (callers in import.rs, wire settings, vault/launch.rs). That is allowed while callers remain: say so in the ADR as what's left, don't change code. Run make check and make precheck in the foreground until they finish before you end your turn, then commit.
- 2026-10-19T16:52:51.780Z: [claude] Done on dispatch/task-210 (f6fdd7ff, docs/adr/0007 only). Timings: 9e2c01bc, 66c2aa54, a40e7745, each in its own worktree, nothing run in parallel. The machine was never quiet: with no build running, load was 4-7 from processes that aren't Hotline's (VM, video decoder, Android emulator, Parsec), and they were left running. So: one sequential pass, then 3 interleaved rounds of the cold builds and dispatcher touches, then each other incremental row with the three commits back to back. Round 1 and the sequential passes at 66c2aa54/a40e7745 fell in a slowdown that hit every commit (baseline 48 s vs 32 s) and are left out, said so in the ADR. Result: core lib 30.8→29.6 s, front end 19.4→18.6 s (4% for 6.5% of the lines); cold build 32.2→31.1 s; cold test build 60.8 vs 62.0 s (within the 60.8-67.5 spread; task-209's 74.5 s was load); floors 3.2/8.5→3.1/8.4 s. An edit to room.rs now builds 3.4→4.0 s (crate below the core, by design). Room's tests alone 0.7 s, mcp's 0.9 s. Stopping-rule verdict: steps 1+2 gave 0.1/0.1/0.8 s against expected 0.2/0.6/1.3 s, so the numbers do NOT support starting step 3 for build time; the ADR says it waits on George (a case for boundaries, not speed). Epic check (1)-(4) all hold at a40e7745: make check (2,064 tests) and make precheck (8 legs) pass, contract.ts identical to 9e2c01bc. One contract validation line fails as written: `! grep -rnE --exclude-dir=adr ... docs AGENTS.md` hits docs/audits/0.40/{errors,lints,structure,tests-and-docs}.md, dated 0.40 audits citing that tree with line numbers (26 epic-16 old paths are there too). I left them as written; the check passes with `--exclude-dir=audits` added. Contract delivered, not validated, for that reason. epic-17: only task-202's room.rs and task-207's voice/spoken.rs moved, both in Step 1's table; dispatcher/duplex/front_prompt/front_tools/session.rs stayed in core. The ADR lists the 13 remaining re-exports with their caller counts (crate::room 57 files ... mcp::normalize_servers 3: import.rs, wire settings, vault/launch.rs); none removed, since callers remain.
- 2026-10-19T16:53:00.998Z: Landing sent back (1 of 1): with release/0.41.0 brought in, `! grep -rnE --exclude-dir=adr 'crates/hotline-core/src/(room|linked|session/ledger|wire/room_handle|mcp/oauth|mcp/tool)\.rs' docs AGENTS.md` failed: messages | ⏎ docs/audits/0.40/errors.md:264:| Normalized MCP state: `crates/hotline-core/src/mcp/mod.rs:246`, `crates/hotline-core/src/mcp/mod.rs:787`, `crates/hotline-core/src/mcp/oauth.rs:284`, `crates/hotline-core/src/driver/acp.rs:2024` | Normalization/object checks and connected-proxy filtering precede access; proxy entries are not concurrently removed on this path | ⏎ docs/audits/0.40/errors.md:288:`crates/hotline-core/src/room.rs:52` always seeds that key and spending defaults. ⏎ docs/audits/0.40/errors.md:293:(`crates/hotline-core/src/mcp/tool.rs:91`), ASCII-only generated image stems. Fix it on this branch, commit, and run the checks again.
