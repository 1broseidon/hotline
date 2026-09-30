`budget.wav` is a fixed system sentence, used when paid speech is unavailable:
“The voice budget is unavailable or spent. Chat carries on by text.”

Generated locally with FFmpeg's Flite `slt` voice, then resampled to 16 kHz mono
PCM16 WAV. There is no runtime Flite or FFmpeg dependency. Its text is
`BUDGET_LINE` in the parent module; change the recording when changing that text.

```sh
ffmpeg -f lavfi -i "flite=text='The voice budget is unavailable or spent. Chat carries on by text.':voice=slt" -ar 16000 -ac 1 -c:a pcm_s16le -map_metadata -1 budget.wav
```
