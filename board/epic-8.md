---
id: epic-8
title: "Running processes: a server a teammate leaves going is shown, stoppable, and never holds the turn (0.39)"
type: epic
column: todo
position: 0
priority: high
tags:
  - processes
  - shell
  - voice
  - ui
createdAt: "2026-10-16T17:24:27.523Z"
landsOn: release/0.39.0
updatedAt: "2026-10-16T17:24:31.292Z"
---

## Description
A teammate often has to leave a process running: a local server so the person can open a page, a dev server, a watcher. Today a Hotline Agent's shell command is a job owned by the turn. The turn parks on it, a process that never exits keeps it "Working" forever, and on a call the front keeps saying "Still on it" (QA 2026-10-16: Goldie served a chart with `python3 -m http.server 8770`, said "the chart is open", and both GPT-Live and Grok calls stalled). This is a common voice flow: the person asks the teammate to fix something and open it in the browser.

Goal: a process the teammate means to leave running is a **running process**:
- It belongs to the teammate's session, not the turn, so the turn ends normally.
- It shows as a chip, as subagents do.
- It can be stopped from the chip, or by the teammate in a later turn.
- It never makes the chat or a call read as working.

Scope: the Hotline Agent's own shell (`session/jobs.rs`, `driver/rig/turn.rs`). ACP agents run their own shells and are out of scope. A phone mirror of the chip is a later hotline-mobile task.

Done when:
- A teammate can serve a page and open it, its turn ends, and the chip shows the server.
- Stop kills the process tree.
- A call on either front goes quiet after the reply instead of looping "Still on it".
