---
id: task-55
title: Desk drops a phone's dead socket sooner
column: todo
position: 3
priority: low
tags:
  - remote
createdAt: "2026-10-14T19:29:48.338Z"
updatedAt: "2026-10-18T23:40:47.707Z"
parentId: epic-14
---

## Description
Seen 2026-10-14: after the phone left Wi-Fi for the Cloudflare tunnel, the desk kept the old LAN socket ESTABLISHED with ~20 KB queued until TCP gave up. The desk should notice on its own (ping/idle timeout on a seat whose writes back up) and close it.

0.40 triage (2026-10-18): Remaining: a 90 s silence timeout exists for phone links (wire/socket.rs:207), yet the Oct 14 socket stayed up. Find which seat it was, then close a seat whose writes back up (bounded send queue) or have the desk ping it. Over-the-wire test.
