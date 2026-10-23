//! Requests that cross a link: forwarded to a linked desk, accepted from one,
//! and failed when the link ends.

use super::*;

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
        let forwarded = self
            .forward_linked(
                &request.id,
                &request.from,
                &request.to,
                &request.message,
                request.intent,
                request.continues.as_deref(),
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
        to: &str,
        message: &str,
        intent: Intent,
        continues: Option<String>,
    ) -> Result<(), String> {
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
                // Sent again after it was settled here: it is over, and says so.
                return match pair.requests.iter().find(|r| r.id == id) {
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
                message: message.to_string(),
                intent,
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
                reply_thread: None,
                reply_job: None,
                returned: false,
                lead_call: None,
                continues,
                progress: Vec::new(),
                progress_left_out: 0,
                progress_wakes: false,
                released: false,
                about: None,
                owed_to_thread: false,
            });
            self.save_pair(&pair)?;
            lock(&self.exchange_leases).insert(
                id.to_string(),
                (
                    self.capability_lease(from).scoped(),
                    self.capability_lease(to).scoped(),
                ),
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

    /// A link ended: every request still waiting to or from a teammate of
    /// that desk fails, and each sender here is told in a sentence.
    pub(in crate::session) fn stop_linked_exchanges(&self, desk_id: &str, desk_name: &str) {
        let theirs = |id: &str| crate::linked::split(id).is_some_and(|(desk, _)| desk == desk_id);
        let _guard = lock(&self.exchange_lock);
        let mut stopped = Vec::new();
        for mut pair in self.exchange_pairs() {
            if !theirs(&pair.a) && !theirs(&pair.b) {
                continue;
            }
            let mut changed = false;
            for request in &mut pair.requests {
                changed |= progress::release(request);
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
                self.cancel_handoff(request);
                request.phase = Phase::Stopped;
                request.failed = true;
                request.reply = format!("{name} is on {desk_name}, which isn't linked any more.");
                request.owed_to_thread = self.delegated_origin(request).is_some();
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
    /// Sees that a stopped request's sender is told, by the pair's worker,
    /// whichever path stopped it.
    pub(super) fn wake_owed(&self, pair: &Pair) {
        if pair.requests.iter().any(Self::owes_answer)
            && let Some(room) = lock(&self.me).upgrade()
        {
            room.wake_exchange(&pair.id);
        }
    }
}
