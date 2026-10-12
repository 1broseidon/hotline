---
id: task-3
title: A curated list of MCP servers people can connect
column: todo
position: 5
priority: medium
tags:
  - bro-9
createdAt: "2026-10-11T21:05:15.361Z"
description: |-
  ## Description

  Supersedes task-14 and the credential-broker idea. Toad's MCP client already does the whole authorization spec through rmcp: protected-resource and authorization-server discovery, dynamic client registration, PKCE with S256, exchange and refresh, with Toad's policy refusing the legacy fallback and binding registrations to the configured URL and issuer. Dynamic registration is what removes the need for a broker: a client registers itself at run time, so there is no client secret to ship or to host on anyone's behalf. So Toad builds no broker and adopts no gateway of its own. The work is discovery: a short curated list in Settings, each entry naming who runs the server and whether the user's data passes through it, so connecting Gmail or a calendar is a choice rather than a pasted URL. A user who wants a managed gateway brings their own; the choice, the terms and the data path stay theirs.

  ## Log

  * 2026-09-14T15:19:49.714426Z: Migration ownership: toad desktop Settings, although the source ID stays toad-mobile/task-15. Supersedes archived task-14; does not supersede the computer credential-filling epic.

  ## Migration source

  Brainfile: toad-mobile/task-15
  Source file: .brainfile/board/task-15.md

  Migrated from Linear BRO-9: https://linear.app/1broseidon/issue/BRO-9/a-curated-list-of-mcp-servers-people-can-connect (2026-10-11).

  ## Status (2026-10-11 review)

  Settings › Tools has only a manual Add server form (ui/src/components/Settings.tsx ~1168-1240). Description still says "Toad"; decide whether runprism is one of the curated entries. (This came over from the old phone board too.)
updatedAt: "2026-10-11T21:05:15.657Z"
---

## Log
- 2026-10-12T01:51:10.801Z: [claude] Won't do (2026-10-11 sort): a curated MCP list goes stale fast; runprism's catalogue is the better home for this.
