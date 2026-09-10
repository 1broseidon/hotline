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

use crate::contract::{
    Attachment, ConfigChoice, NoticeLevel, PermissionOption, Persona, Reach, SessionCapabilities,
    SessionConfig, TokenUsage,
};
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::{mpsc, watch};

/// The backend id of Toad Agent, which runs in this process.
///
/// It stays `pi` because it is written into every teammate's record and into
/// the checkpoints kept per backend: an identifier, not a label. Any other id
/// names an ACP child, which is what makes this the one place the two kinds
/// of agent are told apart.
pub const PI_BACKEND_ID: &str = "pi";

/// The authority shared by every handle a session hands to a driver.
///
/// A policy update first advances the generation and closes the epoch. The
/// room opens it again only after the new record is durable, so a session that
/// races the update can never capture the old policy in a usable lease. A
/// stop advances the generation while leaving the epoch open: a later,
/// explicit start may use the current policy, while handles from the stopped
/// session remain permanently dead.
#[derive(Clone)]
pub(crate) struct CapabilityEpoch {
    state: Arc<Mutex<CapabilityState>>,
}

struct CapabilityState {
    generation: u64,
    active: bool,
}

impl Default for CapabilityEpoch {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(CapabilityState {
                generation: 0,
                active: true,
            })),
        }
    }
}

/// An immutable snapshot of one session's authority. The snapshot is cheap to
/// clone into tool closures, but validity always consults the shared epoch.
#[derive(Clone)]
pub(crate) struct CapabilityLease {
    epoch: CapabilityEpoch,
    generation: u64,
    active_at_capture: bool,
    revoked: Arc<AtomicBool>,
    dependencies: Vec<Arc<CapabilityLease>>,
}

impl CapabilityEpoch {
    pub(crate) fn lease(&self) -> CapabilityLease {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        CapabilityLease {
            epoch: self.clone(),
            generation: state.generation,
            active_at_capture: state.active,
            revoked: Arc::new(AtomicBool::new(false)),
            dependencies: Vec::new(),
        }
    }

    /// Quarantines all existing and newly captured leases until the room has
    /// appended the replacement policy and calls [`Self::activate`].
    pub(crate) fn invalidate(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.generation = state.generation.wrapping_add(1);
        state.active = false;
    }

    /// Invalidates old handles while keeping the authority available to a
    /// later explicit start under the still-current policy.
    pub(crate) fn stop(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let active = state.active;
        state.generation = state.generation.wrapping_add(1);
        state.active = active;
    }

    pub(crate) fn activate(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.active = true;
    }
}

impl CapabilityLease {
    /// Whether this lease still names the room's active generation.
    pub(crate) fn is_current(&self) -> bool {
        if self.revoked.load(Ordering::SeqCst)
            || self.dependencies.iter().any(|lease| !lease.is_current())
        {
            return false;
        }
        let state = self
            .epoch
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.active_at_capture && state.active && state.generation == self.generation
    }

    /// Refuses a call made through a stale driver, tool closure, or workspace.
    pub(crate) fn check(&self) -> Result<(), String> {
        if self.is_current() {
            Ok(())
        } else {
            Err("This teammate's capabilities have been revoked.".to_string())
        }
    }

    /// Permanently invalidates this lease and every handle cloned from it.
    /// Peer sessions use this in addition to their caller and target room
    /// epochs, because revoking the caller must also kill a target driver that
    /// is otherwise still within the target's current policy epoch.
    pub(crate) fn revoke(&self) {
        self.revoked.store(true, Ordering::SeqCst);
    }

    /// Creates an independently revocable child lease in this same session
    /// generation. A peer session must be able to end without revoking the
    /// caller's main tool handles, while a stop or policy change on the
    /// caller still invalidates the peer through the shared epoch.
    pub(crate) fn scoped(&self) -> Self {
        Self {
            epoch: self.epoch.clone(),
            generation: self.generation,
            active_at_capture: self.active_at_capture,
            revoked: Arc::new(AtomicBool::new(false)),
            dependencies: vec![Arc::new(self.clone())],
        }
    }

