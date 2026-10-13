---
id: task-17
title: "Linked roster: teammates on linked desks appear in list_teammates and the rosters, node-qualified"
column: todo
position: 19
priority: high
tags:
  - feature
  - bro-180
parentId: epic-3
createdAt: "2026-10-11T21:05:27.281Z"
description: |-
  Slice 3 of BRO-177, built on the link channel (slice 2).

  ## What

  * Each linked desk publishes its teammates' **roster metadata only**: id, name, state (idle, working, waiting on the person, stopped), current activity, chapter title or goal, and last turn. This is exactly what `list_teammates` already exposes for local teammates (BRO-125), and nothing more: no workspaces, tools, models, messages or secrets.
  * The receiving desk holds them as **linked teammates**:
    * `Persona.node = Some(PersonaNode {id, name})` (the existing, never-set stub at `contract.rs:143/222`).
    * Node-qualified ids: `nodeId/personaId`, as that stub's doc already specifies.
    * They're kept out of the local room log as full personas. They're a live mirror, refreshed over the link, and gone when the link is revoked.
  * `list_teammates` includes them, with the desk name as a field. A teammate should see "Lou (grizzly), working, on 'CI runners'" the way it sees Toad.
  * **Name resolution** for `message_teammate`:
    * A unique name anywhere resolves.
    * A collision across desks needs the desk ("Lou on grizzly", or the qualified id).
    * Local teammates win ties **only** for display ordering, never silently for routing. An ambiguous name is an error that lists the choices.
  * **Windows:** the unified view (slice 1) already shows each connected desk's own teammates. Linked teammates of a desk the window *isn't* connected to aren't shown as chat rows; they appear only as the other side of threads (slice 4).
  * **Clean up the old fleet code:** `import.rs:171`'s "this Hotline has no fleet" skip stays correct (importing still doesn't pull other desks' teammates), but reword it now that links exist.

  ## Done when

  * With the laptop linked to grizzly, Clementine's `list_teammates` shows Lou, M and Wren with desk "grizzly" and live state.
  * Revoking the link removes them within a few seconds.
  * A test covers ambiguous names erroring with the choices listed.
  * A test covers that no non-roster field crosses the link.

  Migrated from Linear BRO-180: https://linear.app/1broseidon/issue/BRO-180/linked-roster-teammates-on-linked-desks-appear-in-list-teammates-and (2026-10-11).
updatedAt: "2026-10-11T21:05:27.563Z"
---

## Log
- 2026-10-13T07:31:35.699Z: [claude] On dispatch/epic-3: each side publishes LinkedTeammate rows, the same struct as a list_teammates row, cut to plausible values and cached per teammate. list_teammates shows linked teammates as deskId/personaId with a `desk` field, and as offline from the record's names while the link is down. Name resolution accepts "Name on Desk" and refuses ambiguous names, listing the choices. import.rs is reworded. Tests are in session/links/tests.rs and linked.rs.
