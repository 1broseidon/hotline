# Turn-by-turn voice: latency and presence

Design spec for reducing turn latency and fixing waiting behavior on Hotline's Turn-by-turn calls (`release/0.50.0` onward), without sacrificing call-model quality (e.g. Sonnet 5.5 medium) or local speech (Parakeet STT + Ollama Phonon TTS).

Written after auditing the 0.50 voice pipeline (`hotline-voice`, `hotline-room`, `hotline-core`, desktop `ui/src/voice`, and `hotline-mobile/src/voice`).

---

## 1. The rules

1. **No phone-tree audio on ordinary turns.** Remove the 1.8 s waiting blip. Two to three seconds of quiet feels like a person gathering a thought; a beep at 1.8 s screams IVR.
2. **No repeating canned progress filler.** Remove `Work::Progress` and its three cycling stock lines (`"I'm still working on that."`, `"Still on it."`, `"I'm continuing to work on this."`). This also resolves the contradiction in `docs/voice.md`, which already states no progress filler is generated.
3. **Work turns set expectations in coarse intent, never exact clocks.** Before calling tools for real work, the teammate says what it is about to do and roughly how big it is, using three broad sizes (*a moment*, *a couple of minutes*, *a while*) — never a promised minute count.
4. **Silence during announced work; one real update on overrun.** Once the teammate names the work and scope, stay quiet while tools run (the caller can still talk over it to steer or check in). If it runs well past its scope, the teammate gives one natural update carrying real news (what is done, what is left, no revised estimate) — never a repeating timer.
5. **Reserve acoustic cues for exceptions only.** A distinct cue for errors/disconnects, and at most one soft, warm presence cue after **8 seconds** of *unexplained* silence (nothing spoken yet on the turn). That covers phone-in-pocket anxiety without ever touching ordinary 2–4 s turns.
6. **Shave dead time around the model before touching model effort.** Cut endpointing delay, cold-start TTS, full-sentence buffering, and serial synthesis first. Keep Sonnet 5.5 medium untouched until per-stage logs show how much wait is actually hidden thinking.
7. **Instrument every hop before tuning thresholds.** Replace the single `"utterance to first clip"` lump with a per-turn stage timeline.

---

## 2. Where latency actually goes in 0.50

Audited directly from the `release/0.50.0` worktree and `hotline-mobile`:

| # | Stage | Current 0.50 implementation | Cost / issue |
|---|---|---|---|
| 1 | End-of-turn pause | `TurnDetector` (`SILENCE_MS = 1200`) shared by desktop (`ui/src/voice/turn.ts`) and mobile (`src/voice/turn.ts`); Android `HotlineSpeechModule.kt` also hardcodes `1200L` | **1,200 ms fixed** paid on every turn before anything else starts |
| 2 | Clip encode & upload | Non-PCM path waits for silence end, then downsamples + encodes WAV (desktop) or finalizes AAC (mobile) + base64 + wire send | ~50–250 ms sequential after silence end |
| 3 | Desk receipt & STT | `Calls::hear` reserves budget (`self.pay`), then runs `stt.transcribe()` (local Parakeet via `sherpa-onnx` in `speech/local.rs`) | ~23–130 ms (local Parakeet) |
| 4 | Prompt dispatch | `Calls::respond` → `room.call_prompt()` → session turn start | Unmeasured (lumped into total) |
| 5 | Model TTFT + hidden thinking | Teammate call model (e.g. Sonnet 5.5 medium) | Unsplit — logs cannot separate network TTFT, hidden thinking, and first spoken token |
| 6 | Spoken text buffering | `spoken::take_sentences` (`hotline-room/src/voice/spoken.rs`) waits for `.` / `!` / `?` + trailing whitespace (or `\n`) | Holds audio until the entire first sentence + next character arrives |
| 7 | TTS synthesis | `OpenAiShape::say` (`speech/openai_shape.rs`) buffers the full HTTP WAV body (`speech::call`); `say_reply` (`synthesis.rs`) synthesizes sentences serially | ~27–140 ms warm in Ollama Phonon, **~546 ms cold** on Turn 1 |
| 8 | Clip emit → playback | Base64 WAV over wire → WebAudio `decodeAudioData` (`ui/src/voice/player.ts`) or mobile `CallAudio.swift` | Unmeasured on desk; partially tracked client-side (`onLatency`) |
| — | Wait & progress timers | `waiting_sound.rs`: `waiting.wav` at **1,800 ms**, then `Work::Progress` every **20 s** forever while `Thinking` | Fires right before normal replies land; repeats canned lines 3–4× during work |

