---
id: epic-23
title: "CI and build speed: CI under 15 min, then a fast gate under 10, each change measured before and after"
type: epic
column: todo
position: 1
priority: high
tags:
  - ci
  - build
  - "0.42"
createdAt: "2026-10-21T02:18:25.798Z"
---

## Description
George, 2026-10-20 21:14: "go ahead, let's do it properly and efficiently". The goal is to cut Rust build and CI times. Each change is its own PR into release/0.42.0, with before/after numbers measured the same way, by workflow_dispatch on the branch against the baseline below.

Baseline (CI at a185fff1, release runs for 0.40 and 0.41):
- CI wall time 25–30 min, set by the Windows check (29 min; its one step of fmt, clippy, cargo test and the window checks takes 24.2 min). macOS takes 21 min (clippy 6.2, cargo test 14.4), Linux 16 (that step 15.5), the dependency gates under 1.
- On macOS, cargo test runs its tests for only 162 s across ~80 test binaries. About 11 min goes on compiling and linking them. Integration test binaries: hotline-core 14, hotline-cli 4, hotline-app 2, hotline-support 1, each linking hotline-core and its dependencies.
- release-cache builds 4 release binaries (12–19 min each) on every non-PR run, release-branch dispatches included, but saves its cache only on main.
- desktop-release takes 15–19 min wall, already a parallel matrix. The slowest leg is Windows (12–17) or Apple silicon (up to 18). It restores main's release-<target> cache.
- No [profile.release]: default (no LTO, 16 codegen units). No linker is configured. The workspace split is done (ADR 0007: an edit to hotline-core rebuilds in ~5 s, the test build in ~12 s).

Not in scope: the phone (hotline-mobile, built with Expo and EAS), LTO (none is set), more crate splitting (ADR 0007 says it buys little more).

Order: the single test binary (biggest) and release-cache on main only go first, in parallel. Then faster linkers. Then nextest. Then the gate, which George decides with the numbers. The local-loop docs come last.
