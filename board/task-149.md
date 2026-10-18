---
id: task-149
title: "P2-40 (D): By-hand lints and reasoned invariants"
column: review
position: 0
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:41:01.159Z"
relatedFiles:
  - crates/hotline-core/src/
  - crates/hotline-app/src/
  - crates/hotline-cli/src/
updatedAt: "2026-10-18T09:31:17.958Z"
contract:
  status: delivered
  validation:
    commands:
      - "! git grep -nE 'unreachable!\\(\\)' -- 'crates/*/src'"
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T08:43:43.190Z"
    reworkCount: 1
    deliveredAt: "2026-10-18T09:31:17.958Z"
    duration: 2855
dependsOn:
  - task-148
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T08:43:31.268Z"
  until: done
  tries: 2
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 35147
    at: "2026-10-18T08:43:37.552Z"
assignee: claude
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 40 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**By-hand lints and reasoned invariants.** The "[2]" lints and the one- to five-hit pedantic picks, reading `suspicious_operation_groupings`, `missing_fields_in_debug` and `unchecked_time_subtraction` first. Then replace the 26 non-lock `unwrap`s with reasoned `expect`s, and give the empty `unreachable!`s the narrowing that justifies them

- Source: lints, task 3; errors F5, task 6
- Files: the hit sites; the clusters in the errors.md "Leave alone" table
- Collides with: 39
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

Related Files:
  - crates/hotline-core/src/
  - crates/hotline-app/src/
  - crates/hotline-cli/src/

Settle note (Pork Chop, 2026-10-18 03:45): the run stopped as 'done' with crates/hotline-core/src/log.rs half-edited and uncommitted (a nonminimal_bool rewrite to is_none_or, not cargo-fmt'd) and no done note. Finish the row: the remaining [2] lints and the reasoned expect/unreachable work. Commit each through the hooks, run make precheck, then write the done note with what was covered and what was left on purpose.

## Log
- 2026-10-18T08:39:54.251Z: Not landed: 1 file is not committed in the worktree: send it back, or commit in your editor
- 2026-10-18T09:31:17.634Z: [claude] Done on dispatch/task-149 (86773caa..aae47758, one commit per lint or invariant, all through the hooks). make check passes. make precheck passes 8/8 (Windows too). The unfiltered core tests pass and contract.ts is unchanged.

COVERED. [2]: undocumented_unsafe_blocks (every block has a SAFETY: comment, Windows-only ones included; the toast delegate's mem::forget is now Retained::into_raw), case_sensitive_file_extension_comparisons (file names go through Path::extension in any case; non-file suffixes are rewritten or carry an #[expect] with a reason), match_wildcard_for_single_variants. redundant_clone and unchecked_time_subtraction landed in the earlier commits on this branch. Pedantic picks: nonminimal_bool, iter_on_single_items, tuple_array_conversions, manual_assert, match_wild_err_arm, if_then_some_else_none, manual_assert_eq, match_bool, unnecessary_literal_bound (output_mime is &'static str), unnecessary_debug_formatting, ref_option (#[expect] on serde's serialize_with), trivially_copy_pass_by_ref, used_underscore_binding, single_option_map. Also unused_self at two Google sites. suspicious_operation_groupings, missing_fields_in_debug, midpoint, borrow_as_ptr, stable sort and the rest came from the earlier commits. Invariants: all 26 non-lock shipping unwraps are reasoned expects; updater.rs binds its target once and needs none. No bare unreachable!() is left in crates/: each names its narrowing, and rig/images.rs became a match. The vague expects "text", "guarded" and the session lease message are reworded.

LEFT ON PURPOSE, for row 52 not to set to warn:
- filetype_is_file: all 5 hits refuse symlinks and devices on purpose; the lint's !is_dir() would weaken them.
- needless_raw_strings: 3 regexes stay raw like their neighbours.
- useless_let_if_seq: log.rs rename_backend and wire/phone.rs are accumulating flags.
- struct_field_names: Note.note, Calls.calls, etc.; one is a serialized exchange field.
- collection_is_never_read: 2 test false positives, a Vec held for RAII and bytes read by hash().
- future_not_send: the hits are main, test helpers and a generic; tokio::spawn already enforces Send.
- float_cmp: spending.rs:188 compares exactly on purpose to detect an amount too small to add.
- unused_self: skills.rs offers and realtime/live.rs's three error constructors, which mirror the socket's self-reading ones.
- unneeded_field_pattern: oauth.rs names the ignored field so a new one breaks the match.

The lints were checked on macOS only; Linux- and Windows-only code was only compiled by precheck. Nothing left unwired.
