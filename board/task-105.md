---
id: task-105
title: "Audit: error handling and reachable panics"
column: todo
position: 3
priority: high
tags:
  - audit
  - errors
parentId: epic-12
createdAt: "2026-10-17T19:28:04.212Z"
---

## Description
Audit: error handling and panics.

- Count unwrap/expect/panic!/unreachable!/indexing in non-test code only (exclude #[cfg(test)] modules and tests/).
- Classify each cluster: an invariant that truly can't fail (keep, with an expect message saying why); reachable from user input, the wire, disk or a provider (must become an error); lock poisoning (one pattern).
- Review the 12 allow(clippy::result_large_err) and the error types: String errors vs typed errors, and where a typed error would pay off.
- Check that errors shown to the person stay a few words, with detail behind (ui rules).
- Report: docs/audits/0.40/errors.md, with the top reachable panics first.