    /// Delegated tools need both the recipient's authority and the requester's
    /// authority. Keeping the whole dependency includes session replacement,
    /// as well as explicit revocation, even across different teammates.
    pub(crate) fn with_dependency(mut self, requester: &Self) -> Self {
        self.dependencies.push(Arc::new(requester.clone()));
        self
    }
}

/// What a driver says about itself once it is up: who is answering, what it
/// can be switched between, and — for a child that reopened a conversation of
/// its own — which session it is in and whether it remembers it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DriverInfo {
    pub agent_name: String,
    pub agent_version: Option<String>,
    /// The agent's own id for this conversation, which is what a checkpoint
    /// is. Only a child has one; Toad Agent's memory is the tape.
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
    /// Select options that are not the model or mode picker. Toad Agent
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
    /// until it is answered. Only a child driver asks: Toad Agent's one policy
    /// is how far its tools reach, decided before the turn starts.
    Permission {
        request_id: String,
        /// What is actually being asked for, in a sentence somebody can answer.
        title: String,
        options: Vec<PermissionOption>,
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

/// One agent, driven.
#[async_trait]
pub trait Driver: Send + Sync {
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

    /// Admits new operator input into the current activity. False means the
    /// activity already ended or this driver cannot steer; the session keeps
    /// the message in its queue. This never means Stop.
    fn steer(&self, _text: String, _attachments: Vec<Attachment>) -> bool {
        false
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

#[cfg(test)]
mod tests {
    use super::{CapabilityEpoch, CapabilityLease};

    #[test]
    fn a_lease_captured_during_quarantine_never_revives() {
        let epoch = CapabilityEpoch::default();
        let before = epoch.lease();

        epoch.invalidate();
        let during = epoch.lease();
        assert!(!before.is_current());
        assert!(!during.is_current());

        epoch.activate();
        assert!(!before.is_current());
        assert!(!during.is_current());
        assert!(epoch.lease().is_current());
    }

    #[test]
    fn stopping_invalidates_old_leases_but_allows_a_fresh_start() {
        let epoch = CapabilityEpoch::default();
        let old = epoch.lease();

        epoch.stop();

        assert!(!old.is_current());
        assert!(epoch.lease().is_current());
    }

    #[test]
    fn stopping_during_quarantine_does_not_reopen_it() {
        let epoch = CapabilityEpoch::default();
        epoch.invalidate();

        epoch.stop();

        assert!(!epoch.lease().is_current());
        epoch.activate();
        assert!(epoch.lease().is_current());
    }

    #[test]
    fn revoking_one_lease_does_not_revoke_a_new_capture() {
        let epoch = CapabilityEpoch::default();
        let peer = epoch.lease();
        let current = epoch.lease();

        peer.revoke();

        assert!(!peer.is_current());
        assert!(current.is_current());
        assert!(epoch.lease().is_current());
    }

    #[test]
    fn a_scoped_lease_follows_ancestors_but_not_its_children() {
        let epoch = CapabilityEpoch::default();
        let parent = epoch.lease();
        let child = parent.scoped();
        let grandchild = child.scoped();

        child.revoke();

        assert!(parent.is_current());
        assert!(!child.is_current());
        assert!(!grandchild.is_current());

        let child = parent.scoped();
        let grandchild = child.scoped();
        grandchild.revoke();

        assert!(parent.is_current());
        assert!(child.is_current());
        assert!(!grandchild.is_current());

        parent.revoke();
        assert!(!child.is_current());
    }

    #[test]
    fn delegated_authority_requires_both_teammates_current_sessions() {
        let caller = CapabilityEpoch::default();
        let recipient = CapabilityEpoch::default();
        let independent = recipient.lease();
        let delegated = recipient.lease().with_dependency(&caller.lease());
        assert!(delegated.is_current());
        caller.stop();
        assert!(!delegated.is_current());
        assert!(independent.is_current());

        let delegated = recipient.lease().with_dependency(&caller.lease());
        recipient.stop();
        assert!(!delegated.is_current());
        assert!(caller.lease().is_current());
    }

    #[test]
    fn capability_check_explains_a_stale_handle() {
        let epoch = CapabilityEpoch::default();
        let lease: CapabilityLease = epoch.lease();
        epoch.invalidate();

        assert_eq!(
            lease.check().unwrap_err(),
            "This teammate's capabilities have been revoked."
        );
    }
}
