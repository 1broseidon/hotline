//! The schedules view: one teammate's scheduled jobs and loops, for a seat
//! that may read them but not the room stream they live on.
//!
//! A job is a room event, and the room stream also carries every setting,
//! grant and credential reference, which a phone may not subscribe to. So
//! the view watches the room stream here and sends one teammate's jobs, as
//! `ScheduleEntry`s, the whole list each time it changes. A list sent whole
//! cannot drift from the desk's: a new job, a cancellation, a one-shot that
//! fired and a loop whose next run moved are each just the next list, with
//! no tombstone for a client to apply and no entry to count twice.
//!
//! A tombstone does not say whose job it was, so every job event is a reason
//! to read the teammate's list again; a list that did not change is not sent.

use super::{Outbox, send};
use crate::contract::ScheduleEntry;
use crate::log::Log;
use crate::room;
use serde_json::{Value, json};
use tokio::sync::broadcast;

/// Why a teammate's schedules cannot be shown. Each is its own answer to a
/// client, and neither is "nothing scheduled".
pub(super) enum Unreadable {
    /// The room holds no living teammate by that id.
    NoTeammate(String),
    /// The room stream could not be read.
    Failed(String),
}

/// One teammate's jobs as the view sends them, soonest first.
pub(super) fn read(log: &Log, persona_id: &str) -> Result<Vec<ScheduleEntry>, Unreadable> {
    match room::teammate_schedules(log, persona_id) {
        Ok(Some(jobs)) => Ok(jobs.into_iter().map(ScheduleEntry::from).collect()),
        Ok(None) => Err(Unreadable::NoTeammate(format!(
            "There is no teammate {persona_id}."
        ))),
        Err(error) => Err(Unreadable::Failed(error)),
    }
}

/// Keeps a subscription's list current. `shown` is the list its snapshot
/// already sent, read after `room_events` was subscribed, so a change that
/// landed in between arrives here as an event and is not lost.
///
/// The view ends when its teammate is deleted, after `removed`, and when the
/// room can no longer be read, after an `error` frame, since a list it cannot
/// refresh is one the client must stop treating as current.
pub(super) async fn view(
    id: i64,
    persona_id: String,
    log: Log,
    mut room_events: broadcast::Receiver<Value>,
    mut shown: Vec<ScheduleEntry>,
    sender: Outbox,
) {
    loop {
        match room_events.recv().await {
            Ok(event) => {
                let kind = event.get("kind").and_then(Value::as_str);
                let this_teammate =
                    event.get("id").and_then(Value::as_str) == Some(persona_id.as_str());
                if kind != Some("schedule") && !(kind == Some("persona") && this_teammate) {
                    continue;
                }
            }
            // Too far behind to know which jobs changed, so it reads them
            // all, which is all this view ever does anyway.
            Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => return,
        }
        match read(&log, &persona_id) {
            Ok(jobs) if jobs == shown => {}
            Ok(jobs) => {
                if !send(&sender, json!({ "sub": id, "snapshot": jobs })) {
                    return;
                }
                shown = jobs;
            }
            Err(Unreadable::NoTeammate(_)) => {
                send(&sender, json!({ "sub": id, "removed": persona_id }));
                return;
            }
            Err(Unreadable::Failed(error)) => {
                send(
                    &sender,
                    json!({ "sub": id, "error": error, "code": super::UNREADABLE }),
                );
                return;
            }
        }
    }
}
