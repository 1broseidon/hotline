# Voice scenario pass — 0.42

Acceptance set: [voice-scenarios.html](voice-scenarios.html). This record does
not replace or weaken it. Task-206 separates deterministic evidence from the
physical pass. **The full scenario pass is pending**, not passed by this branch.
Every original “What must be true” line is copied below for a check or a filed
failure; the boxes record the physical pass, not a claim about test coverage.

## How to record the pass

Pork Chop runs every scenario on the Linux QA computer first; George then runs
every scenario on the desk and phone. For each requirement below, record the
operator, date, build SHA, device, driver, voice provider, call id and result
(or a task id for failure). Check a box only after all applicable cells pass or
have a filed disposition accepted by George. Do not infer phone adoption from
desktop behavior. Use Hotline Agent and ACP, and Grok Voice, GPT Realtime,
GPT-Live and explicitly selected Legacy where applicable. The ignored
`hotline-voice` scripted_front provider tests are optional additional evidence,
not a substitute for this pass and not run by ordinary `make check`.

- Linux QA / Pork Chop: done 2026-10-21 with the results below; failures filed or noted.
- Desk / George: pending complete pass, including browser audio and native Mac recognition.
- Phone / George: pending complete pass, including speaker and relay.

For Interrupting, test speech onset and tap during a long reply on ordinary
and loud speakers, including double-talk. Confirm no self-transcription or
self-interruption, immediate local stop, late audio rejection and a next reply
that answers the correction. Native recognition against webview output is
unverified. Phone capture must remain open through playback and send
`voice.interrupt` or `voice.front_barge` with playback evidence.

For Dictating something, pause within “draft…”, “say…” and “write…”, finish
the sentence, correct the read-back and drop work. Check varied first-person
acknowledgements and exactly one contextual pointer after a real post.
For A bad network, lose the relay for less than and more than 20 seconds,
check same-device resume versus hang-up, explicit hold, and real Reconnecting
state. The Pi incident still needs its host/time and journal diagnosis.
For cards, verify phone `voice.input_state` and `card.resolved` adoption.

## Existing observations and filed gaps

- Pork Chop, 2026-10-21, desk call `61d35a97`: roster lookup answered, but
  recited model/tools/machine reach. This branch changes the lead instruction
  to names and current work; rechecked live on `2735d01a`: "Rex is idle.
  That's the whole team right now." Named work reached Rex
  directly and `rain.txt` was written.
- Pork Chop, 2026-10-21, Grok call `790a5312`: `fox.txt` was written and spoken
  about, closing note worked, file did not reach chat. George subsequently
  chose core-owned delivery in **task-253**. The implementation now delivers
  recorded writes/edits, combines explicit sends, and groups batches above
  three files. Both drivers pass the scripted wire test; confirmation on a
  real-provider call is still pending.
- The deck’s push-approval example is broader than the shipped trusted-card
  boundary. Under decision-15, opaque driver approvals wait for the desk;
  exercise a core-owned session collaboration approval or informational
  question for the allowed path and an opaque push card for the denied path.

## Linux QA pass, Pork Chop, 2026-10-21

Linux QA computer, Hotline Agent teammate Rex on Grok 4.7, Grok Voice
(subscription) unless stated, speech from espeak-ng through a virtual
microphone, so no speaker, echo or phone checks. Builds `1f3a2cae` to
`2735d01a`; each line names the build it was seen on. Boxes below stay open
until George's desk and phone pass.

