---
id: task-244
title: "The call's thread is its transcript: any agent can read a thread whole, and the home chat acts on \"like we said on the call\""
column: review
position: 0
priority: high
tags:
  - voice
  - threads
parentId: epic-21
dependsOn:
  - task-242
createdAt: "2026-10-21T01:32:33.014Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T01:32:43.986Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T01:32:46.063Z"
assignee: codex
updatedAt: "2026-10-21T01:54:41.275Z"
---

## Description
George, 2026-10-20 at 20:28, after his proving call on the desk: "the bones are there". Before S6:
1. "We will almost never follow up in the side thread. The call thread should display transcripts and tool calls if any."
2. "The user will almost always want to continue from the home thread, so saying in text 'like we said on the call, implement it exactly that way', the home thread should have access to that thread to find what 'that way' means."

What happened: Goldie (Haiku 5.5) was asked about the call in the home chat. It ran search_thread on call:bcc51703… and found the call's lines, your request and its replies. That is the whole transcript, since the call was on Legacy voice. Goldie still answered that it had "no exact transcript" and that one "would have to come from Hotline's call record". Agents don't know that a call's thread IS its transcript, and they have no tool that reads a thread in full: search_thread returns matching snippets. ADR 0008 §0 promised "the thread readers"; only search_thread, list_chapters and resume_chapter exist.

Done when:
- An agent can read any thread of its own in full, in order, with a thread reader tool on Hotline's MCP server, for both drivers and under the thread's lease: a call, a work thread or a job, by the key a post or search hit names. Long threads page.
- A call's thread reads as its transcript: who said what, in order, with each tool call as its own line. The tool's description, search_thread's hits from a call and the posts handed to the home agent all say that a call's thread is the call's transcript, and name its key.
- When the person says in the home chat "like we said on the call, do it that way", the home agent reads the call's thread and acts on what was said, without asking what was meant. A desk-harness test on both drivers puts a detail only in the call's thread ("Bramble, a wiry terrier") and asks for it from the home chat. It is answered from a thread read, not a search snippet.
- In the window, a call's thread opens as its transcript. What was said on each side is shown in order, and every tool call is shown inline (name and one line of what it did), not folded behind "N steps". The composer stays, but the view is built for reading.

Proved by the harness tests above, a UI test for the inline tool calls, and a check in the scenario pass (task-206).

## Log
- 2026-10-21T01:37:34.969Z: [codex] Implemented read_thread through the shared lease-checked MCP handler, own-thread ownership checks and ordered event-id pagination; call search hits and home-agent posts identify the transcript. Wiring inline call tools and both-driver harness proofs now. Work stays on dispatch/task-244.
- 2026-10-21T01:46:29.506Z: [codex] Committed ffde536c on dispatch/task-244. The 21 call-thread desk scenarios pass on both drivers; UI transcript proof, full-text/fold paging, cross-teammate refusal and both-transport lease revocation pass. Final make check is through native clippy and core unit tests and running harnesses; make precheck started. Live scenario-pass check handed to task-206; S6 live-front routing remains outside this task.
- 2026-10-21T01:54:40.985Z: [codex] Ready for review on dispatch/task-244: ffde536c and 921111c0, worktree clean. read_thread pages full ordered own-thread records through both drivers under the caller's lease; call hits/posts identify the transcript and key; call UI names both speakers and shows every tool inline. 21 call-thread scenarios pass, including Bramble recovery from a full read and work/job reads; UI, paging/folding, ownership and revocation proofs pass. Final make check and all 8 make precheck legs passed. An earlier unrelated process-chip timeout passed its isolated rerun and subsequent full runs. Nothing in task-244 left unwired; live-front routing remains S6, and task-206 has the explicit live Linux/desk/phone scenario check (not claimed run here).
