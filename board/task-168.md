---
id: task-168
title: "P2-53 (F): Biome for lint only"
column: review
position: 57
priority: high
tags:
  - phase2
  - gates
parentId: epic-13
createdAt: "2026-10-17T22:41:11.082Z"
relatedFiles:
  - ui/biome.json
  - ui/package.json
  - ui/bun.lock
  - Makefile
updatedAt: "2026-10-18T03:32:33.656Z"
contract:
  status: delivered
  validation:
    commands:
      - test -f ui/biome.json
      - grep -q biome Makefile ui/package.json
      - make ui-check
      - make precheck
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T03:26:47.562Z"
    reworkCount: 0
    deliveredAt: "2026-10-18T03:32:33.656Z"
    duration: 346
dependsOn:
  - task-159
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:28.895Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 39764
    at: "2026-10-18T03:26:03.467Z"
assignee: claude
---

## Description
Phase 2 group F (Gates), plan row 53 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Biome for lint only.** `biome.json` with the window.md rule set, formatter off, a11y at warn until 50 lands, and a `lint` step in `make ui-check`

- Source: window F6, task 7
- Files: `ui/biome.json`, `ui/package.json`, `Makefile`
- Collides with: 48
- Size: S

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T03:32:33.222Z: [claude] Done on dispatch/task-168 (528e3c80): ui/biome.json (Biome 2.5.15 pinned, lint only, no formatter or assist, CSS linter off, src+tests minus src/generated), `bun run lint` in make ui-check, CI's per-OS check and precheck's ui leg; docs/development.md and AGENTS.md say so. window.md rule set: recommended + useExhaustiveDependencies/useHookAtTopLevel/noUnusedImports/noExplicitAny at error, noNonNullAssertion and useLiteralKeys off (noDescendingSpecificity moot with CSS off). a11y is at error (50 landed) except four recommended rules that still have hits, which are at warn with the others the tree does not pass yet. Fixed the 3 useButtonType hits in AgentSignIn. Left at warn for a later task (39 warnings): a11y useAriaPropsSupportedByRole 7, useAriaPropsForRole 3, useKeyWithClickEvents 3 (Viewer), noAriaHiddenOnFocusable 1 (Rail kbd); noVoidTypeReturn 11 (voice/call.ts, dictation.ts), noArrayIndexKey 10, noAssignInExpressions 1 (tests/dock.test.ts); plus 3 recommended warns/infos (useOptionalChain 2, noCommaOperator 1, noUselessFragments 2). Each fix should flip its rule to error in biome.json. lint does not use --error-on-warnings. make ui-check and make precheck pass.
