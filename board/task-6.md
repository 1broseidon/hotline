---
id: task-6
title: A moved data directory should say so instead of asking for every sign-in again
column: backlog
position: 7
priority: medium
tags:
  - bro-20
createdAt: "2026-10-11T21:05:17.853Z"
description: |-
  **Rechecked 30 Sep 2026 on main:** still valid. The key is now built in `crates/hotline-core/src/credentials.rs`, `CredentialFile::key` (line 269). It is still a SHA-256 of the canonical root joined with the relative path, plus generation and chunk. The `toad-core` path below dates from before the rename.

  OS-store keys are a SHA-256 of the canonical data-root path plus the credential file's relative path plus a generation id (`crates/toad-core/src/credentials.rs`, `CredentialFile::key`, verified at line 171 at edb40b2). Binding to the path is deliberate: a copied reference file cannot resolve another install's secrets. The cost is that renaming or moving the data directory on the same machine — or a change in how the OS resolves it (macOS /var vs /private/var, a symlinked home) — makes every stored credential unreachable, and each one fails with "This credential is missing from the OS store. Restore it or sign in again." with no hint that the folder moved.

  Decide on one of:

  (a) detect the mismatch and say the data directory moved and which sign-ins are affected;
  (b) record the root path in the reference file and offer a one-time re-key of every entry to the new root when the user confirms the move;
  (c) key by a per-install id stored in the data root instead of the path, which keeps copies from resolving across roots while surviving a rename.

  Keep the cross-machine behaviour the changelog documents. Add a test that renames the root and asserts the chosen behaviour.

  ## Migration source

  Brainfile: toad-next/task-67 · Source file: `.brainfile/board/task-67.md` on the Linux desk (gitignored, local only)

  Migrated from Linear BRO-20: https://linear.app/1broseidon/issue/BRO-20/a-moved-data-directory-should-say-so-instead-of-asking-for-every-sign (2026-10-11).

  Closed 2026-10-18 (backlog triage): Won't do (2026-10-11 review): the path-bound key is deliberate. Option (a), a 'data directory moved' message, is the cheap path if it's ever reopened.
updatedAt: "2026-10-18T23:41:02.370Z"
---

## Log
- 2026-10-12T01:51:08.456Z: [claude] Won't do (2026-10-11 sort): rare edge case (moving the data directory by hand); re-signing in is an acceptable cost.
