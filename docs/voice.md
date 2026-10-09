# Voice: hearing, speaking, and what it costs

The desk turns a spoken clip into words and words into a spoken clip through
a provider the owner has already connected, or hears with a model of its own
that the owner downloaded. Voice never asks for a key of its own. The call, the dispatcher and the wire commands are in
[wire.md](wire.md); this page is the speech layer under them and the ledger
beside them (`crates/hotline-core/src/voice/`).

## The seam

`Speech` (`voice/speech/mod.rs`) has `transcribe(clip)` and `speak(text)`, and
`resolve(vault, settings)` returns a `SpeechSet` of three: `stt`, `tts` and an
optional `fallback_tts`. An adapter is built for one job, so `id()` names the
model behind that job, and the other job answers `WrongJob`. `output_mime()`
reports the primary TTS format for the call descriptor (default `audio/wav`).
`transcribe` takes `audio/wav` (16 kHz mono PCM16) and `audio/mp4` (AAC); any
other type is refused before a request.

Negotiated device-text calls use `resolve_output` to select `tts` and optional
`fallback_tts` without an STT adapter. They accept a finalized transcript and
make no remote transcription request or STT reservation. Audio remains the
default input mode.

Adapters can additionally accept live input through `transcribe_live` and
produce progressive output through `speak_chunks`. The call negotiates
these paths explicitly; existing providers and callers retain whole clips.
Only a finalized live transcript enters the desk or teammate's session.

The fallback is the desk's to use: when the voice fails it asks `fallback_tts`,
once (`Calls::synthesize`), and reserves the second call's cost like the first.
Each clip carries its own type: `audio/wav` where the provider can make one,
`audio/mpeg` where it cannot.

A speech error carries the provider and an HTTP status and never a response
body, because a provider's error text can echo what was said.

## Device text on a Mac

The macOS desktop shell can hear an utterance on the machine itself, the
same way the phone does. The Swift in `crates/hotline-app/macos/speech/` is
a port of the phone's HotlineSpeech module; `build.rs` compiles it with
`swiftc` into a static library, and `src/speech.rs` exposes it to the main
window only, as `speech_capability`, `speech_permit`, `speech_start`,
`speech_stop` and `speech_cancel`, with recognition events arriving as
`speech-event`. Other platforms report it unavailable.

On macOS 26 it uses `SpeechAnalyzer` with a `SpeechTranscriber`; that path
is compiled only by a Swift 6.2 or newer compiler (Xcode 26). Without it, or
without the language's model, it uses `SFSpeechRecognizer` with
`requiresOnDeviceRecognition`, and only where the recognizer supports
on-device recognition; no network recognizer is ever made. `speech_capability`
reads support without prompting, downloading or opening the microphone.
`speech_permit` asks for speech recognition and the microphone (both usage
descriptions are in `Info.plist`; the signed build's hardened runtime also
needs the `audio-input` entitlement), then installs the analyzer's language
model through `AssetInventory`, giving up after 60 seconds. The download is
the only network use; recognition is not.

Each start is a fresh utterance from the default input device through
`AVAudioEngine`, with voice processing on where the device allows it and
other audio ducked as little as it permits. Every text event is the whole
utterance so far. Level events are the dBFS of each microphone buffer.
`speech_stop` closes the microphone and waits up to five seconds for the
final text, failing rather than returning a partial. A changed input device
ends the utterance with an error. Cancelling with an empty id cancels
whatever is current, a model download included, and a reload of the window
does that too.

## Calls from the window on a Mac

A call placed from the desktop window on macOS is heard by that engine when
it can be (`ui/src/voice/call.ts`, `ui/src/voice/transcription.ts`). The
window starts the call with `inputMode: "text"` only when the desk's
`voice.status` capabilities include `voiceTextInput`, `speech_capability`
says this Mac can hear, and `speech_permit` is granted; it then never opens
the webview's microphone. Otherwise the call is the audio call it always
was, and a desk that answers a text call without `text/plain` in its input
is hung up with a message to update it. A recognition session runs only
while the call listens: the desk speaking or thinking, hold and hang-up
cancel it, so the engine never hears the desk. Its level events drive the
same turn detector as the webview's microphone, with a 100 ms onset because
the words corroborate a short "yes". Partial text is the person's live line
once the meter has heard a voice, less any words recognized more than half
a second before that onset, which were the room's. When the detector ends
the turn, the window stops the session for its complete final text and
sends it once with `voice.text` under the next sequence number; an empty
final sends nothing. Settings › Providers › Use for describes Hearing as
"On this Mac when you call from here" while this Mac can hear.

