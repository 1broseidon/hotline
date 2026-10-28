//! Requests that cross a link: forwarded to a linked desk, accepted from one,
//! and failed when the link ends; and the reports of the thread a linked
//! desk's handoff opened here, sent there.

use super::*;
use crate::contract::LinkedDelivery;
use crate::linked::CallError;

/// The most requests a linked desk's teammates may have waiting here at once:
/// four pairs paused at their cap.
const LINKED_WAITING_MAX: usize = 4 * EXCHANGE_CAP as usize;

impl Room {
    /// Takes up a request to a linked teammate: the pair's brake applies as
    /// it does to a local one, then it crosses the link. It stays queued while
    /// the link is down, and is running once the other desk has it.
    pub(super) async fn forward_queued(
        self: &Arc<Self>,
        key: &str,
        request: &Request,
    ) -> Result<(), String> {
        if self.exchange_pair(key).is_none_or(|pair| pair.paused) {
            return Ok(());
        }
        let outgoing = Outgoing {
            to: &request.to,
            message: &request.message,
            intent: request.intent,
            thread: request.continues.as_deref(),
        };
        let forwarded = self
            .forward_linked(
                &request.id,
                &request.from,
                &outgoing,
                request.about.as_deref(),
            )
            .await;
        let forwarded = match forwarded {
            Ok(forwarded) => forwarded,
            Err(error) => return self.set_exchange_reply(key, &request.id, error, true),
        };
        if !forwarded {
            return Ok(());
        }
        self.exchange_thread_line(key, &request.from, &request.id, &request.message)?;
        let _guard = lock(&self.exchange_lock);
        let mut pair = self.exchange_pair(key).ok_or("Exchange disappeared")?;
        let Some(r) = pair
            .requests
            .iter_mut()
            .find(|r| r.id == request.id && r.phase == Phase::Queued)
        else {
            return Ok(());
        };
        r.phase = Phase::Running;
        r.started = true;
        if !r.request_counted {
            r.request_counted = true;
            pair.exchanges += 1;
        }
        pair.paused = pair.exchanges >= EXCHANGE_CAP;
        self.save_pair(&pair)?;
        if pair.paused {
            self.exchange_card(&pair, ExchangePauseStatus::Pending);
        }
        Ok(())
    }

    /// Queues a request a linked desk sent for one of this desk's teammates.
    /// The same request sent again, after a channel closed before it was
    /// answered, is the one already queued.
    pub(in crate::session) fn accept_linked_request(
        self: &Arc<Self>,
        id: &str,
        from: &str,
        outgoing: &Outgoing<'_>,
        about: Option<String>,
    ) -> Result<(), String> {
        let to = outgoing.to;
        let _working = self.working()?;
        let key = thread_key(from, to).ok_or("Invalid teammate pair")?;
        {
            let _guard = lock(&self.exchange_lock);
            let pairs = self.exchange_pairs();
            if let Some(pair) = pairs
                .iter()
                .find(|pair| pair.requests.iter().any(|r| r.id == id))
            {
                if pair.id != key {
                    return Err("That request id belongs to another exchange.".into());
                }
                // Sent again after it was settled here: it is over, and says so,
                // unless the person here stopped it and that stop has still to
                // cross. It is what the other desk is to hear, as their stop,
                // so the request is taken as still queued meanwhile.
                return match pair.requests.iter().find(|r| r.id == id) {
                    Some(r) if r.stopped_by_person && !r.returned => Ok(()),
                    Some(r) if matches!(r.phase, Phase::Done | Phase::Stopped) => {
                        Err(if r.reply.is_empty() {
                            "That message was already answered.".to_string()
                        } else {
                            r.reply.clone()
                        })
                    }
                    _ => Ok(()),
                };
            }
            // A linked desk's teammates may have only so much waiting here
            // at once, whatever that desk sends.
            let desk = crate::linked::split(from).map(|(desk, _)| desk);
            let waiting = pairs
                .iter()
                .flat_map(|pair| &pair.requests)
                .filter(|r| Self::exchange_pending(r))
                .filter(|r| crate::linked::split(&r.from).map(|(desk, _)| desk) == desk)
                .count();
            if waiting >= LINKED_WAITING_MAX {
                return Err("Too many messages from that desk are already waiting here.".into());
            }
            let mut pair = pairs.into_iter().find(|p| p.id == key).unwrap_or(Pair {
                id: key.clone(),
                a: from.to_string(),
                b: to.to_string(),
                exchanges: 0,
                paused: false,
                requests: vec![],
            });
            pair.requests.push(Request {
                id: id.to_string(),
                from: from.to_string(),
                to: to.to_string(),
                message: outgoing.message.to_string(),
                intent: outgoing.intent,
                phase: Phase::Queued,
                reply: String::new(),
                failed: false,
                reply_counted: false,
                request_counted: false,
                inline: false,
                started: false,
                result_consumed: false,
                human_actions: vec![],
                thread: None,
                taken_up_in: None,
                reply_thread: None,
                reply_job: None,
                returned: false,
                lead_call: None,
                continues: outgoing.thread.map(str::to_string),
                progress: Vec::new(),
                progress_left_out: 0,
                progress_wakes: false,
                released: false,
                about,
                owed_to_thread: false,
                stopped_by_person: false,
            });
            self.save_pair(&pair)?;
            lock(&self.exchange_leases).insert(
                id.to_string(),
                (self.request_lease(from), self.request_lease(to)),
            );
        }
        self.wake_exchange(&key);
        Ok(())
    }

