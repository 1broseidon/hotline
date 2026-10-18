//! A realtime voice model over a WebSocket that speaks OpenAI's Realtime
//! protocol, which xAI's follows too, so one driver carries both:
//!
//! - Grok Voice, `wss://api.x.ai/v1/realtime`, with the stored Grok login or
//!   an API key. https://docs.x.ai/developers/rest-api-reference/inference/voice
//!   and https://docs.x.ai/developers/models/realtime-api (120-minute sessions).
//! - GPT Realtime, `wss://api.openai.com/v1/realtime`, with an API key.
//!   https://developers.openai.com/api/reference/resources/realtime/client-events
//!   (60-minute sessions, PCM at 24 kHz only).
//!
//! Audio travels as base64 JSON events, the default transport. Turns are the
//! provider's server VAD, which stops a reply itself when the person talks.
//! The dialects differ only in the model, the rates, the session limit and
//! where `session.update` puts the voice, turn detection and transcription.

use super::{Command, Realtime, RealtimeConfig, RealtimeEvent, RealtimeInput, RealtimeSession};
use crate::credentials::CredentialFile;
use crate::voice::speech::xai::{Access, Socket};
use crate::voice::speech::{Endpoint, SpeechError, SpeechId};
use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep, sleep_until, timeout};
use tokio_tungstenite::tungstenite::{Error as WsError, Message, error::ProtocolError};

pub const GROK_MODEL: &str = "grok-voice-latest";
pub const OPENAI_MODEL: &str = "gpt-realtime-2.1";
/// The linear PCM rates each provider accepts both ways.
const GROK_RATES: [u32; 7] = [8_000, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000];
const OPENAI_RATES: [u32; 1] = [24_000];
/// xAI ends a session at 120 minutes and OpenAI at 60. A fresh one replaces
/// it between replies five minutes before, or mid-reply four minutes later.
const GROK_RENEW_AFTER: Duration = Duration::from_mins(115);
const OPENAI_RENEW_AFTER: Duration = Duration::from_mins(55);
/// What OpenAI transcribes the person with, for the call's thread; xAI
/// transcribes without being asked.
const OPENAI_TRANSCRIBE: &str = "gpt-4o-mini-transcribe";
const RENEW_GRACE: Duration = Duration::from_mins(4);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);
/// Unreachable providers are retried this many times, a doubling wait apart.
const RECONNECTS: u32 = 3;
const RECONNECT_WAIT: Duration = Duration::from_millis(250);
/// One event, a base64 audio delta at most.
const MESSAGE_LIMIT: usize = 4 << 20;
/// The transcript a renewed session is given, in bytes, newest kept.
const CARRY_LIMIT: usize = 16 << 10;
const CODE_LIMIT: usize = 64;
const COMMAND_QUEUE: usize = 64;
const EVENT_QUEUE: usize = 256;

/// Whose shape of the protocol a socket speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dialect {
    Grok,
    OpenAi,
}

impl Dialect {
    fn model(self) -> &'static str {
        match self {
            Self::Grok => GROK_MODEL,
            Self::OpenAi => OPENAI_MODEL,
        }
    }

    fn rates(self) -> &'static [u32] {
        match self {
            Self::Grok => &GROK_RATES,
            Self::OpenAi => &OPENAI_RATES,
        }
    }
}

#[derive(Clone)]
pub struct RealtimeSocket {
    access: Access,
    dialect: Dialect,
    renew_after: Duration,
}

impl RealtimeSocket {
    /// Grok Voice on an xAI key; [`with_subscription`](Self::with_subscription)
    /// puts it on the Grok login instead.
    pub fn grok(endpoint: Endpoint) -> Self {
        Self {
            access: Access::new(endpoint),
            dialect: Dialect::Grok,
            renew_after: GROK_RENEW_AFTER,
        }
    }

    /// GPT Realtime on an OpenAI key.
    pub fn openai(endpoint: Endpoint) -> Self {
        Self {
            access: Access::new(endpoint),
            dialect: Dialect::OpenAi,
            renew_after: OPENAI_RENEW_AFTER,
        }
    }

