---
id: task-87
title: "Voice: a call never loops \"Still on it\" once the reply is said"
column: todo
position: 3
priority: high
tags:
  - processes
  - voice
  - front
parentId: epic-8
createdAt: "2026-10-16T17:24:29.145Z"
---

## Description
Voice: a call never loops "Still on it" once the reply is said.

- A background process is not work. With the core task in place, a turn that only leaves processes running ends, and the call goes to Listening as usual.
- Safety net for a turn that still parks on a foreground job:
  - Once the teammate's reply for the current hand-off has been said on the call, the bridge stops giving progress lines (`front.rs`, `waiting`) until the person hands over something new.
  - If the person asks "is something still pending?", the front answers from what it knows ("I'm done; there's a command still running"). It does not re-say a progress line.
- The desktop's call header follows the session's state, not the call's.

Tests:
- Scripted front: after a spoken reply, while the session stays Thinking on a parked job, no progress line arrives for 3 minutes of simulated time.
- After a new hand-off, progress lines resume.
