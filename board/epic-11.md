---
id: epic-11
title: "Relay-only servers: a server can just relay, and a desktop can pair with it only to relay"
type: epic
column: backlog
position: 43
priority: high
tags:
  - relay
  - server
  - security
  - pi
createdAt: "2026-10-17T05:37:00.361Z"
landsOn: release/0.40.0
updatedAt: "2026-10-17T05:39:55.720Z"
---

## Description
A server can be just a relay. Today `hotline serve` always boots the whole desk (room, scheduler, agent drivers, computer runtime, model catalogue, provider keys), and a desktop can only stand in on its relay as an **owner** (`remote/server.rs:384`; `DeviceRole` is Owner | Companion, `remote/mod.rs:54`). Using a Raspberry Pi as a relay means running an agent server on it and handing the desktop full control of it. George, 2026-10-17: "what if I only want to allow relay capability, like if the relay lives on a pi and I don't want to run agents on there."

Two separate axes:

1. **What a server runs: capabilities.** `hotline serve --capabilities agents,relay`, both on by default, so existing servers are unchanged. Relay-only boots Remote (listener, admission, pairing, relay) and a small door for `hotline status`. It has no room, scheduler, agents, computer runtime, model catalogue or provider keys. Modes can be switched later, keeping pairings. Adding agents to a relay-only server is its own setup step (models, store).
2. **What a paired device may do there: roles.** A third grant role, `relay`, beside owner and companion. A relay grant may stand in, take visits, and carry `/v2/link` for linked desks. It may not run commands, read conversations or files, or change settings. A relay-only server hands out only relay grants; a full server may give a desktop owner or just relay. The person administers the server from its own terminal.

Discovery: the server says what it runs (capabilities in the hello, building on epic-10's deskKind), so the desktop and phone know a relay from a desk with a team.

Name: **relay**, not gateway. It carries sealed records it cannot read; "gateway" suggests a box that inspects, and is runprism's word. Code stays in hotline-core behind the mode split (George, 2026-10-17).

Migration: a desktop that paired as owner only to use the relay is offered a move to the relay role in place (least privilege: a stolen desktop key would then only stand in). Declining changes nothing. A server where teammates are used stays owner.

Done when, on a Pi:
- `hotline serve --capabilities relay` runs with no agent machinery;
- a desktop pairs with it as relay and a phone reaches the desktop through it;
- that desktop cannot prompt, read or configure the Pi;
- the Pi can later switch to agents,relay without re-pairing.