    /// Saves the answer a linked desk sent for a request forwarded to it.
    pub(in crate::session) fn settle_linked_request(
        self: &Arc<Self>,
        desk_id: &str,
        id: &str,
        reply: String,
        failed: bool,
    ) -> Result<(), String> {
        let key = {
            let _guard = lock(&self.exchange_lock);
            let mut pair = self
                .exchange_pairs()
                .into_iter()
                .find(|pair| {
                    pair.requests.iter().any(|r| {
                        r.id == id
                            && crate::linked::split(&r.to).is_some_and(|(desk, _)| desk == desk_id)
                    })
                })
                .ok_or("This desk sent no such request.")?;
            let request = pair
                .requests
                .iter_mut()
                .find(|r| r.id == id)
                .expect("the pair was found by this request, under the exchange lock");
            if !matches!(request.phase, Phase::Queued | Phase::Running) {
                return Ok(());
            }
            request.phase = Phase::Reply;
            request.reply = reply;
            request.failed = failed;
            self.save_pair(&pair)?;
            pair.id
        };
        self.wake_exchange(&key);
        Ok(())
    }

    /// Ends a request forwarded to a linked desk that the person there
    /// stopped, as one the person stopped here ends: its sender is told
    /// once in `report`, without a wake, never handed a failure to retry.
    pub(in crate::session) fn stop_linked_request(
        self: &Arc<Self>,
        desk_id: &str,
        id: &str,
        report: String,
    ) -> Result<(), String> {
        let sent_there = self
            .exchange_pairs()
            .iter()
            .flat_map(|pair| &pair.requests)
            .any(|r| {
                r.id == id && crate::linked::split(&r.to).is_some_and(|(desk, _)| desk == desk_id)
            });
        if !sent_there {
            return Err("This desk sent no such request.".into());
        }
        self.end_for_the_person(id, true, Cut::Stop, Some(report));
        Ok(())
    }

    /// Hears a report a linked desk sent from the thread a handoff of this
    /// desk's opened there. Its request id is only the key the handoff is
    /// looked up by here; the report is cut to what a thread's summary
    /// holds, and whether it wakes anyone is this desk's to decide.
    pub(in crate::session) fn receive_linked_progress(
        self: &Arc<Self>,
        desk_id: &str,
        request_id: &str,
        text: String,
        wakes: bool,
    ) -> Result<(), String> {
        let (key, handoff) = self
            .exchange_pairs()
            .into_iter()
            .find_map(|pair| {
                let handoff = pair.requests.into_iter().find(|r| {
                    r.id == request_id
                        && r.intent == Intent::Handoff
                        && crate::linked::split(&r.to).is_some_and(|(desk, _)| desk == desk_id)
                })?;
                Some((pair.id, handoff))
            })
            .ok_or("This desk sent no such request.")?;
        let text = crate::driver::clip(text.trim(), progress::PROGRESS_CHARS);
        if text.is_empty() || !self.delegation_live(&handoff) {
            return Ok(());
        }
        self.hear_progress(&key, request_id, &text, wakes)
    }

