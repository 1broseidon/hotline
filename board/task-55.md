---
id: task-55
title: Desk drops a phone's dead socket sooner
column: todo
position: 34
priority: low
tags:
  - remote
createdAt: "2026-10-14T19:29:48.338Z"
---

## Description
Seen 2026-10-14: after the phone left Wi-Fi for the Cloudflare tunnel, the desk kept the old LAN socket ESTABLISHED with ~20 KB queued until TCP gave up. The desk should notice on its own (ping/idle timeout on a seat whose writes back up) and close it.
