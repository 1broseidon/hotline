# Interrupting a Turn by turn reply by speaking

Status: a prototype behind **Settings › Voice › Calls › Interrupt by speaking**,
on by default, on the Apple speech path only. A tap (Cut in) still interrupts
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
30 dB down, and the leak alone transcribed word for word. So words count only
within 3 s of an onset. To interrupt they need two words that are not listening
sounds ("yeah", "mm-hmm", "okay"), or one cut-in word (stop, wait, no, hold,
hang, sorry, actually, hey). The person's words are those new since the
onset (the session's text is cumulative and revised, so only still-unchanged
words count as old) less runs of three or more of the reply's words in order.
The reply is only the lines played since the call last listened. Filtering
any word found in the last four replies, as at first, took nearly every
everyday word for echo once a call had a few replies behind it (the owner's
2026-11-20 call; reproduced in `voice-call.test.ts`).

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
   - **No words yet: it resumes turned down** (-14 dB) after 500 ms of quiet,
     until 1.5 s after the voice ended, the engine's lag for a short phrase;
     words then still cut it. A cough: paused about 0.8 s, quieter about 1 s.
     After 4 s of wordless voice it resumes at full volume, and that voice
     pauses nothing more until it stops.

## How the teammate learns it was cut off

This uses the existing mechanism (decision-9). `voice.interrupt` makes
`SpeechLine::interrupt` (`crates/hotline-voice/src/heard.rs`) write the
cut notice on the call's thread, and the next turn gets it through
`take_interruption`, apart from the person's saved words.

The window says where playback stood (`played` on `voice.interrupt`,
`voice.hold` and `voice.call_end`): the last clip of a `said` line that
began to play, by line and index, how far into it the audio clock is, and
the clip's length (`ClipPlayer.position`). It is read before playback
stops. Talking over a reply suspends the call's AudioContext first, and
the clock stands still while it is suspended, so the position is where the
reply paused, not where the words decided it seconds later.

The desk keeps, for the last few lines it said, each sentence's words and
the clips that carried it, with each clip's length read from its WAV. From
the report alone:

- every sentence whose sound played to its end was heard, and every one
  whose sound never began was not, which is certain;
- at most one sentence was cut. Where in it is exact only from a voice's
  own word timings (`SpeechChunk.marks`, which no voice Hotline speaks with
  supplies yet). Otherwise it is played time over the sentence's length,
  snapped to the nearest word end, and labelled an estimate; a streamed
  sentence whose length was not known yet uses the call's speaking rate so
  far (14 characters a second until it has one).

The notice says it in three labelled parts, in the words as spoken
(`speech_text`):

```text
Speech interrupted: the caller cut in.
Heard in full: "I looked at the logs."
Cut off during: "The cache went stale [cut about here] after the deploy." (0.5 s of its 1.0 s played)
Not heard: "I can clear it now."
Where it stopped in that sentence is estimated from how long it played. Do not claim a more precise stopping point than this. Answer what the caller said, and carry on from what was not heard only if it still matters after that.
```

"about" goes only with an estimate, and an estimate never says "exact".
A cut between sentences says the split is certain. A part over 40 words
keeps the words nearest the cut and counts the rest ("… [36 words] …").
A cut that leaves nothing unheard writes no notice. The desk's
acknowledgement and its line on the work are lines like any other: a cut
in one leaves the reply said after it under Not heard. A hold with a
position notes the cut the same way; a hang-up does too, with no
instruction, and the call's closing note reads it as `[speech cut off]`,
so what never played is not summarised as said.

A client that reports nothing (the phone today) gets the whole generated
line and "Where playback stopped is unknown: the caller's app did not
report it." A live front reports `playedMs` on `voice.front_barge`, which
has no clips to place in words, and says so.

## How it fits with the rest of the call

- **Acknowledgement and progress lines** are clips like any other and pause
  and interrupt the same way. Cutting one sends `voice.interrupt`, which mutes
  the earlier turn's speech as a tap does; the person's words go as their own
  queued turn (the queued-turn fix), so they are not lost (tested).
- **Listen after speech (64e75718)** is unchanged: when the reply's sound
  ends, the call listens afresh in a new session.
- **The hold rule** and the mute for clips arriving while words are on their
  way are unchanged; after an interruption the call is `hearing`, so the cut
  line's clips are muted and others wait as before.

## When it stays off

- The person switched the setting off: the behaviour before this feature
  exactly (tested). Never set is on (tested).
- A call not heard by this Mac's analyzer: desk model, older recognizer, or a
  live call (which has its own barge-in).
- Level events say voice processing is off (`processed: false`): off for the call.
- Two onsets in one call whose only words were the reply's own: off for the
  call. That is the canceller failing, as on loud open speakers, detected.
- Open speakers can't be told from a USB headset reliably, so the person
  chooses; the tooltip says "Best with headphones".

## The setting

**Interrupt by speaking** under Turn by turn in Settings › Voice › Calls,
tooltip "Best with headphones"; this computer's choice (`localStorage`
`hotline.interruptBySpeaking`: `off` is off, and never set or `on` is on, so
an explicit off stays off), shown only where this Mac's analyzer hears.

## Reading a call from the log

The window writes `[voice] call <id>: interrupt by speaking: …` lines to the
app's `logs/hotline.log` through `voice_note`: `on` or `off` with the reason;
`paused the reply, voice X over Y`; `resumed the reply, quiet, turned down …,
N words, the first M ms after the voice began` (or `no words yet`); `the reply
is at full volume again, no words 1500 ms after the voice ended`; `interrupted
the reply, 2 words the reply did not say, N words, the first M ms after …`;
`off for this call, <why>`. They hold counts and timings, never words.

## What is proven and what is not

Proven by unit tests and the Swift harness with `say` clips: the meter reads
channel 0; a reply leaking 30 dB down gives only the reply's words; a person
over it is heard; and the call-level behaviour in the acceptance list above.
Not proven: the thresholds on a real microphone, how a pause feels, and the
canceller on open speakers or in the window's capture. Those need a call with
the setting on, read from the log.
