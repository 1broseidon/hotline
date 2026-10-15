---
id: epic-6
title: "Live calls get a voice front: Grok Voice / GPT-Live talk, the teammate works (0.38)"
type: epic
column: todo
position: 36
priority: high
tags:
  - feature
  - voice
  - calls
  - 0.38.0
createdAt: "2026-10-14T20:03:32.697Z"
assignee: claude
updatedAt: "2026-10-15T01:56:09.941Z"
lead:
  agent: claude
  model: sonnet
  effort: medium
  by: person
  at: "2026-10-15T01:56:09.669Z"
  until: done
---

## Description
Target: 0.38.0 (moved from 0.37 on 2026-10-14 so 0.37 could be cut). George's 2026-10-14 call. Speech-to-speech models are the next wave (Grok Voice 3.0 shipped 2026-10-14, $0.08/min API; GPT-Live-1 $0.05/min) and both run on subscriptions he already pays for.

Decision: reverses 'No front' in docs/voice-calls.md (Direct turns). A realtime voice model holds the conversation; the teammate (Hotline Agent or any ACP harness) stays the one that works. This is Codex's own design (openai/codex 71923ba: realtime model = persona, agent = hidden backend; prompts 'Delegate all user requests', 'Treat backend outputs as authoritative'; agent told it is 'behind an intermediary' and input is a transcript; client, not model, picks what is spoken; voice transcript kept apart from agent history). The current STT -> teammate -> TTS pipeline stays as the fallback and for providers without a realtime model.

Spike 2026-10-14 (/tmp/rt-spike, not committed):
- Grok: ~/.grok OIDC login opened wss://api.x.ai/v1/realtime?model=grok-voice-latest (documented API), session.update + text in -> streamed audio + transcript out. No API key.
- OpenAI: ChatGPT login (Codex auth) POST chatgpt.com/backend-api/codex/realtime/calls?intent=quicksilver&architecture=avas, model gpt-live-1-codex, JSON {sdp, session{delegation:{type:client}}}, header openai-alpha: quicksilver=v2 -> 201, rtc_ call id, WebRTC connected, model spoke. Private Codex route; direct WS needs an API key. Codex adds an attestation header when it has one.
Unverified: subscription usage limits/billing, tool calling, mic input, barge-in quality.

Design: desk opens the Grok realtime socket and owns the front (tools, prompt, relay); phone streams mic PCM up and plays PCM down over the existing sealed wire. Front tools: hand the person's own words to the teammate (start-or-steer its turn in its open chapter, unrephrased), remain silent. Teammate output returns as progress notes plus its <spoken> line, which the front says verbatim; the front never claims work the teammate didn't report. GPT-Live is the second provider: desk creates the call and holds the sideband, phone holds WebRTC media.

Sequence: decision record -> provider seam + Grok adapter -> bridge + prompts -> call wire -> phone duplex audio (hotline-mobile board) -> recording/ledger -> settings -> GPT-Live (stretch).

Done when a phone call to a teammate runs on Grok Voice with sub-second replies and barge-in, work still happens in the teammate's session, the call record and tape stay truthful, the old pipeline still works when the front is off, and 0.38.0 release notes describe it.
