//! `thread.*`: one set of commands over every kind of thread.
//!
//! A command names a [`ThreadId`], and what it does is read from the kind: the
//! live handle behind the room (a teammate's session, a work thread) for the
//! verbs that act, the [`ThreadStore`] for the ones that list. The older
//! commands (`session.prompt`, `side.prompt`, `side.cancel`, `side.archive`,
//! the three `answer_permission`s, `human.answer`, `tape.page`) are these
//! handlers under the name they had. The ones that answer a kind's own summary
//! (`side.start`, `side.continue`, `side.list`, `peers.list`) call the same
//! room methods and keep the shape they always answered.
//!
//! Which kinds a verb applies to is what the kind is, said once here: the
//! person speaks in a DM and in a work thread, a pair is two teammates' and a
//! run and a call have their own surfaces, and a card is answered where the
//! kind's [`Policy`] says someone answers it.

use super::RoomHandle;
use crate::contract::{
    Attachment, LinkState, Persona, SessionState, SideOpener, ThreadAnswer, ThreadSummary,
};
use crate::log::Log;
use crate::paths::thread_participants;
use crate::store::previews;
use crate::thread::{
    Answer, Link, Participant, Policy, Thread, ThreadId, ThreadKind, ThreadState, ThreadStore,
};
use serde_json::{Value, json};
use std::sync::Arc;

/// Why a verb does not apply to a kind: the sentence for the person, naming
/// where the thing is done instead.
fn not_here(thread: &ThreadId, what: &str) -> String {
    let kind = match thread.kind {
        ThreadKind::Dm => "the main conversation",
        ThreadKind::Side => "a work thread",
        ThreadKind::Pair => "a thread between two teammates",
        ThreadKind::Call => "a call",
        ThreadKind::Run => "a subagent's run",
    };
    format!("{what} does not apply to {kind}.")
}

/// Says something in a thread. The person speaks in the main conversation and
/// in a work thread; a parked work thread is brought back first.
pub(super) async fn prompt(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    thread: &ThreadId,
    text: &str,
    reply_to: Option<String>,
    attachments: Option<Vec<Attachment>>,
) -> Result<Value, String> {
    match thread.kind {
        ThreadKind::Dm => {
            room.prompt(&thread.key, text, reply_to, attachments)
                .await?;
            if let Ok(persona) = super::commands::living(log, &thread.key) {
                super::commands::remember_model(log, room, &persona).await?;
            }
            Ok(Value::Null)
        }
        ThreadKind::Side => {
            room.side_prompt(&thread.key, text, attachments).await?;
            Ok(Value::Null)
        }
        _ => Err(not_here(thread, "Speaking")),
    }
}

/// Stops the turn in flight. For a pair that is the automatic exchange between
/// its two teammates.
pub(super) fn cancel(room: &Arc<dyn RoomHandle>, thread: &ThreadId) -> Result<Value, String> {
    match thread.kind {
        ThreadKind::Dm => room.cancel(&thread.key)?,
        ThreadKind::Side => room.side_cancel(&thread.key)?,
        ThreadKind::Pair => {
            let (a, b) = pair_of(thread)?;
            room.stop_exchange(a, b)?;
        }
        _ => return Err(not_here(thread, "Cancelling")),
    }
    Ok(Value::Null)
}

/// Lets go of a work thread's agent and keeps the thread open.
pub(super) fn park(room: &Arc<dyn RoomHandle>, thread: &ThreadId) -> Result<Value, String> {
    match thread.kind {
        ThreadKind::Side => room.side_park(&thread.key)?,
        _ => return Err(not_here(thread, "Parking")),
    }
    Ok(Value::Null)
}

/// Ends a work thread, keeping its transcript.
pub(super) fn close(room: &Arc<dyn RoomHandle>, thread: &ThreadId) -> Result<Value, String> {
    match thread.kind {
        ThreadKind::Side => room.side_archive(&thread.key)?,
        _ => return Err(not_here(thread, "Closing")),
    }
    Ok(Value::Null)
}

