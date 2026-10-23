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
