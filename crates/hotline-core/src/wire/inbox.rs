//! The inbox view: one teammate's inbox, what is open, what closed and what
//! it sent others, for `inbox.list` and the `{"inbox": "<personaId>"}`
//! subscription (`docs/design/agent-inbox.md`).
//!
//! Items are kept on their receivers' own streams, so what one teammate sent
//! is spread over everybody else's. Every change to an item wakes the roster
//! rows of its receiver and its sender, which is when the view reads the
//! inbox again, and it sends the whole of it when it changed: a list sent
//! whole cannot drift from the desk's.

use super::send;
use crate::contract::{InboxItemDetail, InboxView, SessionInfo};
use crate::log::Log;
use crate::room;
use crate::store::inbox as kept;
use serde_json::json;
use tokio::sync::broadcast;

/// The teammate's inbox, or why there is none: no such teammate.
pub(super) fn read(log: &Log, persona_id: &str) -> Result<InboxView, String> {
    let roster = room::roster(log);
    if !roster.iter().any(|persona| persona.id == persona_id) {
        return Err(format!("There is no teammate {persona_id}."));
    }
    Ok(kept::view(
        log,
        persona_id,
        roster.iter().map(|persona| persona.id.as_str()),
    ))
}

/// One item by id, wherever it is kept, with the items that answer it.
pub(super) fn item(log: &Log, id: &str) -> Result<InboxItemDetail, String> {
    let roster = room::roster(log);
    let teammates = || roster.iter().map(|persona| persona.id.as_str());
    let item = kept::find(log, id, teammates()).ok_or("There is no such inbox item.")?;
    let replies = teammates()
        .flat_map(|teammate| kept::of(log, teammate))
        .filter(|reply| reply.reply_to.as_deref() == Some(item.id.as_str()))
        .collect();
    Ok(InboxItemDetail { item, replies })
}

/// Keeps a subscription's inbox current. `shown` is what its snapshot
/// already sent, read after `infos` was subscribed, so a change that landed
/// in between arrives here and is not lost. Ends when the teammate is
/// deleted, after `removed`.
pub(super) async fn view(
    id: i64,
    persona_id: String,
    log: Log,
    mut infos: broadcast::Receiver<SessionInfo>,
    mut shown: InboxView,
    sender: super::socket::Outbox,
) {
    loop {
        match infos.recv().await {
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => return,
        }
        match read(&log, &persona_id) {
            Ok(inbox) if inbox == shown => {}
            Ok(inbox) => {
                if !send(&sender, json!({ "sub": id, "snapshot": inbox })) {
                    return;
                }
                shown = inbox;
            }
            Err(_) => {
                send(&sender, json!({ "sub": id, "removed": persona_id }));
                return;
            }
        }
    }
}
