---
id: task-32
title: "Decisions: notify loudly only when a report needs the person"
column: backlog
position: 3
tags:
  - improvement
  - bro-186
parentId: epic-2
createdAt: "2026-10-11T21:05:56.483Z"
description: |-
  Today every finished turn sends one push with `sound: "default"` (`push.rs` `notify`, via `send_glance` in `session/mod.rs`). The only filter is `at_the_desk`: nothing for 120 s after the desk says the person is using it.

  With a decisions provider connected, ask one Choice about the final report: **needs you now** (blocked, a question, an error the teammate can't get past) / **done** (finished something the person asked for) / **for your information**. Needs-you and done stay loud; for-your-information becomes a silent push. Cards waiting on the person (permission, human_action, passkey) are never downgraded.

  * Changes loudness only, never whether a push exists. If the decision fails or times out, it's loud, as today.
  * Short state (the report's glance plus the person's last request), so Tev1 on local Ollama is enough.
  * Before shipping: label a sample of real final reports and measure false-quiet (a needs-you report sent silently). That's the error that matters.

  Part of BRO-78's decisions capability.

  Migrated from Linear BRO-186: https://linear.app/1broseidon/issue/BRO-186/decisions-notify-loudly-only-when-a-report-needs-the-person (2026-10-11).
updatedAt: "2026-10-12T20:52:36.427Z"
---

## Log
- 2026-10-12T01:51:06.097Z: [claude] Won't do (2026-10-11 sort): child of the parked Jev research (epic-2).
