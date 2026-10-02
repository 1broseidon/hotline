//! Native xAI speech: live PCM through `/v1/stt`, and streamed PCM from
//! `/v1/tts`. Network chunks are not playable clips: we retain the final
//! half-second and wrap ordered PCM batches in complete WAV containers.
//! Protocol: https://docs.x.ai/developers/model-capabilities/audio/speech-to-text
//! and https://docs.x.ai/developers/model-capabilities/audio/text-to-speech.

use super::{
    Clip, Endpoint, Reply, Speech, SpeechChunk, SpeechError, SpeechId, Timing, TurnClock,
    base_mime, call, http_client, wav,
};
use crate::credentials::CredentialFile;
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use reqwest::multipart::{Form, Part};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::time::{Instant, sleep_until, timeout};
use tokio_tungstenite::tungstenite::{
    Error as WsError, Message, client::IntoClientRequest, protocol::WebSocketConfig,
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async_with_config};

pub(super) const PROVIDER_ID: &str = "xai";
pub(super) const SUBSCRIPTION_PROVIDER_ID: &str = "xai-subscription";
pub(super) const BASE_URL: &str = "https://api.x.ai/v1";
pub(super) const LISTEN_MODEL: &str = "grok-voice-transcribe-2.0";
// The native TTS API selects a voice, not a model. This names its speech job
// consistently with the Grok voice already offered through OpenRouter.
pub(super) const SPEAK_MODEL: &str = "grok-voice-tts-1.0";
const INPUT_RATE: u32 = 16_000;
const OUTPUT_RATE: u32 = 24_000;
const INPUT_FRAME_LIMIT: usize = 32 << 10;
const INPUT_LIMIT: usize = INPUT_RATE as usize * 2 * 20;
const PCM_FRAME_BYTES: usize = INPUT_RATE as usize * 2 / 10;
const EVENT_LIMIT: usize = 4096;
const TRANSCRIPT_LIMIT: usize = 1 << 20;
const AUDIO_LIMIT: usize = 20 << 20;
const TEXT_LIMIT: usize = 15_000;
const CLIP_PCM_BYTES: usize = OUTPUT_RATE as usize; // Half a second of mono PCM16.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const FINAL_TIMEOUT: Duration = Duration::from_secs(15);
const LIVE_TIMEOUT: Duration = Duration::from_secs(50);

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

enum Job {
    Listen { model: String },
    Speak { model: String, voice: String },
}

#[derive(Clone, Copy)]
struct AudioRange {
    start: f64,
    end: f64,
}

impl AudioRange {
    fn from_event(event: &Value) -> Option<Self> {
        let start = event.get("start")?.as_f64()?;
        let duration = event.get("duration")?.as_f64()?;
        let end = start + duration;
        (start.is_finite() && start >= 0.0 && duration > 0.0 && end.is_finite())
            .then_some(Self { start, end })
    }

    fn covers(self, other: Self) -> bool {
        // Decimal timestamps may differ by floating-point rounding only.
        self.start <= other.start + 0.000_001 && self.end + 0.000_001 >= other.end
    }
}

struct FinalSegment {
    text: String,
    range: Option<AudioRange>,
}

/// Chunk finals are deltas. A speech-final partial replaces the current
/// chunks with its stitched utterance; done may contain only the remaining
/// transcript. Never recover text from an interim-only event.
#[derive(Default)]
struct FinalTranscript {
    utterances: Vec<FinalSegment>,
    chunks: Vec<FinalSegment>,
}

