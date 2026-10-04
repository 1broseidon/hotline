//! Reading threads out of the records each kind already keeps.
//!
//! Nothing here writes, and nothing is a new record. A DM is the teammate's
//! persona and its tape; a side thread is the marker at the head of its stream;
//! a run is its marker, and the tape that holds the line the person presses to
//! open it; a pair is its sidecar and, on the room stream, the `exchange_pair`
//! that says whether a request is being answered. The folding of those into a
//! [`Thread`] is the whole of this module.

use super::{
    AgentBinding, End, Participant, Thread, ThreadId, ThreadKind, ThreadLink, ThreadState,
};
use crate::contract::{Persona, SideEnd, SideStatus, SubagentStatus, TranscriptEvent};
use crate::log::{Log, StreamId, thread as pair_files};
use crate::paths::{sides_dir, thread_meta_path, thread_participants};
use crate::room;
use serde_json::Value;

/// Every thread in a room, listed and loaded through one door.
///
/// The state a record gives is the record's word. A side thread or run that a
/// dead process left `Live` reads as `Live` here, and the room parks it when it
/// next starts; a pair is `Live` while a request is being answered on the room
/// stream. A DM is always `Live`: whether it has an agent is the room's
/// knowledge, not the tape's, and the DM is never closed.
pub struct ThreadStore {
    log: Log,
}

impl ThreadStore {
    pub fn new(log: &Log) -> Self {
        Self { log: log.clone() }
    }

    /// One thread, or none when no record of it exists. A call has no record
    /// yet, so it is never found.
    pub fn load(&self, id: &ThreadId) -> Option<Thread> {
        match id.kind {
            ThreadKind::Dm => self.dm(&id.key),
            ThreadKind::Side => self.side(&id.key),
            ThreadKind::Pair => self.pair(&id.key),
            ThreadKind::Run => {
                let owner = self.owner_of_run(&id.key);
                self.run(&id.key, owner.as_deref())
            }
            ThreadKind::Call => None,
        }
    }

    /// The threads a teammate is in: its DM, its side threads and runs, then
    /// the pairs it is one side of. Empty for a teammate the room does not
    /// hold.
    pub fn list(&self, persona_id: &str) -> Vec<Thread> {
        let Some(dm) = self.dm(persona_id) else {
            return Vec::new();
        };
        let mut threads = vec![dm];
        threads.extend(
            self.side_ids()
                .iter()
                .filter_map(|side_id| self.side(side_id))
                .filter(|side| {
                    side.participants
                        .contains(&Participant::Persona(persona_id.into()))
                }),
        );
        threads.extend(
            self.run_ids_on_tape(persona_id)
                .iter()
                .filter_map(|run_id| self.run(run_id, Some(persona_id))),
        );
        threads.extend(
            pair_files::keys_for(self.log.root(), persona_id)
                .iter()
                .filter_map(|key| self.pair(key)),
        );
        threads
    }

    /// Every thread in the room, each once. A pair is listed with the first of
    /// its teammates, not with both.
    pub fn all(&self) -> Vec<Thread> {
        let mut threads: Vec<Thread> = Vec::new();
        for persona in room::roster(&self.log) {
            for thread in self.list(&persona.id) {
                if !threads.iter().any(|seen| seen.id == thread.id) {
                    threads.push(thread);
                }
            }
        }
        threads
    }

    fn dm(&self, persona_id: &str) -> Option<Thread> {
        let persona = self.persona(persona_id)?;
        let binding = persona
            .session_checkpoints
            .iter()
            .find(|checkpoint| checkpoint.backend_id == persona.backend_id)
            .map(|checkpoint| AgentBinding {
                backend_id: checkpoint.backend_id.clone(),
                session_id: checkpoint.session_id.clone(),
            });
        Some(Thread {
            id: ThreadId::dm(persona_id),
            parent: None,
            participants: vec![Participant::Person, Participant::Persona(persona.id)],
            state: ThreadState::Live,
            title: None,
            binding,
            note: None,
        })
    }

