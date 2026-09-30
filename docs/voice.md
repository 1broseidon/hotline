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

The fallback is the desk's to use: when the voice fails it asks `fallback_tts`,
once (`Calls::synthesize`), and reserves the second call's cost like the first.
Each clip carries its own type: `audio/wav` where the provider can make one,
`audio/mpeg` where it cannot.

A speech error carries the provider and an HTTP status and never a response
body, because a provider's error text can echo what was said.

## Providers

| Connected as | Listens with | Speaks with |
| -- | -- | -- |
| `openai` | `gpt-4o-mini-transcribe` | `gpt-4o-mini-tts`, voice `marin`, WAV |
| `google` | `gemini-3.5-flash-lite` | `gemini-3.8-flash-tts`, voice `Sulafat`, WAV |
| `openrouter` | `openai/whisper-large-v3-turbo` | `x-ai/grok-voice-tts-1.0`, voice `eve`, MP3 |
| `groq` | `whisper-large-v3-turbo` | `canopylabs/orpheus-v1-english`, voice `hannah`, WAV |
| `mistral` | `voxtral-mini-latest` | none |
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

Not covered: xAI, whose speech is `/v1/tts` and `/v1/stt` and not the OpenAI
shape (its voice is reachable through OpenRouter), and Mistral's voice, which
answers with base64 inside JSON. A custom connection counts only if it lists a
speech model or the owner names one.

## Choosing

Each job goes to the first connected provider that can do it, in the order the
credentials were created; the fallback is the next connected provider that can
speak. `settings.voice` overrides any of it:

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
desk takes the quickest chat model of the room's default provider: a model whose
id says `tts`, `embed`, `whisper`, `transcribe`, `image` or `audio` is never
picked, because a gateway lists those beside its chat models. `provider` alone
picks that provider's quickest; `model` is taken as given. A provider that is
not connected is an error, like the speech choices above.

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
and `monthUsd`, $2 and $20 by default; zero turns voice off. Once the owner
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