impl FinalTranscript {
    fn accept(&mut self, event: &Value) -> Result<(), ()> {
        let speech_final = event.get("speech_final").and_then(Value::as_bool) == Some(true);
        if !speech_final && event.get("is_final").and_then(Value::as_bool) != Some(true) {
            return Ok(());
        }
        let text = event.get("text").and_then(Value::as_str).ok_or(())?.trim();
        if text.is_empty() {
            return Ok(());
        }
        let range = AudioRange::from_event(event);
        if let Some(range) = range {
            if self
                .utterances
                .iter()
                .any(|segment| segment.range.is_some_and(|old| old.covers(range)))
            {
                return Ok(());
            }
            if !speech_final
                && self.chunks.iter().any(|segment| {
                    segment
                        .range
                        .is_some_and(|old| old.covers(range) && range.covers(old))
                })
            {
                return Ok(());
            }
        }
        let segment = FinalSegment {
            text: text.into(),
            range,
        };
        if speech_final {
            match range {
                Some(range) => self
                    .chunks
                    .retain(|segment| segment.range.is_some_and(|old| !range.covers(old))),
                None => self.chunks.clear(),
            }
            // A replay or stitched final can cover previously emitted finals.
            // Audio ranges identify those; identical words alone do not,
            // because a caller may intentionally repeat them.
            let at = range
                .and_then(|range| {
                    self.utterances
                        .iter()
                        .position(|segment| segment.range.is_some_and(|old| range.covers(old)))
                })
                .unwrap_or(self.utterances.len());
            if let Some(range) = range {
                self.utterances
                    .retain(|segment| !segment.range.is_some_and(|old| range.covers(old)));
            }
            self.utterances.insert(at, segment);
        } else {
            self.chunks.push(segment);
        }
        let segments = self.utterances.len() + self.chunks.len();
        let bytes = self
            .utterances
            .iter()
            .chain(&self.chunks)
            .map(|segment| segment.text.len())
            .sum::<usize>()
            + segments.saturating_sub(1);
        if bytes > TRANSCRIPT_LIMIT {
            Err(())
        } else {
            Ok(())
        }
    }

    fn finish(&self, event: &Value) -> Result<String, ()> {
        // Grok Build treats an omitted done.text as an empty trailing result.
        match event.get("text") {
            Some(Value::String(text)) if !text.trim().is_empty() => Ok(text.trim().into()),
            Some(Value::String(_)) | None => Ok(self.words()),
            _ => Err(()),
        }
    }

