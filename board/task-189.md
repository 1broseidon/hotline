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
contract:
  status: ready
  validation:
    commands:
      - make check
      - make verify
      - make precheck
  metrics:
    readyAt: "2026-10-18T23:53:55.554Z"
updatedAt: "2026-10-18T23:53:55.552Z"
dependsOn:
  - task-8
  - task-37
  - task-49
  - task-55
  - task-89
  - task-180
  - task-181
  - task-182
  - task-183
  - task-184
  - task-185
  - task-186
  - task-187
  - task-188
---

## Description
The last child of epic-14. Every other child has landed on release/0.40.0 by the time this starts. The slices were built apart, several of them in the same files (failure.rs wording from task-8, task-188 and task-181; wire/socket.rs from task-187 and task-55; the generated contract from task-180, task-185 and the failure kinds).

Do:
1. On a fresh fork of release/0.40.0, run `make check`, `make verify` and `make precheck`. Fix anything that broke where two slices met: a failure notice one slice reworded and another's test still expects, contract.ts not matching an unfiltered `cargo test -p hotline-core`, a socket-lane change from task-187 that task-55's backed-up-seat close doesn't cover. No new behaviour.
2. Check docs/ still describes what is: anything a slice changed (request_human withdraw and lifetime, the OAuth page's CSP, the registry's last good list, the failure notices) is stated where docs/design.md or docs/security.md already speak to it. task-180 and task-182 touch what a teammate can reach or what a browser loads: confirm their docs/security.md entries exist.
3. Write a QA checklist as a note on epic-14, one line per child, for George's pass on the Linux QA desktop with his xAI sign-in: what to do and what he should see.

Done when all three make targets pass on the branch with every slice in, and the checklist is on epic-14.

## Log
- 2026-10-19T00:15:53.353Z: [claude] Lead, from task-180: hotline-mobile labels a withdrawn card 'Expired' until it regenerates its contract. That's cosmetic, but put it in the QA checklist so George expects it. Also check that the docs/security.md row for withdraw_request exists.
- 2026-10-19T00:20:18.721Z: [claude] Lead, from task-183: two items for the QA checklist. Item 5, a link ask that was sent but never answered, now keeps the record and keeps dialing, and no wire test covers it, so check it by hand: link two desks, stop the other desk before it answers, then start it again. Item 2 differs from the brief: a failed room.unlinked keeps the dial loop retrying and shows an error, instead of clearing 'dialing'.
- 2026-10-19T00:46:12.356Z: [claude] Lead, from task-184: no test covers the peers.list fallback for an older desk, and its agent didn't run make check or verify. If a small UI test for the fallback fits beside the others, add it; otherwise put a hand check in the QA checklist: a teammate card against an older desk still lists its threads.
- 2026-10-19T00:49:44.125Z: [claude] Lead, from task-37: no test reproduces the real permission case, because that needs a build whose signature no longer matches its Speech Recognition grant, and only the call's 'Use desk model' offer is unit-tested; the composer's isn't. QA checklist, by hand on a Mac (the Linux desktop has no Apple speech engine): with the Mac's Speech Recognition refused, a call or dictation that hears 3 s of voice says 'No words heard', and 'Use desk model' appears only when the desk uses its own local model.
- 2026-10-19T00:57:58.046Z: [claude] Lead, from task-89: the Seatbelt profile is unchanged, and the fix waits on George's probe run. Put it in the QA checklist: Actions → ci → Run workflow, then read the shell-probe job on macos-26 and macos-15. If socket.getfqdn("127.0.0.1") is slow there, the 0.39.0 hang was a reverse-DNS lookup in http.server and not the sandbox. The ignored test developer_shims_run_in_both_shells must still compile under make check.
