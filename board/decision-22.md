---
id: decision-22
title: Independent socket reads run concurrently; global mutations, ping and subscription controls retain…
type: decision
status: current
claimKey: wire.command-ordering
kind: decision
createdAt: "2026-10-21T10:31:25.391Z"
---

## Description
Independent socket reads run concurrently; global mutations, ping and subscription controls retain arrival order, and teammate commands retain their existing lanes.
Reads see committed state when executed. A caller needing a preceding mutation awaits its reply or an ordered ping first; ping covers global writes, not teammate lanes or reads. Client negotiation stays on the reader so silence detection and frame shaping take effect before further dispatch. Remote tasks cancel with the seat; accepted desktop mutations finish. This keeps slow work from stopping the reader without racing ordered appends and subscriptions.