    fn words(&self) -> String {
        self.utterances
            .iter()
            .chain(&self.chunks)
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

pub(super) struct Xai {
    endpoint: Endpoint,
    job: Job,
    http: reqwest::Client,
    clock: TurnClock,
    subscription: Option<CredentialFile>,
}

impl Xai {
    pub(super) fn listener(endpoint: Endpoint, model: &str) -> Result<Self, String> {
        Ok(Self {
            endpoint,
            job: Job::Listen {
                model: model.into(),
            },
            http: http_client()?,
            clock: TurnClock::default(),
            subscription: None,
        })
    }

    pub(super) fn speaker(endpoint: Endpoint, model: &str, voice: &str) -> Result<Self, String> {
        Ok(Self {
            endpoint,
            job: Job::Speak {
                model: model.into(),
                voice: voice.into(),
            },
            http: http_client()?,
            clock: TurnClock::default(),
            subscription: None,
        })
    }

    pub(super) fn with_clock(mut self, clock: &TurnClock) -> Self {
        self.clock = clock.clone();
        self
    }

    /// The subscription bearer is resolved afresh for every HTTP request or
    /// WebSocket connection. It can only be sent to xAI's fixed voice origin.
    pub(super) fn with_subscription(mut self, tokens: CredentialFile) -> Result<Self, String> {
        if self.endpoint.base_url != BASE_URL {
            return Err("Grok subscription voice requires the xAI voice service.".into());
        }
        self.endpoint.provider_id = SUBSCRIPTION_PROVIDER_ID.into();
        self.endpoint.key = None;
        self.subscription = Some(tokens);
        Ok(self)
    }

    fn status_error(&self, status: u16) -> SpeechError {
        let provider_id = self.endpoint.provider_id.clone();
        if self.subscription.is_some() {
            match status {
                401 => return SpeechError::SignInRequired { provider_id },
                402 | 403 | 429 => {
                    return SpeechError::Entitlement {
                        provider_id,
                        status,
                    };
                }
                _ => {}
            }
        }
        SpeechError::Refused {
            provider_id,
            status,
        }
    }

    async fn bearer(&self, rejected: Option<&str>) -> Result<Option<String>, SpeechError> {
        let Some(tokens) = &self.subscription else {
            return Ok(self.endpoint.key.clone());
        };
        timeout(
            CONNECT_TIMEOUT,
            crate::providers::xai::bearer(tokens, rejected),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .map(Some)
        .ok_or_else(|| SpeechError::SignInRequired {
            provider_id: self.endpoint.provider_id.clone(),
        })
    }

    fn authorized(
        request: reqwest::RequestBuilder,
        bearer: Option<&str>,
    ) -> reqwest::RequestBuilder {
        match bearer {
            Some(bearer) => request.bearer_auth(bearer),
            None => request,
        }
    }

    async fn speech_response(&self, text: &str) -> Result<reqwest::Response, SpeechError> {
        // Validation precedes credential access. A failed HTTP authorization
        // may refresh once, before any generated audio is consumed. The
        // multipart STT path never replays already transmitted caller audio.
        let request = self.speech_request(text)?;
        let bearer = self.bearer(None).await?;
        let response = Self::authorized(request, bearer.as_deref())
            .send()
            .await
            .map_err(|_| self.unreachable())?;
        let response = if response.status() == reqwest::StatusCode::UNAUTHORIZED
            && self.subscription.is_some()
        {
            drop(response);
            let bearer = self.bearer(bearer.as_deref()).await?;
            Self::authorized(self.speech_request(text)?, bearer.as_deref())
                .send()
                .await
                .map_err(|_| self.unreachable())?
        } else {
            response
        };
        if !response.status().is_success() {
            return Err(self.status_error(response.status().as_u16()));
        }
        Ok(response)
    }

    async fn speech_reply(&self, text: &str) -> Result<Reply, SpeechError> {
        let mut timing = Timing::start();
        let mut response = self.speech_response(text).await?;
        timing.headers_arrived();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|header| header.to_str().ok())
            .map(str::to_string);
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| self.unreachable())? {
            timing.first_byte_arrived();
            if bytes.len() + chunk.len() > AUDIO_LIMIT {
                return Err(self.malformed());
            }
            bytes.extend_from_slice(&chunk);
        }
        eprintln!("{}", timing.line("speak", &self.id()));
        Ok(Reply {
            bytes,
            content_type,
        })
    }

    fn malformed(&self) -> SpeechError {
        SpeechError::Malformed {
            provider_id: self.endpoint.provider_id.clone(),
        }
    }

    fn unreachable(&self) -> SpeechError {
        SpeechError::Unreachable {
            provider_id: self.endpoint.provider_id.clone(),
        }
    }

    fn socket_error(&self, error: WsError) -> SpeechError {
        // Neither provider bodies nor transport errors may expose words or keys.
        match error {
            WsError::Http(response) => self.status_error(response.status().as_u16()),
            WsError::Capacity(_) | WsError::Protocol(_) | WsError::Utf8(_) => self.malformed(),
            _ => self.unreachable(),
        }
    }

    fn post(&self, route: &str) -> reqwest::RequestBuilder {
        self.http.post(format!(
            "{}/{route}",
            self.endpoint.base_url.trim_end_matches('/')
        ))
    }

    async fn hear(&self, clip: Clip) -> Result<String, SpeechError> {
        let Job::Listen { model } = &self.job else {
            return Err(SpeechError::WrongJob);
        };
        let mime = base_mime(&clip.mime);
        let name = match mime.as_str() {
            "audio/wav" => "clip.wav",
            "audio/mp4" => "clip.m4a",
            _ => return Err(SpeechError::UnsupportedFormat(clip.mime)),
        };
        let part = Part::bytes(clip.bytes)
            .file_name(name)
            .mime_str(&mime)
            .map_err(|_| self.malformed())?;
        let form = Form::new().text("model", model.clone()).part("file", part);
        let bearer = self.bearer(None).await?;
        let reply = call(
            Self::authorized(self.post("stt").multipart(form), bearer.as_deref()),
            &self.id(),
            "transcribe",
            TRANSCRIPT_LIMIT,
        )
        .await
        .map_err(|error| match error {
            SpeechError::Refused { status, .. } => self.status_error(status),
            other => other,
        })?;
        let body: Value = serde_json::from_slice(&reply.bytes).map_err(|_| self.malformed())?;
        self.words(&body)
    }

    fn words(&self, body: &Value) -> Result<String, SpeechError> {
        body.get("text")
            .and_then(Value::as_str)
            .map(|text| text.trim().to_string())
            .ok_or_else(|| self.malformed())
    }

    async fn connect(&self, model: &str) -> Result<Socket, SpeechError> {
        let bearer = self.bearer(None).await?;
        match self.connect_once(model, bearer.as_deref()).await {
            Err(SpeechError::SignInRequired { .. }) if self.subscription.is_some() => {
                let bearer = self.bearer(bearer.as_deref()).await?;
                self.connect_once(model, bearer.as_deref()).await
            }
            result => result,
        }
    }

    async fn connect_once(&self, model: &str, bearer: Option<&str>) -> Result<Socket, SpeechError> {
        crate::desk::install_crypto_provider();
        let mut url = url::Url::parse(&format!(
            "{}/stt",
            self.endpoint.base_url.trim_end_matches('/')
        ))
        .map_err(|_| self.malformed())?;
        let scheme = match url.scheme() {
            "https" => "wss",
            "http" => "ws",
            _ => return Err(self.malformed()),
        };
        url.set_scheme(scheme).map_err(|_| self.malformed())?;
        url.query_pairs_mut()
            .append_pair("sample_rate", "16000")
            .append_pair("encoding", "pcm")
            .append_pair("channels", "1")
            .append_pair("model", model);
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|_| self.malformed())?;
        if let Some(key) = bearer {
            let mut header = http::HeaderValue::from_str(&format!("Bearer {key}"))
                .map_err(|_| self.malformed())?;
            header.set_sensitive(true);
            request
                .headers_mut()
                .insert(http::header::AUTHORIZATION, header);
        }
        let config = WebSocketConfig::default()
            .max_message_size(Some(TRANSCRIPT_LIMIT))
            .max_frame_size(Some(TRANSCRIPT_LIMIT))
            .write_buffer_size(8 << 10)
            .max_write_buffer_size(128 << 10);
        let (socket, _) = timeout(
            CONNECT_TIMEOUT,
            connect_async_with_config(request, Some(config), true),
        )
        .await
        .map_err(|_| self.unreachable())?
        .map_err(|error| self.socket_error(error))?;
        Ok(socket)
    }

