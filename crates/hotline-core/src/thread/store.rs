//! Reading threads out of the records each kind already keeps.
//!
//! Nothing here writes, and nothing is a new record. A DM is the teammate's
//! persona and its tape; a side thread is the link at the head of its stream;
//! a run is its link, and the tape that holds the line the person presses to
//! open it; a call is its link too, at the head of its own stream, and so is
//! a job, whose record on the room stream says it is the teammate's; a pair is
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
            ThreadKind::Job => self.job(&id.key),
        }
    }

    /// The threads a teammate is in: its DM, its side threads, runs, calls
    /// and jobs, then the pairs it is one side of. Empty for a teammate the
    /// room does not hold.
    pub fn list(&self, persona_id: &str) -> Vec<Thread> {
        self.list_from(Some(persona_id), &self.log.load(&StreamId::Room))
    }

    /// Each source is read once for the selected teammates, in roster order.
    pub(crate) fn list_from(&self, selected: Option<&str>, events: &[Value]) -> Vec<Thread> {
        let personas: Vec<_> = room::personas(events)
            .into_iter()
            .filter(|p| selected.is_none_or(|id| p.id == id))
            .collect();
        if personas.is_empty() {
            return Vec::new();
        }
        let sides: Vec<_> = self
            .side_ids()
            .iter()
            .filter_map(|id| self.side(id))
            .collect();
        let jobs = room::jobs(events);
        let pairs: Vec<_> = pair_files::list_all_keys(self.log.root())
            .iter()
            .filter_map(|key| self.pair_from(key, events))
            .collect();
        let mut threads = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for persona in personas {
            let id = persona.id.clone();
            let mut own = vec![Self::dm_from(persona)];
            own.extend(
                sides
                    .iter()
                    .filter(|side| {
                        side.participants
                            .contains(&Participant::Persona(id.clone()))
                    })
                    .cloned(),
            );
            let links: Vec<_> = self
                .log
                .load(&StreamId::Tape(id.clone()))
                .iter()
                .filter_map(Link::read)
                .collect();
            own.extend(
                links
                    .iter()
                    .filter(|link| link.thread.kind == ThreadKind::Run)
                    .filter_map(|link| self.run(&link.thread.key, Some(&id))),
            );
            own.extend(
                links
                    .iter()
                    .filter(|link| link.thread.kind == ThreadKind::Call)
                    .filter_map(|link| self.call(&link.thread.key)),
            );
            own.extend(
                jobs.iter()
                    .filter(|job| job.persona_id == id)
                    .filter_map(|job| self.job(&job.id)),
            );
            own.extend(
                pairs
                    .iter()
                    .filter(|pair| {
                        pair.participants
                            .contains(&Participant::Persona(id.clone()))
                    })
                    .cloned(),
            );
            threads.extend(
                own.into_iter()
                    .filter(|thread| seen.insert(thread.id.clone())),
            );
        }
        threads
    }

    fn dm(&self, persona_id: &str) -> Option<Thread> {
        Some(Self::dm_from(self.persona(persona_id)?))
    }

    fn dm_from(persona: Persona) -> Thread {
        let binding = persona
            .session_checkpoints
            .iter()
            .find(|checkpoint| checkpoint.backend_id == persona.backend_id)
            .map(|checkpoint| AgentBinding {
                backend_id: checkpoint.backend_id.clone(),
                session_id: checkpoint.session_id.clone(),
            });
        Thread {
            id: ThreadId::dm(&persona.id),
            parent: None,
            participants: vec![Participant::Person, Participant::Persona(persona.id)],
            state: ThreadState::Live,
            title: None,
            binding,
            note: None,
        }
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
            // A teammate that handed the work over is in the thread too: the
            // result goes back to it.
            participants: [Participant::Person, Participant::Persona(persona_id)]
                .into_iter()
                .chain(
                    link.opener
                        .map(|opener| Participant::Persona(opener.persona_id)),
                )
                .collect(),
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

    /// A job's thread, from the link at the head of its own stream, which
    /// its first firing writes. The person and the teammate are in it; its
    /// parent is the teammate's DM.
    fn job(&self, job_id: &str) -> Option<Thread> {
        let link = self.link(&ThreadId::job(job_id))?;
        let persona_id = link.persona_id?;
        Some(Thread {
            id: link.thread,
            parent: Some(ThreadLink {
                thread: ThreadId::dm(&persona_id),
                event: None,
            }),
            participants: vec![Participant::Person, Participant::Persona(persona_id)],
            state: link.state,
            title: Some(link.title),
            binding: link.binding,
            note: link.note,
        })
    }

    /// The link at the head of a thread's own stream, as last written.
    fn link(&self, thread: &ThreadId) -> Option<Link> {
        self.log.thread_link(thread)
    }

    /// A pair, from its sidecar's key. The pair's thread has no agent of its
    /// own: it is `Live` while the room stream holds a request of it running
    /// or waiting on the person, and `Parked` the rest of the time.
    fn pair(&self, key: &str) -> Option<Thread> {
        self.pair_from(key, &self.log.load(&StreamId::Room))
    }

    fn pair_from(&self, key: &str, events: &[Value]) -> Option<Thread> {
        let (a, b) = thread_participants(key)?;
        if !thread_meta_path(self.log.root(), key)?.exists() {
            return None;
        }
        let answering = events.iter().any(|event| {
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
