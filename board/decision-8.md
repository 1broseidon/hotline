---
id: decision-8
title: Windows MSVC uses Rust's bundled LLD in lld-link mode with +crt-static; the shipped GNU/Linux…
type: decision
status: current
claimKey: build.platform-linkers
kind: decision
recheck: Revisit if native release builds or binary smoke tests reveal linker incompatibilities.
createdAt: "2026-10-21T03:43:21.740Z"
---

## Description
Windows MSVC uses Rust's bundled LLD in lld-link mode with +crt-static; the shipped GNU/Linux targets explicitly request LLD through the compiler driver; macOS keeps Apple's linker. This reduces Windows link overhead without adding an LLVM installation there, makes Linux's fast-linker requirement explicit on ARM as well as x86-64, and preserves native system-library discovery. x86-64 stable Rust already uses bundled LLD, so Linux speed gains must be measured rather than presumed.
