//! End thread: the person ending a work thread a teammate handed over.
//!
//! A request ends only when someone ends it, and this is one of those ends:
//! the person's. The thread closes as the person's Archive closes any thread;
//! each handoff into it is released, as the person's decision, and one whose
//! work had not come back is stopped with no result. The teammate whose item
//! the work was is not woken and gets no failure in its chat: its item closes
//! as dropped, which it is told at its next turn's start. One on a linked
//! desk is sent the end as the handoff's result, marked as the person's stop,
//! which its desk tells it the same way.

use super::*;

/// What a handoff the person ended says, as its reply.
const ENDED: &str = "The person ended this thread before it finished.";

impl Room {
    /// Ends the handoffs into a thread the person has just closed.
    pub(in crate::session) fn person_ended(&self, side_id: &str) {
        let person = self.person_name().unwrap_or_else(|| "The person".into());
        let text = format!("{person} ended this thread, and the work you handed over with it.");
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
                        request.phase(),
                        Phase::Queued | Phase::Running | Phase::WaitingHuman
                    ) {
                        // The thread is closed already, so this revokes the
                        // request's leases and stops nothing else.
                        let by_person = linked::is_linked(&request.from);
                        let reply = if by_person { text.as_str() } else { ENDED };
                        changed |= self.end_request(
                            request,
                            Ending::Closed { by_person },
                            reply,
                            &mut ended,
                        );
                    }
                }
                if changed && let Err(error) = self.save_pair(&pair) {
                    eprintln!("end the handoffs into thread {side_id}: {error}");
                }
                self.wake_owed(&pair);
            }
        }
        self.mark_handoffs(&ended);
        // A question an Interrupt left the thread carrying has no turn left
        // to answer it.
        self.end_carried(side_id, None);
    }
}
