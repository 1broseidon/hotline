---
id: epic-10
title: "Remote reality fixes: pair from away, which phone and which desk, public isn't VPN"
type: epic
column: todo
position: 0
priority: high
tags:
  - remote
  - phone
  - reachability
createdAt: "2026-10-17T05:24:53.348Z"
landsOn: release/0.40.0
updatedAt: "2026-10-17T05:55:31.974Z"
dispatch:
  agent: claude
  model: opus
  by: cli
  at: "2026-10-17T05:43:03.005Z"
  until: done
  role: lead
  claim:
    host: Georges-Mac-Studio
    runner: desktop
    pid: 50333
    at: "2026-10-17T05:55:31.973Z"
assignee: claude
---

## Description
Reaching a desk from away already works: Listen on, a Public address (tunnel, reverse proxy or forwarded port), a Relay through a paired Hotline server, and documented Cloudflare tunnels. An audit on 2026-10-17 (desk origin/main abdf6262 = 0.39.0, phone release/0.7.0) found three things that don't hold up. This epic fixes those and nothing more. Tailscale Funnel is dropped (George, 2026-10-16); neither app names a VPN product.

1. **Pairing away from home** (task-56). The QR carries only the first local endpoint, so a phone pairing off-network fails unless a relay is set. The desktop's tunnel hint is wrong for a specific Listen address.
2. **Which phone, which desk** (task-12). The device list doesn't mark the caller. The hello carries neither the desk's kind nor its version.
3. **Public address shown as VPN** (hotline-mobile task-25). The phone can't tell the public endpoint from a VPN one.

Done when:
- a phone off your network pairs from the QR with a Public address set;
- the tunnel hint names the address the desk really listens on;
- the phone labels "This phone" and the desk's kind and version;
- a desk with a Public address reads "Reachable anywhere".

## Log
- 2026-10-17T05:44:30.119Z: [claude] Plan (lead claude/opus, 2026-10-17): serial, task-12 → task-56 → task-96, all on release/0.40.0. Why serial: task-12 and task-56 both change remote/mod.rs (how public_url folds into endpoints), remote/v2.rs, remote/tests.rs and the generated contract.ts; task-56's QR fallback should reuse task-12's public-endpoint marking, so 12 goes first. task-96 (added) wires both and proves the epic's done-when on the desk side in one over-the-wire test, plus docs. Scope trimmed: task-12 subtasks 3/4/5 (agent unavailable, Settings reachability, desk.status) are outside this epic and left for the person to file. Contracts added to 12/56/96: unfiltered cargo test -p hotline-core, contract.ts unchanged after it, make check. Phone side (hotline-mobile task-25, QR param parsing) is on the other board, not here.
- 2026-10-17T06:48:11.327Z: [claude] Lead: task-56 has used its one retry and is waiting in Review for George. Its own work looks done. Landing keeps failing on a different flaky test each time, most recently workspace a_revoked_handle_refuses_its_folders_too, and task-56 doesn't touch that file. The note on task-56 has the likely cause and fix (TestDirectory names collide). task-96 waits on task-56.
