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

  **Also (seen 2026-10-09):** a Claude Code subagent that ends its turn while its own background job (a long test run) is still going is reported as finished, so the chip and run show done. When the job finishes, the subagent wakes and keeps working, but the adapter doesn't announce it again, so Hotline never shows it. The work view should keep tracking a resumed subagent; check whether the adapter sends updates for that child session id after `subagent_state_update: completed`.

  Migrated from Linear BRO-270: https://linear.app/1broseidon/issue/BRO-270/subagent-and-runner-runs-open-as-their-work-not-as-a-chat (2026-10-11).
updatedAt: "2026-10-11T21:06:00.118Z"
---
