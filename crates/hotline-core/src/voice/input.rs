//! What a call hears: typed words, utterances and audio, checked before any of
//! it is paid for.

use super::ledger::{self, Kind};
use super::settings::VoiceSettings;
use super::speech::local;
use super::speech::{self, Clip, SpeechOutput};
use super::synthesis::own_voice;
use super::{CallSpeech, Calls, FRONT_LINE, Input, MAX_AUDIO, TranscriptSource, Work, realtime};
use crate::contract::{VoiceInputMode, VoiceState};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

impl Calls {
    pub fn utterance(
        &self,
        id: &str,
        seq: u32,
        mime: &str,
        data: &str,
        duration: u32,
    ) -> Result<(), String> {
        if !(1..=20_000).contains(&duration) || !matches!(mime, "audio/wav" | "audio/mp4") {
            return Err("Send a WAV or AAC/MP4 utterance lasting at most 20 seconds.".into());
        }
        if data.len() > MAX_AUDIO * 4 / 3 + 4 {
            return Err("That voice clip is too large.".into());
        }
        let bytes = STANDARD
            .decode(data)
            .map_err(|_| "Voice audio must be standard base64.".to_string())?;
        if bytes.is_empty() || bytes.len() > MAX_AUDIO {
            return Err("That voice clip is empty or too large.".into());
        }
        let duration = validate_audio(mime, &bytes, duration)?;
        self.queue_utterance(id, seq, Some(VoiceInputMode::Audio), |speech| {
            Work::Utterance {
                seq,
                clip: Clip {
                    mime: mime.into(),
                    bytes,
                },
                duration,
                speech,
            }
        })
    }

    /// Only a finalized device transcript enters the paid response pipeline.
    pub fn text(&self, id: &str, seq: u32, text: &str) -> Result<(), String> {
        self.queue_text(id, seq, text, TranscriptSource::Device)
    }

    /// The words a voice front relays to the teammate on this call, handed
    /// over as a typed message is ([`front`]).
    pub(crate) fn relay(&self, id: &str, seq: u32, words: &str) -> Result<(), String> {
        self.queue_text(id, seq, words, TranscriptSource::Front)
    }

    pub(super) fn queue_text(
        &self,
        id: &str,
        seq: u32,
        text: &str,
        source: TranscriptSource,
    ) -> Result<(), String> {
        if text.len() > 32_000 || text.chars().count() > 8_000 {
            return Err("A voice transcript may contain at most 8,000 characters.".into());
        }
        let text = text.trim();
        if text.is_empty() {
            return Err("A finalized voice transcript cannot be empty.".into());
        }
        let input = match source {
            TranscriptSource::Front => None,
            _ => Some(VoiceInputMode::Text),
        };
        self.queue_utterance(id, seq, input, |speech| Work::Text {
            seq,
            text: text.into(),
            source,
            speech,
        })
    }

    /// Work on what was said. `input_mode` is the person's own input, which
    /// a front call refuses; `None` is the front relaying, which only a front
    /// call takes.
    pub(super) fn queue_utterance(
        &self,
        id: &str,
        seq: u32,
        input_mode: Option<VoiceInputMode>,
        work: impl FnOnce(CancellationToken) -> Work,
    ) -> Result<(), String> {
        self.change(id, |call| {
            match (input_mode, call.front.is_some()) {
                (Some(_), true) => return Err(FRONT_LINE.into()),
                (None, false) => return Err("This call has no voice front.".into()),
                _ => {}
            }
            if let Some(input_mode) = input_mode
                && call.input_mode != input_mode
            {
                return Err(match call.input_mode {
                    VoiceInputMode::Text => {
                        "This call accepts finalized text, not microphone audio."
                    }
                    VoiceInputMode::Audio => {
                        "This call accepts microphone audio, not device transcripts."
                    }
                }
                .into());
            }
            if call.state == VoiceState::Held {
                return Err("Resume the call before speaking.".into());
            }
            if call.seq.is_some_and(|previous| seq <= previous) {
                return Err("Utterance sequence numbers must rise.".into());
            }
            if call.utterance_pending {
                return Err("The previous utterance is still being handled.".into());
            }
            let speech = CancellationToken::new();
            // A front speaks for itself; only the person's input is heard
            // and answered by the pipeline's providers.
            if let Some(input_mode) = input_mode {
                call.speech_services = self.resolve_speech(input_mode, call.target.as_deref())?;
            }
            call.work
                .try_send(work(speech.clone()))
                .map_err(|_| "The voice call is busy. Try again shortly.".to_string())?;
            call.input = None;
            call.speech.cancel();
            call.speech = speech;
            call.deliveries.cancel();
            call.deliveries = CancellationToken::new();
            call.stop_streams(false);
            call.utterance_pending = true;
            call.first_clip_started = Some(Instant::now());
            call.seq = Some(seq);
            call.activity = Instant::now();
            call.state(VoiceState::Thinking, None);
            Ok(())
        })
    }

