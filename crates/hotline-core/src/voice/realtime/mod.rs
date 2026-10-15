//! The realtime seam: one live spoken conversation with a speech-to-speech
//! model, beside the turn-by-turn [`Speech`](super::speech::Speech) seam.
//!
//! The caller pushes the person's microphone PCM and hears the model's PCM
//! back, with what each side said, the tools the model asks for, and the
//! moment the person starts talking over it. Opening a session spawns one
//! driver task that owns the provider connection: it renews the session
//! before the provider's time limit and reconnects a dropped one, carrying the
//! conversation's transcript across, so the caller sees one session for the
//! whole call. Dropping every [`RealtimeInput`] closes it.

pub mod xai;

pub use xai::GrokVoice;

use super::speech::{SpeechError, SpeechId};
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;

/// The largest microphone frame one push may carry: two seconds of 16 kHz.
const FRAME_LIMIT: usize = 64 << 10;
/// The longest text item, in characters.
const TEXT_LIMIT: usize = 15_000;

/// One function the model may call. `parameters` is its JSON Schema.
#[derive(Clone, Debug, PartialEq)]
pub struct RealtimeTool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// How the model behaves for one call. Audio both ways is mono PCM16
/// little-endian, at the rates the caller's microphone and speaker use.
#[derive(Clone, Debug, PartialEq)]
pub struct RealtimeConfig {
    pub instructions: String,
    pub voice: String,
    pub tools: Vec<RealtimeTool>,
    pub input_rate: u32,
    pub output_rate: u32,
}

/// What the conversation produced, in the order the provider sent it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RealtimeEvent {
    /// Model speech as mono PCM16 at the output rate.
    Audio(Vec<u8>),
    /// The person started speaking. The provider has stopped the reply; stop
    /// playback and report how much of it was heard with
    /// [`RealtimeInput::truncate`].
    SpeechStarted,
    /// What the person said in one turn, as the model heard it.
    Heard(String),
    /// The next piece of what the model is saying.
    Saying(String),
    /// The model asks for a tool; answer with [`RealtimeInput::tool_result`].
    ToolCall {
        call_id: String,
        name: String,
        arguments: String,
    },
    /// One reply is finished: all of its audio has arrived.
    ResponseDone,
    /// The provider refused one request and kept the session as it was. The
    /// code is the provider's short identifier, never its message.
    Refused { code: Option<String> },
    /// A fresh provider session continues the call, after the time limit or a
    /// dropped connection. Audio of an unfinished reply is lost, and the new
    /// session knows the conversation only from its transcript.
    Renewed,
}

/// What a caller asks of a live session; the driver task carries it out.
enum Command {
    Audio(Vec<u8>),
    ToolResult { call_id: String, output: String },
    Text(String),
    Truncate { played_ms: u32 },
}

/// The sending half of a session. Clones share one session; it closes when
/// the last one is dropped.
#[derive(Clone)]
pub struct RealtimeInput {
    commands: mpsc::Sender<Command>,
}

impl RealtimeInput {
    async fn send(&self, command: Command) -> Result<(), SpeechError> {
        self.commands
            .send(command)
            .await
            .map_err(|_| SpeechError::Cancelled)
    }

    /// One frame of microphone PCM at the input rate.
    pub async fn push_audio(&self, pcm: Vec<u8>) -> Result<(), SpeechError> {
        if pcm.is_empty() || !pcm.len().is_multiple_of(2) || pcm.len() > FRAME_LIMIT {
            return Err(SpeechError::UnsupportedFormat(
                "a partial PCM16 frame".into(),
            ));
        }
        self.send(Command::Audio(pcm)).await
    }

    /// The answer to a [`RealtimeEvent::ToolCall`]. The model replies once
    /// its current reply, if any, is done.
    pub async fn tool_result(&self, call_id: &str, output: &str) -> Result<(), SpeechError> {
        self.send(Command::ToolResult {
            call_id: call_id.into(),
            output: output.into(),
        })
        .await
    }

    /// A text turn the model replies to, as if the person had said it.
    pub async fn text(&self, text: &str) -> Result<(), SpeechError> {
        if text.trim().is_empty() {
            return Err(SpeechError::NothingToSay);
        }
        if text.chars().count() > TEXT_LIMIT {
            return Err(SpeechError::UnsupportedFormat("text this long".into()));
        }
        self.send(Command::Text(text.into())).await
    }

    /// After a barge-in: the model keeps only the first `played_ms` of its
    /// interrupted reply, so it does not believe it said what nobody heard.
    pub async fn truncate(&self, played_ms: u32) -> Result<(), SpeechError> {
        self.send(Command::Truncate { played_ms }).await
    }
}

/// One live call. `events` ends after an error, or once the session closes.
pub struct RealtimeSession {
    pub input: RealtimeInput,
    pub events: mpsc::Receiver<Result<RealtimeEvent, SpeechError>>,
}

impl RealtimeSession {
    /// Closes the provider connection and waits until it is closed. Other
    /// clones of the input keep the session open until they are dropped.
    pub async fn close(self) {
        let Self { input, mut events } = self;
        drop(input);
        while events.recv().await.is_some() {}
    }
}

#[async_trait]
pub trait Realtime: Send + Sync {
    /// Provider and model; the voice is chosen per session.
    fn id(&self) -> SpeechId;
    /// Subscription sessions do not incur an additional metered charge.
    fn is_subscription(&self) -> bool;
    /// Connects and applies `config`. Fails, without a session, when the
    /// provider is unreachable, refuses the login or rejects the config.
    async fn open(&self, config: RealtimeConfig) -> Result<RealtimeSession, SpeechError>;
}
