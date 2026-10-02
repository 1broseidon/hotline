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
| `voice.audio` | Negotiated by `audio/pcm` in the call's input list. Existing WAV/AAC utterances remain available. | PCM16 little endian, mono, 16 kHz. At most 32 KiB per chunk and 20 seconds per turn. Sequence and chunk index increase. Empty final chunk commits. No partial transcript can dispatch work. |
| `clip` events | Existing independently playable WAV/MP3 clips, increasing indices; last chunk has `final: true`. | Bounded producer channel; cancellation drops provider work and rejects stale output. Whole-clip fallback for other providers/clients. |
| Direct replies | Committed acknowledgement/report messages only. | Internal call/turn origin follows queued or accepted steering inputs. Unrelated agent output cannot enter a direct call. |

Changes to optional fields are additive (R1); routing, streaming and reply
attribution change behavior (R2). Core owns origin and seat enforcement;
clients cannot claim voice origin through a normal session command.

Native xAI API keys support live PCM transcription and progressive PCM speech.
Other providers retain their existing whole-clip adapters. xAI subscription
logins are not substituted for paid speech API keys.

Server logs measure accepted utterance to first audio publication. Client
timing measures the end of capture to playback start. Provider and device
latency still require live measurement; tests establish ordering and
cancellation rather than claiming a particular latency reduction.

## Verification and rollout

Headless wire/session tests cover desk compatibility, target validation,
owner/companion boundaries, stream bounds, commit/cancellation and reply
isolation. Provider protocol tests use local fixtures. UI and mobile tests
cover negotiated fallback, target pinning and ordered playback.

Release the core/server and desktop first. Mobile uses capabilities and the
native AudioStream feature check; older desks and installed runtimes retain
the existing AAC call path. An OTA must not assume a newly installed native
module. Device audio routing and real provider performance need an iPhone
call after rollout.