    /// A direct call to `target` speaks in that teammate's own voice when it has one.
    pub(super) fn resolve_speech(
        &self,
        input_mode: VoiceInputMode,
        target: Option<&str>,
    ) -> Result<CallSpeech, String> {
        if let Some(services) = &self.injected {
            return Ok(match input_mode {
                VoiceInputMode::Audio => CallSpeech::audio(services.speech.clone()),
                VoiceInputMode::Text => CallSpeech::output(SpeechOutput {
                    tts: services.speech.tts.clone(),
                    fallback_tts: services.speech.fallback_tts.clone(),
                }),
            });
        }
        let resolve = |settings: &VoiceSettings| match input_mode {
            VoiceInputMode::Audio => speech::resolve(&self.vault, settings).map(CallSpeech::audio),
            VoiceInputMode::Text => {
                speech::resolve_output(&self.vault, settings).map(CallSpeech::output)
            }
        };
        let settings = self.settings();
        let desk = resolve(&settings)?;
        let own = target
            .and_then(|target| {
                crate::room::roster(&self.log)
                    .into_iter()
                    .find(|persona| persona.id == target)
            })
            .and_then(|persona| persona.voice)
            .and_then(|voice| own_voice(&settings, &voice, &desk.tts.id()));
        // A voice the provider no longer takes leaves the desk's in place.
        Ok(own
            .and_then(|settings| resolve(&settings).ok())
            .unwrap_or(desk))
    }

    pub(super) fn speech_for(&self, id: &str) -> Result<CallSpeech, String> {
        self.change(id, |call| {
            // Connection revocation ends the call; disconnected providers may not
            // continue to use a key retained by its current sentence adapters.
            if !self.connected(call.speech_services.tts.as_ref()) {
                return Err("The voice provider is no longer connected.".into());
            }
            Ok(call.speech_services.clone())
        })
    }

    pub(super) fn connected(&self, speech: &dyn speech::Speech) -> bool {
        if self.injected.is_some() {
            return true;
        }
        if speech.id().provider_id == local::PROVIDER_ID {
            return local::chosen(self.vault.root(), Some(&speech.id().model_id)).is_some();
        }
        self.login_connected(&speech.id().provider_id, speech.is_subscription())
    }

    /// Whether the key, or the Grok login, a provider was resolved with is
    /// still in the vault.
    pub(super) fn login_connected(&self, provider_id: &str, subscription: bool) -> bool {
        let auth = self.vault.provider_auth();
        if provider_id == realtime::live::PROVIDER_ID {
            matches!(
                auth.get(provider_id),
                Some(crate::session::ProviderAuth::Login { .. })
            )
        } else if subscription {
            matches!(
                auth.get("xai"),
                Some(crate::session::ProviderAuth::StoredLogin { .. })
            )
        } else {
            auth.contains_key(provider_id)
        }
    }