    fn side(&self, side_id: &str) -> Option<Thread> {
        let marker_id = Value::from(format!("side:{side_id}"));
        let marker = self
            .log
            .load(&StreamId::Side(side_id.to_string()))
            .into_iter()
            .find(|event| event.get("id") == Some(&marker_id))?;
        let TranscriptEvent::Side {
            id,
            persona_id,
            title,
            status,
            archived_by,
            note,
            session_id,
            backend_id,
            ..
        } = serde_json::from_value(marker).ok()?
        else {
            return None;
        };
        let state = match status {
            SideStatus::Live => ThreadState::Live,
            SideStatus::Parked => ThreadState::Parked,
            SideStatus::Archived => ThreadState::Closed(match archived_by {
                Some(SideEnd::Agent) => End::Agent,
                Some(SideEnd::Idle) => End::Idle,
                Some(SideEnd::Stopped) => End::Stopped,
                Some(SideEnd::Person) | None => End::Person,
            }),
        };
        Some(Thread {
            id: ThreadId::side(side_id),
            parent: Some(ThreadLink {
                thread: ThreadId::dm(&persona_id),
                event: Some(id),
            }),
            participants: vec![Participant::Person, Participant::Persona(persona_id)],
            state,
            title: Some(title),
            binding: session_id
                .zip(backend_id)
                .map(|(session_id, backend_id)| AgentBinding {
                    backend_id,
                    session_id,
                }),
            note,
        })
    }

    /// A run, from the marker at the head of its own stream. The marker does
    /// not say whose run it was; the teammate's tape does, so the owner is
    /// handed in by whoever found it.
    fn run(&self, run_id: &str, owner: Option<&str>) -> Option<Thread> {
        let marker_id = Value::from(format!("subagent:{run_id}"));
        let marker = self
            .log
            .load(&StreamId::Run(run_id.to_string()))
            .into_iter()
            .find(|event| event.get("id") == Some(&marker_id))?;
        let TranscriptEvent::Subagent {
            id, title, status, ..
        } = serde_json::from_value(marker).ok()?
        else {
            return None;
        };
        let state = match status {
            SubagentStatus::Running => ThreadState::Live,
            SubagentStatus::Done => ThreadState::Closed(End::Done),
            SubagentStatus::Failed => ThreadState::Closed(End::Failed),
            SubagentStatus::Cancelled => ThreadState::Closed(End::Cancelled),
        };
        Some(Thread {
            id: ThreadId::run(run_id),
            parent: owner.map(|persona_id| ThreadLink {
                thread: ThreadId::dm(persona_id),
                event: Some(id),
            }),
            participants: owner
                .map(|persona_id| Participant::Persona(persona_id.to_string()))
                .into_iter()
                .collect(),
            state,
            title: Some(title),
            binding: None,
            note: None,
        })
    }

    /// A pair, from its sidecar's key. It is `Live` while the room stream
    /// holds a request of it that an agent is answering, and `Parked` the rest
    /// of the time: a pair's agent lasts one turn.
    fn pair(&self, key: &str) -> Option<Thread> {
        let (a, b) = thread_participants(key)?;
        if !thread_meta_path(self.log.root(), key)?.exists() {
            return None;
        }
        let answering = self.log.load(&StreamId::Room).iter().any(|event| {
            event.get("kind").and_then(Value::as_str) == Some("exchange_pair")
                && event.get("id").and_then(Value::as_str) == Some(key)
                && crate::session::exchanges::answering(event)
        });
        Some(Thread {
            id: ThreadId::pair(key),
            parent: None,
            participants: vec![
                Participant::Persona(a.to_string()),
                Participant::Persona(b.to_string()),
            ],
            state: if answering {
                ThreadState::Live
            } else {
                ThreadState::Parked
            },
            title: None,
            binding: None,
            note: None,
        })
    }

    fn persona(&self, persona_id: &str) -> Option<Persona> {
        room::roster(&self.log)
            .into_iter()
            .find(|persona| persona.id == persona_id)
    }

    /// The side threads on disk, by id.
    fn side_ids(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(sides_dir(self.log.root())) else {
            return Vec::new();
        };
        let mut ids: Vec<String> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                name.strip_suffix(".jsonl").map(str::to_string)
            })
            .collect();
        ids.sort();
        ids
    }

    /// The runs a teammate's tape carries a marker for, in the order they
    /// started.
    fn run_ids_on_tape(&self, persona_id: &str) -> Vec<String> {
        self.log
            .load(&StreamId::Tape(persona_id.to_string()))
            .iter()
            .filter(|event| event.get("kind").and_then(Value::as_str) == Some("subagent"))
            .filter_map(|event| {
                event
                    .get("runId")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect()
    }

    fn owner_of_run(&self, run_id: &str) -> Option<String> {
        room::roster(&self.log)
            .into_iter()
            .find(|persona| {
                self.run_ids_on_tape(&persona.id)
                    .iter()
                    .any(|id| id == run_id)
            })
            .map(|persona| persona.id)
    }
}
