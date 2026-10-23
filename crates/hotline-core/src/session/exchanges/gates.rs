//! A handoff waits for the answers to the questions its turn asks colleagues,
//! as it waits on a card: the question is a gate on the handoff, and its
//! answer is the handoff's next turn, in its thread.

use super::*;

impl Room {
    /// Makes a question asked from inside a handoff's turn a gate the
    /// handoff's result waits behind. Called under the exchange lock once the
    /// question is saved: a crash between the two leaves the handoff waiting
    /// on nothing, never on a question that does not exist.
    pub(super) fn gate_on_ask(&self, sent_from: &ThreadId, ask_id: &str) {
        if sent_from.kind != ThreadKind::Side {
            return;
        }
        let Some(handoff) = self
            .sides
            .get(&sent_from.key)
            .and_then(|side| side.request())
        else {
            return;
        };
        for mut pair in self.exchange_pairs() {
            let Some(request) = pair.requests.iter_mut().find(|r| {
                r.id == handoff && r.intent == Intent::Handoff && r.phase == Phase::Running
            }) else {
                continue;
            };
            request.human_actions.push(HumanGate {
                id: ask_id.into(),
                consumed: false,
            });
            if let Err(error) = self.save_pair(&pair) {
                eprintln!("gate a handoff on its question: {error}");
            }
            return;
        }
    }

    /// A request's answer as its sender hears it: the delivery's id, its
    /// cause, where it came from and what it says. A stopped request's
    /// failure is not its result: it has an id of its own and names no
    /// request in its `from`, so a turn's door does not refuse it as a result
    /// the exchange stopped, and an unheard result of the same request does
    /// not stand in for it. The teammate it went to may have gone with it,
    /// deleted or on a desk no longer linked, and is then "That teammate".
    pub(super) fn answer_delivery(
        &self,
        key: &str,
        request: &Request,
    ) -> Result<(String, DeliveryCause, DeliveryFrom, String), String> {
        let stopped = request.phase == Phase::Stopped;
        let (persona_id, name) = match self.participant(&request.to) {
            Ok(target) => (target.id, target.name),
            Err(_) if stopped => (request.to.clone(), "That teammate".to_string()),
            Err(error) => return Err(error),
        };
        let cause = DeliveryCause::Peer {
            request_id: Some(request.id.clone()),
            persona_id,
            name,
            thread_key: key.into(),
            status: if request.failed {
                PeerStatus::Failed
            } else {
                PeerStatus::Done
            },
            about: peers::about(&request.message),
        };
        if stopped {
            return Ok((
                format!("exchange-stopped:{}", request.id),
                cause,
                DeliveryFrom::new(&ThreadId::pair(key), None),
                request.reply.clone(),
            ));
        }
        let from = self.delivery_source(&request.from, &cause);
        Ok((
            format!("exchange-result:{}", request.id),
            cause,
            from,
            request.reply.clone(),
        ))
    }

    /// Hands what came of a question, its answer or its failure, to the
    /// handoff waiting on it, in the handoff's thread.
    pub(super) async fn hand_over_asked(
        self: &Arc<Self>,
        handoff: &Request,
        key: &str,
        ask: &Request,
    ) -> Result<(), String> {
        let Some(thread) = &handoff.thread else {
            return Ok(());
        };
        let (id, cause, from, text) = self.answer_delivery(key, ask)?;
        let carried = RequestLine {
            request: handoff.id.clone(),
            answer: Some(ask.id.clone()),
        };
        self.deliver_into_work(thread, &id, cause, from, text, Some(carried))
            .await
    }

    /// The handoff a question's answer carries on: the one whose turn asked
    /// it, while that still runs or waits on it.
    pub(super) fn carried_on(&self, ask: &Request) -> Option<RequestLine> {
        self.exchange_pairs()
            .into_iter()
            .flat_map(|pair| pair.requests)
            .find(|r| {
                r.to == ask.from
                    && matches!(r.phase, Phase::Running | Phase::Waiting)
                    && r.human_actions.iter().any(|gate| gate.id == ask.id)
            })
            .map(|handoff| RequestLine {
                request: handoff.id,
                answer: Some(ask.id.clone()),
            })
    }
}
