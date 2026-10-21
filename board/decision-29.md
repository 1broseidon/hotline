---
id: decision-29
title: Voice answers are core-owned session approvals, core-owned preferred-name saves, or informational…
type: decision
status: current
claimKey: voice.card-answer-boundary
kind: decision
recheck: Revisit only when another operation has independently trusted core metadata and explicit authorization for voice approval.
supersedes: decision-15
createdAt: "2026-10-21T20:04:16.288Z"
---

## Description
Voice answers are core-owned session approvals, core-owned preferred-name saves, or informational yes/no answers; opaque operation cards remain desk-only.
Task-261 explicitly adds set_person_name. The core binds the proposed name and requesting lease to the approval id before publishing 'Save your name as …?'. Only the person's approved answer writes personName; no other setting is reachable. Finalized caller yes/no may settle this bounded operation on the matching live call. Ordinary ask_question prose grants no operation, collaboration yes remains session-only, and credentials, 2FA, screens, payments, deletions, passkeys and opaque driver permissions remain desk-only. Expiry, withdrawal, revocation or loss of the in-memory authority prevents the name write.
