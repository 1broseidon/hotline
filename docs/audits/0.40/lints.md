# Audit: lint policy (0.40)

Method: `cargo clippy --workspace --all-targets` on rustc 1.99.0 with
`-W clippy::pedantic -W clippy::nursery`, then a second pass over a chosen set
of restriction and rustc lints. Counts are distinct (lint, file:line, message)
triples, so a hit seen by both the lib and test targets counts once. Test code
and `#[cfg(test)]` modules are included in the totals; the second number, where
given, is a rough non-test count (path filter only, so inline test modules in
`src/` are still counted). Baseline: `make check`'s clippy (`-D warnings`)
and `cargo fmt --check` both pass today. No `[lints]` table, `clippy.toml`,
`rustfmt.toml` or `rust-toolchain` file exists; the three crates share
`edition = "2024"` through `[workspace.package]`.

## Findings

### Policy state

- Every crate takes `edition.workspace = true`: consistent, nothing to fix.
- rustfmt runs on defaults (no `rustfmt.toml`) and is clean. Leave it so; the
  edition is read from Cargo, so no `edition` key is needed either.
- No `rust-version` in `[workspace.package]` and no toolchain pin. Adding
  `rust-version` is worth it: the `[lints]` table and clippy's MSRV-aware lints
  (`manual_midpoint`, `duration_suboptimal_units`) both key off it.
- `[workspace.package] version = "0.39.0"` while this branch is release 0.40.0.
  Not a lint matter; noted because the release process owns it.
- Lint levels are decided only by the Makefile's `-D warnings`. A plain
  `cargo clippy` or editor run is therefore warn-only, and lint levels are
  invisible to a reader of Cargo.toml. That is what the table fixes.

### Pedantic and nursery: worth adopting (small, mechanical, most auto-fixable)

