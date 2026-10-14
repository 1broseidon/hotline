---
id: task-4
title: Write a user guide for getting started and using Hotline
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
updatedAt: "2026-10-14T22:06:45.198Z"
assignee: claude
completedAt: "2026-10-14T22:06:45.198Z"
---

## Log
- 2026-10-13T06:32:19.609Z: [claude] On branch dispatch/task-4 (2 commits). Added a "Use Hotline" section to docs-site with six pages: common workflows, working in a conversation (incl. chapters), side threads & subagents, teammates together (asks, handoffs, grants), calls & dictation, and pictures. Audited and rewrote get-started, setup and reference pages against the 0.36.1 UI and code: welcome flow, Thinks with/Folders/Abilities, pane layout, Use for + Budgets, keychain storage, skills, web search, full room tool list, schedules in the pane, computer image/limits/cookies/secrets/passkeys, request_human not blocking (a card expires after 24h), shortcuts. Providers page carries the Anthropic API-key / Claude Code sign-in guidance; recheck Anthropic policy before publishing. Build passes and every internal link and anchor resolves. Not done: screenshots, phone/ and server/ pages (out of scope). Code-side mismatches for a follow-up: budget error text says "Settings › Budgets" (Budgets is under Providers); the web search hint "Tried in order" contradicts the code, which queries every provider at once; the web_search refusal mentions a per-teammate switch the UI doesn't have; the changelog says both 2 and 3 side threads (code: 3).
- 2026-10-14T20:54:00.138Z: [claude] 2026-10-14 merge-readiness: merges clean, site builds; ~25 stale/wrong claims vs 0.36.3 main (Muse Code missing, Apple container hardening, keychain claim, tools list, labels). Correction pass then PR.