## Three ways to talk

Where the window can dictate there are three: a **conversation**, which is
the hands-free call; **hold to talk**, dictation for as long as a key or
the microphone is held down; and **toggle to talk**, dictation started by
one tap and stopped by the next. Dictation puts words in the field. A
window dictates where its Mac hears speech itself, or where the desk has a
speech model of its own; elsewhere there is only the conversation.

## Dictation

Where `speech_capability` says this Mac can hear, or `voice.models` says
the desk has a model installed, the composer's key on an empty field is a
microphone, Dictate, and the words go into the field rather than to anyone
(`ui/src/voice/dictation.ts`). A call is the phone key in the
conversation's band, Call <name>, shown when the desk can put a call
through to the teammate; it turns into End the call while one with that
teammate is live. Where neither can hear, there is no dictation and the
empty composer's key starts the call, as the band's does. The window asks
`voice.models` again whenever a composer mounts, and hears it from Settings
when a model is installed or removed.

A press of the microphone or the Dictate shortcut starts listening at
once. Its release tells the two apart (`TapOrHold`): held for 300 ms or
more, the press was a hold and the release stops; shorter, it was a tap
and listening goes on until the next press, which stops. A press again
before its release is the key repeating and is ignored, unless it comes
more than 2.5 seconds after the last one, so a release the system lost
cannot wedge the key. The microphone hears the pointer go down on it and
up anywhere; Enter or Space on it starts or stops as a tap does, and
Enter in the field stops.

The first dictation of a run on this Mac's engine asks `speech_permit`; a
refusal says to allow Hotline under Speech Recognition and Microphone in
System Settings › Privacy & Security. The desk's engine asks for the
microphone when it opens it. A refusal says to allow the microphone for
Hotline, no microphone at all says to connect one or pick it in the
computer's sound settings, and one that is busy or failing says to close
other apps using it (`microphoneTrouble`). With neither engine able to hear, the dictation says to
download a speech model for the desk. Listening starts a recognition session. The field
keeps whatever was typed before dictation began, and each partial replaces
the dictated words after it, separated by one space. While listening, the
placeholder reads "Listening…", the field cannot be typed in, and the key
holds a voice meter (`ui/src/components/VoiceMeter.tsx`) and shows a stop
glyph when pointed at or focused. The meter is five bars that rise with
the level, which spreads -60 dBFS to -10 dBFS evenly over 0 to 1 and is
smoothed each frame, rising with a 40 ms time constant and falling with
260 ms; they breathe on the 1800 ms beat while it is quiet and shimmer in
turn while the final text is awaited. With reduced motion they only
follow the level. When the engine ends a session on a pause (`ended`
with `final` or `no-speech`), its words are kept and a fresh session
starts, so a pause does not end the dictation. Stopping waits for the
engine's complete final text and puts it in the field. Escape cancels and
puts the field back as it was before listening began. An engine error, or
a stop that does not finish, ends the dictation with the words heard so
far left in the field and one sentence under it. One dictation listens at
a time: starting another cancels the first, and a call that starts lets
the dictation go, keeping its words. The empty composer offers no Dictate
while a call is live.

What happens next is Settings › General › Shortcuts › After dictating,
kept per computer in `localStorage` under `hotline.dictation.after`.
**Don't send**, the default, sends nothing. **Send** counts down 1.5 seconds once a dictation the person
stopped heard at least two characters besides spaces (`SendCountdown`):
a line at the head of the composer reads "Sending to <name>…" beside a
ring that fills over the wait, and "Esc to cancel". Escape, typing in the
field or clicking it calls the send off and leaves the words; Enter sends
at once; otherwise the field goes when the wait is over, through Send.
A cancelled or failed dictation never counts down.

The side thread's composer dictates the same way; the Dictate shortcut
reaches only the conversation's.

The controller knows an engine only as a `DictationEngine`: capability,
permit, start with a callback for events, stop for the final text, and
cancel, where events are whole-utterance text, a level already in 0 to 1,
an error, or the session's end. This Mac's engine is one adapter
(`macEngine`), which turns its dBFS into a level. The desk's is another
(`deskEngine`, `ui/src/voice/desk.ts`): the window opens the microphone
through the call's audio (`ui/src/voice/audio.ts`), meters each block's
RMS in dBFS onto the same 0 to 1 scale, and keeps the samples at 16 kHz.
The desk's model hears whole clips, so about once a second, while no
answer is outstanding and something new was said, the window sends the
clip so far to `voice.transcribe` and shows the words as the session's
partial text; stopping sends the whole clip and waits up to twenty
seconds for its words. A session ends itself after thirty seconds, as a
pause ends one on a Mac: its words are final and a fresh session listens
on, so no clip is longer than the desk takes. A microphone that goes away
ends the dictation with an error. On a Mac whose engine hears, each
session is this Mac's unless the person picked something other than On
this Mac for hearing and the desk has a model; each engine is asked its
permission the first time it is chosen (`eitherEngine`). Every other
window uses the desk's. The meter reads any stream of 0 to 1 levels
(`LevelSource`), so it does not depend on dictation.

