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
//! - **The person's follow-up is a line in their own chat** that names an
//!   item (`session.prompt`'s `aboutItem`); the teammate is told which topic
//!   it is about, and sends it on itself. Nothing is ever sent as a teammate.
//! - **A one-line ask is its own title**: `send` without a `title` takes the
//!   message's first plain line.

use super::{Landed, Room, TITLE_CHARS, kept, short, short_line};
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

/// Where a send to one teammate landed, as its result says it, the fact
/// first: in their inbox, or not sent, never received before it is.
pub(super) fn delivery(landed: &Landed) -> &'static str {
    match landed {
        Landed::In => "in inbox",
        Landed::Held => "not sent yet, waiting on the person's card",
        Landed::Declined => "not sent, the person declined",
        Landed::Refused(_) => "not sent",
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

    /// What the teammate is told with the person's line that follows up on
    /// the item `named` (Ask a follow-up): which of its topics with a
    /// colleague the line is about, and how to send it on into that topic.
    /// The item is one the teammate received or sent; any other is refused,
    /// so the line is not said.
    pub(in crate::session) fn follow_up_line(
        &self,
        persona_id: &str,
        named: &str,
    ) -> Result<String, String> {
        let named = named.trim();
        let roster = crate::room::roster(&self.log);
        let all = kept::everything(&self.log, roster.iter().map(|persona| persona.id.as_str()));
        let party = |item: &InboxItem| {
            (item.to == persona_id && !item.held)
                || (item.from == persona_id && item.notice.is_none())
        };
        let item = all
            .iter()
            .find(|item| item.id == named && party(item))
            .ok_or("That inbox message is not one this teammate sent or received, so nothing was said.")?;
        let topic = item.topic.as_deref().unwrap_or(&item.id);
        let mut held: Vec<&InboxItem> = all
            .iter()
            .filter(|known| known.topic.as_deref() == Some(topic) && party(known))
            .collect();
        held.sort_by_key(|known| known.ts);
        let first = held.first().copied().unwrap_or(item);
        let mut others: Vec<String> = Vec::new();
        for known in &held {
            for teammate in [&known.from, &known.to] {
                let name = self.name_of(teammate);
                if teammate != persona_id && !others.contains(&name) {
                    others.push(name);
                }
            }
        }
        let with = others.join(", ");
        let title = crate::fence::quoted(first.title.trim_start_matches("Re: "));
        let id = short(&item.id);
        Ok(format!(
            "(The person is following up on your inbox conversation {title} with {with}; its latest message is {id}. If this is for {with}, send it on with `send` and `reply_to` {id}, so it stays in that conversation. If you can answer it yourself, do.)"
        ))
    }
}
