---
id: task-56
title: Pairing QR carries the public address when one is set; tunnel hint names the real listen address
column: backlog
position: 39
priority: medium
tags:
  - remote
  - pairing
createdAt: "2026-10-14T19:29:48.828Z"
updatedAt: "2026-10-14T20:05:32.092Z"
---

## Description
QR u= is always the first local endpoint (crates/hotline-core/src/remote/v2.rs:117), so pairing off-network fails even with a Public address set. When public_url is set, put it in u= (phones already accept a hostname; LAN addresses arrive in the hello). Also the Public address hint says 'Point it at https://localhost:8788', which is wrong when Listen on is a single IP (172.16.30.28 here): show the actual listen address.