    async fn event(&self, socket: &mut Socket, events: &mut usize) -> Result<Value, SpeechError> {
        loop {
            let message = socket
                .next()
                .await
                .ok_or_else(|| self.malformed())?
                .map_err(|error| self.socket_error(error))?;
            *events += 1;
            if *events > EVENT_LIMIT {
                return Err(self.malformed());
            }
            match message {
                Message::Text(text) => {
                    let event: Value = serde_json::from_str(&text).map_err(|_| self.malformed())?;
                    if event.get("type").and_then(Value::as_str).is_none() {
                        return Err(self.malformed());
                    }
                    return Ok(event);
                }
                Message::Ping(_) => socket
                    .flush()
                    .await
                    .map_err(|error| self.socket_error(error))?,
                Message::Pong(_) => (),
                _ => return Err(self.malformed()),
            }
        }
    }

    async fn hear_live(&self, mut input: Receiver<Vec<u8>>) -> Result<String, SpeechError> {
        let Job::Listen { model } = &self.job else {
            return Err(SpeechError::WrongJob);
        };
        let mut timing = Timing::start();
        let mut socket = self.connect(model).await?;
        timing.headers_arrived();
        let mut events = 0;
        let ready = timeout(CONNECT_TIMEOUT, self.event(&mut socket, &mut events))
            .await
            .map_err(|_| self.unreachable())??;
        timing.first_byte_arrived();
        if ready["type"] != "transcript.created" {
            return Err(self.malformed());
        }
        let mut bytes = 0;
        let mut frames = 0;
        let mut finalized = FinalTranscript::default();
        let mut next_send = Instant::now();
        loop {
            tokio::select! {
                chunk = input.recv() => {
                    let Some(chunk) = chunk else { break; };
                    self.accept_frame(&chunk, &mut bytes, &mut frames)?;
                    for frame in chunk.chunks(PCM_FRAME_BYTES) {
                        // A buffered pre-roll must not become one giant provider
                        // frame or a burst of unpaced audio after the handshake.
                        next_send = next_send.max(Instant::now());
                        sleep_until(next_send).await;
                        socket.send(Message::Binary(frame.to_vec().into())).await.map_err(|error| self.socket_error(error))?;
                        next_send += Duration::from_secs_f64(frame.len() as f64 / (INPUT_RATE * 2) as f64);
                    }
                }
                event = self.event(&mut socket, &mut events) => {
                    let event = event?;
                    match event["type"].as_str() {
                        Some("transcript.partial") => finalized.accept(&event).map_err(|_| self.malformed())?,
                        _ => return Err(self.malformed()),
                    }
                }
            }
        }
        socket
            .send(Message::Text(
                json!({"type": "audio.done"}).to_string().into(),
            ))
            .await
            .map_err(|error| self.socket_error(error))?;
        let words = timeout(FINAL_TIMEOUT, async {
            loop {
                let event = self.event(&mut socket, &mut events).await?;
                match event["type"].as_str() {
                    Some("transcript.partial") => {
                        finalized.accept(&event).map_err(|_| self.malformed())?
                    }
                    Some("transcript.done") => {
                        return finalized.finish(&event).map_err(|_| self.malformed());
                    }
                    _ => return Err(self.malformed()),
                }
            }
        })
        .await
        .map_err(|_| self.unreachable())?;
        eprintln!("{}", timing.line("transcribe live", &self.id()));
        words
    }

