---
id: task-206
title: "End to end, docs and the scenario pass: every scenario in the deck proved"
column: backlog
position: 41
priority: medium
tags:
  - tests
  - docs
assignee: codex
relatedFiles:
  - crates/hotline-core/tests
  - docs
parentId: epic-21
createdAt: "2026-10-19T13:59:39.590Z"
dependsOn:
  - task-204
  - task-207
  - task-203
  - task-239
  - task-240
  - task-241
updatedAt: "2026-10-21T07:50:07.295Z"
decisions:
  - decision-9
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T07:50:07.154Z"
  until: done
  tries: 0
---

## Description
The proof that the deck is true, and the docs.

Done when:
- Each of the deck's scenarios has a test, or a recorded manual check where audio, the network or the phone can't be scripted. Each test's doc comment names its scenario.
- Scenario pass: Pork Chop runs every scenario on the Linux QA computer, then George on the desk and the phone. Every "What must be true" line is checked off or filed.
- One wire-level harness proof: make a teammate lead, call the desk, ask a roster question (answered on the call), then give a task for a named teammate (handed straight to it, no dispatcher hop). The single-teammate auto-lead and clearing the lead on removal are covered.
- docs/voice.md and docs/voice-calls.md describe what shipped: a call is a thread, the closing note, the work tool, cards by voice, drops and fallbacks. ADR 0005 is marked amended, and ADR 0008 §5 records the 2026-10-20 decisions. The CHANGELOG has its lines. docs/design/voice-scenarios.html stays as the acceptance set.

## Log
- 2026-10-21T01:42:49.835Z: [codex] task-244 adds a concrete 'Following up in the chat' scenario-pass check to docs/design/voice-scenarios.html: say 'Bramble, a wiry terrier' only on the call, then ask from home chat to use it exactly that way; verify read_thread retrieves the detail on both drivers, and Open the call shows each tool inline. Deterministic both-driver proof is tests/call_threads/reader.rs::the_home_agent_reads_the_call_transcript_to_recover_a_detail_only_said_there; UI proof is said-aloud.test.tsx. Keep the live Pork Chop/Linux + George desk/phone check in task-206's pass; this task does not claim that manual pass.
- 2026-10-21T05:16:31.915Z: [codex] task-240 handoff for Interrupting: manually test speech onset and tap during a long reply on desk (browser audio and native Mac recognition) and phone, for Grok/GPT Realtime/GPT-Live and Legacy. On speakers at ordinary/loud volumes, verify no self-transcription or self-barge, including double-talk; check cut notice and next reply answers the interruption. Desk uses getUserMedia AEC and native voice processing, whose cross-process webview echo coverage remains unverified. Phone client is not in task-240's worktree: confirm it keeps capture open during playback, stops locally at onset/tap, sends voice.interrupt or voice.front_barge with playedMs, and rejects late cut audio. Fix any failed device/provider case before treating this scenario as proved.
- 2026-10-21T05:32:54.550Z: [codex-task-225] S6 hand-check handoff from task-225: exercise real Grok/GPT Realtime/GPT-Live calls for incomplete dictation (draft…, say…, write…), varied first-person acknowledgments, correction read-back, dropping work, and exactly one chat pointer after an actual post. GPT Realtime now requests semantic_vad with low eagerness; Grok/GPT-Live rely on their native turn detection plus remain-silent/listening instructions, so real pause behavior is not claimed by scripted tests. Code is on dispatch/task-225 pending its review.
- 2026-10-21T06:57:34.783Z: [claude] 2026-10-21 Pork Chop QA on a real desk call (61d35a97): asked 'Who is on the team?', the lead answered from list_teammates but listed Rex's model, tools and machine reach — a roster answer should be names and what they're on, short. Named work ('Have Rex write…') went straight to Rex: 'Rex has it.', rain.txt written.
- 2026-10-21T07:21:50.668Z: [claude] 2026-10-21 Pork Chop QA (790a5312, real Grok Voice call): 'Write a four line poem about a fox and save it as fox.txt' — written, spoken, closing note fine, but fox.txt never reached the chat as a file. Earlier crane.txt did, because that clone chose send_file. Whether a file made on a call reaches the chat is still the clone's choice, not code's. Deck intent is that results reach the chat; consider having the core attach files written by work_on_it.
- 2026-10-21T07:50:06.144Z: [claude] 2026-10-21 Pork Chop, scope for the Codex run: do the scripted part — a test per deck scenario (doc comment names it) or a recorded manual-check line where audio/network/phone can't be scripted; the wire-level desk-lead proof; docs/voice.md, docs/voice-calls.md, ADR 0005 amended, ADR 0008 §5 with the 2026-10-20 decisions (live means live: no silent Legacy fallback, Legacy only offered), CHANGELOG lines. Also fix the roster nit: a roster question on a desk call should answer names and what each is on, short, not model/tools/machine reach. Do NOT build auto-attaching files written on a call (that's George's call; file it as a new task). The real-audio scenario pass is Pork Chop + George, not this run. Base is a180a43d (includes cards wording fixes).