    pub fn with_subscription(mut self, tokens: CredentialFile) -> Result<Self, String> {
        if self.dialect != Dialect::Grok {
            return Err("Only Grok Voice runs on the Grok login.".into());
        }
        self.access = self.access.with_subscription(tokens)?;
        Ok(self)
    }

    /// A socket with `config` applied, its instructions extended by the
    /// conversation so far when this continues an earlier session.
    async fn connect(&self, config: &RealtimeConfig, carry: &Carry) -> Result<Socket, SpeechError> {
        let mut url = self.access.socket_url("realtime")?;
        url.query_pairs_mut()
            .append_pair("model", self.dialect.model());
        let mut socket = self.access.socket(url, MESSAGE_LIMIT).await?;
        let update = session_update(
            self.dialect,
            config,
            &carry.instructions(&config.instructions),
        );
        socket
            .send(Message::Text(update.to_string().into()))
            .await
            .map_err(|error| self.access.socket_error(error))?;
        timeout(CONNECT_TIMEOUT, async {
            loop {
                let Read::Event(event) = self.read(&mut socket).await? else {
                    return Err(self.access.unreachable());
                };
                match event["type"].as_str() {
                    Some("session.updated") => return Ok(()),
                    // The provider keeps its previous config on an error; a
                    // session without ours is not one this call can use.
                    Some("error") => return Err(self.access.malformed()),
                    _ => {}
                }
            }
        })
        .await
        .map_err(|_| self.access.unreachable())??;
        Ok(socket)
    }

    async fn read(&self, socket: &mut Socket) -> Result<Read, SpeechError> {
        loop {
            let message = match socket.next().await {
                None => return Ok(Read::Lost),
                // A peer that vanished is a lost connection, not a bad one.
                Some(Err(WsError::Protocol(ProtocolError::ResetWithoutClosingHandshake))) => {
                    return Ok(Read::Lost);
                }
                Some(Err(error)) => {
                    return match self.access.socket_error(error) {
                        error @ SpeechError::Malformed { .. } => Err(error),
                        _ => Ok(Read::Lost),
                    };
                }
                Some(Ok(message)) => message,
            };
            match message {
                Message::Text(text) => {
                    let event: Value =
                        serde_json::from_str(&text).map_err(|_| self.access.malformed())?;
                    if event["type"].as_str().is_none() {
                        return Err(self.access.malformed());
                    }
                    return Ok(Read::Event(event));
                }
                Message::Close(_) => return Ok(Read::Lost),
                // Tungstenite queues the pong; flushing sends it.
                Message::Ping(_) => {
                    if socket.flush().await.is_err() {
                        return Ok(Read::Lost);
                    }
                }
                Message::Pong(_) | Message::Frame(_) => {}
                Message::Binary(_) => return Err(self.access.malformed()),
            }
        }
    }
}

#[async_trait]
impl Realtime for RealtimeSocket {
    fn id(&self) -> SpeechId {
        SpeechId {
            provider_id: self.access.endpoint.provider_id.clone(),
            model_id: self.dialect.model().into(),
            voice: None,
        }
    }

    fn is_subscription(&self) -> bool {
        self.access.is_subscription()
    }

    async fn open(&self, config: RealtimeConfig) -> Result<RealtimeSession, SpeechError> {
        for rate in [config.input_rate, config.output_rate] {
            if !self.dialect.rates().contains(&rate) {
                return Err(SpeechError::UnsupportedFormat(format!("PCM at {rate} Hz")));
            }
        }
        let socket = self.connect(&config, &Carry::default()).await?;
        let (commands, command_queue) = mpsc::channel(COMMAND_QUEUE);
        let (event_queue, events) = mpsc::channel(EVENT_QUEUE);
        let driver = Driver {
            voice: self.clone(),
            config,
            events: event_queue,
            carry: Carry::default(),
            saying: String::new(),
            audio_item: None,
            responding: false,
            respond_after: false,
            quiet_reply: false,
            opened: Instant::now(),
        };
        tokio::spawn(driver.run(socket, command_queue));
        Ok(RealtimeSession {
            input: RealtimeInput { commands },
            events,
        })
    }
}

