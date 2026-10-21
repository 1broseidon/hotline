---
id: decision-14
title: The core permits a contextual chat pointer only for a concrete written result, at most once in…
type: decision
status: current
claimKey: voice.chat-pointer
kind: decision
recheck: Revisit thresholds if the scenario pass shows missed useful results or excessive reminders.
createdAt: "2026-10-21T07:04:30.902Z"
---

## Description
The core permits a contextual chat pointer only for a concrete written result, at most once in three caller turns.
Code, tables, links, sent files, lists longer than three items and write-ups over 120 words qualify. Conversation keeps its spoken words as the written record. A pointer reservation counts even if generation or playback is interrupted, because generated speech is not proof of hearing; the model supplies wording, not eligibility. Automatic result posts retain the call's own persona/thread and the existing hang-up rule.