## Shortcuts from any app

Three shortcuts work while Hotline is in the background, through the
`global-shortcut` plugin (`ui/src/hotkeys.ts`): Dictate, `Control+Option+H`
(⌃⌥H) unless changed, offered only where the window can dictate; Call your
agent; and Call the desk, both off until set. All are this computer's, kept
in the window's `localStorage` under `hotline.hotkeys` (Call your agent as
`conversation`, read from `call` where it was stored under that name), and set in Settings ›
General › Shortcuts, where a row records new keys (at least one of
Control, Option or Command, or Ctrl or Alt elsewhere, with a key; Escape
gives up), turns the shortcut off, and says when the system would not
give Hotline the keys. Keys another shortcut, or one of the window's own
chords, already uses are refused there. While keys are being recorded,
every shortcut is let go so the recorder hears them.

The plugin reports each press and release. A Dictate press brings the
main window forward and goes, with its release, to the open
conversation's composer, as a tap or a hold as above; with a pane open in
its place, the pane closes and the last teammate's conversation opens and
takes it; with no teammate selected it does nothing. A Call your agent or
Call the desk press hangs up a live call where the person is, without
bringing the window forward, or else brings it forward and calls the open
teammate, or the desk, when the desk can. The window comes forward from the page (`showWindow`), which is
why the main window may show and unminimize itself. The window registers
the shortcuts on startup, again whenever they change, and lets them go
when they are turned off; a reloaded page first lets go of the ones its
previous load held. Only the main window may register shortcuts. Help ›
Keyboard shortcuts lists them, with their current keys, under Anywhere on
this computer.

## Hearing on the desk

The desk can turn speech into text itself, with no provider and no network
(`voice/speech/local.rs`). The engine is sherpa-onnx 1.13.8, whose build
links that release's prebuilt static library, onnxruntime inside, so it runs
wherever the desk does (macOS arm64 and x86_64, Windows x64, Linux x64 and
arm64) with nothing installed beside it. It adds about 18 MB to a stripped binary
on a Mac and 26 to 30 MB on Linux. The models are NVIDIA's Parakeet transducers as sherpa-onnx exports
them, quantized to eight bits, most accurate first:

| Model | id | Hears | Download | On disk |
| -- | -- | -- | -- | -- |
| Parakeet | `parakeet-tdt-0.6b-v3` | 25 European languages | 487 MB | 670 MB |
| Parakeet English | `parakeet-tdt-110m-en` | English | 108 MB | 136 MB |

Both are CC BY 4.0, which asks for credit: each model's card in Settings
names NVIDIA, the licence and sherpa-onnx's quantization, and links the
licence.

Nothing is installed until the owner asks. `voice.model_install` downloads
the model's one archive from sherpa-onnx's `asr-models` release into
`<data dir>/speech-models/<id>.download`, hashing it as it arrives; an archive
that is not the size and SHA-256 pinned in `local/install.rs` is deleted
before any of it is read. From a verified archive only the model's four files
are taken, by name (`encoder.int8.onnx`, `decoder.int8.onnx`,
`joiner.int8.onnx`, `tokens.txt`), whatever path the archive gives them; links
and every other entry are skipped. They go into `<id>.unpacking/` beside a
`model.json` recording each file's size and SHA-256, and the directory is
renamed to `<id>/` once it is whole, so a model is all there or not there.
What a run left half done is deleted when the desk starts; nothing resumes.
`voice.models` reports each model as `available`, `downloading` (with
`receivedBytes`), `unpacking` or `installed`, with the last failure; the
window asks again while a download runs. `voice.model_cancel` stops a
download and throws away what arrived; `voice.model_remove` takes a model off
the disk, and a call hearing with it finds it gone at its next utterance.

An installed model is a directory named for its id whose `model.json` lists
every file at its size, so hearing never reads the download catalogue and a
model the catalogue later drops still hears and can be removed. The engine is
C++ behind a C API, and an exception it throws cannot be caught in Rust: it
would stop the desk. Two inputs make it throw, and neither reaches it. Each
file is hashed against its record the first time the model loads in a run,
and a damaged model is refused with a sentence; and audio shorter than a
tenth of a second is heard as nothing without running the model.

