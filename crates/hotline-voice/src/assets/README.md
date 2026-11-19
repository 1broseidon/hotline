These fixed system sentences remain playable without a paid speech request:

| File | Spoken text |
| --- | --- |
| `retry.wav` | Sorry, say that again. |
| `error.wav` | Voice keeps failing. Please continue by text. |
| `goodbye.wav` | Goodbye. |
| `live-budget.wav` | The Voice budget is spent. Raise it in Settings or choose Turn by turn. |
| `live-unavailable.wav` | Live voice is unavailable. You can try again or choose Turn by turn. |
| `budget.wav` | The voice budget is unavailable or spent. Chat carries on by text. |
| `unspoken.wav` | I can't speak right now. My answer is on screen. |

Generated locally with FFmpeg's Flite `slt` voice, resampled to 16 kHz mono
PCM16 WAV. There is no runtime Flite or FFmpeg dependency. The matching text
lives in the parent module; update the recording when changing a sentence.
Every bundled clip is explicitly tagged `audio/wav`, regardless of provider.

```sh
ffmpeg -f lavfi -i "flite=text='Sorry, say that again.':voice=slt" -ar 16000 -ac 1 -c:a pcm_s16le -map_metadata -1 retry.wav
```

`waiting.wav` is a 240 ms, 330 Hz tone with a subtle 660 Hz overtone (0.25x) at peak amplitude 1400/32768, with a sine-squared envelope, sampled as 16 kHz mono PCM16. It is a warm, soft presence cue, with no speech. The core sends it at most once per turn after 8 seconds of unexplained silence, only if a Turn by turn reply has not started speaking.

The two `live-*.wav` notices and `unspoken.wav` use the same Flite `slt`
voice directly:
`flite -voice slt -t 'SENTENCE' -o FILE.wav`. They are 16 kHz mono PCM16
and make no provider request. The live notices play once before the terminal
live state; `unspoken.wav` plays the first time in a call that the speech
provider refuses to speak, under a shown line that names the provider.
