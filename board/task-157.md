---
id: task-157
title: "P2-48 (D): Window fixes that Biome needs first"
column: in-progress
position: 0
priority: high
tags:
  - phase2
  - fixes
parentId: epic-13
createdAt: "2026-10-17T22:41:05.313Z"
relatedFiles:
  - ui/src/App.tsx
  - ui/src/components/Settings.tsx
  - ui/src/components/settings/
  - ui/src/components/ConnectProvider.tsx
  - ui/src/components/Welcome.tsx
  - ui/src/icons.tsx
  - ui/src/stepText.ts
  - ui/src/voice/call.ts
  - ui/src/voice/dictation.ts
updatedAt: "2026-10-18T01:32:06.707Z"
contract:
  status: in_progress
  validation:
    commands:
      - make ui-check
      - make precheck
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T01:32:06.707Z"
    reworkCount: 2
    deliveredAt: "2026-10-18T01:28:56.844Z"
    duration: 646
dependsOn:
  - task-142
  - task-172
  - task-173
  - task-174
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:15:18.843Z"
  until: done
  tries: 1
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59932
    at: "2026-10-18T01:31:24.773Z"
assignee: claude
---

## Description
Phase 2 group D (Fixes in the moved files), plan row 48 in docs/audits/0.40/README.md on release/0.40.0. Read that row, its source report section and the Standards before starting.

**Window fixes that Biome needs first.** Triage the 52 `useExhaustiveDependencies` hits, one file per commit and `App.tsx` first, with a failing test before each real stale closure. Fix `stepText.ts:14` (`biome-ignore` with a reason), the two `useIterableCallbackReturn` and the two `useExponentiationOperator` hits. Give the icon wrapper `aria-hidden` (37 hits)

- Source: window F1, F6, tasks 8, 13, and the fixes in 7
- Files: `ui/src/App.tsx`, `Settings.tsx`, `ConnectProvider.tsx`, `Welcome.tsx`, `icons.tsx`, `stepText.ts`, `voice/call.ts`, `dictation.ts`
- Collides with: 33 (land first)
- Size: M

End with `make precheck` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T01:22:32.843Z: The claim on task-157 (desktop on Georges-Mac-Studio, pid 50333) was not given back as review: task-157 is claimed by cli (pid 59932) on Georges-Mac-Studio
- 2026-10-18T01:23:03.279Z: Not landed: Claude Code is still working on task-157
- 2026-10-18T01:28:56.356Z: [claude] Done on dispatch/task-157 (HEAD 3bfa851d), one file per commit, App.tsx first. Biome 2.5.15 (recommended plus the four window.md rules, run ad hoc from a config outside the tree) found 61 useExhaustiveDependencies hits after the C splits, not 52. All 61 are now 0. Triage: stable setters from useNavigation (setDock/setInspector are useCallback over a stable shapeSpace) are added to deps. In useWindowHotkeys the menu effect no longer re-subscribes on selectedId/pane, which it never read. Budgets' `shown` moved to module scope. Every deliberate re-run trigger or mount-once effect has a `biome-ignore lint/correctness/useExhaustiveDependencies: <why>` directly above it. Biome 2.5 has no per-dependency suppression for the "more than necessary" kind, so the ignore covers the whole hook. One real stale closure: App's removeTeammate compared the pre-await selectedId, so picking B while A's removal was in flight deselected B. tests/remove-teammate.test.tsx failed on that, and the fix (setSelectedId(open => open === personaId ? null : open)) landed with it in 4d934927. ConnectProvider's login poll keeps the finish/onCancel from the render that started it; that is safe because the provider is fixed by the key and both callers' callbacks read nothing that changes during a sign-in. If a caller ever passes a closure over changing state, that one needs a ref. Also: stepText.ts ANSI regex has a biome-ignore with its reason. call.ts and dictation.ts forEach become for-of, and Math.pow becomes **. icons.tsx draws all 36 icons through one Icon wrapper with a literal aria-hidden, and Welcome.tsx's 4 glyphs carry aria-hidden on the svg. noSvgWithoutTitle is now 0. Not wired, for P2-53 (task-168): there is no ui/biome.json yet. useExhaustiveDependencies can go straight to error, and the setter additions could instead be a `hooks` stableResult entry for useNavigation if preferred. Remaining a11y (noAutofocus 9, useSemanticElements 8, noStaticElementInteractions 8, useAriaPropsSupportedByRole 8, noVoidTypeReturn 11, etc.) belong to P2-50/53. make ui-check (422 pass) and make precheck (all 7 legs) pass.
- 2026-10-18T01:29:01.010Z: [claude] Correction: make ui-check is 421 pass (that count includes the new remove-teammate test), not 422.
- 2026-10-18T01:29:48.078Z: The claim on task-157 (cli on Georges-Mac-Studio, pid 59932) was not given back as review: task-157 is claimed by desktop (pid 50333) on Georges-Mac-Studio
- 2026-10-18T01:29:49.590Z: Not landed: Claude Code is still working on task-157
