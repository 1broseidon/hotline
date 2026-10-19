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
---

## Description
George, 2026-10-19 09:07 (call): split by area of concern for maintainability; start with the room and MCP now, voice later when he's back at the desk. Follows ADR 0007's "Next: splitting the core" (docs/adr/0007-split-hotline-core.md on release/0.41.0), steps 1 and 2. Voice (step 3) and remote (step 4) are not part of this epic.

Same rules as epic-16: moves only, no behaviour change, no single-implementation trait to break the session/driver, session/voice or mcp/session cycles (docs/standards.md); contract.ts byte-identical; docs/security.md citations, docs/standards.md's module map and AGENTS.md follow the moves; each new crate gets the workspace lints, a license and a crate doc saying what it owns and must never depend on; make check and make precheck pass before landing. A module that leaves the core renames its own crate:: paths to the new crates; the core keeps re-exports only while callers remain.

Done when: the room crate and the MCP client crate are out, ADR 0007 has the timings after each, and epic-17 (voice) can start on top without a conflict in the moved files.
