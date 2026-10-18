---
id: task-173
title: "P2-33c (C): Window split, move-only: Transcript.tsx (folds to transcriptFolds.ts first)"
description: |-
  Phase 2 group C, plan row 33 (third landing) in docs/audits/0.40/README.md on release/0.40.0. Read that row, window.md F2/F3 tasks 9–12 and the Standards first.

  Move-only: first move the fold logic out of `ui/src/components/Transcript.tsx` into `ui/src/transcriptFolds.ts`, then split the rest of the component by concern, re-exporting everything from `components/Transcript.tsx` so no importer changes. No behaviour change.

  End with `make precheck` and `make ui-check` (the hooks are on; never --no-verify). Follow the epic's rules.
priority: high
tags:
  - phase2
  - restructure
  - window
relatedFiles:
  - ui/src/components/Transcript.tsx
  - ui/src/transcriptFolds.ts
parentId: epic-13
createdAt: "2026-10-17T22:45:36.110Z"
contract:
  status: delivered
  validation:
    commands:
      - test -f ui/src/transcriptFolds.ts
      - make ui-check
      - make precheck
  metrics:
    readyAt: "2026-10-17T22:48:38.572Z"
    pickedUpAt: "2026-10-18T01:06:32.689Z"
    reworkCount: 2
    deliveredAt: "2026-10-18T01:12:12.437Z"
    duration: 340
updatedAt: "2026-10-18T01:15:59.674Z"
dependsOn:
  - task-122
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-18T00:45:43.011Z"
  until: done
  tries: 2
  claim:
    host: Georges-Mac-Studio
    runner: cli
    pid: 59932
    at: "2026-10-18T01:06:05.806Z"
    ended: "2026-10-18T01:11:43.835Z"
    outcome: review
assignee: claude
completedAt: "2026-10-18T01:15:59.674Z"
---

## Description
Phase 2 group C, plan row 33 (third landing) in docs/audits/0.40/README.md on release/0.40.0. Read that row, window.md F2/F3 tasks 9–12 and the Standards first.

Move-only: first move the fold logic out of `ui/src/components/Transcript.tsx` into `ui/src/transcriptFolds.ts`, then split the rest of the component by concern, re-exporting everything from `components/Transcript.tsx` so no importer changes. No behaviour change.

End with `make precheck` and `make ui-check` (the hooks are on; never --no-verify). Follow the epic's rules.

## Log
- 2026-10-18T00:36:37.437Z: Not landed: 1 file is not committed in the worktree: send it back, or commit in your editor
- 2026-10-18T00:45:42.844Z: Landing sent back (1 of 1): with release/0.40.0 brought in, `make precheck` failed: warning: build failed, waiting for other jobs to finish... ⏎ log: /Users/george/.brainfile-worktrees/hotline-e23ecf/task-173/target/precheck/contract.log ⏎ rerun: scripts/precheck contract ⏎ clippy pass 70s ⏎ windows pass 92s ⏎ ui pass 12s ⏎ precheck: 1 of 7 legs failed in 182s: contract ⏎ make: *** [precheck] Error 1. Fix it on this branch, commit, and run the checks again.
- 2026-10-18T01:11:38.740Z: [claude] The sent-back contract failure was environmental, not this branch: target/sherpa-onnx-prebuilt had only the Windows extraction (the osx-arm64 one was gone) while cargo's cached sherpa-onnx-sys build output still pointed at it, so linking failed; `cargo clean -p sherpa-onnx-sys` re-extracted it. A second run showed a contract diff against a half-written kept copy; run alone, contract.ts matches HEAD and a full unfiltered `cargo test -p hotline-core` leaves it unchanged. No new commit needed: the work is 9019afd3 + dc0fd993 + merge a4c8c49f on dispatch/task-173. `make precheck` (7/7) and `make ui-check` (420 pass) pass. For later tasks: if precheck's contract leg says `could not find native static library sherpa-onnx-c-api`, run `cargo clean -p sherpa-onnx-sys` (the host and windows legs share target/sherpa-onnx-prebuilt). Nothing in the Transcript split is left unwired.
- 2026-10-18T01:11:45.378Z: Not landed: Claude Code is still working on task-173
- 2026-10-18T01:12:11.970Z: [claude] The precheck failure was not in the code. The macOS sherpa-onnx prebuilt in this worktree's target/ was only partly extracted (no sherpa-onnx-c-api), and a second session ran `cargo test -p hotline-core` here while the contract leg ran, so both wrote contract.ts at once. I deleted the partial prebuilt and reran with nothing else running: `make precheck` passes all 7 legs and `make ui-check` passes. The branch dispatch/task-173 is unchanged at a4c8c49f (folds in transcriptFolds.ts; leaf renderers under components/transcript/, re-exported from Transcript.tsx). Nothing is left unwired. For later tasks: never run precheck in a worktree where another session is running cargo test.
- 2026-10-18T01:15:59.204Z: Landed on release/0.40.0
