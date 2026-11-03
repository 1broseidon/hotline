//! Requests that cross a link: forwarded to a linked desk, accepted from one,
//! stopped there by the person, and failed when the link ends; and the notes
//! a work thread here leaves a sender on a linked desk, sent there as asides.
//!
//! Each desk resolves what crosses in its own record. A request id from the
//! other desk is only the key a request is looked up by, a thread it names is
//! looked up among this desk's own handoffs from that sender, and a note or a
//! question back is honoured only for a handoff this desk sent that desk.

use super::*;
use crate::contract::{Aside, AsideLevel, AsideSource, LinkedDelivery, Persona};
use crate::linked::CallError;

/// The most requests a linked desk's teammates may have waiting here at once:
/// four pairs paused at their cap.
const LINKED_WAITING_MAX: usize = 4 * EXCHANGE_CAP as usize;

/// A work thread on a linked recipient's desk, named as that desk names it
/// (`side:<id>`, or `deskId/side:<id>` with the recipient's desk), which that
/// desk resolves among its own handoffs from this sender. None for anything
/// else.
pub(super) fn linked_thread(named: &str, target: &Persona) -> Option<String> {
    let desk = &target.node.as_ref()?.id;
    let thread = ThreadId::parse(named)?;
    (thread.kind == ThreadKind::Side
        && thread.desk.as_ref().is_none_or(|named| named == desk)
        && linked::valid_persona_id(&thread.key))
    .then(|| format!("side:{}", thread.key))
}

