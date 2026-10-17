---
id: task-112
title: Colleagues see each other's goal, agent, model and capabilities (computer, web search, images, background work, MCP server names)
column: review
position: 0
priority: high
tags:
  - teammates
  - security
  - mcp
createdAt: "2026-10-17T22:24:05.896Z"
landsOn: release/0.40.0
updatedAt: "2026-10-17T23:11:31.704Z"
parentId: epic-13
relatedFiles:
  - crates/hotline-core/src/mcp/server.rs
  - crates/hotline-core/src/linked.rs
  - crates/hotline-core/src/desk.rs
  - crates/hotline-core/skills/hotline-room/SKILL.md
  - docs/security.md
  - ui/src/generated/contract.ts
contract:
  status: delivered
  validation:
    commands:
      - make precheck
      - make check
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-17T22:49:12.409Z"
    reworkCount: 0
    deliveredAt: "2026-10-17T23:11:31.704Z"
    duration: 1339
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-17T22:48:57.813Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-17T22:48:59.764Z"
assignee: claude
---

## Description
Colleagues see more of each other: the model and what a teammate can do (George, 2026-10-17).

Today `list_teammates` (mcp/server.rs:340) promises "never workspaces, tools, or model choices", while it has shown a colleague's goal (as workingOn, until a chapter is titled) since 98ee7021 (2026-09-25). docs/security.md:683 still claims the goal is hidden (audit task-108; plan README "Task 1"). George's decision: the goal stays visible, and colleagues should also see the model and capabilities.

Add to each listed teammate (local and linked desks):
- `goal` explicitly, not only as the workingOn fallback;
- `agent`: the backend kind in plain words (Hotline Agent, Claude Code, Codex, Muse, Grok…);
- `model`: the model id, plus effort if set;
- `capabilities`: what it can do here, booleans or short names only:
  - computer attached (plus whether it's in use now);
  - web search;
  - image generation;
  - background work (schedule/loop);
  - can message teammates;
  - reach as a coarse word ("workspace" | "whole machine"), never a path;
  - the *names* of granted MCP servers, never their URLs, tokens or tool lists.

Never: cwd/workspace paths, mcpPolicy internals, secrets, server URLs or headers, node addresses, backendId internals, message text.

Update the tool description, and `docs/security.md` (the :683 row re-worded to the five-plus fields, with residual risk: a secret typed into a goal, or an MCP server's name, reaches colleagues). Linked desks: a linked desk sends only these fields.

Tests: the fields are present for local and linked teammates; none of the "never" fields leak (secret-shaped cwd/goal/url fixtures); a teammate without a computer shows none. Run `make precheck`.

## Log
- 2026-10-17T23:11:31.355Z: [claude] Done on branch dispatch/task-112 (eaadcf65); make check and make precheck (7/7) pass. Each list_teammates row (local and linked) now carries goal, agent, model, effort, capabilities {computer: none|attached|in use, webSearch, imageGeneration, backgroundWork, messageTeammates: directly|after approval, reach: workspace|whole machine, mcpServers: [names]}. Type: linked::Profile (flattened into LinkedTeammate), built by mcp::server::profile(room, persona, settings); linked::plausible_profile cuts a linked desk's words to known ones (older desks read as narrowest). For later tasks: there is no constant 'can message' bool: messageTeammates says directly vs after approval, and both it and 'whole machine' are true only for a Hotline Agent (ACP harness callbacks stay in workspace). Agent names come from new driver::acp::registry::name_of (no PATH probing). Federation row cache key now includes the profile, so settings changes republish. docs/security.md: new 'What a colleague sees' section (residual risk: secrets in a goal/MCP server name/model id reach colleagues and linked desks), :683 matrix row and :43 table cell re-worded; wire.md, sessions.md, ADR 0003, hotline-room SKILL.md and the tool description updated. No UI change.