| Lint | Count | Sample | Verdict |
|---|---|---|---|
| `clippy::uninlined_format_args` | 1 | `crates/hotline-core/src/voice/mod.rs:1990` | adopt now |
| `clippy::manual_string_new` | 1 | `crates/hotline-core/src/vault/shared.rs:696` | adopt now |
| `clippy::range_plus_one` | 1 | `crates/hotline-core/src/voice/speech/xai/tests.rs:883` | adopt now |
| `clippy::explicit_iter_loop` | 1 | `crates/hotline-core/src/driver/rig/turn.rs:1708` | adopt now |
| `clippy::implicit_clone` | 1 | `crates/hotline-core/src/voice/dispatcher.rs:999` | adopt now |
| `clippy::iter_on_single_items` | 1 | `crates/hotline-core/src/computer/cookies.rs:1024` | adopt now |
| `clippy::tuple_array_conversions` | 1 | `crates/hotline-core/src/remote/tests/viewer_token_tests.rs:110` | adopt now |
| `clippy::manual_midpoint` | 1 | `crates/hotline-core/src/voice/realtime/live.rs:563` | adopt now |
| `clippy::needless_raw_string_hashes` | 1 | `crates/hotline-core/src/mcp/oauth.rs:1617` | adopt now |
| `clippy::float_cmp` | 1 | `crates/hotline-core/src/spending.rs:267` | adopt now (test compares money floats; use an epsilon) |
| `clippy::manual_assert` | 1 | `crates/hotline-app/build.rs:93` | adopt now |
| `clippy::match_wild_err_arm` | 1 | `crates/hotline-cli/tests/serve.rs:210` | adopt now |
| `clippy::missing_fields_in_debug` | 1 | `crates/hotline-core/src/providers/copilot.rs:422` | adopt now (hand-written Debug that may hide a secret; check on purpose) |
| `clippy::single_option_map` | 1 | `crates/hotline-core/src/driver/acp.rs:2807` | adopt now |
| `clippy::suspicious_operation_groupings` | 1 | `crates/hotline-core/src/driver/mod.rs:125` | adopt now (read it: may be a real typo) |
| `clippy::nonminimal_bool` | 2 | `crates/hotline-core/src/log/mod.rs:172` | adopt now |
| `clippy::borrow_as_ptr` | 2 | `crates/hotline-core/src/computer/capacity.rs:158` | adopt now |
| `clippy::unnecessary_literal_bound` | 2 | `crates/hotline-core/src/voice/speech/mod.rs:147` | adopt now |
| `clippy::unnecessary_debug_formatting` | 2 | `crates/hotline-cli/src/update.rs:93` | adopt now |
| `clippy::trivially_copy_pass_by_ref` | 2 | `crates/hotline-core/tests/voice.rs:87` | adopt now |
| `clippy::useless_let_if_seq` | 3 | `crates/hotline-core/src/log/mod.rs:490` | adopt now |
| `clippy::equatable_if_let` | 3 | `crates/hotline-core/src/remote/server.rs:366` | adopt now |
| `clippy::manual_assert_eq` | 3 | `crates/hotline-core/src/computer/mod.rs:2116` | adopt now |
| `clippy::ref_option` | 3 | `crates/hotline-core/src/contract.rs:4109` | adopt now |
| `clippy::used_underscore_binding` | 3 | `crates/hotline-core/src/remote/tests/relay_tests.rs:842` | adopt now |
| `clippy::stable_sort_primitive` | 4 | `crates/hotline-core/src/linked.rs:736` | adopt now (`sort_unstable`) |
| `clippy::unchecked_time_subtraction` | 4 | `crates/hotline-core/src/computer/capacity.rs:346` | adopt now: `Instant - Duration` panics on underflow; real bug class |
| `clippy::struct_field_names` | 4 | `crates/hotline-core/src/session/chapters.rs:92` | adopt now |
| `clippy::match_wildcard_for_single_variants` | 4 | `crates/hotline-core/src/driver/acp.rs:2034` | adopt now: a new variant should break the match |
| `clippy::collection_is_never_read` | 3 | `crates/hotline-app/src/desks.rs:660` | adopt now (dead state) |
| `clippy::future_not_send` | 5 | `crates/hotline-core/src/providers/discovery.rs:182` | adopt now (read each; Send futures matter on tokio) |
| `clippy::unused_self` | 5 | `crates/hotline-core/src/imagegen/google.rs:31` | adopt now |
| `clippy::match_bool` | 5 | `crates/hotline-core/src/providers/compat.rs:180` | adopt now |
| `clippy::option_option` | 5 | `crates/hotline-core/src/contract.rs:4101` | not worth it: deliberate absent/null/value PATCH shape; `#[expect]` at the five sites |
| `clippy::struct_excessive_bools` | 5 | `crates/hotline-core/src/contract.rs:1060` | not worth it: wire-contract structs |
| `clippy::manual_is_variant_and` | 5 | `crates/hotline-core/src/computer/runtime.rs:166` | adopt now |
| `clippy::implicit_hasher` | 7 | `crates/hotline-core/src/driver/rig.rs:155` | not worth it: no caller varies the hasher |
| `clippy::unnecessary_wraps` | 8 | `crates/hotline-core/src/computer/mod.rs:1323` | adopt now (read each; some satisfy a trait) |
| `clippy::needless_collect` | 8 | `crates/hotline-core/src/session/peers.rs:415` | adopt now |
| `clippy::cast_lossless` | 9 | `crates/hotline-core/src/remote/link.rs:324` | adopt now (`u64::from`) |
| `clippy::needless_continue` | 9 | `crates/hotline-core/src/tools/workspace.rs:997` | adopt now |
| `clippy::match_same_arms` | 9 | `crates/hotline-core/src/mcp/oauth.rs:1302` | adopt later: some duplicate arms document distinct cases |
| `clippy::unnested_or_patterns` | 6 | `crates/hotline-core/src/driver/acp.rs:2928` | adopt now |
| `clippy::large_stack_arrays` | 11 | `crates/hotline-core/src/remote/sealed.rs:171` | adopt later: check the sites are tests/fixtures before setting the limit |
| `clippy::semicolon_if_nothing_returned` | 12 | `crates/hotline-core/src/desk.rs:709` | adopt now |
| `clippy::case_sensitive_file_extension_comparisons` | 13 | `crates/hotline-core/src/driver/acp/install.rs:112` | adopt now: archive names from a remote are matched case-sensitively today |
| `clippy::unused_async` | 13 | `crates/hotline-core/src/driver/acp/auth.rs:67` | adopt later: some are trait-shaped |
| `clippy::or_fun_call` | 14 | `crates/hotline-core/src/computer/mod.rs:685` | adopt now |
| `clippy::unused_async_trait_impl` | 15 | `crates/hotline-core/src/driver/rig.rs:566` | not worth it: the trait fixes `async` |
| `clippy::cast_sign_loss` | 16 | `crates/hotline-core/src/driver/auth.rs:187` | adopt later (see casts below) |
| `clippy::wildcard_imports` | 16 | `crates/hotline-core/src/driver/acp/auth.rs:2` | adopt later: check whether these are `use super::*` in tests (the lint exempts that) and fix the rest |
| `clippy::items_after_statements` | 17 | `crates/hotline-core/src/driver/acp.rs:645` | adopt later |
| `clippy::suboptimal_flops` | 17 | `crates/hotline-core/src/driver/rig/turn.rs:50` | not worth it: `mul_add` obscures budget arithmetic |
| `clippy::unreadable_literal` | 17 | `crates/hotline-core/src/vault.rs:1003` | adopt now (add `_`) |
| `clippy::missing_panics_doc` | 20 | `crates/hotline-core/src/log/thread.rs:69` | not worth it: binary-heavy crate, not a library API |
| `clippy::assigning_clones` | 21 | `crates/hotline-core/src/computer/mod.rs:582` | adopt now (`clone_from`, autofix) |
| `clippy::duration_suboptimal_units` | 21 | `crates/hotline-core/src/computer/mod.rs:88` | adopt now (`from_mins`, autofix) |
| `clippy::format_collect` | 22 | `crates/hotline-core/src/paths.rs:41` | adopt later |
| `clippy::significant_drop_in_scrutinee` | 23 | `crates/hotline-core/src/desk.rs:685` | adopt later: a real lock-held-across-match bug class; each site needs reading |
| `clippy::single_match_else` | 23 | `crates/hotline-core/src/desk.rs:747` | adopt now |
| `clippy::manual_let_else` | 24 | `crates/hotline-core/src/computer/mod.rs:567` | adopt now |
| `clippy::redundant_clone` | 25 | `crates/hotline-core/src/driver/rig/recovery.rs:94` | adopt now: pure waste |
| `clippy::format_push_string` | 26 | `crates/hotline-core/src/sent/generated.rs:151` | adopt now |
| `clippy::redundant_closure_for_method_calls` | 29 | `crates/hotline-core/src/driver/rig.rs:1372` | adopt now |
| `clippy::cast_possible_wrap` | 33 | `crates/hotline-core/src/driver/acp.rs:2809` | adopt later (see casts below) |
| `clippy::cast_precision_loss` | 35 | `crates/hotline-core/src/computer/cookies.rs:521` | not worth it: money and size display only |
| `clippy::map_unwrap_or` | 45 | `crates/hotline-core/src/computer/cookies.rs:529` | adopt now (`map_or`, autofix) |
| `clippy::derive_partial_eq_without_eq` | 47 | `crates/hotline-core/src/contract.rs:34` | not worth it: ts-rs types with floats or `Value`; adds derives nobody uses |
| `clippy::default_trait_access` | 52 | `crates/hotline-core/src/capabilities.rs:189` | adopt now (autofix) |
| `clippy::cast_possible_truncation` | 57 | `crates/hotline-core/src/computer/login.rs:144` | adopt later (see casts below) |
| `clippy::similar_names` | 61 | `crates/hotline-core/src/computer/cookies.rs:774` | not worth it: pure taste, noisy |
| `clippy::option_if_let_else` | 63 | `crates/hotline-core/src/computer/cookies.rs:687` | not worth it: nursery, pushes toward `map_or_else` chains that read worse than `match` |
| `clippy::needless_pass_by_value` | 78 | `crates/hotline-core/src/computer/gate.rs:377` | adopt later: real signal on `String`/`Vec` params, but many are `async` or spawn-moved |
| `clippy::too_long_first_doc_paragraph` | 84 | `crates/hotline-core/src/computer/runtime.rs:86` | not worth it: the house style is long explanatory doc paragraphs; it is the point of the AGENTS.md comment rule |
| `clippy::doc_markdown` | 88 | `crates/hotline-app/build.rs:4` | not worth it: product and protocol names (`WebSocket`, `OAuth`, `Hotline`) trip it; needs a long `doc-valid-idents` list |
| `clippy::missing_const_for_fn` | 108 | `crates/hotline-core/src/computer/runtime.rs:31` | not worth it: nursery, churns signatures, promises const-ness nobody asked for |
| `clippy::large_futures` | 114 | `crates/hotline-core/src/remote/relay.rs:807` | adopt later: valid concern for a server with many tasks; fix is `Box::pin`, set `future-size-threshold` once |
| `clippy::too_many_lines` | 115 | `crates/hotline-core/src/computer/cookies.rs:114` | adopt later: leave off in the table; use it as a one-off audit lens, since the 115 long functions are a refactor backlog |
| `clippy::ignored_unit_patterns` | 123 | `crates/hotline-core/src/desk.rs:736` | adopt now (`Ok(())`; autofix) |
| `clippy::use_self` | 180 | `crates/hotline-core/src/computer/cookies.rs:46` | adopt later (autofix, but large diff; land in one mechanical commit) |
| `clippy::assert_is_empty` | 201 | `crates/hotline-cli/tests/pair.rs:142` | adopt now (autofix, tests only) |
| `clippy::redundant_pub_crate` | 216 | `crates/hotline-core/src/computer/files.rs:25` | not worth it: conflicts with `unreachable_pub` and the repo uses `pub(crate)` as documentation |
| `clippy::significant_drop_tightening` | 239 | `crates/hotline-core/src/computer/mod.rs:548` | not worth it: nursery, known false positives; pursue the real bug class with `significant_drop_in_scrutinee` above |
| `clippy::must_use_candidate` | 239 | `crates/hotline-core/src/computer/runtime.rs:31` | not worth it: noise on a binary-shaped codebase |
| `clippy::missing_errors_doc` | 259 | `crates/hotline-core/src/computer/cookies.rs:537` | not worth it: errors here are `String`/internal; the docs explain why, not "Returns Err when" |

