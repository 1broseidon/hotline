---
id: epic-6
title: "Live calls get a voice front: Grok Voice / GPT-Live talk, the teammate works (0.38)"
type: epic
column: review
position: 0
priority: high
tags:
  - feature
  - voice
  - calls
  - 0.38.0
createdAt: "2026-10-14T20:03:32.697Z"
assignee: claude
updatedAt: "2026-10-16T01:06:10.024Z"
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

## Log
- 2026-10-15T01:57:10.875Z: [claude] PLAN (lead, 2026-10-14). Order: [57 docs ‖ 58 realtime seam+Grok] -> 59 bridge -> [60 prompt ‖ 61 call wire] -> [62 record/cost ‖ 63 settings] -> 64 GPT-Live (stretch). Why: 58 owns the Realtime trait everything else uses. 59 owns voice/front.rs and voice/mod.rs, so 61 waits on it (added task-59 to 61's dependsOn) rather than collide in voice/mod.rs. 60 is a new file (front_prompt.rs) and runs beside 61. 62 touches record/ledger/mod.rs and waits on 61 (added). 63 changes VoiceStatus/settings and regenerates ui/src/generated/contract.ts, so it waits on 61 (added) to avoid conflicting regenerations; 62 and 63 may both touch contract.ts, so whoever lands second must rerun unfiltered cargo test -p hotline-core. 64 waits on 61 and 63 (added). Every child now has relatedFiles and a contract: fmt, clippy -D warnings, UNFILTERED cargo test -p hotline-core (AGENTS.md hazard 3), plus ui typecheck/test where UI changes. Phone duplex audio lives on the hotline-mobile board, outside this epic; 61 defines the wire it needs. Agents: opus on 58/59/61 (protocol and concurrency), sonnet on the rest. Open risk: Grok function calling, server VAD barge-in and PCM rates are unverified; 58 must confirm live and note findings, and if they fail, stop and replan before 59.
- 2026-10-15T03:57:56.980Z: [claude] 2026-10-14 review: 57-63 were merged only into a stale local main (57-59) or left on unmerged branches (60-63). Rebuilt as release/0.38.0 from origin/main with 57-63 merged in order; ADR renumbered 0004 (0003 is linked desks). Integration and live test missing: new child task added. Local main backed up as backup/local-main-2026-10-14, to be reset to origin/main. dispatch contract validate resolves paths in the main checkout, so it reports deliverables 'not found' before merge.
- 2026-10-15T05:33:08.598Z: [claude] 2026-10-15: desktop path (task-67) on release/0.38.0; phone path (hotline-mobile task-30) on feat/voice-front, installed on George's iPhone. Both default to the front when the desk offers it. Morning QA: desk + phone against the dev window.
- 2026-10-15T18:50:18.759Z: [claude] 2026-10-15 day: George QA'd Grok Voice on desk (headset) and phone (speaker): near-instant replies, barge-in by voice works on speaker, echo cancellation holds. Phone PR hotline-mobile#75 (feat/voice-front, stacked on #72). Added on release/0.38.0 and QA'd: first-person front on direct calls with one ack per request (new child), Live calls Model picker grouped by provider (new child), GPT Realtime on an OpenAI key incl. the relay-loop fix (new child). Built, pending QA: GPT-Live on the ChatGPT login (task-64, desk holds WebRTC; live test 3/3). OpenRouter has no speech-to-speech API (open FR). Remaining before release: George's GPT-Live QA, then reconcile children to done and open the 0.38.0 PR. Next after release: survey every provider for speech-to-speech (Gemini Live first).
- 2026-10-16T01:06:09.400Z: [claude] 2026-10-15 evening: George QA'd the whole package and called it ready for 0.38 (20:02). After the 18:50 note, also built and QA'd on release/0.38.0: GPT-Live's hour-limit renewal (live-tested); agent voice settings follow the active voice path (desk and phone); provider list shows Chat/Images/Voice; the call UI and the new shell (task-73: composer call key, inset call card with face and voice bars, transcript key, Said on the call, written follow-up for a question the call ended on, folded call lines, rail dock with a real search field and the desk's call key, title bar New teammate + Settings, … menu merged into Settings); short tooltips and 2-3 word call trouble with an info key (task-72; decisions ui.no-fake-ui and ui.tooltip-length). Phone (hotline-mobile, feat/voice-front, PR #75 on #72): front voice path in agent settings, 22 hints shortened (task-32), call key in the composer and out of the … menu, Stop a quiet round (task-33); installed on George's iPhone. All children complete. Release prep on release/0.38.0: version 0.38.0, CHANGELOG, docs/releases/0.38.0.md. Done pending the 0.38.0 PR, tag desktop-v0.38.0, and the phone's TestFlight build.
- 2026-10-16T02:53:30.873Z: [claude] 2026-10-15 21:45: George said ship it. Release 0.38.0 committed (52ae9ff2). Codex review on PR #187 raised a real P1 (a ChatGPT-only desk could not start GPT-Live: start_call and voice.status gated on legacy speech) and a real P2 (a refused response.create left the socket mute); both fixed with tests (df1859b5, 8337a38f), CI green, PR #187 merged (c5964d09), tag desktop-v0.38.0 pushed, desktop-release.yml run 38842698038 building. Phone: #72 and #75 merged to main; EAS build 29 (0.5.0) submitted to TestFlight.