| Scenario | Result | Seen |
|---|---|---|
| 1. A quick question | Pass. "Capital of Portugal" answered in one line; lookups say a few words first ("Looking up the workspace files."). | `6722376c`, `61d35a97` |
| 2. Where are we on it? | Partial. Three files said, not posted, from a lookup; but the three were wrong (not the newest) and it added more than asked. | `2735d01a` |
| 3. Asking for real work | Pass. First person ("Saving the poem…", "Saved as fox.txt."), no hand-off words, one short acknowledgement, work as a clone on Rex's own model. Pointer said once only when a post exists. | `1f3a2cae`–`790a5312` |
| 4. Changing your mind | Pass. "Stop. Just tell me the keeper's name." steered the running work ("Stopping the story."), no second clone. | `2735d01a` |
| 5. Thinking out loud | Pass. Conversational replies, no work, no pointer; "Write that down" became work with the whole call as its brief, and the note was posted. | `2735d01a` |
| 6. The rundown | Pass for length and numbered parts. Interrupting it with a question: **fail**, see task-254. | `2735d01a` |
| 7. Dictating | **Fail, recovered** (task-255). A 3.5 s pause inside "Draft a message to the harbour master saying that…" ended the turn; work started on the half sentence with guessed content, then the second half steered it. | `2735d01a` |
| 8. Something needs you | Pass. Action card said and left for the desk; question card said ("A question for you: … Say yes or no."), answered "Yes"/"No" by voice and recorded ("Answered on the call: Yes — …"). After the card the voice repeated the ask once. | `2d173ee7`, `a180a43d` |
| 9. Calling the desk | Pass. Roster from list_teammates, now short ("Rex is idle. That's the whole team right now."); named work went straight to Rex ("Rex has it."). The first words of the first line are clipped on desk calls ("… team"). | `61d35a97`, `2735d01a` |
| 10. Hanging up while work runs | Pass. The turn in flight finished after hang-up, its reply posted to the chat, closing note 30 s later. | `2735d01a` |
| 11. Picking it back up | Pass. Calling again on a call's thread kept its context; closing notes cover only new work. | `1f3a2cae` |
| 12. Following up in the chat | Pass. "Like we said on the call" from the home chat used the call's words via read_thread (dog.txt written word for word). Files written on a call don't always reach the chat: task-253. | `e837fc29`, `790a5312` |
| 13. Calling on a work thread | Pass for answering from the work thread's own history; the "From <title>" label not checked. | `a185fff1` |
| 14. Interrupting | Pass when no lookup preceded the speech (cut at once, cut recorded, next reply answers it). Fail after lookups: task-254. Echo on speaker not testable here. | `1f3a2cae`, `2735d01a` |
| 15. A bad network | Not testable on the desk window (calls run in-process; a reload kept the call up). Core wire tests cover the 20 s hold; phone and relay are George's. | `6722376c` |
| 16. Live voice goes away | Not forced: no safe way to break the live provider here. | — |
| 17. Legacy voice | **Fail on the first sentence** (task-256): a tool-heavy turn (web search) went straight to tools with no spoken first sentence for over a minute. The soft cue can't be heard here. The espeak voice was misheard ("Bell Rock" as "Male Rock"), which made the turn long. | `a180a43d` |

## Scripted evidence and requirement checklist

Rust paths below are relative to `crates/hotline-core/tests/` unless stated.
Tests name their scenarios in doc comments. Fixtures establish routing and
records, not whether a provider speaks well or a microphone cancels echo.

### 1. A quick question on the go

Scripted: [`call_threads.rs::closing_note_waits_thirty_seconds_and_for_finished_work_then_parks`](../../crates/hotline-core/tests/call_threads.rs). Closing note timing and parking; real lookup choice and time to first words remain manual.

Device results: Linux — pending; desk — pending; phone — pending.

- [ ] The answer comes from a lookup. A question about state, code or results is never answered from memory.
- [ ] No pointer to the chat: nothing here is in it.
- [ ] The first words follow the question at once; a lookup that takes a moment is covered by a few words naming it ("Let me look"), never by silence.
- [ ] Small talk stays on the call's own thread. The chat gets one closing note, not the exchange.

### 2. Where are we on it?

Scripted: [`voice/direct.rs::conversation_keeps_one_written_record_and_never_posts_or_points`](../../crates/hotline-core/tests/voice/direct.rs). Conversation record and no empty chat pointer; spoken list length and live lookup discipline remain manual.

Device results: Linux — pending; desk — pending; phone — pending.

- [ ] Three items are said, not posted. A list goes to the chat only when it is longer than a few items, or holds links or code.
- [ ] A one-word answer is allowed when one word is the answer.
- [ ] Follow-ups stay on the call; nothing is re-asked of the person.

### 3. Asking for real work

Scripted: [`call_threads/live_work.rs::live_work_uses_its_own_driver_and_posts_its_file_without_touching_home`](../../crates/hotline-core/tests/call_threads/live_work.rs). Both real drivers, all three front identities, explicit file sending, lookups during work and home isolation. Task-253 adds `call_threads/files.rs::call_files_are_delivered_once_and_large_batches_are_one_openable_card` for automatic attachment on both drivers, grouping, deduplication and indexed readback. Real-call confirmation of that change is pending.

Device results: Linux — pending; desk — pending; phone — pending.

- [ ] The voice is the teammate: its name, its manner, its recent chat and the call so far. It says what it will do in its own words, often with no announcement at all ("I'll keep it to a page").
- [ ] Said as the teammate's own action, in the first person. Never "handing that off", "passing that on", "I'll get someone on it".
- [ ] The acknowledgement names what was asked, in a few words, and is not one the call has already used.
- [ ] The person can keep talking while the work runs; lookups still answer.
- [ ] The home agent is never interrupted. The work is done by the teammate's own model and harness, as if typed.
- [ ] The pointer to the chat is said once, because there is a file in it.

### 4. Changing your mind mid-work

Scripted: [`call_threads/live_work.rs::changing_and_dropping_live_work_keeps_one_call_agent_and_home_keeps_running`](../../crates/hotline-core/tests/call_threads/live_work.rs). One occupant, steering and stop on both drivers; live confirmation wording remains manual.

