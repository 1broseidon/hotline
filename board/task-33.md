---
id: task-33
title: "Decisions: answer routine permission prompts (allow once, never deny)"
column: todo
position: 35
tags:
  - improvement
  - bro-187
parentId: epic-2
createdAt: "2026-10-11T21:05:57.321Z"
description: |-
  Today a harness permission prompt reaches the phone exactly as the harness sent it (`driver/acp.rs` `ask_permission`). Hotline has no auto-approve of its own, and an unanswered card is cancelled after 10 min.

  With a decisions provider connected and the teammate opted in, ask two questions with the person's request plus the tool call as state:

  * **in scope:** is this call something the person's request needs?
  * **reversible:** can it be undone without loss? Score: read-only / local and reversible / irreversible or external.

  When both are confidently safe, Hotline picks the harness's own **allow once** option and posts a visible line: what it allowed and why. Otherwise the card goes out as today. Hotline never auto-denies and never picks an "always" option.

  * Opt-in per teammate. This is the one use that needs a setting.
  * Deterministic grants and the harness's own policy stay authoritative. This only answers what would otherwise be a card.
  * Before shipping: a replay set of real permission prompts with labels; thresholds chosen for near-zero false-allow on the irreversible ones. Reference: OpenRouter's "Auto-Approve Coding Agent Permission Prompts with Jev" recipe.

  Part of BRO-78's decisions capability.

  Migrated from Linear BRO-187: https://linear.app/1broseidon/issue/BRO-187/decisions-answer-routine-permission-prompts-allow-once-never-deny (2026-10-11).
updatedAt: "2026-10-11T21:05:57.615Z"
---
