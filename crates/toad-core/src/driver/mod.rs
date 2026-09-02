//! What runs a teammate's turn, and nothing else.
//!
//! A driver is handed a line of text and hands back a stream of updates until
//! the turn is over. It does not know what a tape is, that there is a log, or
//! that anybody is listening: the session above it decides what an update
//! means and writes it down. That is what lets one set of supervisor rules
//! serve both kinds of agent — Toad Agent on Rig in this process
//! ([`rig::InProcess`]) and, from Phase 2, an external harness as a child.
//!
//! The vocabulary is ACP's, because one of the two drivers speaks ACP on the
//! wire and translating in only one direction is the smaller job: a prompt is
//! text, a turn is a run of session updates, and a turn ends with a stop
//! reason.

pub mod acp;
pub mod rig;

use crate::contract::{ConfigChoice, NoticeLevel, Persona, Reach, TokenUsage};
use async_trait::async_trait;
use tokio::sync::mpsc;

/// What a driver says about itself once it is up: who is answering, which
/// models it can be switched to, and which one it is on.
#[derive(Clone, Debug, PartialEq)]
pub struct DriverInfo {
    pub agent_name: String,
    pub models: Vec<ConfigChoice>,
    pub current_model_id: String,
    /// The picker's label for the current model, when the driver has one.
    pub model_label: Option<String>,
}

/// Whether the agent is speaking or thinking. The same distinction names a
/// finished message and the deltas that built it, because it is the same
/// distinction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageKind {
    Agent,
    Thought,
}

/// One thing that happened during a turn.
#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    /// Text as it arrives. Ephemeral: the durable line is the [`Update::Message`]
    /// that lands when the message is whole.
    Delta {
        kind: MessageKind,
        message_id: String,
        text: String,
    },
    Message {
        kind: MessageKind,
        id: String,
        text: String,
    },
    ToolCall {
        call_id: String,
        /// The one line the transcript shows: the tool and what it touched.
        title: String,
        /// The tool's own name, which the UI picks an icon by.
        kind: String,
    },
    ToolResult {
        call_id: String,
        ok: bool,
        output: String,
    },
    /// The turn is over. A driver sends this last, cancelled or not.
    Turn {
        stop_reason: String,
        usage: Option<TokenUsage>,
    },
    Notice {
        level: NoticeLevel,
        text: String,
    },
}

/// One agent, driven.
#[async_trait]
pub trait Driver: Send + Sync {
    /// Brings the agent up for this teammate. Everything a driver needs that
    /// is not on the persona — a preamble, the conversation so far — it was
    /// built with.
    async fn start(&self, persona: &Persona) -> Result<DriverInfo, String>;

    /// Runs one turn. The turn is over when the receiver ends, and the last
    /// update before that is a [`Update::Turn`].
    async fn prompt(&self, text: String, reach: Reach) -> mpsc::Receiver<Update>;

    /// Stops the turn in flight. A driver with no turn running does nothing.
    fn cancel(&self);

    async fn set_model(&self, model_id: &str) -> Result<DriverInfo, String>;
}

/// How much of a string a transcript line keeps, with an ellipsis standing
/// for the rest. A tool's title and a tool's output are cut the same way and
/// at very different lengths; what they share is that the cut is presentation
/// and never touches what the model was given.
pub(crate) fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept}…")
}
