---
id: task-43
title: "Desk: @mentions reach the teammate resolved to the teammate they name"
column: todo
position: 46
priority: high
tags:
  - composer
  - mentions
  - 0.37.0
parentId: epic-4
createdAt: "2026-10-11T21:25:34.873Z"
---

## Description
When a sent message carries mention tokens, the desk delivers them with the teammate ids they name (and their public names), so 'ask @The Dude to do this' lets the receiving teammate call message_teammate without guessing from prose. Names with spaces, duplicates across linked desks later. Old clients sending plain '@Name' text keep working.
