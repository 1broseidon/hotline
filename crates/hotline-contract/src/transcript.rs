//! The transcript: `TranscriptEvent` and its parts. This is the tape's shape,
//! byte-compatible with the reference tree's (AGENTS.md rule 4).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::*;

// ---------------------------------------------------------------------------
// The transcript
// ---------------------------------------------------------------------------

/// One durable line in a teammate's transcript. Appended as JSONL and replayed
/// at startup. Note this is Hotline's own record: replaying it is not the same as
/// the agent remembering the conversation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub enum TranscriptEvent {
    User {
        id: String,
        ts: i64,
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        attachments: Option<Vec<Attachment>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reactions: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reply_to: Option<String>,
        /// Nobody typed this: a schedule fired. The text stays the prompt in
        /// full; this is what lets the transcript draw one line and keep the
        /// prompt behind an expander.
        #[serde(skip_serializing_if = "Option::is_none")]
        scheduled: Option<ScheduledRun>,
        /// An emphasis on this bubble.
        #[serde(skip_serializing_if = "Option::is_none")]
        ring: Option<RingIntent>,
        #[serde(skip_serializing_if = "Option::is_none")]
        receipt: Option<Receipt>,
        /// Which of the person's apps this was written in. Never drawn: the
        /// teammate hears it beside the time, so "my browser" means the
        /// right one.
        #[serde(skip_serializing_if = "Option::is_none")]
        client: Option<Client>,
    },
    Agent {
        id: String,
        ts: i64,
        /// With a file, the caption, which may be empty.
        text: String,
        /// A file the teammate sent the person with `send_file`: one per
        /// message, read back with `file.read`.
        #[serde(skip_serializing_if = "Option::is_none")]
        attachments: Option<Vec<Attachment>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reactions: Option<Vec<String>>,
        /// An emphasis on this bubble.
        #[serde(skip_serializing_if = "Option::is_none")]
        ring: Option<RingIntent>,
        #[serde(skip_serializing_if = "Option::is_none")]
        receipt: Option<Receipt>,
        /// On a reply to a turn said on a call, the version written to be
        /// heard, while `text` is the version written to be read. On the
        /// first bubble of the reply only. A client that does not know it
        /// shows `text`, which stands alone.
        #[serde(skip_serializing_if = "Option::is_none")]
        spoken: Option<String>,
        /// Posted while the person was on the call; kept without an unread or notification.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        silent: Option<bool>,
        /// A post from one of the teammate's other threads (a call, a work
        /// thread): where it was said. A client draws "From the call" from
        /// it. Absent from what the DM's own turns say, and from every tape
        /// written before posts were stamped.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<DeliveryFrom>,
        /// A post from a scheduled run: which job it came from.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scheduled: Option<ScheduledRun>,
    },
    Thought {
        id: String,
        ts: i64,
        text: String,
    },
    Tool {
        id: String,
        ts: i64,
        tool_call_id: String,
        title: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_kind: Option<String>,
        status: ToolStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        locations: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        files: Option<Vec<ChangedFile>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        output: Option<Vec<ToolOutput>>,
    },
    Permission {
        /// The call that raised this card in the home chat.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<DeliveryFrom>,
        id: String,
        ts: i64,
        request_id: String,
        title: String,
        options: Vec<PermissionOption>,
        #[serde(skip_serializing_if = "Option::is_none")]
        decision: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        decided_option_name: Option<String>,
    },
    Plan {
        id: String,
        ts: i64,
        entries: Vec<PlanEntry>,
    },
    Notice {
        id: String,
        ts: i64,
        level: NoticeLevel,
        text: String,
    },
    /// The computer image being pulled for this teammate, as the runtime
    /// reports its layers: one line per pull, rewritten in place as layers
    /// land, so a minute of download is a bar and not a silence. `done` and
    /// `failed` are the line's afterlife. A runtime whose output the desk
    /// cannot count leaves `layersTotal` at zero, and the bar is indeterminate.
    ComputerPull {
        id: String,
        ts: i64,
        image: String,
        layers_done: u32,
        layers_total: u32,
        status: PullStatus,
        /// How long the pull took, once it is done.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        elapsed_ms: Option<i64>,
    },
    /// A frame of the teammate's computer screen, taken as its capture tool
    /// ran. The chat is where the work actually happens, so what the agent saw
    /// belongs in it — a thumbnail, with the live screen a click away.
    ComputerFrame {
        id: String,
        ts: i64,
        data_url: String,
    },
    /// The agent asked the human to take an action it cannot — credentials, a
    /// 2FA tap, a CAPTCHA — usually on its computer. Pending renders a card
    /// with the way in; any other status is the card's afterlife.
    HumanAction {
        /// The call that raised this card in the home chat.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<DeliveryFrom>,
        id: String,
        ts: i64,
        action_id: String,
        reason: String,
        /// An informational yes/no question. It never approves an operation.
        /// Older cards are actions and remain desk-only.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        question: Option<bool>,
        status: HumanActionStatus,
        /// What the person said with their answer, when they said anything.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        /// The asking turn did not wait: the answer comes back to the
        /// teammate as a [`TranscriptEvent::Delivery`], whenever it comes.
        /// Such a card is not orphaned by a restart or a stop, because
        /// nothing was parked on it; only the person, its lifetime running
        /// out, or the teammate withdrawing it settles it. Absent on a card a
        /// tool is waiting on.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delivers: Option<bool>,
        /// When a card that delivers is taken down unanswered, in ms. The
        /// teammate chose how long it waits, up to a day, when it asked, and
        /// a restart must keep that choice, so it is on the card. Absent on a
        /// card a tool is waiting on, and on one written before cards carried
        /// it, which lives the default lifetime from its `ts`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at: Option<i64>,
        /// The work thread the card was raised in, when it was raised in one:
        /// answer goes back to that thread's agent. Call cards live on the
        /// home tape with a mirrored status on the call; other work cards
        /// stay on their thread. Absent on the tape's own cards.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread: Option<String>,
    },
    /// A site asked the teammate's browser to make a passkey, under an
    /// arming the person started for that site: the request waits in the
    /// browser until the person answers here. Pending renders the card
    /// with Approve and Deny; any other status is the card's afterlife.
    PasskeyAsk {
        /// The call that raised this card in the home chat.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<DeliveryFrom>,
        id: String,
        ts: i64,
        ask_id: String,
        /// The name the passkey is stored under once made.
        name: String,
        rp_id: String,
        origin: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rp_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_display_name: Option<String>,
        status: PasskeyAskStatus,
    },
    Peer {
        id: String,
        ts: i64,
        thread_key: String,
        with_persona_id: String,
        with_name: String,
        role: PeerRole,
        exchanges: i64,
        status: PeerStatus,
        /// What kind of citizen the other side is. Absent means a teammate —
        /// every marker written before client seats existed, and every one
        /// written for a teammate since. `client` means an outside MCP agent
        /// holding a seat in this room; `withName` already carries the desk
        /// it connected through, as in "Claude Code @ beastie".
        ///
        /// The field exists because the name alone cannot carry it: "Claude
        /// Code @ beastie" and "Boris @ beastie" read identically, and a
        /// message from outside the room must never look like one from a
        /// teammate.
        #[serde(skip_serializing_if = "Option::is_none")]
        seat: Option<PeerSeat>,
    },
    /// Something that came back to this teammate after it had moved on: a
    /// colleague's answer to a message it sent. Nobody typed it and it is not
    /// a bubble; the tape keeps it so it reaches the agent exactly once, in
    /// order, across a restart, and so the turn it starts can say why it
    /// started. `text` is what came back; the agent hears it framed.
    Delivery {
        id: String,
        ts: i64,
        cause: DeliveryCause,
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        receipt: Option<Receipt>,
        /// The thread it came from and the request it answers. Absent from a
        /// delivery written before this was kept, which is told apart by the
        /// id it was written under.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<DeliveryFrom>,
    },
    /// A pair reached the automatic message cap. Its queue waits for the
    /// person to Keep going or Stop exchange, across intents and restarts.
    ExchangePaused {
        id: String,
        ts: i64,
        with_persona_id: String,
        with_name: String,
        exchanges: i64,
        status: ExchangePauseStatus,
    },
    Turn {
        id: String,
        ts: i64,
        stop_reason: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<TokenUsage>,
    },
    /// A subagent this teammate started: one line on the teammate's tape,
    /// written again under the same id as the run goes, that opens the run's
    /// own transcript. The run itself never writes to this tape — what it did
    /// is on its own stream, `runs/<runId>`, and what it reported came back
    /// to the teammate as a job result.
    ///
    /// Stored as a thread's link (`thread::Link`) and sent as this, so a
    /// client that draws markers needs nothing else; lines written before
    /// links are this already.
    Subagent {
        id: String,
        /// When the run started. The line keeps its place as it is rewritten.
        ts: i64,
        run_id: String,
        /// The short label the teammate gave the task.
        title: String,
        status: SubagentStatus,
        /// How long it ran, once it has stopped.
        #[serde(skip_serializing_if = "Option::is_none")]
        elapsed_ms: Option<i64>,
    },
    /// A side thread this teammate has going: a second conversation with the
    /// same teammate about another task, running in parallel with this one.
    /// One line on the main tape, written again under the same id as the
    /// thread goes, that opens the thread's own transcript at `sides/<sideId>`.
    /// Nothing the thread says is on this tape. While `status` is `live` the
    /// line reads "Started a side thread"; `parked` is the same thread with
    /// its agent let go of, still open; once `archived` it is a one-line
    /// `result` with an Open, and `note` holds the closing handoff note.
    ///
    /// Stored as a thread's link (`thread::Link`) and sent as this.
    Side {
        id: String,
        /// When the thread started. The line keeps its place as it is rewritten.
        ts: i64,
        side_id: String,
        persona_id: String,
        /// The task, shortened to a label.
        title: String,
        status: SideStatus,
        /// Once archived: what the thread came to, in a line. Absent when
        /// nothing was said worth keeping.
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<String>,
        /// Once archived: who ended it.
        #[serde(skip_serializing_if = "Option::is_none")]
        archived_by: Option<SideEnd>,
        #[serde(skip_serializing_if = "Option::is_none")]
        archived_at: Option<i64>,
        /// Once archived: the closing handoff note (goal, what got done,
        /// what is still open, key files), when a model could write one.
        /// What `search_thread` finds the thread by.
        #[serde(skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        /// The agent's own id for the thread's conversation, written after
        /// its first turn, the harness that issued it, and the agent account
        /// it ran on. What lets a parked or archived thread be reopened with
        /// recall.
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        backend_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        account_id: Option<String>,
        /// The teammate that opened the thread, when one did: a handoff is a
        /// thread on the teammate it was handed to, and the line is on both
        /// teammates' tapes. Absent when the person opened it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        opened_by: Option<SideOpener>,
    },
    /// A voice call with this teammate: one quiet line on the tape, written
    /// again under the same id as the call goes, that stands for the call
    /// kept in `calls/<callId>`. What was said on it is not on this tape
    /// unless the teammate was handed it. While `status` is `live` the line
    /// reads as a call in progress; once `ended` it says how long it lasted
    /// and what it came to.
    ///
    /// Stored as a thread's link (`thread::Link`) and sent as this. Clients
    /// that do not know the kind skip it.
    Call {
        id: String,
        /// When the call started. The line keeps its place as it is rewritten.
        ts: i64,
        call_id: String,
        /// "Call", or what the closing note called it.
        title: String,
        status: CallStatus,
        /// How long it lasted, once it has ended.
        #[serde(skip_serializing_if = "Option::is_none")]
        duration_ms: Option<i64>,
        /// Once ended: what it came to, in a line, or how it ended.
        #[serde(skip_serializing_if = "Option::is_none")]
        outcome: Option<String>,
    },
    /// A thread that hangs off this one: the line a side thread, a run or a
    /// call leaves on its parent, written again under the same id as the
    /// thread goes. It is the stored model of what `Side`, `Subagent` and
    /// `Call` are drawn from, and a client that declared `threads2` is sent
    /// this for every kind in their place; one that did not is sent those.
    ///
    /// `threadKind` and not `kind`, because the event's own `kind` is `link`.
    Link {
        id: String,
        /// When the thread started. The line keeps its place as it is rewritten.
        ts: i64,
        /// The thread's key: with `threadKind`, its `ThreadId`.
        thread: String,
        thread_kind: ThreadKind,
        /// Whose thread it is. Absent from a run's marker written before links.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        persona_id: Option<String>,
        title: String,
        state: LinkState,
        /// Once closed: how it ended.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<End>,
        /// Once closed: what the thread came to, in a line.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outcome: Option<String>,
        /// When it closed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<i64>,
        /// The closing note.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        /// The agent's own id for the thread's conversation, the harness that
        /// issued it and the agent account it ran on, once a turn has
        /// completed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        backend_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account_id: Option<String>,
        /// How long a run ran, once it has stopped.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        elapsed_ms: Option<i64>,
        /// The teammate that opened the thread, when one did.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        opener_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        opener_name: Option<String>,
    },
    /// A chapter boundary: one working context of the agent, marked in the
    /// tape it belongs to. Written once when the chapter opens and superseded
    /// by id when it closes, carrying what the next chapter needs to know.
    /// `sessionId` is the agent's own memory of this stretch, so a chapter
    /// can be reopened rather than merely summarised. `accountId` is the
    /// account that memory lives under, absent for the default: a chapter
    /// reopens only on the account it ran on.
    Chapter {
        id: String,
        ts: i64,
        backend_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        account_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        ended_at: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// The handoff note: goal, outcome, open loops, decisions, files.
        #[serde(skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<ChapterStatus>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tags: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        closed_by: Option<ChapterClose>,
        /// Set on a chapter that reopened an earlier one's context.
        #[serde(skip_serializing_if = "Option::is_none")]
        resumed_from: Option<String>,
    },
}

