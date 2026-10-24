//! A colleague's question, delivered to the conversation that answers it.

use super::*;
use crate::contract::NoticeLevel;
use crate::session::runner::Driven;
use crate::session::turns::Line;

const QUESTION_ENDED: &str =
    "The turn answering this question ended without a result; inspect before retrying.";

impl Room {
    /// Hands an ask to its recipient's main conversation: a line that starts a
    /// turn there, or waits behind the one in flight, and whose turn's reply
    /// is the ask's answer. The pair's thread keeps the question.
    pub(super) async fn ask_in_main(
        self: &Arc<Self>,
        key: &str,
        request: &Request,
    ) -> Result<(), String> {
        self.ask_in(key, request, Route::Dm(request.to.clone()), None)
            .await
    }

    /// Hands a question the delegation was the grant for to the conversation
    /// that handed its thread over, by the handoff's reply route. One whose
    /// delegation has ended since fails rather than reach a conversation no
    /// grant was given for. The turn answering it is that conversation's own,
    /// so stopping the ask never cancels it: only its answer is no longer
    /// returned.
    pub(super) async fn ask_back(
        self: &Arc<Self>,
        key: &str,
        request: &Request,
    ) -> Result<(), String> {
        let handoff = self.about_live(request).ok_or(
            "The handoff this question was about ended before it could be asked. Ask again if you still need it.",
        )?;
        let work = peers::about(&handoff.message);
        self.ask_in(key, request, self.route_of(&handoff), Some(work))
            .await
    }

    /// A line that starts a turn in `route`'s conversation, or waits behind
    /// the one in flight, whose turn's reply is the ask's answer. `work` is
    /// the handoff the question is about, when it is about one.
    async fn ask_in(
        self: &Arc<Self>,
        key: &str,
        request: &Request,
        route: Route,
        work: Option<String>,
    ) -> Result<(), String> {
        let caller = self.participant(&request.from)?;
        if let Route::Dm(persona_id) = &route {
            // A conversation quiet past the room's idle window begins its
            // next chapter on a fresh session first, so the question is the
            // first line of that chapter. The swap leaves the ask's leases,
            // its teammates' lasting authority, standing.
            self.start(persona_id).await?;
            drop(self.in_this_chapter(persona_id).await?);
            self.still_asked(&request.id)?;
        }
        self.exchange_thread_line(key, &request.from, &request.id, &request.message)?;
        let id = format!("question:{}", request.id);
        let cause = DeliveryCause::Question {
            request_id: request.id.clone(),
            persona_id: caller.id.clone(),
            name: super::super::links::shown(&caller),
            thread_key: key.into(),
            about: peers::about(&request.message),
            work,
        };
        let text = request.message.clone();
        let answers = Some(RequestLine {
            request: request.id.clone(),
            answer: None,
        });
        match route {
            Route::Dm(persona_id) => {
                self.deliver_identified(&persona_id, &id, cause, text, answers)
                    .await
            }
            Route::Work(thread) => {
                let from = self.delivery_source(&request.to, &cause);
                self.deliver_into_work(&thread, &id, cause, from, text, answers)
                    .await
            }
            Route::Job(job_id) => {
                let from = self.delivery_source(&request.to, &cause);
                self.deliver_into_job(&job_id, &id, cause, from, text, answers)
            }
        }
    }

    /// The thread a colleague handed over that an ask was sent from: the
    /// ask's answer or failure is heard there, and its stop is shown there.
    pub(super) fn delegated_origin(&self, request: &Request) -> Option<String> {
        request
            .reply_thread
            .clone()
            .filter(|thread| request.intent == Intent::Ask && self.opener_of(thread).is_some())
    }

    /// Whether an ask stopped by a revocation leaves its thread owed its
    /// failure: it was sent from a thread a colleague handed over, and the
    /// asker's own authority, which that thread runs under, has not ended.
    /// A teammate the person stopped is not woken to hear it. Asked before
    /// the ask's leases are revoked.
    pub(super) fn owes_thread(&self, request: &Request, revoked: Option<&str>) -> bool {
        self.delegated_origin(request).is_some()
            && revoked != Some(request.from.as_str())
            && lock(&self.exchange_leases)
                .get(&request.id)
                .is_none_or(|(asker, _)| asker.is_current())
    }

