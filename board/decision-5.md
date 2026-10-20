---
id: decision-5
title: Only the turn in flight at hang-up automatically posts its completed reply
type: decision
status: current
claimKey: voice.hangup-reply-post
kind: decision
stance: requires
subject: bounded completed-turn posts at hang-up
createdAt: "2026-10-20T23:11:32.955Z"
---

## Description
Capture the running turn synchronously when voice detaches, not when the asynchronous call log catches up. Post its complete written reply once; later typed or spoken turns stay on their thread unless explicitly posted or included in the closing note. A reply exceeding 900 characters or three paragraphs is summarised into one subject-first paragraph of at most 600 characters. If the summariser fails or exceeds its bound, post a bounded notice linking to the full thread, never the long reply. This implements task-242's no-blob requirement on both drivers.
