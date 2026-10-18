---
id: task-132
title: "P2-23 (C): `session/tests.rs`"
column: in-progress
position: 21
priority: high
tags:
  - phase2
  - restructure
parentId: epic-13
createdAt: "2026-10-17T22:40:52.292Z"
relatedFiles:
  - crates/hotline-core/src/session/tests.rs
  - crates/hotline-core/src/session/tests/
updatedAt: "2026-10-18T05:27:58.989Z"
contract:
  status: in_progress
  validation:
    commands:
      - test $(wc -l < crates/hotline-core/src/session/tests.rs) -le 2000
      - git diff --exit-code $(git merge-base HEAD release/0.40.0) -- ui/src/generated/contract.ts
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T05:27:58.989Z"
    reworkCount: 1
    deliveredAt: "2026-10-18T05:11:55.746Z"
    duration: 1243
dependsOn:
  - task-130
  - task-113
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:50.788Z"
  until: done
  tries: 2
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T05:27:48.463Z"
assignee: claude
---

## Description
Phase 2 group C (Move-only renames and splits), plan row 23 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**`session/tests.rs`** → fixtures plus `session/tests/*.rs`, 131 tests before and after

- Source: structure F6, task 10
- Files: `session/tests.rs`
- Collides with: 1 (new test: land 1 first), 45
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T05:11:55.254Z: [claude] Done on branch dispatch/task-132 (fb6631eb). session/tests.rs (7,247 lines) is now 735 lines of fixtures (Scripted, Fake, DeskKeys and the helpers) plus session/tests/{turns,preamble,chat,subagents,computer,scheduled,chapters,cards,runs,sent_files,voice}.rs, following structure F6's ranges at today's line numbers. There are 132 tests before and after, not 131, because task-113 added one; it sits in voice.rs beside its short-reply twin. The path crate::session::tests is unchanged, so the other modules' imports still work. Helpers shared across topics moved up into the fixtures: saying, heard, spoken, write_tape, until_state, ComputerRoom, computer_room, TWO_RELEASES, notices. computer.rs is gated once with #[cfg(unix)] mod computer; in place of the per-item attributes, so Windows has no unused import. The section-divider comments became //! module docs. docs/security.md now cites the new files (rows :670 :682 :688 :691 :695 :704 :708 :709, :951 and :1501), and the runs:: and sent_files:: prefixes are gone; every cited name was checked against its file. make check and make precheck (7/7, Windows included) pass, and contract.ts is unchanged. For task 45 and the others after this one: new session tests go in the topic file, not tests.rs. Nothing left unwired.
- 2026-10-18T05:27:47.669Z: Landing sent back (1 of 1): with release/0.40.0 brought in, `make check` failed: failures: ⏎ voice::tests::a_call_that_thinks_says_so_again ⏎ test result: FAILED. 1878 passed; 1 failed; 9 ignored; 0 measured; 0 filtered out; finished in 36.91s ⏎ error: test failed, to rerun pass `-p hotline-core --lib` ⏎ make: *** [check] Error 101. Fix it on this branch, commit, and run the checks again.
- 2026-10-18T05:32:00.664Z: [claude] Fixed the landing failure on dispatch/task-132 (55bfcb06). voice::tests::a_call_that_thinks_says_so_again is not touched by this split and passes on its own. It flaked under the full run's load because it asserted the repeated Thinking came within 2×THINKING_AGAIN (200 ms). The test now asserts only that a second Thinking arrives with nothing else changing the call, and event()'s 15 s wait bounds it. make check and make precheck (7/7) pass. session/tests.rs is 766 lines and contract.ts is unchanged. Nothing left unwired.
