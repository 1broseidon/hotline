---
id: epic-20
title: "One agent thread layer: every modality attaches to it, and the chat is the log of truth"
type: epic
column: todo
position: 11
priority: high
tags:
  - architecture
  - threads
  - voice
  - "0.41"
assignee: claude
createdAt: "2026-10-19T17:32:26.752Z"
landsOn: release/0.41.0
updatedAt: "2026-10-19T20:00:46.124Z"
dispatch:
  agent: claude
  model: opus
  effort: high
  by: cli
  at: "2026-10-19T20:00:43.857Z"
  until: done
  role: lead
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-19T20:00:46.124Z"
---

## Description
George, 2026-10-19 12:20-12:31 (phone):
- Agent = thread. A text DM, a side thread, a subagent, a scheduled or quiet run, a pair thread, a voice call: each is the same agent thread with a different modality attached. One central agent-thread layer that any modality attaches to, rather than a separate path per feature.
- Voice is a modality, not a separate agent: on a call the teammate itself is the full agent, with its tools, with speech in and speech out (see epic-19 task-213).
- Escalation everywhere. Today only a quiet scheduled run on a Hotline Agent can post to the person, through the Escalate trait and its tell_person tool (driver.rs, BRO-96, one message per run). Adopt it, renamed if a better name fits, as the one way any thread posts to the main chat, on every driver and every modality.
- The person always falls back to the chat as the log of truth, and the system is built that way. On a call, a doc, a link, a file or a summary the agent produces is sent to the chat through that primitive, not read out.

Where the code is today: ThreadId already spans the DM, side threads, runs, calls and pair threads (hotline-contract), and session/threads.rs writes them; but each kind still has its own session path, the voice fronts are separate agents (ADR 0005), and escalation exists only for quiet runs.

First the architecture, for George to approve; then the slices, planned from it.

## Log
- 2026-10-19T19:59:30.117Z: [claude] George 14:58: build ADR 0008 in two releases. 0.41 = waves 1 and 2 (the chat door, pairs, artifacts, jobs as scheduled clones, clone permissions; plus epic-17's team lead core and UI), then Pork Chop's own QA on the Linux QA computer, then George QA's the desk. 0.42 = the voice path (epic-21). Lead: only this epic's children land on release/0.41.0.
- 2026-10-19T20:03:23.647Z: [claude] Lead's plan (claude/opus, 2026-10-19). The order is set by shared code, on release/0.41.0. (1) task-216 (Room::post: session.rs, prompt.rs, dm.rs, escalation.rs, contract) and task-218 (pairs: peers/, runner.rs, policy.rs) run together, since they share almost no files and each contract says to stay out of the other's. (2) task-217 waits on both: post_to_chat's gate reads the thread kind, and a pair's kind should be wired once through thread_agent, not in peer_session's old builder. (3) task-219 waits on 217: a quiet run's file counts as that turn's one post under 217's gate. (4) task-220 waits on 217, 218 and 219: it reshapes runner.rs and removes the quiet path that files.rs and dm.rs touch. Its desktop Jobs group is split off as task-226 (UI only, after 220). (5) task-221 (clone lease) and task-226 run together after 220: core vs ui/. (6) task-222 is the wiring and proof, last: one headless harness for the whole epic, then make check/precheck/verify, CI, docs, then Pork Chop's QA. It also waits on epic-17's task-202 and task-205, which are outside this epic: someone else must land them. Every child now has validation commands: make check, plus greps that the removed code is gone. Agents: 216 claude/opus (contract and tape shape), 218 codex/gpt-6.1-sol (mechanical refactor), 217 and 221 claude/opus (security), 219 codex, 220 claude/opus, 226 claude/sonnet, 222 claude/opus.
- 2026-10-19T20:54:40.816Z: [claude] Watch, 16:20: three children so far ended their turn while make check ran in the background, leaving work uncommitted, and a landing's make check stops after 15 minutes. For every hand-off from here, the lead's note should say: run make check in the foreground and wait for it, commit before ending the turn, and merge release/0.41.0 into the branch and re-run make check before saying done, so the landing's check is incremental.
