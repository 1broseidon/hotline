---
id: task-34
title: "Decisions: choose what a chapter note is written from"
column: backlog
position: 5
tags:
  - improvement
  - bro-188
parentId: epic-2
createdAt: "2026-10-11T21:05:58.160Z"
description: |-
  Today the chapter note is written from the first 8,000 and last 48,000 characters of the transcript (`chapters.rs` `HEAD_CHARS`/`TAIL_CHARS`), so constraints, corrections and decisions in the middle are lost. A teammate feels this as forgetting what was agreed.

  Rank transcript blocks with a decisions model and fill the same budget with the ones that matter. BRO-78's experiment kept 37/37 synthetic important facts against 28/37 for head/tail and 30/37 for a cheap control. Real chapters are still unlabelled.

  * Needs Jev (32k context). Tev1's \~2k context is too small for this.
  * Invisible, with no setting: if no decisions provider is connected or the call fails, it falls back to head/tail.
  * Next step is BRO-78's continuation plan: blind-label the five real chapters, then compare paired handoff notes.

  Part of BRO-78's decisions capability.

  Migrated from Linear BRO-188: https://linear.app/1broseidon/issue/BRO-188/decisions-choose-what-a-chapter-note-is-written-from (2026-10-11).
updatedAt: "2026-10-12T20:52:36.431Z"
---

## Log
- 2026-10-12T01:51:07.243Z: [claude] Won't do (2026-10-11 sort): child of the parked Jev research (epic-2).
