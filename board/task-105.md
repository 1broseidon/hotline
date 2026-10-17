---
id: task-105
title: "Audit: error handling and reachable panics"
column: review
position: 3
priority: high
tags:
  - audit
  - errors
parentId: epic-12
createdAt: "2026-10-17T19:28:04.212Z"
contract:
  status: done
  deliverables:
    - type: docs
      path: docs/audits/0.40/errors.md
      description: Error handling and reachable panics audit report
  validation:
    commands:
      - test $(wc -l < docs/audits/0.40/errors.md) -ge 60
      - grep -q '^## Findings' docs/audits/0.40/errors.md && grep -q '^## Leave alone' docs/audits/0.40/errors.md && grep -q '^## Phase 2 tasks' docs/audits/0.40/errors.md
      - grep -Eq '[A-Za-z0-9_/.-]+\.rs:[0-9]+' docs/audits/0.40/errors.md
      - test -z "$(git diff --name-only release/0.40.0...HEAD -- . ':(exclude)docs/audits/0.40/errors.md')"
  constraints:
    - "Read-only audit: commit only docs/audits/0.40/errors.md"
    - "Report has exactly these sections: '## Findings' (top reachable panics first, each with file:line evidence), '## Leave alone' (with why), '## Phase 2 tasks' (each small, landable alone behind make check)"
    - If a cargo run rewrites ui/src/generated/contract.ts, restore it before committing
  metrics:
    readyAt: "2026-10-17T19:29:18.238Z"
    pickedUpAt: "2026-10-17T19:30:22.528Z"
    reworkCount: 0
    deliveredAt: "2026-10-17T19:38:04.193Z"
    duration: 462
updatedAt: "2026-10-17T19:38:38.712Z"
relatedFiles:
  - docs/audits/0.40/errors.md
  - crates/hotline-core/src
  - crates/hotline-app/src
  - crates/hotline-cli/src
dispatch:
  agent: codex
  model: gpt-6.1-sol
  effort: high
  by: lead
  at: "2026-10-17T19:29:49.248Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-17T19:29:52.955Z"
    ended: "2026-10-17T19:38:18.146Z"
    outcome: review
assignee: codex
---

## Description
Audit: error handling and panics.

- Count unwrap/expect/panic!/unreachable!/indexing in non-test code only (exclude #[cfg(test)] modules and tests/).
- Classify each cluster: an invariant that truly can't fail (keep, with an expect message saying why); reachable from user input, the wire, disk or a provider (must become an error); lock poisoning (one pattern).
- Review the 12 allow(clippy::result_large_err) and the error types: String errors vs typed errors, and where a typed error would pay off.
- Check that errors shown to the person stay a few words, with detail behind (ui rules).
- Report: docs/audits/0.40/errors.md, with the top reachable panics first.

## Log
- 2026-10-17T19:34:03.813Z: [codex] Audit scope is docs-only per contract. Production src inventory (test=false, all OS branches) is 84 unwrap, 94 expect, 1 panic!, 5 unreachable!, 562 index expressions; 58 unwraps are std mutex poisoning. Baseline has 13 result_large_err allowances, 7 production + 6 test helpers. Top source-traced reachable cases: desktop startup disk/socket/Tauri errors, discovery HTTP setup, CLI runtime/signal/entropy setup. Report will distinguish these from guarded invariants; no code fixes in this phase.
- 2026-10-17T19:38:14.007Z: [codex] Delivered docs/audits/0.40/errors.md in commit 4aae9f29 on dispatch/task-105; only that report changed. make check passed (2,024 Rust tests, 11 ignored; 420 UI tests; typecheck/Clippy/fmt/Python checks); contract validation passed in this worktree. Handoff for epic-12/task-109: prioritize desktop startup's four fallible expects, discovery client creation, CLI OS setup, then the 58 poison unwraps (especially Permit::drop). Source totals 84 unwrap/94 expect/1 panic/5 unreachable/562 indices; no unguarded malformed-input indexing panic established. There are 13 result_large_err allowances, seven production and six test: retain eight upstream tungstenite callback allows, consider boxing only five internal Rig Failure results with task-106. Search cooldowns branch on diagnostic strings; updater/provider/settings and retry UI still expose detail inline. Phase 2 proposals are in report; these are recommendations awaiting George's review, no behavior fixes or new standing decisions.
- 2026-10-17T19:38:38.712Z: Landed on release/0.40.0
