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
| `voiceListenWhileThinking` capability; `state.listening` | Additive; `listening: true` appears only on a direct call's `thinking` once the person's words are with the teammate, and is omitted otherwise. An older client ignores it and keeps the microphone shut while thinking. | The desk accepts an utterance whenever no earlier one is still being taken, so the flag only tells a client it may listen. A client listens through it unless speech is playing, keeps its blip-blip out of its detector, and does not play over the person while they talk. |
| `VoiceStatus.directAvailable` | Additive; older desks omit it, so clients fall back to `available`. Desk `available` still includes dispatcher readiness. | Direct readiness requires speech and budget, independent of the dispatcher. Owner/local desk seat enforcement is unchanged. |
| `VoiceStatus.replies` | Additive and read-only; absent until a reply has been counted, and an older client ignores it. One entry per model (`hotline/<provider>/<model>`, `acp/<adapter>[/<model>]`) counting the replies a direct call said as `both`, `spokenOnly`, `unclosed` or `untagged`. | Counted once per reply that reached its end, in `voice-replies.json` under the data directory, written atomically; each count logs one `[voice]` line. |
| `voiceTextInput`, `voice.status.inputMode`, `voice.call_start.inputMode` | Additive; omission retains audio. A text call echoes `inputMode: "text"` and accepts `text/plain`. | Output-only readiness needs no STT configuration. A retained call ID cannot switch target or input mode. |
| `voice.text` | One finalized device transcript `{callId,seq,text}` per turn on a negotiated text call. | Owner/local desk, opening connection, increasing sequence, pending-turn gate and cancellation remain enforced. Nonblank, at most 8,000 characters/32,000 UTF-8 bytes. No remote STT request or spend. |
| `voice.audio` | Negotiated by `audio/pcm` in the call's input list. Existing WAV/AAC utterances remain available. | PCM16 little endian, mono, 16 kHz. At most 32 KiB per chunk and 20 seconds per turn. Sequence and chunk index increase. Empty final chunk commits. No partial transcript can dispatch work. |
| `clip` events | Existing independently playable WAV/MP3 clips, increasing indices; last chunk has `final: true`. Native xAI starts with a 200 ms clip, then keeps half-second chunks and the final tail. | Bounded producer channel; cancellation drops provider work and rejects stale output. Whole-clip fallback for other providers/clients. |
| Direct replies | The teammate's own reply, its `<spoken>` version said as it streams: the first message of a turn as it streams, a report when it lands; narration between tools is not said. Text before the first tag is said as a reply with no tags is. Without tags, the opening up to the first code block or table, at most three sentences. Said text is cleaned for speech. One message is one `said` id and one thread line, closed by an empty final clip. No `delivery` event: the teammate is the voice. | Internal call/turn origin follows queued or accepted steering inputs. Unrelated agent output cannot enter a direct call. A reply to any turn of this call up to the latest is accepted. A partial tag at a chunk edge is held back and never said. |
| Direct call record | A direct call is a thread: its person and voice lines are written to `calls/<callId>.jsonl` behind the call, and the DM carries one `call` marker (`id`, `ts`, `callId`, `title`, `status` `live` or `ended`, `durationMs`, `outcome`) rewritten as the call goes. | Additive; a client that does not know the kind skips it (the phone does). A write never waits on the call's path to the ear. Desk calls stay on the dispatcher tape and have no marker. |
| Direct turns | No front: every utterance is a turn of the teammate's own session, in its open chapter, steering a turn still running. The driver is handed the words and then the contract asking for the answer twice, `<spoken>` then `<written>`; the tape and chat keep the words alone. The chat shows the written version; the reply's first agent event carries the spoken version as optional `spoken`, which an older client ignores. A history rebuilt from the tape shows the model both versions, tagged. The call is `thinking` until the turn ends or parks on subagents, re-sent every 15 s, with `listening: true` once the words are handed over, so the person can speak without a tap; speaking then, or `voice.interrupt`, stops what the call is saying without stopping the turn. Every agent message, on a call or not, is shown, written and pushed as its written version, without a tag. The call assistant is for desk calls only. | The contract is turn text, not a setting, so Hotline Agent and ACP agents get it alike. Only a direct call's origin adds it; a desk call's handoff and typed input never carry it. A direct call names no call-assistant spend. |

Changes to optional fields are additive (R1); routing, streaming and reply
attribution change behavior (R2). Core owns origin and seat enforcement;
clients cannot claim voice origin through a normal session command.

Native xAI API keys support live PCM transcription and progressive PCM speech.
Other providers retain their existing whole-clip adapters. The separate explicit
`xai-subscription` speech choice uses the stored Grok login, resolves fresh OAuth
credentials per request, and retries once after a rejected bearer. It is never
selected automatically and cannot use a paid speech fallback. Provider-side
subscription limits remain applicable.

Progressive xAI output publishes the first 200 ms once one further PCM16 sample
is buffered. It no longer waits for a full second of audio before publishing.
The retained sample keeps the final clip nonempty when the response ends at the
first boundary; shorter responses still publish one whole final clip. Later
chunks retain a half-second tail. This changes clip duration (R2), with the same
WAV format, ordering, final marker, budget and cancellation contracts.

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