Casts (`cast_possible_truncation` 57, `cast_possible_wrap` 33, `cast_sign_loss`
16, `cast_precision_loss` 35, `cast_lossless` 9): of these, only `cast_lossless`
is adopt-now. The rest are worth a one-time read of non-test sites (audio
sample math, byte counts, a Windows tile) but not a standing warn, because each
fix is `#[expect]` plus a reason on legitimately bounded math.

### Restriction and rustc lints (second pass)

Nothing here is adopted as a blanket; the picks are lints that name a
specific mistake.

| Lint | Count (non-test) | Sample | Verdict |
|---|---|---|---|
| `clippy::unwrap_used` | 6772 (2799) | `crates/hotline-app/build.rs:10` | not worth it as a lint: the counts include `unwrap` inside tests and build scripts; a stricter policy belongs on the daemon's request paths, as an audit lens |
| `clippy::indexing_slicing` | 3783 (1090) | `crates/hotline-app/build.rs:123` | not worth it |
| `clippy::str_to_string` | 2219 (1557) | `crates/hotline-app/build.rs:102` | not worth it: style; `to_owned` vs `to_string` |
| `clippy::clone_on_ref_ptr` | 745 (364) | `crates/hotline-core/src/capabilities.rs:135` | not worth it: `.clone()` on `Arc` is the idiom here |
| `clippy::expect_used` | 505 (323) | `crates/hotline-app/build.rs:52` | not worth it: `expect` with a message is the repo's preferred form |
| `unused_qualifications` | 426 (266) | `crates/hotline-core/src/capabilities.rs:69` | adopt later: autofix, but it fights `std::sync::Mutex` vs tokio's, which this repo spells out on purpose; try and see |
| `clippy::map_err_ignore` | 372 (370) | `crates/hotline-core/src/computer/files.rs:122` | not worth it: errors are flattened to `String` on purpose |
| `clippy::let_underscore_must_use` | 357 (312) | `crates/hotline-core/src/computer/gate.rs:229` | not worth it: `let _ = tx.send(...)` is deliberate when the receiver is gone |
| `let_underscore_drop` | 341 (294) | `crates/hotline-core/src/computer/mod.rs:320` | not worth it |
| `clippy::unused_trait_names` | 245 (196) | `crates/hotline-core/src/computer/cookies.rs:24` | adopt later: `use Trait as _`, autofix, large mechanical diff |
| `unreachable_pub` | 201 (175) | `crates/hotline-core/src/capabilities.rs:23` | adopt later: it finds real dead public surface, and AGENTS.md says unreachable code does not ship; first decide whether `hotline-core` is a library for the app/CLI (they use its `pub` items) |
| `clippy::panic` | 179 (83) | `crates/hotline-app/build.rs:84` | not worth it: build scripts and test asserts |
| `clippy::print_stderr` | 164 (150) | `crates/hotline-core/src/desk.rs:332` | adopt later: the core is a daemon with a log file (`tests/log_file.rs`); stray `eprintln!` is a candidate, except in the CLI and `hotline-import` binaries (`#[expect]` or per-crate level) |
| `clippy::print_stdout` | 79 (53) | `crates/hotline-core/src/bin/hotline-import.rs:46` | not worth it: the CLI's output; same per-crate treatment |
| `missing_debug_implementations` | 90 (90) | `crates/hotline-core/src/computer/mod.rs:196` | not worth it: several types hold secrets or sockets, and a derive could print them |
| `clippy::semicolon_outside_block` | 59 (50) | `crates/hotline-core/src/computer/cookies.rs:186` | not worth it: rustfmt-shaped preference |
| `clippy::allow_attributes` | 27 (17) | `crates/hotline-core/src/driver/rig/turn.rs:43` | adopt now: prefer `#[expect]`, which fails when the reason goes away; pairs with deleting stale allows (below) |
| `clippy::allow_attributes_without_reason` | 28 (17) | `crates/hotline-core/src/driver/rig/turn.rs:43` | adopt now: 27 of 28 lack a reason; the lint enforces AGENTS.md "comments say why" |
| `clippy::undocumented_unsafe_blocks` | 22 (18) | `crates/hotline-core/src/driver/auth.rs:166` | adopt now: security-adjacent, and the fix is a `// SAFETY:` comment |
| `elided_lifetimes_in_paths` | 19 (19) | `crates/hotline-core/src/import/records.rs:137` | adopt later (`rust_2018_idioms`) |
| `clippy::filetype_is_file` | 5 (5) | `crates/hotline-core/src/credentials/file.rs:127` | adopt now: `!is_file()` is true for symlinks and devices, which matters beside credentials |
| `trivial_casts` | 4 (3) | `crates/hotline-core/src/session/dm.rs:143` | adopt now |
| `clippy::needless_raw_strings` | 3 (3) | `crates/hotline-core/src/link_preview.rs:198` | adopt now |
| `clippy::exit` | 2 (2) | `crates/hotline-app/src/lib.rs:316` | adopt later: `exit` skips destructors, which matters for the lease and child cleanup; read both sites |
| `clippy::mem_forget` | 1 (1) | `crates/hotline-app/src/notify.rs:97` | adopt now: read the site |
| `clippy::unneeded_field_pattern` | 1 (1) | `crates/hotline-core/src/mcp/oauth.rs:1073` | adopt now |
| `clippy::if_then_some_else_none` | 1 (1) | `crates/hotline-core/src/wire/files.rs:223` | adopt now |

