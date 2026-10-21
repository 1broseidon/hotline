These fixed system sentences remain playable without a paid speech request:

| File | Spoken text |
| --- | --- |
| `retry.wav` | Sorry, say that again. |
| `error.wav` | Voice keeps failing. Please continue by text. |
| `goodbye.wav` | Goodbye. |
| `live-budget.wav` | The Voice budget is spent. Raise it in Settings or choose Legacy voice. |
| `live-unavailable.wav` | Live voice is unavailable. You can try again or choose Legacy voice. |
| `budget.wav` | The voice budget is unavailable or spent. Chat carries on by text. |

Generated locally with FFmpeg's Flite `slt` voice, resampled to 16 kHz mono
PCM16 WAV. There is no runtime Flite or FFmpeg dependency. The matching text
lives in the parent module; update the recording when changing a sentence.
Every bundled clip is explicitly tagged `audio/wav`, regardless of provider.

```sh
ffmpeg -f lavfi -i "flite=text='Sorry, say that again.':voice=slt" -ar 16000 -ac 1 -c:a pcm_s16le -map_metadata -1 retry.wav
```

The two `live-*.wav` notices use the same Flite `slt` voice directly:
`flite -voice slt -t 'SENTENCE' -o FILE.wav`. They are 16 kHz mono PCM16,
play once before the terminal live state, and make no provider request.
