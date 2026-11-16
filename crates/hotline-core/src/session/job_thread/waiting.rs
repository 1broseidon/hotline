//! A job's last run that waits on a colleague (`docs/design/agent-inbox.md`,
//! When an item reaches the receiver).
//!
//! A one-time job, or a loop's last run, that sent a colleague an item that
//! wants an answer or work is not over when its turn ends while that item is
//! still waiting on its reply. It waits: its agent is let go of, so it runs
//! no turn and counts as none, its thread says once whom it waits on, and its
//! record names them (`awaiting`), which the desk says as "Waiting on Huck".
//! A reply wakes it in its thread as any reply to a run does
//! (`inbox/heard.rs`), and the turn that hears the last one it waited on
//! ends the job. It never waits for ever:
//!
//! - an item still open expires three days after it arrived, as any does,
//!   and Hotline's notice of it is heard in the job as a reply is;
//! - an item the colleague closed with no reply (noted, dropped, combined
//!   into another) is heard where it stands at the inbox clock's next look,
//!   a turn of the job, which then ends;
//! - an item taken and never answered, which never expires, is waited on
//!   until three days after it arrived, as it would have expired, and the
//!   job then ends saying no reply came.
//!
//! The person's End job, and Stop on it, end it at once. A restart keeps it
//! waiting: its record says it is in its last run, the items are on disk, and
//! a reply that came meanwhile wakes it as the desk opens.

use super::LiveJob;
use crate::contract::{
    End, InboxItem, InboxState, InboxWants, NoticeLevel, ScheduledJob, ThreadId, TranscriptEvent,
};
use crate::log::StreamId;
use crate::room;
use crate::session::inbox::{arrived_at, expire_after};
use crate::session::{Room, lock, new_id, now_ms};
use crate::store::inbox as kept;
use crate::thread::ThreadState;
use std::sync::Arc;

/// What the thread says when a job's run stops waiting because a taken item
/// was never answered.
const NO_REPLY: &str = "No reply came in three days, so the job has ended.";

/// What the thread says when the person's Stop ends a run that was waiting.
pub(super) const STOPPED_WAITING: &str = "Stopped. That was the job's last run, so it has ended.";

impl Room {
    /// When the run of a job going now began: its latest firing, if it has
    /// fired.
    pub(super) fn run_began(&self, job_id: &str) -> Option<i64> {
        self.log
            .load(&StreamId::Job(job_id.to_string()))
            .iter()
            .rev()
            .find(|event| {
                event["kind"] == "user" && event.get("scheduled").is_some_and(|run| !run.is_null())
            })
            .and_then(|firing| firing["ts"].as_i64())
    }

    /// What the run of `job` going now sent its colleagues that wants an
    /// answer or work back: from its thread, since its latest firing began.
    fn run_asked(&self, job: &ScheduledJob) -> Vec<InboxItem> {
        let thread = ThreadId::job(&job.id);
        let began = self.run_began(&job.id).unwrap_or(i64::MIN);
        let roster = room::roster(&self.log);
        kept::sent_by(
            &self.log,
            &job.persona_id,
            roster.iter().map(|persona| persona.id.as_str()),
        )
        .into_iter()
        .filter(|item| {
            item.from_thread == thread && item.ts >= began && item.wants != InboxWants::Nothing
        })
        .collect()
    }

    /// What the run of `job` going now still waits on at `now`: what it
    /// asked that has not closed, but for an item taken and not answered
    /// three days after it arrived, which a taken item's never expiring
    /// would otherwise leave waited on for ever.
    pub(in crate::session) fn run_awaits(&self, job: &ScheduledJob, now: i64) -> Vec<InboxItem> {
        self.run_asked(job)
            .into_iter()
            .filter(|item| {
                item.state != InboxState::Closed
                    && (item.state != InboxState::Taken || arrived_at(item) + expire_after() > now)
            })
            .collect()
    }

    /// The turn of `live`'s last run has ended with `awaits` still waiting
    /// on their replies: the job waits, its record names whom on, its thread
    /// says so once, and its agent is let go of.
    pub(super) fn wait_for_replies(&self, live: &LiveJob, awaits: &[InboxItem]) {
        let whom = colleagues(awaits);
        if self.set_job_awaiting(&live.job_id, &whom) {
            live.notice(self, NoticeLevel::Info, self.waiting_words(&whom));
        }
        live.release(self);
    }

    /// "Waiting for Huck to reply. The job ends once the reply has come
    /// back."
    fn waiting_words(&self, whom: &[String]) -> String {
        let names: Vec<String> = whom
            .iter()
            .map(|id| self.persona(id).map_or_else(|_| id.clone(), |p| p.name))
            .collect();
        match names.as_slice() {
            [one] => {
                format!("Waiting for {one} to reply. The job ends once the reply has come back.")
            }
            [rest @ .., last] => format!(
                "Waiting for {} and {last} to reply. The job ends once their replies have come back.",
                rest.join(", ")
            ),
            [] => String::new(),
        }
    }

