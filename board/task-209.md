---
id: task-209
title: "hotline-mcp: the MCP client as its own crate; the server stays in the core"
column: todo
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
updatedAt: "2026-10-19T14:09:47.516Z"
contract:
  status: ready
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
      - "! grep -q 'hotline-core' crates/hotline-mcp/Cargo.toml"
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
    readyAt: "2026-10-19T14:09:47.516Z"
---

## Description
ADR 0007 step 2. Move the client (mcp.rs, oauth.rs, tool.rs, about 2,500 production lines) to crates/hotline-mcp. Its reach into the vault becomes rmcp's CredentialStore (which the vault already implements) plus values the core resolves and hands in, with saving an OAuth registration as the one call back; the capability lease comes from hotline-tools and the ledger from the room crate. TeammateTools (the MCP server, Weak<Room>) stays in hotline-core: it is the mcp/session cycle the standards keep whole. Its tests follow the moved code; the desk harness's MCP tests pass unchanged.
