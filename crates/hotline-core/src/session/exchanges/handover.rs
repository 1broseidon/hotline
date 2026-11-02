//! How a queued request is taken up and handed to the conversation that
//! works it: the person's first-contact card, the pair's brake, and a
//! handoff's work thread. Where a question goes is `questions.rs`.

use super::*;

impl Room {
    /// Authorizes a queued request against both teammates' leases, counts it
    /// toward the pair's brake, and hands it over: a handoff to `thread`, or
    /// to a new one, and an ask to the conversation that answers it.
    pub(super) async fn admit_request(
        self: &Arc<Self>,
        key: &str,
        request: &Request,
        thread: Option<String>,
    ) -> Result<(), String> {
        let caller = self.participant(&request.from)?;
        let target = self.persona(&request.to)?;
        let (caller_capability, target_capability) = lock(&self.exchange_leases)
            .entry(request.id.clone())
            .or_insert_with(|| {
                (
                    self.request_lease(&caller.id),
                    self.request_lease(&target.id),
                )
            })
            .clone();
        let auth = tokio::select! {
            biased;
            () = peers::capabilities_expired(&caller_capability, &target_capability) => {
                self.stop_revoked_exchanges(None);
                return Ok(());
            }
            auth = self.authorize_exchange(request, &caller, &target, &caller_capability, &target_capability) => auth,
        };
        if let Err(error) = auth {
            self.set_exchange_reply(key, &request.id, error, true)?;
            return Ok(());
        }
        let request = {
            let _guard = lock(&self.exchange_lock);
            let mut pair = self.exchange_pair(key).ok_or("Exchange disappeared")?;
            if pair.paused {
                return Ok(());
            }
            let Some(r) = pair
                .requests
                .iter_mut()
                .find(|r| r.id == request.id && r.phase() == Phase::Queued)
            else {
                return Ok(());
            };
            // Its files go into the recipient's working directory only now
            // that it is let in, and the message says where.
            if !r.files.is_empty() {
                match self.deliver_held(&r.id, &r.from, &r.to, &r.files) {
                    Ok(line) => {
                        r.message = format!("{}\n\n{line}", r.message);
                        r.files.clear();
                    }
                    Err(error) => {
                        r.advance(Event::Answered { failed: true })?;
                        r.reply = format!("The files did not reach them: {error}");
                        return self.save_pair(&pair);
                    }
                }
            }
            r.advance(Event::Admitted)?;
            if !r.request_counted {
                r.request_counted = true;
                pair.exchanges += 1;
            }
            let request = r.clone();
            pair.paused = pair.exchanges >= EXCHANGE_CAP;
            self.save_pair(&pair)?;
            if pair.paused {
                self.exchange_card(&pair, ExchangePauseStatus::Pending);
            }
            request
        };
        self.hand_over_request(key, &request, thread).await
    }
    /// Hands a handoff to a work thread on the teammate it was handed to, and
    /// its result is the reply the thread gives when it has done the work.
    /// `thread` is the one it continues, or its own from before a restart;
    /// without one, it opens a thread with the handoff as its first message.
    pub(super) async fn dispatch_handoff(
        self: &Arc<Self>,
        key: &str,
        request: &Request,
        thread: Option<String>,
    ) -> Result<(), String> {
        let caller = self.participant(&request.from)?;
        self.exchange_thread_line(key, &request.from, &request.id, &request.message)?;
        let handoff = HandoffStart {
            key: key.to_string(),
            request: request.id.clone(),
        };
        let side_id = thread.unwrap_or_else(new_id);
        self.set_request_thread(key, &request.id, &side_id)?;
        // A handoff stands again in a thread whose earlier one was released.
        self.mark_handoffs(std::slice::from_ref(&side_id));
        // A thread that has run before comes back with what was said in it.
        // One the last process left live was parked when this room was
        // built (`Room::settle`), before any request is taken up.
        let ran_before =
            self.sides.get(&side_id).is_some() || self.link_of(&ThreadId::side(&side_id)).is_some();
        let started = if ran_before {
            self.wake_side(&side_id, true, Some(handoff)).await
        } else {
            self.bring_up(
                &request.to,
                Start {
                    thread: ThreadId::side(&side_id),
                    title: crate::session::sides::title_of(&request.message),
                    started: now_ms(),
                    opener: Some(Opener {
                        persona_id: caller.id.clone(),
                        name: crate::session::links::shown(&caller),
                    }),
                    fresh: true,
                    saved: None,
                    handoff: Some(handoff),
                    asked: None,
                },
            )
            .map(|(side, launch)| (side, Some(launch)))
        };
        let launch = match started {
            Ok((_, launch)) => launch,
            Err(error) => {
                let target = self.persona(&request.to)?;
                return self.set_exchange_reply(
                    key,
                    &request.id,
                    format!("{} could not start a thread for this: {error}", target.name),
                    true,
                );
            }
        };
        let delivered = self
            .deliver_into_work(
                &side_id,
                &format!("handoff:{}", request.id),
                DeliveryCause::Handoff {
                    request_id: request.id.clone(),
                    persona_id: caller.id.clone(),
                    name: caller.name,
                    thread_key: key.into(),
                    about: peers::about(&request.message),
                },
                DeliveryFrom::new(&ThreadId::dm(&caller.id), Some(request.id.clone())),
                request.message.clone(),
                Some(RequestLine {
                    request: request.id.clone(),
                    answer: None,
                }),
            )
            .await;
        // The line is queued first, so the agent's start finds it waiting.
        if let Some(launch) = launch {
            self.launch(launch);
        }
        // A follow-up into a thread at work on its sender's handoff steers it.
        if delivered.is_ok()
            && let Some(side) = self.sides.get(&side_id)
        {
            self.steer_work(&side, &request.from, &request.id);
        }
        delivered
    }
}
