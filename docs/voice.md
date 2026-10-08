# Voice: hearing, speaking, and what it costs

The desk turns a spoken clip into words and words into a spoken clip through
a provider the owner has already connected. Voice never asks for a key of its
own. The call, the dispatcher and the wire commands are in
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
providers offer, not just the defaults. `capabilities.options` asks each
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
speak. Subscription speech is selected explicitly and has no paid fallback.
`settings.voice` overrides any of it:

```json
{ "dayUsd": 2, "monthUsd": 20,
  "stt": { "provider": "groq", "model": "whisper-large-v3-turbo" },
  "tts": { "provider": "openai", "model": "gpt-4o-mini-tts", "voice": "cedar" },
  "fallbackTts": { "provider": "google" },
  "dispatcher": { "provider": "openai", "model": "gpt-5-mini" } }
```

Every key is optional and a value that cannot be read costs only itself. A
`stt` or `tts` that names a provider that is not connected, or cannot do the
job, is an error, not a quiet switch: the audio would go to a provider the
owner did not choose. A `fallbackTts` that cannot be used is no fallback.
When nothing can hear or speak, `resolve` returns a sentence for a person.

`dispatcher` names the chat model that routes what was said. Without it the
desk takes the best-suited quick chat model of the room's default provider: a
middle-tier one (`flash`, `mini`, `small`, `fast`) first, because the lightest
tier (`flash-lite`, `nano`, `luna`, `haiku`, `instant`) is too thin to hold a
conversation, then a light one, then any other. A model whose id says `tts`, `embed`, `whisper`, `transcribe`, `image` or `audio` is never
picked, because a gateway lists those beside its chat models. `provider` alone
picks by the same order within that provider; `model` is taken as given. A provider that is
not connected is an error, like the speech choices above.

A direct teammate call resolves speech and budget without the dispatcher.
`VoiceStatus.available` remains desk readiness; `directAvailable` separately
reports direct readiness. Speech selections remain visible when only the
dispatcher is unavailable. The desktop's secondary call control uses direct
readiness and retains the desk's availability check for its primary call.
`voice.status` accepts `inputMode: "text"` to assess output-only readiness;
omission assesses audio readiness. A direct text call needs output and budget,
while a desk text call also needs the dispatcher.

When the dispatcher is ready, a direct call answers in the teammate's own
voice before the teammate does anything. The dispatcher's model speaks as the
teammate, in the first person, and converses: the system prompt carries the
teammate's name and goal and the workspace's own `AGENTS.md` when a person
wrote one (up to 4000 characters; the file Hotline writes is skipped, and a
linked file is never followed). The call's own exchange (what the person said,
what the voice said, and the reports it relayed; the newest 30 lines) is sent as
real user and assistant turns, so the voice keeps the thread of the call. Each
message also carries, as data, whether the teammate is working, the note its
latest chapter closed with and the newest 24 entries of its conversation. A
scheduled prompt, a colleague's message or an answer to a request is named for
what it is in that data, never as something the person said. A question about
how the work is going is answered from that and reaches no session.
The exchange is only what the voice is given: a direct call is a thread
(`docs/threads.md`), and everything said on it is kept in `calls/<id>.jsonl`,
indexed for `search_thread`, linked from the teammate's DM and read back after
the call ends. A call picked up again under its id rebuilds its exchange from
that thread, under the same caps. A call nobody has spoken on for ten minutes
is ended by the room's sweep, and one a restart cut off is closed as stopped.
The desk's own calls, which name no teammate, are not threads; they stay on the
`voice-dispatcher` tape, which the dispatcher reads across calls.
A request for work calls the front's one tool, `hand_to_session`, which hands
the words to the teammate's session as a call without a front would, at most
once per utterance; the front then says a short acknowledgement. The session
hears the call lines it has not yet been told (the person's and the voice's,
not its own relayed reports) ahead of the person's exact words, framed as the
call's; the conversation shows only the words. The front never claims work is
done and never answers an approval. If the front fails before it decides, the
words are handed over unchanged. A handoff that cannot start the teammate or
reach its session is reported on the call.

While the front speaks for the call, the teammate's own interim
acknowledgements are not spoken, nor a turn that ends on a bare one; its
reply at the end of the turn is. Replies
to any turn handed off on this call are delivered, not only to the latest one,
and longer replies are retold in the first person by the dispatcher's model,
which summarises lists rather than reading them out.
Short plain replies are spoken as written. Without a dispatcher, a direct call
hands every utterance to the session and speaks its replies as before.

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
dispatcher. `check(settings)` returns `Err(Exhausted)` once either cap is spent
and `charge(kind, usd)` records a cost. The caps are `settings.voice.dayUsd`
and `monthUsd`, $2 and $20 by default. Zero turns paid voice off: a cap
only counts as spent once something was spent against it, so a subscription's
free voice still runs and any reservation that costs is refused. Once the owner
has set `settings.spending`, its `dayUsd` and `monthUsd` govern voice instead,
so one cap covers images and voice. Voice's ledger and the image ledger are
still separate tallies; Settings shows their sum.

Days and months are the desk host's local calendar. A clock that goes backwards
keeps counting against the later day. The ledger fails closed: a file that
exists and cannot be read or understood, or a charge that could not be written
down, makes `check` fail until it can. Reading is tried again on every check,
so mending the file mends the ledger, and so is a write that failed: each
`check` (and each status read) writes the balance it is holding again, so a disk
that comes back turns voice back on without a restart. The fsync runs without
the ledger balance lock held. A healthy direct ledger check need not wait for
another write, but retrying a failed write still waits for disk. The desk's
`Budget` gate also serializes checks and reservations through persistence so
paid work cannot start before its reservation is durable. Slow disk therefore
still affects calls. On a multi-thread runtime writes use `block_in_place` so
they do not hold a runtime worker.

Prices in the ledger module are rounded up, since they are a guard and not an
invoice: speech to text per minute and text to speech per 1,000 characters by
provider, one high price for a provider not in the table. The dispatcher is not
in that table: `voice/dispatcher.rs` reserves each call from its model's own
price, the vault's model metadata first, then the bundled catalogue, and $5 per
million input tokens and $25 per million output tokens when neither has one.

## Checking against the real endpoints

`speech_check` speaks a sentence through each provider whose key is in the
environment and has each provider transcribe it back, on the real endpoints
and through `resolve`, on a scratch data directory:

```sh
OPENAI_API_KEY=... GEMINI_API_KEY=... cargo run -p hotline-core --example speech_check
```

The tests in `voice/speech/` use mock servers on localhost and never reach the
network.
