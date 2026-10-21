# Live calls

Calling the desk opens a call thread on its team lead, with that teammate's
own voice front, model and grants. The lead looks up the roster itself and
hands a named teammate the person's words through `message_teammate` with
`handoff`, then says who has it. No dispatcher runs for that routing. A direct
call to the lead remains an ordinary teammate call, without the desk's routing
grant. Hanging up, holding or interrupting speech does not cancel work already
started.

Without a lead, the desk retains its fallback call assistant and bounded
handoff, conversation and schedule tools. There is no `desk_command` or path
from these tools into the wire command table. Its transcript belongs to that
call, never a synthetic teammate tape. Both prompts read the optional
`personName` room setting; the fallback front also receives the roster.
Historical dispatcher tapes remain private to owner seats.

## Wire contract ledger

| Entry | Compatibility | Enforcement |
| --- | --- | --- |
| `voice.call_start` | Optional `personaId` selects a teammate; omitted selects the desk. Optional `streamAudio` enables progressive sentence audio. | Owner/local desk only. Validate target before replacing an existing call. Echo the target; reject reuse with a different target. |
| `voiceDirectCalls` capability in remote hello and `voice.status.capabilities` | Additive; clients require it before sending a target. | A client must check the echoed target to avoid an older desk silently routing the call to its dispatcher. |
| `voiceListenWhileThinking` capability; `state.listening` | Additive; `listening: true` appears only on a direct call's `thinking` once the person's words are with the teammate, and is omitted otherwise. An older client ignores it and keeps the microphone shut while thinking. | The desk accepts an utterance whenever no earlier one is still being taken, so the flag only tells a client it may listen. The desk client listens through it and through speech with echo cancellation, receives the core's single waiting clip, and does not play over the person while they talk. |
| `voiceFront`, `voice.status.liveSelected`, `voice.call_start.mode`, reply `front` | Selection is separate from availability. Omitted mode follows Settings; `front` requires live and `pipeline` is an explicit Legacy choice. A missing front never authorizes automatic Legacy. | Owner/local desk only. The selected mode is enforced before a call is created. A live call refuses all Legacy inputs, including after failure. |
| `voice.front_audio` `{callId,seq,data}` | Microphone PCM16 little endian, mono, at the negotiated `inputRate`, at most 32 KiB, base64. `seq` rises (gaps are allowed: a dropped frame is just lost audio). Chunks of about 20-100 ms are the intent. | Never waits: a front still connecting, or whose bounded queue is full, refuses the frame with an error and the client drops it and sends the next. A held call refuses audio. |
| `frontAudio` `{seq,data}` event | The front's speech as PCM16 mono at `outputRate`, in frames of at most 100 ms. `seq` rises by one from 0, so a gap tells the client the desk lost frames. | Only a front call emits it; held calls emit none. A client too slow to read the call's event queue is disconnected, as for clips. |
| `voice.front_barge` `{callId,playedMs}`, `frontStop` event | Barge-in. When the provider hears the person over the front, the desk sends `frontStop`: the client stops and clears playback, then answers with `voice.front_barge` and the milliseconds actually played. A client that detects speech first may send `voice.front_barge` at once. The front keeps only `playedMs` of the interrupted reply. | Owner/local desk. The call returns to `listening`; the teammate's work is not touched. |
| `state.liveError`; older `frontEnded` event | A failed live call keeps its reason in the ended state and its snapshot. The window offers **Call with Legacy voice** on the same thread. An old `frontEnded` stops the client instead of converting it. | Recovery preserves context and is bounded; a revoked login is never reopened. No Legacy call is made until explicitly requested. |
| Front call states | `listening` while the front waits, `speaking` while it has audio going out, `thinking` with `listening: true` while the teammate works (from the moment the front's `work_on_it` is accepted until the turn ends, as on a pipeline call). A command the teammate leaves running ends its turn at the reply ([running processes](sessions.md#running-processes)), so the call goes `listening` as the reply is said, and while the command runs the call re-sends no `thinking` and the front is given no progress line. `heard {seq,text}` carries what the person said, as the front heard it, and `said {id,text}` what the front said, once its reply is whole, under an id starting `front-`; the teammate's own spoken lines still arrive as `said` under their reply's id, and the desk synthesizes no clips for them: the front says them. | Same `state` events as a pipeline call; an older client sees no new states. |
| `VoiceStatus.directAvailable` | Additive; older desks omit it, so clients fall back to `available`. Desk `available` includes fallback-assistant readiness only when there is no lead. | Direct readiness requires speech and budget, independent of the dispatcher. Owner/local desk seat enforcement is unchanged. |
| `VoiceStatus.replies` | Additive and read-only; absent until a reply has been counted, and an older client ignores it. One entry per model (`hotline/<provider>/<model>`, `acp/<adapter>[/<model>]`) counting the replies a direct call said as `both`, `spokenOnly`, `unclosed`, `untagged` or `rewritten`. | Counted once per reply that reached its end, in `voice-replies.json` under the data directory, written atomically; each count logs one `[voice]` line. One short line with no spoken version, said as written, is not counted. |
| `voiceTextInput`, `voice.status.inputMode`, `voice.call_start.inputMode` | Additive; omission retains audio. A text call echoes `inputMode: "text"` and accepts `text/plain`. | Output-only readiness needs no STT configuration. A retained call ID cannot switch target or input mode. |
| `voice.text` | One finalized device transcript `{callId,seq,text}` per turn on a negotiated text call. | Owner/local desk, opening connection, increasing sequence, pending-turn gate and cancellation remain enforced. Nonblank, at most 8,000 characters/32,000 UTF-8 bytes. No remote STT request or spend. |
| `voice.audio` | Negotiated by `audio/pcm` in the call's input list. Existing WAV/AAC utterances remain available. | PCM16 little endian, mono, 16 kHz. At most 32 KiB per chunk and 20 seconds per turn. Sequence and chunk index increase. Empty final chunk commits. No partial transcript can dispatch work. |
| `clip` events | Existing independently playable WAV/MP3 clips, increasing indices; last chunk has `final: true`. Native xAI starts with a 200 ms clip, then keeps half-second chunks and the final tail. | Bounded producer channel; cancellation drops provider work and rejects stale output. Whole-clip fallback for other providers/clients. |
| Direct replies | The teammate's own reply, its `<spoken>` version said as it streams: the first message of a turn as it streams, a report when it lands; narration between tools is not said. Text before a spoken version is held, and said just before it as a reply with no tags is. Without a spoken version, nothing is said while the reply streams; once it is whole, one short line is said as written, and anything longer is said as the call assistant's rewrite (stored as the reply's `spoken`, counted `rewritten`), or, with no call assistant, a refused Chat budget, a failure or no answer in 6 s, as its opening up to the first code block or table, at most three sentences (counted `untagged`). The call is `thinking` while the rewrite is written. Said text is cleaned for speech. One message is one `said` id and one thread line, closed by an empty final clip. No `delivery` event: the teammate is the voice. | Internal call/turn origin follows queued or accepted steering inputs. Unrelated agent output cannot enter a direct call. A reply to any turn of this call up to the latest is accepted. A partial tag at a chunk edge is held back and never said. |
| Direct call record | A direct call is a thread: its person and voice lines are written to `calls/<callId>.jsonl` behind the call, and the DM carries one `call` marker (`id`, `ts`, `callId`, `title`, `status` `live` or `ended`, `durationMs`, `outcome`) rewritten as the call goes. | Additive; a client that does not know the kind skips it (the phone does). A write never waits on the call's path to the ear. Desk calls with a lead use the same thread record; a no-lead fallback keeps its own call stream without a DM marker. |
| Direct turns | Live voice answers conversation and read-only lookups (`search_thread`, `read_thread`, `list_chapters`, `list_schedules`, `web_search`, `list_teammates`) itself. The desk lead also has the bounded handoff tool described above. `work_on_it(words, action)` starts or steers one clone on the attached thread, using the teammate's model, harness and grants; `action: "stop"` cancels its work and queued changes. It returns after admission, while the person keeps talking and reads keep answering. The whole call is the brief, so “write that down” includes earlier conversation. The clone posts full drafts or files with a subject and returns a short spoken result; the chat pointer is said once only when there is a post. GPT-Live's client delegation invokes this same work tool. The front has no raw MCP or posting tools and no call-assistant delegation loop. Both speakers and all tools are kept on the call thread. Live speech updates fold under one id and are buffered until completion or a work boundary. Clone prose is held until the turn ends, then kept as one Work result tool entry; only the answer after the last tool is offered to the voice, never the clone’s opening acknowledgement. What the voice actually says is recorded separately once. An interruption notice identifies the cut generated line without claiming what the caller heard; the next turn receives playback context separately from the caller’s saved words. The home session is never interrupted. `remain_silent` lets an incomplete utterance continue; dictation instructions explicitly wait through “draft…”, “say…” and “write…”. GPT Realtime uses semantic turn detection with low eagerness; Grok and GPT-Live retain provider turn detection and the listening instructions. Real pauses are checked in task-206's scenario pass. Legacy voice feeds each utterance to the shared thread occupant with the spoken/written contract. Hang-up detaches speech while accepted work finishes; only the in-flight turn automatically posts its completed reply. The room posts a closing note once the thread is idle, then parks it. | Both drivers use the same thread policy and turn runner. Standing grants and the independently revocable thread lease govern work. Live provider failure never silently selects Legacy. |

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

## Call assistant metering

The call assistant's model calls spend the Chat budget. Before each completion
dispatch, one hook reserves an estimate of the complete request and output
ceiling, keyed by Rig's `EffectId`; a budget refusal prevents that dispatch.
The same hook handles unary and streamed replies. Each outcome settles only
its own reservation to reported token cost, including attempts rejected and
retried within a turn: the provider bills both attempts.

A provider failure, missing usage, all-zero usage, or a stream dropped before
its outcome keeps the estimate charged, since the request may have been billed.
Tool dispatches do not reserve model spend. A reported cost above the budget
is recorded in full and the paid reply remains available; the next completion
dispatch is refused, whether in this turn or a later request.

## Verification and rollout

Headless wire/session tests cover desk compatibility, target validation,
owner/companion boundaries, stream bounds, commit/cancellation and reply
isolation, plus output-only text configuration, sequence/mode/bounds and no STT
spend. Provider protocol tests use local fixtures. UI and mobile tests
cover refused live starts, context-preserving recovery, explicit Legacy choice, target pinning and ordered playback.

Release the core/server and desktop first. Mobile uses capabilities and native
speech-recognition/AudioStream feature checks; older desks and installed runtimes retain
the existing AAC call path. An OTA must not assume a newly installed native
module. Device audio routing and real provider performance need an iPhone
call after rollout.


Legacy call threads (S5) detach on hang-up without cancelling the agent. The
room posts a closing note after 30 seconds and an idle queue, then parks the
open thread. There are no periodic summaries or front Digest. Calling a work
thread or a previous call passes `thread` to `voice.call_start`; its context
and agent are reused. Live-front work uses that same occupant and steers it without starting another clone.
Only the turn running at hang-up automatically posts its completed written
reply, as one message. Over 900 characters or three paragraphs, that post
is a short subject-first summary; a failed summariser leaves a bounded
notice instead. Open the call leads to the full reply. Later turns stay on
the thread unless they explicitly post.
The source-thread title labels posts (untitled calls say “From the call”);
posts and closing notes open that thread, and replying to a post routes there. Silent posts made during the
call never become unread later; posts after hang-up notify as replies do.

## Identity, turn choices and acknowledgements

The live prompt carries the teammate's name and goal/manner, 4,000 characters
of chapter context and up to 32 recent chat lines within 12,000 characters.
It chooses small talk, questions, status, exploration, rundown, work, steering,
dictation, cards and routing itself. State, code and results require a lookup.
Acknowledgements name the request briefly when needed, never a hand-off or a
repeated stock phrase. Replies are short by default; an explicit overview
relaxes length for that answer only, in numbered parts with a check-in.

`session.update` refreshes running work, the call so far, recent already-said
lines and actual chat posts at turn/tool boundaries and on a half-second
change check. Context is bounded and scrubbed; interruption notices distinguish
generated words from what the caller heard. Small talk stays on the call thread.
Legacy streams its own opening and reply, with one core-supplied soft sound
if it has not begun speaking after 1.8 seconds. No client repeating chime or
live progress filler is generated.

`realtime::scripted_front` runs a fixed spoken deck against each live model,
rejecting relay wording and repeated acknowledgements within one call. The
provider runs require explicit test credentials; the checker and local wire
regressions run normally. A speech-to-speech model's audio cannot be filtered
before it is heard: prompt, tool framing and this check are the enforcement.