/// A file that rides with a message, either way.
///
/// Handed to a teammate, everything is a path, pasted images included: a
/// pasted screenshot is written into the persona's `attachments` directory
/// before it is ever attached. That keeps one shape on the wire, keeps base64
/// out of the transcript on disk, and means an attachment can still be opened
/// months later from the record of the conversation that mentioned it.
/// For phone readback the desk separately retains supported user images at
/// send time; `file.read` selects that copy by message id and attachment index,
/// never by `path`. Older messages without a retained copy cannot be read back.
///
/// Sent by a teammate with `send_file`, the file is the desk's own copy,
/// under `files/` in the data directory, and `path` is where that copy is. A
/// phone never sees that path mean anything: it reads the file with
/// `file.read`, by the message's id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct Attachment {
    /// Whether call work created this file (false means edited).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<bool>,
    /// Images can be inlined for an agent that takes them, and are drawn in
    /// the conversation when a teammate sends one; files are linked.
    pub kind: AttachmentKind,
    /// Basename, which is all the composer and the bubble ever show.
    pub name: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    /// Bytes on disk, for the size a chip shows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<i64>,
    /// An image's size in pixels, so its place can be held before it loads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Where a teammate's file came from, in words a person reads: a path in
    /// its workspace or on its computer, or the part of the computer's screen
    /// a screenshot shows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