    pub fn audio(
        &self,
        id: &str,
        seq: u32,
        index: u32,
        data: &str,
        last: bool,
    ) -> Result<(), String> {
        if data.len() > 32_768 * 4 / 3 + 4 {
            return Err("A voice audio chunk is too large.".into());
        }
        let bytes = STANDARD
            .decode(data)
            .map_err(|_| "Voice audio must be standard base64.".to_string())?;
        if bytes.len() > 32_768 || bytes.len() % 2 != 0 || (bytes.is_empty() && !last) {
            return Err("Send at most 32 KiB of PCM16 per chunk.".into());
        }
        self.change(id, |call| {
            if call.state == VoiceState::Held {
                return Err("Resume the call before speaking.".into());
            }
            if call.front.is_some() {
                return Err(FRONT_LINE.into());
            }
            if call.input_mode != VoiceInputMode::Audio
                || !call.stream_audio
                || !call
                    .speech_services
                    .stt
                    .as_ref()
                    .is_some_and(|s| s.supports_live_input())
            {
                return Err("This call does not accept live PCM.".into());
            }
            if index == 0 {
                if bytes.is_empty()
                    || call.utterance_pending
                    || call.seq.is_some_and(|previous| seq <= previous)
                {
                    return Err("Start a new, increasing voice sequence with audio.".into());
                }
                let services =
                    self.resolve_speech(VoiceInputMode::Audio, call.target.as_deref())?;
                let stt = services
                    .stt
                    .as_ref()
                    .filter(|s| s.supports_live_input())
                    .cloned()
                    .ok_or("The voice provider no longer accepts live PCM.")?;
                if !self.connected(stt.as_ref()) {
                    return Err("The voice provider no longer accepts live PCM.".into());
                }
                let work_slot = call
                    .work
                    .try_reserve()
                    .map_err(|_| "The voice call is busy. Try again shortly.".to_string())?;
                self.pay(
                    Kind::Stt,
                    ledger::stt_live_usd(&stt.id().provider_id, bytes.len() as f64 / 32_000.0),
                )?;
                let (sender, input) = mpsc::channel(32);
                sender
                    .try_send(bytes.clone())
                    .map_err(|_| "The voice input is busy.".to_string())?;
                let committed = Arc::new(AtomicU32::new(0));
                let speech = CancellationToken::new();
                work_slot.send(Work::Live {
                    seq,
                    input: Mutex::new(Some(input)),
                    committed: committed.clone(),
                    stt: stt.clone(),
                    speech: speech.clone(),
                });
                call.speech.cancel();
                call.speech = speech;
                call.deliveries.cancel();
                call.deliveries = CancellationToken::new();
                call.stop_streams(false);
                call.speech_services = services;
                call.seq = Some(seq);
                call.utterance_pending = true;
                call.input = Some(Input {
                    seq,
                    index: 1,
                    bytes: bytes.len(),
                    sender,
                    committed,
                    stt,
                });
            } else {
                let input = call
                    .input
                    .as_mut()
                    .ok_or("There is no live voice input to continue.")?;
                if input.seq != seq || input.index != index {
                    return Err("Voice audio chunks must arrive in order.".into());
                }
                if input.bytes + bytes.len() > 640_000 {
                    return Err("A voice turn may last at most 20 seconds.".into());
                }
                if !bytes.is_empty() {
                    let slot = input
                        .sender
                        .try_reserve()
                        .map_err(|_| "The live voice input is busy. Call again.".to_string())?;
                    // No provider work is accepted before its budget reservation.
                    self.pay(
                        Kind::Stt,
                        ledger::stt_live_usd(
                            &input.stt.id().provider_id,
                            bytes.len() as f64 / 32_000.0,
                        ),
                    )?;
                    slot.send(bytes.clone());
                }
                input.bytes += bytes.len();
                input.index += 1;
            }
            call.activity = Instant::now();
            if last {
                let input = call.input.take().expect("accepted input");
                input.committed.store(input.bytes as u32, Ordering::SeqCst);
                drop(input.sender);
                call.first_clip_started = Some(Instant::now());
                call.state(VoiceState::Thinking, None);
            }
            Ok(())
        })
    }
}

