//! The roster view: every living teammate with its last line, the tool
//! still running, and its session, kept up to date.
//!
//! Nothing logs this. It is a join of three things that change on their own
//! clocks — the room stream, each teammate's tape, the live sessions — and
//! the window would otherwise have to make that join itself, from three
//! subscriptions, on every keystroke an agent writes.
//!
//! A tape is watched by a pump of its own, because the log hands out one
//! broadcast per stream and a roster is as many tapes as there are
//! teammates. The pumps are stopped by the view ending: dropping its end of
//! the channel they feed is what every one of them is waiting on.

use super::{RoomHandle, roster_entry, send};
use crate::contract::{Persona, SessionInfo};
use crate::log::{Log, StreamId};
use crate::room;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};

pub(super) async fn view(
    id: i64,
    log: Log,
    handle: Arc<dyn RoomHandle>,
    mut room_events: broadcast::Receiver<Value>,
    infos: broadcast::Receiver<SessionInfo>,
    sender: super::Outbox,
) {
    let mut infos = Some(infos);
    let (spoke, mut spoken) = mpsc::unbounded_channel::<String>();
    let mut watched: HashSet<String> = HashSet::new();

    let rows = snapshot(&log, &handle, &spoke, &mut watched);
    if !send(&sender, json!({ "sub": id, "snapshot": rows })) {
        return;
    }

    loop {
        let info = async {
            match infos.as_mut() {
                Some(infos) => infos.recv().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            event = room_events.recv() => match event {
                Ok(event) => {
                    if event.get("kind").and_then(Value::as_str) != Some("persona") {
                        continue;
                    }
                    let Some(persona_id) = event.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    if event.get("deleted").and_then(Value::as_bool) == Some(true) {
                        if !send(&sender, json!({ "sub": id, "removed": persona_id })) {
                            return;
                        }
                        continue;
                    }
                    let Ok(persona) = serde_json::from_value::<Persona>(event.clone()) else {
                        continue;
                    };
                    watch(&log, &persona.id, &spoke, &mut watched);
                    let row = roster_entry(&log, &handle, persona);
                    if !send(&sender, json!({ "sub": id, "event": row })) {
                        return;
                    }
                }
                // Too far behind to say which teammates changed, so it says
                // all of them: a client folding by id absorbs the second
                // snapshot exactly as it absorbed the first.
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let rows = snapshot(&log, &handle, &spoke, &mut watched);
                    if !send(&sender, json!({ "sub": id, "snapshot": rows })) {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            spoken = spoken.recv() => match spoken {
                Some(persona_id) => {
                    if !row_for(&log, &handle, &sender, id, &persona_id) {
                        return;
                    }
                }
                None => return,
            },
            info = info => match info {
                Ok(info) => {
                    if !row_for(&log, &handle, &sender, id, &info.persona_id) {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => infos = None,
            },
        }
    }
}

/// Every living teammate's row, watching any tape not watched already.
fn snapshot(
    log: &Log,
    handle: &Arc<dyn RoomHandle>,
    spoke: &mpsc::UnboundedSender<String>,
    watched: &mut HashSet<String>,
) -> Vec<Value> {
    room::roster(log)
        .into_iter()
        .map(|persona| {
            watch(log, &persona.id, spoke, watched);
            roster_entry(log, handle, persona)
        })
        .collect()
}

/// One teammate's row again, or nothing at all for one the roster no longer
/// holds — a session that reports itself after its teammate was deleted is
/// not a row to put back.
fn row_for(
    log: &Log,
    handle: &Arc<dyn RoomHandle>,
    sender: &super::Outbox,
    id: i64,
    persona_id: &str,
) -> bool {
    let Some(persona) = room::roster(log)
        .into_iter()
        .find(|persona| persona.id == persona_id)
    else {
        return true;
    };
    let row = roster_entry(log, handle, persona);
    send(sender, json!({ "sub": id, "event": row }))
}

/// Forwards this teammate's id whenever the row would change: either side
/// said something, a tool moved, or a turn ended. Only the id: the preview
/// and the activity are read from the tape when the row is rebuilt, so the
/// event itself has nowhere to go.
fn watch(
    log: &Log,
    persona_id: &str,
    spoke: &mpsc::UnboundedSender<String>,
    watched: &mut HashSet<String>,
) {
    if !watched.insert(persona_id.to_string()) {
        return;
    }
    let mut events = log.subscribe(&StreamId::Tape(persona_id.to_string()));
    let spoke = spoke.clone();
    let persona_id = persona_id.to_string();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                // The view ended and dropped the channel. Nothing else stops
                // a pump, and a silent tape must not keep one alive.
                () = spoke.closed() => return,
                event = events.recv() => match event {
                    Ok(event) => {
                        let kind = event.get("kind").and_then(Value::as_str);
                        if !matches!(kind, Some("user") | Some("agent") | Some("tool") | Some("turn"))
                        {
                            continue;
                        }
                        if spoke.send(persona_id.clone()).is_err() {
                            return;
                        }
                    }
                    // Whatever was missed, the preview is read from the tape
                    // itself, so one nudge catches up on all of it.
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        if spoke.send(persona_id.clone()).is_err() {
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                },
            }
        }
    });
}