    /// Writes whom a job's last run waits on onto its record when that
    /// changed, and says whether the wait began with this.
    fn set_job_awaiting(&self, job_id: &str, whom: &[String]) -> bool {
        let _mutations = lock(&self.schedule_mutations);
        let Some(mut job) = room::schedules(&self.log)
            .into_iter()
            .find(|job| job.id == job_id)
        else {
            return false;
        };
        if job.awaiting.as_deref() == Some(whom) {
            return false;
        }
        let began = job.awaiting.is_none();
        job.awaiting = Some(whom.to_vec());
        if let Err(error) = room::append_schedule(&self.log, &job) {
            eprintln!("a job's wait could not be written: {error}");
            return false;
        }
        began
    }

    /// Whether `job`'s thread has something of its inbox it has not heard:
    /// an item that came back to its run, or where something it sent stands
    /// now.
    fn job_has_news(&self, job: &ScheduledJob) -> bool {
        let thread = ThreadId::job(&job.id);
        !self.unoffered(&job.persona_id, &thread).is_empty()
            || self.sent_news(&job.persona_id, &thread)
    }

    /// A turn of a job's thread is over and nothing waits behind it. A last
    /// firing that never became a turn (skipped, or its agent did not start)
    /// is over too, unless its run still waits on a reply; a run that waited
    /// and has nothing more coming ends, unless it has something still to
    /// hear, which wakes it (`inbox_rewake`, or the clock's next look).
    pub(super) fn last_run_rested(self: &Arc<Self>, job_id: &str) {
        let Some(job) = room::schedules(&self.log)
            .into_iter()
            .find(|job| job.id == job_id && job.final_firing.is_some())
        else {
            return;
        };
        let awaits = self.run_awaits(&job, now_ms());
        if !awaits.is_empty() {
            self.set_job_awaiting(&job.id, &colleagues(&awaits));
        } else if job.awaiting.is_none() {
            self.end_final_firing(&job.id);
        } else if !self.job_has_news(&job) {
            self.end_job_wait(&job, None);
        }
    }

    /// The inbox clock's look at `now` at each job whose last run waits and
    /// has no turn: one whose wait is over is woken to hear what came of it,
    /// or, with nothing to hear, ends.
    pub(in crate::session) fn look_at_waiting_jobs(self: &Arc<Self>, now: i64) {
        for job in room::schedules(&self.log) {
            if job.final_firing.is_none() || job.awaiting.is_none() || !self.job_at_rest(&job.id) {
                continue;
            }
            let awaits = self.run_awaits(&job, now);
            if !awaits.is_empty() {
                self.set_job_awaiting(&job.id, &colleagues(&awaits));
            } else if self.job_has_news(&job) {
                self.wake_job_for_inbox(&job.id);
            } else {
                let unanswered = self
                    .run_asked(&job)
                    .iter()
                    .any(|item| item.state != InboxState::Closed);
                self.end_job_wait(&job, unanswered.then_some(NO_REPLY));
            }
        }
    }

    /// As the desk opens, a job its last process left in its last run keeps
    /// waiting while the run waits on a reply, or waited and may still have
    /// one to hear (the clock's first look settles it); any other is over.
    pub(super) fn settle_last_run(&self, job: &ScheduledJob) {
        let awaits = self.run_awaits(job, now_ms());
        if !awaits.is_empty() {
            self.set_job_awaiting(&job.id, &colleagues(&awaits));
        } else if job.awaiting.is_none() {
            self.end_final_firing(&job.id);
        }
    }

    /// The person's Stop on a job whose last run waits: it ends now.
    pub(super) fn stop_waiting_job(self: &Arc<Self>, job_id: &str) -> bool {
        let Some(job) = room::schedules(&self.log)
            .into_iter()
            .find(|job| job.id == job_id && job.final_firing.is_some() && job.awaiting.is_some())
        else {
            return false;
        };
        if !self.job_at_rest(job_id) {
            return false;
        }
        self.end_job_wait(&job, Some(STOPPED_WAITING));
        true
    }

    /// Ends a job whose last run was waiting: its thread says `said`, its
    /// record ends, its thread closes, and what came back to it unheard is
    /// the main conversation's.
    fn end_job_wait(self: &Arc<Self>, job: &ScheduledJob, said: Option<&str>) {
        let thread = ThreadId::job(&job.id);
        if let Some(said) = said {
            self.threads().write(
                &thread,
                &job.persona_id,
                &TranscriptEvent::Notice {
                    id: new_id(),
                    ts: now_ms(),
                    level: NoticeLevel::Info,
                    text: said.to_string(),
                },
            );
        }
        self.end_final_firing(&job.id);
        let live = lock(&self.job_threads).get(&job.id).cloned();
        if let Some(live) = live {
            self.close_job(&live, End::Done, None);
            return;
        }
        // A desk that opened while it waited holds no thread for it yet.
        let binding = self.link_of(&thread).and_then(|link| link.binding);
        self.mark_job_thread(&job.id, ThreadState::Closed(End::Done), None, binding);
        self.inbox_left_job(&job.persona_id);
    }
}

/// The colleagues `items` went to, each once, in the order they were sent.
fn colleagues(items: &[InboxItem]) -> Vec<String> {
    let mut whom: Vec<String> = Vec::new();
    for item in items {
        if !whom.contains(&item.to) {
            whom.push(item.to.clone());
        }
    }
    whom
}
