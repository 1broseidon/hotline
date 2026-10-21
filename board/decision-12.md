---
id: decision-12
title: A lost client transport holds its call for twenty seconds; the same authenticated device resumes…
type: decision
status: current
claimKey: voice.drop-grace
kind: decision
recheck: Revisit if George changes the grace period or introduces explicit call transfer between devices.
tags:
  - voice
createdAt: "2026-10-21T06:30:23.292Z"
---

## Description
A lost client transport holds its call for twenty seconds; the same authenticated device resumes the same call and thread, and expiry is an ordinary hang-up. Pairing revocation bypasses the grace period. This records George's accepted 2026-10-20 hold decision and keeps a temporary network failure separate from withdrawn authority.