    /// Tells the thread a colleague handed over that an ask it sent was
    /// stopped. A handoff waiting on the ask carries on with its failure, as
    /// it would with an answer. Otherwise the thread only shows the stop, in
    /// a line that starts no turn: a turn told of it would send the question
    /// again and undo the stop. A thread closed since, by this stop or
    /// another, is not opened again for it, and one that cannot take the
    /// handoff's delivery now is not waited on: its sender is told in their
    /// DM instead. Either way the debt is settled once.
    pub(super) async fn tell_thread_stopped(
        self: &Arc<Self>,
        key: &str,
        ask: &Request,
    ) -> Result<(), String> {
        let delivered = match self.delegated_origin(ask) {
            Some(thread) if self.work_is_open(&thread) => {
                if let Some(carried) = self.carried_on(ask) {
                    let (id, cause, from, text) = self.answer_delivery(key, ask)?;
                    let handed = self
                        .deliver_into_work(&thread, &id, cause, from, text, Some(carried))
                        .await;
                    if let Err(error) = &handed {
                        eprintln!("a stopped question's thread could not take it: {error}");
                    }
                    handed.is_ok()
                } else {
                    self.threads().write(
                        &self.work_thread(&thread),
                        &ask.from,
                        &TranscriptEvent::Notice {
                            id: format!("exchange-stopped:{}", ask.id),
                            ts: now_ms(),
                            level: NoticeLevel::Info,
                            text: self.stopped_line(&ask.to, &[ask]),
                        },
                    );
                    true
                }
            }
            _ => false,
        };
        {
            let _guard = lock(&self.exchange_lock);
            let mut pair = self.exchange_pair(key).ok_or("Exchange disappeared")?;
            if let Some(told) = pair.requests.iter_mut().find(|r| r.id == ask.id) {
                told.owed_to_thread = false;
            }
            self.save_pair(&pair)?;
        }
        if !delivered {
            self.exchange_notices(&[Request {
                owed_to_thread: false,
                ..ask.clone()
            }]);
        }
        Ok(())
    }

    /// Fails each question among the lines a thread or a job dropped, as the
    /// person's cancel fails one. On a task of its own, since a stop ends a
    /// thread under the exchange lock this takes.
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

    /// For a question sent from a thread a colleague handed over, to that
    /// colleague: the latest handoff the thread serves, which it is about.
    pub(super) fn asked_back_about(&self, sent_from: &ThreadId, to: &str) -> Option<String> {
        if sent_from.kind != ThreadKind::Side {
            return None;
        }
        self.opener_of(&sent_from.key)
            .filter(|opener| opener.persona_id == to)?;
        self.serving(&sent_from.key).map(|(_, handoff)| handoff.id)
    }

    /// The handoff an ask was asked back about, while its delegation stands,
    /// as this desk's own record has it: one the ask's recipient handed its
    /// sender. `about` is only the key it is looked up by.
    pub(super) fn about_live(&self, request: &Request) -> Option<Request> {
        let about = request.about.as_deref()?;
        self.exchange_pairs()
            .into_iter()
            .flat_map(|pair| pair.requests)
            .find(|r| {
                r.id == about
                    && r.from == request.to
                    && r.to == request.from
                    && r.intent == Intent::Handoff
            })
            .filter(|handoff| self.delegation_live(handoff))
    }

    /// Saves what the turn answering a question came to. One the person's
    /// `cut` cut short ends as they decided; one that failed or was stopped
    /// otherwise says so, for its asker to look before trying again.
    pub(in crate::session) fn answer_question(
        self: &Arc<Self>,
        request: &str,
        driven: &Driven,
        cut: Option<Cut>,
    ) {
        if let Some(cut) = cut
            && self.end_cut_by_person(request, driven, cut).is_some()
        {
            return;
        }
        let failed = driven.stop_reason.as_deref().is_none_or(|reason| {
            matches!(
                reason,
                "failed" | "cancelled" | "canceled" | "aborted" | "revoked"
            )
        });
        let mut reply = driven.replies.join("\n\n");
        if failed {
            reply = if reply.is_empty() {
                QUESTION_ENDED.to_string()
            } else {
                format!("{reply}\n\n{QUESTION_ENDED}")
            };
        }
        self.finish_request(request, reply, failed);
    }

    /// What a failed turn that said nothing returns for the request it
    /// answered, worded for the question or the handoff it was.
    pub(in crate::session) fn ended_without_result(&self, request: &str) -> &'static str {
        if self.is_ask(request) {
            QUESTION_ENDED
        } else {
            "The handoff turn ended without a result; inspect before retrying."
        }
    }

    pub(super) fn is_ask(&self, id: &str) -> bool {
        self.exchange_pairs()
            .iter()
            .flat_map(|pair| &pair.requests)
            .any(|r| r.id == id && r.intent == Intent::Ask)
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

    /// Refuses an ask stopped while its recipient's conversation was being
    /// brought up, so it is never delivered there.
    fn still_asked(&self, id: &str) -> Result<(), String> {
        let running = self
            .exchange_pairs()
            .iter()
            .flat_map(|pair| &pair.requests)
            .any(|r| r.id == id && r.phase == Phase::Running);
        if running {
            Ok(())
        } else {
            Err("This ask was stopped before it was delivered.".into())
        }
    }
}
