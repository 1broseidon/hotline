---
id: task-253
title: Decide how files written on a call reach the chat
column: backlog
position: 39
description: "Task-206 manual QA finding from Pork Chop, 2026-10-21, real Grok Voice call 790a5312: fox.txt was written and spoken about with a closing note, but never attached to chat; crane.txt reached chat only because its clone chose send_file. The deck expects results in chat. George must decide whether core should attach files written by work_on_it or use another policy. Do not implement automatic attachment without that decision. Reproduce, settle scope, then prove chosen behavior on both drivers and a real call. Task-206 deliberately leaves this unwired."
createdAt: "2026-10-21T07:53:18.984Z"
---

## Description
Task-206 manual QA finding from Pork Chop, 2026-10-21, real Grok Voice call 790a5312: fox.txt was written and spoken about with a closing note, but never attached to chat; crane.txt reached chat only because its clone chose send_file. The deck expects results in chat. George must decide whether core should attach files written by work_on_it or use another policy. Do not implement automatic attachment without that decision. Reproduce, settle scope, then prove chosen behavior on both drivers and a real call. Task-206 deliberately leaves this unwired.

## Log
- 2026-10-21T19:59:00.665Z: [claude] 2026-10-21 14:58 George's decision: yes — a file written on a call is posted to the chat by the core, not left to the model. If there are many, post one summary card (or a compact visual list) of the files edited instead of one post per file, so it doesn't spam the chat.

Done when:
- Files the call's work wrote or edited are posted to the home chat stamped 'From the call', whether or not the clone called send_file; never twice when it did.
- Up to 3 files: each posted as a file attachment. More than 3: one card listing every file (name, created/edited, size), each openable/savable from the card, instead of separate posts.
- Files are taken from the work's own write/edit tool records on the call thread (not a directory scan), and only from this call.
- Tests: one file → one attachment; send_file already used → no repeat; six files → one summary card listing six; nothing written → nothing posted.
Lands on release/0.42.0. Merge release/0.42.0 first, make check in the foreground, commit before done.
