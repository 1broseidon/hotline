---
id: task-54
title: "Voice: first audio in under a second — the ack race, semantic endpointing, streamed TTS, and the contract on every provider"
priority: medium
tags:
  - research
  - voice
relatedFiles:
  - ui/src/voice/turn.ts
  - ui/src/voice/call.ts
  - crates/hotline-core/src/voice/spoken.rs
  - crates/hotline-core/src/voice/dispatcher.rs
  - crates/hotline-core/src/voice/speech/openai_shape.rs
  - crates/hotline-core/src/voice/speech/xai.rs
  - crates/hotline-core/src/voice/assets/README.md
  - docs/voice.md
  - docs/voice-calls.md
subtasks:
  - id: task-54-1
    title: Measure where the 4–5 s goes per provider and STT
    completed: false
  - id: task-54-2
    title: Prototype the ack race with pre-rendered per-voice clips and measure
    completed: false
  - id: task-54-3
    title: Feasibility of LiveKit's turn-detector model on the live transcript
    completed: false
  - id: task-54-4
    title: Streamed TTS on the OpenAI-shape adapter
    completed: false
  - id: task-54-5
    title: Call Codex Gemini CLI Grok Build and Hotline Agent and read voice-replies.json
    completed: false
  - id: task-54-6
    title: Decide if speech-to-speech has a place in the switchboard only
    completed: false
createdAt: "2026-10-14T14:41:07.344Z"
updatedAt: "2026-10-18T23:40:58.756Z"
completedAt: "2026-10-18T23:40:58.756Z"
---

## Description
Research task from the voice-reply discussion on 2026-10-14 (The Dude, with Grok Bot's framework survey verified against current vendor docs). Full write-up with quotes and URLs: Hotline workspace `research/voice-reply-research.md` (The Dude's room).

## What we know

Hotline already is the cascade LiveKit Agents and Pipecat sell: turn detector → STT → the teammate's own session → TTS, with interruption that stops speech and never the turn, narration between tools not said, the report said when it lands (their idle gate), the blip as the filler, and the `<spoken>` / `<written>` contract as the dual-length reply no vendor documents. Neither framework fits the desk: both own the LLM loop and the tools, and ours is an ACP process or the rig driver with its own grants, approvals, chapters and budget. A speech-to-speech model (GPT-Live 1, GA 2026-09-10; Gemini 3.8 Live, GA 2026-09-15) is a quick voice in front of a backend agent, which 0.36 removed on purpose; BRO-172 stays cancelled. The one place it could fit is the switchboard (BRO-273).

The problem is first audio. Measured on direct calls to Claude Code with xAI speech: utterance to first clip 2.6–3.9 s desk-side, of which TTS first byte is 250–330 ms; the rest is the model writing its first spoken sentence. Add the client's fixed 1200 ms silence wait (`SILENCE_MS`, ui/src/voice/turn.ts) and the person hears the first word 4–5 s after they stop talking. Sourced thresholds: humans turn-take at ~200 ms (Stivers 2009); >700 ms feels unnatural (ElevenLabs), >1 s (Deepgram, Nielsen), >4 s degrades the call even with fillers (Maslych, CUI '25). Target band from Pipecat, Hamming and ElevenLabs: first audio ≤500 ms p50, 800 ms ceiling, 1 s = play something canned.

## Questions to answer, each with a number

1. **Where the 4–5 s goes, per provider.** Split a turn into client silence wait, STT, model first spoken sentence, TTS first byte, transport. The desk logs `utterance to first clip` and `speak … first byte`; the window logs `endpoint to audible start`. Collect for Claude Code, Codex, Gemini CLI, Grok Build and Hotline Agent on two providers, local Parakeet vs xAI live STT.
2. **The ack race.** Pre-render a few short lines per persona voice when the voice is picked ("On it", "One sec", "Let me look"), cached beside `voice/assets/`. At ~500 ms after the endpoint, if no spoken sentence has arrived, say one; the model's own line follows as the next sentence, deduped the way `spoken::lazy` spots a repeated opener. On the phone, ship the clips with the `call_start` answer so the ack plays locally. This is Vapi's `request-start` and LiveKit's `session.say` in Hotline's clip model: no provider dependency, no model round-trip. Prototype, measure first audio before and after, and check it never doubles up with a model that writes its one short line fast.
3. **Semantic endpointing.** Replace the fixed 1200 ms with LiveKit's open turn-detector model (ONNX, reads the live transcript, lets the gap float 0.3–2.5 s). We have live words from Parakeet, Apple and xAI, and sherpa-onnx already bundles onnxruntime. Check licence, size, English vs multilingual, and what it saves on a typical turn.
4. **Streamed TTS on the OpenAI-shape adapter.** Only xAI implements `speak_chunks`; OpenAI's `/audio/speech` streams too. Google does not, leave it whole-clip per sentence. Measure first byte to first playable chunk.
5. **Contract coverage.** `voice-replies.json` shows Claude Code opus 14 both / 2 spoken-only and Muse 6/6; Codex, Gemini CLI, Grok Build and Hotline Agent models have no counts. Call each and read the counts; the contract is the whole multi-provider bet, and the switchboard rests on it.
6. **Speech-to-speech, if ever.** Decide whether GPT-Live 1 or Gemini 3.8 Live has a place as the switchboard's router voice only, metered per minute on the call assistant's line, or not at all. Gemini 3.8 Live is audio-only output and `turn_complete=true` cuts any speech; GPT-Live is $0.05/min.

## Not in scope

Spoken fillers beyond the blip (LiveKit's `with_filler`): the model's one short line before a tool already covers it, and the research found fillers piling up is a real pitfall. Pre-emptive generation on a partial transcript: an ACP turn cannot be cancelled cheaply.

## Done when

Numbers for 1 and 5, a prototype measurement for 2, a feasibility call on 3 and 4, and a decision on 6, written into docs/voice.md › Timing, with the follow-up tasks filed.

Closed 2026-10-18 (backlog triage): The live voice fronts (0.38/0.39) replaced the transcribe-then-speak pipeline this was about. SILENCE_MS remains only for the Legacy voice fallback.