/// A successful file mutation reported by the work's own tool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct ChangedFile {
    pub path: String,
    pub created: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum AttachmentKind {
    Image,
    File,
}

/// A ring: an agent's mark on one of its own messages, saying "this is the
/// one". The set is closed and the theme owns the colours — an agent naming a
/// hex would be an agent doing visual design.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum RingIntent {
    Attention,
    Warning,
    Problem,
}

/// How far one message in a teammate-to-teammate thread has got.
///
/// `sent` is written when the message enters the thread — it has been accepted
/// for delivery and nothing more. `read` means the recipient's *agent* has
/// been handed it. Delivery to a machine is not the interesting fact and is
/// deliberately not a rung.
/// Which of the person's apps a message was written in: the desktop app on
/// this desk's own machine, or their phone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum Client {
    Desktop,
    Phone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum Receipt {
    Sent,
    Read,
}

/// Where a paused exchange's card has got to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum ExchangePauseStatus {
    Pending,
    Resumed,
    Stopped,
}

/// What a [`TranscriptEvent::Delivery`] answers, so a teammate juggling
/// several can tell them apart and a seat can say why a turn began.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = "contract.ts")]
pub enum DeliveryCause {
    /// A colleague answered a message this teammate sent it, or could not.
    /// `status` is `done` for an answer and `failed` for none; the words of
    /// the message and the answer are in the thread named by `threadKey`.
    Peer {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        request_id: Option<String>,
        persona_id: String,
        name: String,
        thread_key: String,
        status: PeerStatus,
        /// The start of the message this answers, clipped to a line.
        about: String,
    },
    /// Work explicitly handed to this teammate, in a work thread of its own.
    /// The result returns automatically to requestId on the sender's tape.
    Handoff {
        request_id: String,
        persona_id: String,
        name: String,
        thread_key: String,
        /// The start of the message, clipped to a line.
        about: String,
    },
    /// The person answered a `request_human` card the teammate did not wait
    /// on, or its lifetime ran out without an answer (`expired`). `text` is the
    /// note they typed with it, or empty.
    Answer {
        action_id: String,
        status: HumanActionStatus,
        /// The start of the card's reason, clipped to a line.
        about: String,
    },
}