pub(super) fn validate_audio(mime: &str, bytes: &[u8], duration: u32) -> Result<u32, String> {
    if mime == "audio/mp4" {
        if bytes.len() < 12 || &bytes[4..8] != b"ftyp" {
            return Err("Send an AAC clip in an MP4 container.".into());
        }
        let actual = mp4_duration(bytes, 0)?.ok_or("The MP4 clip has no duration.")?;
        if actual == 0 || actual > 20_000 || (actual as i64 - duration as i64).abs() > 250 {
            return Err("Send at most 20 seconds of audio with its actual duration.".into());
        }
        return Ok(actual);
    }
    if bytes.len() < 44 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("Send a PCM WAV clip.".into());
    }
    let mut offset = 12;
    let mut format = false;
    let mut saw_format = false;
    let mut samples = None;
    while offset + 8 <= bytes.len() {
        let size = u32::from_le_bytes(
            bytes[offset + 4..offset + 8]
                .try_into()
                .expect("four bytes"),
        ) as usize;
        let end = (offset + 8)
            .checked_add(size)
            .filter(|end| *end <= bytes.len())
            .ok_or("The WAV clip is truncated.")?;
        if &bytes[offset..offset + 4] == b"fmt " {
            if saw_format {
                return Err("The WAV clip repeats its format.".into());
            }
            saw_format = true;
            let f = &bytes[offset + 8..end];
            format = f.len() >= 16
                && f[..2] == 1u16.to_le_bytes()
                && f[2..4] == 1u16.to_le_bytes()
                && f[4..8] == 16_000u32.to_le_bytes()
                && f[14..16] == 16u16.to_le_bytes();
        }
        if &bytes[offset..offset + 4] == b"data" {
            if samples.is_some() {
                return Err("The WAV clip repeats its samples.".into());
            }
            samples = Some(size);
        }
        offset = end + (size % 2);
    }
    let size = samples.ok_or("The WAV clip has no samples.")?;
    if !format
        || size == 0
        || size > 640_000
        || size % 2 != 0
        || (size as i64 * 1000 / 32_000 - duration as i64).abs() > 250
    {
        return Err(
            "Send 16 kHz mono PCM16 WAV, at most 20 seconds, with its actual duration.".into(),
        );
    }
    Ok((size as u32).div_ceil(32))
}

pub(super) fn mp4_duration(bytes: &[u8], depth: u8) -> Result<Option<u32>, String> {
    if depth > 5 {
        return Err("The MP4 clip is too deeply nested.".into());
    }
    let mut offset = 0usize;
    let mut duration = None;
    let malformed = || "The MP4 clip is malformed.".to_string();
    while offset < bytes.len() {
        let header = bytes.get(offset..offset + 8).ok_or_else(malformed)?;
        let mut size = u32::from_be_bytes(header[..4].try_into().expect("four bytes")) as usize;
        let mut header_size = 8;
        if size == 1 {
            size = u64::from_be_bytes(
                bytes
                    .get(offset + 8..offset + 16)
                    .ok_or_else(malformed)?
                    .try_into()
                    .expect("eight bytes"),
            )
            .try_into()
            .map_err(|_| malformed())?;
            header_size = 16;
        } else if size == 0 {
            size = bytes.len() - offset;
        }
        if size < header_size {
            return Err(malformed());
        }
        let end = offset
            .checked_add(size)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(malformed)?;
        let body = &bytes[offset + header_size..end];
        let found = match &header[4..8] {
            b"moov" | b"trak" | b"mdia" => mp4_duration(body, depth + 1)?,
            b"mdhd" => {
                let version = *body.first().ok_or_else(malformed)?;
                let (scale_at, duration_at, width) = match version {
                    0 => (12, 16, 4),
                    1 => (20, 24, 8),
                    _ => return Err(malformed()),
                };
                let scale = u32::from_be_bytes(
                    body.get(scale_at..scale_at + 4)
                        .ok_or_else(malformed)?
                        .try_into()
                        .expect("four bytes"),
                ) as u64;
                let raw = body
                    .get(duration_at..duration_at + width)
                    .ok_or_else(malformed)?;
                let count = if width == 4 {
                    u32::from_be_bytes(raw.try_into().expect("four bytes")) as u64
                } else {
                    u64::from_be_bytes(raw.try_into().expect("eight bytes"))
                };
                if scale == 0 {
                    return Err(malformed());
                }
                Some(
                    count
                        .checked_mul(1000)
                        .ok_or_else(malformed)?
                        .div_ceil(scale)
                        .try_into()
                        .map_err(|_| malformed())?,
                )
            }
            _ => None,
        };
        if let Some(found) = found {
            duration = Some(duration.map_or(found, |old: u32| old.max(found)));
        }
        offset = end;
    }
    Ok(duration)
}
