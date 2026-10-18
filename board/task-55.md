---
id: task-55
title: Desk drops a phone's dead socket sooner
column: todo
position: 3
priority: low
tags:
  - remote
createdAt: "2026-10-14T19:29:48.338Z"
updatedAt: "2026-10-18T23:53:55.552Z"
parentId: epic-14
relatedFiles:
  - crates/hotline-core/src/wire/socket.rs
  - crates/hotline-core/src/wire/phone.rs
  - crates/hotline-core/src/remote/v2.rs
contract:
  status: ready
  deliverables:
    - type: test
      path: crates/hotline-core
      description: "over the wire: a seat whose writes back up (a peer that stops reading) is closed by the desk within a bound"
  validation:
    commands:
      - cargo fmt --all --check
      - cargo clippy --workspace --all-targets -- -D warnings
      - cargo test -p hotline-core
      - git diff --exit-code ui/src/generated/contract.ts
  metrics:
    readyAt: "2026-10-18T23:53:55.554Z"
dependsOn:
  - task-187
  - task-183
---

## Description
Seen 2026-10-14: after the phone left Wi-Fi for the Cloudflare tunnel, the desk kept the old LAN socket ESTABLISHED with ~20 KB queued until TCP gave up. The desk should notice on its own (ping/idle timeout on a seat whose writes back up) and close it.

0.40 triage (2026-10-18): Remaining: a 90 s silence timeout exists for phone links (wire/socket.rs:207), yet the Oct 14 socket stayed up. Find which seat it was, then close a seat whose writes back up (bounded send queue) or have the desk ping it. Over-the-wire test.
