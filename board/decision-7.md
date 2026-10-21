---
id: decision-7
title: Integration harnesses share one explicitly registered binary per crate, except when process-global…
type: decision
status: current
claimKey: build.integration-test-roots
kind: decision
createdAt: "2026-10-21T02:23:11.800Z"
---

## Description
Integration harnesses share one explicitly registered binary per crate, except when process-global state requires isolation. Explicit #[path] declarations in test roots preserve the existing harness files and nested modules; each isolated [[test]] records its reason in Cargo.toml. This reduces repeated linking without coupling process-global fixtures.
