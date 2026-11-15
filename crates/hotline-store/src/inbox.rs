//! The items in a teammate's inbox, read off its own stream.
//!
//! Each is one `inbox_item` line on [`StreamId::Inbox`], written when it is
//! sent and again under the same id each time it changes, so the fold is the
//! whole state: what was sent, and where it stands. The room writes them;
//! this only reads.

use crate::log::{Log, StreamId};
use hotline_contract::{InboxItem, InboxView};
use serde_json::Value;

/// The `kind` an item's line carries.
pub const KIND: &str = "inbox_item";

/// One item as its line on a stream holds it, or nothing for any other line.
pub fn read(event: &Value) -> Option<InboxItem> {
    if event.get("kind").and_then(Value::as_str) != Some(KIND) {
        return None;
    }
    serde_json::from_value(event.clone()).ok()
}

/// An item as its line is written.
pub fn line(item: &InboxItem) -> Value {
    let mut value = serde_json::to_value(item).unwrap_or(Value::Null);
    if let Some(fields) = value.as_object_mut() {
        fields.insert("kind".into(), Value::from(KIND));
    }
    value
}

/// Every item sent to the teammate, held ones included, oldest first.
pub fn of(log: &Log, persona_id: &str) -> Vec<InboxItem> {
    log.load(&StreamId::Inbox(persona_id.to_string()))
        .iter()
        .filter_map(read)
        .collect()
}

/// The items the teammate has: every one sent to it that is not held for the
/// person's first-contact card.
pub fn received(log: &Log, persona_id: &str) -> Vec<InboxItem> {
    of(log, persona_id)
        .into_iter()
        .filter(|item| !item.held)
        .collect()
}

/// How many open items the teammate has not read yet.
pub fn unread(log: &Log, persona_id: &str) -> usize {
    received(log, persona_id)
        .iter()
        .filter(|item| item.state.unread())
        .count()
}

/// The items `persona_id` sent, out of the inboxes of `teammates`, oldest
/// first.
pub fn sent_by<'a>(
    log: &Log,
    persona_id: &str,
    teammates: impl IntoIterator<Item = &'a str>,
) -> Vec<InboxItem> {
    let mut sent: Vec<InboxItem> = teammates
        .into_iter()
        .filter(|teammate| *teammate != persona_id)
        .flat_map(|teammate| of(log, teammate))
        .filter(|item| item.from == persona_id)
        .collect();
    sent.sort_by_key(|item| item.ts);
    sent
}

/// One item by id, out of the inboxes of `teammates`.
pub fn find<'a>(
    log: &Log,
    id: &str,
    teammates: impl IntoIterator<Item = &'a str>,
) -> Option<InboxItem> {
    teammates
        .into_iter()
        .flat_map(|teammate| of(log, teammate))
        .find(|item| item.id == id)
}

/// The teammate's inbox as the window draws it: open and closed newest
/// first, what it sent newest first, and its unread count.
pub fn view<'a>(
    log: &Log,
    persona_id: &str,
    teammates: impl IntoIterator<Item = &'a str>,
) -> InboxView {
    let mut open = Vec::new();
    let mut closed = Vec::new();
    for item in received(log, persona_id).into_iter().rev() {
        if item.state == hotline_contract::InboxState::Closed {
            closed.push(item);
        } else {
            open.push(item);
        }
    }
    let mut sent = sent_by(log, persona_id, teammates);
    sent.reverse();
    let unread = open.iter().filter(|item| item.state.unread()).count();
    InboxView {
        persona_id: persona_id.to_string(),
        open,
        closed,
        sent,
        unread: u32::try_from(unread).unwrap_or(u32::MAX),
    }
}
