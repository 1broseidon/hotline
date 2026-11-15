//! Requests that crossed a link before the inbox: the answer a linked desk
//! sends for one forwarded there, the person's stop of one there, and their
//! end when the link ends. The inbox does not cross a link
//! (`docs/design/agent-inbox.md`), so no request crosses one now; these
//! settle what an older version left crossing.

use super::*;

impl Room {
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
        {
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
            if !self.end_request(request, Ending::Person, report, &mut ended) {
                return Ok(());
            }
            self.save_pair(&pair)?;
        }
        self.drop_held(id);
        self.mark_handoffs(&ended);
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
}
