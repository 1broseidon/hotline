---
id: task-113
title: "P2-1 (A): security.md proves what it cites"
column: review
position: 2
priority: high
tags:
  - phase2
  - security
parentId: epic-13
createdAt: "2026-10-17T22:40:42.496Z"
updatedAt: "2026-10-18T00:29:40.497Z"
relatedFiles:
  - docs/security.md
  - crates/hotline-core/src/session/tests.rs
  - crates/hotline-core/src/voice/mod.rs
contract:
  status: delivered
  validation:
    commands:
      - "! grep -nE 'chatgpt_images_status_requires_owner_selection_without_checking_entitlement|a_computer_from_before_secrets_is_named_only_when_something_was_granted|only_the_reply_to_a_voice_handoff_gets_a_summarised_push|teammate_discovery_never_derives_public_fields_from_private_instructions' docs/security.md"
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T00:16:08.203Z"
    reworkCount: 0
    deliveredAt: "2026-10-18T00:27:43.945Z"
    duration: 696
dependsOn:
  - task-112
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:14:34.954Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-18T00:14:36.590Z"
assignee: claude
---

## Description
Phase 2 group A (Security), plan row 1 in docs/audits/0.40/README.md on release/0.40.0. Read that row, the "Task 1: the four missing proofs" table under it, its source report section and the Standards before starting.

**security.md proves what it cites.** Re-point and re-word the rows at `:330`, `:703` and `:1426` per the verdicts table, and write the missing proof for `:1426`: a reply long enough to summarise is rewritten only when it came from a voice handoff, and a typed one with the same text is pushed as written (through the real path in `voice/mod.rs`, `!from_voice`).

`:683` is decided (George, 2026-10-17): the goal stays visible. task-112 lands first and re-words that row with the new colleague fields; check it reads right and cites tests that exist, and do not redo it.

- Source: tests-and-docs T3, task 1
- Files: `docs/security.md:330`, `:703`, `:1426`; a new test in `crates/hotline-core/src/session/tests.rs`
- Collides with: 23 (`session/tests.rs` split, task-132) lands after this

End with `make precheck` and `cargo test -p hotline-core` unfiltered (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T00:27:43.608Z: [claude] Done on branch dispatch/task-113 (41ecbc4e). security.md :330 now cites chatgpt_images_status_is_automatic_without_checking_entitlement (automatic once signed in, companion denied); :703 row says a release from before secrets is told apart and the conversation says nothing, citing a_computer_from_before_secrets_leaves_the_conversation_alone; voice section (:1450) states and cites the new session::tests::a_long_reply_is_summarised_for_its_push_only_when_a_voice_handoff_asked_for_it (typed/voice/typed with a three-sentence reply: only the voice push is narrated, narrations==1; mutation of `!from_voice` fails it) plus short_voice_handoff_replies_keep_their_text_without_an_extra_model. :684 colleague row from task-112 checked: reads right, all seven cited tests exist; not redone. make check, cargo test -p hotline-core unfiltered and make precheck (7/7) pass; contract grep clean. For task-132 (session/tests.rs split): the new test sits just above a_direct_call_speaks_committed_acknowledgements_and_hangup_keeps_work_running, beside its short-reply twin; move them together. Nothing left unwired.
- 2026-10-18T00:29:40.497Z: Landed on release/0.40.0
