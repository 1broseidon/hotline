---
id: task-65
title: "Linked desks: follow-ups from the merge review"
column: backlog
position: 42
priority: low
tags:
  - linked-desks
  - remote
createdAt: "2026-10-14T20:55:06.480Z"
---

## Description
From the 2026-10-14 review of dispatch/epic-3 (fixed before merge: link frame size/rate bounds, unlink race). Remaining: (1) room.jsonl growth: a peer alternating its roster every 30s saves ~50 KB each time (~140 MB/day); compaction only at startup. (2) Dial loop exits without clearing 'dialing' if room.unlinked fails after a refused key (remote/link.rs ~293-299); no redial until restart. (3) Linking an older desk shows 'unknown variant … expected one of' with every command name (link.rs ~127-131): map to 'Update that desk to link with it.' (4) noise_keys (remote/v2.rs ~90) regenerates a missing key unless phones are paired; count link records too, or the linking desk silently ends its links. (5) link() timing out after the peer accepted leaves a trusted record there and nothing dialing here. (6) LinkedTeammate.state docs omit 'offline' (link.rs ~55, session/links.rs ~70).
