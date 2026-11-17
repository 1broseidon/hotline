//! What one teammate needs to ask another to work: nothing when the other
//! has no more access than it, else the person's card, "Goldie wants Huck to
//! do work", answered Allow (this request), Always (this direction, until the
//! receiver's access grows or it moves to another agent) or Deny; and the
//! decision written back when the card is answered or expires. A receiver on
//! another agent sets its own access, which Hotline cannot see, so asking one
//! raises the card too, unless the sender has the whole machine.

use super::*;
use crate::session::{new_id, now_ms};

/// Where a request up in access was made: the thread it was said in (a
/// call's card is the call's), and the inbox item whose quiet turn said it,
/// which its card names.
#[derive(Default)]
pub(in crate::session) struct Asked<'a> {
    pub(in crate::session) thread: Option<&'a crate::contract::ThreadId>,
    pub(in crate::session) prompted: Option<String>,
}

impl Room {
    /// Checks the directional collaboration policy before anything is handed
    /// to the recipient ([`Room::collaboration_granted`]): a recipient with no
    /// more access than the caller needs nothing; one with more needs the
    /// person's Always for this direction, or their Allow on a card raised
    /// for this request and tied to the two live capability leases.
    pub(in crate::session) async fn authorize_collaboration(
        self: &Arc<Self>,
        caller: &Persona,
        target: &Persona,
        caller_capability: &CapabilityLease,
        target_capability: &CapabilityLease,
        handoff: bool,
        asked: Asked<'_>,
    ) -> Result<(), String> {
        let Asked {
            thread: origin,
            prompted,
        } = asked;
        caller_capability.check()?;
        target_capability.check()?;
        // Discovery ran before these leases were captured. Re-read both
        // policies so an old Whole machine snapshot cannot authorize a
        // request under a newer workspace-only generation.
        let caller = self.participant(&caller.id)?;
        let target = self.persona(&target.id)?;
        caller_capability.check()?;
        target_capability.check()?;
        let scope = self.peers.scope(&caller.id, &target.id);
        if self.collaboration_granted(&caller, &target, handoff) {
            return Ok(());
        }
        let request_id = format!("{COLLAB_REQUEST_PREFIX}{}", new_id());
        let caller_name = crate::session::links::shown(&caller);
        let agent = crate::session::consent::another_agent(&target).then(|| {
            crate::driver::acp::registry::name_of(self.log.root(), &target.backend_id)
                .unwrap_or_else(|| target.backend_id.clone())
        });
        let title = collaboration_title(&caller_name, &target, agent.as_deref());
        let options = collaboration_options();
        // The card is the person's, on the desk whose teammate would do the
        // work: the caller's tape, or the recipient's when the caller lives
        // on a linked desk and has no tape here.
        let card_on = if caller.node.is_some() {
            target.id.clone()
        } else {
            caller.id.clone()
        };
        let receiver = self.peers.wait(CollaborationRequest {
            request_id: request_id.clone(),
            card_on: card_on.clone(),
            caller_id: caller.id.clone(),
            target_id: target.id.clone(),
            title: title.clone(),
            caller_capability: caller_capability.clone(),
            target_capability: target_capability.clone(),
            scope,
            from_item: prompted.clone(),
        })?;
        let _wait_guard = CollaborationWaitGuard {
            room: Arc::downgrade(self),
            request_id: request_id.clone(),
            card_on: card_on.clone(),
        };
        let home = origin
            .filter(|thread| {
                caller.node.is_none() && thread.kind == crate::contract::ThreadKind::Call
            })
            .cloned()
            .unwrap_or_else(|| crate::contract::ThreadId::dm(&card_on));
        self.write_home(
            &card_on,
            &home,
            &TranscriptEvent::Permission {
                from: None,
                id: format!("perm:{request_id}"),
                ts: now_ms(),
                request_id: request_id.clone(),
                title,
                options,
                decision: None,
                decided_option_name: None,
                from_item: prompted,
            },
        );
        let decision = match tokio::time::timeout(crate::session::HUMAN_DEADLINE, receiver).await {
            Ok(Ok(decision)) => decision,
            Ok(Err(_)) => return Err("The collaboration request was cancelled.".to_string()),
            Err(_) => {
                return Err("The collaboration request expired before it was answered.".to_string());
            }
        };
        caller_capability.check()?;
        target_capability.check()?;
        if !self.peers.scope_current(&caller.id, &target.id, scope) {
            return Err("That collaboration approval expired before work started.".to_string());
        }
        match decision {
            // This request only: the next one up in access asks again.
            CollaborationDecision::Once => Ok(()),
            CollaborationDecision::Permanent => {
                if !self
                    .persona(&target.id)?
                    .allowed_senders
                    .iter()
                    .any(|sender| sender == &caller.id)
                {
                    return Err(
                        "That collaboration grant was revoked before work started.".to_string()
                    );
                }
                Ok(())
            }
            CollaborationDecision::Deny => Err(format!(
                "The operator denied {}'s request to ask {} to work.",
                crate::session::links::shown(&caller),
                target.name
            )),
        }
    }

