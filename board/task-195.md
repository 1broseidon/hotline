---
id: task-195
title: "hotline-websearch: web search as its own crate"
column: todo
position: 3
priority: medium
tags:
  - crate-split
assignee: claude
relatedFiles:
  - crates/hotline-core/src/websearch.rs
  - crates/hotline-core/src/websearch
parentId: epic-16
createdAt: "2026-10-19T06:06:59.331Z"
dependsOn:
  - task-194
updatedAt: "2026-10-19T06:07:11.243Z"
---

## Description
websearch (about 6,200 lines) reaches only contract. Move it to crates/hotline-websearch depending on hotline-contract; its HTTP and provider dependencies move with it and leave hotline-core's Cargo.toml if nothing else uses them (cargo machete agrees).
