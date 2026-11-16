//! Work taken from the inbox: the receiver's `update_inbox` takes an item
//! that wants work, and the item becomes a handoff request it runs in a work
//! thread of its own (`docs/design/agent-inbox.md`).
//!
//! The request is the one a handoff always was, so the same machinery holds
//! it: a place among the receiver's live threads ([`crate::session::sides::MAX_LIVE`],
//! waiting its turn when every one is mid-turn), the cards its thread raises,
//! the person's Stop, a restart. What differs is who starts it and where its
//! result goes. The receiver starts it, never the sender; and what it comes
//! to is an item in the sender's inbox that answers the one taken, which
//! closes then, while a request that ends without a result closes it as
//! dropped. The first-contact card was asked when the item was sent, so the
//! request needs none of its own.

use super::*;
use crate::contract::{InboxItem, InboxOutcome, Persona};

impl Room {
    /// Takes `items` into a work thread of `receiver`'s, as the request `id`
    /// the taken item was claimed for: the first is the item taken, the
    /// rest those combined into it, whose words all seed the thread. The
    /// thread is opened when the request is let in.
    pub(in crate::session) fn take_into_work(
        self: &Arc<Self>,
        receiver: &Persona,
        items: &[InboxItem],
        id: &str,
    ) -> Result<(), String> {
        let taken = items.first().ok_or("There is no item to take.")?;
        let key = thread_key(&taken.from, &receiver.id).ok_or("Invalid teammate pair")?;
        let id = id.to_string();
        let leases = (
            self.request_lease(&taken.from),
            self.request_lease(&receiver.id),
        );
        let message = items
            .iter()
            .map(|item| {
                let sender = self
                    .participant(&item.from)
                    .map_or_else(|_| item.from.clone(), |p| crate::session::links::shown(&p));
                let words = if item.body.is_empty() {
                    "(No message beyond its title.)"
                } else {
                    item.body.as_str()
                };
                // The title first: the thread is named by its first line.
                format!(
                    "{}\nFrom {sender}, item {}, wants {}:\n{words}",
                    item.title,
                    crate::session::inbox::short(&item.id),
                    item.wants.name()
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");
        {
            let _guard = lock(&self.exchange_lock);
            let mut pair = self.pair_or_new(&key, &taken.from, &receiver.id)?;
            pair.requests.push(Request {
                id: id.clone(),
                from: taken.from.clone(),
                to: receiver.id.clone(),
                message,
                intent: Intent::Handoff,
                state: State::Queued,
                reply: String::new(),
                answer: String::new(),
                // The item was counted on the brake when it was sent.
                request_counted: true,
                human_actions: vec![],
                thread: None,
                reply_thread: matches!(taken.from_thread.kind, ThreadKind::Side | ThreadKind::Call)
                    .then(|| taken.from_thread.key.clone()),
                reply_job: None,
                lead_call: None,
                continues: None,
                released: false,
                about: None,
                own_thread: false,
                looped_in: false,
                files: Vec::new(),
                items: items.iter().map(|item| item.id.clone()).collect(),
            });
            self.save_pair(&pair)?;
            lock(&self.exchange_leases).insert(id, leases);
        }
        self.wake_exchange(&key);
        Ok(())
    }

    /// A taken item's work thread is open: each of its items says where.
    pub(super) fn inbox_work_started(&self, request: &Request, side_id: &str) {
        let thread = self.work_thread(side_id);
        for id in &request.items {
            self.change_item(&request.to, id, |item| {
                if item.work.as_ref() == Some(&thread) {
                    return false;
                }
                item.work = Some(thread.clone());
                true
            });
        }
    }

    /// What a taken item's work came to goes back to each sender whose item
    /// it carried, as a reply to that sender's first item in it, heard where
    /// it was sent from; the taken item then closes as done, or as dropped
    /// when the work failed. A reply already sent for an item is not sent
    /// again.
    pub(super) fn reply_into_inbox(self: &Arc<Self>, request: &Request) -> Result<(), String> {
        let worker = self.persona(&request.to)?;
        let thread = request
            .thread
            .as_deref()
            .map_or_else(|| ThreadId::dm(&request.to), |side| self.work_thread(side));
        let text = if request.reply.trim().is_empty() {
            "The work ended without a written result.".to_string()
        } else {
            request.reply.clone()
        };
        let items: Vec<InboxItem> = request
            .items
            .iter()
            .filter_map(|id| {
                crate::store::inbox::of(&self.log, &request.to)
                    .into_iter()
                    .find(|item| item.id == *id)
            })
            .collect();
        // Items this build can no longer read leave nothing to answer.
        let Some(taken) = items.first() else {
            return Ok(());
        };
        let mut told: Vec<&str> = Vec::new();
        for item in &items {
            if told.contains(&item.from.as_str()) {
                continue;
            }
            told.push(&item.from);
            let answered = crate::store::inbox::of(&self.log, &item.from)
                .iter()
                .any(|reply| {
                    reply.reply_to.as_deref() == Some(item.id.as_str()) && reply.from == worker.id
                });
            if !answered {
                self.reply_to_item(&worker, &thread, item, text.clone(), false)?;
            }
        }
        if request.failed() {
            self.close_item(
                &request.to,
                &taken.id,
                InboxOutcome::Dropped,
                Some(hotline_room::text::gist(&text, asides::GIST_CHARS)),
            );
        } else {
            self.close_item(&request.to, &taken.id, InboxOutcome::Done, None);
        }
        Ok(())
    }

    /// A taken item's request ended without a result: its items close as
    /// dropped, with what ended it, which their senders are told at their
    /// next turn's start.
    pub(super) fn inbox_work_dropped(&self, request: &Request, why: &str) {
        for id in &request.items {
            self.close_item(
                &request.to,
                id,
                InboxOutcome::Dropped,
                Some(why.to_string()),
            );
        }
    }
}
