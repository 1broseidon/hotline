---
id: task-180
title: "A 'needs you' card a teammate no longer needs goes away: withdraw your own request, and a shorter default lifetime"
column: review
position: 2
priority: high
tags:
  - cards
  - request_human
  - phone
  - bug
createdAt: "2026-10-18T21:59:36.514Z"
landsOn: release/0.40.0
updatedAt: "2026-10-19T00:14:22.381Z"
parentId: epic-14
relatedFiles:
  - crates/hotline-core/src/session.rs
  - crates/hotline-core/src/session/asking.rs
  - crates/hotline-core/src/mcp/server.rs
  - crates/hotline-core/src/contract/command.rs
  - crates/hotline-core/src/contract/transcript.rs
  - crates/hotline-core/skills/hotline-room/SKILL.md
  - crates/hotline-core/src/session/tests/cards.rs
  - ui/src/generated/contract.ts
contract:
  status: delivered
  deliverables:
    - type: test
      path: crates/hotline-core/tests
      description: "over the wire: a withdrawn card is gone for desk and phone seats; a card past its lifetime expires and tells the teammate once; a longer wait is honoured up to the cap"
  validation:
    commands:
      - cargo fmt --all --check
      - cargo clippy --workspace --all-targets -- -D warnings
      - cargo test -p hotline-core
      - cargo test -p hotline-core --test desk
      - git diff --exit-code ui/src/generated/contract.ts
      - make ui-check
  constraints:
    - A teammate may withdraw only its own card
    - A tool change follows docs/security.md
  metrics:
    readyAt: "2026-10-18T23:53:55.554Z"
    pickedUpAt: "2026-10-19T00:11:28.216Z"
    reworkCount: 1
    deliveredAt: "2026-10-19T00:14:12.170Z"
    duration: 164
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-18T23:54:14.920Z"
  until: done
  tries: 2
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59290
    at: "2026-10-19T00:10:43.870Z"
    ended: "2026-10-19T00:14:22.381Z"
    outcome: review
assignee: claude
---

## Description
Reported by George 2026-10-18 from the phone. Pork Chop (a Claude Code teammate) asked twice with request_human, once for the xAI sign-in and once for ChatGPT, on its computer during 0.40 QA. It moved on: the ChatGPT flow was abandoned when the QA app restarted. Both cards kept the conversation reading 'needs you' on the phone for 1.5 hours, until George answered them.

Why: request_human from an ACP teammate returns at once and the card stays up for ASK_TTL = 24 h (crates/hotline-core/src/session.rs). Hotline Agent's waiting call uses HUMAN_DEADLINE = 10 min. A teammate has no way to take back a card it no longer needs.

Change:
1. A teammate can withdraw its own pending request_human card, e.g. a withdraw_request(id) tool, or request_human returning an id that cancel_request takes. The card is superseded as 'withdrawn' (not 'answered'), the phone and desktop stop showing 'needs you', and nothing is sent back to the teammate.
2. The default lifetime of a delivered card drops to about 2 h. request_human gains an optional longer wait (capped, e.g. 24 h) for an ask that really waits for the person to come back. Expiry tells the teammate once, as today.
3. The tool description tells teammates to withdraw a request they no longer need.

Phone: check that hotline-mobile drops 'needs you' when a card is superseded as withdrawn or expired. It should, because it reads the desk's card state; file a phone task only if it doesn't.

Tests: over the wire, a withdrawn card is gone for the desk and phone seats, a card past its lifetime expires and tells the teammate once, and a longer wait is honoured up to the cap.

0.40 triage (2026-10-18): Part of the 0.40 easy wins.

## Log
- 2026-10-19T00:10:32.656Z: [claude] Done on dispatch/task-180 (604cf771). request_human now returns the card's id. New tool withdraw_request{id} supersedes the caller's own pending delivering card as status 'withdrawn' and delivers nothing. Another teammate's id, an answered card or a repeated withdraw are refused. ASK_TTL (24 h) is gone: a card lives ASK_HOURS=2 h, or wait_hours clamped 1–24 (ASK_MAX_HOURS). The card carries expiresAt, so a restart keeps the choice; an old card without it lives 2 h from its ts. Expiry still tells the teammate once, with new wording. Contract: HumanActionStatus gains 'withdrawn' and human_action gains optional expiresAt (contract.ts regenerated, UI label 'Withdrawn', design.md says the importer is unaffected). Handoffs: withdrawing a card raised in a running handoff turn removes its gate, so the result returns at turn end. Once the handoff is WaitingHuman the withdraw is refused. Tests: wire/tests/asks.rs drives the real handler with Desk and Phone seats (withdrawn, expiry-once, wait honoured to the cap), plus session/exchanges/tests.rs for the handoff gate. They are in src/wire/tests, not crates/hotline-core/tests, because the external harness cannot reach a teammate's tool calls. Security: default is every teammate has the tool; grant source is none (it acts only on the caller's own cards); enforced in Room::withdraw_human via human_card(persona_id) under the answering_later lock; revocable through the session lease (TeammateTools::call checks the capability); residual risk is that a teammate can silence its own ask, which is the point. docs/security.md has the row. Phone: hotline-mobile drops 'needs you' on any non-pending status (conversation.ts needsPerson, inbox categoryOf via the roster's waiting), so no phone task was filed. Its card label falls through to 'Expired' for a withdrawn card until it regenerates its contract, which is cosmetic. Checks: fmt, clippy -D warnings, cargo test -p hotline-core (all), --test desk, make ui-check all green; the contract diff is clean after commit.
- 2026-10-19T00:10:43.314Z: Landing sent back (1 of 1): dispatch/task-180 conflicts with release/0.40.0 in crates/hotline-core/src/wire/tests.rs. On release/0.40.0 since this branch started: 002103b6 A global read answers when it is ready, and the writes behind it keep their order without waiting for it. (task-187). Merge release/0.40.0 into this branch (git merge release/0.40.0), keep what both sides meant, commit the merge, and run the checks: cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p hotline-core && cargo test -p hotline-core --test desk && git diff --exit-code ui/src/generated/contract.ts && make ui-check.
- 2026-10-19T00:11:19.354Z: [claude] Lead: the conflict in wire/tests.rs comes from task-187 (landed 002103b6), which added tests for global reads running in the background, and task-8 also landed. Merge release/0.40.0 into your branch and keep both sets of tests. Then run an unfiltered `cargo test -p hotline-core` so contract.ts is written whole, and check `git diff --exit-code ui/src/generated/contract.ts` is clean before you deliver.
- 2026-10-19T00:14:14.220Z: [claude] Merged release/0.40.0 into dispatch/task-180 (a7eee4ab). The one conflict, in wire/tests.rs, kept both sides: task-187's slow-read test, and then `mod asks;` with the other test modules. fmt, clippy -D warnings, cargo test -p hotline-core (1889 passed), --test desk, the contract diff and make ui-check (431 pass) are all green. The work is on its branch, ready to land. For the tasks after this one: request_human now returns a card id, withdraw_request{id} exists, and HumanActionStatus has 'withdrawn'.