/// Where a delivery came from: the thread that produced it, and the request it
/// answers when it answers one. It is a field of the delivery, so the turn that
/// takes it never has to read an id to find out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct DeliveryFrom {
    /// All source threads of a combined call note. Older readers use `thread`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<ThreadId>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The thread's key: see [`ThreadId`].
    pub thread: String,
    pub kind: ThreadKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
}

impl DeliveryFrom {
    pub fn new(thread: &ThreadId, request: Option<String>) -> Self {
        Self {
            title: None,
            sources: None,
            thread: thread.key.clone(),
            kind: thread.kind,
            request,
        }
    }

    pub fn thread(&self) -> ThreadId {
        ThreadId::new(self.kind, self.thread.clone())
    }
}

/// What a firing stamps on the user event it writes.
///
/// The event's text is still the whole prompt, because debugging a schedule
/// means reading what it actually said. This is what lets the transcript show
/// one line instead — and what tells the quiet gate which job is speaking.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct ScheduledRun {
    pub job_id: String,
    pub kind: ScheduleKind,
    pub name: String,
    /// The immutable provenance captured when the scheduler queued this run.
    /// A one-shot may be tombstoned before its queued turn reaches the driver,
    /// so dispatch cannot recover this fact from the room stream.
    #[serde(default)]
    pub operator_created: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiet: Option<bool>,
}

