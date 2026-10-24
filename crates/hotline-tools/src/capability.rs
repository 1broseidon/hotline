//! A session's authority over its tools, which every tool checks before it acts.
//!
//! Every field is private: outside this module a lease can only be captured
//! from an epoch, narrowed or tied to another lease, and revoked, so a crate
//! that holds one cannot forge a current lease or revive a dead one.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// The authority shared by every handle a session hands to a driver.
///
/// A policy update first advances the generation and closes the epoch. The
/// room opens it again only after the new record is durable, so a session that
/// races the update can never capture the old policy in a usable lease. A
/// stop advances the generation while leaving the epoch open: a later,
/// explicit start may use the current policy, while handles from the stopped
/// session remain permanently dead. A session's replacement, as a chapter
/// turns over, ends that session's handles the same way, but not the
/// teammate's authority: a [lasting](Self::lasting) lease outlives it.
#[derive(Clone)]
pub struct CapabilityEpoch {
    state: Arc<Mutex<CapabilityState>>,
}

struct CapabilityState {
    /// Advanced by every stop, session replacement and policy update: what an
    /// ordinary lease follows.
    generation: u64,
    /// Advanced by a stop and a policy update only: what a lasting lease
    /// follows.
    authority: u64,
    active: bool,
}

impl Default for CapabilityEpoch {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(CapabilityState {
                generation: 0,
                authority: 0,
                active: true,
            })),
        }
    }
}

/// An immutable snapshot of one session's authority. The snapshot is cheap to
/// clone into tool closures, but validity always consults the shared epoch.
#[derive(Clone)]
pub struct CapabilityLease {
    epoch: CapabilityEpoch,
    generation: u64,
    authority: u64,
    /// Follows the teammate's authority alone, not the session it was
    /// captured beside.
    lasting: bool,
    active_at_capture: bool,
    revoked: Arc<AtomicBool>,
    dependencies: Vec<Arc<CapabilityLease>>,
}

impl CapabilityEpoch {
    pub fn lease(&self) -> CapabilityLease {
        self.capture(false)
    }

    /// The teammate's authority as a request between two teammates holds it:
    /// it outlives the session it was captured beside, a replacement of that
    /// session included, and ends as every lease does with a stop or a policy
    /// update.
    pub fn lasting(&self) -> CapabilityLease {
        self.capture(true)
    }

    fn capture(&self, lasting: bool) -> CapabilityLease {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        CapabilityLease {
            epoch: self.clone(),
            generation: state.generation,
            authority: state.authority,
            lasting,
            active_at_capture: state.active,
            revoked: Arc::new(AtomicBool::new(false)),
            dependencies: Vec::new(),
        }
    }

    /// Quarantines all existing and newly captured leases until the room has
    /// appended the replacement policy and calls [`Self::activate`].
    pub fn invalidate(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.generation = state.generation.wrapping_add(1);
        state.authority = state.authority.wrapping_add(1);
        state.active = false;
    }

    /// Invalidates old handles while keeping the authority available to a
    /// later explicit start under the still-current policy.
    pub fn stop(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.generation = state.generation.wrapping_add(1);
        state.authority = state.authority.wrapping_add(1);
    }

    /// Invalidates the handles of a session being replaced by the next one,
    /// as a chapter turns over, and leaves the teammate's authority, and so
    /// every lasting lease, as it was. Like a stop, it never reopens a
    /// quarantined epoch.
    pub fn replace_session(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.generation = state.generation.wrapping_add(1);
    }

    pub fn activate(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.active = true;
    }
}

impl CapabilityLease {
    /// Whether this lease still names the room's active generation.
    pub fn is_current(&self) -> bool {
        if self.revoked.load(Ordering::SeqCst)
            || self.dependencies.iter().any(|lease| !lease.is_current())
        {
            return false;
        }
        let state = self
            .epoch
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let same_session = self.lasting || state.generation == self.generation;
        let epoch_still_active = state.active && state.authority == self.authority && same_session;
        self.active_at_capture && epoch_still_active
    }

    /// Refuses a call made through a stale driver, tool closure, or workspace.
    pub fn check(&self) -> Result<(), String> {
        if self.is_current() {
            Ok(())
        } else {
            Err("This teammate's capabilities have been revoked.".to_string())
        }
    }

    /// Permanently invalidates this lease, every handle cloned from it and
    /// every lease scoped from it. A request between two teammates holds a
    /// lease for each side, so stopping it ends both while each teammate's
    /// epoch stays current.
    pub fn revoke(&self) {
        self.revoked.store(true, Ordering::SeqCst);
    }

