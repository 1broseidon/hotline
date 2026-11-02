//! What comes of a question reaches the thread that asked it: its answer, or
//! its failure. A handoff waits for the answers to the questions its turn
//! asks colleagues, as it waits on a card: the question is a gate on the
//! handoff, and what comes of it, answer or failure, is the handoff's next
//! turn in its thread. A question stopped before its answer (Stop exchange,
//! a revocation, its teammate deleted or unlinked) hands its failure to the
//! work thread or job that asked it; one the person's Stop drops from the
//! queue it waited in fails to its asker at once.

use super::*;
use crate::session::turns::Line;

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
    /// the exchange stopped. The teammate it went to may have gone with it,
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
            answer: (!stopped && !request.failed && !request.answer.is_empty())
                .then(|| request.gist()),
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

    /// A question a handoff's gate names, once it has been answered or
    /// stopped, with its pair's key.
    pub(super) fn settled_ask(&self, id: &str) -> Option<(String, Request)> {
        self.exchange_pairs().into_iter().find_map(|pair| {
            pair.requests
                .into_iter()
                .find(|r| r.id == id && matches!(r.phase, Phase::Done | Phase::Stopped))
                .map(|r| (pair.id, r))
        })
    }

    /// The handoff a question's answer carries on: the one whose turn asked
    /// it, while that still runs or waits on it.
    pub(super) fn carried_on(&self, ask: &Request) -> Option<RequestLine> {
        self.exchange_pairs()
            .into_iter()
            .flat_map(|pair| pair.requests)
            .find(|r| {
                r.to == ask.from
                    && matches!(r.phase, Phase::Running | Phase::WaitingHuman)
                    && r.human_actions.iter().any(|gate| gate.id == ask.id)
            })
            .map(|handoff| RequestLine {
                request: handoff.id,
                answer: Some(ask.id.clone()),
            })
    }

    /// Whether a stopped ask leaves the thread that asked it owed its
    /// failure: it was sent from a work thread or a job, its answer had not
    /// been handed there yet, and the asker is not what was revoked, so there
    /// is someone there to hear it. Asked before the ask's leases are revoked.
    pub(super) fn owes_thread(&self, request: &Request, revoked: Option<&str>) -> bool {
        request.intent == Intent::Ask
            && request.phase != Phase::Done
            && (request.reply_thread.is_some() || request.reply_job.is_some())
            && !linked::is_linked(&request.from)
            && revoked != Some(request.from.as_str())
            && lock(&self.exchange_leases)
                .get(&request.id)
                .is_none_or(|(asker, _)| asker.is_current())
    }

    /// Hands a stopped ask's failure to the work thread or job that asked
    /// it: a handoff waiting on the ask carries on with it, as it would with
    /// an answer, else the thread hears it as any delivery. A thread or job
    /// that has ended since is not opened again for it: its sender is told in
    /// their DM instead. Either way the debt is settled once.
    pub(super) async fn tell_thread_stopped(
        self: &Arc<Self>,
        key: &str,
        ask: &Request,
    ) -> Result<(), String> {
        let (id, cause, from, text) = self.answer_delivery(key, ask)?;
        // A handoff waiting on the ask may have taken its failure up already
        // (`hand_over_asked`), and closed its thread since.
        let taken_up = ask.reply_thread.as_deref().is_some_and(|thread| {
            StreamId::of_thread(&self.work_thread(thread)).is_some_and(|stream| {
                self.log
                    .load(&stream)
                    .iter()
                    .any(|v| v["id"] == id.as_str())
            })
        });
        let handed = match self.route_of(ask) {
            _ if taken_up => Ok(()),
            Route::Work(thread) => {
                let carried = self.carried_on(ask);
                self.deliver_into_work(&thread, &id, cause, from, text, carried)
                    .await
            }
            Route::Job(job_id) => self.deliver_into_job(&job_id, &id, cause, from, text, None),
            Route::Dm(_) => Err("The thread that asked has ended.".into()),
        };
        {
            let _guard = lock(&self.exchange_lock);
            let mut pair = self.exchange_pair(key).ok_or("Exchange disappeared")?;
            if let Some(told) = pair.requests.iter_mut().find(|r| r.id == ask.id) {
                told.owed_to_thread = false;
            }
            self.save_pair(&pair)?;
        }
        if handed.is_err() {
            self.exchange_notices(&[Request {
                owed_to_thread: false,
                ..ask.clone()
            }]);
        }
        Ok(())
    }

    /// Fails at once each question the person's Stop dropped from `to`'s
    /// queue before its turn began, once: it is not handed over again, and
    /// a question no turn took up has nothing to inspect.
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
            .map_or_else(|_| to.to_string(), |p| crate::session::links::shown(&p));
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

    /// Fails each question among the lines a work thread or a job dropped,
    /// as the person's cancel fails one. On a task of its own, since a stop
    /// ends a thread under the exchange lock this takes.
    pub(in crate::session) fn fail_dropped_lines(
        &self,
        persona_id: &str,
        dropped: impl IntoIterator<Item = Line>,
    ) {
        let asks: Vec<String> = dropped
            .into_iter()
            .filter_map(|line| line.request)
            .map(|line| line.request)
            .collect();
        if asks.is_empty() {
            return;
        }
        let Some(room) = lock(&self.me).upgrade() else {
            return;
        };
        let persona_id = persona_id.to_string();
        tokio::spawn(async move {
            room.fail_dropped_questions(&persona_id, asks.iter().map(String::as_str));
        });
    }
}
