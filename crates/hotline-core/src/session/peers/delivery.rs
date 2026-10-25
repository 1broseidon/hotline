//! The collaboration grant one teammate needs to ask another: the person's
//! card that authorizes it, and the decision written back when the card is
//! answered or expires.

use super::*;
use crate::contract::Reach;
use crate::session::{new_id, now_ms};

impl Room {
    /// Who is sending to whom, and the message trimmed, or why not.
    pub(in crate::session) fn checked<'m>(
        &self,
        from: &str,
        to: &str,
        message: &'m str,
    ) -> Result<(Persona, Persona, &'m str), String> {
        let caller = self.participant(from)?;
        let target = self.teammate_named(to)?;
        if caller.id == target.id {
            return Err("A teammate cannot message itself.".to_string());
        }
        let message = message.trim();
        if message.is_empty() {
            return Err("A message to a teammate cannot be empty.".to_string());
        }
        if message.chars().count() > TEAMMATE_MESSAGE_MAX {
            return Err(format!(
                "A message to a teammate is at most {TEAMMATE_MESSAGE_MAX} characters."
            ));
        }
        Ok((caller, target, message))
    }

    /// Checks the directional collaboration policy before anything is handed
    /// to the recipient. A Whole machine Hotline Agent caller has implicit
    /// authority; every workspace caller needs either the recipient's stable
    /// sender grant or an approval tied to the two live capability leases.
    pub(in crate::session) async fn authorize_collaboration(
        self: &Arc<Self>,
        caller: &Persona,
        target: &Persona,
        caller_capability: &CapabilityLease,
        target_capability: &CapabilityLease,
        handoff: bool,
        origin: Option<&crate::contract::ThreadId>,
    ) -> Result<(), String> {
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
        let title = collaboration_title(&caller_name, &target.name);
        let options = collaboration_options(&caller_name);
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
            caller_name,
            target_id: target.id.clone(),
            target_name: target.name.clone(),
            caller_capability: caller_capability.clone(),
            target_capability: target_capability.clone(),
            scope,
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
            CollaborationDecision::Session => {
                lock(&self.peers.session_grants).insert(
                    (caller.id.clone(), target.id.clone()),
                    SessionGrant {
                        caller: caller_capability.clone(),
                        target: target_capability.clone(),
                        scope,
                    },
                );
                Ok(())
            }
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

    /// Whether `caller` may ask `target` to work without the person's card.
    pub(crate) fn collaboration_granted(
        &self,
        caller: &Persona,
        target: &Persona,
        _handoff: bool,
    ) -> bool {
        // Whatever a teammate may do on its own desk, it gains nothing by
        // crossing a link: one on a linked desk is never implicitly trusted.
        let implicit = caller.node.is_none()
            && caller.reach.unwrap_or_default() == Reach::Machine;
        let allowed = implicit
            || target
                .allowed_senders
                .iter()
                .any(|sender| sender == &caller.id);
        allowed || self.peers.session_granted(&caller.id, &target.id)
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
        self.log.append(&StreamId::Room, &serde_json::json!({"kind":"collaboration_informed",
            "id":format!("collaboration:{sender_id}:{target_id}"), "from":sender_id, "to":target_id}))
            .map_err(|e|e.to_string())?;
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
                title: collaboration_title(&wait.caller_name, &wait.target_name),
                options: collaboration_options(&wait.caller_name),
                decision: Some(decision.to_string()),
                decided_option_name: Some(option_name.to_string()),
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

    pub(in crate::session) fn settle_collaboration(&self, persona_id: &str) {
        for wait in self.peers.settle_waits(persona_id) {
            self.write_collaboration_decision(&wait, "expired", "Expired unanswered");
            let _ = wait.sender.send(CollaborationDecision::Deny);
        }
        self.settle_invalid_collaboration();
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