fn session_update(dialect: Dialect, config: &RealtimeConfig, instructions: &str) -> Value {
    let tools: Vec<Value> = config
        .tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
            })
        })
        .collect();
    let input = json!({"type": "audio/pcm", "rate": config.input_rate});
    let output = json!({"type": "audio/pcm", "rate": config.output_rate});
    let session = match dialect {
        Dialect::Grok => json!({
            "instructions": instructions,
            "voice": config.voice,
            "turn_detection": {"type": "server_vad"},
            "audio": {"input": {"format": input}, "output": {"format": output}},
            "tools": tools,
        }),
        Dialect::OpenAi => json!({
            "type": "realtime",
            "instructions": instructions,
            "audio": {
                "input": {
                    "format": input,
                    "turn_detection": {"type": "server_vad"},
                    "transcription": {"model": OPENAI_TRANSCRIBE},
                },
                "output": {"format": output, "voice": config.voice},
            },
            "tools": tools,
        }),
    };
    json!({"type": "session.update", "session": session})
}

/// A provider's error code, when it is a short identifier. Its message can
/// echo what was said, so it never leaves the adapter.
/// The provider's code for a `response.create` sent while a reply is in progress.
const ACTIVE_RESPONSE: &str = "conversation_already_has_active_response";

fn error_code(event: &Value) -> Option<String> {
    let code = event["error"]["code"].as_str()?;
    let identifier = !code.is_empty()
        && code.len() <= CODE_LIMIT
        && code
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'));
    identifier.then(|| code.into())
}

enum Read {
    Event(Value),
    /// The connection ended without a protocol fault.
    Lost,
}

/// The conversation's transcript, newest kept, for a renewed session.
#[derive(Clone, Default)]
pub(super) struct Carry {
    lines: VecDeque<String>,
    bytes: usize,
}

impl Carry {
    pub(super) fn push(&mut self, speaker: &str, words: &str) {
        let words = words.trim();
        if words.is_empty() {
            return;
        }
        let line = format!("{speaker}: {words}");
        self.bytes += line.len();
        self.lines.push_back(line);
        while self.bytes > CARRY_LIMIT {
            let Some(oldest) = self.lines.pop_front() else {
                break;
            };
            self.bytes -= oldest.len();
        }
    }

    pub(super) fn instructions(&self, base: &str) -> String {
        if self.lines.is_empty() {
            return base.into();
        }
        let lines: Vec<&str> = self.lines.iter().map(String::as_str).collect();
        format!(
            "{base}\n\nThis call was already under way when this session began. \
             The conversation so far, as transcribed:\n{}",
            lines.join("\n")
        )
    }
}

enum Next {
    Command(Option<Command>),
    Read(Read),
    Renew,
}

/// Owns one call's provider connection, replacing it as the session limit
/// and the network require.
struct Driver {
    voice: RealtimeSocket,
    config: RealtimeConfig,
    events: mpsc::Sender<Result<RealtimeEvent, SpeechError>>,
    carry: Carry,
    /// The current reply's transcript so far.
    saying: String,
    /// The reply item whose audio is playing, for a barge-in's truncation.
    audio_item: Option<String>,
    responding: bool,
    /// A tool result or text arrived mid-reply; ask for the next reply after it.
    respond_after: bool,
    /// The next reply only says a line it was given, so it may not call a
    /// tool (OpenAI only).
    quiet_reply: bool,
    opened: Instant,
}

impl Driver {
    async fn run(mut self, mut socket: Socket, mut commands: mpsc::Receiver<Command>) {
        match self.drive(&mut socket, &mut commands).await {
            Ok(()) | Err(SpeechError::Cancelled) => {}
            Err(error) => {
                let _ = self.events.send(Err(error)).await;
            }
        }
        let _ = timeout(CLOSE_TIMEOUT, socket.close(None)).await;
    }

