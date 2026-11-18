//! Topics: an ask, its answers and the follow-ups on them, as one
//! conversation between teammates (`docs/design/agent-inbox.md`, Topics).
//!
//! - **Each item keeps its topic** when it is made: the topic of the item it
//!   answers, else the send id of the `send` that made it, which the items
//!   one send made for several teammates share. An item kept before either
//!   was recorded is given them as it is read (`kept::topics`).
//! - **A follow-up stays in its topic.** `send`'s `reply_to` names an item
//!   the teammate received or one it sent, so it can follow up on its own
//!   unanswered send as well as on an answer. A `send` that answers a
//!   question it received closes that question as answered, as
//!   `update_inbox`'s `answer` does.
//! - **A one-line ask is its own title**: `send` without a `title` takes the
//!   message's first plain line.

use super::{Room, TITLE_CHARS, kept, short_line};
use crate::contract::{InboxItem, InboxState, InboxWants, Persona};

/// What a `send` without a title is kept as: the message's first plain line
/// as its title, and the whole message as its words, unless that line is
/// the whole message, which is then said once, as the title alone.
pub(super) fn untitled(message: &str) -> Result<(String, String), String> {
    let message = message.trim();
    let first = message
        .lines()
        .map(|line| line.trim().trim_start_matches(['#', '>', '-', '*', ' ']))
        .find(|line| !line.is_empty())
        .ok_or("send needs a `message`, or a `title` for an item with no words.")?;
    let title = short_line(first, TITLE_CHARS);
    let whole = !message.contains('\n') && title == crate::fence::one_line(message);
    Ok((
        title,
        if whole {
            String::new()
        } else {
            message.to_string()
        },
    ))
}

/// Where a send to one teammate landed, as its result says it: in their
/// inbox, or waiting for the person's approval, never received before it.
pub(super) fn delivery(held: bool) -> &'static str {
    if held {
        "awaiting approval"
    } else {
        "in inbox"
    }
}

/// Whether a `send` from `caller` to `targets` that names `asked` in
/// `reply_to` answers it: a question in the caller's own inbox, still open,
/// sent back to whoever asked it.
pub(super) fn answers(caller: &Persona, targets: &[Persona], asked: &InboxItem) -> bool {
    asked.to == caller.id
        && asked.notice.is_none()
        && asked.state != InboxState::Closed
        && asked.wants == InboxWants::Answer
        && targets.iter().any(|target| target.id == asked.from)
}

impl Room {
    /// The topic an item is in: its own, or, for one kept before topics were
    /// recorded, the one its chain of answers gives it.
    pub(super) fn topic_for(&self, item: &InboxItem) -> String {
        if let Some(topic) = &item.topic {
            return topic.clone();
        }
        let roster = crate::room::roster(&self.log);
        kept::everything(&self.log, roster.iter().map(|persona| persona.id.as_str()))
            .into_iter()
            .find(|known| known.id == item.id)
            .and_then(|known| known.topic)
            .unwrap_or_else(|| item.id.clone())
    }

    /// The item `send`'s `reply_to` names: one in the teammate's own inbox,
    /// or one it sent a colleague, by its id or a part of it that names one
    /// item alone.
    pub(super) fn linked_item(&self, persona_id: &str, named: &str) -> Result<InboxItem, String> {
        let received = match self.item_of(persona_id, named) {
            Ok(found) => return Ok(found),
            Err(why) => why,
        };
        let named = named.trim();
        let roster = crate::room::roster(&self.log);
        let sent = kept::sent_by(
            &self.log,
            persona_id,
            roster.iter().map(|persona| persona.id.as_str()),
        );
        if let Some(found) = sent.iter().find(|item| item.id == named) {
            return Ok(found.clone());
        }
        let mut found = sent
            .into_iter()
            .filter(|item| named.len() >= 4 && item.id.starts_with(named));
        match (found.next(), found.next()) {
            (Some(item), None) => Ok(item),
            (Some(_), Some(_)) => Err(format!(
                "{named} could be more than one item: give more of its id."
            )),
            _ => Err(received.replace(
                "in your inbox. read_inbox lists them.",
                "in your inbox or among what you sent. read_inbox lists them.",
            )),
        }
    }
}
