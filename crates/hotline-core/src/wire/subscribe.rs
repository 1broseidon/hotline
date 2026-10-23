//! Subscriptions: a stream's snapshot, its events and its live deltas.

use super::phone::{PHONE_WINDOW, desk_only, link_as_sent, phone_event, snapshot_shaped};
use super::seat::Seat;
use super::socket::{FORBIDDEN, Outbox, Refused, UNKNOWN_TEAMMATE, UNREADABLE, reply};
use super::{Agents, RoomHandle, roster, schedules, send};
use crate::contract::{DeltaKind, StreamDelta, Target, ViewName};
use crate::contract::{ThreadId, ThreadKind};
use crate::log::{Log, StreamId};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

/// Opens a subscription.
///
/// The acknowledgement is queued before the task that forwards the stream
/// exists, because a task the runtime starts on another thread would
/// otherwise be free to queue its snapshot first, and a client is promised
/// `ok`, then one snapshot, then events. Inside that task the broadcast is
/// subscribed to before the fold is loaded, so nothing lands in between.
pub(super) fn subscribe(
    id: i64,
    frame: &Value,
    seat: Seat,
    desk: &Agents,
    sender: &Outbox,
    subscriptions: &mut HashMap<i64, JoinHandle<()>>,
) -> Result<(), Refused> {
    let Agents { log, room, parts } = desk;
    let target: Target = serde_json::from_value(frame["sub"].clone())
        .map_err(|error| format!("That is not something to subscribe to: {error}."))?;
    if !seat.permits_sub(&target) {
        return Err(Refused::because(
            FORBIDDEN,
            "That seat may not subscribe to that.",
        ));
    }
    // A task that ended on its own — the stream closed, the socket's writer
    // went away — still occupies this map unless we notice. Reusing the id
    // would otherwise be "already open" for a subscription that will never
    // deliver.
    subscriptions.retain(|_, handle| !handle.is_finished());
    if seat == Seat::Phone && subscriptions.len() >= 8 {
        return Err(String::from("A phone can open at most eight subscriptions.").into());
    }
    if subscriptions.contains_key(&id) {
        return Err(format!("Subscription {id} is already open.").into());
    }

    let stream = match target {
        Target::Call(call_id) => {
            let voice = parts
                .voice
                .as_ref()
                .ok_or_else(|| "Voice is not available on this desk.".to_string())?;
            let (before, mut events) = voice.subscribe(&call_id)?;
            // The subscription is replayed by both clients after reconnecting.
            // An ended call still supplies its terminal snapshot.
            if !matches!(
                before,
                crate::contract::VoiceEvent::State {
                    state: crate::contract::VoiceState::Ended,
                    ..
                }
            ) {
                voice.bind_connection(
                    call_id.clone(),
                    &sender.call_owner,
                    sender.cancel.clone(),
                    sender.pairing_cancel.clone(),
                )?;
            }
            let (snapshot, _) = voice.subscribe(&call_id)?;
            reply(sender, id, Ok(Value::Null));
            if !send(sender, json!({"sub": id, "snapshot": [snapshot]})) {
                return Ok(());
            }
            let sender = sender.clone();
            subscriptions.insert(
                id,
                tokio::spawn(async move {
                    loop {
                        match events.recv().await {
                            Ok(event) => {
                                let ended = matches!(
                                    event,
                                    crate::contract::VoiceEvent::State {
                                        state: crate::contract::VoiceState::Ended,
                                        ..
                                    }
                                );
                                if !send(&sender, json!({"sub": id, "event": event})) || ended {
                                    break;
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => {
                                // Missing a clip changes what was said. Reconnect for a fresh state.
                                sender.cancel.cancel();
                                break;
                            }
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }),
            );
            return Ok(());
        }
        // The list is read before the subscription is acknowledged, because a
        // teammate the room does not hold, or a room that cannot be read, is
        // a refusal — never `ok` followed by an empty list. The room stream
        // is subscribed to first, so a job that lands between the read and
        // the view's first wait is still seen.
        Target::Schedules(persona_id) => {
            let room_events = log.subscribe(&StreamId::Room);
            let shown =
                schedules::read(log, &persona_id).map_err(|unreadable| match unreadable {
                    schedules::Unreadable::NoTeammate(error) => {
                        Refused::because(UNKNOWN_TEAMMATE, error)
                    }
                    schedules::Unreadable::Failed(error) => Refused::because(UNREADABLE, error),
                })?;
            reply(sender, id, Ok(Value::Null));
            if !send(sender, json!({ "sub": id, "snapshot": shown })) {
                return Ok(());
            }
            let view = schedules::view(
                id,
                persona_id,
                log.clone(),
                room_events,
                shown,
                sender.clone(),
            );
            subscriptions.insert(id, tokio::spawn(view));
            return Ok(());
        }
        Target::View(ViewName::Roster) => {
            reply(sender, id, Ok(Value::Null));
            let room_events = log.subscribe(&StreamId::Room);
            let infos = room.subscribe_info();
            let view = roster::view(
                id,
                log.clone(),
                room.clone(),
                room_events,
                infos,
                sender.clone(),
                seat,
            );
            subscriptions.insert(id, tokio::spawn(view));
            return Ok(());
        }
        Target::Room => StreamId::Room,
        Target::Tape(persona_id) => StreamId::Tape(persona_id),
        Target::Thread(key) => StreamId::Pair(key),
        Target::ThreadId(thread) => {
            StreamId::of_thread(&thread).expect("every thread has a stream")
        }
        Target::Run(id) => StreamId::Run(id),
        Target::Side(id) => StreamId::Side(id),
    };

    reply(sender, id, Ok(Value::Null));
    // A phone shows the last lines of a thread, as many as it asked for.
    let window = frame
        .get("window")
        .and_then(Value::as_u64)
        .map(|window| (window as usize).clamp(1, PHONE_WINDOW));
    let forward = stream_events(
        id,
        stream,
        window,
        seat,
        log.clone(),
        room.clone(),
        sender.clone(),
    );
    subscriptions.insert(id, tokio::spawn(forward));
    Ok(())
}

/// A page of older lines, and the most one may ask for.
const TAPE_PAGE: usize = 400;
const TAPE_PAGE_MAX: usize = 2_000;
/// Context kept above a line a page was asked to reach.
const THROUGH_CONTEXT: usize = 40;

/// The lines of a thread's stream before `before`, oldest first: a page of
/// `limit`, or back to a little above `through` when that line is further up.
/// A `before` the stream does not hold answers an empty page, not an error: the
/// line may have been rewritten out, and the window then simply stops.
pub(super) fn thread_page(
    log: &Log,
    stream: &StreamId,
    before: &str,
    limit: Option<i64>,
    through: Option<&str>,
    threads2: bool,
) -> Value {
    let events = log.load(stream);
    let id_of = |event: &Value| event.get("id").and_then(Value::as_str).map(str::to_string);
    let Some(end) = events
        .iter()
        .position(|event| id_of(event).as_deref() == Some(before))
    else {
        return json!({ "events": [], "more": false });
    };
    let limit = limit
        .and_then(|limit| usize::try_from(limit).ok())
        .unwrap_or(TAPE_PAGE)
        .clamp(1, TAPE_PAGE_MAX);
    let reach = through
        .and_then(|through| {
            events[..end]
                .iter()
                .position(|event| id_of(event).as_deref() == Some(through))
        })
        .map(|at| at.saturating_sub(THROUGH_CONTEXT));
    let start = reach.unwrap_or_else(|| end.saturating_sub(limit));
    let page: Vec<Value> = events[start..end]
        .iter()
        .cloned()
        .map(|event| link_as_sent(event, threads2))
        .collect();
    json!({ "events": page, "more": start > 0 })
}

async fn stream_events(
    id: i64,
    stream: StreamId,
    window: Option<usize>,
    seat: Seat,
    log: Log,
    room: Arc<dyn RoomHandle>,
    sender: Outbox,
) {
    let mut events = log.subscribe(&stream);
    let mut deltas = match &stream {
        StreamId::Tape(_) | StreamId::Side(_) | StreamId::Run(_) | StreamId::Job(_) => {
            Some(room.subscribe_deltas())
        }
        _ => None,
    };
    let scope = delta_scope(&stream);
    let phone = || seat == Seat::Phone || sender.lean();
    let snapshot = |sender: &Outbox| {
        let window = if phone() { window } else { None };
        snapshot_shaped(&log, &stream, phone(), window, sender.threads2())
    };
    if !send(&sender, json!({ "sub": id, "snapshot": snapshot(&sender) })) {
        return;
    }

    loop {
        let delta = async {
            match deltas.as_mut() {
                Some(deltas) => deltas.recv().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            event = events.recv() => match event {
                Ok(event) if phone() && desk_only(&event) => {}
                Ok(event) => {
                    let event = if stream == StreamId::Room { crate::mcp::public_room_event(event) } else { link_as_sent(event, sender.threads2()) };
                    if !send(&sender, json!({ "sub": id, "event": if phone() { phone_event(event) } else { event } })) {
                        return;
                    }
                }
                // Too far behind to be told what it missed, so it is told
                // everything instead: a second snapshot, which a client that
                // folds by id absorbs the same way it absorbed the first.
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    if !send(&sender, json!({ "sub": id, "snapshot": snapshot(&sender) })) {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            delta = delta => match delta {
                // The phone draws words; a download ring is the desk's.
                Ok(StreamDelta::ComputerPull { .. }) if phone() => {}
                Ok(delta) => {
                    let heard = scope.as_ref().and_then(|scope| delta_for(delta, scope, sender.threads2()));
                    if let Some(heard) = heard
                        && sender
                            .send_droppable(json!({ "sub": id, "ephemeral": heard }).to_string())
                            .is_err()
                    {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                // Nothing to recover: a delta nobody saw is a few characters
                // the durable line will carry anyway.
                Err(broadcast::error::RecvError::Closed) => deltas = None,
            },
        }
    }
}

/// The thread whose words a stream's subscription hears live: a tape is the
/// teammate's DM, a side thread, a run and a job their own. None for a stream
/// that has no live words.
fn delta_scope(stream: &StreamId) -> Option<ThreadId> {
    match stream {
        StreamId::Tape(persona_id) => Some(ThreadId::dm(persona_id)),
        StreamId::Side(side_id) => Some(ThreadId::side(side_id)),
        StreamId::Run(run_id) => Some(ThreadId::run(run_id)),
        StreamId::Job(job_id) => Some(ThreadId::job(job_id)),
        _ => None,
    }
}

/// A delta as this subscription's client is sent it, or none when it is not
/// this thread's. A client that declared `threads2` is sent a `ThreadDelta`;
/// one that did not is sent the shape its kind has always had, and for a kind
/// that never had one, nothing. Never the other way round, so a side thread's
/// words cannot land in the teammate's main conversation.
fn delta_for(delta: StreamDelta, scope: &ThreadId, threads2: bool) -> Option<StreamDelta> {
    match delta {
        StreamDelta::ThreadDelta {
            thread,
            message_id,
            kind,
            text,
        } if thread == *scope => {
            if threads2 {
                return Some(StreamDelta::ThreadDelta {
                    thread,
                    message_id,
                    kind,
                    text,
                });
            }
            let text_kind = kind == DeltaKind::Text;
            match (thread.kind, text_kind) {
                (ThreadKind::Dm, true) => Some(StreamDelta::AgentDelta {
                    persona_id: thread.key,
                    message_id,
                    text,
                }),
                (ThreadKind::Dm, false) => Some(StreamDelta::ThoughtDelta {
                    persona_id: thread.key,
                    message_id,
                    text,
                }),
                (ThreadKind::Side, true) => Some(StreamDelta::SideAgentDelta {
                    side_id: thread.key,
                    message_id,
                    text,
                }),
                (ThreadKind::Side, false) => Some(StreamDelta::SideThoughtDelta {
                    side_id: thread.key,
                    message_id,
                    text,
                }),
                _ => None,
            }
        }
        StreamDelta::ComputerPull { ref persona_id, .. }
            if scope.kind == ThreadKind::Dm && *persona_id == scope.key =>
        {
            Some(delta)
        }
        // The older shapes are what a `ThreadDelta` is sent as. The room
        // broadcasts only the one, so none of them is a thread's to forward.
        _ => None,
    }
}
