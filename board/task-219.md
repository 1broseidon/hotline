---
id: task-219
title: "Slice 3: files and images from any thread land in the chat"
column: todo
position: 15
priority: medium
tags:
  - threads
  - chat
assignee: claude
relatedFiles:
  - crates/hotline-core/src/session/files.rs
parentId: epic-20
createdAt: "2026-10-19T19:59:32.080Z"
dependsOn:
  - task-216
updatedAt: "2026-10-19T19:59:34.546Z"
---

## Description
ADR 0008 §6 slice 3: send_file and generate_image from a work thread, a pair or a call post to the home thread through Room::post, stamped with where they came from; a quiet run's file is its post. Removes the quiet refusal and its kept-copy cleanup in session/files.rs.
