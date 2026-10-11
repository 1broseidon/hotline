---
id: task-4
title: Write a user guide for getting started and using Hotline
column: todo
position: 6
priority: medium
tags:
  - bro-18
createdAt: "2026-10-11T21:05:16.192Z"
description: |-
  A guide already exists: `docs-site/` is an Astro site (not the VitePress originally proposed) with twelve pages — install, room, first teammate, providers, teammates, tools, schedules, computer, updates, data, import, shortcuts. What remains is an audit of that copy against the current desk (0.25) and the missing pieces: a common-workflows page, a check of the chapters/context coverage (touched in get-started/room.md and reference/data.md), and whatever else the running app shows that the pages do not.

  Organize around user goals, explain terms plainly, include tested walkthroughs and screenshots where useful, and distinguish current behavior from future plans. Audit against the running app and docs/design.md, docs/sessions.md and docs/wire.md so examples match shipped behavior.

  Authentication guidance to carry into the providers page, from official-policy research on 2026-09-09:

  * Keep native Anthropic connections on API keys. Claude Code owns its own subscription sign-in: [code.claude.com/docs/en/legal-and-compliance](<http://code.claude.com/docs/en/legal-and-compliance>) prohibits collecting or intermediating Claude session tokens or offering Claude login inside another app, while permitting the unmodified Claude Code binary with end-user authentication under stated conditions.
  * Toad ACP uses claude-agent-acp, which uses the Agent SDK and the Claude Code executable. A planned billing change for third-party SDK use was paused in June 2026 with no replacement date; do not promise third-party usage is credit-only.
  * Recheck these evolving policies before publishing.

  Deliver updated Markdown content in the existing Astro structure; publishing the site is a later step.

  ## Migration source

  Brainfile: toad-next/task-62 · Source file: `.brainfile/board/task-62.md` on the Linux desk (gitignored, local only)

  Migrated from Linear BRO-18: https://linear.app/1broseidon/issue/BRO-18/write-a-user-guide-for-getting-started-and-using-hotline (2026-10-11).

  ## Status (2026-10-11 review)

  docs-site has 26 pages incl. Server and Phone (0.26.0). Missing: calls/voice, images, web_search, threads/handoffs, side threads, a workflows page.
updatedAt: "2026-10-11T21:05:16.482Z"
---