impl Room {
    /// Takes up a request to a linked teammate: the pair's brake applies as
    /// it does to a local one, then it crosses the link with its files. It
    /// stays queued while the link is down, and is running once the other
    /// desk has it.
    pub(super) async fn forward_queued(
        self: &Arc<Self>,
        key: &str,
        request: &Request,
    ) -> Result<(), String> {
        if self.exchange_pair(key).is_none_or(|pair| pair.paused) {
            return Ok(());
        }
        let files = match self.held(&request.id, &request.files) {
            Ok(files) => crate::session::files_between::to_link(files),
            Err(error) => return self.set_exchange_reply(key, &request.id, error, true),
        };
        let outgoing = Outgoing {
            to: &request.to,
            message: &request.message,
            intent: request.intent,
            thread: request.continues.as_deref(),
            files: None,
        };
        let forwarded = self
            .forward_linked(
                &request.id,
                &request.from,
                &outgoing,
                request.about.as_deref(),
                files,
            )
            .await;
        let forwarded = match forwarded {
            Ok(forwarded) => forwarded,
            Err(error) => return self.set_exchange_reply(key, &request.id, error, true),
        };
        if !forwarded {
            return Ok(());
        }
        self.drop_held(&request.id);
        self.exchange_thread_line(key, &request.from, &request.id, &request.message)?;
        let _guard = lock(&self.exchange_lock);
        let mut pair = self.exchange_pair(key).ok_or("Exchange disappeared")?;
        let Some(r) = pair
            .requests
            .iter_mut()
            .find(|r| r.id == request.id && r.phase() == Phase::Queued)
        else {
            return Ok(());
        };
        r.advance(Event::Forwarded)?;
        r.files.clear();
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

    /// Queues a request a linked desk sent for one of this desk's teammates,
    /// holding the files it carries until it is let in. The same request
    /// sent again, after a channel closed before it was answered, is the one
    /// already queued; so is one the person here stopped while that stop has
    /// still to cross, which is what the other desk is to hear.
    pub(in crate::session) fn accept_linked_request(
        self: &Arc<Self>,
        id: &str,
        from: &str,
        outgoing: &Outgoing<'_>,
        about: Option<String>,
        files: &[(String, Vec<u8>)],
    ) -> Result<(), String> {
        let _working = self.working()?;
        let to = outgoing.to;
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
                    Some(r) if r.stopped_by_person() && r.owed() == Owed::Desk => Ok(()),
                    Some(r) if matches!(r.phase(), Phase::Done | Phase::Stopped) => {
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
            let desk = linked::split(from).map(|(desk, _)| desk);
            let waiting = pairs
                .iter()
                .flat_map(|pair| &pair.requests)
                .filter(|r| Self::exchange_pending(r))
                .filter(|r| linked::split(&r.from).map(|(desk, _)| desk) == desk)
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
            // A thread named as this desk names it is found among this
            // sender's own handoffs here, now: that desk cannot know it any
            // other way, and a name no handoff of its opened is refused.
            let continues = match outgoing.thread {
                Some(named) if named.starts_with("side:") => Some(
                    pair.thread_named(named, from, to)
                        .ok_or("That work thread was not opened by this teammate's handoff.")?,
                ),
                other => other.map(str::to_string),
            };
            let names = self.hold(id, files)?;
            pair.requests.push(Request {
                id: id.to_string(),
                from: from.to_string(),
                to: to.to_string(),
                message: outgoing.message.to_string(),
                intent: outgoing.intent,
                state: State::Queued,
                reply: String::new(),
                answer: String::new(),
                request_counted: false,
                human_actions: vec![],
                thread: None,
                reply_thread: None,
                reply_job: None,
                lead_call: None,
                continues,
                released: false,
                about,
                own_thread: false,
                looped_in: false,
                files: names,
                crossing: Vec::new(),
            });
            if let Err(error) = self.save_pair(&pair) {
                self.drop_held(id);
                return Err(error);
            }
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
                        r.id == id && linked::split(&r.to).is_some_and(|(desk, _)| desk == desk_id)
                    })
                })
                .ok_or("This desk sent no such request.")?;
            let request = pair
                .requests
                .iter_mut()
                .find(|r| r.id == id)
                .expect("the pair was found by this request, under the exchange lock");
            if !matches!(request.phase(), Phase::Queued | Phase::Running) {
                return Ok(());
            }
            request.advance(Event::Answered { failed })?;
            request.reply = reply;
            self.save_pair(&pair)?;
            pair.id
        };
        self.wake_exchange(&key);
        Ok(())
    }

    /// Ends a handoff this desk forwarded to `desk_id` that the person there
    /// stopped, as one the person stopped here ends: released, its leases
    /// revoked, and its sender told once by a note aside of `report`, what
    /// the colleague had said and the person's sentence, never woken nor
    /// handed a failure to retry. One this desk still holds as queued, whose
    /// forward was answered too late to be heard, is ended too. A stop said
    /// of an ask is only its failure.
    pub(in crate::session) fn stop_linked_request(
        self: &Arc<Self>,
        desk_id: &str,
        id: &str,
        report: String,
    ) -> Result<(), String> {
        let sent_there = |r: &Request| {
            r.id == id && linked::split(&r.to).is_some_and(|(desk, _)| desk == desk_id)
        };
        let handoff = self
            .exchange_pairs()
            .iter()
            .flat_map(|pair| &pair.requests)
            .find(|r| sent_there(r))
            .map(|r| r.intent == Intent::Handoff)
            .ok_or("This desk sent no such request.")?;
        if !handoff {
            return self.settle_linked_request(desk_id, id, report, true);
        }
        let mut ended = Vec::new();
        let stopped = {
            let _guard = lock(&self.exchange_lock);
            let Some(mut pair) = self
                .exchange_pairs()
                .into_iter()
                .find(|pair| pair.requests.iter().any(sent_there))
            else {
                return Ok(());
            };
            let request = pair
                .requests
                .iter_mut()
                .find(|r| sent_there(r))
                .expect("the pair was found by it");
            if !matches!(
                request.phase(),
                Phase::Queued | Phase::Running | Phase::WaitingHuman
            ) {
                return Ok(());
            }
            if !self.end_request(
                request,
                Ending::Person,
                peers::HANDOFF_STOPPED_BY_THE_PERSON,
                &mut ended,
            ) {
                return Ok(());
            }
            let stopped = request.clone();
            self.save_pair(&pair)?;
            (pair.id, stopped)
        };
        self.drop_held(id);
        self.mark_handoffs(&ended);
        self.note_stopped(&stopped.0, &stopped.1, report);
        Ok(())
    }

    /// A link ended: every request still waiting to or from a teammate of
    /// that desk fails, and each sender here is told in a sentence.
    pub(in crate::session) fn stop_linked_exchanges(&self, desk_id: &str, desk_name: &str) {
        let theirs = |id: &str| linked::split(id).is_some_and(|(desk, _)| desk == desk_id);
        let _guard = lock(&self.exchange_lock);
        let (mut stopped, mut ended) = (Vec::new(), Vec::new());
        for mut pair in self.exchange_pairs() {
            if !theirs(&pair.a) && !theirs(&pair.b) {
                continue;
            }
            let mut changed = false;
            for request in &mut pair.requests {
                changed |= request.release(&mut ended);
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
                // Its desk is not linked to be told.
                self.end_request(
                    request,
                    Ending::Unlinked,
                    format!("{name} is on {desk_name}, which isn't linked any more."),
                    &mut ended,
                );
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
            request.advance(Event::Told)?;
        }
        self.save_pair(&pair)
    }

    /// Keeps a note a work thread here leaves the sender of the handoff it
    /// serves, a teammate on a linked desk, for the pair's worker to send
    /// that desk ([`Room::send_crossing`]). While the handoff stands only.
    pub(super) fn keep_for_link(&self, key: &str, request_id: &str, aside: Aside) {
        {
            let _guard = lock(&self.exchange_lock);
            let Some(mut pair) = self.exchange_pair(key) else {
                return;
            };
            let Some(request) = pair
                .requests
                .iter_mut()
                .find(|r| r.id == request_id && r.stands())
            else {
                return;
            };
            request.crossing.push(aside);
            if let Err(error) = self.save_pair(&pair) {
                eprintln!("keep a note for a linked desk: {error}");
                return;
            }
        }
        if let Some(room) = lock(&self.me).upgrade() {
            room.wake_exchange(key);
        }
    }

    /// Sends a linked sender's desk, one by one, the notes a handoff's
    /// thread here has left it, as they stand now: none a stop, a revocation
    /// or the link's end has dropped since. They wait while the link is down
    /// and until its roster has said what the other desk takes; a desk that
    /// does not take asides, as one older than them, is sent none, and they
    /// are dropped, while the handoff's result still goes.
    pub(super) async fn send_crossing(&self, key: &str, id: &str) {
        let Some(request) = self
            .exchange_pair(key)
            .and_then(|pair| pair.requests.into_iter().find(|r| r.id == id))
        else {
            return;
        };
        let Some((desk_id, persona_id)) = linked::split(&request.from) else {
            return;
        };
        let mut sent = 0;
        match self.links.takes(desk_id, linked::ASIDES) {
            None => return,
            Some(false) => sent = request.crossing.len(),
            Some(true) => {
                for aside in &request.crossing {
                    let AsideSource::Thread(from) = &aside.from else {
                        sent += 1;
                        continue;
                    };
                    let delivery = LinkedDelivery::Aside {
                        id: aside.id.clone(),
                        from_persona_id: linked::split(&request.to)
                            .map_or(request.to.as_str(), |(_, persona)| persona)
                            .to_string(),
                        to: persona_id.to_string(),
                        from: from.clone(),
                        text: aside.text.clone(),
                        level: AsideLevel::Note,
                        about: None,
                        request_id: Some(request.id.clone()),
                        files: Vec::new(),
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
                        Err(CallError::Offline) => break,
                        // Its handoff is over there, or that desk would not
                        // take it: it is not sent again.
                        Err(CallError::Refused(error)) => {
                            eprintln!("[link] a note for {} was refused: {error}", request.from);
                        }
                    }
                    sent += 1;
                }
            }
        }
        if sent == 0 {
            return;
        }
        let _guard = lock(&self.exchange_lock);
        let Some(mut pair) = self.exchange_pair(key) else {
            return;
        };
        if let Some(kept) = pair.requests.iter_mut().find(|r| r.id == id) {
            let gone: Vec<&str> = request.crossing[..sent]
                .iter()
                .map(|aside| aside.id.as_str())
                .collect();
            kept.crossing
                .retain(|aside| !gone.contains(&aside.id.as_str()));
            if let Err(error) = self.save_pair(&pair) {
                eprintln!("clear notes sent over a link: {error}");
            }
        }
    }

    /// Hears a note a linked desk sent from the work thread a handoff of
    /// this desk's opened there, and keeps it as an aside to that handoff's
    /// sender, heard where its answer would go. `request_id` is only the key
    /// the handoff is looked up by: it must be one this desk sent `by`, a
    /// teammate of `desk_id`, and still stand. It needs no other grant, as
    /// the handoff's result needs none, and it is a note whatever level it
    /// says: whether anyone is woken is this desk's to decide.
    pub(in crate::session) fn receive_linked_note(
        &self,
        desk_id: &str,
        request_id: &str,
        by: &str,
        mut aside: Aside,
    ) -> Result<(), String> {
        let handoff = self
            .exchange_pairs()
            .into_iter()
            .flat_map(|pair| pair.requests)
            .find(|r| r.id == request_id && r.intent == Intent::Handoff && r.to == by)
            .filter(|r| linked::split(&r.to).is_some_and(|(desk, _)| desk == desk_id))
            .ok_or("This desk sent no such handoff.")?;
        if !handoff.stands() || !self.exchange_lease_current(&handoff.id) {
            return Ok(());
        }
        aside.to.clone_from(&handoff.from);
        if self.aside_kept(&aside.to, &aside.id) {
            return Ok(());
        }
        aside.about = self.route_about(&handoff);
        aside.level = AsideLevel::Note;
        self.keep_aside(&aside)
    }
}
