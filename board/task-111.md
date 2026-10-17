---
id: task-111
title: "Git hooks in .githooks (like ketch): pre-commit fmt/typecheck/contract guard, pre-push make precheck"
column: review
position: 0
priority: high
tags:
  - ci
  - hooks
  - quality
createdAt: "2026-10-17T20:18:20.491Z"
dependsOn:
  - task-110
landsOn: release/0.40.0
updatedAt: "2026-10-17T21:08:39.392Z"
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-17T20:18:22.773Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-17T21:06:16.036Z"
assignee: claude
---

## Description
Git hooks for the desk repo, in the same style as George's Go projects (ketch, recoil): plain POSIX sh scripts in a versioned `.githooks/` folder, enabled with `git config core.hooksPath .githooks`. No hook manager and no extra runtime (George, 2026-10-17). ketch's .githooks/pre-commit is the model: a `==> step` line, then the command, with `set -e`.

- **.githooks/pre-commit** (seconds):
  - `cargo fmt --all -- --check`;
  - prettier --check and the TypeScript typecheck on staged ui/ files only;
  - refuse a staged ui/src/generated/contract.ts that is shorter than HEAD's by a large margin or fails to parse (the filtered-cargo-test truncation hazard), saying to run the unfiltered `cargo test -p hotline-core`;
  - refuse staged secrets-looking files (.p8, *.pem, auth.json) as a cheap guard.
- **.githooks/pre-push** (minutes): `make precheck` from task-110. Print how to skip it in an emergency (`git push --no-verify`), never skip silently.
- `make hooks` sets core.hooksPath. `make dev` / `make check` print a one-line nudge when hooks aren't enabled; they never enable it themselves.
- Worktrees share core.hooksPath, so dispatched brainfile agents get the same gate. Check that `brainfile land`'s merge commits aren't blocked wrongly: merges run pre-merge-commit, not pre-commit, so decide whether to add one.
- docs/development.md: what each hook runs, how long it takes, and how to enable it.

Tests: a script test (or a CI-free shell test under scripts/) that a mis-formatted Rust file and a truncated contract.ts are each refused by pre-commit, and a clean tree passes.

## Log
- 2026-10-17T21:08:39.049Z: [claude] Done on branch dispatch/task-111 (0bb2106b). .githooks/pre-commit (1–2 s): refuses staged *.p8/*.pem/auth.json; cargo fmt --check when .rs staged; staged contract.ts refused if it lost >1/4 of HEAD's lines or fails tsc alone; bun typecheck when ui/ staged. .githooks/pre-push: make precheck, naming --no-verify on failure. make hooks sets core.hooksPath; make dev/check print a nudge only. scripts/test_githooks.py (in make check) proves clean passes and misformatted Rust, truncated contract, uncompilable contract, key file are refused. No pre-merge-commit: clean merges join pre-commit-checked commits, conflicted merges finish via git commit which runs pre-commit. Left out: prettier, which the repo neither depends on nor configures, so --check would mean reformatting the whole window first. Not enabled: George runs make hooks (shared config, affects every worktree).