Not found with hits: `dbg_macro`, `todo`, `unimplemented`, `empty_drop`,
`lossy_float_literal`, `get_unwrap`, `string_to_string`, `rc_buffer`,
`try_err`, `verbose_file_reads`, `unsafe_op_in_unsafe_fn`, `unused_import_braces`,
`unused_lifetimes`, `single_use_lifetimes`, `keyword_idents`, `trivial_numeric_casts`.
These are free to deny today and keep the door shut. (One `renamed_and_removed_lints`
note appeared; it came from a flag name on this toolchain, not from the repo.)

### The `#[allow]`s

There are 28 attributes under `crates/` (no `#![allow]` at crate roots except
`tests/common/stalled.rs:8`). The task brief said 8 `too_many_arguments`
allows; there are 9.

**`too_many_arguments` (9).** Clippy's default limit is 7; each of these has
7-9. Whether a refactor removes the need:

| Site | Args | Refactor |
|---|---|---|
| `crates/hotline-core/src/wire/mod.rs:1903` `subscribe` | 8 | Yes. `log`, `room`, `sender`, `subscriptions`, `seat` are the connection's context; passing a `&mut Connection` (or the existing per-socket state) removes five arguments. `stream_events` below builds from the same set. |
| `crates/hotline-core/src/wire/mod.rs:2270` `stream_events` | 8 | Yes, same context struct plus `StreamFollow { id, stream, window }`. |
| `crates/hotline-core/src/wire/commands.rs:965` `mobile_persona_create` | 8 | Yes. `name, goal, backend_id, model_id, effort_id` is a persona draft; it exists already as the create command's own payload, so pass that. |
| `crates/hotline-core/src/voice/mod.rs:768` `start_call` | 8 (with self) | Yes. `target, stream_audio, input_mode, mode, mic_rate` are call options; one `CallOptions` struct. |
| `crates/hotline-core/src/voice/mod.rs:2339` `speak_sentence` | 10 (with self) | Yes. `id, line, speech` are the call's identity; `sentence, index, last` are the sentence; `interrupted, ended` are the cancel tokens. Three structs or one `Sentence` plus the call. |
| `crates/hotline-core/src/voice/mod.rs:2374` `synthesize_stream` | 8 | Yes, as above; it is called from `speak_sentence`, so they shrink together. |
| `crates/hotline-core/src/session/exchanges.rs:176` `enqueue_exchange` | 8 | Yes. `from, to, intent, capability, sent_from` is a routing envelope that other `peers` code also builds. |
| `crates/hotline-core/src/remote/mod.rs:737` `prompt` | 8 | Yes. `text, attachment_ids, reply_to, thread` is one prompt body; check whether the phone command already holds it. |
| `crates/hotline-core/src/driver/rig/turn.rs:689` `recover` | 9 | Yes. `retries, repaired, history, results, jobs` are the turn's mutable state: one `&mut TurnState`. |