Device results: Linux — pending; desk — pending; phone — pending.

- [ ] A change to running work steers it; it does not start a second clone.
- [ ] The confirmation repeats back what changed, in a few words, so the person knows it was understood.

### 5. Thinking out loud

Scripted: [`voice/direct.rs::conversation_keeps_one_written_record_and_never_posts_or_points`](../../crates/hotline-core/tests/voice/direct.rs). Conversation is recorded as spoken. Whole-call work briefing is also exercised by call_threads/live_work.rs; natural conversational lengths remain manual.

Device results: Linux — pending; desk — pending; phone — pending.

- [ ] No pointer to the chat on a conversational turn, ever.
- [ ] A conversational turn's written version is the spoken words; there is no second answer to invent.
- [ ] Lengths follow the talk: a short reply to a short line.
- [ ] When it ends in "write that down", it becomes the work scenario, carrying the whole conversation as its brief.

### 6. Give me the rundown, I have time

Scripted: [`call_threads.rs::interrupted_speech_is_kept_and_the_next_turn_hears_the_cut`](../../crates/hotline-core/tests/call_threads.rs). Both drivers receive cut context. Numbered spoken rundown, temporary length relaxation and check-in remain manual.

Device results: Linux — pending; desk — pending; phone — pending.

- [ ] "I've got time" or "give me the overview" relaxes the length cap for that answer only.
- [ ] A long answer is spoken in numbered parts with a check-in, so the person can steer it.
- [ ] Interrupting stops the speech at once; the next reply answers the interruption, not the rest of the old one.

### 7. Dictating something

Manual provider pause test required. front_prompt.rs::conversation_and_changes_have_distinct_tools_and_no_handoff_narration checks dictation, summary, stop and no-send instructions only; it cannot prove acoustic turn boundaries.

Device results: Linux — pending; desk — pending; phone — pending.

- [ ] A pause mid-dictation is not the end of the turn. "Draft…", "say…", "write…" open a longer listening window.
- [ ] The draft is read back as a summary, not in full; the full text is in the chat.
- [ ] Nothing is sent on the person's behalf without them saying so.

### 8. Something needs you during the call

Scripted: [`call_threads/cards.rs::live_cards_wait_for_the_caller_and_desk_actions_refuse_spoken_yes`](../../crates/hotline-core/tests/call_threads/cards.rs). Finalized answers and busy-floor ordering; legacy_answers_a_question_once_and_records_yes_and_no_from_the_call covers both drivers. Opaque push approvals remain desk-only under decision-15 despite the illustrative deck dialogue.

Device results: Linux — pending; desk — pending; phone — pending.

- [ ] A card is said once, at a natural break, not over the person.
- [ ] A yes or no by voice answers it, and the answer is written to the chat so there is a record.
- [ ] Anything needing a credential, a 2FA tap or a screen is never done by voice: it is said, and waits for the desk.

### 9. Calling the desk

Scripted: [`voice/lead.rs::the_desk_lead_looks_up_the_roster_and_hands_named_words_straight_on`](../../crates/hotline-core/tests/voice/lead.rs). Explicit lead choice, roster answer on the wire, named handoff, no dispatcher. desk/teammates.rs covers single-teammate auto-lead and clearing on removal; a_no_lead_front_knows_the_person_and_roster covers fallback context. Real concise roster speech must be rechecked.

Device results: Linux — pending; desk — pending; phone — pending.

- [ ] The lead answers roster questions itself, from a lookup.
- [ ] When the person names a teammate, the lead hands the words straight to them; saying who is right here, because routing is the lead's job.
- [ ] No separate dispatcher model when the owner is clear.
- [ ] With no lead set, today's desk call, but it knows the person's name and the roster.

### 10. Hanging up while work runs

Scripted: [`call_threads.rs::hanging_up_while_work_runs_finishes_in_the_chat_without_speech`](../../crates/hotline-core/tests/call_threads.rs). Both drivers finish work after hang-up. closing_note_waits_thirty_seconds_and_for_finished_work_then_parks and posts_are_quiet_on_the_call_and_notify_like_replies_after_hang_up cover timing and notification semantics.

Device results: Linux — pending; desk — pending; phone — pending.

- [ ] A turn in flight at hang-up finishes; its reply goes to the chat.
- [ ] The closing note comes 30 seconds after hang-up once no turn is in flight, so it covers the finished work.
- [ ] Once the call is down, a post lights the rail and notifies as any reply does.

### 11. Picking it back up

Scripted: [`call_threads.rs::picking_it_back_up_keeps_context_and_the_next_note_covers_only_new_work`](../../crates/hotline-core/tests/call_threads.rs). Binding, text continuation and incremental closing note on both drivers.

Device results: Linux — pending; desk — pending; phone — pending.

