---
id: task-260
title: "Legacy: the progress line varies instead of repeating 'I'm still working on that.'"
column: review
position: 1
priority: medium
tags:
  - voice
parentId: epic-21
createdAt: "2026-10-21T19:59:26.416Z"
dispatch:
  agent: codex
  model: gpt-6-astra
  by: cli
  at: "2026-10-21T19:59:35.624Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-21T19:59:38.896Z"
assignee: codex
updatedAt: "2026-10-21T20:13:22.598Z"
decisions:
  - decision-28
---

## Description
George, 2026-10-21: vary it. task-256 says 'I'm still working on that.' after each 20 quiet seconds of a tool-first Legacy turn, word for word. Done when: consecutive progress lines on one turn differ (a small set of short, first-person lines, never a hand-off phrase, e.g. 'Still on it.', 'Nearly there…' only if true is not knowable — keep them neutral), chosen without repeating the previous one; where the turn's latest tool is known, a line may name it plainly ('Still searching the web.'). Same 20 s cadence, still turn-checked. Tests: three lines on one turn are not all the same and none repeats its predecessor. Lands on release/0.42.0. Merge release/0.42.0 first, make check in the foreground, commit before done.

## Log
- 2026-10-21T20:00:22.635Z: [codex] Started on dispatch/task-260; release/0.42.0 is already contained in HEAD (no merge needed). Keeping feedback queue/cadence/turn guards; rotating neutral first-person lines and extending paused-clock coverage to three consecutive lines.
- 2026-10-21T20:09:14.756Z: [codex] Implemented neutral three-line rotation in the existing Legacy speech queue, preserving timing and guards. Unit and real-wire tests now verify three non-repeating lines (both drivers), with completion/hold silence retained. make check passed; make precheck running. Decision-28 updates the wording rule as requested.
- 2026-10-21T20:13:22.104Z: [codex] Committed e51a1940 on dispatch/task-260: neutral progress lines rotate without consecutive repeats; 20 s cadence and turn guards unchanged. Paused-clock and real-wire tests cover three lines on both drivers. make check and all eight make precheck legs passed (disk exhaustion resolved with cargo clean -p hotline-core). Design and decision-28 updated. Next: reviewer inspect and merge into release/0.42.0; no push or branch switch performed.
