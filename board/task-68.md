---
id: task-68
title: "Desk: a direct call's front is the teammate, in the first person"
column: review
position: 5
priority: high
tags:
  - voice
  - calls
  - 0.38.0
relatedFiles:
  - crates/hotline-core/src/voice/front.rs
  - crates/hotline-core/src/voice/front_prompt.rs
parentId: epic-6
createdAt: "2026-10-15T18:50:10.567Z"
---

## Description
Done 2026-10-15 (f3a6f917, f89377eb), QA'd by George on Grok and GPT Realtime. On a call to a teammate the front speaks as that teammate: acknowledges once as it sends ('One sec, I'll pull that up'), never says it passed something on or is waiting on someone; progress and needs-you lines say 'I' and rotate. A send asks for no spoken reply, so there is one ack per request; progress lines back off 45 s, 90 s, then every 3 min. Desk calls still speak for the desk. Files: voice/front.rs, voice/front_prompt.rs, voice/duplex.rs.
