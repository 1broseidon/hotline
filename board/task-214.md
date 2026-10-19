---
id: task-214
title: The calls move into hotline-voice behind the approved interface
column: todo
position: 11
priority: high
tags:
  - crate-split
  - voice
assignee: claude
relatedFiles:
  - crates/hotline-core/src/voice
parentId: epic-19
createdAt: "2026-10-19T17:20:19.714Z"
dependsOn:
  - task-213
updatedAt: "2026-10-19T18:23:56.772Z"
contract:
  status: ready
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
---

## Description
Only after George approves the design (task-213). Move the call code into hotline-voice behind the interface it settles; the room implements it in the core, the voice harness tests and test rooms implement it in tests. hotline-core keeps no voice module beyond the glue that hands the room to the calls. Desk and phone calls, the live fronts (Grok Voice, GPT Realtime, GPT-Live) and Legacy voice behave exactly as before: the desk harness's call tests and processes/voice tests pass unchanged. Timings re-run into ADR 0007.

Lead's note (2026-10-19): this is the epic's last child and the one that proves it whole: the engines (task-212) and the calls live together in hotline-voice, the room implements the approved interface, and the full harness (tests/voice, tests/local_speech, tests/processes, make check, make precheck) passes on it. The lead may split it per task-213's answer before handing it off; if the design recommends stopping, it is not handed off and George decides its fate.

Carried from task-212: the core's glue is voice/connected.rs (it reads the vault into speech::Connected and keeps the five &Vault readers). Once the calls take a Connected, delete any of the core's re-exports that no longer have callers (crate::voice::speech, crate::voice::realtime, crate::voice::front::SEND_TO_TEAMMATE, crate::models::is_custom), and list in your notes the ones that remain. local_speech_check stays in the core because it needs Desk.