All nine are removable by grouping, and AGENTS.md's rule against adding a layer
holds: each group is a thing that already travels together. None should be
done before the table lands; the table keeps `too_many_arguments` at its default
(warn, deny in CI) so each allow is converted to a refactor or an
`#[expect(..., reason)]`.

**`result_large_err` (13 sites).** Sites: `wire/mod.rs:1021`,
`driver/rig/turn.rs:43,141,194,689`, `driver/rig.rs:718`, `remote/bridge.rs:236`
and the tests in `remote/tests.rs:1553`, `voice/speech/xai/tests.rs:213,426`,
`voice/realtime/socket/tests.rs:29,342,484`. The non-test `driver` ones probably
return the in-house `Failure` (`driver/failure.rs:37`); its size was not
measured here. If it is over clippy's 128-byte limit, boxing its heavy field
removes five allows. The
`wire/mod.rs:1021` and `remote/bridge.rs:236` ones are forced by tungstenite's
`ErrorResponse` callback type (the latter already says so in a comment), and
the test ones return tungstenite errors through helpers. Verdict: measure `Failure` and box it if it is the cause;
leave the rest as `#[expect(clippy::result_large_err, reason = "...")]`.

**`dead_code` (6).** `tests/common/mod.rs:26,32,37,50` and
`tests/common/stalled.rs:8` are the shared test-helper module compiled into
each test binary, which is the standard reason; keep. `import/records.rs:39`
and `process_windows.rs:38` (a `Job` that holds a handle only for its `Drop`)
should become `#[expect(dead_code, reason = ...)]`, or `import/records.rs:39`
should lose the unread field outright (AGENTS.md "delete, don't disable").

