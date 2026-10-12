---
id: task-36
title: Subagent and runner runs open as their work, not as a chat
column: todo
position: 38
tags:
  - feature
  - bro-270
createdAt: "2026-10-11T21:05:59.835Z"
description: |-
  From George, 2026-10-09.

  **Today:** opening a subagent or runner (from the composer chip, or the run line in the conversation) shows it in the right pane as a chat. The task shows as a green "message" bubble, and replies, thinking and tools show as a conversation. An operator opening a run wants to see what it is doing, not how it is talking.

  **Want:** runs open as a "show the work" view.

  * **Header:** the run's title, its state (working / done in 3 min / failed), how long it has run, and Stop.
  * **The task:** what it was asked to do, shown once at the top as a brief, not as a chat bubble.
  * **The work:** its steps as a log, like the Work card's steps but full height: tools with their titles and results (expandable), files touched, and commands with their output. Thinking is collapsed by default.
  * **The result:** what it reported back, at the end, clearly marked as its result.
  * **Live:** steps stream in as it works, and the view follows the latest unless the operator has scrolled up.

  **Keep the chat view for real threads:** DMs between agents, side chats, and anything a person or another agent talks in.

  **Notes:**

  * Applies to ACP subagents (Claude Code / Codex subagent sessions) and Hotline Agent runners and subagents alike. Both already arrive as runs (`kind: "run"` threads) with the subagent's messages and tool calls.
  * Decide what the composer does on a run (today a run has no composer, or a read-only one). Proposal: none. Steer the run through the agent that started it.
  * Check the phone too: it opens runs as threads. A matching compact work view there could be a follow-up.


  The separate completed → resumed ACP lifecycle bug is now task-51; this task owns only the work-view presentation. Audit 2026-10-11: ui/src/components/Dock.tsx still opens runs through Transcript. Keep live step following, collapsed thinking, brief/result sections and Stop; preserve chat presentation for real conversations. A phone work view belongs on the hotline-mobile board.

  Migration source: BRO-270.
updatedAt: "2026-10-12T04:21:41.439Z"
priority: medium
---
