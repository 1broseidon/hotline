---
id: task-91
title: "Core: a relay grant role that may only stand in and carry visits"
column: todo
position: 5
priority: high
tags:
  - relay
  - security
  - core
parentId: epic-11
createdAt: "2026-10-17T05:37:01.415Z"
dependsOn:
  - task-90
updatedAt: "2026-10-17T07:46:57.499Z"
relatedFiles:
  - crates/hotline-core/src/remote/mod.rs
  - crates/hotline-core/src/remote/server.rs
  - crates/hotline-core/src/remote/admission.rs
  - crates/hotline-core/src/remote/v2.rs
  - crates/hotline-core/src/remote/link.rs
  - crates/hotline-core/src/remote/tests/relay_tests.rs
  - crates/hotline-core/src/remote/tests/admission_tests.rs
  - crates/hotline-cli/src/main.rs
  - crates/hotline-cli/src/remote.rs
  - crates/hotline-cli/tests/pair.rs
  - ui/src/generated/contract.ts
  - docs/security.md
---

## Description
Core: a `relay` grant role. Follows ADR 0006 (task-90).

- `DeviceRole` gains `Relay` (`remote/mod.rs:54`), with serde and the old-reader story ADR 0006 settles. Note `role` is a plain serde enum inside `remote.json`: an unknown role must refuse that one grant, never treat it as owner and never fail the whole file (which would lose every pairing).
- Relay grants may only open the relay control socket (`/v2/relay`), claim visits (`/v2/relay/accept`) and carry link-through (`/v2/link` via the relay). Every other seat purpose (`/v2` wire, computer paths, pair, door) is refused at admission in `sealed_session` with the sealed refusal. The owner-only check near server.rs:384 becomes "owner or relay".
- `hotline pair --link --role relay` makes one (keep `--companion` working, or fold it into `--role companion` and delete the old flag).
- `remote.devices` lists the role. Revocation works as today.
- Not here: a relay-only server making only relay grants (task-92, which owns capabilities).

Tests: a relay grant stands in and carries a phone visit end to end, and is refused every owner and companion command, the computer paths and the door. An unknown role on disk is refused, not upgraded, and the other grants still load. Add the relay role's row to docs/security.md's matrix and test index. Run `cargo test -p hotline-core` unfiltered before committing contract.ts.
