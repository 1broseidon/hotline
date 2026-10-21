---
id: task-253
title: "Files written on a call reach the chat: each file, or one summary card when there are many"
column: in-progress
position: 0
description: "Task-206 manual QA finding from Pork Chop, 2026-10-21, real Grok Voice call 790a5312: fox.txt was written and spoken about with a closing note, but never attached to chat; crane.txt reached chat only because its clone chose send_file. The deck expects results in chat. George must decide whether core should attach files written by work_on_it or use another policy. Do not implement automatic attachment without that decision. Reproduce, settle scope, then prove chosen behavior on both drivers and a real call. Task-206 deliberately leaves this unwired."
createdAt: "2026-10-21T07:53:18.984Z"
landsOn: release/0.42.0
updatedAt: "2026-10-21T20:29:10.389Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T19:59:35.384Z"
  until: done
  tries: 2
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T20:28:55.791Z"
assignee: codex
decisions:
  - decision-31
relatedFiles:
  - crates/hotline-core/src/session/call_files.rs
  - crates/hotline-core/src/session/turns.rs
  - crates/hotline-core/src/session/files.rs
  - crates/hotline-core/src/driver/acp/translate.rs
  - crates/hotline-core/src/driver/rig/turn.rs
  - crates/hotline-store/src/sent.rs
  - crates/hotline-core/tests/call_threads/files.rs
  - ui/src/components/SentFile.tsx
  - ui/src/components/transcript/Bubbles.tsx
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
- 2026-10-21T20:00:16.988Z: [codex] Starting implementation on dispatch/task-253 using George's recorded policy: call-owned successful write/edit records, deduplicated send_file, individual attachments through three and a functional grouped card above three. Checking shared driver completion and file reach boundaries first.
- 2026-10-21T20:11:47.661Z: [codex] Implementation now preserves explicit mutation metadata from Rig and ACP, batches call attachments including explicit send_file, and adds indexed readback plus compact UI. UI checks pass. Full check hit host disk exhaustion; cargo clean -p hotline-core reclaimed this worktree's generated artifacts, rerunning without incremental compilation. Real-call QA credentials are not configured (HOTLINE_LIVE_XAI_KEY/OPENAI_KEY/CHATGPT_DIR); requested isolated QA setup rather than touching the running daily driver.
- 2026-10-21T20:22:16.303Z: [codex] Foreground make check now passes on the final code (CARGO_INCREMENTAL=0 after reclaiming this worktree's build cache). Both-driver wire matrix covers 0/1/3/6 files, explicit sends, refused writes, duplicate basenames and every indexed readback; UI and lease/reach/correction tests pass. Deduplication is per work result so a later correction is delivered. Running make precheck before commit; real-provider call QA remains pending because no explicit test credentials/QA endpoint were supplied.
- 2026-10-21T20:27:56.057Z: [codex] Final make check passes, including the functional in-app Open path for non-PDF files (stored-byte text/binary preview, paging, inert markup). Native opening remains restricted to images/PDFs. Initial make precheck passed all 8 legs; rerunning it with the added preview/paging UI tests, then committing on dispatch/task-253. Remaining manual step: a real-provider call against an isolated QA instance/test credentials.
- 2026-10-21T20:28:48.847Z: [codex] Committed 2f818157 on dispatch/task-253: core-owned per-work file delivery from successful Rig/ACP mutation records, deduplicated explicit sends, ≤3 attachments or one larger summary, indexed readback and functional text/binary previews. Foreground make check and make precheck (all 8 legs) pass; working tree is clean. Ready for code review. Remaining acceptance step: reproduce fox.txt and a six-file batch on a real-provider QA call; no isolated endpoint/test credentials were supplied here.
- 2026-10-21T20:28:55.286Z: Landing sent back (1 of 1): dispatch/task-253 conflicts with release/0.42.0 in docs/security.md. On release/0.42.0 since this branch started: c643f405 An agent saves the person's name only after their approval. (task-261). Merge release/0.42.0 into this branch (git merge release/0.42.0), keep what both sides meant, commit the merge, and run the checks.
- 2026-10-21T20:30:19.808Z: [codex] Merged release/0.42.0 for the landing retry; resolved the sole docs/security.md conflict by retaining both call-file delivery and approved person-name storage sections. Code auto-merges retain both modules and harnesses. Foreground make check running, then precheck and merge commit; real-provider QA remains the previously recorded outstanding step.
- 2026-10-21T20:33:40.176Z: [codex] Landing conflict resolved and committed as merge 0af2d908 on dispatch/task-253. Both security sections preserved; release/0.42.0 is an ancestor and worktree is clean. Foreground CARGO_INCREMENTAL=0 make check passed, including both-driver file matrix and name approvals; make precheck passed all 8 legs. Ready for review/landing retry. Remaining acceptance: isolated real-provider call QA for fox.txt and six-file batch, as previously recorded.
