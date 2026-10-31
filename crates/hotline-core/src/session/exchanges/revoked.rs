//! What a revocation ends of a teammate's exchanges.

use super::*;

impl Room {
    pub(in crate::session) fn revoke_exchanges(&self, persona: &str) {
        self.stop_revoked_exchanges(Some(persona));
    }
    /// Ends what `persona` holds or was given, and anything whose leases have
    /// ended, which only a stop or a policy change on either teammate does
    /// (see [`Room::request_lease`]): the sweep finds a request whose
    /// revocation it did not see.
    pub(super) fn stop_revoked_exchanges(&self, persona: Option<&str>) {
        let _guard = lock(&self.exchange_lock);
        let mut stopped = Vec::new();
        for mut pair in self.exchange_pairs() {
            let mut changed = false;
            for request in &mut pair.requests {
                if !Self::exchange_pending(request)
                    || (!persona.is_some_and(|id| pair.a == id || pair.b == id)
                        && self.exchange_lease_current(&request.id))
                {
                    continue;
                }
                self.cancel_handoff(request);
                request.phase = Phase::Stopped;
                request.failed = true;
                request.reply = "Collaboration was revoked or a participant stopped. Send again if still needed.".into();
                stopped.push(request.clone());
                changed = true;
            }
            if changed && let Err(error) = self.save_pair(&pair) {
                eprintln!("revoke exchange: {error}");
            }
            self.wake_owed(&pair);
        }
        self.exchange_notices(&stopped);
    }
}
