---
id: epic-18
title: "Split hotline-core, stage 2: the room and its plain pieces, then the MCP client (0.41)"
type: epic
column: todo
position: 8
priority: high
tags:
  - crate-split
  - rust
  - "0.41"
assignee: claude
createdAt: "2026-10-19T14:07:40.639Z"
landsOn: release/0.41.0
updatedAt: "2026-10-19T14:08:07.198Z"
dispatch:
  agent: claude
  model: opus
  effort: high
  by: cli
  at: "2026-10-19T14:08:05.029Z"
  until: done
  role: lead
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-19T14:08:07.198Z"
---

## Description
George, 2026-10-19 09:07 (call): split by area of concern for maintainability; start with the room and MCP now, voice later when he's back at the desk. Follows ADR 0007's "Next: splitting the core" (docs/adr/0007-split-hotline-core.md on release/0.41.0), steps 1 and 2. Voice (step 3) and remote (step 4) are not part of this epic.

Same rules as epic-16: moves only, no behaviour change, no single-implementation trait to break the session/driver, session/voice or mcp/session cycles (docs/standards.md); contract.ts byte-identical; docs/security.md citations, docs/standards.md's module map and AGENTS.md follow the moves; each new crate gets the workspace lints, a license and a crate doc saying what it owns and must never depend on; make check and make precheck pass before landing. A module that leaves the core renames its own crate:: paths to the new crates; the core keeps re-exports only while callers remain.

Done when: the room crate and the MCP client crate are out, ADR 0007 has the timings after each, and epic-17 (voice) can start on top without a conflict in the moved files.

## Log
- 2026-10-19T14:10:27.505Z: [claude] Plan (lead, 2026-10-19): strictly serial, because each stage touches the same lines (workspace members, Cargo.lock, core Cargo.toml/lib.rs, standards.md module map, AGENTS.md, ADR 0007). Order: task-211 → task-208 → task-209 → task-210. task-211 (new) splits out of task-208 the one real refactor: RoomHandle's voice()/remote()/links() leave the trait and AuthStatus/import::Report become plain data, done in place in the core so it is reviewed apart from the mechanical move. task-208 then makes the room crate and moves the plain pieces. task-209 moves the MCP client and needs task-208's ledger and normalize_servers. task-210 is the last child: timings at 9e2c01bc, the 208 merge and the 209 merge, and a check of the whole epic against "done when" (crates out, neither depends on core, contract.ts byte-identical to 9e2c01bc, docs name new paths, make check and make precheck). Every child's validation is make check, contract.ts diff and make precheck, plus file-absence checks. Overlap with epic-17: task-202 names room.rs and task-207 names voice/spoken.rs, and task-208 moves both. Their leads should start after task-208 lands and use ADR 0007's old→new path list (208 and 209 must write it). I can't patch epic-17. All children: claude/opus.
