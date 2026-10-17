---
id: task-102
title: "Installer: `install --server --relay` sets up a relay-only server with the hotline-relay unit"
column: review
position: 1
description: |-
  Left by task-95 (a3b9927e, on release/0.40.0): the server tarball now ships packaging/hotline-relay.service (`--capabilities relay`, installed as hotline.service so pair/status/update find the room), but the curl installer (site/public/install) has no relay mode, so "Run a relay" on a Pi is a manual install.

  Give the installer's server path a relay option (match the installer's existing flag style; read how it installs hotline.service today) that installs hotline-relay.service in place of hotline.service, and nothing for agents. Re-running without it must not silently turn a relay into a full server, and re-running with it on a full server must not silently drop agents: say what it would do and stop, or follow the installer's existing rule for re-runs. Then replace the manual steps in docs-site server/relay.md with the one command (and mention it in server/install.md).

  Tests: extend site/tests/test_install.py for the relay unit chosen, the default unchanged, and the re-run rule. Never touch a real systemd; the tests already fake it.
priority: medium
tags:
  - relay
  - packaging
  - pi
relatedFiles:
  - site/public/install
  - site/tests/test_install.py
  - packaging/hotline-relay.service
  - docs-site/src/content/docs/server/relay.md
  - docs-site/src/content/docs/server/install.md
parentId: epic-11
contract:
  status: ready
  validation:
    commands:
      - python3 -m unittest discover -s site/tests -p 'test_install.py' -v
      - cd docs-site && bun install --frozen-lockfile && bun run build
  metrics:
    readyAt: "2026-10-17T09:23:10.406Z"
createdAt: "2026-10-17T09:19:46.947Z"
dispatch:
  agent: claude
  model: opus
  by: lead
  at: "2026-10-17T09:19:49.816Z"
  until: done
  tries: 2
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-17T09:22:50.259Z"
assignee: claude
updatedAt: "2026-10-17T09:23:10.406Z"
---

## Description
Left by task-95 (a3b9927e, on release/0.40.0): the server tarball now ships packaging/hotline-relay.service (`--capabilities relay`, installed as hotline.service so pair/status/update find the room), but the curl installer (site/public/install) has no relay mode, so "Run a relay" on a Pi is a manual install.

Give the installer's server path a relay option (match the installer's existing flag style; read how it installs hotline.service today) that installs hotline-relay.service in place of hotline.service, and nothing for agents. Re-running without it must not silently turn a relay into a full server, and re-running with it on a full server must not silently drop agents: say what it would do and stop, or follow the installer's existing rule for re-runs. Then replace the manual steps in docs-site server/relay.md with the one command (and mention it in server/install.md).

Tests: extend site/tests/test_install.py for the relay unit chosen, the default unchanged, and the re-run rule. Never touch a real systemd; the tests already fake it.

## Log
- 2026-10-17T09:22:24.699Z: [claude] Done on dispatch/task-102 (6fd3d051): `install --server --relay` installs hotline-relay.service as hotline.service under the hotline service account (no pick_user; --relay --user refused), keeps `--capabilities relay` when --listen rewrites ExecStart, and on a fresh relay offers `hotline pair --link`. Re-run rule: refuse and change nothing — relay unit (ExecStart has `--capabilities relay`, agents,relay counts as full) without --relay, or full unit with --relay. Docs: relay.md's manual steps replaced by the one command, install.md and docs/serve.md mention it, CHANGELOG Unreleased. For later tasks: `hotline update` never touches the unit, so mode only changes by hand-editing --capabilities. 44 installer tests pass; docs-site builds.
- 2026-10-17T09:22:48.737Z: Landing sent back (1 of 1): with release/0.40.0 brought in, `python3 -m pytest site/tests/test_install.py` failed: /Users/george/.local/share/mise/installs/python/3.14.8/bin/python3: No module named pytest. Fix it on this branch, commit, and run the checks again.
