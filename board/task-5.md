---
id: task-5
title: Sign Windows releases with a trusted publisher identity when demand warrants it
column: wont-do
position: 6
priority: low
tags:
  - bro-19
createdAt: "2026-10-11T21:05:17.027Z"
description: |-
  Deferred by George on 2026-09-09: Windows Authenticode signing adds a recurring cost that is not justified yet. 0.6.0 shipped the Windows installer and updater without publisher signing, so Windows may still show an installation reputation warning.

  Revisit when Windows adoption warrants it: evaluate trusted signing for both the executable and installer, timestamp signatures, protect CI signing credentials, and document that SmartScreen reputation may still warn for new builds. Tauri updater signatures remain required and are separate from Windows publisher identity. Do not provision a signing service or incur cost as part of current release work.

  ## Migration source

  Brainfile: toad-next/task-65 · Source file: `.brainfile/board/task-65.md` on the Linux desk (gitignored, local only)

  Migrated from Linear BRO-19: https://linear.app/1broseidon/issue/BRO-19/sign-windows-releases-with-a-trusted-publisher-identity-when-demand (2026-10-11).
updatedAt: "2026-10-12T01:51:08.146Z"
---

## Log
- 2026-10-12T01:51:07.834Z: [claude] Won't do (2026-10-11 sort): no Windows demand yet; revisit when there is.
