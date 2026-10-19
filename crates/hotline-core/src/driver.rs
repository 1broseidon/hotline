//! What runs a teammate's turn, and nothing else.
//!
//! A driver is handed a line of text and hands back a stream of updates until
//! the turn is over. It does not know what a tape is, that there is a log, or
//! that anybody is listening: the session above it decides what an update
//! means and writes it down. That is what lets one set of supervisor rules
//! serve both kinds of agent — Hotline Agent on Rig in this process
//! ([`rig::InProcess`]) and, from Phase 2, an external harness as a child.
//!
//! The vocabulary is ACP's, because one of the two drivers speaks ACP on the
//! wire and translating in only one direction is the smaller job: a prompt is
//! text, a turn is a run of session updates, and a turn ends with a stop
//! reason.

pub mod acp;
pub mod auth;
pub(crate) mod failure;
pub mod rig;

use crate::contract::{
    Attachment, ConfigChoice, NoticeLevel, PermissionOption, Persona, Reach, SessionCapabilities,
    SessionConfig, SubagentStatus, TokenUsage,
};
use async_trait::async_trait;
pub(crate) use hotline_room::text::clip;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::{mpsc, watch};

/// The backend id of Hotline Agent, which runs in this process.
///
/// It is `hotline` because it is written into every teammate's record and into
/// the checkpoints kept per backend: an identifier, not a label. Any other id
/// names an ACP child, which is what makes this the one place the two kinds
/// of agent are told apart.
pub const HOTLINE_BACKEND_ID: &str = "hotline";

pub(crate) use hotline_tools::capability::{CapabilityEpoch, CapabilityLease};

/// What a driver says about itself once it is up: who is answering, what it
/// can be switched between, and — for a child that reopened a conversation of
/// its own — which session it is in and whether it remembers it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DriverInfo {
    pub agent_name: String,
    pub agent_version: Option<String>,
    /// The agent's own id for this conversation, which is what a checkpoint
    /// is. Only a child has one; Hotline Agent's memory is the tape.
    pub session_id: Option<String>,
    /// Whether the agent genuinely recalls the conversation, as against
    /// reading history it has never seen. Never guessed: see docs/design.md.
    pub context_restored: bool,
    pub models: Vec<ConfigChoice>,
    pub current_model_id: String,
    /// The picker's label for the current model, when the driver has one.
    pub model_label: Option<String>,
    pub modes: Vec<ConfigChoice>,
    pub current_mode_id: Option<String>,
    /// The picker's own name, which agents spell differently ("Mode",
    /// "Thinking"), when the driver offers modes at all.
    pub mode_label: Option<String>,
    /// Select options that are not the model or mode picker. Hotline Agent
    /// reports effort here; an ACP child reports whatever else the harness
    /// offered. The session copies this onto `SessionInfo.configs`.
    pub configs: Vec<SessionConfig>,
    pub capabilities: SessionCapabilities,
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
    /// The session drains earlier updates before writing a chapter boundary.
    Chapter {
        boundary: ChapterBoundary,
    },
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
        /// Images that rode with this result. The session writes a
        /// `computer_frame` for each when the tool is the computer's; any
        /// other origin keeps the placeholder in `output` and nothing else.
        images: Vec<ToolImage>,
    },
    /// The agent is asking to be allowed to do something, and will not go on
    /// until it is answered. Only a child driver asks: Hotline Agent's one policy
    /// is how far its tools reach, decided before the turn starts.
    Permission {
        request_id: String,
        /// What is actually being asked for, in a sentence somebody can answer.
        title: String,
        options: Vec<PermissionOption>,
    },
    /// The turn is still open but has nothing more to say until one of its
    /// background jobs finishes or the person says something: Hotline Agent
    /// with a shell command or a subagent still running, or a harness that
    /// holds its prompt open for subagents it started. Whatever it said last
    /// is what the person should be reading while they wait, and a turn left
    /// open only for subagents reads as done (`awaiting_subagents`).
    Parked,
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

/// A subagent a harness runs on its own, as it reports it: an agent that
/// shows its subagents as sessions of their own rather than as tool calls.
/// `child` is the harness's name for the subagent's session; the reports for
/// one child arrive in order, beginning with `Started`.
#[derive(Clone, Debug, PartialEq)]
pub enum SubagentReport {
    Started {
        child: String,
        title: String,
        /// What it was asked to do, as near the words it was handed as the
        /// harness says.
        task: String,
    },
    /// Something the subagent did, in the vocabulary of a turn.
    Update { child: String, update: Update },
    Ended {
        child: String,
        status: SubagentStatus,
    },
}

