//! What the inbox keeps, and for how long (`docs/design/agent-inbox.md`,
//! What is kept).
//!
//! - **Closed items are kept [`keep_days`] days**, then removed: 30 by
//!   default, the person's to change in Settings (`inboxKeepDays`). The
//!   clock removes them as it looks, and the desk as it opens.
//! - **The inbox's files are tidied as the desk opens**: each teammate's
//!   stream is written again with each item's latest line alone (every move
//!   writes the whole item again), and the work an item was taken into, once
//!   it has finished and its items are gone, leaves the two teammates'
//!   record (`exchanges/inbox.rs`).
//!
//! Nothing here touches what a newer Hotline wrote and this build cannot
//! read: its lines are kept byte for byte ([`crate::log::Log::tidy`]) and
//! never removed, however old, and its requests stay on the pair's record.

use super::{Room, kept, lock};
use crate::contract::{InboxItem, InboxState};
use crate::log::StreamId;

const DAY_MS: i64 = 24 * 3_600_000;

/// How many days a closed item is kept: the room's `inboxKeepDays`, or the
/// default when it says nothing this build can use.
pub(in crate::session) fn keep_days(log: &crate::log::Log) -> i64 {
    crate::room::settings(log)
        .get(crate::room::INBOX_KEEP_DAYS)
        .and_then(serde_json::Value::as_i64)
        .filter(|days| (1..=crate::room::MAX_KEEP_DAYS).contains(days))
        .unwrap_or(crate::room::DEFAULT_KEEP_DAYS)
}

/// When an item closed: its last receipt that closed it, else when it was
/// sent.
fn closed_at(item: &InboxItem) -> i64 {
    item.receipts
        .iter()
        .rev()
        .find(|receipt| receipt.state == InboxState::Closed)
        .map_or(item.ts, |receipt| receipt.at)
}

/// Whether an item is to be removed at `now`: closed, and longer ago than
/// it is kept.
fn spent(item: &InboxItem, now: i64, keep_ms: i64) -> bool {
    item.state == InboxState::Closed && closed_at(item) + keep_ms <= now
}

impl Room {
    /// Removes every closed item kept longer than [`keep_days`] at `now`,
    /// from each teammate's stream, and lets go of the finished work those
    /// items were taken into. `at_start` also writes each stream again with
    /// each item's latest line alone, as the desk opens. Under the inbox
    /// lock, so no move of an item lands between the look and the rewrite.
    pub(in crate::session) fn inbox_tidy(&self, now: i64, at_start: bool) {
        let keep_ms = keep_days(&self.log) * DAY_MS;
        let mut removed = false;
        {
            let _changing = lock(&self.inbox_lock);
            for persona in crate::room::roster(&self.log) {
                let due = kept::of(&self.log, &persona.id)
                    .iter()
                    .any(|item| spent(item, now, keep_ms));
                if !due && !at_start {
                    continue;
                }
                let stream = StreamId::Inbox(persona.id.clone());
                // A line this build cannot read is no item here, and stays.
                match self.log.tidy(&stream, |line| {
                    kept::read(line).is_none_or(|item| !spent(&item, now, keep_ms))
                }) {
                    Ok(_) => removed |= due,
                    Err(error) => {
                        eprintln!("{}'s inbox could not be tidied: {error}", persona.id);
                    }
                }
            }
        }
        if removed || at_start {
            self.forget_finished_work();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{InboxReceipt, ThreadId};

    #[test]
    fn an_item_is_spent_once_it_has_been_closed_longer_than_it_is_kept() {
        let mut item: InboxItem = serde_json::from_value(serde_json::json!({
            "id": "i", "ts": 0, "from": "ada", "fromThread": ThreadId::dm("ada"), "to": "bob",
            "title": "t", "body": "", "wants": "answer", "state": "taken", "receipts": [],
            "senderTold": 0,
        }))
        .unwrap();
        assert!(!spent(&item, 100 * DAY_MS, DAY_MS), "an open item stays");
        item.state = InboxState::Closed;
        item.receipts.push(InboxReceipt {
            state: InboxState::Closed,
            at: 10 * DAY_MS,
            reason: None,
            outcome: None,
            into: None,
            newer: serde_json::Map::new(),
        });
        assert!(!spent(&item, 10 * DAY_MS + DAY_MS - 1, DAY_MS));
        assert!(
            spent(&item, 11 * DAY_MS, DAY_MS),
            "kept a day from when it closed"
        );
    }
}
