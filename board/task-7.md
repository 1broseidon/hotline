---
id: task-7
title: Run quiet scheduled jobs on their own context, and bring back only the note
column: backlog
position: 40
priority: medium
tags:
  - improvement
  - bro-101
createdAt: "2026-10-11T21:05:18.681Z"
description: |-
  ## Why

  A quiet scheduled run goes through the teammate's chat session. Its tool calls and results, screenshots included, stay in the chat history and are resent with every later message. Each firing is also written as a user event, which counts as chapter activity (`store/chapters.rs`, `is_message`), so an hourly loop keeps the chapter open indefinitely.

  The result: background work fills the context and forces a chapter handoff in the middle of the person's own conversation.

  ## What

  Run quiet jobs on the runner from BRO-94 with a fresh context, the way subagents run. Only the escalation note from BRO-96 comes back, as the prompt the teammate answers in the open. A firing that finds nothing leaves nothing in the chat's context.

  ## Open questions

  * The job no longer sees the conversation, so its prompt has to stand on its own. Should the schedule tool say so, or should the run get the chapter note?
  * Should a quiet firing still count as activity for the chapter idle timer?

  ## Done when

  * A quiet firing that finds nothing adds nothing to the chat's history.
  * One that finds something still ends in the teammate's open reply, as today.

  Found by checking the rig driver against Cursor's harness checklist (subagents keep the parent lean and return short handoffs).

  Migrated from Linear BRO-101: https://linear.app/1broseidon/issue/BRO-101/run-quiet-scheduled-jobs-on-their-own-context-and-bring-back-only-the (2026-10-11).
updatedAt: "2026-10-14T20:05:32.628Z"
---
