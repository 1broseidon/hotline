//! The line a thread leaves on its parent, and where a delivery came from.
//!
//! A thread that hangs off another writes one `link` event on it, and writes
//! the same line again, under the same id, as the thread goes: started,
//! parked, closed with a note. The same line heads the thread's own stream, so
//! the thread says what it is to whoever opens it, and it is where a side
//! thread keeps the session its agent can be reopened from.
//!
//! ```json
//! {"kind": "link", "id": "link:side:7f3", "ts": 1760000000000,
//!  "thread": "7f3", "threadKind": "side", "personaId": "ada",
//!  "title": "Fix the CI badge", "state": "closed", "end": "agent",
//!  "outcome": "It was the cache.", "at": 1760000900000}
//! ```
//!
//! `threadKind` and not `kind`, because the event's own `kind` is `link`.
//!
//! A link is the stored model and not what a client is sent. Phones on
//! current builds draw `side` and `subagent` markers and nothing else of a
//! thread, so [`Link::wire`] turns a link back into the marker its kind has
//! always had on its way out, and the markers already on disk are read by
//! [`Link::read`] as the links they stand for. A tape can hold both: an old
//! marker is rewritten as a link under its own id the next time its thread
//! changes, so it is replaced in place and never doubled.

use super::{AgentBinding, End, ThreadId, ThreadKind, ThreadState};
use crate::contract::{SideEnd, SideStatus, SubagentStatus, TranscriptEvent};
use serde_json::{Value, json};

/// The `kind` of a link line.
pub const KIND: &str = "link";

/// One thread, as the line on its parent says it stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// What the line is rewritten by. A marker written before links existed
    /// keeps the id it was written under.
    pub id: String,
    /// When the thread started. The line keeps its place as it is rewritten.
    pub ts: i64,
    pub thread: ThreadId,
    /// Whose thread it is. Absent from a run's marker written before links.
    pub persona_id: Option<String>,
    pub title: String,
    pub state: ThreadState,
    /// What the thread came to, in a line, once it has closed.
    pub outcome: Option<String>,
    /// When it closed.
    pub at: Option<i64>,
    /// The closing note.
    pub note: Option<String>,
    /// The agent's own memory of the thread, once a turn has completed.
    pub binding: Option<AgentBinding>,
    /// How long a run ran, once it has stopped.
    pub elapsed_ms: Option<i64>,
}

impl End {
    fn name(self) -> &'static str {
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

