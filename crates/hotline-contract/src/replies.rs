//! What the room answers with: thread answers and summaries, targets, schedules,
//! the roster, and what an import did.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::*;

/// The answer to a card in a thread, by the kind of card it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub enum ThreadAnswer {
    Credential(CredentialAnswer),
    /// An option on a permission card.
    Permission {
        request_id: String,
        option_id: String,
    },
    /// A `request_human` card: done, or declined with a note the teammate
    /// hears word for word.
    Human {
        action_id: String,
        status: HumanAnswer,
        #[serde(default)]
        note: Option<String>,
    },
}

/// One thread as a list of every kind reads: what a person scanning them
/// needs, whichever kind each is. The kind is `thread.kind`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct ThreadSummary {
    pub thread: ThreadId,
    /// The teammate whose thread it is. A pair is listed with the first of its
    /// two, and `with` names the other; a pair with a teammate on a linked
    /// desk is listed with this desk's teammate.
    pub persona_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with_persona_id: Option<String>,
    /// What to call `with`, when it is a teammate on a linked desk ("Lou on
    /// grizzly"): it is in no roster here for the window to read its name in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with_name: Option<String>,
    /// What the thread is called: a work thread's or run's task, a call's
    /// title. A DM and a pair have none, and are named by who is in them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub state: LinkState,
    /// How a closed thread ended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<End>,
    /// The teammate that opened it, when one did: "from Mack" on the row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opener: Option<SideOpener>,
    /// The handoff it serves has ended, as its link records: what is said in
    /// it no longer reaches `opener`. Absent while it stands.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub handoff_ended: bool,
    pub started_at: i64,
    /// The newest thing in the thread: what the window counts unread against.
    pub updated_at: i64,
    /// A turn of the thread is running right now.
    pub working: bool,
    /// A card in the thread is waiting for an answer.
    pub waiting: bool,
    /// The newest thing said in the thread, in a line: the teammate's last
    /// words, or what was asked when it has said none yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    /// What a closed thread came to, in a line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

/// What a subscription is a subscription to: a stream, or a view the core
/// maintains and nobody logs.
///
/// Externally tagged, so a stream reads as the word or the pair naming it —
/// `"room"`, `{"tape": "<personaId>"}`, `{"thread": "<key>"}`, `{"run":
/// "<runId>"}`, `{"view": "roster"}`, `{"schedules": "<personaId>"}` — which
/// is the shape the window would have written by hand. `{"thread": "<key>"}`
/// is a pair's, and stays so; any thread of any kind is `{"threadId": {"kind",
/// "key"}}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum Target {
    Call(String),
    Room,
    Tape(String),
    /// A thread between two teammates, by its pair key. The name predates
    /// threads of every kind and means this for clients that know no other.
    Thread(String),
    /// Any thread, by kind and key: its own transcript and live words. For a
    /// DM it is the teammate's tape, and for a work thread, a run and a call
    /// the stream of that name.
    #[serde(rename = "threadId")]
    ThreadId(ThreadId),
    /// One subagent run's own transcript, by run id.
    Run(String),
    /// One side thread's own transcript and live words, by side id.
    Side(String),
    View(ViewName),
    /// One teammate's scheduled jobs and loops, by persona id, as
    /// `ScheduleEntry`s: the whole list on opening and the whole list again
    /// whenever it changes. The way a phone reads them, since the room stream
    /// that holds them is the desk's alone.
    Schedules(String),
}

