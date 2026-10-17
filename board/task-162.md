---
id: task-162
title: "P2-D-2 (E): Server docs"
column: todo
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
updatedAt: "2026-10-17T22:48:38.570Z"
contract:
  status: ready
  validation:
    commands:
      - grep -q 'hotline update' docs/serve.md
      - git diff --quiet $(git merge-base HEAD release/0.40.0) -- crates ui
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
---

## Description
Phase 2 group E (Docs), plan row D-2 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Server docs.** `hotline update` in operate.md; status and the file table; relay and phone labels; the desk's account stated once; the `Paired` signature; `hotline update` and `help` in serve.md

- Source: tests-and-docs D1, D5, D7, D10, D11 (CLI part), tasks 12, 13, 15 (serve.md part), 17 (CLI part)
- Files: `docs-site/src/content/docs/server/*`, `phone/pair.md`, `docs/serve.md`
- Collides with: none
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.
