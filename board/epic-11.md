---
id: epic-11
title: "Relay-only servers: a server can just relay, and a desktop can pair with it only to relay"
type: epic
column: todo
position: 3
priority: high
tags:
  - relay
  - server
  - security
  - pi
createdAt: "2026-10-17T05:37:00.361Z"
landsOn: release/0.40.0
updatedAt: "2026-10-17T07:44:21.543Z"
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-17T07:44:20.215Z"
  until: done
  role: lead
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-17T07:44:21.543Z"
assignee: claude
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

## Log
- 2026-10-17T07:47:29.220Z: [claude] Lead plan (claude/opus, 2026-10-17). Order: task-90 ADR → task-91 relay role → task-92 capabilities + relay-only boot → task-98 desktop relay pairing/store + owner→relay move (core+app) → {task-93 UI, task-94 phone contract} in parallel → task-99 end-to-end harness (crates/hotline-cli/tests/relay.rs, proves the epic's Done-when) → task-95 docs/packaging. Why: 91 and 92 both change remote/mod.rs, remote/server.rs and cli main.rs, and a relay-only server needs the relay role, so 92 waits on 91 ("relay-only makes only relay grants" moved from 91 to 92). 93 was split: its core/shell half (relay kept apart from desks, since a relay grant cannot open /v2; owner→relay move; undeclared pair claims refused on relay-only) is task-98, so 93 is UI only. 94 re-scoped to the desk side in this repo (QR marker, refusal code, docs/wire.md); the phone screens are hotline-mobile work George files from 94's note. ADR 0006 must settle the old-reader story: DeviceRole is a plain serde enum in remote.json, so an unknown "relay" would fail the whole file on an old desk. Every child now has relatedFiles and validation commands. Pi footprint: agents measure on aarch64 macOS; George confirms on a Pi. Agents: claude/opus for 90, 91, 92, 98, 99 (security and protocol); claude/sonnet for 93, 94, 95.
