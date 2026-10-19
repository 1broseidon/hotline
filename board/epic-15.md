---
id: epic-15
title: "Rust quality, round two: typed errors, a staged crate split, and the four tangled functions"
type: epic
column: backlog
position: 32
priority: medium
tags:
  - quality
  - rust
  - post-0.40
createdAt: "2026-10-19T04:35:57.944Z"
---

## Description
From the post-0.40 sniff test (2026-10-18, release/0.40.0 ed9972cb). The tree passes: tidy layout, strict lints, 1,939 tests, design notes in the big modules. Three things keep it short of top-tier Rust.

1. Errors are mostly text: 846 functions return Result<_, String>, and only speech, tools, images and a few others have their own error types. Give each area a typed error (thiserror-style enums, no new dependency needed) so callers can tell failures apart; turn it into words only at the wire boundary. Do this first: it's the most visible gap.
2. hotline-core is one 163k-line crate with 58 dependencies. Split it in stages so its boundaries are enforced and builds get faster: contract (protocol types, already a leaf) first, then store, then voice and remote.
3. About 30 functions are over 120 lines. Routing tables like wire/commands.rs::run are fine. These four hold real logic and should read as named steps: session/exchanges.rs::exchange_step (264), voice/duplex.rs::carry_front (208), session/agent.rs::thread_agent (216), driver/rig/turn.rs::stream_response (183).

Not started without George.
