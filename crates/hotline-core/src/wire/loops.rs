//! The open-loops view: one teammate's living open loops (active, waiting,
//! stale, recently done, archived), for `loops.list` and the
//! `{"loops": "<personaId>"}` subscription.
//!
//! Each write lands on `StreamId::Loops(persona_id)` and teammate deletion
//! lands on `StreamId::Room`. Watching both sends a fresh whole snapshot
//! whenever the folded list changes, or `removed` when the teammate is gone.

use super::send;
use super::socket::{Outbox, UNREADABLE};
use crate::contract::OpenLoop;
use crate::log::{Log, StreamId};
use crate::room;
use crate::store::loops as kept;
use serde_json::{Value, json};
use tokio::sync::broadcast;

/// Why a teammate's open loops could not be read.
pub(super) enum Unreadable {
    NoTeammate(String),
    Failed(String),
}

/// One teammate's loops in display order, or why there is no list to read.
pub(super) fn read(log: &Log, persona_id: &str) -> Result<Vec<OpenLoop>, Unreadable> {
    let events = log
        .try_load(&StreamId::Room)
        .map_err(|error| Unreadable::Failed(format!("The room could not be read: {error}")))?;
    if !room::personas(&events)
        .iter()
        .any(|persona| persona.id == persona_id)
    {
        return Err(Unreadable::NoTeammate(format!(
            "There is no teammate {persona_id}."
        )));
    }
    Ok(kept::of(log, persona_id))
}

/// Keeps a subscription's list current. `shown` is the list its snapshot
/// already sent, read after `loop_events` and `room_events` were subscribed,
/// so a change that landed in between arrives here and is not lost.
pub(super) async fn view(
    id: i64,
    persona_id: String,
    log: Log,
    mut loop_events: broadcast::Receiver<Value>,
    mut room_events: broadcast::Receiver<Value>,
    mut shown: Vec<OpenLoop>,
    sender: Outbox,
) {
    loop {
        let should_refresh = tokio::select! {
            received = loop_events.recv() => match received {
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => true,
                Err(broadcast::error::RecvError::Closed) => return,
            },
            received = room_events.recv() => match received {
                Ok(event) => {
                    event.get("kind").and_then(Value::as_str) == Some("persona")
                        && event.get("id").and_then(Value::as_str) == Some(persona_id.as_str())
                }
                Err(broadcast::error::RecvError::Lagged(_)) => true,
                Err(broadcast::error::RecvError::Closed) => return,
            },
        };
        if !should_refresh {
            continue;
        }
        match read(&log, &persona_id) {
            Ok(loops) if loops == shown => {}
            Ok(loops) => {
                if !send(&sender, json!({ "sub": id, "snapshot": loops })) {
                    return;
                }
                shown = loops;
            }
            Err(Unreadable::NoTeammate(_)) => {
                send(&sender, json!({ "sub": id, "removed": persona_id }));
                return;
            }
            Err(Unreadable::Failed(error)) => {
                send(
                    &sender,
                    json!({ "sub": id, "error": error, "code": UNREADABLE }),
                );
                return;
            }
        }
    }
}
