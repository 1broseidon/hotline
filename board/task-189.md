---
id: task-189
title: "0.40 easy wins land together: the release branch passes check, verify and precheck with every slice in"
column: todo
position: 15
description: |-
  The last child of epic-14. Every other child has landed on release/0.40.0 by the time this starts. The slices were built apart, several of them in the same files (failure.rs wording from task-8, task-188 and task-181; wire/socket.rs from task-187 and task-55; the generated contract from task-180, task-185 and the failure kinds).

  Do:
  1. On a fresh fork of release/0.40.0, run `make check`, `make verify` and `make precheck`. Fix anything that broke where two slices met: a failure notice one slice reworded and another's test still expects, contract.ts not matching an unfiltered `cargo test -p hotline-core`, a socket-lane change from task-187 that task-55's backed-up-seat close doesn't cover. No new behaviour.
  2. Check docs/ still describes what is: anything a slice changed (request_human withdraw and lifetime, the OAuth page's CSP, the registry's last good list, the failure notices) is stated where docs/design.md or docs/security.md already speak to it. task-180 and task-182 touch what a teammate can reach or what a browser loads: confirm their docs/security.md entries exist.
  3. Write a QA checklist as a note on epic-14, one line per child, for George's pass on the Linux QA desktop with his xAI sign-in: what to do and what he should see.

  Done when all three make targets pass on the branch with every slice in, and the checklist is on epic-14.
priority: high
tags:
  - "0.40"
  - easy-wins
  - integration
relatedFiles:
  - ui/src/generated/contract.ts
  - docs/design.md
  - docs/security.md
parentId: epic-14
createdAt: "2026-10-18T23:53:30.443Z"
---

## Description
The last child of epic-14. Every other child has landed on release/0.40.0 by the time this starts. The slices were built apart, several of them in the same files (failure.rs wording from task-8, task-188 and task-181; wire/socket.rs from task-187 and task-55; the generated contract from task-180, task-185 and the failure kinds).

Do:
1. On a fresh fork of release/0.40.0, run `make check`, `make verify` and `make precheck`. Fix anything that broke where two slices met: a failure notice one slice reworded and another's test still expects, contract.ts not matching an unfiltered `cargo test -p hotline-core`, a socket-lane change from task-187 that task-55's backed-up-seat close doesn't cover. No new behaviour.
2. Check docs/ still describes what is: anything a slice changed (request_human withdraw and lifetime, the OAuth page's CSP, the registry's last good list, the failure notices) is stated where docs/design.md or docs/security.md already speak to it. task-180 and task-182 touch what a teammate can reach or what a browser loads: confirm their docs/security.md entries exist.
3. Write a QA checklist as a note on epic-14, one line per child, for George's pass on the Linux QA desktop with his xAI sign-in: what to do and what he should see.

Done when all three make targets pass on the branch with every slice in, and the checklist is on epic-14.