/// `schedule` is once. `loop` is every interval until cancelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum ScheduleKind {
    Schedule,
    Loop,
}

/// One of the abilities a teammate's home agent may leave out of a clone it
/// starts, a subagent or a job, for that one task (ADR 0008 §0). A clone has
/// the home thread's whole permission set unless it is narrowed with these;
/// nothing can widen it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "contract.ts")]
pub enum Ability {
    /// The teammate's computer.
    Computer,
    /// Anything outside the working directory: the granted folders, and
    /// whole-machine reach, which becomes the working directory's.
    Folders,
    /// The app's MCP servers the teammate is granted.
    McpServers,
}

/// Work a teammate has asked Hotline to do later: a job (ADR 0008 §7).
///
/// `schedule` is once. `loop` is every `every` milliseconds until cancelled
/// or until its end. `nextAt` is the next fire, so the window can say when
/// without doing the math. Each firing is a turn on the job's own thread
/// (`ThreadId::job(id)`), never the teammate's DM; a reminder is posted to the
/// DM as it is and has no thread. Jobs live on the room stream as events of
/// kind `schedule`; that event kind is the stream's, not this `kind`, and a
/// loop is recovered from `every` being present. An ended job stays on the
/// stream with `endedAt`, so its thread is still listed; a delete is a
/// tombstone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct ScheduledJob {
    pub id: String,
    pub persona_id: String,
    pub kind: ScheduleKind,
    /// The original fire time of a one-shot, milliseconds since epoch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<i64>,
    /// The interval of a loop, in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub every: Option<i64>,
    pub prompt: String,
    /// The report policy: the job posts to the chat only what is worth
    /// telling, and a firing that finds nothing says nothing there. Absent
    /// means each firing's report is posted. Stored only when true, so "not
    /// quiet" has one representation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiet: Option<bool>,
    /// A plain reminder: the prompt is posted to the chat as it is when the
    /// job fires, and no work is done. Stored only when true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reminder: Option<bool>,
    /// The goal the job ends itself on reaching, with a final post.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// The last moment a loop fires, milliseconds since epoch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<i64>,
    /// How many more times a loop fires.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub times: Option<u32>,
    /// What the teammate left out of the job's permission set when it made
    /// it. Absent means none: the job has the home thread's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub without: Option<Vec<Ability>>,
    /// True only when the authenticated desk created this job. Jobs created by
    /// a teammate, and jobs written by older versions without provenance,
    /// require that teammate's current background-work grant at every wake.
    #[serde(default)]
    pub operator_created: bool,
    pub next_at: i64,
    pub created_at: i64,
    /// What the job last posted to the chat.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_post: Option<JobPost>,
    /// When the job ended: it fires no more, and its thread stays readable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
}

