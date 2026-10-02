# Live calls

The desk remains the primary call target. A conversation can also call its
teammate through the same session, chapter, harness and standing grants used
by a typed operator message. Hanging up, holding or interrupting speech does
not cancel accepted teammate work.

## Wire contract ledger

| Entry | Compatibility | Enforcement |
| --- | --- | --- |
| `voice.call_start` | Optional `personaId` selects a teammate; omitted selects the desk. Optional `streamAudio` enables progressive sentence audio. | Owner/local desk only. Validate target before replacing an existing call. Echo the target; reject reuse with a different target. |
| `voiceDirectCalls` capability in remote hello and `voice.status.capabilities` | Additive; clients require it before sending a target. | A client must check the echoed target to avoid an older desk silently routing the call to its dispatcher. |
| `VoiceStatus.directAvailable` | Additive; older desks omit it, so clients fall back to `available`. Desk `available` still includes dispatcher readiness. | Direct readiness requires speech and budget, independent of the dispatcher. Owner/local desk seat enforcement is unchanged. |
| `voiceTextInput`, `voice.status.inputMode`, `voice.call_start.inputMode` | Additive; omission retains audio. A text call echoes `inputMode: "text"` and accepts `text/plain`. | Output-only readiness needs no STT configuration. A retained call ID cannot switch target or input mode. |
| `voice.text` | One finalized device transcript `{callId,seq,text}` per turn on a negotiated text call. | Owner/local desk, opening connection, increasing sequence, pending-turn gate and cancellation remain enforced. Nonblank, at most 8,000 characters/32,000 UTF-8 bytes. No remote STT request or spend. |
| `voice.audio` | Negotiated by `audio/pcm` in the call's input list. Existing WAV/AAC utterances remain available. | PCM16 little endian, mono, 16 kHz. At most 32 KiB per chunk and 20 seconds per turn. Sequence and chunk index increase. Empty final chunk commits. No partial transcript can dispatch work. |
| `clip` events | Existing independently playable WAV/MP3 clips, increasing indices; last chunk has `final: true`. | Bounded producer channel; cancellation drops provider work and rejects stale output. Whole-clip fallback for other providers/clients. |
| Direct replies | Committed acknowledgement/report messages only; with a dispatcher, the turn's final reply only, retold in the first person when long. | Internal call/turn origin follows queued or accepted steering inputs. Unrelated agent output cannot enter a direct call. Replies to any of the call's last 16 handed-off turns are accepted. |
| Direct front | With a dispatcher, the dispatcher model answers as the teammate and hands work to the session through `hand_to_session`. | At most one handoff per utterance. Status questions reach no session. A front failure before handoff forwards the words unchanged. |

Changes to optional fields are additive (R1); routing, streaming and reply
attribution change behavior (R2). Core owns origin and seat enforcement;
clients cannot claim voice origin through a normal session command.

Native xAI API keys support live PCM transcription and progressive PCM speech.
Other providers retain their existing whole-clip adapters. The separate explicit
`xai-subscription` speech choice uses the stored Grok login, resolves fresh OAuth
credentials per request, and retries once after a rejected bearer. It is never
selected automatically and cannot use a paid speech fallback. Provider-side
subscription limits remain applicable.

Server logs measure accepted utterance to first audio publication. Client
timing measures the end of capture to playback start. Provider and device
latency still require live measurement; tests establish ordering and
cancellation rather than claiming a particular latency reduction.

## Verification and rollout

Headless wire/session tests cover desk compatibility, target validation,
owner/companion boundaries, stream bounds, commit/cancellation and reply
isolation, plus output-only text configuration, sequence/mode/bounds and no STT
spend. Provider protocol tests use local fixtures. UI and mobile tests
cover negotiated fallback, target pinning and ordered playback.

Release the core/server and desktop first. Mobile uses capabilities and native
speech-recognition/AudioStream feature checks; older desks and installed runtimes retain
the existing AAC call path. An OTA must not assume a newly installed native
module. Device audio routing and real provider performance need an iPhone
call after rollout.