/// Brings a parked or closed work thread back, or lets a paused exchange go
/// on, and answers the thread as it now stands.
pub(super) async fn resume(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    thread: &ThreadId,
) -> Result<Value, String> {
    match thread.kind {
        ThreadKind::Side => {
            room.side_continue(&thread.key).await?;
        }
        ThreadKind::Pair => {
            let (a, b) = pair_of(thread)?;
            room.resume_exchange(a, b)?;
        }
        _ => return Err(not_here(thread, "Continuing")),
    }
    summary_of(log, room, thread)
}

/// Opens a work thread with a teammate, already running its first turn.
pub(super) async fn open(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    persona_id: &str,
    text: &str,
) -> Result<Value, String> {
    let started = room.side_start(persona_id, text).await?;
    summary_of(log, room, &ThreadId::side(started.side_id))
}

/// Answers a card raised in a thread, by the kind's policy: nobody answers one
/// in a run or a call, and a pair's is a permission the owner answers. A work
/// thread's request for the person is answered through its teammate, whose
/// cards it holds along with its work threads'.
pub(super) async fn answer(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    thread: &ThreadId,
    answer: ThreadAnswer,
) -> Result<Value, String> {
    if Policy::of(thread.kind).answer == Answer::Nobody {
        return Err(not_here(thread, "Answering a card"));
    }
    match (thread.kind, answer) {
        (
            ThreadKind::Dm,
            ThreadAnswer::Permission {
                request_id,
                option_id,
            },
        ) => {
            room.answer_permission(&thread.key, &request_id, &option_id)
                .await?;
        }
        (
            ThreadKind::Side,
            ThreadAnswer::Permission {
                request_id,
                option_id,
            },
        ) => {
            room.side_answer_permission(&thread.key, &request_id, &option_id)
                .await?;
        }
        (
            ThreadKind::Pair,
            ThreadAnswer::Permission {
                request_id,
                option_id,
            },
        ) => {
            room.peers_answer_permission(&thread.key, &request_id, &option_id)
                .await?;
        }
        (
            ThreadKind::Dm | ThreadKind::Side,
            ThreadAnswer::Human {
                action_id,
                status,
                note,
            },
        ) => {
            let persona_id = owner_of(log, thread)?;
            room.answer_human(&persona_id, &action_id, status, note)?;
        }
        // A pair raises permissions and nothing else. A run and a call are
        // refused above, by their policy.
        (_, ThreadAnswer::Human { .. }) => {
            return Err(not_here(thread, "A request for the person"));
        }
        (ThreadKind::Run | ThreadKind::Call, _) => {
            return Err(not_here(thread, "Answering a card"));
        }
    }
    Ok(Value::Null)
}

/// Older lines of a thread than its subscription opened with.
pub(super) fn page(
    log: &Log,
    thread: &ThreadId,
    before: &str,
    limit: Option<i64>,
    through: Option<&str>,
) -> Result<Value, String> {
    if thread.kind == ThreadKind::Dm {
        super::commands::living(log, &thread.key)?;
    }
    let stream = thread.stream().ok_or("That thread has no lines.")?;
    Ok(super::thread_page(
        log,
        &stream,
        before,
        limit,
        through,
        super::commands::threads2(),
    ))
}

/// The threads of one teammate, or of the room: live first, then parked, then
/// closed, each newest first. A companion is listed only what it may read: not
/// pairs, which it may not list under their older name either, and not calls,
/// whose summaries carry what was spoken in their `preview`.
pub(super) fn list(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    persona_id: Option<&str>,
    companion: bool,
) -> Result<Value, String> {
    let store = ThreadStore::new(log);
    let threads = match persona_id {
        Some(persona_id) => store.list(persona_id),
        None => store.all(),
    };
    let roster = crate::room::roster(log);
    let mut summaries: Vec<ThreadSummary> = threads
        .iter()
        .filter(|thread| {
            !companion || (thread.kind() != ThreadKind::Pair && super::phone_may_read(&thread.id))
        })
        .map(|thread| summarize(log, room, thread, &roster))
        .collect();
    summaries.sort_by_key(|summary| {
        (
            match summary.state {
                LinkState::Live => 0,
                LinkState::Parked => 1,
                LinkState::Closed => 2,
            },
            std::cmp::Reverse(summary.updated_at),
        )
    });
    Ok(json!(summaries))
}

