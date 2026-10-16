---
id: task-84
title: "GPT-Live: the responses delegation fails validation, so a tool call goes silent"
column: todo
position: 0
priority: high
tags:
  - voice
  - front
  - gpt-live
  - bug
contract:
  status: ready
  deliverables:
    - type: file
      path: crates/hotline-core/src/voice/realtime/live.rs
      description: Valid responses delegation, and a client-delegation fallback
    - type: test
      path: crates/hotline-core/src/voice/realtime/live/tests.rs
      description: Schemas are cleaned; validation failure falls back
  validation:
    commands:
      - make check
  constraints:
    - "AGENTS.md rules: commit messages are one line stating the invariant; new behaviour needs a headless harness; doc drift is a bug"
    - "Front refusals log the code, param and a clipped message only: never the transcript or a credential"
    - Work on release/0.39.0; do not touch main
  metrics:
    readyAt: "2026-10-16T15:57:47.298Z"
createdAt: "2026-10-16T15:57:47.304Z"
updatedAt: "2026-10-16T15:58:36.169Z"
landsOn: release/0.39.0
---

## Description
George QA'd 2026-10-16 10:55 on the dev desk, twice: Grok Voice runs the desk-side tools flawlessly; GPT-Live acknowledges ('Okay, I'm on it') and then goes silent. The dev desk log (~/Projects/personal/hotline/.hotline-dev/logs/hotline.log) shows, per call: '[voice] the front refused a request: Some("responses_validation_failed")' then 'Some("invalid_request_error")'.

What is known: task-79's tool replies match OpenAI's guide exactly (https://developers.openai.com/api/docs/guides/live-delegation: response.item.create with a function_call_output item, then a bodiless response.create), so the reply shape is not the fault. responses_validation_failed means the session's delegation.responses settings are invalid; validation runs asynchronously after the call is placed, the session stays open without rollback, and settings can be fixed with session.update. The invalid_request_error that follows is the backend refusing to run with them. The ChatGPT sign-in route does accept 'responses' delegation (the call placed).

Find which setting fails (voice/realtime/live.rs delegation()): (1) BACKEND_MODEL 'gpt-6-luna' may not be served on the ChatGPT sign-in route (try the models the Codex app lists for the account, or gpt-6-sol); (2) a function's parameters may not be a schema Responses accepts (the MCP/rmcp input_schema can carry $schema, title, definitions, or nullable type arrays; strip to type/properties/required/items/enum/description); (3) descriptions or names out of bounds. First make the error say which: log the error event's 'param' and a clipped 'message' for front refusals (no secrets or transcript), then fix. If validation still fails mid-call, fall back for that call: place a replacement session with client delegation through the renewal machinery (a mode cannot be switched on a running session: immutable_field_update) so tools run through the desk-side loop instead of going silent, and the person hears that the front keeps going. Prove it against the real service with the ignored live test (HOTLINE_TEST_CHATGPT_DIR) asking for a web search and a thread search.
