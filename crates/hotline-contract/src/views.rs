//! Views of the transcript, what the room pushes, and the keys that name a
//! thread.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::*;

// ---------------------------------------------------------------------------
// Views of the transcript
// ---------------------------------------------------------------------------

/// A chapter as the drawer lists it: the marker plus how much was said in it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct ChapterSummary {
    pub id: String,
    pub started_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ChapterStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_by: Option<ChapterClose>,
    pub messages: i64,
}

/// The last thing either side said, shown under a name in the roster.
///
/// A roster of names alone says who is there; a roster with these says what is
/// going on. Only messages count — a teammate's last tool call is Hotline's
/// business, not a line of conversation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "contract.ts")]
pub struct Preview {
    pub from: Side,
    pub text: String,
    pub at: i64,
}

/// Which side of a conversation with the user a line came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum Side {
    Me,
    Them,
}

/// One hit from a thread search: a chapter by its note, or a message by its
/// text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub enum ThreadSearchHit {
    Chapter {
        chapter_id: String,
        ts: i64,
        title: String,
        excerpt: String,
        /// The status column as the index holds it, which is a word the
        /// indexer copied rather than one this build named.
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
    Message {
        event_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        chapter_id: Option<String>,
        ts: i64,
        from: Side,
        excerpt: String,
    },
}

/// The last thing said in a peer thread, and which of the two said it.
///
/// Not [`Preview`]'s `me`/`them`: both sides are teammates and neither of them
/// is the person reading, so the name is spelled out rather than implied.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct PeerPreview {
    pub from_name: String,
    pub text: String,
    pub at: i64,
}

/// One thread between two teammates, as a list of them is read.
///
/// `lastAt` is the newest thing in the thread, which is what the window counts
/// unread against — it remembers the latest it has shown, exactly as it does
/// for a tape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct PeerThreadSummary {
    pub thread_key: String,
    /// The other side, from the point of view of the teammate that asked.
    pub with_persona_id: String,
    pub with_name: String,
    /// How many turns have been answered in this thread.
    pub exchanges: i64,
    pub last_at: i64,
    /// A permission card in this thread is still waiting for an answer.
    pub waiting: bool,
    /// Who is mid-reply in *this* thread right now, or absent for nobody.
    ///
    /// Not "is that teammate busy": a teammate deep in its own conversation
    /// with the user is not working on this, and a line saying it is would be
    /// a lie the reader can check.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_persona_id: Option<String>,
    /// Written as `null` rather than omitted, because a thread with nothing
    /// said in it is a row the window still draws.
    #[ts(optional = nullable)]
    pub preview: Option<PeerPreview>,
}

/// A search hit that names whose conversation it came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub enum GlobalSearchHit {
    Chapter {
        persona_id: String,
        chapter_id: String,
        ts: i64,
        title: String,
        excerpt: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
    Message {
        persona_id: String,
        event_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        chapter_id: Option<String>,
        ts: i64,
        from: Side,
        excerpt: String,
    },
}

// ---------------------------------------------------------------------------
// What the room pushes
// ---------------------------------------------------------------------------

/// `transcriptAppended` and `transcriptUpdated`: one event, and whose tape it
/// belongs to. The two pushes differ in what the window does with the event,
/// not in what is on the wire.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct TranscriptPush {
    pub persona_id: String,
    pub event: TranscriptEvent,
}

/// `peerThreadAppended` and `peerThreadUpdated`. A peer thread has no "me" in
/// it, so the line is addressed by thread rather than by teammate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct PeerThreadPush {
    pub thread_key: String,
    pub event: TranscriptEvent,
}

/// `streamDelta`: text arriving as the agent writes it. Never persisted — the
/// durable line is the `TranscriptEvent` that lands when the message is whole.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = "contract.ts")]
pub enum StreamDelta {
    AgentDelta {
        persona_id: String,
        message_id: String,
        text: String,
    },
    ThoughtDelta {
        persona_id: String,
        message_id: String,
        text: String,
    },
    /// Text arriving in a side thread. Addressed by side id, because a side
    /// thread is a second conversation with the same teammate and its words
    /// must never land in the main one. Subscribe to `{"side": "<sideId>"}`.
    SideAgentDelta {
        side_id: String,
        message_id: String,
        text: String,
    },
    SideThoughtDelta {
        side_id: String,
        message_id: String,
        text: String,
    },
    /// Text arriving in any thread, addressed by its `ThreadId`: what a client
    /// that declared `threads2` is sent in place of `AgentDelta`,
    /// `ThoughtDelta`, `SideAgentDelta` and `SideThoughtDelta`. Subscribe to
    /// `{"threadId": {"kind", "key"}}`.
    ThreadDelta {
        thread: ThreadId,
        message_id: String,
        kind: DeltaKind,
        text: String,
    },
    /// How far the teammate's computer image has downloaded. Drawn as a ring
    /// where the computer's button sits, so the conversation carries on
    /// around it; never written to the tape.
    ComputerPull {
        persona_id: String,
        layers_done: u32,
        layers_total: u32,
        status: PullStatus,
    },
}