One model is in memory at a time. It loads in about half a second (the first
load of a run also hashes it: about a second more for Parakeet), holds about
0.4 GB (English) or 1.2 GB (Parakeet), and is let go after five minutes
unused. Decoding takes up to four threads and one utterance at a time. On an
M5 Max, 2.4 seconds of speech is heard in about 40 ms by the English model
and 155 ms by Parakeet.

The adapter is provider `local`, named On the desk, and hears `audio/wav`
(mono PCM16 at any rate, which the engine resamples), `audio/mp4` (AAC,
decoded with symphonia) and live PCM, which it gathers until the turn ends,
so a phone that streams its microphone keeps streaming it. It takes at most a
minute at a time and never speaks. Its price is zero, so a zero Voice
budget never stops it.

In the window the models are rows in Settings › Providers › Use for ›
Hearing (`ui/src/components/DeskModels.tsx`), a fold under Voice that opens
even when nothing can speak yet. Folded, it says what hears you now; while
nothing can, it reads "None yet. Download a free speech model." with Set up.
Open, it holds Hears with and the models. The call assistant is a row of its
own below the fold, whose line says it is only for calls to the desk, since a
teammate answers its own calls. A row says what the model hears and its download
size, and its Download button names the size; while it downloads the row
shows how much has arrived over a bar and offers Cancel, and once installed
it shows its size on the desk and offers Remove. The window asks
`voice.models` every half second while anything is downloading or unpacking,
and asks for the options again when what is installed changes, so the
Hearing picker gains or loses On the desk. A failed download's sentence takes
the row's second line until the next try. Under the rows each model's credit
links its licence, and About credits the models, sherpa-onnx, ONNX Runtime
and Symphonia.

`voice.transcribe` hears one clip outside any call, for dictation: with the
model picked for hearing when that is one of the desk's, else the first
installed. It asks no budget and keeps nothing.

## Providers

These are the defaults, what a provider uses when the owner picks nothing and
what the picker shows when a provider cannot be asked what else it offers.

| Connected as | Listens with | Speaks with |
| -- | -- | -- |
| `openai` | `gpt-4o-mini-transcribe` | `gpt-4o-mini-tts`, voice `marin`, WAV |
| `google` | `gemini-3.5-flash-lite` | `gemini-3.8-flash-tts`, voice `Sulafat`, WAV |
| `openrouter` | `openai/whisper-large-v3-turbo` | `x-ai/grok-voice-tts-1.0`, voice `eve`, MP3 |
| `groq` | `whisper-large-v3-turbo` | `canopylabs/orpheus-v1-english`, voice `hannah`, WAV |
| `mistral` | `voxtral-mini-latest` | none |
| `xai` API key | `grok-voice-transcribe-2.0` | native `/v1/tts`, voice `eve`, WAV |
| explicit `xai-subscription` login | `grok-voice-transcribe-2.0` | native `/v1/tts`, voice `eve`, WAV |
| a custom `openai-compatible` connection | its first model named `whisper` or `transcribe` | its first model named `tts` or `speech`, voice `alloy`, MP3 |

The OpenAI, Groq, OpenRouter, Mistral and custom rows share one adapter: a
multipart `POST {base}/audio/transcriptions` with `model` and `file` and
nothing else, and a JSON `POST {base}/audio/speech` with `model`, `input`,
`voice` and `response_format`. Groq's Orpheus takes 200 characters a request,
so a longer sentence is cut at a sentence end, a comma or a space, spoken in
parts and joined.

Google has its own adapter. It transcribes with `generateContent` and the clip
inline (`audio/mp4` goes as `audio/m4a`), and speaks with `generateContent` on
the TTS model, sending the words and nothing else: Gemini TTS reads any style
instruction aloud. It returns raw PCM at the rate its mime type names, which is
wrapped as a WAV. The key travels in `x-goog-api-key`, never the URL.

xAI has its own native adapter. Whole-clip STT uses multipart `POST /v1/stt`;
live STT uses `wss://api.x.ai/v1/stt` with mono PCM16 at 16 kHz. TTS sends
`text`, `voice_id`, `language: "auto"`, and a PCM output format to `POST /v1/tts`.
That route chooses a voice rather than a model; `grok-voice-tts-1.0` identifies
the speech job in settings. Voices are `eve`, `ara`, `rex`, `sal`, and `leo`.
Returned 24 kHz PCM is wrapped into a whole WAV or independently playable
progressive WAV chunks. API keys travel in Authorization headers.

