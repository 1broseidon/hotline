---
id: task-148
title: "P2-39 (D): Lint autofix slice"
column: in-progress
position: 37
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:41:00.635Z"
relatedFiles:
  - crates/hotline-core/src/
  - crates/hotline-core/tests/
  - crates/hotline-app/src/
  - crates/hotline-cli/src/
  - crates/hotline-cli/tests/
updatedAt: "2026-10-18T07:18:24.603Z"
contract:
  status: in_progress
  validation:
    commands:
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T07:18:24.603Z"
    reworkCount: 0
dependsOn:
  - task-129
  - task-131
  - task-132
  - task-133
  - task-136
  - task-138
  - task-139
  - task-140
  - task-141
  - task-143
  - task-144
  - task-145
  - task-147
  - task-152
  - task-153
  - task-155
  - task-175
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:08.731Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T07:18:10.748Z"
assignee: claude
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 39 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Lint autofix slice.** The fifteen "[1]" lints from lints.md, one commit per lint, followed by an unfiltered `cargo test -p hotline-core`. Prove each slice clean with a one-off `cargo clippy -- -W clippy::<lint>`; the table comes later (C4)

- Source: lints, task 2
- Files: about 700 sites workspace-wide
- Collides with: every split in C: land after 33
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T08:16:17.943Z: [claude] Done on dispatch/task-148 (2f896d04 + a follow-up, 16 commits): one commit per "[1]" lint, about 600 sites. Each lint is proven clean with `cargo clippy --workspace --all-targets -- -W clippy::<lint>`, and all fifteen together come back with zero hits. The unfiltered `cargo test -p hotline-core` passes, contract.ts is unchanged, and make check and make precheck (8/8) pass. No [lints] table was added; that is row 52. Choices a reviewer should know about: (1) assert_is_empty: where the empty literal can't be inferred, it is `assert_eq!(x, [] as [T; 0])`, with the short type name where it's in scope. pair.rs compares to `json!([])`. (2) format_push_string: `let _ = write!(s, …)` (writeln! where it ends in \n), with `use std::fmt::Write` (`as _` in avatar.rs, which imports io::Write). Writing to a String cannot fail. (3) default_trait_access names each type and adds the imports it needs; session_new.rs uses rmcp::model::ClientCapabilities, because acp's own type shares that name. (4) The last commit adds two semicolons that the let-else rewrite left bare. For row 52: I linted on macOS only. Code compiled only on Linux or Windows (#[cfg(target_os)] bodies) was not checked against these lints. Run clippy -W for each on those targets before setting them to warn under -D warnings. Nothing left unwired.
