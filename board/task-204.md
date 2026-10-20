---
id: task-204
title: "The live voice is the teammate: its identity, the kinds of turn, and acknowledgements that never sound like a hand-off"
column: backlog
position: 40
priority: high
tags:
  - voice
assignee: claude
relatedFiles:
  - crates/hotline-core/src/voice/front_prompt.rs
  - crates/hotline-core/src/voice/duplex.rs
  - crates/hotline-core/src/voice/assets
parentId: epic-21
createdAt: "2026-10-19T13:59:38.640Z"
dependsOn:
  - task-225
updatedAt: "2026-10-20T20:13:23.286Z"
---

## Description
Scenarios: A quick question on the go, Where are we on it?, Thinking out loud, Give me the rundown, Legacy voice on a slower teammate, and every acknowledgement in the deck. This is George's point 2: today front_prompt.rs tells the voice it hands work to a "working session" through send_to_teammate, and gives it "On it.", "One sec, I'll pull that up." and "Got it, passing that on." to copy.

Done when:
- The live voice is given the teammate's identity: its name, its manner, its recent chat and the call so far. The context goes well beyond today's 600-character handoff and 6 lines, within a budget, scrubbed as today. It is told it is the teammate, with no one behind it to mention.
- The prompt describes the kinds of turn and what each needs, so the live model makes the call itself: small talk, quick question, status, thinking out loud, rundown, real work, steering, dictation, a card, routing. There is no separate decision model.
- The prompt has no stock phrases to copy. It describes a good acknowledgement (a few words naming what was asked), says that often none is needed and the teammate just says what it will do, and lists what never to say: "handing that off", "passing that on", "let me check that for you", a repeat.
- The prompt is refreshed mid-call (session.update) with the call's state: running work, the last few acknowledgements as already said, and what has been posted.
- State, code and results are answered from a lookup, never from memory. A lookup that takes a moment is covered by a few words naming it, never by silence or filler.
- Replies are short by default, and one word when one word is the answer. "I've got time" or "give me the overview" relaxes the length for that answer only, spoken in numbered parts with a check-in.
- Small talk stays on the call's thread.
- On Legacy voice, the first thing said is the agent's own first sentence, before its tools. A turn that says nothing first gets one soft sound, on desk and phone, never filler words. The reply is spoken as it streams and can be interrupted at any time.
- A regression check in the scripted-front tests runs a fixed set of requests on each live model and fails on relay phrasing or on an acknowledgement repeated within a call.

Caveat for the notes: a speech-to-speech model's audio can't be filtered before it is heard, so the prompt, the tool's framing and this check are the enforcement.