The separate **Grok subscription** choice uses the existing stored xAI OAuth
login. It must be selected explicitly as `xai-subscription`; it is excluded
from automatic speech selection and never falls back to paid API-key speech.
Each request resolves fresh stored credentials and retries once after a
rejected bearer. Sign-in and entitlement failures are typed voice errors.
The voice ledger adds no API speech cost for this selection; provider-side
subscription limits still apply. Live STT retains finalized partial segments
when the completion frame omits text, without dispatching interim recognition.

### What the picker offers

Settings › Providers › Use for lists every speech model the owner's connected
providers offer, not just the defaults, after the desk's own installed models
under On the desk. `capabilities.options` asks each
built-in provider for its model list and sorts it into hearing and speaking
models: OpenAI (`transcribe` and `whisper` ids, and `tts` ids, without dated
snapshots or the diarizing model), Google (Gemini flash text models for
hearing, `-tts` models for speaking), Groq (`whisper*`, `orpheus*`), Mistral
(Voxtral for hearing only, leaving out its realtime models) and OpenRouter
(its speech and transcription catalogues, with each model's own
`supported_voices`; a model that lists no voices is left out, because we
cannot ask it for one). OpenAI, Google and Groq do not list voices, so the
documented sets are used, the default first. All OpenRouter models are asked
for MP3; OpenAI and Google for WAV; Groq's Orpheus for WAV, 200 characters a
request.

Each provider's list is cached for a day in `cache/speech-models-{provider}.json`
under the room, holding model names and voices and never a key. Fetches time out
after five seconds and run together; one that fails falls back to the cached copy,
even a stale one, and then to the provider's default, so the picker is never
empty for want of a network. Starting a call never asks: it resolves from
settings, the defaults and the cache. A model picked without a voice speaks in
its own first voice.

Not covered: Mistral's voice, which answers with base64 inside JSON. A custom
connection counts only if it lists a speech model or the owner names one.
xAI subscription logins use their own explicit speech choice, independently
of the API-key connection.

## Choosing

Each job goes to the first connected provider that can do it, in the order the
credentials were created; the fallback is the next connected provider that can
speak. Hearing goes first to a model installed on the desk, the most accurate
one, because it costs nothing, nothing said leaves the machine, and
installing it was the owner's own act; the order among paid keys is
unchanged. Subscription speech is selected explicitly and has no paid
fallback.
`settings.voice` overrides any of it:

```json
{ "stt": { "provider": "groq", "model": "whisper-large-v3-turbo" },
  "tts": { "provider": "openai", "model": "gpt-4o-mini-tts", "voice": "cedar" },
  "fallbackTts": { "provider": "google" },
  "dispatcher": { "provider": "openai", "model": "gpt-5-mini" } }
```

Every key is optional and a value that cannot be read costs only itself. A
`stt` or `tts` that names a provider that is not connected, or cannot do the
job, is an error, not a quiet switch: the audio would go to a provider the
owner did not choose. `"stt": {"provider": "local", "model": "parakeet-tdt-110m-en"}`
hears with that model on the desk, and naming one that is not installed is
the same error. A `fallbackTts` that cannot be used is no fallback.
When nothing can hear or speak, `resolve` returns a sentence for a person.

`dispatcher` names the chat model that routes what was said on a call to the
desk; Settings › Providers › Use for calls it the call assistant. A call to a
teammate never uses it. Without that setting the
desk takes the best-suited quick chat model of the room's default provider: a
middle-tier one (`flash`, `mini`, `small`, `fast`) first, because the lightest
tier (`flash-lite`, `nano`, `luna`, `haiku`, `instant`) is too thin to hold a
conversation, then a light one, then any other. A model whose id says `tts`, `embed`, `whisper`, `transcribe`, `image` or `audio` is never
picked, because a gateway lists those beside its chat models. `provider` alone
picks by the same order within that provider; `model` is taken as given. A provider that is
not connected is an error, like the speech choices above.

A direct teammate call resolves speech and budget without the dispatcher.
`VoiceStatus.available` remains desk readiness; `directAvailable` separately
reports direct readiness, which needs speech and the Voice budget only.
Speech selections remain visible when only the
dispatcher is unavailable. The desktop's secondary call control uses direct
readiness and retains the desk's availability check for its primary call.
`voice.status` accepts `inputMode: "text"` to assess output-only readiness;
omission assesses audio readiness. A direct text call needs output and budget,
while a desk text call also needs the dispatcher.

