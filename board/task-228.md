---
id: task-228
title: "0.41 QA fixes: a job that ends by its count posts its closing note; the Jobs row's countdown ticks; the skip says the real time"
column: todo
position: 1
priority: high
tags:
  - jobs
  - qa
  - "0.41"
assignee: claude
relatedFiles:
  - crates/hotline-core/src/session
  - ui/src
createdAt: "2026-10-20T01:18:02.586Z"
---

## Description
Found in Pork Chop's 0.41 QA on the Linux QA computer (2026-10-20 01:05-01:20, release/0.41.0 at 58b47e16, Rex on Grok 4.7):

1. A job that ends by its count (or goal) doesn't post its closing summary to the chat, and its last firing doesn't know it is the last. "Count files every minute, end after 4 firings": firing 4 posted its change, then called cancel_schedule on itself, which failed ("12 steps · one failed") because the 4-firing end had already removed the job; its summary ("Counts across the four firings: 3, 4, 4, 5") stayed in the job's thread. ADR 0008 §7: a job that meets its end "cancels itself, with a final post saying what came of it". Fix: the firing's context says when it is the final one (the job ends after it; don't cancel it), and a job that ends by its count, date or goal posts its closing note through Room::post, stamped as the job's, as Cancel already does ("Cancelled. Where it stood: …").
2. The Jobs group's row shows "Next in Ns" frozen at the value it had when the list rendered (51s stayed 51s while the header's own "Next in" counted down to 32s). Make the row's countdown tick like the header's.
3. The overlap skip says "Skipped the Tue 01:13 run: the one before it was still going after 2m" on a 1-minute job, one minute after the firing began. Say the time the earlier firing had actually been running, and keep the suggested interval ("Every 2m would fit the time this takes") as it is.

Tests: a harness test for 1 (count end posts a closing note; the last firing's context says it is the last); a UI test for 2; a unit test for 3's wording.