#[derive(Clone, Debug)]
pub struct ChapterBoundary(Arc<Mutex<Option<tokio::sync::oneshot::Sender<Option<String>>>>>);

impl PartialEq for ChapterBoundary {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl ChapterBoundary {
    pub(crate) fn new() -> (Self, tokio::sync::oneshot::Receiver<Option<String>>) {
        let (send, receive) = tokio::sync::oneshot::channel();
        (Self(Arc::new(Mutex::new(Some(send)))), receive)
    }
    pub(crate) fn finish(&self, note: Option<String>) {
        let sender = self.0.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(sender) = sender {
            let _ = sender.send(note);
        }
    }
}

/// One image a tool returned, as base64 and the mime type the server named.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolImage {
    pub data: String,
    pub mime_type: String,
}

impl ToolImage {
    pub fn data_url(&self) -> String {
        format!("data:{};base64,{}", self.mime_type, self.data)
    }

    pub fn placeholder(&self) -> String {
        let bytes = self.data.len().saturating_mul(3) / 4;
        let size = if bytes < 1024 {
            format!("{bytes} B")
        } else {
            format!("{} KB", bytes / 1024)
        };
        format!("[image {}, {size}]", self.mime_type)
    }
}

/// Where a quiet scheduled run's one message goes (BRO-96).
///
/// A quiet run's words become thinking by kind, so nothing it writes reaches
/// the person. This is the exception it has to ask for by name: the room
/// posts the text as a visible message stamped with the job, and notifies.
pub trait Escalate: Send + Sync {
    /// Posts `text` to the person. Refuses a second message from the same
    /// run, so a confused run cannot flood the conversation.
    fn escalate(&self, text: &str) -> Result<(), String>;
}

/// One agent, driven.
#[async_trait]
pub trait Driver: Send + Sync {
    /// Only the desk may call these; auth I/O is never a driver Update.
    async fn auth_start(
        self: Arc<Self>,
        _method: &str,
        _owner: tokio_util::sync::CancellationToken,
    ) -> Result<String, String> {
        Err("This agent does not offer sign-in.".into())
    }
    fn auth_poll(&self, _id: &str) -> Result<crate::wire::AuthStatus, String> {
        Err("That sign-in is no longer available.".into())
    }
    fn auth_input(&self, _id: &str, _input: &str) -> Result<(), String> {
        Err("That sign-in is no longer available.".into())
    }
    fn auth_cancel(&self, _id: &str) -> Result<(), String> {
        Err("That sign-in is no longer available.".into())
    }
    fn take_auth_success(&self) -> bool {
        false
    }
    /// A signed-out child is initialized and recoverable, but has no session yet.
    fn startup_failure(&self) -> Option<String> {
        None
    }

    /// Brings the agent up for this teammate. Everything a driver needs that
    /// is not on the persona — a preamble, the conversation so far — it was
    /// built with.
    async fn start(&self, persona: &Persona) -> Result<DriverInfo, String>;

    /// Subscribes to metadata that can change while the driver is alive.
    ///
    /// Most drivers learn their picker state only during `start` and return
    /// it there. An ACP harness may advertise a new mode or config option at
    /// any point in the session, so it can opt into this watch without making
    /// every driver own a background task.
    fn subscribe_info(&self) -> Option<watch::Receiver<DriverInfo>> {
        None
    }

    /// Work the agent takes up by itself between turns: a subagent reporting
    /// back, or a command it left running finishing, that it answers without
    /// being prompted. Each receiver is one such activity, run by the session
    /// as it runs a turn. A driver whose agent only speaks when prompted
    /// answers `None`.
    fn subscribe_unprompted(&self) -> Option<mpsc::UnboundedReceiver<mpsc::Receiver<Update>>> {
        None
    }

    /// Lines the agent is to be woken with between turns: a command a turn
    /// of its left running ended on its own. The session takes each as a
    /// turn of its own, queued behind any in flight. Subscribing is what lets
    /// the driver leave commands running past a turn, so only the session
    /// that can be woken does; a driver that never leaves one answers `None`.
    fn subscribe_wakes(&self) -> Option<mpsc::UnboundedReceiver<String>> {
        None
    }

