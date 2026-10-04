//! Reading threads out of the records each kind already keeps.
//!
//! Nothing here writes, and nothing is a new record. A DM is the teammate's
//! persona and its tape; a side thread is the link at the head of its stream;
//! a run is its link, and the tape that holds the line the person presses to
//! open it; a call is its link too, at the head of its own stream; a pair is
//! its sidecar and, on the room stream, the `exchange_pair` that says whether a
//! request is being answered. The folding of those into a [`Thread`] is the
//! whole of this module.

use super::{
    AgentBinding, Link, Participant, Thread, ThreadId, ThreadKind, ThreadLink, ThreadState,
};
use crate::contract::Persona;
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

    /// One thread, or none when no record of it exists.
    pub fn load(&self, id: &ThreadId) -> Option<Thread> {
        match id.kind {
            ThreadKind::Dm => self.dm(&id.key),
            ThreadKind::Side => self.side(&id.key),
            ThreadKind::Pair => self.pair(&id.key),
            ThreadKind::Run => {
                let owner = self.owner_of_run(&id.key);
                self.run(&id.key, owner.as_deref())
            }
            ThreadKind::Call => self.call(&id.key),
        }
    }

    /// The threads a teammate is in: its DM, its side threads, runs and calls, then
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
            self.call_ids_on_tape(persona_id)
                .iter()
                .filter_map(|call_id| self.call(call_id)),
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
        let link = self.link(&ThreadId::side(side_id))?;
        let persona_id = link.persona_id?;
        Some(Thread {
            id: link.thread,
            parent: Some(ThreadLink {
                thread: ThreadId::dm(&persona_id),
                event: Some(link.id),
            }),
            participants: vec![Participant::Person, Participant::Persona(persona_id)],
            state: link.state,
            title: Some(link.title),
            binding: link.binding,
            note: link.note,
        })
    }

    /// A run, from the link at the head of its own stream. A link written
    /// before links existed does not say whose run it was; the teammate's tape
    /// does, so the owner is handed in by whoever found it.
    fn run(&self, run_id: &str, owner: Option<&str>) -> Option<Thread> {
        let link = self.link(&ThreadId::run(run_id))?;
        let owner = link.persona_id.as_deref().or(owner);
        Some(Thread {
            id: link.thread.clone(),
            parent: owner.map(|persona_id| ThreadLink {
                thread: ThreadId::dm(persona_id),
                event: Some(link.id.clone()),
            }),
            participants: owner
                .map(|persona_id| Participant::Persona(persona_id.to_string()))
                .into_iter()
                .collect(),
            state: link.state,
            title: Some(link.title),
            binding: None,
            note: None,
        })
    }

    /// A call, from the link at the head of its own stream. A call is the
    /// person and a teammate's voice; its parent is the teammate's DM.
    fn call(&self, call_id: &str) -> Option<Thread> {
        let link = self.link(&ThreadId::call(call_id))?;
        let persona_id = link.persona_id?;
        Some(Thread {
            id: link.thread,
            parent: Some(ThreadLink {
                thread: ThreadId::dm(&persona_id),
                event: Some(link.id),
            }),
            participants: vec![Participant::Person, Participant::Voice(persona_id)],
            state: link.state,
            title: Some(link.title),
            binding: None,
            note: link.note,
        })
    }

    /// The link at the head of a thread's own stream, as last written.
    fn link(&self, thread: &ThreadId) -> Option<Link> {
        Link::find(&self.log.load(&thread.stream()?), thread)
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
            .filter_map(Link::read)
            .filter(|link| link.thread.kind == ThreadKind::Run)
            .map(|link| link.thread.key)
            .collect()
    }

    /// The calls a teammate's tape carries a link for, in the order they
    /// started.
    fn call_ids_on_tape(&self, persona_id: &str) -> Vec<String> {
        self.log
            .load(&StreamId::Tape(persona_id.to_string()))
            .iter()
            .filter_map(Link::read)
            .filter(|link| link.thread.kind == ThreadKind::Call)
            .map(|link| link.thread.key)
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