    async fn drive(
        &mut self,
        socket: &mut Socket,
        commands: &mut mpsc::Receiver<Command>,
    ) -> Result<(), SpeechError> {
        loop {
            let mut renew_at = self.opened + self.voice.renew_after;
            if self.responding {
                renew_at += RENEW_GRACE;
            }
            let next = tokio::select! {
                command = commands.recv() => Next::Command(command),
                read = self.voice.read(socket) => Next::Read(read?),
                () = sleep_until(renew_at) => Next::Renew,
                () = self.events.closed() => return Err(SpeechError::Cancelled),
            };
            match next {
                Next::Command(None) => return Ok(()),
                Next::Command(Some(command)) => self.command(socket, command).await?,
                Next::Read(Read::Event(event)) => self.event(socket, event).await?,
                Next::Read(Read::Lost) => self.reconnect(socket).await?,
                Next::Renew => {
                    let fresh = self.voice.connect(&self.config, &self.carry).await?;
                    self.replace(socket, fresh).await?;
                }
            }
        }
    }

    async fn emit(&self, event: RealtimeEvent) -> Result<(), SpeechError> {
        self.events
            .send(Ok(event))
            .await
            .map_err(|_| SpeechError::Cancelled)
    }

    async fn command(&mut self, socket: &mut Socket, command: Command) -> Result<(), SpeechError> {
        match command {
            Command::Audio(pcm) => {
                let append = json!({
                    "type": "input_audio_buffer.append",
                    "audio": STANDARD.encode(pcm),
                });
                self.send(socket, append).await
            }
            Command::ToolResult {
                call_id,
                output,
                reply,
            } => {
                let item = json!({
                    "type": "conversation.item.create",
                    "item": {"type": "function_call_output", "call_id": call_id, "output": output},
                });
                self.send(socket, item).await?;
                if !reply {
                    return Ok(());
                }
                self.quiet_reply = false;
                self.respond(socket).await
            }
            Command::Say(line) => {
                let text = format!("{}{line}", super::SAY_THIS);
                Box::pin(self.command(socket, Command::Text(text))).await
            }
            Command::Text(text) => {
                // OpenAI's model takes a user item for the person's words and
                // relays it, so a line to say goes in as a system item, and
                // its reply may not call a tool. xAI's says it as given.
                let role = match self.voice.dialect {
                    Dialect::Grok => "user",
                    Dialect::OpenAi => "system",
                };
                let item = json!({
                    "type": "conversation.item.create",
                    "item": {
                        "type": "message",
                        "role": role,
                        "content": [{"type": "input_text", "text": text}],
                    },
                });
                self.send(socket, item).await?;
                self.quiet_reply = self.voice.dialect == Dialect::OpenAi
                    && (self.quiet_reply || !self.respond_after);
                self.respond(socket).await
            }
            Command::Truncate { played_ms } => {
                let Some(item_id) = self.audio_item.take() else {
                    return Ok(());
                };
                let truncate = json!({
                    "type": "conversation.item.truncate",
                    "item_id": item_id,
                    "content_index": 0,
                    "audio_end_ms": played_ms,
                });
                self.send(socket, truncate).await
            }
        }
    }

    /// The provider refuses a second reply while one is in progress, so a
    /// result that arrives mid-reply waits for that reply's end.
    async fn respond(&mut self, socket: &mut Socket) -> Result<(), SpeechError> {
        if self.responding {
            self.respond_after = true;
            return Ok(());
        }
        self.responding = true;
        let create = if std::mem::take(&mut self.quiet_reply) {
            json!({"type": "response.create", "response": {"tool_choice": "none"}})
        } else {
            json!({"type": "response.create"})
        };
        self.send(socket, create).await
    }