## One brain, two outputs

A call to a teammate has nothing in front of the teammate
(`voice/spoken.rs`). Every utterance goes into the teammate's own session as a
turn of its conversation, in the open chapter of its main thread, the same way
a typed message does (`Calls::hand_off`, `Room::prompt`): same session, same
harness, same grants. The tape keeps the person's words exactly as they were
heard, under an id that marks the line as said on the call
(`voice:<callId>:<seq>:agent:…`), and the chat shows them like any message.
Said into a turn that is still running, the words steer it, as typing does.

What makes the turn a voice turn is one thing the agent is handed after the
person's words, the contract (`spoken::CONTRACT`):

> One response, two parts, in this order, separated by `<<<ENDSPEAK>>>`.
> Part 1 is spoken: short, conversational, self-contained, leading with the
> answer; anything visual described in words and never reproduced, with no
> code, tables, lists, links or markdown, and no reference to what follows.
> Part 2 is shown in the chat and never spoken: the full detail, code, tables
> and links. It may be empty. The marker is always written. Anything written
> before using a tool is spoken as it comes, so it is a brief line.

The contract travels in the text of the turn the driver is handed, after the
words and a blank line, and nowhere else. A model setting or a system note
would reach only Hotline Agent, and an ACP meta field is not something Claude
Code or Codex read as instructions; the turn's text reaches every driver alike,
and it rides along when the turn steers one already running. The tape never
holds it: `Room::prompt` writes what the person said and hands the driver the
words with the contract, so the conversation, a rebuilt history and a search
see only the words. Nothing else about the turn changes what the model sees.

The reply is rendered twice from one stream:

- **Said.** The DM's witness hands the call each chunk of the reply's words as
  the agent writes them (`Calls::reply_delta`), and the whole message when it
  lands (`Calls::delivery`). `spoken::Spoken` takes the spoken part: the
  sentences before the marker, each said as soon as it is whole. It holds back
  the end of a chunk that could be the start of the marker, so no part of the
  marker is ever said, and it never reads out a fenced code block or a table.
  A reply with no marker was written to be read, so the call says its opening,
  up to the first code block or table and at most three sentences, and the
  rest is only shown; until the marker comes, a sentence the fallback would
  not say waits for it.
- **Shown.** The chat shows the whole reply with the marker taken out and the
  two parts a paragraph apart, as it streams (`spoken::Unmarked`, which holds
  back a partial marker the same way) and as it is written to the tape
  (`spoken::unmarked`, before the reply is paced into bubbles). Only the
  first marker is the reply's. A turn not said on a call is shown as written.

On an agent turn, what the agent writes before its first tool call is said as
it streams, as the acknowledgement. Its words between tools are narration
(`session/narration.rs`) and are not said; the call stays `thinking`, and the
client's blip-blip covers the work. The message that lands as the report is
said whole when it lands, up to its marker. Each message said is one reply:
one `said` id, the line growing sentence by sentence, its clips under that id
closed by an empty final clip, and one line on the call's thread.

The call is `thinking` from an utterance until the teammate's session has
finished that turn, or left it open only for subagents (`Calls::turn_ended`),
apart from while it speaks; then it listens. When the person speaks again
before a turn ends, the call waits for the turn that has their latest words.
A reply the turn never finished is said as far as it got. While it thinks it
sends `thinking`
again every 15 seconds, so a phone, which gives up on a desk it has not heard
from in 45, does not take a long piece of work for a desk that went quiet.

Speaking over the teammate (`voice.interrupt`) stops what it is saying at once
and gives the person the floor: the call listens even though the turn is still
open, the rest of that reply is not said, and what the person says next goes
into the open turn as a steer. The turn itself is never stopped by the call,
and what the teammate says after it, such as its report, is said. A hold stops
a reply too, and a reply cut off by a hold, or that lands while the call is
held, reaches the phone as a notification instead.

What is said is cleaned for speech first (`spoken::speech_text`): code marks,
markdown emphasis, headings and list bullets go; a link is "a link", and a
labelled link is its label; money, scales and percentages are words ("$3.4B"
is "3.4 billion dollars", "12%" is "12 percent"), "->" and "=>" are "to", "#42"
is "number 42" and "~5" is "about 5". The line shown on the call keeps the
words as written. The marker is the only markup the agent is asked for.

The latency of a call to a teammate is the teammate's: its first word waits on
its model's first sentence. That is accepted. A desk call keeps its router:
the call assistant answers what was said on the desk, hands work to teammates
with `session.prompt`, and narrates their replies, as below.

