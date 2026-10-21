---
id: decision-15
title: Voice answers are core-owned session approvals or informational yes/no answers; opaque operation…
type: decision
status: superseded
claimKey: voice.card-answer-boundary
kind: decision
recheck: Revisit when driver cards carry independently trusted operation metadata, or when a question needs authority rather than information.
createdAt: "2026-10-21T07:08:22.020Z"
supersededBy: decision-29
updatedAt: "2026-10-21T20:04:16.289Z"
---

## Description
Voice answers are core-owned session approvals or informational yes/no answers; opaque operation cards stay at the desk.
Only finalized caller speech enters the answer path, never a model tool. A collaboration approval is backed by the core's live wait and grants this session only. ask_question creates an informational card with a live capability lease and no operation authority. request_human, passkeys and driver permissions cannot be promoted by text, option names or supplied flags; old/restarted questions without a live lease remain desk-only. No voice path confirms a credential, 2FA, screen action, payment or deletion.