## Leave alone

- **rustfmt defaults.** `cargo fmt --check` is clean and there is no config. A
  `rustfmt.toml` with anything beyond defaults would be a diff against every
  file for no invariant.
- **Edition.** One value, inherited three times. Nothing to change.
- **`-D warnings` in `make check`.** Keep it. Moving levels into the table
  does not replace it: `-D warnings` is what turns the table's `warn`
  entries into failures in CI while keeping editors quiet.
- **`dead_code` in `tests/common`.** Each integration test binary compiles the
  module and uses a subset of it. Removing the allow means splitting the helpers
  per binary for nothing.
- **Docs-heavy pedantic lints** (`missing_errors_doc`, `missing_panics_doc`,
  `too_long_first_doc_paragraph`, `doc_markdown`, `must_use_candidate`,
  `missing_const_for_fn`, `redundant_pub_crate`, `significant_drop_tightening`,
  `option_if_let_else`, `similar_names`). About 1,300 hits between them, samples read were
  noise rather than defects, and several argue with the house style in
  AGENTS.md (long explanatory comments, `pub(crate)` as documentation).
- **Panic/unwrap/index/print lints as blanket.** Test code and build scripts
  make up most hits; the daemon's request path is the interesting part and is
  better handled in review than by a standing 6,700-hit warn.
