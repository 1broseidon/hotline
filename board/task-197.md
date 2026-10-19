---
id: task-197
title: "hotline-store: the tape, the log and the search index as their own crate"
column: review
position: 5
priority: medium
tags:
  - crate-split
assignee: claude
relatedFiles:
  - crates/hotline-core/src/store.rs
  - crates/hotline-core/src/store
  - crates/hotline-core/src/log.rs
  - crates/hotline-core/src/log
  - crates/hotline-core/src/sent.rs
  - crates/hotline-core/src/thread.rs
parentId: epic-16
createdAt: "2026-10-19T06:07:00.407Z"
dependsOn:
  - task-194
  - task-196
updatedAt: "2026-10-19T08:14:20.536Z"
contract:
  status: delivered
  deliverables:
    - type: file
      path: crates/hotline-store/Cargo.toml
      description: store, log and sent, on hotline-contract and hotline-support
  validation:
    commands:
      - test -f crates/hotline-store/src/lib.rs
      - for f in store log sent; do test ! -e crates/hotline-core/src/$f.rs || { echo "$f.rs still in hotline-core"; exit 1; }; done
      - test -f crates/hotline-core/src/thread.rs
      - "! grep -qE '^hotline-(core|tools|websearch)' crates/hotline-store/Cargo.toml"
      - grep -A1 '^\[lints\]' crates/hotline-store/Cargo.toml | grep -q 'workspace = true'
      - git diff --quiet release/0.41.0...HEAD -- crates/hotline-core/tests/desk.rs crates/hotline-core/tests/desk
      - grep -q hotline-store docs/standards.md && grep -q hotline-store AGENTS.md
      - make check
      - git diff --exit-code ui/src/generated/contract.ts
      - make precheck
  constraints:
    - "Moves only: no behaviour change"
    - Tapes byte-compatible with the reference tree
    - thread stays in hotline-core
    - Desk harness unchanged
    - contract.ts byte-identical
  metrics:
    readyAt: "2026-10-19T06:11:04.523Z"
    pickedUpAt: "2026-10-19T08:04:18.855Z"
    reworkCount: 0
    deliveredAt: "2026-10-19T08:14:11.206Z"
    duration: 592
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T08:04:11.939Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 54892
    at: "2026-10-19T08:04:13.344Z"
    ended: "2026-10-19T08:14:20.536Z"
    outcome: review
---

## Description
store, log and sent, on hotline-contract and hotline-support. First cut the cycle: store/search.rs uses thread::{Link, ThreadState} (Link is in thread/link.rs, ThreadState in thread.rs), plain types that move down to hotline-contract or into the store crate; thread itself (it reaches room and session) stays in hotline-core. sent reaches images and thumbs (in hotline-support by now), log reaches credentials (atomic_write) and paths. The search index (store/search.rs) moves whole. Its tests move with it; the desk harness in crates/hotline-core/tests still passes unchanged (validation refuses any diff to tests/desk.rs or tests/desk/). Tapes stay byte-compatible: no event shape or segment layout change. If Link or ThreadState carries a ts-rs derive, contract.ts must still come out byte-identical.

Lead's note: this runs after task-196 and before task-198 because every crate move edits the same lines (workspace members, Cargo.lock, hotline-core's Cargo.toml and lib.rs, docs/standards.md's module map, AGENTS.md); run in parallel they would not land cleanly.

From task-194 and task-196: hotline-core re-exports hotline_contract as crate::contract and each hotline-support module under its old name (crate::paths, crate::credentials, crate::images, …), so no caller moved; do the same for store, log and sent unless moving callers is plainly simpler, and say which in the commit. hotline-support depends on no Hotline crate. is_linked's string grammar now lives in hotline-support's paths. Commit Cargo.lock with the move. Run `make check` and `make precheck` in the foreground (one call each, long timeout) and end your turn only once both have passed and the task is in Review; ending while they run in the background loses the hand-off.

## Log
- 2026-10-19T08:14:16.869Z: [claude] On dispatch/task-197 (98f84a15). hotline-store holds log, sent, and the store's chapters/previews/search flattened at its crate root, plus a `link` module that took Link, Opener, ThreadState and AgentBinding out of thread (none has a ts-rs derive; contract.ts is byte-identical). Its only Hotline dependencies are hotline-contract and hotline-support. Callers stayed put, re-exported rather than moved: hotline-core has `pub use hotline_store::log`, `use hotline_store::sent` and `pub(crate) use hotline_store as store`, so crate::log and crate::store::search still work, and thread re-exports the link types. To cross the crate boundary, Log::close_broadcasts and sent::read lost their #[cfg(test)], because core tests call them and see the store crate built without cfg(test). roxmltree moved from core to store. wire/files.rs now computes mime_guess on its own line, because cargo-machete could not see the call inside the one-line json!. The ("store","thread") cut is gone from scripts/module-map, and scripts/security-citations searches crates/hotline-store/src. Docs updated: standards, development layout, AGENTS.md, ADR 0007 cut 2, log.md, sessions.md. In the foreground: make check passed (3m21s) and make precheck passed (95s, all 8 legs); every validation command passes, and the desk harness is unchanged. For task-198: add hotline-tools to the workspace members after hotline-store, add the security-citations root if security.md cites tools files, and remove ("tools","driver") from PLANNED_CUTS. Nothing is left unwired.
