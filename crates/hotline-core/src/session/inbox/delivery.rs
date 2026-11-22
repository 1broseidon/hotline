//! Persists and hands off a new colleague item after its caller checks admission.

use super::held::{Landed, held_key};
use super::{Room, moved};
use crate::contract::{InboxItem, InboxState, TranscriptEvent};
use crate::paths::thread_key;
use std::sync::Arc;
use tokio::sync::oneshot;

pub(super) enum Delivery<'a> {
    Send(&'a [(String, Vec<u8>)]),
    Hold {
        files: &'a [(String, Vec<u8>)],
        prompted: Option<String>,
    },
    /// The exchange already counted a work result; a plain answer counts here.
    Reply {
        count: bool,
    },
    /// The operator's Loop in has its own transcript line, not a sent row.
    LoopIn,
}

impl Room {
    /// Keeps the item once, before counting, waking or raising its card. A
    /// held send returns a wait; settling it edits that same item in place.
    pub(super) fn deliver(
        self: &Arc<Self>,
        item: &mut InboxItem,
        via: Delivery<'_>,
    ) -> Result<Option<oneshot::Receiver<Landed>>, String> {
        match &via {
            Delivery::Send(files) => self.with_files(item, files)?,
            Delivery::Hold { files, .. } => {
                item.held = true;
                item.files = self.hold(&held_key(&item.id), files)?;
            }
            Delivery::Reply { .. } | Delivery::LoopIn => {}
        }
        if !item.held {
            moved(item, InboxState::New, None, None, None);
            item.sender_told = 1;
        }
        self.keep_item(item)?;

        // A send wakes before writing its row; an answer writes the row first.
        let (wait, sent_row, land_after_row) = match via {
            Delivery::Send(_) => {
                self.finish_delivery(item, true);
                (None, true, None)
            }
            Delivery::Hold { prompted, .. } => {
                (Some(self.await_consent(item.clone(), prompted)), true, None)
            }
            Delivery::Reply { count } => (None, true, Some(count)),
            Delivery::LoopIn => (None, false, Some(true)),
        };
        if sent_row {
            self.threads().write(
                &item.from_thread,
                &item.from,
                &TranscriptEvent::InboxSent {
                    id: format!("inbox-sent:{}", item.id),
                    ts: item.ts,
                    item: item.id.clone(),
                    to: item.to.clone(),
                    title: item.title.clone(),
                },
            );
        }
        if let Some(count) = land_after_row {
            self.finish_delivery(item, count);
        }
        Ok(wait)
    }

    /// Counts and wakes only once an item is visible, including after consent.
    pub(super) fn finish_delivery(self: &Arc<Self>, item: &InboxItem, count: bool) {
        if count && let Some(key) = thread_key(&item.from, &item.to) {
            self.count_wake(&key, &item.from, &item.to);
        }
        self.arrived(item);
    }
}