- [ ] The thread keeps its agent and its whole context after hang-up; typing in it is talking to the same agent.
- [ ] Calling from a thread attaches voice to it again; nothing starts from scratch.
- [ ] The next hang-up's closing note covers only what is new since the last one.

### 12. Following up in the chat

Scripted: [`call_threads/reader.rs::the_home_agent_reads_the_call_transcript_to_recover_a_detail_only_said_there`](../../crates/hotline-core/tests/call_threads/reader.rs). Both drivers recover the call-only detail through read_thread. replying_to_a_calls_post_goes_back_to_the_calls_thread proves reply routing; ui/tests/said-aloud.test.tsx covers inline tools and composer. Repeat the Bramble check live.

Device results: Linux — pending; desk — pending; phone — pending.

- [ ] Every post names what it is about. None leans on the call with "it" or "that one"; it reads right to someone who wasn't on the call.
- [ ] A reply on a call's post goes to that call's thread, where the context is.
- [ ] Scenario-pass check (task-244): say only on the call that the mascot is Bramble, a wiry terrier, then hang up. From the home chat ask 'Like we said on the call, use the mascot exactly that way.' On both drivers the agent must use read_thread on the call key and recover that detail from the full transcript, without asking you to repeat it or relying on a search snippet.
- [ ] Open the call: both speakers appear in order and every tool call is inline with its name and one line of what it did. The composer remains available.
- [ ] The home agent is handed a call's posts and closing note before its next line, and opens the call's thread when it needs more. It never answers "I don't know what it is" when the call knows.

### 13. Calling on a thread already under way

Scripted: [`call_threads.rs::calling_on_a_thread_under_way_queues_speech_on_that_same_thread`](../../crates/hotline-core/tests/call_threads.rs). Both drivers attach to the existing thread and queue work there.

Device results: Linux — pending; desk — pending; phone — pending.

- [ ] The call is the work thread with voice attached; it opens no new call thread.
- [ ] Posts read "From QA for 0.42", not "From the call".
- [ ] If the thread's agent is mid-turn, the spoken line waits or steers, as a typed one would.

### 14. Interrupting

Scripted: [`call_threads.rs::interrupted_speech_is_kept_and_the_next_turn_hears_the_cut`](../../crates/hotline-core/tests/call_threads.rs). Cut context reaches both drivers; voice/front.rs::a_live_cut_is_recorded_and_late_audio_cannot_resume_it covers late output. Physical onset, tap and echo are not proved. A cut is not proof of the last heard word (decision-9).

Device results: Linux — pending; desk — pending; phone — pending.

- [ ] Speech stops the moment the person starts talking, or taps.
- [ ] What was cut is recorded as cut, so the teammate knows what the person did not hear.
- [ ] On speaker, the teammate never hears itself: echo cancellation covers its own speech, on the desk and the phone.

### 15. A bad network

Scripted: [`voice/front.rs::a_wire_drop_resumes_the_same_live_call_and_announces_it`](../../crates/hotline-core/tests/voice/front.rs). Real wire reconnect and one recovery line; `crates/hotline-core/src/voice/room/tests/text.rs::a_short_transport_drop_resumes_but_twenty_seconds_detaches` proves the 20-second boundary, same-device ownership and lead-grant lifetime with paused time. Physical relay, phone adoption and long-loss behavior still need the device pass.

Device results: Linux — pending; desk — pending; phone — pending.

- [ ] A short drop resumes the same call on the same thread, and the teammate says one line about it.
- [ ] A drop longer than the hold window is a hang-up, with everything a hang-up does.
- [ ] The phone says Reconnecting only while it really is.

### 16. When the live voice goes away

Scripted: [`voice/front.rs::a_dropped_live_call_that_cannot_reconnect_ends_once_with_a_persistent_reason`](../../crates/hotline-core/tests/voice/front.rs). Bounded recovery and persistent failure. The same file covers failed starts, missing front and spent budget without automatic Legacy; explicit phone choice still needs a real device.

Device results: Linux — pending; desk — pending; phone — pending.

- [ ] A live call never silently becomes Legacy; retry with context first.
- [ ] If live cannot start or reconnect, the card says why and offers Call with Legacy voice.
- [ ] A spent budget is said once, with what the person can do about it.

### 17. Legacy voice on a slower teammate

Scripted: [`voice/direct.rs::legacy_speaks_its_own_untagged_opening`](../../crates/hotline-core/tests/voice/direct.rs). Opening reaches speech through the real core. Audible streaming, single waiting sound and interruption remain device checks.

Device results: Linux — pending; desk — pending; phone — pending.

- [ ] The first thing said is the agent's own first sentence, before its tools.
- [ ] A turn that says nothing first gets one soft sound, never filler words.
- [ ] The reply is spoken as it streams, and can be interrupted at any time.

