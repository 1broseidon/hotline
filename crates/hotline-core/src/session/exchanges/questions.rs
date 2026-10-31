//! A colleague's question, delivered to the conversation that answers it
//! (`docs/design/thread-awareness.md`, decision 1).
//!
//! There is no session of the pair's own: a question goes where the memory
//! is. Its recipient's main conversation answers it when it is idle, as a turn
//! of its own that takes in nothing the person says. While that conversation
//! is busy the question never waits behind it: a work thread of the
//! recipient's answers at once, opened for it alone and seeded as any work
//! thread is (the main conversation's last note and lines, and its thread
//! index on every turn) with the asides the teammate was left lately, and
//! when it has answered it leaves the main conversation a note aside saying
//! what it answered on its behalf. A question that names a thread is answered
//! in that thread, which stays open; one a handed-over thread asks the
//! teammate that handed it over reaches the conversation that did, by the
//! handoff's own reply route, and needs no card while the delegation stands.

use super::*;
use crate::contract::{Aside, AsideLevel, AsideSource};
use crate::session::sides::title_of;

/// The most of a busy conversation's answer the aside back carries.
const ANSWERED_CHARS: usize = 300;

/// Where the sender of a request hears what came of it, or where a question
/// back to the teammate that handed work over is answered.
pub(super) enum Route {
    Dm(String),
    Work(String),
    Job(String),
}

impl Room {
    /// Hands an accepted request to the conversation that works it: a handoff
    /// to its work thread; an ask to the thread it names, else, when a
    /// delegated thread asks back, to the conversation that handed it over,
    /// else to its recipient's main conversation. An ask that cannot be
    /// handed over fails to its sender.
    pub(super) async fn hand_over_request(
        self: &Arc<Self>,
        key: &str,
        request: &Request,
        thread: Option<String>,
    ) -> Result<(), String> {
        if request.intent == Intent::Handoff {
            return self.dispatch_handoff(key, request, thread).await;
        }
        let handed_over = match thread {
            Some(thread) => self.ask_in_thread(key, request, &thread).await,
            None if request.about.is_some() => self.ask_back(key, request).await,
            None => self.ask_in_main(key, request, &request.to, None).await,
        };
        match handed_over {
            Ok(()) => Ok(()),
            Err(error) => self.set_exchange_reply(key, &request.id, error, true),
        }
    }

    /// Hands a question the delegation is the grant for to the conversation
    /// that handed its thread over, by the handoff's reply route. One whose
    /// delegation has ended since fails rather than reach a conversation no
    /// grant was given for.
    async fn ask_back(self: &Arc<Self>, key: &str, request: &Request) -> Result<(), String> {
        let handoff = self.about_live(request).ok_or(
            "The handoff this question was about ended before it could be asked. Ask again if you still need it.",
        )?;
        let work = Some(peers::about(&handoff.message));
        match self.route_of(&handoff) {
            Route::Dm(persona_id) => self.ask_in_main(key, request, &persona_id, work).await,
            Route::Work(thread) => self.ask_in_thread(key, request, &thread).await,
            Route::Job(job_id) => {
                self.exchange_thread_line(key, &request.from, &request.id, &request.message)?;
                let cause = self.question(key, request, work)?;
                let from = self.delivery_source(&request.to, &cause);
                self.deliver_into_job(
                    &job_id,
                    &question_id(request),
                    cause,
                    from,
                    request.message.clone(),
                    Some(answering(request)),
                )
            }
        }
    }

    /// Hands a question to `persona_id`'s main conversation when no turn holds
    /// it, as a turn of its own whose reply is the answer; while it is busy,
    /// to a work thread of the teammate's opened for it alone. The line is
    /// written to the tape, and the pair's thread keeps the question.
    async fn ask_in_main(
        self: &Arc<Self>,
        key: &str,
        request: &Request,
        persona_id: &str,
        work: Option<String>,
    ) -> Result<(), String> {
        let cause = self.question(key, request, work)?;
        // Starting a teammate can take a while; a question stopped meanwhile
        // is not handed over.
        self.start(persona_id).await?;
        self.still_running(key, &request.id)?;
        let answered_here = self
            .deliver_to_main(
                persona_id,
                &question_id(request),
                cause.clone(),
                request.message.clone(),
                Some(answering(request)),
                true,
            )
            .await?;
        if answered_here {
            return self.exchange_thread_line(key, &request.from, &request.id, &request.message);
        }
        self.ask_in_own_thread(key, request, persona_id, cause)
            .await
    }

