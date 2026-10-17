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
updatedAt: "2026-10-17T05:39:55.063Z"
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