    async fn event(&mut self, socket: &mut Socket, event: Value) -> Result<(), SpeechError> {
        match event["type"].as_str().unwrap_or_default() {
            "response.output_audio.delta" | "response.audio.delta" => {
                let delta = event["delta"].as_str().ok_or_else(|| self.malformed())?;
                let pcm = STANDARD.decode(delta).map_err(|_| self.malformed())?;
                if !pcm.len().is_multiple_of(2) {
                    return Err(self.malformed());
                }
                if let Some(item_id) = event["item_id"].as_str() {
                    self.audio_item = Some(item_id.into());
                }
                if !pcm.is_empty() {
                    self.emit(RealtimeEvent::Audio(pcm)).await?;
                }
            }
            "response.output_audio_transcript.delta" => {
                let delta = event["delta"].as_str().ok_or_else(|| self.malformed())?;
                if self.saying.len() + delta.len() <= CARRY_LIMIT {
                    self.saying.push_str(delta);
                }
                self.emit(RealtimeEvent::Saying(delta.into())).await?;
            }
            "response.output_audio_transcript.done" => self.said(),
            "input_audio_buffer.speech_started" => {
                self.emit(RealtimeEvent::SpeechStarted).await?;
            }
            "conversation.item.input_audio_transcription.completed" => {
                let words = event["transcript"]
                    .as_str()
                    .ok_or_else(|| self.malformed())?
                    .trim();
                if !words.is_empty() {
                    self.carry.push("Caller", words);
                    self.emit(RealtimeEvent::Heard(words.into())).await?;
                }
            }
            "response.function_call_arguments.done" => {
                let field = |name: &str| event[name].as_str().map(str::to_string);
                let (Some(call_id), Some(name), Some(arguments)) =
                    (field("call_id"), field("name"), field("arguments"))
                else {
                    return Err(self.malformed());
                };
                self.emit(RealtimeEvent::ToolCall {
                    call_id,
                    name,
                    arguments,
                })
                .await?;
            }
            "response.created" => self.responding = true,
            "response.done" => {
                self.said();
                self.responding = false;
                self.emit(RealtimeEvent::ResponseDone).await?;
                if std::mem::take(&mut self.respond_after) {
                    self.respond(socket).await?;
                }
            }
            "error" => {
                // A refused `response.create` ends in no `response.done`, so
                // the flag it set comes down here and a reply queued behind
                // it goes now. Refused because a reply is already in
                // progress, the flag is right, and the queued reply waits
                // for that reply's end.
                let code = error_code(&event);
                let in_progress = code.as_deref() == Some(ACTIVE_RESPONSE);
                if in_progress {
                    self.responding = true;
                    self.respond_after = true;
                } else {
                    self.responding = false;
                }
                self.emit(RealtimeEvent::Refused { code }).await?;
                if !in_progress && std::mem::take(&mut self.respond_after) {
                    self.respond(socket).await?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn malformed(&self) -> SpeechError {
        self.voice.access.malformed()
    }

    /// The model's finished words join the transcript a renewal carries.
    fn said(&mut self) {
        let words = std::mem::take(&mut self.saying);
        self.carry.push("You", &words);
    }

    /// A send that fails reconnects once and is sent again; the session it
    /// was meant for is gone, but the call is not.
    async fn send(&mut self, socket: &mut Socket, event: Value) -> Result<(), SpeechError> {
        let text = event.to_string();
        if socket
            .send(Message::Text(text.clone().into()))
            .await
            .is_ok()
        {
            return Ok(());
        }
        self.reconnect(socket).await?;
        socket
            .send(Message::Text(text.into()))
            .await
            .map_err(|error| self.voice.access.socket_error(error))
    }

    /// Only an unreachable provider is retried: a refused login, a refused
    /// entitlement or a rejected config will not change by asking again.
    async fn reconnect(&mut self, socket: &mut Socket) -> Result<(), SpeechError> {
        let mut wait = RECONNECT_WAIT;
        let mut attempt = 0;
        loop {
            match self.voice.connect(&self.config, &self.carry).await {
                Ok(fresh) => return self.replace(socket, fresh).await,
                Err(error @ SpeechError::Unreachable { .. }) => {
                    attempt += 1;
                    if attempt > RECONNECTS {
                        return Err(error);
                    }
                }
                Err(error) => return Err(error),
            }
            sleep(wait).await;
            wait *= 2;
        }
    }

    async fn replace(&mut self, socket: &mut Socket, fresh: Socket) -> Result<(), SpeechError> {
        let mut old = std::mem::replace(socket, fresh);
        let _ = timeout(CLOSE_TIMEOUT, old.close(None)).await;
        self.said();
        self.audio_item = None;
        self.responding = false;
        self.respond_after = false;
        self.quiet_reply = false;
        self.opened = Instant::now();
        self.emit(RealtimeEvent::Renewed).await
    }
}

#[cfg(test)]
mod tests;
