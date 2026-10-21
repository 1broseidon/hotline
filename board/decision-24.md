---
id: decision-24
title: Only a seat whose write exceeds the deadline selects zero TCP linger before transport drop.
type: decision
status: current
claimKey: wire.abortive-close
kind: decision
createdAt: "2026-10-21T11:23:31.284Z"
---

## Description
Only a seat whose write exceeds the deadline selects zero TCP linger before transport drop. Ordinary WebSocket and HTTP closes retain normal draining so Windows cannot discard final pairing or authentication replies. The seat joins its writer and reunites the transport before setting linger; no socket is configured abortively at accept or dial.
