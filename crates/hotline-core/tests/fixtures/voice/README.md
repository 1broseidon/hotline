These PCM16, 16 kHz, mono WAV fixtures were synthesized locally with FFmpeg's
Flite `slt` voice. `ask-mack.wav` says “Ask Mack to check the failing PR.” (2.39s).
`acknowledgement.wav` says “I have asked Mack to check the failing PR.” (2.79s).
The scripted client uses a fake Speech adapter over these real playable clips;
it proves routing and playback framing, not a provider's transcription accuracy.
`ask-mack.m4a` is `ask-mack.wav` as a phone sends it, AAC in MP4, made with
macOS's `afconvert -f m4af -d aac -b 32000`; the desk's own model decodes it.
