---
id: task-242
title: "S5 follow-up: a parked call's typed turns stay in its thread, a long reply at hang-up posts as a summary, and untitled posts read \"From the call\""
column: in-progress
position: 0
priority: high
tags:
  - voice
  - bug
parentId: epic-21
dependsOn:
  - task-223
createdAt: "2026-10-20T23:05:51.298Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-20T23:06:04.226Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-20T23:06:05.653Z"
assignee: codex
updatedAt: "2026-10-20T23:06:27.281Z"
---

## Description
Found in Pork Chop's proving-call pass on the Linux QA desk, 2026-10-20, with a real Legacy voice call to Rex (Hotline Agent, Grok 4.7), on release/0.42.0 at b66bd710. Scenarios: Hanging up while work runs, Picking it back up (deck: docs/design/voice-scenarios.html).

What happened:
1. I typed "Make the lighthouse poem rhyme" in the parked call thread. Rex's reply ("Changed lighthouse.txt. It now rhymes in couplets.") was also posted to the main chat as "From Harbor poem and Eddystone history files", though I was reading it in the thread. ADR 0008 §4 says a call's talk stays on its own thread.
2. I hung up while Rex was writing a five-paragraph Eddystone history. Its reply was posted to the main chat in full, as five long "From Call" bubbles. That is the "blob" George asked to avoid (George's point 3), in another form.
3. Before the thread had a title, its posts read "From Call". The deck says "From the call".

Done when:
- A turn typed or spoken in a call's thread that was not in flight at hang-up leaves its reply in the thread. The chat gets nothing from it except a post_to_chat post or the closing note.
- A reply in flight at hang-up is posted as one message. When its written version is longer than a few short paragraphs, the post is a short summary that stands on its own and names its subject, with "Open the call" leading to the full text in the call's thread. The full text is never pasted into the chat.
- A post from a call's thread that has no title yet reads "From the call", never "From Call".

Proved by desk-harness tests, one per line, on both drivers, named for the scenario.

## Log
- 2026-10-20T23:09:50.706Z: [codex] Traced the leak to Told::write posting every segment whenever a call thread was disconnected. Replacing it with one completed-turn post, marked synchronously at detach (so delayed call-log writes cannot catch a later typed turn); long replies use the existing no-tool chapter summariser with a bounded fallback. Added both-driver desk scenarios and source-link UI proof.
