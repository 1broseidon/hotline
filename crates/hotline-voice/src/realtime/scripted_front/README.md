The fixed spoken deck in `../scripted_front.rs` runs on Grok Voice, GPT Realtime
and GPT-Live. These mono PCM16 24 kHz WAVs were made locally with macOS Samantha:
`/usr/bin/say -v Samantha --file-format=WAVE --data-format=LEI16@24000 -o question.wav 'Can you hear me?'`.
The exact text of each clip is beside its include in the test.

Run `cargo test -p hotline-voice scripted_front_ -- --ignored --nocapture`
with `HOTLINE_LIVE_XAI_KEY`, `HOTLINE_LIVE_OPENAI_KEY` and
`HOTLINE_TEST_CHATGPT_DIR` (fresh login directory) set. Missing credentials fail
explicitly. Each model gets one call for the whole deck, so acknowledgements
are checked across turns. The regular test run proves the checker catches
relay language and repeated acknowledgements. Real models require the opt-in
run; canned replies cannot establish their behaviour.

Speech-to-speech audio is already heard when its transcript arrives. The prompt,
tool framing and this regression check are the enforcement, not an audio filter.
