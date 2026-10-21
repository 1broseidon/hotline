---
id: task-257
title: "task-20 follow-up: zero TCP linger on every WebSocket breaks all remote tests on Windows"
column: in-progress
position: 0
priority: high
tags:
  - performance
  - windows
parentId: epic-5
createdAt: "2026-10-21T11:21:12.260Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T11:21:18.478Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T11:21:20.348Z"
assignee: codex
updatedAt: "2026-10-21T11:23:31.285Z"
decisions:
  - decision-24
---

## Description
CI on dispatch/epic-5 at dfb4048d (run 39587319717): 68 tests fail on Windows only — every remote::tests module (relay 17, phone 11, sealed_network 7, rust_client 7, link 7, admission 5, …) and wire::tests::door. Typical panics: tests.rs:175 pairing_result unwrap on None; 'The desk did not authenticate this device.' Linux and macOS pass; make precheck's windows leg only cross-compiles.

Cause: task-20 sets SO_LINGER=0 on every WebSocket transport (wire/door.rs serve, remote relay upgraded_socket, remote/client.rs dial). On Windows, closesocket with zero linger resets at once and discards unsent data, so an ordinary close after the last reply (pairing claim, auth answer, final frame) loses that reply.

Done when: normal closes drain as before; only a seat dropped for stalling (the write-bound path) closes abortively, e.g. set zero linger right before dropping that seat, not at accept/dial. a_seat_that_stops_reading_is_closed_once_a_frame_waits_past_the_bound still proves the reset. Lands on dispatch/epic-5; merge it, make check in the foreground, commit. Pork Chop runs Windows CI on the branch to prove it.

## Log
- 2026-10-21T11:23:30.974Z: [codex] Confirmed zero linger at all three transport setup paths. Moving reset selection into joined seat teardown only after WRITE_STALL, preserving normal TLS/WebSocket drains. Existing stalled-seat reset harness remains the required proof; pairing/auth and door tests cover final replies.
- 2026-10-21T11:29:20.755Z: [codex] make check passed in the foreground: 1,240 core unit tests plus workspace/integration suites, including the unchanged stalled-seat TCP-reset proof and new normal-revocation drain regression. make precheck is running; Windows runtime validation remains Pork Chop's CI step.