**Key takeaway:** We do not need a weaker model or a risky speculative LLM loop to make Turn-by-turn feel dramatically faster. Roughly **1.0–1.4 s** of wait sits in fixed client silence, cold-start Phonon, and full-sentence buffering — plus whatever hidden handoff time telemetry uncovers.

---

## 3. Part 1 — Per-turn latency telemetry (ship first)

Replace the single `[voice] utterance to first clip` line (`synthesis.rs:378`) with one structured timeline per turn `seq`.

### Desk log fields

Emit one line when the first audio clip for a turn is published (or when the turn fails/cancels):

```text
[voice] turn call=<id> seq=<n> silence=<ms?> upload=<ms?> stt=<ms> dispatch=<ms>
        llm_think=<ms?> llm_spoken=<ms> first_unit=<ms> tts_hdr=<ms> tts_done=<ms>
        desk_total=<ms> model=<writer> tts=<provider/model>
```

Definitions (all durations in ms, anchored to utterance arrival on desk, plus optional client offsets):
- `silence`: client-reported ms from last voiced frame to turn-end trigger (verifies endpointing savings in production).
- `upload`: client-reported ms from turn-end trigger to wire send + wire transit if clocks allow, or client-only local metric.
- `stt`: `stt.transcribe()` wall time (`0` for `voice.text`).
- `dispatch`: STT finish (or `voice.text` arrival) to `room.call_prompt()` handing the request to the model driver.
- `llm_think`: dispatch to first stream event (thinking delta or tool/text delta) — exposes network + provider TTFT.
- `llm_spoken`: dispatch to first non-empty text token inside `<spoken>` (or opening prose) — `llm_spoken - llm_think` directly measures hidden thinking time on Sonnet medium.
- `first_unit`: dispatch to first flushable clause/sentence emitted by `spoken::Spoken`.
- `tts_hdr` / `tts_done`: TTS request start to response headers and full clip ready (tied to `seq`, reusing `speech::Timing`).
- `desk_total`: utterance arrival to first `VoiceEvent::Clip` emitted.