- **`option_option`, `struct_excessive_bools`, `derive_partial_eq_without_eq`.**
  They describe the wire contract shapes (absent/null/value; ts-rs types).
- **`generated.rs` and `ui/src/generated/contract.ts`.** Generated, and
  `cargo clippy` does not rewrite the contract; I confirmed the tree stayed
  clean after the runs. Do not lint-fix generated output by hand.

## Phase 2 tasks

Order: put the table in first at warn level with the existing Makefile
`-D warnings` unchanged, but with the lints that currently fire set `allow`,
so that every later slice flips one lint to `warn` after fixing it. Each task
below is landable alone behind `make check`.

Proposed table (for a later task to add; not added here). Each crate then
declares `[lints] workspace = true`.

```toml
[workspace.lints.rust]
unsafe_op_in_unsafe_fn = "deny"
trivial_numeric_casts = "warn"
unused_lifetimes = "warn"
unused_import_braces = "warn"
single_use_lifetimes = "warn"
keyword_idents = "warn"
# Slice 4: trivial_casts = "warn"
# Later: unreachable_pub, unused_qualifications, elided_lifetimes_in_paths

[workspace.lints.clippy]
# Group baseline: clippy's defaults stay on, pedantic is opt-in per lint,
# not wholesale, because most of its hits are lints the repo's style
# rejects (see "Leave alone").
dbg_macro = "deny"
todo = "deny"
unimplemented = "deny"
get_unwrap = "deny"
string_to_string = "deny"
rc_buffer = "deny"
try_err = "deny"
empty_drop = "deny"
lossy_float_literal = "deny"
verbose_file_reads = "deny"

# Adopt after the slice that fixes them lands (slice number in brackets).
# [1] autofixable
ignored_unit_patterns = "warn"
map_unwrap_or = "warn"
default_trait_access = "warn"
assert_is_empty = "warn"
assigning_clones = "warn"
duration_suboptimal_units = "warn"
redundant_closure_for_method_calls = "warn"
semicolon_if_nothing_returned = "warn"
uninlined_format_args = "warn"
cast_lossless = "warn"
unreadable_literal = "warn"
unnested_or_patterns = "warn"
manual_let_else = "warn"
single_match_else = "warn"
format_push_string = "warn"
# [2] small by hand
redundant_clone = "warn"
unchecked_time_subtraction = "warn"
case_sensitive_file_extension_comparisons = "warn"
match_wildcard_for_single_variants = "warn"
filetype_is_file = "warn"
undocumented_unsafe_blocks = "warn"
# [3] allows become expects with reasons
allow_attributes = "warn"
allow_attributes_without_reason = "warn"
too_many_arguments = "warn"      # default; slice 5 retires the nine allows
result_large_err = "warn"        # default; slice 6 retires them
```

