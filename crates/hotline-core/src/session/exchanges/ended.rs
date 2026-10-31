//! End thread: the person ending a work thread a teammate handed over.
//!
//! A request ends only when someone ends it, and this is one of those ends:
//! the person's. The thread closes as the person's Archive closes any thread;
//! each handoff into it is released, as the person's decision, and one whose
//! work had not come back is stopped with no result. The teammate that handed
//! it over is not woken and gets no failure in its chat: it is left one `note`
//! aside, heard with its next turn where the result would have gone.

use super::*;
use crate::contract::{Aside, AsideLevel, AsideSource};

/// What a handoff the person ended says, as its reply.
const ENDED: &str = "The person ended this thread before it finished.";

impl Room {
    /// Ends the handoffs into a thread the person has just closed, and tells
    /// the teammate whose handoff stood there, once.
    pub(in crate::session) fn person_ended(&self, side_id: &str) {
        let standing = self.handoff_standing(side_id);
        let mut ended = Vec::new();
        {
            let _guard = lock(&self.exchange_lock);
            for mut pair in self.exchange_pairs() {
                let mut changed = false;
                for request in pair
                    .requests
                    .iter_mut()
                    .filter(|r| r.intent == Intent::Handoff && r.thread.as_deref() == Some(side_id))
                {
                    changed |= request.release(&mut ended);
                    if matches!(
                        request.phase,
                        Phase::Queued | Phase::Running | Phase::WaitingHuman
                    ) {
                        // The thread is closed already, so this revokes the
                        // request's leases and stops nothing else.
                        self.cancel_request(request);
                        request.phase = Phase::Stopped;
                        request.failed = true;
                        request.reply = ENDED.into();
                        changed = true;
                    }
                }
                if changed && let Err(error) = self.save_pair(&pair) {
                    eprintln!("end the handoffs into thread {side_id}: {error}");
                }
            }
        }
        self.mark_handoffs(&ended);
        // A question an Interrupt left the thread carrying has no turn left
        // to answer it.
        self.end_carried(side_id, None);
        let Some(request) = standing else {
            return;
        };
        let person = self.person_name().unwrap_or_else(|| "The person".into());
        let aside = Aside {
            id: new_id(),
            ts: now_ms(),
            to: request.from.clone(),
            from: AsideSource::Thread(self.work_thread(side_id)),
            by: Some(request.to.clone()),
            text: format!("{person} ended this thread, and the work you handed over with it."),
            about: self.route_about(&request),
            level: AsideLevel::Note,
            heard: None,
            seen: None,
            looped_in: false,
            source_name: None,
        };
        if let Err(error) = self.keep_aside(&aside) {
            eprintln!("tell the opener of thread {side_id} it ended: {error}");
        }
    }
}
