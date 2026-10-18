---
id: task-180
title: "A 'needs you' card a teammate no longer needs goes away: withdraw your own request, and a shorter default lifetime"
column: todo
position: 0
priority: high
tags:
  - cards
  - request_human
  - phone
  - bug
createdAt: "2026-10-18T21:59:36.514Z"
landsOn: release/0.40.0
updatedAt: "2026-10-18T23:54:18.613Z"
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
  status: ready
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
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-18T23:54:14.920Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59290
    at: "2026-10-18T23:54:18.613Z"
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