    /// Opens a work thread of `persona_id`'s for a question its busy main
    /// conversation cannot take now, with the question as its first line,
    /// and a link on the pair's thread. A teammate whose every thread is
    /// mid-turn has no place for one, and the question waits behind its main
    /// conversation's turn instead.
    async fn ask_in_own_thread(
        self: &Arc<Self>,
        key: &str,
        request: &Request,
        persona_id: &str,
        cause: DeliveryCause,
    ) -> Result<(), String> {
        if !self.sides.has_room(persona_id) {
            self.deliver_to_main(
                persona_id,
                &question_id(request),
                cause,
                request.message.clone(),
                Some(answering(request)),
                false,
            )
            .await?;
            return self.exchange_thread_line(key, &request.from, &request.id, &request.message);
        }
        let side_id = new_id();
        {
            let _guard = lock(&self.exchange_lock);
            let mut pair = self.exchange_pair(key).ok_or("Exchange disappeared")?;
            let r = pair
                .requests
                .iter_mut()
                .find(|r| r.id == request.id && r.phase == Phase::Running)
                .ok_or("This question was stopped before it was asked.")?;
            r.thread = Some(side_id.clone());
            r.own_thread = true;
            self.save_pair(&pair)?;
        }
        self.exchange_thread_line(key, &request.from, &request.id, &request.message)?;
        let (_, launch) = self.bring_up(
            persona_id,
            Start {
                thread: ThreadId::side(&side_id),
                title: title_of(&request.message),
                started: now_ms(),
                opener: None,
                fresh: true,
                saved: None,
                handoff: Some(HandoffStart {
                    key: key.to_string(),
                    request: request.id.clone(),
                }),
                asked: Some(key.to_string()),
            },
        )?;
        let from = self.delivery_source(persona_id, &cause);
        let delivered = self
            .deliver_into_work(
                &side_id,
                &question_id(request),
                cause,
                from,
                request.message.clone(),
                Some(answering(request)),
            )
            .await;
        // The line is queued first, so the agent's start finds it waiting.
        self.launch(launch);
        delivered
    }

    /// Hands a question to a work thread: the one it names, the one its
    /// delegator handed over from, or its own, brought back after a restart.
    /// A line that waits behind the turn in flight, whose turn's reply is the
    /// answer, by the teammate doing the work there. A parked or closed thread
    /// comes back for it, and one that cannot start fails the question. Only
    /// a thread opened for the question closes with its answer.
    async fn ask_in_thread(
        self: &Arc<Self>,
        key: &str,
        request: &Request,
        thread: &str,
    ) -> Result<(), String> {
        let work = self
            .serving(thread)
            .map(|handoff| peers::about(&handoff.message));
        let cause = self.question(key, request, work)?;
        self.exchange_thread_line(key, &request.from, &request.id, &request.message)?;
        self.set_request_thread(key, &request.id, thread)?;
        let (_, launch) = self
            .wake_side(
                thread,
                true,
                Some(HandoffStart {
                    key: key.into(),
                    request: request.id.clone(),
                }),
            )
            .await?;
        let from = self.delivery_source(&request.to, &cause);
        let delivered = self
            .deliver_into_work(
                thread,
                &question_id(request),
                cause,
                from,
                request.message.clone(),
                Some(answering(request)),
            )
            .await;
        if let Some(launch) = launch {
            self.launch(launch);
        }
        delivered
    }

    /// Refuses a request no longer running: one stopped while it was being
    /// handed over is not.
    fn still_running(&self, key: &str, id: &str) -> Result<(), String> {
        self.exchange_pair(key)
            .is_some_and(|pair| {
                pair.requests
                    .iter()
                    .any(|r| r.id == id && r.phase == Phase::Running)
            })
            .then_some(())
            .ok_or_else(|| "This question was stopped before it was asked.".to_string())
    }

    /// A question as the conversation answering it is handed it.
    fn question(
        &self,
        key: &str,
        request: &Request,
        work: Option<String>,
    ) -> Result<DeliveryCause, String> {
        let caller = self.participant(&request.from)?;
        Ok(DeliveryCause::Question {
            request_id: request.id.clone(),
            persona_id: caller.id.clone(),
            name: crate::session::links::shown(&caller),
            thread_key: key.into(),
            about: peers::about(&request.message),
            work,
        })
    }

