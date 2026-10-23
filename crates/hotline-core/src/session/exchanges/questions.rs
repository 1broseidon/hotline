//! A colleague's question, delivered to the conversation that answers it.

use super::*;

impl Room {
    /// Hands an ask to its recipient's main conversation: a line that starts a
    /// turn there, or waits behind the one in flight, and whose turn's reply
    /// is the ask's answer. The pair's thread keeps the question.
    pub(super) async fn ask_in_main(
        self: &Arc<Self>,
        key: &str,
        request: &Request,
    ) -> Result<(), String> {
        let caller = self.participant(&request.from)?;
        // A conversation quiet past the room's idle window begins its next
        // chapter on a fresh session, which ends the leases of the one the ask
        // was accepted beside. The ask is answered by the session it reaches.
        self.start(&request.to).await?;
        drop(self.in_this_chapter(&request.to).await?);
        self.bind_to_answering_session(&request.id, &request.to)?;
        self.exchange_thread_line(key, &request.from, &request.id, &request.message)?;
        self.deliver_identified(
            &request.to,
            &format!("question:{}", request.id),
            DeliveryCause::Question {
                request_id: request.id.clone(),
                persona_id: caller.id.clone(),
                name: super::super::links::shown(&caller),
                thread_key: key.into(),
                about: peers::about(&request.message),
                work: None,
            },
            request.message.clone(),
            Some(RequestLine {
                request: request.id.clone(),
                answer: None,
            }),
        )
        .await
    }

    /// Hands an ask to the work thread a handoff of its sender's opened: a line
    /// that waits behind the turn in flight, whose turn's reply is the ask's
    /// answer, by the teammate doing the work. A thread that closed or parked
    /// comes back for it, and one that cannot start fails the ask. Only a
    /// handoff's result closes a thread, so this one stays open.
    pub(super) async fn ask_in_thread(
        self: &Arc<Self>,
        key: &str,
        request: &Request,
        thread: String,
    ) -> Result<(), String> {
        let caller = self.participant(&request.from)?;
        self.exchange_thread_line(key, &request.from, &request.id, &request.message)?;
        self.set_handoff_thread(key, &request.id, &thread)?;
        let (_, launch) = self
            .wake_side(
                &thread,
                true,
                Some(HandoffStart {
                    key: key.into(),
                    request: request.id.clone(),
                }),
            )
            .await?;
        let cause = DeliveryCause::Question {
            request_id: request.id.clone(),
            persona_id: caller.id.clone(),
            name: super::super::links::shown(&caller),
            thread_key: key.into(),
            about: peers::about(&request.message),
            work: self
                .serving(&thread)
                .map(|(_, handoff)| peers::about(&handoff.message)),
        };
        let from = self.delivery_source(&request.to, &cause);
        let delivered = self
            .deliver_into_work(
                &thread,
                &format!("question:{}", request.id),
                cause,
                from,
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
        delivered
    }

    /// Fails at once each question the person's cancel dropped from `to`'s
    /// queue before its turn began. Their Stop drops what waited behind the
    /// turn, and a question no turn took up has nothing to inspect.
    pub(in crate::session) fn fail_dropped_questions<'a>(
        &self,
        to: &str,
        dropped: impl IntoIterator<Item = &'a str>,
    ) {
        let dropped: Vec<&str> = dropped.into_iter().collect();
        if dropped.is_empty() {
            return;
        }
        let name = self
            .participant(to)
            .map_or_else(|_| to.to_string(), |p| super::super::links::shown(&p));
        let reply = format!(
            "{name} was stopped before answering this question. Send it again if you still need it."
        );
        let mut failed = Vec::new();
        {
            let _guard = lock(&self.exchange_lock);
            for mut pair in self.exchange_pairs() {
                let mut changed = false;
                for r in &mut pair.requests {
                    if r.intent == Intent::Ask
                        && r.phase == Phase::Running
                        && !r.started
                        && dropped.contains(&r.id.as_str())
                    {
                        r.phase = Phase::Reply;
                        r.failed = true;
                        r.reply.clone_from(&reply);
                        changed = true;
                    }
                }
                if !changed {
                    continue;
                }
                if let Err(error) = self.save_pair(&pair) {
                    eprintln!("fail a dropped question: {error}");
                }
                failed.push(pair.id);
            }
        }
        if let Some(room) = lock(&self.me).upgrade() {
            for key in failed {
                room.wake_exchange(&key);
            }
        }
    }

    /// Ties an ask still at work to its recipient's current session. A stop,
    /// a policy change or a revocation stops the ask itself, so one that is
    /// still running here lost only the session its recipient had.
    fn bind_to_answering_session(&self, id: &str, to: &str) -> Result<(), String> {
        let _guard = lock(&self.exchange_lock);
        let running = self
            .exchange_pairs()
            .iter()
            .flat_map(|pair| &pair.requests)
            .any(|r| r.id == id && r.phase == Phase::Running);
        if !running {
            return Err("This ask was stopped before it was delivered.".into());
        }
        if let Some((_, recipient)) = lock(&self.exchange_leases).get_mut(id) {
            *recipient = self.capability_lease(to).scoped();
        }
        Ok(())
    }
}
