---
id: task-113
title: "P2-1 (A): security.md proves what it cites"
column: todo
position: 2
priority: high
tags:
  - phase2
  - security
parentId: epic-13
createdAt: "2026-10-17T22:40:42.496Z"
updatedAt: "2026-10-17T22:48:38.570Z"
relatedFiles:
  - docs/security.md
  - crates/hotline-core/src/session/tests.rs
  - crates/hotline-core/src/voice/mod.rs
contract:
  status: ready
  validation:
    commands:
      - "! grep -nE 'chatgpt_images_status_requires_owner_selection_without_checking_entitlement|a_computer_from_before_secrets_is_named_only_when_something_was_granted|only_the_reply_to_a_voice_handoff_gets_a_summarised_push|teammate_discovery_never_derives_public_fields_from_private_instructions' docs/security.md"
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
dependsOn:
  - task-112
---

## Description
Phase 2 group A (Security), plan row 1 in docs/audits/0.40/README.md on release/0.40.0. Read that row, the "Task 1: the four missing proofs" table under it, its source report section and the Standards before starting.

**security.md proves what it cites.** Re-point and re-word the rows at `:330`, `:703` and `:1426` per the verdicts table, and write the missing proof for `:1426`: a reply long enough to summarise is rewritten only when it came from a voice handoff, and a typed one with the same text is pushed as written (through the real path in `voice/mod.rs`, `!from_voice`).

`:683` is decided (George, 2026-10-17): the goal stays visible. task-112 lands first and re-words that row with the new colleague fields; check it reads right and cites tests that exist, and do not redo it.

- Source: tests-and-docs T3, task 1
- Files: `docs/security.md:330`, `:703`, `:1426`; a new test in `crates/hotline-core/src/session/tests.rs`
- Collides with: 23 (`session/tests.rs` split, task-132) lands after this

End with `make precheck` and `cargo test -p hotline-core` unfiltered (the hooks are on; never --no-verify). Follow the epic's rules.
