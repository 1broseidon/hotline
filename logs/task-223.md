---
id: task-223
title: "S5: a call is a thread: hang-up detaches, it goes on as text, a call starts on any thread, one closing note"
priority: high
tags:
  - voice
  - threads
assignee: codex
relatedFiles:
  - crates/hotline-core/src/voice
  - crates/hotline-core/src/session
parentId: epic-21
createdAt: "2026-10-19T20:00:06.937Z"
dependsOn:
  - task-214
updatedAt: "2026-10-20T22:47:16.818Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-20T22:09:07.408Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-20T22:09:08.362Z"
    ended: "2026-10-20T22:47:16.818Z"
    outcome: done
completedAt: "2026-10-20T22:47:16.432Z"
---

## Description
Scenarios: Hanging up while work runs, Picking it back up, Calling on a thread already under way, Following up in the chat. Also the closing note in every scenario's "In the chat afterwards". ADR 0008 §5 and §6 slice 5. Live voices keep running as today until S6.

Done when:
- A call is a thread whose agent is the teammate: the Call row per ADR 0008 §5, built by Room::thread_agent and driven by Room::run_queue. On Legacy voice each utterance is a line on the call's thread, never the DM.
- Hanging up takes the voice away and stops nothing. A turn in flight at hang-up finishes, and its reply is posted to the chat instead of said.
- The closing note comes 30 seconds after hang-up once no turn is in flight, so it covers finished work. Then the thread parks, still open.
- The closing note is a summary of what was discussed, decided and made, never a transcript ("you said, I said"). It opens the call's thread, where the transcript lives. A call that only chatted gets a one-line note. The five-minute summaries and the front's Digest (voice/record.rs) are gone.
- After hang-up the thread keeps its agent and its whole context. Typing in it is talking to the same agent. Calling from it attaches voice again and starts nothing from scratch. The next closing note covers only what is new since the last one.
- A call can be placed on any thread under way, a work thread or an earlier call. Voice attaches to that thread and opens no new call thread. Its posts read "From <thread title>", not "From the call". If the thread's agent is mid-turn, a spoken line waits or steers as a typed one would.
- While the call is up, a post from it neither lights the rail nor notifies. Once it is down, a post does both, as any reply does.
- A reply on a call's post goes to that call's thread, where the context is.
- A new line in the chat goes to the home agent. Before its next line it has been handed the call's posts and closing note, and it opens the call's thread with the thread readers when it needs more. It never answers "I don't know what it is" when the call knows.
- Every post from a call names what it is about and reads right to someone who wasn't on the call. The post_to_chat description and the closing-note summariser's prompt both say so.

Proved by desk-harness tests, one per line above, on both drivers (Hotline Agent and an ACP teammate), named for the scenario they prove.

## Log
- 2026-10-20T22:13:48.369Z: [codex] Implementation underway on dispatch/task-223: sharing the work-thread occupant for Call policy and queue/restoration, routing Legacy input to the thread, and replacing front Digest/periodic summaries with a delayed room closing note. Live-front work delegation remains the S6 path. Validating build and lifecycle races next.
- 2026-10-20T22:33:33.031Z: [codex] Committed S5 as af53f2e2 on dispatch/task-223. Twelve call-thread scenarios passed on both drivers, plus the UI notification regression; a thirteenth covers answering a parked call's human card in the full run. make check and make precheck are running; precheck has passed formatting, lockfile, workflows, citations, contract and clippy and is cross-checking Windows.
- 2026-10-20T22:47:02.492Z: [codex] S5 delivered on dispatch/task-223 through 3d3f561a; worktree clean. Legacy calls use thread_agent/run_queue, detach without stopping work, resume context in text or voice, attach to existing threads, and leave one delayed interval summary. Posts link to their source, stay quiet during voice, and feed home-agent context; Digest/five-minute summaries are removed. Thirteen new desk scenarios pass on both drivers, with updated voice integration and notification proofs. make check passed (2,150 Rust tests, 465 UI tests plus Python/type/lint/dependency gates); make precheck passed all eight legs including Windows. S6 remains intentionally unwired: live-front DM delegation, unanswered-question routing, and CallRoom start/prompt/info/voice_spoken/front_reach retain their current behavior; the closing-note wait follows the thread occupant, not transitional front DM work.
- 2026-10-20T22:47:15.790Z: Landed on release/0.42.0
