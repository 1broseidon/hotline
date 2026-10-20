---
id: task-214
title: "S0: the calls live in hotline-voice; hotline-core keeps only the room's side"
column: in-progress
position: 43
priority: high
tags:
  - crate-split
  - voice
assignee: codex
relatedFiles:
  - crates/hotline-core/src/voice
parentId: epic-21
createdAt: "2026-10-19T17:20:19.714Z"
updatedAt: "2026-10-20T21:08:06.183Z"
contract:
  status: in_progress
  deliverables:
    - type: file
      path: crates/hotline-voice/src
      description: the calls (call, duplex, fronts, dispatcher, turn, synthesis, replies, record, input) behind the approved interface
    - type: docs
      path: docs/adr/0007-split-hotline-core.md
      description: the calls' move done, each moved file's old and new path, and the timings re-run
  validation:
    commands:
      - "! test -e crates/hotline-core/src/voice/call.rs"
      - "! test -e crates/hotline-core/src/voice/duplex.rs"
      - "! test -e crates/hotline-core/src/voice/dispatcher.rs"
      - "! test -e crates/hotline-core/src/voice/front.rs"
      - "! test -e crates/hotline-core/src/voice/turn.rs"
      - "! test -e crates/hotline-core/src/voice/synthesis.rs"
      - "! grep -qE '^hotline-core[[:space:]]*=' crates/hotline-voice/Cargo.toml"
      - cargo test -p hotline-voice
      - cargo test -p hotline-core --test voice --test local_speech --test processes
      - make check
      - git diff --exit-code -- ui/src/generated/contract.ts
      - make precheck
  constraints:
    - Only the interface George approved in task-213's ADR section; no extra trait
    - "Moves only, no behaviour change: desk and phone calls, Grok Voice, GPT Realtime, GPT-Live and Legacy voice behave as before"
    - hotline-voice never depends on hotline-core or Tauri
    - ui/src/generated/contract.ts byte-identical after an unfiltered cargo test
    - Harness tests keep their assertions; only their import paths may follow the move
    - Run cargo test, make check and make precheck in the foreground and to completion before ending the turn
  metrics:
    readyAt: "2026-10-19T17:22:11.362Z"
    pickedUpAt: "2026-10-20T21:08:06.183Z"
    reworkCount: 0
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-20T21:07:47.448Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-20T21:07:48.457Z"
---

## Description
Scenarios: none directly. This is the boundary every later slice works inside. George, 2026-10-20: "extract voice into its own crate". This comes first, reversing ADR 0008 §5's "after slice 6".

Move the call code still in crates/hotline-core/src/voice into hotline-voice, behind the call-room interface sized in ADR 0008 §5: open and close a call thread, queue a heard line on it, take the agent's spoken stream back, post to the chat, be told of a card. That's call, duplex, front, front_prompt, front_tools, dispatcher (+ meter), record, replies, synthesis, turn and input, about 6.6k lines. Anything today's code needs beyond those methods stays on the interface until the slice that deletes it, each marked with that slice (S5, S6, task-203). It's a mechanical move with no behaviour change.

Done when:
- hotline-core has no voice module beyond the room's implementation of the call-room interface and the voice/connected.rs glue. hotline-voice does not depend on hotline-core (Cargo.toml).
- Each interface method that exists only for code a later slice deletes says which slice removes it.
- Desk and phone calls, the live voices (Grok Voice, GPT Realtime, GPT-Live) and Legacy voice behave exactly as before. tests/voice, tests/local_speech, the voice tests in tests/processes and the desk harness's call tests pass unchanged.
- make check, make precheck and workspace-isolation are green, and contract.ts is byte-identical.
- Core re-exports left with no callers are deleted (crate::voice::speech, crate::voice::realtime, crate::voice::front::SEND_TO_TEAMMATE, crate::models::is_custom, as task-212 found). Any that remain are listed in the notes.
- ADR 0007's build timings are re-run, and ADR 0008 §5 is amended to say the move came first.

## Log
- 2026-10-19T20:00:10.807Z: [claude] Re-homed to epic-21 (0.42): after slice 6, the calls move into hotline-voice. task-213's sizing is done by ADR 0008 §5 (open and close a call thread, queue a heard line, take the spoken stream back, post to the chat, be told of a card); task-213 is closed into this.
- 2026-10-20T21:10:56.312Z: [codex] Picked up contract on dispatch/task-214. Audited call dependencies: temporary seam must preserve core provider construction, command scopes/origins, leased front tools and recording callbacks; no behavioral changes intended. Removing obsolete compatibility re-exports first.
- 2026-10-20T21:20:35.203Z: [codex] Calls now compile in hotline-voice behind one CallRoom trait; core retains room implementation and connected glue. Voice crate tests (161 passed) and unchanged assertions in voice/local_speech/processes/desk harnesses passed. Full make check and precheck remain; docs/timings being updated. Temporary methods identify S5/S6/task-203.
