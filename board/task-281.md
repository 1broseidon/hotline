---
id: task-281
title: "Team lead: a Hotline tool to add a teammate when the person asks for one"
column: backlog
position: 36
priority: medium
tags:
  - feature
  - team-lead
  - tools
createdAt: "2026-10-24T02:01:57.541Z"
---

## Description
George, 2026-10-23 20:59: the desk's team lead should be able to spawn new teammates through a Hotline tool when the person asks for it ("make me a researcher for X"), instead of the person going through New teammate.

Sketch, to settle before building:
- Lead only, like the other team-lead powers; a teammate that is not the lead does not get the tool.
- Arguments: name, goal, and optionally agent/model and folder; anything left out takes the room's defaults for new teammates (Settings > General > New teammates), as the New teammate pane does (wire personas a_new_teammate_takes_the_rooms_defaults).
- The person approves on a card showing name, goal, model and folder before it exists (the ask_question/approval card path), and can say yes by voice on a call.
- Only when the person asked: the lead does not add teammates on its own initiative.
- The new teammate shows in the rail (now with an Add teammate row at the list's foot, ui/full-height-pane mock) and the lead can message_teammate it straight away.

Done when: a lead asked for a teammate raises one approval card; Approve creates it with the given name and goal and room defaults, Decline creates nothing; a non-lead never sees the tool; a headless harness in crates/hotline-core/tests covers all three.