    /// Creates an independently revocable child lease in this same session
    /// generation. A thread, a job's agent or a request between teammates
    /// must be able to end without revoking the teammate's main tool handles,
    /// while a stop or policy change on the teammate still ends it through
    /// the shared epoch.
    pub fn scoped(&self) -> Self {
        Self {
            epoch: self.epoch.clone(),
            generation: self.generation,
            authority: self.authority,
            lasting: self.lasting,
            active_at_capture: self.active_at_capture,
            revoked: Arc::new(AtomicBool::new(false)),
            dependencies: vec![Arc::new(self.clone())],
        }
    }

    /// Delegated tools need both the recipient's authority and the requester's
    /// authority. Keeping the whole dependency includes session replacement,
    /// as well as explicit revocation, even across different teammates.
    pub fn with_dependency(mut self, requester: &Self) -> Self {
        self.dependencies.push(Arc::new(requester.clone()));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{CapabilityEpoch, CapabilityLease};

    #[test]
    fn a_lease_captured_during_quarantine_never_revives() {
        let epoch = CapabilityEpoch::default();
        let before = epoch.lease();

        epoch.invalidate();
        let during = epoch.lease();
        assert!(!before.is_current());
        assert!(!during.is_current());

        epoch.activate();
        assert!(!before.is_current());
        assert!(!during.is_current());
        assert!(epoch.lease().is_current());
    }

    #[test]
    fn stopping_invalidates_old_leases_but_allows_a_fresh_start() {
        let epoch = CapabilityEpoch::default();
        let old = epoch.lease();

        epoch.stop();

        assert!(!old.is_current());
        assert!(epoch.lease().is_current());
    }

    #[test]
    fn stopping_during_quarantine_does_not_reopen_it() {
        let epoch = CapabilityEpoch::default();
        epoch.invalidate();

        epoch.stop();

        assert!(!epoch.lease().is_current());
        epoch.activate();
        assert!(epoch.lease().is_current());
    }

    #[test]
    fn a_lasting_lease_outlives_a_session_replacement_but_not_a_stop_or_a_policy_update() {
        let epoch = CapabilityEpoch::default();
        let session = epoch.lease();
        let lasting = epoch.lasting();
        let scoped = lasting.scoped();

        epoch.replace_session();
        assert!(!session.is_current());
        assert!(lasting.is_current());
        assert!(scoped.is_current());
        assert!(epoch.lease().is_current());

        epoch.stop();
        assert!(!lasting.is_current());
        assert!(!scoped.is_current());

        let lasting = epoch.lasting();
        epoch.invalidate();
        let during = epoch.lasting();
        epoch.activate();
        assert!(!lasting.is_current());
        assert!(
            !during.is_current(),
            "a capture in quarantine never revives"
        );
        assert!(epoch.lasting().is_current());
    }

    #[test]
    fn replacing_a_session_during_quarantine_does_not_reopen_it() {
        let epoch = CapabilityEpoch::default();
        epoch.invalidate();

        epoch.replace_session();

        assert!(!epoch.lease().is_current());
        assert!(!epoch.lasting().is_current());
        epoch.activate();
        assert!(epoch.lasting().is_current());
    }

    #[test]
    fn revoking_one_lease_does_not_revoke_a_new_capture() {
        let epoch = CapabilityEpoch::default();
        let peer = epoch.lease();
        let current = epoch.lease();

        peer.revoke();

        assert!(!peer.is_current());
        assert!(current.is_current());
        assert!(epoch.lease().is_current());
    }

    #[test]
    fn a_scoped_lease_follows_ancestors_but_not_its_children() {
        let epoch = CapabilityEpoch::default();
        let parent = epoch.lease();
        let child = parent.scoped();
        let grandchild = child.scoped();

        child.revoke();

        assert!(parent.is_current());
        assert!(!child.is_current());
        assert!(!grandchild.is_current());

        let child = parent.scoped();
        let grandchild = child.scoped();
        grandchild.revoke();

        assert!(parent.is_current());
        assert!(child.is_current());
        assert!(!grandchild.is_current());

        parent.revoke();
        assert!(!child.is_current());
    }

    #[test]
    fn delegated_authority_requires_both_teammates_current_sessions() {
        let caller = CapabilityEpoch::default();
        let recipient = CapabilityEpoch::default();
        let independent = recipient.lease();
        let delegated = recipient.lease().with_dependency(&caller.lease());
        assert!(delegated.is_current());
        caller.stop();
        assert!(!delegated.is_current());
        assert!(independent.is_current());

        let delegated = recipient.lease().with_dependency(&caller.lease());
        recipient.stop();
        assert!(!delegated.is_current());
        assert!(caller.lease().is_current());
    }

    #[test]
    fn capability_check_explains_a_stale_handle() {
        let epoch = CapabilityEpoch::default();
        let lease: CapabilityLease = epoch.lease();
        epoch.invalidate();

        assert_eq!(
            lease.check().unwrap_err(),
            "This teammate's capabilities have been revoked."
        );
    }
}
