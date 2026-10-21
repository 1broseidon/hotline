---
id: task-260
title: "Legacy: the progress line varies instead of repeating 'I'm still working on that.'"
column: todo
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
updatedAt: "2026-10-21T19:59:38.897Z"
---

## Description
George, 2026-10-21: vary it. task-256 says 'I'm still working on that.' after each 20 quiet seconds of a tool-first Legacy turn, word for word. Done when: consecutive progress lines on one turn differ (a small set of short, first-person lines, never a hand-off phrase, e.g. 'Still on it.', 'Nearly there…' only if true is not knowable — keep them neutral), chosen without repeating the previous one; where the turn's latest tool is known, a line may name it plainly ('Still searching the web.'). Same 20 s cadence, still turn-checked. Tests: three lines on one turn are not all the same and none repeats its predecessor. Lands on release/0.42.0. Merge release/0.42.0 first, make check in the foreground, commit before done.