/// One of a teammate's jobs as the schedules view shows it: what a person
/// reading the list needs, and nothing that decides what the job may do.
///
/// `name` is the job's label, the first breath of its prompt. `kind` is
/// `schedule` for once and `loop` for every `every` milliseconds. `when` is a
/// one-shot's original time, milliseconds since the epoch. `nextAt` is when
/// the desk next means to fire it, also milliseconds since the epoch, and a
/// plan rather than a promise: a desk that is closed or asleep then fires it
/// once when it can, a fire that could not start tries again a minute later,
/// and a loop counts its next interval from when a firing starts. A job's
/// firings are turns on its own thread, `{"kind": "job", "key": id}`.
/// `lastPost` is what it last posted to the chat. A job that has fired for
/// the last time, been cancelled or met its goal is still listed, with
/// `endedAt`, because its thread stays readable; `schedule.cancel` on an
/// ended job takes it off the list. `without` is what the teammate left out
/// of the job's permission set when it made it, so a person can see a job
/// is a narrowed clone; it is read here, and decided on the job.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct ScheduleEntry {
    pub id: String,
    pub persona_id: String,
    pub name: String,
    pub kind: ScheduleKind,
    /// What the teammate is asked when the job fires.
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub every: Option<i64>,
    pub next_at: i64,
    /// Present and true when the job was asked to say nothing in the chat
    /// unless it finds something worth saying.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiet: Option<bool>,
    /// Present and true for a plain reminder, which posts its prompt and
    /// does no work.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reminder: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_post: Option<JobPost>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub without: Option<Vec<Ability>>,
}

/// The views the core maintains. One so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum ViewName {
    Roster,
}

/// One row of the roster view: who the teammate is, the last thing either
/// side said, and what its session is doing.
///
/// Three sources — the room stream, the tape's tail, the live session — that
/// the window would otherwise have to join for itself on every change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct RosterEntry {
    pub persona: Persona,
    /// Absent for a teammate that has never spoken.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<Preview>,
    /// The preview's `at`, kept beside it so the window can count unread
    /// without opening every tape.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest: Option<i64>,
    /// The title of the tool still running on this tape, only while the
    /// session is thinking. Absent, not null, when there is none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    /// A card on this tape is still waiting on the person: a permission with
    /// no decision, or a `request_human` or passkey card still pending.
    ///
    /// On the row so a phone can say "Needs you" for every teammate on
    /// every desk from one roster, without opening each tape. Always
    /// written, so a phone can tell `false` from a desk that predates it.
    pub waiting: bool,
    /// Its picture is being drawn: the face shows it is on its way rather
    /// than leaving the initial looking final.
    #[serde(default)]
    pub drawing: bool,
    /// Where this teammate is pinned on the desk, 0-based, absent when it is
    /// not. On the row so a companion phone, which cannot read settings, sees
    /// the same pins as the desk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<u8>,
    /// This teammate is the desk's lead. Absent when it is not, so a client
    /// that predates the role reads the row as it always did.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub lead: bool,
    /// Subagents this teammate has running, oldest first, so the
    /// conversation can show them without scrolling back to their lines.
    /// Absent when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<RunningSubagent>>", optional)]
    pub subagents: Vec<RunningSubagent>,
    /// Side threads this teammate has live, oldest first. Absent when there
    /// are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<RunningSide>>", optional)]
    pub sides: Vec<RunningSide>,
    /// The jobs of this teammate with a turn running right now, by id: a
    /// firing, or a line the person said in the job's thread. Absent when
    /// there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<String>>", optional)]
    pub working_jobs: Vec<String>,
    /// Call threads with a turn in flight. An open call between turns is
    /// not working. Absent when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<String>>", optional)]
    pub working_calls: Vec<String>,
    pub session: SessionInfo,
}

/// A subagent still running, as its teammate's roster row lists it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct RunningSubagent {
    pub run_id: String,
    /// The short label the teammate gave the task.
    pub title: String,
    pub started_at: i64,
}

/// A side thread still live, as its teammate's roster row lists it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct RunningSide {
    pub side_id: String,
    pub title: String,
    pub started_at: i64,
    /// A turn of the thread is running right now.
    pub working: bool,
}

/// What an import did: how many of each thing came over, what it left
/// behind, and notes about things that came over with a caveat.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct Report {
    pub teammates: i64,
    pub tapes: i64,
    pub settings: i64,
    pub keys: i64,
    pub threads: i64,
    pub schedules: i64,
    pub skipped: Vec<Skipped>,
    pub notes: Vec<Skipped>,
}

/// One row the import wants the person to see: left behind (`skipped`)
/// or imported with a caveat (`notes`). Same shape so the window can
/// word the list, not the row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct Skipped {
    pub item: String,
    pub reason: String,
}
