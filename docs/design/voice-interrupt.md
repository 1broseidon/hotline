# Interrupting a Turn by turn reply by speaking

Status: a prototype behind **Settings › Voice › Calls › Interrupt by speaking**,
off by default, on the Apple speech path only. A tap (Cut in) still interrupts
on every path. Code: `ui/src/voice/bargeIn.ts` (the decisions), `call.ts`
(`overhear`, `overheard`, `interruptedBySpeech`). Tests:
`ui/tests/voice-barge-in.test.ts`, the last block of `ui/tests/voice-call.test.ts`,
and `crates/hotline-app/macos/speech/check/run`.

## Why it was turned off before

- **2026-10-21**, 2a4c599b (task-240, shipped in 0.42.0). Speech onset, by energy
  alone, cut the reply (`silenceLegacy()`), and the Mac recognizer ran through
  playback. Desk QA passed that day.
- **2026-10-25**, fd4591c7 (task-291, shipped in 0.44.0), with the phone the same
  night (hotline-mobile b0d72e9). The window stopped listening while a reply
  plays. design.md gives the reason: echo cancellation in the browser capture
  and in the Mac's voice processing "did not keep a reply on a speaker from
  being heard as the person and cutting itself off".
- **A likely cause then, not proven.** Until ae83b727 (2026-11-20), the
  Mac's meter and recognizer read every tap channel. Voice processing's
  reference channels carry the reply itself, so an energy onset measured over
  them trips on the reply through any output. The meter fixed in this
  branch's first commit made the same mistake.

The failures on record are this attempt's acceptance tests:

1. The reply's own echo cuts the reply off. Energy never cuts here; it only pauses.
2. The recognizer transcribes the reply (self-transcription, task-206). The
   check below shows the analyzer turns a reply 30–40 dB down into its exact
   words, so words the reply said never count as the person's.
3. Room words recognized before the onset are taken for the person's.
4. Late clips of a cut line play again.
5. Phantom cuts: a cut notice at 0 ms with nothing cut (task-258).
6. Words lost or merged after a barge-in (task-254, live calls).
7. Words lost on a queued turn (eff1e73e).

## Which inputs are echo-cancelled

| Path | Signal | Verdict |
| --- | --- | --- |
| Apple speech, analyzer (macOS 26), voice processing on | Channel 0 is the processed microphone. Channels 1..n are references carrying what the Mac plays. The analyzer and the meter now read channel 0 alone, and level events carry `processed`. | Used. |
| Apple speech, older `SFSpeechRecognizer` | It is handed the whole multichannel tap buffer, references included. | Off; tap only. |
| Window `getUserMedia` (desk-model and audio calls) | `echoCancellation: true` is requested (`audio.ts`), and WebKit runs its own voice-processing unit. With AirPods or wired headphones there is little coupling to cancel. On open speakers, the 10-25 record says it failed. | Can't be shown reliable without device runs. Tap only; these paths have no recognizer partials either. |
| The phone | — | Out of scope. |

## Telling the person from noise and echo

**Energy, on channel 0 only.** The processed level must be above the larger
of the room's floor × 3.2, 0.018, and the echo floor: the median over the last
2 s of processed level ÷ the output's loudest in the last 300 ms, × the output
now × 3. It must stay there for 200 ms within 400 ms. Nothing can start in the first
400 ms of audible playback, while that ratio is still unknown. Echo has a
steady ratio to the output; a person adds energy the output does not explain.
No device measurements exist yet: these constants are first guesses, and every
decision is logged.

**Words, the confirmation.** The check harness measured partials about once a
second, the person's first words 2.6 s after they began over a reply leaking
30 dB down, and the leak alone transcribed word for word. So words count only within 3 s of an onset. To interrupt they need either two
words that are neither the reply's nor listening sounds ("yeah", "mm-hmm",
"okay"), or one cut-in word (stop, wait, no, hold, hang, sorry, actually, hey).
The person's words are what follow the engine's text at the onset (matched
loosely, since the engine re-spells "looked" as "look") and any run of three or
more of the reply's words in order.