/// What a `ThreadDelta` carries: words the agent is saying, or thinking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum DeltaKind {
    Text,
    Thought,
}

// ---------------------------------------------------------------------------
// Threads
// ---------------------------------------------------------------------------

/// What kind of conversation a thread is. The kind picks its [`Policy`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum ThreadKind {
    /// The person and a teammate: the teammate's tape.
    Dm,
    /// A work thread: a second, parallel conversation with a teammate, with
    /// its own context and the teammate's full tools. The person opens one
    /// beside the DM (a side thread), and a teammate opens one on another by
    /// handing it work. "Side" is the name it is stored and sent under.
    Side,
    /// Two teammates talking to each other: an ask. A handoff is a work thread
    /// on the teammate it was handed to.
    Pair,
    /// A voice call with a teammate, kept in `calls/<id>.jsonl`.
    Call,
    /// A subagent's run of one task.
    Run,
}

/// Which thread: its kind and the key its stream is named by. A teammate's id
/// for a DM, the minted id for a side thread, run or call, and the pair key
/// (the core's `paths::thread_key`) for a pair.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export, export_to = "contract.ts")]
pub struct ThreadId {
    pub kind: ThreadKind,
    pub key: String,
}

impl ThreadKind {
    /// The kind as it is written in a link and in a log line.
    pub fn name(self) -> &'static str {
        match self {
            Self::Dm => "dm",
            Self::Side => "side",
            Self::Pair => "pair",
            Self::Call => "call",
            Self::Run => "run",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        [Self::Dm, Self::Side, Self::Pair, Self::Call, Self::Run]
            .into_iter()
            .find(|kind| kind.name() == name)
    }
}

impl ThreadId {
    pub fn dm(persona_id: impl Into<String>) -> Self {
        Self::of(ThreadKind::Dm, persona_id)
    }

    pub fn side(side_id: impl Into<String>) -> Self {
        Self::of(ThreadKind::Side, side_id)
    }

    pub fn pair(key: impl Into<String>) -> Self {
        Self::of(ThreadKind::Pair, key)
    }

    pub fn run(run_id: impl Into<String>) -> Self {
        Self::of(ThreadKind::Run, run_id)
    }

    pub fn call(call_id: impl Into<String>) -> Self {
        Self::of(ThreadKind::Call, call_id)
    }

    pub fn new(kind: ThreadKind, key: impl Into<String>) -> Self {
        Self::of(kind, key)
    }

    fn of(kind: ThreadKind, key: impl Into<String>) -> Self {
        Self {
            kind,
            key: key.into(),
        }
    }
}

/// `side:<id>`, the way a log line or a search hit names a thread.
impl std::fmt::Display for ThreadId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}:{}", self.kind.name(), self.key)
    }
}

/// How a closed thread ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts", rename = "ThreadEnd")]
pub enum End {
    /// The person ended it.
    Person,
    /// The teammate said it was done.
    Agent,
    /// Nobody spoke in it for long enough.
    Idle,
    /// Its teammate was stopped, changed or removed, or the desk restarted.
    Stopped,
    /// A run finished its task.
    Done,
    /// A run could not finish.
    Failed,
    /// A run was stopped.
    Cancelled,
}

impl End {
    pub fn name(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Agent => "agent",
            Self::Idle => "idle",
            Self::Stopped => "stopped",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        [
            Self::Person,
            Self::Agent,
            Self::Idle,
            Self::Stopped,
            Self::Done,
            Self::Failed,
            Self::Cancelled,
        ]
        .into_iter()
        .find(|end| end.name() == name)
    }
}

impl From<SideEnd> for End {
    fn from(end: SideEnd) -> Self {
        match end {
            SideEnd::Agent => Self::Agent,
            SideEnd::Person => Self::Person,
            SideEnd::Idle => Self::Idle,
            SideEnd::Stopped => Self::Stopped,
        }
    }
}
