---
id: epic-14
title: "0.40 easy wins: the small fixes from the backlog triage"
type: epic
column: todo
position: 1
priority: high
tags:
  - "0.40"
  - easy-wins
  - triage
createdAt: "2026-10-18T23:40:44.455Z"
landsOn: release/0.40.0
updatedAt: "2026-10-18T23:51:16.037Z"
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T23:51:14.029Z"
  until: done
  role: lead
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59290
    at: "2026-10-18T23:51:16.037Z"
assignee: claude
---

## Description
The next wave for 0.40: small, low-risk fixes found by the 2026-10-18 backlog triage (four reviewers checked all 44 backlog items against the post-restructure code at release/0.40.0 78255a4f). Each child is one slice. Where a backlog item was only partly in scope, the slice is a new task, and the original keeps the rest and stays deferred.

Rules: each child is one landable change behind `make precheck` (the hooks are on), no new feature surface, and follows docs/standards.md. QA at the end on the Linux QA desktop with George's xAI sign-in, as for the 2026-10-18 pass. All land on release/0.40.0.

## Log
- 2026-10-18T23:54:10.198Z: [claude] Lead plan (claude/opus, 2026-10-18). Every child has relatedFiles and a ready contract. Validation is fmt + clippy -D warnings + an unfiltered `cargo test -p hotline-core` + `git diff --exit-code ui/src/generated/contract.ts`, which proves the committed contract is whole (AGENTS.md trap 3), plus `make ui-check` for window work and the named harness where one applies. ORDER: chain A, the failure notices, runs one at a time because all three reword driver/failure.rs: task-8 → task-188 → task-181. Chain B, the wire: task-187 and task-183 → task-55, because task-55 changes wire/socket.rs (same as task-187) and the remote seats (near task-183). Chain C, the generated contract: task-180 → task-185, since both add contract types and conflict in contract.ts. Independent, run as slots free up: task-182 (oauth page), task-184 (Threads.tsx), task-186 (hotline-app desks.rs), task-37 (voice ui), task-49 (Transcript), task-89 (probe first, see its note). Last: new task-189 depends on all 14, runs make check/verify/precheck on release/0.40.0 with everything in, fixes only where slices meet, checks docs/, and writes George's Linux QA checklist here. First wave: task-8, task-180, task-187 (each heads a chain). Agent choice: claude opus for core and wire work, claude sonnet for the small window-only slices (184, 49, 37).
- 2026-10-19T00:27:28.336Z: [claude] Lead: task-188 left two follow-ups for task-97, which is outside this epic, so I can't edit it from here. Whoever triages next should copy them across. (1) Connecting a provider doesn't restart sessions that are waiting for one; only the next message does, so the band stays 'error' until then. (2) The composer's no-provider refusal is text only, with no Settings link.
- 2026-10-19T00:37:52.688Z: [claude] Lead: task-55 left two follow-ups for task-20, which is outside this epic, so whoever triages next should copy them across. (1) A command answered inline that hangs still stops the 90 s silence timer from running, because the timer isn't selected while an inline command is being answered. (2) A stalled seat only has its file descriptor dropped; nothing resets the connection (no SO_LINGER reset).