**As shipped:** the line is `[voice] turn call= seq= silence= stt= dispatch= llm_think= llm_spoken= first_unit= ack= progress= tts_done= desk_total= model= tts=` ([voice.md](../voice.md#timing)). `silence` is the desktop window's turn detector pause, sent as `silenceMs` with `voice.text`, `voice.utterance` and the final `voice.audio` chunk; the phone does not send it yet, so its turns say `silence=-`. `upload` and `tts_hdr` are not in it.

Client side (`ui/src/voice/call.ts` and `hotline-mobile/src/voice/call.ts`): keep logging `endpoint → first audible sample` (`onLatency`), and split `clip_received → audio_started` so decode/buffering overhead is visible too.

---

## 4. Part 2 — Cutting latency without sacrificing quality

### 4.1 Adaptive end-of-turn detection (saves ~700–900 ms)

Replace the single 1,200 ms silence wait with a two-tier endpointing pipeline.

#### Tier A — Desk-side semantic endpointing + speculative Parakeet (primary path)

When the client streams 16 kHz PCM during Turn-by-turn (already supported on desktop via `PcmTurn` / `voice.audio`, and available on mobile via `live.start` when the call engine is active):

1. **At 300 ms of post-speech silence**, keep recording open, and launch two local desk tasks **in parallel** on the buffered audio so far:
   - **Semantic turn-completeness check** (Pipecat Smart Turn v3, 8 s audio window, CPU <100 ms, 23 languages).
   - **Speculative Parakeet transcription** on the buffered clip (~23–130 ms on Apple Silicon).
2. **If Smart Turn predicts turn complete** (probability ≥ threshold, start at `0.55` and tune against false cut-offs):
   - Commit the turn immediately.
   - Because Parakeet ran in parallel with Smart Turn, **the transcript is already ready (or within ~30 ms)** when the turn closes — saving both the silence tail *and* sequential upload + STT time.
3. **If Smart Turn predicts incomplete** (mid-thought pause, trailing intonation):
   - Keep listening. Re-check every **250 ms** of continued silence up to a **1,200 ms hard ceiling** (or **1,500 ms** if the speculative transcript ends on a clear hesitation/connector such as `"um"`, `"uh"`, `"and"`, `"so"`, `"but"`).
   - If the user resumes speaking before commit, **discard the speculative transcript** and continue recording. Nothing is sent to the teammate, so a false check costs zero LLM spend and causes zero side effects.

**Implementation note for `hotline-voice`:**
`hotline-voice` currently pins `sherpa-onnx = "=1.13.8"` with static ONNX Runtime inside `.speech-engine`. Smart Turn v3 is a standalone ONNX model, not a sherpa recognizer. Before committing to a second ONNX runtime binary, Pork Chop should verify the cleanest way to host Smart Turn v3 on the desk (either calling into the already-linked static ORT C API via a thin wrapper, or shipping a small verified model download managed by `speech/local/install.rs` alongside Parakeet). If packaging delays Tier A, Tier B ships independently on Day 1.

#### Tier B — Client-side adaptive fallback (on-device `voice.text`, AAC mobile, or Smart Turn uninstalled)

When the desk does not have live PCM or Smart Turn installed:
- **Clients with live partial text** (`voice.text` on macOS/iOS): use a two-threshold timer in `TurnDetector`:
  - **550 ms** (`FAST_SILENCE_MS`) only when the recognizer's locale is English and the draft ends in terminal punctuation (`.`, `?`, `!`, not a trailing `...`), or is a complete short utterance: ≤4 words not ending on a conjunction, preposition, article, filler, helper verb or "you", and a single word only when it is a whole answer (`thanks`, `nope`, `done`), never an opener (`okay`, `yeah`, `what`, `how`, `so`, `well`, `wait`).
  - **1,200 ms** (`SILENCE_MS`) for everything else: a draft past four words without a full stop, one ending mid-clause or on a filler, and every draft from a non-English recognizer, since the word lists are English.
- **Audio-only whole-clip clients**: `AUDIO_SILENCE_MS` stays at **1,200 ms**, the pause before 0.50, as its own constant so it can be lowered once a listening test shows a shorter pause does not cut people off. Android `HotlineSpeechModule.kt` stays at `1200L` with it.

As shipped (desktop, `ui/src/voice/turn.ts`): every threshold is a named constant in that file, and `silenceAfter(draft, locale)` picks one.

---

### 4.2 Opening-clause streaming (saves ~200–500 ms on longer first sentences)

Today `spoken::take_sentences` (`hotline-room/src/voice/spoken.rs:693`) only flushes on `\n` or `.` / `!` / `?` followed by whitespace. A 20-word opening sentence blocks TTS until the 21st token lands.

Add a **first-piece clause flush** to `spoken::Spoken` (Turn-by-turn streaming mode only):

- **Scope:** Applies **only to the first emitted unit of a reply** (`!self.first_unit_sent`). Every subsequent unit uses full-sentence splitting so longer answers keep natural cadence.
- **Trigger:** Flush at a clause boundary (`, `, `; `, `: `, ` — `, ` - `) only when **all** of the following hold:
  1. Accumulated text has **at least 6 words and at least 32 characters**.
  2. The candidate clause is **not** a bare leading discourse marker (e.g. `"Well,"`, `"Sure,"`, `"Actually,"`, `"Okay,"` by itself — wait for the real clause).
  3. The boundary is **not** inside parentheses or a formatted number (`1,200`).
- **Full stops unchanged:** `.` / `!` / `?` + whitespace (or `</spoken>`) still flush immediately at any length (`"Yep."`, `"On it."`).

**As shipped:** the flush is always on for Turn by turn replies (`Spoken::streaming()`); there is no setting. The listening check below was not run before it shipped and is still owed.

**Prosody quality gate:**
Listen to 10–15 real Phonon clips splitting on commas vs whole sentences. Phonon is sensitive to terminal punctuation for pitch drop. If comma splits sound clipped or fall in pitch too early, restrict the first-piece split to stronger boundaries (`; `, ` — `, `, and `, `, but `, `, so `) or raise the floor to 8 words / 45 chars. Do not ship choppy speech to save 200 ms.

---

### 4.3 Phonon pre-warm and 1-deep TTS pipelining

Two small desk-side wins in `hotline-voice` with zero UX downside:

1. **Pre-warm Ollama TTS on `voice.call_start`:**
   When a Turn-by-turn call starts with `ollama` as TTS, spawn a background one-shot request (`"."` or `"ok"`, discarding the audio, zero ledger charge) so Ollama loads `phonon` into memory while the call connects and the user speaks their first sentence. Eliminates the observed **~546 ms Turn-1 cold-start hit**.
2. **1-deep synthesis pipelining in `Calls::say_reply` (`synthesis.rs:122`):**
   Currently `say_reply` awaits `speak_sentence` for sentence 1 before pulling sentence 2 from `answers`. Allow synthesizing piece `N+1` while piece `N` is being encoded/sent (bounded concurrency of 2, strictly preserving `VoiceEvent::Clip` `index` ordering, and cancelling in-flight tasks on `interrupted`). This prevents micro-gaps between the opening clause (4.2) and the remainder of sentence 1.
3. **Defer Ollama audio-chunk streaming (`OpenAiShape::speak_chunks`):**
   Warm Phonon returns a whole short clip in 27–140 ms. Streaming raw PCM chunks out of Ollama saves <60 ms while adding framing complexity. Revisit only if Phase 1 telemetry shows `tts_done` > 200 ms p50.

---

### 4.4 The call thread's agent starts with the call (0.50)

A call runs in its own call thread with an agent of its own, and until 0.50
that agent was started by the first utterance: the first turn of a call paid
for it and later turns did not (real calls: Claude Code over ACP
`llm_spoken` 17.5 s first, 0.84 s second). What the first turn alone did,
from `Room::prompt_call_work` → `wake_work` → `bring_up` → `launch` →
`Room::thread_agent`: bind the workspace, write `AGENTS.md` for a child, take
the computer's grant, read the main conversation's tail, build the driver, and
`Driver::start` — an ACP child's spawn, `initialize`, Hotline's MCP endpoint,
`session/new` with the MCP servers, then model, mode and effort put back; on
Hotline Agent, MCP connect and tool listing. Then its first prompt carried the
ACP briefing, and the provider saw an uncached prefix.

Measured with the scripted agents (debug build): the desk's own part is
~2 ms on Hotline Agent and ~150 ms on the ACP fixture, of which spawn and
handshake ~35 ms; reading a 10 MB tape's tail ~80 ms. The fixture spawns in
milliseconds, so the real harness's start (Node, its CLI, MCP connections
inside `session/new`) is not measured here and is inferred at several
seconds; the provider's first uncached request is the rest.

Now `Room::prepare_call` makes that start when the call is placed and sends
nothing; the first line joins it. `session_ready=` on the first turn's line
says how long it took, so a real call shows how much of the first turn it
covered. What cannot move without a prompt stays on the first turn: the ACP
briefing and the provider's cache.

## 5. Part 3 — Presence, silence, and work-turn grammar

Unify desktop and mobile under one calm, predictable audio contract.

### 5.1 State matrix

| Situation | Audio | Visual UI (desktop & phone) |
|---|---|---|
| **Ordinary turn, 0–8 s** | **Silence** | Calm `thinking` state (CallCard / CallScreen / Live Activity) |
| **Quiet ≥ 6 s, teammate visibly working** (with `voice.acknowledge` on; 6 s from end of speech, or from the end of the acknowledgement's sound) | **One** short line in the teammate's voice on what it is doing now, from the kind of step (§5.7); no presence cue follows | Remains in `thinking` + live `activity` label |
| **Unexplained wait ≥ 8 s** (no speech emitted yet on this turn) | **One** soft, warm presence cue (max once per turn, never repeats) | Remains in `thinking` |
| **Work turn — before tools** | Teammate's own short opening sentence naming the action + coarse scope | Switches to `speaking`, then `thinking` + live `activity` label |
| **Work turn — tools running** | **Silence** (caller can still speak to steer or ask) | Live `activity` text updates CallCard / Live Activity |
| **Work turn — overrun** (> 90–120 s silent since last spoken line, at next tool boundary) | Teammate emits **one** natural update with real news (what's done, what's left; no revised estimate) | Updates transcript + live `activity` |
| **Work turn — finished** | Teammate's spoken report, as today | Normal report / written attachment |
| **Error / disconnect / turn failure** | Distinct muted falling earcon + short system recovery line | Error/reconnecting banner |

**Why 8 s for unexplained silence:**
On desktop, you see `thinking`, so 2–4 s of silence is comfortable. On a phone in your pocket (screen locked on CallKit), 2–4 s still feels like natural human thinking, whereas **8 s of total dead air with no opening line** is where callers start wondering if the connection dropped. A single soft cue at 8 s solves the pocket case while staying completely silent on normal turns.

### 5.2 Audio design of the presence cue

Replace `crates/hotline-voice/src/assets/waiting.wav` (and deprecate the high-pitched 1,174 Hz double-blip `think` tone in `hotline-mobile/src/voice/tones.ts` for active calls):
- Lower, warmer register (muted marimba/felt-like single soft pulse around ~330–440 Hz with gentle attack/decay, low gain) — reads as *"line is alive"*, never *"please hold"*.
- Distinct from `connect` (rising two-note), `end` (falling two-note), and `error` (short muted low double-tap).
- **Single source of truth:** only the core emits the 8 s presence cue (`waiting_sound.rs`); clients must not layer a second local `think` tone on top.

### 5.3 Exact changes to `spoken::CONTRACT`

In `crates/hotline-room/src/voice/spoken.rs`, update the tool-opening paragraph of `CONTRACT`:

**Current:**
> Before a tool, begin with your own short spoken sentence naming the action when acknowledgement helps. Often just answer. State, code and results require a lookup. If cut off, answer the new words without repeating yourself.

**Proposed:**
> Before a tool, begin with one short spoken sentence naming what you are about to do. If it will take more than a quick lookup, set expectations with a coarse size — a moment, a couple of minutes, or a while — never an exact number of minutes or seconds (e.g. "I'll check that file, give me a moment", "I'll dig into the audio path and write it up, this'll take a couple of minutes", "This refactor will take a while, I'll chime back in when it's done"). Once you've said that, work in silence until you finish. Often just answer. State, code and results require a lookup. If cut off, answer the new words without repeating yourself.

### 5.4 One-shot overrun update (no canned timer)

Instead of `Work::Progress` firing canned TTS every 20 s:
1. Track `last_spoken_at: Option<Instant>` and `overrun_nudged: bool` on the active call turn; both reset when the person's next turn starts.
2. When **≥ 90 s** have passed since `last_spoken_at`, or since the 8 s cue on a turn that never spoke (`last_spoken_at.or(feedback_at)`), leave the running turn one internal note:
   `[Voice call: this work has been quiet for a while. Before your next tool call, speak one brief sentence with real progress — what's done and what's left — with no revised time estimate.]`
3. Set `overrun_nudged = true`. There is no cooldown and no second note on the same turn. Hold, a cut-in and the turn ending suppress it.
4. **Delivery:** the note goes through `Driver::note`, never `steer`. On Hotline Agent it bumps no steering revision, so a running `wait_jobs`, a model request in flight and tool calls queued in the same response all go on; the loop puts it in history right after the next tool (or job) results, before the next model request. An ACP agent gets it through its steering extension, which takes it at its next step; one it refuses comes back unconsumed and is dropped, never said as the person's prompt.
5. **Why this works:** zero extra API calls (rides the next tool boundary the agent is already taking), real context-aware speech instead of `"Still on it."`, and stays completely quiet if the task finishes cleanly within 90 s.

### 5.5 Code & doc cleanup

- **Delete** `Work::Progress` and `progress_lines` in `crates/hotline-voice/src/turn.rs`.
- **Simplify** `crates/hotline-voice/src/waiting_sound.rs`:
  - Remove the `1_800 ms` trigger and the `20 s` `Work::Progress` branch.
  - Fire the single soft presence clip **only** when `!call.waiting_sound && at.elapsed() >= Duration::from_secs(8)`; the first real spoken clip sets `waiting_sound`, so a turn that has spoken never gets the cue.
- **Audit** `hotline-mobile/src/voice/call.ts` and `tones.ts` to ensure no legacy client-side `cue('think')` fires during a Turn-by-turn wait on 0.50+ desks.
- **Update** `docs/voice.md` (lines 140–142) to document the 8 s unexplained-wait cue, coarse work-turn scope, and one-shot overrun update.

---

## 5.6 Desk acknowledgement while the teammate thinks (as shipped)

Measured on a real call, the person waited about 18 s from the end of a sentence to the first word: 1.2 s of endpointing, about 2.9 s of local transcription, about 10.5 s for the teammate's model to write its first words, and about 2.9 s for the voice to return a clip. Only the model's 10 s is a wait the desk can cover with words of its own.

Behind one setting, `voice.acknowledge` (Settings › Providers › Calls, *Acknowledge while the teammate thinks*, off by default), the desk's call assistant writes one line of at most about eight words naming what was asked, in the teammate's first person, or nothing. The line is synthesized in the call's own voice and played only if the teammate has no piece of its own ready or being synthesized, and only if its sound is ready within 4 s of the end of the person's speech (`ACK_CAP`); otherwise it is discarded, since a late acknowledgement is worse than none. It is skipped on live calls and calls to the desk, while the teammate is still on an earlier turn, after a turn whose `llm_spoken` was under 3 s, and without a call assistant or Chat budget; its model and speech are reserved on Chat and Voice like a rewrite, and a refusal skips it. `ack=` in the turn line times it from the end of speech to its sound being ready, and each turn without one logs why. Rules and edge cases are in [voice.md](../voice.md#acknowledging-while-the-teammate-thinks).

The first version held dispatch up to 500 ms for the line so its exact words could go in the teammate's prompt as already said. On a real call no line came in time on any of three turns (`dispatch` 686, 672 and 703 ms against about 160 ms without the setting), while the teammate then took 5 to 13 s to say its first words: a hosted fast model does not answer inside 500 ms, so the feature only added delay. Now dispatch is never held. The turn goes to the teammate at once, as with the setting off, and the line is asked for in parallel. Since its words cannot be in the prompt, the turn's contract says instead that the desk will say a brief acknowledgement of the request aloud for the teammate, so it must not open with one of its own or refer to it, and goes straight to the work or the answer (`spoken::ACKNOWLEDGED_CONTRACT`, which drops the usual opening line before a tool). A line that is said is kept on the call's thread as the teammate's and among the recent speech later turns are told of; the turn it opened is not interrupted.

Still owed: a listening check on real calls of how often the line lands before the teammate's own opening, and whether the teammate keeps to not opening with its own acknowledgement.

## 5.7 Saying what a working teammate is doing (as shipped)

The owner's case: the agent goes straight to work, and a few seconds later the desk should say something like "I'm searching the web for that now." The failure to avoid, which he had heard: "Still searching the schedule" with nothing said before it.

Under the same `voice.acknowledge` setting, the teammate's session tells the call of each tool step as it writes it, by the group the call pane says the step by (`glance::step_said`'s `key`), and the call keeps only the kind of work (`progress::Activity`). Once a turn, when the teammate has said nothing of its own, the turn has been quiet 6 s since the end of speech (`QUIET_AFTER_SPEECH`) or since the acknowledgement's sound ended (`QUIET_AFTER_ACK`), and a step is running or ended within 10 s (`STEP_FRESH`), the call assistant writes one line (`dispatcher::PROGRESS`) from the person's words, that kind of work in plain words, and what was said aloud on the turn so far. The code drops a line that leans on an earlier one when nothing was said ("still", "also", "again", "as I said", "continuing", "keep"), says "still" at all, repeats the acknowledgement, claims a result or a time, opens with filler, runs past ten words or carries a path, address, flag, `file.ext`, `_` or id. Without a call assistant a fixed line covers web search, reading, running and the browser; other kinds say nothing. The 8 s tone waits while the line is made (at most 4 s, `PROGRESS_CAP`) and plays if it is not said; a turn with nothing to name gets the tone, and no line follows the tone. `progress=` in the turn line times it from the end of speech, and each turn without one logs why.

On a slow turn without an acknowledgement the person hears the line at about 6 s instead of the tone at 8; with one, the acknowledgement at about 2.5 s, then the line about 6 s after it ends, and never the tone.

Still owed: a listening check on real calls of whether 6 s feels right, and how often the fast model's line is dropped by the checks.

## 6. Explicit non-goals & open tuning points

- **Do not lower Sonnet 5.5 medium effort yet.** Wait for Phase 1 telemetry (`llm_spoken - llm_think`) from real M3 calls. If thinking is <600 ms, leave effort alone. Only consider tuning effort (or disabling thinking on simple small-talk turns) if telemetry shows thinking is >1.5 s p50.
- **No speculative LLM prefill or early generation.** Cloud call models don't offer a free prefill-only call, and speculative agent turns carry tool side-effect risk.
- **Pocket-phone tuning check:** After Phase 1 lands, test a locked-screen iPhone call (CallKit) to verify whether the unexplained-wait threshold feels better at **6 s** or **8 s** in a pocket.
- **Overrun threshold check:** Start at **90 s**, once per turn; revisit if "a while" tasks feel nagged too soon.
- **Audio-only endpointing:** `AUDIO_SILENCE_MS` stays at 1,200 ms until a listening test on real calls shows a shorter pause does not cut people off.

---

## 7. Phased rollout & verification

### Phase 1 — Telemetry, presence grammar, pre-warm, and clause streaming (fast, low risk)
1. Ship per-stage turn timing logs (desk + client).
2. Remove `Work::Progress` / canned lines; move `waiting_sound.rs` to single 8 s unexplained-silence cue; redesign `waiting.wav`.
3. Update `spoken::CONTRACT` for coarse scope + add one-shot tool-boundary overrun nudge (a `Driver::note`, never a steer).
4. Add Ollama Phonon pre-warm on `voice.call_start` + 1-deep TTS pipelining in `say_reply`.
5. Implement first-piece clause flushing in `spoken::Spoken` (shipped always on; the 10–15 clip Phonon listening check is still owed).

**Verification:** Capture 10+ real turns on the M3 (mix of short Q&A, longer spoken answers, and multi-minute tool work) and confirm baseline vs Phase 1 stage numbers + zero phone-tree blips / zero `"Still on it."` loops.

### Phase 2 — Adaptive end-of-turn (largest raw latency win)
6. Wire desktop/mobile Turn-by-turn PCM streaming into desk-side Smart Turn v3 + speculative Parakeet at 300 ms silence (with Tier B client fallback for non-PCM paths and updated Android silence extra).

**Verification:** Measure false cut-off rate on natural thinking pauses (*"Let me think... actually..."*) vs latency drop on crisp turns. Target: **≥700 ms** reduction in `silence + stt` p50 with <5% premature cut-offs.

### Phase 3 — Data-driven follow-ups (only if telemetry warrants)
7. Review `llm_spoken - llm_think` from Phase 1/2 logs to decide if call-model effort needs any adjustment.
8. Review `tts_done` to decide if `OpenAiShape::speak_chunks` streaming is worth adding for Ollama Phonon.
