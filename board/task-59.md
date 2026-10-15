---
id: task-59
title: "Desk: front-to-teammate bridge"
column: review
position: 0
priority: high
tags:
  - voice
  - calls
  - 0.38.0
parentId: epic-6
dependsOn:
  - task-58
createdAt: "2026-10-14T20:03:56.919Z"
relatedFiles:
  - crates/hotline-core/src/voice/front.rs
  - crates/hotline-core/src/voice/mod.rs
  - crates/hotline-core/src/voice/realtime/mod.rs
  - crates/hotline-core/src/voice/spoken.rs
  - crates/hotline-core/tests/voice.rs
updatedAt: "2026-10-15T02:22:31.505Z"
contract:
  status: failed
  deliverables:
    - type: file
      path: crates/hotline-core/src/voice/front.rs
      description: Bridge between front session and teammate session
    - type: test
      path: crates/hotline-core/tests/voice.rs
      description: "Headless harness: front hands words to teammate unrephrased, spoken line returns verbatim"
  validation:
    commands:
      - cargo fmt --all --check
      - cargo clippy -p hotline-core --all-targets -- -D warnings
      - cargo test -p hotline-core
  constraints:
    - Same path, grants and chapter as a typed message; voice origin kept
    - Front transcript stays out of the teammate tape
    - Extend tests/voice.rs; run unfiltered
  metrics:
    readyAt: "2026-10-15T01:57:04.232Z"
    pickedUpAt: "2026-10-15T02:10:40.649Z"
    reworkCount: 0
    deliveredAt: "2026-10-15T02:22:24.470Z"
    duration: 704
  feedback: "crates/hotline-core/src/voice/front.rs (/Users/george/Projects/personal/hotline/crates/hotline-core/src/voice/front.rs): File not found"
assignee: claude
dispatch:
  agent: claude
  model: sonnet
  effort: medium
  by: lead
  at: "2026-10-15T02:10:23.780Z"
---

## Description
Front tools: send_to_teammate(words) passes the person's own words, unrephrased, into the teammate's session as start-or-steer in its open chapter (same path, grants and chapter as a typed message; voice origin kept); remain_silent. Teammate side: start/end fragments like Codex's realtime_start/end ('behind an intermediary; input is a transcript and may have recognition errors'; 'call ended, resume normal behaviour'). Back to the front: short progress notes while the turn runs, the reply's <spoken> line spoken verbatim (inject as text the front must say, or client-side speech), approvals/questions become 'it needs you to approve on the desk/phone'. A second send while a turn runs steers it. Front never reports work the teammate did not.

## Log
- 2026-10-15T02:22:31.269Z: [claude] Done on branch dispatch/task-59 (4c792686). voice/front.rs: Front::open(calls, call_id), tools() (send_to_teammate, remain_silent), tool_call(name, args)->String output for the realtime tool_result, next_note()->Note::{Spoken,Progress,NeedsYou} with instruction() text ("say exactly this"). Relay = Calls::relay -> the call's text-input path (Work::Text with TranscriptSource::Front; Origin gained `front`, which makes spoken::voice_turn add FRONT_NOTE 'behind an intermediary... recognition errors' before the contract). Notes: (1) Front is built on the call's own events (Said is cumulative per line, so Front gives out only the new part; Thinking+listening repeats every 15s = Progress; Card = needs-you), and does NOT depend on voice/realtime (task-58 is not merged into this branch) - it uses plain Tool structs, the next task maps them to RealtimeTool. (2) The front call must be started with VoiceInputMode::Text and a target (start_with_input); the call-wire task decides how a front call is requested. (3) No END fragment is injected: each relayed turn carries the note, so a turn without it is already 'resume normal behaviour' (an end line would need its own turn). (4) A relayed 'goodbye' does not hang up; hanging up is the front's/wire's call. (5) Harness: tests/voice.rs a_front_hands_words_to_the_teammate_and_says_its_spoken_line_verbatim. Full cargo test -p hotline-core, fmt and clippy pass (build needs SHERPA_ONNX_ARCHIVE_DIR pointing at a .speech-engine dir).
- 2026-10-15T02:22:35.670Z: [claude] contract validate reported failed only because it resolves deliverable paths in the main checkout (/Users/george/Projects/personal/hotline), where this branch's front.rs is not merged yet; the file is on dispatch/task-59 and the validation commands (fmt, clippy -D warnings, cargo test -p hotline-core unfiltered) were run by hand in the worktree and pass.
