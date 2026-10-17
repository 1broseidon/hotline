---
id: task-162
title: "P2-D-2 (E): Server docs"
column: review
position: 51
priority: high
tags:
  - phase2
  - docs
parentId: epic-13
createdAt: "2026-10-17T22:41:07.934Z"
relatedFiles:
  - docs-site/src/content/docs/server/
  - docs-site/src/content/docs/phone/pair.md
  - docs/serve.md
updatedAt: "2026-10-17T23:25:49.409Z"
contract:
  status: delivered
  validation:
    commands:
      - grep -q 'hotline update' docs/serve.md
      - git diff --quiet $(git merge-base HEAD release/0.40.0) -- crates ui
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-17T23:12:35.345Z"
    reworkCount: 0
    deliveredAt: "2026-10-17T23:19:56.121Z"
    duration: 441
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-17T23:10:42.436Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-17T23:11:47.306Z"
assignee: claude
---

## Description
Phase 2 group E (Docs), plan row D-2 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Server docs.** `hotline update` in operate.md; status and the file table; relay and phone labels; the desk's account stated once; the `Paired` signature; `hotline update` and `help` in serve.md

- Source: tests-and-docs D1, D5, D7, D10, D11 (CLI part), tasks 12, 13, 15 (serve.md part), 17 (CLI part)
- Files: `docs-site/src/content/docs/server/*`, `phone/pair.md`, `docs/serve.md`
- Collides with: none
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-17T23:19:59.055Z: [claude] Done on dispatch/task-162 (7da5d9ca), docs only, make precheck passes. operate.md: sudo hotline update, the full status output (data, runs, relay-only form), serve.json and pending.json in the file table. relay.md/pair.md: Revoke and Relay labels, the sudo -u hotline form for pair --link, the --capabilities line no longer reads as runnable. D5: troubleshooting, computers and serve.md say "the account the desk runs as" with systemctl show -p User. install.md's manual-install line still says the hotline account, which is correct because the shipped unit does run as hotline. serve.md: Paired/PairedDesk/PairedRelay and pair_relay, a new ## Update section and a hotline help line. Left for D-3+: HOTLINE_EXTRA_DESKS and HOTLINE_DEV_SIGN_IDENTITY in development.md (task 17's other half), threads.md (D8).
- 2026-10-17T23:25:49.409Z: Landed on release/0.40.0