    fn parse(name: &str) -> Option<Self> {
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

impl Link {
    /// The id a new thread's link is written under.
    pub fn fresh_id(thread: &ThreadId) -> String {
        format!("{KIND}:{}:{}", thread.kind.name(), thread.key)
    }

    /// The link of `thread` among the lines of a stream.
    pub fn find(events: &[Value], thread: &ThreadId) -> Option<Self> {
        events
            .iter()
            .filter_map(Self::read)
            .find(|link| link.thread == *thread)
    }

    /// The line as it is stored.
    pub fn event(&self) -> Value {
        let mut line = json!({
            "kind": KIND,
            "id": self.id,
            "ts": self.ts,
            "thread": self.thread.key,
            "threadKind": self.thread.kind.name(),
            "title": self.title,
            "state": match self.state {
                ThreadState::Live => "live",
                ThreadState::Parked => "parked",
                ThreadState::Closed(_) => "closed",
            },
        });
        let fields = line.as_object_mut().expect("a link is an object");
        if let ThreadState::Closed(end) = self.state {
            fields.insert("end".into(), end.name().into());
        }
        let mut put = |name: &str, value: Option<Value>| {
            if let Some(value) = value {
                fields.insert(name.into(), value);
            }
        };
        put("personaId", self.persona_id.clone().map(Value::from));
        put("outcome", self.outcome.clone().map(Value::from));
        put("at", self.at.map(Value::from));
        put("note", self.note.clone().map(Value::from));
        put(
            "sessionId",
            self.binding
                .as_ref()
                .map(|binding| Value::from(binding.session_id.clone())),
        );
        put(
            "backendId",
            self.binding
                .as_ref()
                .map(|binding| Value::from(binding.backend_id.clone())),
        );
        put("elapsedMs", self.elapsed_ms.map(Value::from));
        line
    }

    /// The link a line stands for: a `link`, or a `side` or `subagent` marker
    /// written before links existed. Anything else is not one.
    pub fn read(event: &Value) -> Option<Self> {
        match event.get("kind")?.as_str()? {
            KIND => Self::read_link(event),
            "side" | "subagent" => Self::read_marker(event),
            _ => None,
        }
    }

    fn read_link(event: &Value) -> Option<Self> {
        let text = |name: &str| event.get(name).and_then(Value::as_str).map(str::to_string);
        let state = match event.get("state")?.as_str()? {
            "live" => ThreadState::Live,
            "parked" => ThreadState::Parked,
            "closed" => ThreadState::Closed(End::parse(event.get("end")?.as_str()?)?),
            _ => return None,
        };
        Some(Self {
            id: text("id")?,
            ts: event.get("ts")?.as_i64()?,
            thread: ThreadId::new(
                ThreadKind::parse(event.get("threadKind")?.as_str()?)?,
                text("thread")?,
            ),
            persona_id: text("personaId"),
            title: text("title")?,
            state,
            outcome: text("outcome"),
            at: event.get("at").and_then(Value::as_i64),
            note: text("note"),
            binding: text("sessionId")
                .zip(text("backendId"))
                .map(|(session_id, backend_id)| AgentBinding {
                    backend_id,
                    session_id,
                }),
            elapsed_ms: event.get("elapsedMs").and_then(Value::as_i64),
        })
    }

    fn read_marker(event: &Value) -> Option<Self> {
        match serde_json::from_value::<TranscriptEvent>(event.clone()).ok()? {
            TranscriptEvent::Side {
                id,
                ts,
                side_id,
                persona_id,
                title,
                status,
                result,
                archived_by,
                archived_at,
                note,
                session_id,
                backend_id,
            } => Some(Self {
                id,
                ts,
                thread: ThreadId::side(side_id),
                persona_id: Some(persona_id),
                title,
                state: match status {
                    SideStatus::Live => ThreadState::Live,
                    SideStatus::Parked => ThreadState::Parked,
                    SideStatus::Archived => {
                        ThreadState::Closed(archived_by.map_or(End::Person, End::from))
                    }
                },
                outcome: result,
                at: archived_at,
                note,
                binding: session_id
                    .zip(backend_id)
                    .map(|(session_id, backend_id)| AgentBinding {
                        backend_id,
                        session_id,
                    }),
                elapsed_ms: None,
            }),
            TranscriptEvent::Subagent {
                id,
                ts,
                run_id,
                title,
                status,
                elapsed_ms,
            } => Some(Self {
                id,
                ts,
                thread: ThreadId::run(run_id),
                persona_id: None,
                title,
                state: match status {
                    SubagentStatus::Running => ThreadState::Live,
                    SubagentStatus::Done => ThreadState::Closed(End::Done),
                    SubagentStatus::Failed => ThreadState::Closed(End::Failed),
                    SubagentStatus::Cancelled => ThreadState::Closed(End::Cancelled),
                },
                outcome: None,
                at: None,
                note: None,
                binding: None,
                elapsed_ms,
            }),
            _ => None,
        }
    }

    /// The marker this link has always been sent to clients as, for the kinds
    /// that had one. None for a kind whose clients have never drawn one.
    fn marker(&self) -> Option<TranscriptEvent> {
        match self.thread.kind {
            ThreadKind::Side => Some(TranscriptEvent::Side {
                id: self.id.clone(),
                ts: self.ts,
                side_id: self.thread.key.clone(),
                persona_id: self.persona_id.clone()?,
                title: self.title.clone(),
                status: match self.state {
                    ThreadState::Live => SideStatus::Live,
                    ThreadState::Parked => SideStatus::Parked,
                    ThreadState::Closed(_) => SideStatus::Archived,
                },
                result: self.outcome.clone(),
                archived_by: match self.state {
                    ThreadState::Closed(End::Agent) => Some(SideEnd::Agent),
                    ThreadState::Closed(End::Idle) => Some(SideEnd::Idle),
                    ThreadState::Closed(End::Stopped) => Some(SideEnd::Stopped),
                    ThreadState::Closed(_) => Some(SideEnd::Person),
                    _ => None,
                },
                archived_at: self.at,
                note: self.note.clone(),
                session_id: self
                    .binding
                    .as_ref()
                    .map(|binding| binding.session_id.clone()),
                backend_id: self
                    .binding
                    .as_ref()
                    .map(|binding| binding.backend_id.clone()),
            }),
            ThreadKind::Run => Some(TranscriptEvent::Subagent {
                id: self.id.clone(),
                ts: self.ts,
                run_id: self.thread.key.clone(),
                title: self.title.clone(),
                status: match self.state {
                    ThreadState::Live | ThreadState::Parked => SubagentStatus::Running,
                    ThreadState::Closed(End::Done) => SubagentStatus::Done,
                    ThreadState::Closed(End::Failed) => SubagentStatus::Failed,
                    ThreadState::Closed(_) => SubagentStatus::Cancelled,
                },
                elapsed_ms: self.elapsed_ms,
            }),
            ThreadKind::Dm | ThreadKind::Pair | ThreadKind::Call => None,
        }
    }

    /// An event as a client is sent it. A link becomes the marker its kind
    /// has always had, so a phone or a window that does not know links draws
    /// the thread as before; everything else is passed on as it is.
    pub fn wire(event: Value) -> Value {
        if event.get("kind").and_then(Value::as_str) != Some(KIND) {
            return event;
        }
        Self::read_link(&event)
            .and_then(|link| link.marker())
            .and_then(|marker| serde_json::to_value(marker).ok())
            .unwrap_or(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed_side() -> Link {
        Link {
            id: Link::fresh_id(&ThreadId::side("s1")),
            ts: 10,
            thread: ThreadId::side("s1"),
            persona_id: Some("ada".into()),
            title: "Fix the badge".into(),
            state: ThreadState::Closed(End::Agent),
            outcome: Some("It was the cache.".into()),
            at: Some(20),
            note: Some("Goal: the badge.".into()),
            binding: Some(AgentBinding {
                backend_id: "claude".into(),
                session_id: "sess".into(),
            }),
            elapsed_ms: None,
        }
    }

    #[test]
    fn a_link_reads_back_as_it_was_written() {
        let link = closed_side();
        assert_eq!(Link::read(&link.event()), Some(link));
        let run = Link {
            id: Link::fresh_id(&ThreadId::run("r1")),
            ts: 1,
            thread: ThreadId::run("r1"),
            persona_id: Some("ada".into()),
            title: "Look it up".into(),
            state: ThreadState::Closed(End::Failed),
            outcome: None,
            at: None,
            note: None,
            binding: None,
            elapsed_ms: Some(40),
        };
        assert_eq!(Link::read(&run.event()), Some(run));
    }

    #[test]
    fn a_marker_written_before_links_reads_as_the_link_it_stands_for() {
        let old = json!({
            "kind": "side", "id": "side:s1", "ts": 10, "sideId": "s1",
            "personaId": "ada", "title": "Fix the badge", "status": "archived",
            "result": "It was the cache.", "archivedBy": "agent", "archivedAt": 20,
            "note": "Goal: the badge.", "sessionId": "sess", "backendId": "claude"
        });
        let read = Link::read(&old).unwrap();
        assert_eq!(read.id, "side:s1", "it is rewritten under the id it has");
        assert_eq!(
            Link {
                id: Link::fresh_id(&ThreadId::side("s1")),
                ..read
            },
            closed_side()
        );
        let run = Link::read(&json!({
            "kind": "subagent", "id": "subagent:r1", "ts": 1, "runId": "r1",
            "title": "Look it up", "status": "cancelled"
        }))
        .unwrap();
        assert_eq!(run.thread, ThreadId::run("r1"));
        assert_eq!(run.state, ThreadState::Closed(End::Cancelled));
    }

    #[test]
    fn a_client_is_sent_the_marker_its_kind_has_always_had() {
        let sent = Link::wire(closed_side().event());
        assert_eq!(sent["kind"], "side");
        assert_eq!(sent["id"], "link:side:s1");
        assert_eq!(sent["sideId"], "s1");
        assert_eq!(sent["status"], "archived");
        assert_eq!(sent["archivedBy"], "agent");
        assert_eq!(sent["result"], "It was the cache.");
        assert_eq!(sent["sessionId"], "sess");
        let parked = Link {
            state: ThreadState::Parked,
            ..closed_side()
        };
        assert_eq!(Link::wire(parked.event())["status"], "parked");
        let run = Link::wire(
            Link {
                thread: ThreadId::run("r1"),
                state: ThreadState::Closed(End::Done),
                elapsed_ms: Some(5),
                ..closed_side()
            }
            .event(),
        );
        assert_eq!(run["kind"], "subagent");
        assert_eq!(run["runId"], "r1");
        assert_eq!(run["status"], "done");
        assert_eq!(run["elapsedMs"], 5);
        let said = json!({"kind": "agent", "id": "a", "ts": 1, "text": "hi"});
        assert_eq!(Link::wire(said.clone()), said);
        let old = json!({"kind": "side", "id": "side:s1"});
        assert_eq!(Link::wire(old.clone()), old);
    }
}
