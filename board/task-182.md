---
id: task-182
title: "OAuth callback page: a styled, dark page with a strict Content-Security-Policy"
column: todo
position: 8
priority: high
tags:
  - mcp
  - oauth
  - security
parentId: epic-14
createdAt: "2026-10-18T23:40:54.225Z"
relatedFiles:
  - crates/hotline-core/src/mcp/oauth.rs
updatedAt: "2026-10-19T00:33:55.864Z"
contract:
  status: ready
  deliverables:
    - type: test
      path: crates/hotline-core/src/mcp/oauth.rs
      description: the success and error pages carry the strict CSP header and no script
  validation:
    commands:
      - cargo fmt --all --check
      - cargo clippy --workspace --all-targets -- -D warnings
      - cargo test -p hotline-core
      - git diff --exit-code ui/src/generated/contract.ts
  metrics:
    readyAt: "2026-10-18T23:53:55.554Z"
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-19T00:33:55.688Z"
  until: done
  tries: 0
assignee: claude
---

## Description
Was subtask task-25-1. mcp/oauth.rs callback pages (around :785-804) are bare <html><body> with no CSP. Render a small templated page in the app's style (light and dark), saying 'Signed in, you can close this tab' or the error in a few words, served with a strict CSP (default-src 'none'; style-src 'unsafe-inline' or a hash) and no script. Test the headers and both outcomes.

Split from task-25, which keeps the rest and stays deferred.