    fn accept_frame(
        &self,
        frame: &[u8],
        bytes: &mut usize,
        frames: &mut usize,
    ) -> Result<(), SpeechError> {
        if frame.is_empty()
            || !frame.len().is_multiple_of(2)
            || frame.len() > INPUT_FRAME_LIMIT
            || *bytes > INPUT_LIMIT.saturating_sub(frame.len())
            || *frames >= EVENT_LIMIT
        {
            return Err(self.malformed());
        }
        *bytes += frame.len();
        *frames += 1;
        Ok(())
    }

    fn speech_request(&self, text: &str) -> Result<reqwest::RequestBuilder, SpeechError> {
        let Job::Speak { voice, .. } = &self.job else {
            return Err(SpeechError::WrongJob);
        };
        if text.trim().is_empty() {
            return Err(SpeechError::NothingToSay);
        }
        if text.chars().count() > TEXT_LIMIT {
            return Err(self.malformed());
        }
        Ok(self.post("tts").json(&json!({
            "text": text,
            "voice_id": voice,
            "language": "auto",
            "output_format": {"codec": "pcm", "sample_rate": OUTPUT_RATE},
        })))
    }

    fn is_pcm(&self, content_type: Option<&str>) -> Result<(), SpeechError> {
        if content_type.map(base_mime).as_deref() == Some("audio/pcm") {
            Ok(())
        } else {
            Err(self.malformed())
        }
    }