/// A job's latest post to the chat, for the listing to draw.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct JobPost {
    pub at: i64,
    /// The post's first words, clipped for a line.
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "contract.ts")]
pub enum ToolStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = "contract.ts")]
pub enum ToolOutput {
    Text {
        text: String,
    },
    Diff {
        path: String,
        /// Written as `null` rather than omitted when there was no old text,
        /// because that is what the ACP session writes and the record is
        /// whatever it wrote.
        #[ts(optional = nullable)]
        old_text: Option<String>,
        new_text: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct PermissionOption {
    pub option_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct PlanEntry {
    pub content: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
}

/// What one turn's requests cost in tokens. On Hotline Agent the input counts
/// every input token, cached or not, and the two cache counts are part of it;
/// an ACP agent's numbers are the agent's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct TokenUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<i64>,
    /// Input the provider read back from its prompt cache.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<i64>,
    /// Input the provider wrote to its prompt cache for the next request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum NoticeLevel {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum PullStatus {
    Pulling,
    Done,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum HumanActionStatus {
    Pending,
    Done,
    Dismissed,
    Expired,
    /// The teammate took the card back with `withdraw_request`: it no longer
    /// needs the person, so nothing goes back to it.
    Withdrawn,
}

/// Where a passkey card stands: waiting for the person, answered either
/// way, or gone — the request left with its page, the arming ran out or
/// was cancelled, or the computer stopped — before anyone answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum PasskeyAskStatus {
    Pending,
    Approved,
    Denied,
    Expired,
}

/// What the person can say to a waiting `request_human` card.
///
/// The tape still writes `done` or `dismissed` — `declined` is this
/// command's word for the afterlife the previous edition called dismissed, so
/// an imported tape still reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum HumanAnswer {
    Done,
    Declined,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum PeerRole {
    Caller,
    Target,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum PeerStatus {
    Open,
    Done,
    Waiting,
    Failed,
}

/// Where a subagent's run has got to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum SubagentStatus {
    Running,
    Done,
    Failed,
    Cancelled,
}

/// Whether a call is still going.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum CallStatus {
    Live,
    Ended,
}

/// Where a thread stands, in every kind's words: an agent is running it, it is
/// open with none, or it is over (and `end` says how).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum LinkState {
    Live,
    Parked,
    Closed,
}

/// Whether a side thread is still going.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum SideStatus {
    /// An agent is running it.
    Live,
    /// Still open, with its agent let go of: nobody spoke in it for a few
    /// hours, or the desk restarted. Saying something in it brings it back.
    Parked,
    /// Ended on purpose. Read-only until it is continued.
    Archived,
}

/// Who or what ended a side thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum SideEnd {
    /// The teammate said it was done (`archive_thread`).
    Agent,
    /// The person pressed Archive.
    Person,
    /// Nobody spoke in it for a few hours. Threads written before parking
    /// existed carry it; now such a thread is parked, not archived.
    Idle,
    /// The teammate was stopped, its policy changed, it was removed, or the
    /// desk restarted, so the thread's agent is gone.
    Stopped,
}

/// One side thread, as a list of them is read: live, parked and archived.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct SideThreadSummary {
    pub side_id: String,
    pub persona_id: String,
    pub title: String,
    pub status: SideStatus,
    pub started_at: i64,
    /// The newest line in the thread.
    pub last_at: i64,
    /// A turn of the thread is running right now. Always false once archived.
    pub working: bool,
    /// A permission card in the thread is waiting for an answer.
    pub waiting: bool,
    /// The newest thing said in the thread, in a line: the teammate's last
    /// words, or what the person asked when it has said none yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_by: Option<SideEnd>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<i64>,
    /// The teammate that opened it, when one did: "from Mack" on the row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opened_by: Option<SideOpener>,
}

/// A teammate that opened a work thread on another by handing it work.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct SideOpener {
    pub persona_id: String,
    pub name: String,
}

/// An outside MCP agent holding a seat in this room, rather than a teammate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum PeerSeat {
    Client,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export, export_to = "contract.ts")]
pub enum ChapterStatus {
    InProgress,
    Done,
}

/// What ended a chapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum ChapterClose {
    Idle,
    User,
    Agent,
    Resume,
}