    /// Persists a directional standing grant on the recipient. The caller
    /// holds the policy update lock when this runs, so a removal cannot be
    /// reordered behind an approval that was already taken from the map.
    pub(in crate::session) fn allow_sender(
        &self,
        target_id: &str,
        sender_id: &str,
    ) -> Result<(), String> {
        let mut target = self.persona(target_id)?;
        crate::session::consent::record_informed(&self.log, sender_id, target_id)?;
        if !target
            .allowed_senders
            .iter()
            .any(|sender| sender == sender_id)
        {
            target.allowed_senders.push(sender_id.to_string());
        }
        target.updated_at = now_ms();
        room::append_persona(&self.log, &target)
    }

    /// A card written by this room, superseding the pending card with one
    /// operator decision. The event stays on the caller's tape so the owner
    /// of the request can see what authority was delegated.
    pub(in crate::session) fn write_collaboration_decision(
        &self,
        wait: &CollaborationWait,
        decision: &str,
        option_name: &str,
    ) {
        self.write(
            &wait.card_on,
            &TranscriptEvent::Permission {
                from: None,
                id: format!("perm:{}", wait.request_id),
                ts: now_ms(),
                request_id: wait.request_id.clone(),
                title: wait.title.clone(),
                options: collaboration_options(),
                decision: Some(decision.to_string()),
                decided_option_name: Some(option_name.to_string()),
                from_item: wait.from_item.clone(),
            },
        );
    }

    /// The wait behind a card, taken by the one whose tape the card is on.
    pub(in crate::session) fn take_collaboration_wait(
        &self,
        request_id: &str,
        card_on: &str,
    ) -> Option<CollaborationWait> {
        self.peers.take_wait(request_id, card_on)
    }

    /// Whether `caller` may ask `target` to work without the person's card:
    /// when `target` has no more access than `caller`, it gains nothing a
    /// card would guard, so nothing asks; up in access, or to a teammate on
    /// another agent from one without the whole machine
    /// ([`crate::session::consent::asks_the_person`]), only the person's
    /// Always for this direction (`allowedSenders`, and for work the informed
    /// record beside it) lets it through unasked.
    pub(in crate::session) fn collaboration_granted(
        &self,
        caller: &Persona,
        target: &Persona,
        handoff: bool,
    ) -> bool {
        // Whatever a teammate may do on its own desk, it gains nothing by
        // crossing a link: one on a linked desk is always asked about.
        if caller.node.is_none() && !crate::session::consent::asks_the_person(caller, target) {
            return true;
        }
        let room = self.log.load(&StreamId::Room);
        let informed = crate::session::consent::informed(&room, &caller.id, &target.id);
        (!handoff || informed)
            && target
                .allowed_senders
                .iter()
                .any(|sender| sender == &caller.id)
    }

    pub(in crate::session) fn settle_collaboration(&self, persona_id: &str) {
        for wait in self.peers.settle_waits(persona_id) {
            self.write_collaboration_decision(&wait, "expired", "Expired unanswered");
            let _ = wait.sender.send(CollaborationDecision::Deny);
        }
        self.settle_invalid_collaboration();
    }

    /// The person stopped two teammates' exchange: a first-contact card up
    /// between them is expired with it, and what waited on it is denied.
    pub(in crate::session) fn settle_pair_collaboration(&self, a: &str, b: &str) {
        for wait in self.peers.settle_pair_waits(a, b) {
            self.write_collaboration_decision(&wait, "expired", "Expired unanswered");
            let _ = wait.sender.send(CollaborationDecision::Deny);
        }
    }

    pub(in crate::session) fn settle_invalid_collaboration(&self) {
        for wait in self.peers.settle_invalid_waits() {
            self.write_collaboration_decision(&wait, "expired", "Expired unanswered");
            let _ = wait.sender.send(CollaborationDecision::Deny);
        }
    }

    pub(in crate::session) fn settle_all_collaboration(&self) {
        for wait in self.peers.settle_all_waits() {
            self.write_collaboration_decision(&wait, "expired", "Expired unanswered");
            let _ = wait.sender.send(CollaborationDecision::Deny);
        }
    }
}