    async fn stream_speech(
        &self,
        text: &str,
        output: &Sender<SpeechChunk>,
    ) -> Result<(), SpeechError> {
        let mut timing = Timing::start();
        let mut response = self.speech_response(text).await?;
        timing.headers_arrived();
        self.is_pcm(
            response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|header| header.to_str().ok()),
        )?;
        let mut pcm = Vec::new();
        let mut received = 0;
        while let Some(chunk) = response.chunk().await.map_err(|_| self.unreachable())? {
            if chunk.is_empty() {
                continue;
            }
            timing.first_byte_arrived();
            if received + chunk.len() > AUDIO_LIMIT {
                return Err(self.malformed());
            }
            received += chunk.len();
            for part in chunk.chunks(CLIP_PCM_BYTES) {
                pcm.extend_from_slice(part);
                // Keep one half-second back so the final clip includes the
                // tail, rather than making a tiny network-sized audio file.
                // Processing a large network chunk in batches also keeps
                // the PCM staging buffer under three half-seconds.
                while pcm.len() >= CLIP_PCM_BYTES * 2 {
                    let bytes: Vec<u8> = pcm.drain(..CLIP_PCM_BYTES).collect();
                    self.clock.first_clip(&self.id());
                    output
                        .send(SpeechChunk {
                            clip: Clip {
                                mime: "audio/wav".into(),
                                bytes: wav::pcm16_wav(&bytes, OUTPUT_RATE),
                            },
                            final_chunk: false,
                        })
                        .await
                        .map_err(|_| SpeechError::Cancelled)?;
                }
            }
        }
        if pcm.is_empty() || pcm.len() % 2 != 0 {
            return Err(self.malformed());
        }
        self.clock.first_clip(&self.id());
        output
            .send(SpeechChunk {
                clip: Clip {
                    mime: "audio/wav".into(),
                    bytes: wav::pcm16_wav(&pcm, OUTPUT_RATE),
                },
                final_chunk: true,
            })
            .await
            .map_err(|_| SpeechError::Cancelled)?;
        eprintln!("{}", timing.line("speak stream", &self.id()));
        Ok(())
    }
}

#[cfg(test)]
mod tests;

#[async_trait]
impl Speech for Xai {
    fn is_subscription(&self) -> bool {
        self.subscription.is_some()
    }
    fn id(&self) -> SpeechId {
        let (model_id, voice) = match &self.job {
            Job::Listen { model } => (model.clone(), None),
            Job::Speak { model, voice } => (model.clone(), Some(voice.clone())),
        };
        SpeechId {
            provider_id: self.endpoint.provider_id.clone(),
            model_id,
            voice,
        }
    }

    fn supports_live_input(&self) -> bool {
        matches!(self.job, Job::Listen { .. })
    }

    async fn transcribe(&self, clip: Clip) -> Result<String, SpeechError> {
        self.clock.heard();
        let words = self.hear(clip).await;
        self.clock.unanswered(&words);
        words
    }

    async fn transcribe_live(
        &self,
        input: Receiver<Vec<u8>>,
        sample_rate: u32,
    ) -> Result<String, SpeechError> {
        if !self.supports_live_input() {
            return Err(SpeechError::WrongJob);
        }
        if sample_rate != INPUT_RATE {
            return Err(SpeechError::UnsupportedFormat(
                "live PCM outside 16000 Hz".into(),
            ));
        }
        self.clock.heard();
        let words = timeout(LIVE_TIMEOUT, self.hear_live(input))
            .await
            .unwrap_or_else(|_| Err(self.unreachable()));
        self.clock.unanswered(&words);
        words
    }

    async fn speak(&self, text: &str) -> Result<Clip, SpeechError> {
        let reply = self.speech_reply(text).await?;
        self.is_pcm(reply.content_type.as_deref())?;
        if reply.bytes.is_empty() || reply.bytes.len() % 2 != 0 {
            return Err(self.malformed());
        }
        let clip = Clip {
            mime: "audio/wav".into(),
            bytes: wav::pcm16_wav(&reply.bytes, OUTPUT_RATE),
        };
        self.clock.first_clip(&self.id());
        Ok(clip)
    }

    async fn speak_chunks(
        &self,
        text: &str,
        output: Sender<SpeechChunk>,
    ) -> Result<(), SpeechError> {
        if output.is_closed() {
            return Err(SpeechError::Cancelled);
        }
        tokio::select! {
            _ = output.closed() => Err(SpeechError::Cancelled),
            result = self.stream_speech(text, &output) => result,
        }
    }
}
