---
id: decision-25
title: Only the turn in flight at hang-up may automatically post its reply, and only when this call has…
type: decision
status: current
claimKey: voice.hangup-reply-post
kind: decision
stance: requires
subject: bounded completed-turn posts at hang-up
supersedes: decision-5
createdAt: "2026-10-21T14:15:33.798Z"
---

## Description
Only the turn in flight at hang-up may automatically post its reply, and only when this call has not already delivered a result or file to chat. A delivered result suppresses the late reply; the closing note accounts for it. Capture the running turn synchronously on detach. Replies over 900 characters or three paragraphs still use a bounded summary (600 characters), or a short fallback pointing to the full thread.