    /// The latest handoff a work thread serves.
    fn serving(&self, thread: &str) -> Option<Request> {
        self.exchange_pairs()
            .into_iter()
            .flat_map(|pair| pair.requests)
            .rfind(|r| r.intent == Intent::Handoff && r.thread.as_deref() == Some(thread))
    }

    /// For a question sent from a thread a colleague handed over, to that
    /// colleague: the latest handoff the thread serves, which it is about.
    pub(super) fn asked_back_about(&self, sent_from: &ThreadId, to: &str) -> Option<String> {
        if sent_from.kind != ThreadKind::Side {
            return None;
        }
        self.serving(&sent_from.key)
            .filter(|handoff| handoff.from == to)
            .map(|handoff| handoff.id)
    }

    /// The handoff an ask was asked back about, while its delegation stands,
    /// as this desk's own record has it: one the ask's recipient handed its
    /// sender, not released or stopped, its leases current. `about` is only
    /// the key it is looked up by.
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
            .filter(|handoff| {
                !handoff.released
                    && handoff.phase != Phase::Stopped
                    && self.exchange_lease_current(&handoff.id)
            })
    }

    /// Where the sender hears what came of a request: the job it was sent
    /// from while that job lives, else the work thread it was sent from while
    /// that is open, else its own conversation.
    pub(super) fn route_of(&self, request: &Request) -> Route {
        if let Some(job_id) = request
            .reply_job
            .as_deref()
            .filter(|job_id| self.job_lives(job_id))
        {
            return Route::Job(job_id.to_string());
        }
        if let Some(thread) = request
            .reply_thread
            .as_deref()
            .filter(|thread| self.work_is_open(thread))
        {
            return Route::Work(thread.to_string());
        }
        Route::Dm(request.from.clone())
    }

    /// Whether `request` is a question its recipient's main conversation was
    /// handed, its line on that teammate's tape, whose turn has not begun and
    /// which the session up there was never handed: a chapter's swap drops
    /// the questions waiting behind the turn it cuts, and every other end of
    /// that session ends or fails them. A question handed to a work thread or
    /// a job is that conversation's to answer. A session not up yet is waited
    /// for, so the swap's own start comes first.
    pub(super) fn dropped_by_a_swap(&self, request: &Request) -> bool {
        if request.intent != Intent::Ask
            || request.started
            || request.thread.is_some()
            || linked::is_linked(&request.to)
        {
            return false;
        }
        let line = question_id(request);
        self.session(&request.to)
            .is_ok_and(|session| !lock(&session.dispatched_deliveries).contains(&line))
            && self
                .log
                .contains(&StreamId::Tape(request.to.clone()), &line)
    }

    /// Who is answering in a pair's thread now: the recipient of a request
    /// of it whose turn has begun and not ended.
    pub(in crate::session) fn pair_answering(&self, key: &str) -> Option<String> {
        self.exchange_pair(key)?
            .requests
            .into_iter()
            .find(|r| r.phase == Phase::Running && r.started)
            .map(|r| r.to)
    }

    /// The note a work thread leaves its teammate's main conversation once it
    /// has answered a question there on its behalf: who asked, and what it
    /// said, in its own words and short. Heard at that conversation's next
    /// turn; never said to the person.
    pub(super) fn answered_for_main(&self, request: &Request) {
        let Some(thread) = &request.thread else {
            return;
        };
        let asker = self.participant(&request.from).map_or_else(
            |_| "a colleague".to_string(),
            |p| crate::session::links::shown(&p),
        );
        let aside = Aside {
            id: format!("answered:{}", request.id),
            ts: now_ms(),
            to: request.to.clone(),
            from: AsideSource::Thread(ThreadId::side(thread)),
            by: Some(request.to.clone()),
            text: format!(
                "Answered {asker} for you: {}",
                crate::driver::clip(request.reply.trim(), ANSWERED_CHARS)
            ),
            about: None,
            level: AsideLevel::Note,
            heard: None,
        };
        if self.aside_kept(&request.to, &aside.id) {
            return;
        }
        if let Err(error) = self.keep_aside(&aside) {
            eprintln!("the note of an answer in thread {thread}: {error}");
        }
    }
}

/// The id a question's line is written under, wherever it is answered.
fn question_id(request: &Request) -> String {
    format!("question:{}", request.id)
}

/// The request a question's turn answers.
fn answering(request: &Request) -> RequestLine {
    RequestLine {
        request: request.id.clone(),
        answer: None,
    }
}