1. **Install the table and `rust-version`.** Add `[workspace.lints.rust]`
   and `[workspace.lints.clippy]` with only the "deny today" lints (no hits), set
   `rust-version` to the oldest toolchain that builds, add `[lints] workspace = true`
   to the three crate manifests. Proof: `make check` unchanged. No code edits.
2. **Autofix slice.** `cargo clippy --fix` for the "[1]" lints above (about 700
   edits, no behaviour change), one commit per lint so each is reviewable; enable
   each in the table in the same commit. Run the whole `cargo test -p hotline-core`
   unfiltered, then restore `ui/src/generated/contract.ts` if it moved.
3. **By-hand singles.** The "[2]" lints plus the 1- to 5-hit pedantic lints in the
   first table (about 60 sites). Read `suspicious_operation_groupings`
   (`driver/mod.rs:125`), `missing_fields_in_debug` (`providers/copilot.rs:422`),
   and `unchecked_time_subtraction` first: they are the likeliest to be bugs.
4. **Allow hygiene.** Convert the remaining `#[allow]`s to `#[expect(..., reason)]`,
   delete `import/records.rs:39`'s unread field, and enable `allow_attributes` and
   `allow_attributes_without_reason`. This must follow slices 5-6 or be done
   together with them, since those delete most of the clippy ones.
5. **Argument groups.** Nine `too_many_arguments` refactors, one commit per
   function chain: `wire` (`subscribe` + `stream_events`), `voice` (`start_call`;
   `speak_sentence` + `synthesize_stream`), `mobile_persona_create`,
   `enqueue_exchange`, `remote::prompt`, `recover`. Each shrinks its allow to
   nothing. Test with the existing headless harnesses (`make verify`) for wire,
   voice and remote.
6. **`Failure` size.** Measure `driver/failure.rs`'s `Failure`; if it is the
   cause, box its large field so the five `driver` `result_large_err` allows go; the tungstenite and test ones
   become `#[expect]`.
7. **Audit lenses, no table entry.** Read the 23 `significant_drop_in_scrutinee`
   and 114 `large_futures` hits (and the 6 non-test `exit`/`mem_forget`
   sites) as a review of lock and future-size behaviour; adopt the lint only if
   the review finds a bug class. Same for `unreachable_pub` against the
   question of whether `hotline-core`'s `pub` surface is a library API.
