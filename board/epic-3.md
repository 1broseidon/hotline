---
id: epic-3
title: "Linked desks: teammates on different desks collaborate as if they shared one room"
column: in-progress
type: epic
priority: high
tags:
  - feature
  - bro-177
createdAt: "2026-10-11T21:05:10.868Z"
description: |-
  **George, 30 Sep:** the operator can **link** their local desk to one or more remote desks (servers), and the teammates on linked desks collaborate across them. It must feel exactly like local: no special friction, no different tools. "Clementine, ask Lou on grizzly to check the CI runners" works the same way Clementine messages Toad today, as long as the desks are linked and the grant exists.

  Not every desk is linked. Connecting to a server (BRO-145) makes the window a client of it. **Linking** is a separate, deliberate step that lets the two desks' teammates reach each other.

  ## Words

  * **Connected:** the window (desktop or phone) shows a desk. That's BRO-145/BRO-112 and exists today.
  * **Linked:** two desks trust each other so their teammates can see and message each other. It's operator-only, per pair, and revocable. New.

  ## What "the same as local" means

  * `list_teammates` includes linked desks' teammates, with the same roster metadata and the desk name as one more field.
  * `message_teammate` with a name reaches them, with the same `ask` and `handoff` intents, async delivery (BRO-122) and replies arriving as their own message.
  * A name is qualified (`Lou on grizzly`) only when two linked desks have the same name.
  * The same grants apply. The recipient's `allowedSenders` and the operator's first-contact approval (BRO-123) are enforced by **the recipient's desk**. The approval card reaches the operator wherever they're looking (the desktop's unified view, the phone).
  * Threads between a local teammate and a linked one show in both desks' thread views, like local threads (BRO-110).
  * Nothing about a linked teammate's workspace, tools, secrets or computer crosses the link. Only roster metadata and messages do.

  ## Slices

  1. **Desktop: one unified list across every connected desk** (child). The window shows every desk's teammates in one rail, with a desk filter, the way the phone already does (BRO-112). It's pure client work, it can ship first, and it's the foundation for everything below.
  2. **Desk-to-desk link: pairing, trust and a durable link channel** (child).
  3. **Linked roster: teammates on linked desks are visible to teammates** (child). This revives `Persona.node` and node-qualified ids.
  4. **Cross-desk messaging: message_teammate, threads and replies over the link, same grants** (child).

  Later, not now: files in cross-desk threads, the voice dispatcher (BRO-146) handing work to linked desks, phone views of links, and linking two servers with no laptop in between.

  ## Existing pieces to reuse, not duplicate

  * **The sealed channel and pinned Noise key** from BRO-114/BRO-145 pairing.
  * **The** `PersonaNode` **stub:** `Persona.node: Option<PersonaNode {id, name}>` (`contract.rs:143`, `:222`) was designed for exactly this, with node-qualified `nodeId/personaId` ids ("every call about them rides the fleet wire"). It's never set today. `import.rs:171` still skips "lives on another desk; this Hotline has no fleet".
  * **The durable exchange queue** behind `message_teammate` (BRO-122), so a message to a linked desk that's offline waits and delivers rather than failing.

  ## Done when

  George, on his laptop linked to grizzly, says "Clementine, ask Lou to check the CI runners":

  * Clementine's `message_teammate` reaches Lou with the same approval as a local colleague.
  * Lou's reply arrives in Clementine's conversation.
  * The thread shows on both desks.
  * Unlinking grizzly makes Lou disappear from Clementine's `list_teammates`, and a message to Lou fails with a plain sentence.

  ---

  ## Merged in from BRO-211 (Relay 5: desktops and server agents collaborate across systems)

  **Transport:** use the relay (0.33, ADR 0002) rather than a separate dial-out link: the Rust client already falls back to `PairedDesk.relay`, and pairing/discovery through a relay landed in BRO-216.

  **Authority** (from the BRO-209 review, item 5): reachability is not authority. Standing in on a relay today needs OWNER, which grants the whole desk command set. Collaboration needs a **narrower registration grant** bound to the desk's Noise key and **scoped collaboration permissions**; asking or handing off to a named teammate must not need owner rights. A served desk can't choose a relay itself yet, so a private server isn't reachable for collaboration until it can.

  **Open questions:** whether room-to-room pairing is a new grant role beside owner and companion; what a remote teammate may do in the other room (reach, tools, computer); how threads cross rooms (likely a thread on each side); whether every desk can relay or only served desks.

  **First slice / done when:** ADR 0003, and a desktop teammate asks a server teammate a question and gets the answer back as a delivery.

  Backlog review 2026-10-11: still unbuilt (`PersonaNode` never set, no `Seat::Link`, no ADR 0003). BRO-178 (unified list) stays as the client-only first slice.

  Migrated from Linear BRO-177: https://linear.app/1broseidon/issue/BRO-177/linked-desks-teammates-on-different-desks-collaborate-as-if-they (2026-10-11).
updatedAt: "2026-10-13T07:31:43.490Z"
assignee: claude
---

## Log
- 2026-10-13T06:32:12.078Z: [claude] Plan: a link is a room event on both desks ({kind:"link", id: peer desk id, dials, key}) trusting the peer desk's own Noise key, made over the owner wire (link.accept) from Settings › Desks. The linking desk's core dials /v2/link (also through the relay) with its desk key; the answering desk seats it as Seat::Link, which permits only link.roster/link.deliver/link.unlink. Linked teammates are a live in-memory mirror (names kept on the link record for routing), node-qualified as nodeId/personaId. Cross-desk asks and handoffs ride the existing exchange_pair queue on both desks: the sender forwards a queued request over the link, the recipient runs it under its own allowedSenders and first-contact card (raised on the recipient's tape), and returns the result over the link. ADR 0003 records it. UI: one rail across desks with a desk filter.
- 2026-10-13T07:31:27.932Z: [claude] Done on branch dispatch/epic-3 (not merged). All four slices are built, and ADR 0003 records the design: a desk_link room record on each desk trusts the other desk's own Noise key; /v2/link (also through the relay) is answered under Seat::Link, which allows only federation.roster/deliver/unlink and ping. Linked teammates are deskId/personaId participants with nothing granted. Messages cross both desks' exchange_pair queues, under the recipient's allowedSenders and a card on the recipient's desk. Unlinking from either side fails pending messages, withdraws cards and ends grants. The window has one rail across desks with a desk filter, Link/Unlink in Settings › Desks, a desk picker in Settings and New teammate, and `hotline links`/`unlink` on the CLI. An independent review's findings are fixed. cargo test --workspace (1917), clippy -D warnings, and the UI typecheck and 378 tests all pass. Not done: the Tauri app was not run by hand; linking server to server and files across desks are deferred as the epic says.
- 2026-10-14T20:53:59.663Z: [claude] 2026-10-14 merge-readiness: trial merge with main passes make check (1919 Rust tests). Review: no blockers; fixing before PR: link channel uses 32 MiB Channel and no frame rate limit (Seat::Link 1 MiB never applies); unlink race can re-save a removed link record. Follow-ups on a separate card.
