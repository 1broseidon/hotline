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
updatedAt: "2026-10-16T19:52:07.216Z"
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-16T19:52:05.863Z"
  until: done
  role: lead
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 12851
    at: "2026-10-16T19:52:07.216Z"
assignee: claude
---

## Description
A teammate often has to leave a process running: a local server so the person can open a page, a dev server, a watcher. Today a Hotline Agent's shell command is a job owned by the turn. The turn parks on it, a process that never exits keeps it "Working" forever, and on a call the front keeps saying "Still on it" (QA 2026-10-16: Goldie served a chart with `python3 -m http.server 8770`, said "the chart is open", and both GPT-Live and Grok calls stalled). This is a common voice flow: the person asks the teammate to fix something and open it in the browser.

Goal: a shell job still running when the teammate has said its reply is a **running process**:
- The rule is deterministic and needs nothing from the model: when the turn parks (`driver/rig/turn.rs`, the model has replied and made no `wait_jobs` call) every shell job still running is handed to the teammate's session and the turn ends. A model that waits with `wait_jobs` is unchanged. There is no flag and no timer: a timer has no right value, and a flag is something the model forgets (Goldie did not know her server would never exit).
- It belongs to the session, not the turn, so the turn ends normally and everything that waits on a turn (chapters, schedules, queued lines, stop) proceeds.
- It shows as a chip, as subagents do, and can be stopped from the chip or by the teammate in a later turn.
- When it exits on its own, its result wakes the teammate once (the subagent precedent: the report wakes you), so a build the model did not wait on is still reported, and a server that dies is said. A stop from the chip does not wake it: the person did it.
- It never makes the chat or a call read as working.

Scope: the Hotline Agent's own shell (`session/jobs.rs`, `driver/rig/turn.rs`, `session/dm.rs`). ACP agents run their own shells and are out of scope. A phone mirror of the chip is a later hotline-mobile task.

Done when:
- A teammate can serve a page and open it, its turn ends the moment it replies, and the chip shows the server.
- Stop kills the process tree.
- A call on either front goes Listening after the reply instead of looping "Still on it".
- A process that exits on its own is reported by the teammate in one new turn.

## Log
- 2026-10-16T17:55:47.009Z: [claude] Design discussion: dropped `background: true` and port sniffing for one deterministic rule — a shell job still running when the turn parks is handed to the session and the turn ends; its exit wakes the teammate once (subagent precedent). A flag is forgotten, a timer has no right value. task-87's safety net contradicted task-85 and named code that does not exist; its one real proof moved into task-88, so task-87 should be deleted.
