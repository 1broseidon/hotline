---
id: task-182
title: "OAuth callback page: a styled, dark page with a strict Content-Security-Policy"
column: review
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
updatedAt: "2026-10-19T00:41:19.859Z"
contract:
  status: delivered
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
    pickedUpAt: "2026-10-19T00:34:12.502Z"
    reworkCount: 0
    deliveredAt: "2026-10-19T00:41:19.859Z"
    duration: 427
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-19T00:33:55.688Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59290
    at: "2026-10-19T00:33:58.587Z"
assignee: claude
---

## Description
Was subtask task-25-1. mcp/oauth.rs callback pages (around :785-804) are bare <html><body> with no CSP. Render a small templated page in the app's style (light and dark), saying 'Signed in, you can close this tab' or the error in a few words, served with a strict CSP (default-src 'none'; style-src 'unsafe-inline' or a hash) and no script. Test the headers and both outcomes.

Split from task-25, which keeps the rest and stays deferred.

## Log
- 2026-10-19T00:41:23.557Z: [claude] Done on branch dispatch/task-182 (b7c2849b). mcp/oauth.rs: callback_page() renders one escaped card (heading + detail; 'Signed in' / 'You can close this tab.', or 'Sign-in failed' + the error) with inline styles copied from ui/src/tokens.css, dark by default and light via prefers-color-scheme (the browser tab cannot see the window's data-theme). Served with CALLBACK_CONTENT_SECURITY_POLICY = default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none', plus Cache-Control: no-store; no script. Tests: the_callback_pages_say_what_happened_under_a_strict_policy (real socket, 400 then 200, headers + no <script), callback_page_escapes_what_it_is_told. For later tasks: the page's colours are a copy of tokens.css, so a token change there does not reach it; a worktree build needs .speech-engine (make speech-engine).