    /// The commands the agent's turns left running, for the room to show the
    /// person and let them read and stop. Only Hotline Agent leaves any.
    fn processes(&self) -> Option<Arc<crate::session::jobs::Processes>> {
        None
    }

    /// The subagents the agent runs and shows, each as it starts, works and
    /// ends. Hotline Agent's subagents are the room's own runs, so only a
    /// harness that reports its own answers `Some`.
    fn subscribe_subagents(&self) -> Option<mpsc::UnboundedReceiver<SubagentReport>> {
        None
    }

    /// Picker metadata worked out again now, for a driver whose choices come
    /// from the room rather than from the agent. Hotline Agent's models are
    /// the room's connections, so a key added or a list refreshed changes
    /// them mid-session. A harness reports its own and answers `None`.
    fn current_info(&self) -> Option<DriverInfo> {
        None
    }

    /// Runs one turn. The turn is over when the receiver ends, and the last
    /// update before that is a [`Update::Turn`].
    ///
    /// The attachments are files handed over with the words. They arrive
    /// beside the text rather than inside it because the two kinds of agent
    /// take them differently — one reads a path with its own tool, the other
    /// is given a link block — and only the driver knows which it is.
    async fn prompt(
        &self,
        text: String,
        attachments: Vec<Attachment>,
        reach: Reach,
    ) -> mpsc::Receiver<Update>;

    /// Runs the turn that just failed again, for the person's Retry. The
    /// words and attachments are the line that started it, already on the
    /// tape. A driver that keeps its own history re-runs it without adding
    /// the line twice; one that cannot (a harness, whose session ended with
    /// the failure) is asked the same words again, which is the default.
    async fn retry(
        &self,
        text: String,
        attachments: Vec<Attachment>,
        reach: Reach,
    ) -> mpsc::Receiver<Update> {
        self.prompt(text, attachments, reach).await
    }

    /// Hands the next turn, and only that turn, a way to be heard: a quiet
    /// scheduled run's `tell_person`. `None` clears one left unused. A
    /// driver that cannot offer a tool of its own for one turn ignores it,
    /// and that run stays silent as it always has.
    fn escalate_next(&self, _escalation: Option<Arc<dyn Escalate>>) {}

    /// Admits new operator input into the current activity. False means the
    /// activity already ended or this driver cannot steer; the session keeps
    /// the message in its queue. This never means Stop.
    fn steer(&self, _text: String, _attachments: Vec<Attachment>) -> bool {
        false
    }

    /// Input admitted just before a failed activity closed, but never used in a request.
    fn take_unconsumed(&self) -> Vec<(String, Vec<Attachment>)> {
        Vec::new()
    }

    /// A broken child session must not be restored, even when Stop hid its error.
    fn checkpoint_valid(&self) -> bool {
        true
    }

    /// Stops the turn in flight. A driver with no turn running does nothing.
    fn cancel(&self);

    /// Permanently invalidates handles owned by this session, in addition to
    /// stopping its current turn. Room policy changes use this boundary so a
    /// queued or cached call cannot wake up with the old rights later.
    fn invalidate(&self) {
        self.cancel();
    }

    async fn set_model(&self, model_id: &str) -> Result<DriverInfo, String>;

    /// Switches the agent's mode. Only agents that offer modes have one, so
    /// the default is the refusal a caller would otherwise have to guess at.
    async fn set_mode(&self, _mode_id: &str) -> Result<DriverInfo, String> {
        Err("This agent does not offer modes.".to_string())
    }

    /// Sets a config the agent offers beyond the model and the mode. The
    /// default is the refusal a caller would otherwise have to guess at.
    async fn set_config(&self, _config_id: &str, _value: &str) -> Result<DriverInfo, String> {
        Err("This agent does not offer that setting.".to_string())
    }

    /// Answers a permission the agent is waiting on, and says whether there
    /// was still anything behind that request. `false` is a card whose turn
    /// ended, whose session stopped, or that was answered already.
    fn answer_permission(&self, _request_id: &str, _option_id: &str) -> bool {
        false
    }
}

/// The transcript line for a tool result: the text the tool produced, and
/// a one-line stand-in for each image so the card still says something.
pub(crate) fn with_image_placeholders(text: &str, images: &[ToolImage]) -> String {
    if images.is_empty() {
        return text.to_string();
    }
    let mut parts = Vec::new();
    if !text.is_empty() {
        parts.push(text.to_string());
    }
    parts.extend(images.iter().map(ToolImage::placeholder));
    parts.join("\n")
}
