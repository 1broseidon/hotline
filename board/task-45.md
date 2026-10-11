---
id: task-45
title: "Desktop: / picker anywhere in the sentence"
column: todo
position: 48
priority: high
tags:
  - composer
  - ui
  - slash
  - 0.37.0
relatedFiles:
  - ui/src
parentId: epic-4
createdAt: "2026-10-11T21:25:35.454Z"
---

## Description
/ at any position opens the catalogue (Hotline commands, skills, agent commands). Decide and implement how a mid-sentence command combines with the message (e.g. 'tidy this up and /new chapter after' runs the command after the message sends). A / that isn't a known command (paths, and/or, URLs) stays plain text. Replaces BRO-136's 'only at the start' rule.