A desk call's dispatcher answers are spoken sentence by sentence as the model
writes them, as one reply: one line growing under one `said` id, its audio
continuing under that id and ending with an empty final clip, kept once on the
`voice-dispatcher` tape when it is over (or when the person speaks over it,
with what was said by then). A teammate's reply to work the desk handed it is
narrated by the call assistant, plainly and briefly, when it is too long or
too marked up to say as written.

A direct call is a thread (`docs/threads.md`): everything said on it is kept in
`calls/<id>.jsonl`, indexed for `search_thread`, linked from the teammate's DM
and read back after the call ends. A call nobody has spoken on for ten minutes
is ended by the room's sweep, and one a restart cut off is closed as stopped.
The desk's own calls, which name no teammate, are not threads; they stay on the
`voice-dispatcher` tape, which the dispatcher reads across calls.

A teammate may have its own voice (`Persona.voice`: provider, model and
voice), picked on its card from the voices of the model the desk speaks with.
A direct call to that teammate speaks in it while the desk still speaks with
that provider and model; otherwise, or if the provider refuses the voice, the
call uses the desk's voice. Desk calls always use the desk's voice.

## Timing

Every provider call logs three moments, tagged with the provider and model, to
stderr: when the response headers arrived, when the first byte of the body did,
and when it was whole. A provider that streams can send headers early and sound
late, and one that does not lands all three together.

```
[voice] speak openai/gpt-4o-mini-tts: headers 180ms, first byte 412ms, done 655ms
```

The desk times each accepted utterance to its first published clip, including
the durable STT reservation. The line includes
the call and sequence, not the words or audio; it measures desk publication,
not transport or client playback latency:

```text
[voice] utterance to first clip call=<call-id> seq=0: 1412ms
```

A shared speech adapter set also logs `transcription to synthesized clip` when
its first TTS result follows transcription. That provider-only measurement
excludes bundled desk audio and is separate from the call measurement.

## Clip checks

`voice/speech/clip.rs` has two checks that need only a clip's size and length.
Neither is called by the speech layer; the desk asks them beside its own.

- `billable_ms(mime, bytes, claimed_ms) -> u32` is what a speech-to-text
  reservation should use for a clip's length. An MP4 carries its length in a
  header the client wrote, so an MP4 is never billed for less than could fit in
  its bytes at 32 kbit/s (at most 20 s); a WAV is billed as claimed, since its
  length is its size.
- `plausible_goodbye(duration_ms, bytes) -> bool` is a necessary condition for
  hanging up on a farewell: a clip of at least 400 ms (`MIN_GOODBYE_MS`) with at
  least a byte for each millisecond. Whisper-style engines answer noise with
  "Bye." or "Thank you.", so the desk asks `goodbye(text) &&
  plausible_goodbye(duration_ms, bytes)`. This rejects short or sparse clips;
  it cannot distinguish noise from speech in a longer, sufficiently large clip.

## The ledger

`voice/ledger.rs` keeps today's and this month's spend in
`<data dir>/voice-ledger.json`, split into speech to text, text to speech and
dispatcher. `spent()` reads what each kind has spent today and this month
and `charge(kind, usd)` records a cost. `reserve(kind, usd)` records an
estimate and returns a `Reservation` naming the day and month it was charged
to; `settle(reservation, actual)` replaces it with the actual cost. A cost
above the estimate is charged in full to the day it became known. A cost below
it is refunded from the reservation's own day and month while they are still
current, so a day that rolled over in between keeps its estimate and its
month is refunded. A refund never takes a total below zero or a month below
its day. A reservation that is dropped, or settled with a cost that is not a
number, stays charged.

Each kind is spent against a budget of `settings.spending`: speech to text
and text to speech against **Voice**, the call assistant against **Chat**.
Chat also covers teammates' own turns on per-token keys, which the same
`Budget` keeps in `<data dir>/chat-ledger.json` (kind `teammates`); the Chat
budget is judged on both files together. There is one `Budget` per desk,
shared by calls and teammates. Images have their own budget and
ledger. A budget has optional daily and monthly limits and none by default;
no limit is never spent, and zero turns that budget's paid use off. A room
that never set `settings.spending` but kept an early version's
`settings.voice.dayUsd` and `monthUsd` has those as its Voice limits. A
spending setting that cannot be read turns paid chat and voice off.

