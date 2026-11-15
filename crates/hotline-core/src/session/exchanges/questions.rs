//! What is left of colleagues' questions now that the inbox carries them
//! (`docs/design/agent-inbox.md`): where a request's sender hears what came
//! of it, and a question from before the inbox, which nothing answers any
//! more and which fails to its sender if it is ever handed over.

use super::*;

/// Where the sender of a request hears what came of it.
pub(super) enum Route {
    Dm,
    Work(String),
    Job(String),
}

impl Room {
    /// Hands an accepted request to the conversation that works it: a
    /// handoff to its work thread. An ask, from before the inbox, fails to
    /// its sender.
    pub(super) async fn hand_over_request(
        self: &Arc<Self>,
        key: &str,
        request: &Request,
        thread: Option<String>,
    ) -> Result<(), String> {
        if request.intent == Intent::Handoff {
            return self.dispatch_handoff(key, request, thread).await;
        }
        self.set_exchange_reply(key, &request.id, ASKED_BEFORE_THE_INBOX.into(), true)
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
            .filter(|handoff| handoff.stands() && self.exchange_lease_current(&handoff.id))
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
        Route::Dm
    }

    /// Who is answering in a pair's thread now: the recipient of a request
    /// of it whose turn has begun and not ended.
    pub(in crate::session) fn pair_answering(&self, key: &str) -> Option<String> {
        self.exchange_pair(key)?
            .requests
            .into_iter()
            .find(Request::begun)
            .map(|r| r.to)
    }
}
