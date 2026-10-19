---
id: task-209
title: "hotline-mcp: the MCP client as its own crate; the server stays in the core"
column: review
position: 10
priority: high
tags:
  - crate-split
assignee: claude
relatedFiles:
  - crates/hotline-core/src/mcp.rs
  - crates/hotline-core/src/mcp
parentId: epic-18
createdAt: "2026-10-19T14:07:42.047Z"
dependsOn:
  - task-208
updatedAt: "2026-10-19T15:32:02.217Z"
contract:
  status: delivered
  deliverables:
    - type: file
      path: crates/hotline-mcp/Cargo.toml
      description: the MCP client crate
    - type: docs
      path: docs/adr/0007-split-hotline-core.md
      description: step 2 done, with each moved file's old and new path
  validation:
    commands:
      - test -f crates/hotline-mcp/Cargo.toml
      - "! test -e crates/hotline-core/src/mcp/oauth.rs"
      - "! test -e crates/hotline-core/src/mcp/tool.rs"
      - grep -rq 'struct TeammateTools' crates/hotline-core/src
      - "! grep -qE '^hotline-core[[:space:]]*=' crates/hotline-mcp/Cargo.toml"
      - cargo test -p hotline-core --test mcp
      - make check
      - git diff --exit-code -- ui/src/generated/contract.ts
      - make precheck
  constraints:
    - Moves only, no behaviour change; no new trait beyond rmcp's existing CredentialStore
    - TeammateTools (the MCP server) stays in hotline-core
    - ui/src/generated/contract.ts byte-identical after an unfiltered cargo test
    - hotline-mcp never depends on hotline-core or Tauri
  metrics:
    readyAt: "2026-10-19T14:10:20.624Z"
    pickedUpAt: "2026-10-19T15:05:10.603Z"
    reworkCount: 0
    deliveredAt: "2026-10-19T15:31:51.589Z"
    duration: 1601
dispatch:
  agent: claude
  model: opus
  effort: high
  by: lead
  at: "2026-10-19T15:05:02.180Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 94104
    at: "2026-10-19T15:05:04.316Z"
    ended: "2026-10-19T15:32:02.217Z"
    outcome: review
---

## Description
ADR 0007 step 2, on top of task-208 (read its notes: the ledger and normalize_servers now live in the room crate). Move the client (mcp.rs, mcp/oauth.rs, mcp/tool.rs, about 2,500 production lines) to crates/hotline-mcp. Its reach into the vault becomes rmcp's CredentialStore (which the vault already implements) plus values the core resolves and hands in, with saving an OAuth registration as the one call back; the capability lease comes from hotline-tools and the ledger from the room crate. TeammateTools (the MCP server in mcp/server.rs, Weak<Room>) stays in hotline-core: it is the mcp/session cycle the standards keep whole. Its tests follow the moved code; the desk harness's MCP tests (tests/mcp.rs) pass unchanged. The crate gets the workspace lints, a license and a crate doc saying what it owns and what it must never depend on. docs/standards.md's module map, docs/security.md citations (the MCP tool origin and its lease are cited there) and AGENTS.md follow the move; what a teammate can reach does not change, and the PR says so. In ADR 0007, list each moved file's old and new path.

## Log
- 2026-10-19T15:05:01.244Z: [claude] Lead: task-208 landed on release/0.41.0. The tool ledger is now hotline_room::tool_ledger (was session/ledger.rs) and normalize_servers is hotline_room::mcp_servers; the core still re-exports both at the old paths (session::ledger, mcp::normalize_servers): point the moved client at hotline-room directly and drop a re-export once no core caller remains. redact_text and same_secret live in hotline_room::secret, clip in hotline_room::text. ADR 0007 has a "Step 1, done" old→new table: write "Step 2, done" the same way. Run the full `cargo test -p hotline-core`, `make check` and `make precheck` in the foreground and to completion before ending your turn, then commit.
- 2026-10-19T15:31:58.473Z: [claude] Done on dispatch/task-209 (51cd5108 code, e6874234 ADR). crates/hotline-mcp (on contract, support, tools, room; never core or Tauri) holds lib.rs (was mcp.rs minus `pub mod server`), oauth.rs, tool.rs, plus McpOAuthRegistration (was in vault.rs). Seam: no new trait. The client takes `OAuthVault<S>` (the vault's McpCredentialStore, token lock, registration as Result) and `Vaulted<S>` (that, or a secret server's token); `connect_granted` takes `Granted<S>` (server already resolved by the vault, or the error); McpOAuthService holds no vault (Default), start takes a save-registration closure (the one call back), sign_out clears via CredentialStore::clear. The core's mcp.rs (~170 lines) keeps TeammateTools (mcp/server.rs), re-exports the client at crate::mcp, builds the values (connect_with_capability_and_vault, oauth_vault) and wraps the service as `mcp::McpOAuth` for the desk. Tests: 22 moved into the crate; the six vault-backed OAuth flow tests and their mock auth server stayed in the core as mcp/tests.rs (they need a real Vault); how_to_use test → mcp/server.rs; the computer-ledger test → driver/rig/tests.rs over loopback HTTP (Connections::for_test is gone). tests/mcp.rs unchanged. security.md citations, standards.md, development.md, AGENTS.md, timings-summary OWN updated; ADR 0007 has "Step 2, done" with old→new tables, module-map and timings (lib unit 30.1 s / front end 18.9 s vs 31.8/19.7; the cold test build read 74.5 s under load 7-13 and should be re-taken on a quiet machine before step 3 is judged). Teammate reach unchanged. Left for later: the core still reaches the client and normalize_servers through re-exports at crate::mcp (callers in import.rs, wire settings, vault/launch.rs). make check, the full core tests and make precheck (8 legs) pass; contract.ts byte-identical.