    /// Sends a linked sender, in one, the reports its handoff's thread made
    /// here, as they stand now: none a stop or a revocation has erased. They
    /// wait while the link is down; a desk that refuses them, as one from
    /// before reports does, never takes them, so they are dropped.
    pub(super) async fn send_linked_progress(&self, key: &str, id: &str) {
        let request = {
            let _guard = lock(&self.exchange_lock);
            let Some(request) = self
                .exchange_pair(key)
                .and_then(|pair| pair.requests.into_iter().find(|r| r.id == id))
                .filter(Self::owes_linked_progress)
            else {
                return;
            };
            request
        };
        let Some((desk_id, _)) = crate::linked::split(&request.from) else {
            return;
        };
        let left_out = match request.progress_left_out {
            0 => String::new(),
            n => format!("\n({n} earlier reports were left out.)"),
        };
        let delivery = LinkedDelivery::Progress {
            request_id: request.id.clone(),
            text: format!("{}{left_out}", request.progress.join("\n\n")),
            wakes: request.progress_wakes,
        };
        match self
            .links
            .call(
                desk_id,
                "federation.deliver",
                json!({ "delivery": delivery }),
            )
            .await
        {
            Ok(_) => {}
            Err(CallError::Offline) => return,
            Err(CallError::Refused(error)) => {
                eprintln!("[link] a report for {} was refused: {error}", request.from);
            }
        }
        let _guard = lock(&self.exchange_lock);
        let Some(mut pair) = self.exchange_pair(key) else {
            return;
        };
        if let Some(kept) = pair.requests.iter_mut().find(|r| r.id == request.id) {
            take_sent(kept, &request);
            if let Err(error) = self.save_pair(&pair) {
                eprintln!("clear reports sent over a link: {error}");
            }
        }
    }

    /// A link ended: every request still waiting to or from a teammate of
    /// that desk fails, and each sender here is told in a sentence.
    pub(in crate::session) fn stop_linked_exchanges(&self, desk_id: &str, desk_name: &str) {
        let theirs = |id: &str| crate::linked::split(id).is_some_and(|(desk, _)| desk == desk_id);
        let _guard = lock(&self.exchange_lock);
        let (mut stopped, mut ended) = (Vec::new(), Vec::new());
        for mut pair in self.exchange_pairs() {
            if !theirs(&pair.a) && !theirs(&pair.b) {
                continue;
            }
            let mut changed = false;
            for request in &mut pair.requests {
                changed |= progress::release(request, &mut ended);
                if !Self::exchange_pending(request) {
                    continue;
                }
                let linked_side = if theirs(&request.to) {
                    &request.to
                } else {
                    &request.from
                };
                let name = self
                    .participant(linked_side)
                    .map_or_else(|_| "That teammate".to_string(), |persona| persona.name);
                request.owed_to_thread = self.owes_thread(request, None);
                self.cancel_handoff(request);
                request.phase = Phase::Stopped;
                request.failed = true;
                request.reply = format!("{name} is on {desk_name}, which isn't linked any more.");
                // Its desk is not linked to be told.
                request.returned = true;
                stopped.push(request.clone());
                changed = true;
            }
            if changed && let Err(error) = self.save_pair(&pair) {
                eprintln!("unlink exchange: {error}");
            }
            self.wake_owed(&pair);
        }
        self.mark_handoffs(&ended);
        self.exchange_notices(&stopped);
    }

    pub(super) fn mark_returned(&self, key: &str, id: &str) -> Result<(), String> {
        let _guard = lock(&self.exchange_lock);
        let mut pair = self.exchange_pair(key).ok_or("Exchange disappeared")?;
        if let Some(request) = pair.requests.iter_mut().find(|r| r.id == id) {
            request.returned = true;
        }
        self.save_pair(&pair)
    }
    /// A request stopped here whose sender has not been told: one from a
    /// linked teammate whose desk has not heard, or an ask whose thread is
    /// owed its failure. Its sender is waiting on an answer until it is.
    pub(super) fn owes_answer(request: &Request) -> bool {
        request.phase == Phase::Stopped
            && (request.owed_to_thread
                || (crate::linked::is_linked(&request.from) && !request.returned))
    }
    /// A handoff from a linked teammate whose thread has reports here still
    /// to send it.
    pub(super) fn owes_linked_progress(request: &Request) -> bool {
        crate::linked::is_linked(&request.from) && !request.progress.is_empty()
    }
    /// Sees that a stopped request's sender is told, by the pair's worker,
    /// whichever path stopped it.
    pub(super) fn wake_owed(&self, pair: &Pair) {
        if pair
            .requests
            .iter()
            .any(|r| Self::owes_answer(r) || Self::owes_linked_progress(r))
            && let Some(room) = lock(&self.me).upgrade()
        {
            room.wake_exchange(&pair.id);
        }
    }
}

/// Takes from a handoff's reports the ones `sending` held when they were
/// sent: those still kept, and those the limit has dropped since. What a
/// turn added meanwhile stays, to go next.
pub(super) fn take_sent(kept: &mut Request, sending: &Request) {
    let dropped_since = kept
        .progress_left_out
        .saturating_sub(sending.progress_left_out) as usize;
    let sent = sending.progress.len();
    let still_kept = sent.saturating_sub(dropped_since).min(kept.progress.len());
    kept.progress.drain(..still_kept);
    kept.progress_left_out = dropped_since.saturating_sub(sent) as u32;
    kept.progress_wakes &= !kept.progress.is_empty();
}