## The two stages

1. **Onset: the reply pauses at once.** `pausePlayback` suspends the call's
   AudioContext. The whole playback clock stops, so clips already scheduled,
   and clips that arrive meanwhile, play on from the same sample. Nothing is
   re-requested and the desk is not told, because nothing was cut. That is
   what "resume" means for a streamed clip.
2. **Then the words decide.**
   - **Words arrive: an interruption.** Playback stops and `voice.interrupt`
     goes to the desk, exactly as a tap sends it. The desk cancels the speech,
     records the cut notice and mutes the rest of that turn, so late clips are
     dropped. The same recognition session carries on as the person's turn.
     The turn detector begins at the onset (`TurnDetector.begin`), the turn
     ends on the usual pause, and it is sent with `voice.text` after the
     interrupt.
   - **No words: the reply resumes.** That happens after 500 ms of quiet
     (pauses last at least 500 ms), or after 4 s whatever the voice is doing.
     A voice the reply already resumed over pauses nothing more until it
     stops. Words that arrive late, within the 3 s, still interrupt.

## How the teammate learns it was cut off

This uses the existing mechanism (decision-9). `voice.interrupt` makes
`SpeechLine::interrupt` write the notice: "Speech interrupted. The caller did
not hear this line to the end … The exact last word heard is unknown … do not
resume the unfinished speech." The next turn gets it through
`take_interruption`, apart from the person's saved words.

Not built: `voice.interrupt` has no `playedMs`. The window knows it
(`playedMs()`), and an additive field would let the notice say where playback
stopped, as `voice.front_barge` does. That is a contract change, left for a
follow-up.

## How it fits with the rest of the call

- **Acknowledgement and progress lines** are clips like any other and pause
  and interrupt the same way. Cutting one sends `voice.interrupt`, which mutes
  the earlier turn's speech as a tap does; the person's words go as their own
  queued turn (the queued-turn fix), so they are not lost (tested).
- **Listen after speech (64e75718)** is unchanged. The session opened when
  playback starts is separate; when the reply's sound ends, the call listens
  afresh.
- **The hold rule** is unchanged: a clip that arrives while the person talks
  waits, and is dropped if they said words. After an interruption the call is
  `hearing`, so the cut line's further clips are muted, and other clips wait
  as before. The mute for clips that arrive while the words are on their way
  is unchanged.

## When it stays off

- The setting is off, the default: today's behaviour exactly (tested).
- A call not heard by this Mac's analyzer: desk model, older recognizer, or a
  live call (which has its own barge-in).
- Level events say voice processing is off (`processed: false`): off for the call.
- Two onsets in one call whose only words were the reply's own: off for the
  call. That is the canceller failing, as on loud open speakers, detected.
- Open speakers can't be told from a USB headset reliably, so the person
  chooses; the tooltip says "Best with headphones".

## The setting

The **Interrupt by speaking** row sits under Turn by turn in Settings › Voice
› Calls, with the tooltip "Best with headphones". It belongs to this computer
(`localStorage` `hotline.interruptBySpeaking`), like On this Mac, and is shown
only where this Mac's analyzer hears calls.

## Reading a call from the log

The window writes `[voice] call <id>: interrupt by speaking: …` lines to the
app's `logs/hotline.log` through `voice_note`: `on` or `off` with the reason;
`paused the reply, voice X over Y`; `resumed the reply, quiet, no words` (or
`no words in 4000 ms`, or `its only words were the reply's own`); `interrupted
the reply, 2 words the reply did not say, N ms after the voice began`; `off
for this call, <why>`. They hold counts and timings, never words.

## What is proven and what is not

Proven by unit tests and the Swift harness with `say` clips: the meter reads
channel 0; a reply leaking 30 dB down gives only the reply's words; a person
over it is heard; and the call-level behaviour in the acceptance list above.
Not proven: the thresholds on a real microphone, how a pause feels, and the
canceller on open speakers or in the window's capture. Those need a call with
the setting on, read from the log.