/// One thread as a list reads it.
fn summary_of(log: &Log, room: &Arc<dyn RoomHandle>, thread: &ThreadId) -> Result<Value, String> {
    let thread = ThreadStore::new(log)
        .load(thread)
        .ok_or_else(|| format!("There is no such thread {thread}."))?;
    Ok(json!(summarize(
        log,
        room,
        &thread,
        &crate::room::roster(log)
    )))
}

fn summarize(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    thread: &Thread,
    roster: &[Persona],
) -> ThreadSummary {
    let persona_id = thread
        .participants
        .iter()
        .find_map(|participant| match participant {
            Participant::Persona(id) | Participant::Voice(id) => Some(id.clone()),
            Participant::Person => None,
        })
        .unwrap_or_default();
    let with_persona_id = (thread.kind() == ThreadKind::Pair)
        .then(|| {
            thread
                .participants
                .iter()
                .filter_map(|participant| match participant {
                    Participant::Persona(id) => Some(id.clone()),
                    _ => None,
                })
                .nth(1)
        })
        .flatten();

    // A DM is read from the end of its tape, which can be very long; every
    // other thread is small enough to read whole.
    let (events, started_at, preview) = match thread.kind() {
        ThreadKind::Dm => {
            let tail = previews::tail(log.root(), &thread.id.key);
            let preview =
                previews::preview_reaching(log.root(), &thread.id.key, &tail).and_then(|found| {
                    found
                        .get("text")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                });
            let started = roster
                .iter()
                .find(|persona| persona.id == thread.id.key)
                .map_or(0, |persona| persona.created_at);
            (tail, started, preview)
        }
        _ => {
            let events = thread
                .id
                .stream()
                .map(|stream| log.load(&stream))
                .unwrap_or_default();
            let preview = crate::session::preview_line(&events);
            let started = events.iter().filter_map(ts_of).min().unwrap_or_default();
            (events, started, preview)
        }
    };
    let link = Link::find(&events, &thread.id);
    let updated_at = events.iter().filter_map(ts_of).max().unwrap_or(started_at);

    let running = room.sides(&persona_id);
    let (state, end) = match thread.state {
        // A work thread whose record says live and that the room holds no
        // agent for is one a dead process left: the next start parks it.
        ThreadState::Live
            if thread.kind() == ThreadKind::Side
                && !running.iter().any(|side| side.side_id == thread.id.key) =>
        {
            (LinkState::Parked, None)
        }
        ThreadState::Live => (LinkState::Live, None),
        ThreadState::Parked => (LinkState::Parked, None),
        ThreadState::Closed(end) => (LinkState::Closed, Some(end)),
    };
    let working = match thread.kind() {
        ThreadKind::Dm => matches!(
            room.info(&thread.id.key).state,
            SessionState::Thinking | SessionState::Starting
        ),
        ThreadKind::Side => running
            .iter()
            .any(|side| side.side_id == thread.id.key && side.working),
        ThreadKind::Pair => room
            .peer_threads(&persona_id)
            .iter()
            .any(|pair| pair.thread_key == thread.id.key && pair.working_persona_id.is_some()),
        ThreadKind::Run => state == LinkState::Live,
        ThreadKind::Call => false,
    };

    ThreadSummary {
        thread: thread.id.clone(),
        persona_id,
        with_persona_id,
        title: thread.title.clone(),
        state,
        end,
        opener: link
            .as_ref()
            .and_then(|link| link.opener.clone())
            .map(SideOpener::from),
        started_at,
        updated_at,
        working,
        waiting: super::waiting_on(&events),
        preview,
        outcome: link.and_then(|link| link.outcome),
    }
}

fn ts_of(event: &Value) -> Option<i64> {
    event.get("ts").and_then(Value::as_i64)
}

/// The two teammates of a pair.
fn pair_of(thread: &ThreadId) -> Result<(&str, &str), String> {
    thread_participants(&thread.key)
        .ok_or_else(|| "That is not a thread between two teammates.".into())
}

/// The teammate a DM or a work thread is with.
fn owner_of(log: &Log, thread: &ThreadId) -> Result<String, String> {
    match thread.kind {
        ThreadKind::Dm => Ok(thread.key.clone()),
        _ => ThreadStore::new(log)
            .load(thread)
            .and_then(|found| found.parent)
            .map(|parent| parent.thread.key)
            .ok_or_else(|| format!("There is no such thread {thread}.")),
    }
}
