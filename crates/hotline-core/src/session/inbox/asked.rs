//! What the person asked for, once it is finished
//! (`docs/design/agent-inbox.md`, What the person asked for).
//!
//! An item a teammate sent in a turn of its main conversation that the
//! person read is theirs (`for_person`, set at `send`). While colleagues
//! work on it nothing of it reaches the person: an answer to one of several,
//! a receipt, a reply that is only progress, each wakes the sender's quiet
//! turn as any item does. Once every item one such send made is closed
//! (answered, done, noted, dropped, combined or expired; one held for the
//! person's first-contact card is still open), the send is finished, and
//! the sender's next wake in its main conversation is a turn the person
//! reads, whose line says to tell them the result in its own words. The
//! desk makes the turn loud; the words are the teammate's, since a
//! colleague's reply was written to the teammate, not to the person. A send
//! that wants nothing back finishes nothing the person waits on.
//!
//! Finished is news once: a send is offered as finished while one of its
//! items has closed since its sender was last told (`sender_told`), which
//! the turn's index tells, or the mid-turn notice marks as told.

use super::{Room, kept};
use crate::contract::{InboxItem, InboxState, InboxWants, ThreadId};
use std::collections::HashSet;

/// The mark a finished ask's item carries in the index's SENT lines.
pub(super) const FINISHES_ASK: &str =
    " · finishes what the person asked of you: tell them the result, in your own words";

/// What tells the teammate how to say a finished ask, after naming it: to
/// the person, in its own words, never the colleague's.
const TELL_THEM: &str = "Tell the person the result in your own words, addressed to them, in a line or two. Do not paste or quote your colleague's reply: it was written to you, not to them. If anything they asked is still open, say so in a few words.";

/// Whether an item, closing, may finish something the person asked for.
pub(super) fn may_finish_ask(item: &InboxItem) -> bool {
    item.for_person && item.notice.is_none() && item.wants != InboxWants::Nothing
}

/// Whether the items one send made are finished and that is news: all of
/// them closed, one since the sender was last told.
fn finished(items: &[InboxItem]) -> bool {
    items.iter().all(|item| item.state == InboxState::Closed)
        && items.iter().any(|item| {
            item.receipts.len() > usize::try_from(item.sender_told).unwrap_or(usize::MAX)
        })
}

impl Room {
    /// The sends `persona_id` made for the person that are finished and not
    /// yet told, each as the items it made, oldest first.
    pub(super) fn finished_asks(&self, persona_id: &str) -> Vec<Vec<InboxItem>> {
        let roster = crate::room::roster(&self.log);
        let sent = kept::sent_by(
            &self.log,
            persona_id,
            roster.iter().map(|persona| persona.id.as_str()),
        );
        let mut sends: Vec<(String, Vec<InboxItem>)> = Vec::new();
        for item in sent.into_iter().filter(may_finish_ask) {
            let send = item.send_id.clone().unwrap_or_else(|| item.id.clone());
            match sends.iter_mut().find(|(known, _)| *known == send) {
                Some((_, items)) => items.push(item),
                None => sends.push((send, vec![item])),
            }
        }
        sends
            .into_iter()
            .map(|(_, items)| items)
            .filter(|items| finished(items))
            .collect()
    }

    /// The ids of the items of `persona_id`'s finished asks.
    pub(super) fn finished_ask_items(&self, persona_id: &str) -> HashSet<String> {
        self.finished_asks(persona_id)
            .into_iter()
            .flatten()
            .map(|item| item.id)
            .collect()
    }

    /// An item for the person closed: its sender's main conversation is
    /// woken once the gather delay is up, which is a turn the person reads
    /// when that finished its send ([`Room::finished_asks`]).
    pub(super) fn ask_closed(&self, item: &InboxItem) {
        if let Some(room) = self.this() {
            room.gather(&item.from, ThreadId::dm(&item.from));
        }
    }

    /// What finished, as a line names it: each send's title, quoted, and
    /// how each colleague's part of it ended ("Rerun the build" (Bob:
    /// answered, Cy: done)).
    fn finished_names(&self, asks: &[Vec<InboxItem>]) -> String {
        asks.iter()
            .map(|items| {
                let parts: Vec<String> = items
                    .iter()
                    .map(|item| {
                        format!(
                            "{}: {}",
                            self.name_of(&item.to),
                            item.outcome.map_or("closed", |outcome| outcome.name())
                        )
                    })
                    .collect();
                format!(
                    "{} ({})",
                    crate::fence::quoted(&items[0].title),
                    parts.join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// The line of the turn the person reads that a finished ask wakes: what
    /// finished, where what came back is, and how to tell them.
    pub(super) fn finished_line(&self, asks: &[Vec<InboxItem>]) -> String {
        format!(
            "inbox · Finished: what you sent colleagues for something the person asked of you, {}. What came back is above, under INBOX; read it with `read_inbox`. The person reads this turn. {TELL_THEM}",
            self.finished_names(asks)
        )
    }

    /// The same, said into a turn the person reads that is in flight.
    pub(super) fn finished_notice(&self, asks: &[Vec<InboxItem>]) -> String {
        format!(
            "(Hotline: what you sent colleagues for something the person asked of you is finished, {}. What came back is in your inbox; read it with `read_inbox`. Tell the person in this turn. {TELL_THEM})",
            self.finished_names(asks)
        )
    }

    /// The items of finished asks a turn in flight was told of: its sender
    /// has been told where they stand, so no later turn tells it again.
    pub(super) fn told_finished(&self, asks: &[Vec<InboxItem>]) {
        for item in asks.iter().flatten() {
            let count = u32::try_from(item.receipts.len()).unwrap_or(u32::MAX);
            self.change_item(&item.to, &item.id, |kept| {
                kept.sender_told = count;
                true
            });
        }
    }
}