`voice/metering.rs` is the gate. `reserve(kind, usd)` is refused with
`Exhausted::Day(budget)`, `Month(budget)` or `Off(budget)` when the kind's
own budget would go over a limit, and each says which budget in a sentence
("The Voice budget for today is spent. Raise it in Settings › Budgets.").
A reservation of nothing (a subscription, the desk's own engine, a signed-in
call assistant) is never refused and never reads the ledger, so free voice
runs even when the ledger cannot be read. `ready(kinds)` asks the same of
the kinds a call would pay for before work starts: transcription and speech
when their provider charges, and on a desk call the call assistant when it is
billed per token. A call to a teammate pays for no call assistant; its turns
are the teammate's own and are metered by its session, against Chat on a
per-token key, as typed turns are. `voice.call_start`
refuses a call whose paid budget is spent with that sentence; during a call
a refused reservation ends it with the bundled budget line. A call that
pays for nothing names no kinds, so no budget can end it.

Days and months are the desk host's local calendar. A clock that goes backwards
keeps counting against the later day. The ledger fails closed: a file that
exists and cannot be read or understood, or a charge that could not be written
down, makes paid reservations fail until it can. Reading is tried again on
every read, so mending the file mends the ledger, and so is a write that
failed: each read (and each status read) writes the balance it is holding
again, so a disk that comes back turns paid voice back on without a restart. The fsync runs without
the ledger balance lock held. A healthy direct ledger check need not wait for
another write, but retrying a failed write still waits for disk. The desk's
`Budget` gate also serializes checks and reservations through persistence so
paid work cannot start before its reservation is durable. Slow disk therefore
still affects calls. On a multi-thread runtime writes use `block_in_place` so
they do not hold a runtime worker.

Prices in the ledger module are rounded up, since they are a guard and not an
invoice: speech to text per minute and text to speech per 1,000 characters by
provider, one high price for a provider not in the table, and zero for the
desk's own model (`local`). Speech is priced from what is sent (seconds of
audio, characters of text) and no provider reports usage back, so its
reservation is its charge and is never settled.

The call assistant is not in that table. `voice/dispatcher.rs` prices its
model from the vault's model metadata first, then the bundled catalogue; a
sign-in or a local server costs nothing. Each model call reserves an estimate
before it goes out: the request's bytes (prompt, history, preamble, and 2 KB
per tool) divided by three as input tokens, plus the request's own output
ceiling (512 tokens for an answer, 160 for a narration). When the response
reports usage, the reservation is settled to what it cost: `input_tokens` at
the input price, cache reads and writes at the catalogue's cache prices, and
output. Anthropic reports cache tokens beside `input_tokens`; the
OpenAI-style APIs count them inside it, and are priced net of them. Tokens the
total holds beyond input and output (a Gemini model's thinking) are priced as
output. A blocking call settles on its response; a streamed call settles when
its turn finishes, which includes turns that only call a tool. A call that
fails, is cancelled, or reports no usage keeps its reservation, since the
provider may have billed it. A cost above the estimate is written down in
full and can end the run when it spends the cap.

A model on an API key with no listed price is metered at a guard rate of $5
per million input tokens and $25 per million output tokens (`UNPRICED`), so
the caps still bound it. The desk logs once per model that it is doing so
(`[pricing] … has no listed price`), because the budget then runs down faster
than the bill. The fix is a catalogue entry: run `hotline-models-sync` (see
[development.md](development.md#the-model-catalogue)). The catalogue holds
one price per model, so a model priced by prompt length (Claude Haiku 5.5
above 100,000 prompt tokens) is metered at its base price.

## Checking against the real endpoints

`speech_check` speaks a sentence through each provider whose key is in the
environment and has each provider transcribe it back, on the real endpoints
and through `resolve`, on a scratch data directory:

```sh
OPENAI_API_KEY=... GEMINI_API_KEY=... cargo run -p hotline-core --example speech_check
```

`local_speech_check` downloads one of the desk's own models from its pinned
release, verifies and unpacks it through the desk's own code, and hears the
speech fixtures with it, on a scratch data directory, printing how long each
step took. It is the check that the pinned archive is still there and the
engine links and runs on the machine at hand:

```sh
cargo run --release -p hotline-core --example local_speech_check -- parakeet-tdt-110m-en
```

On an M5 Max over a home line the English model took 5 seconds to fetch,
check and unpack and Parakeet 24; the first clip after that took 0.5 and 1.8
seconds (loading, and the run's one hash check), and each clip after it 40
and 150 ms.

The tests in `voice/speech/` use mock servers on localhost and never reach the
network. `tests/local_speech.rs` serves small fake archives on localhost, and
runs a real model only when `HOTLINE_TEST_SPEECH_MODEL` names a directory
holding one.
