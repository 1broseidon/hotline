//! A delivery from one teammate to another, and the collaboration grant it
//! needs first: the exchange itself, the person's card that authorizes it, and
//! the decision written back when the card is answered or expires.

use super::*;
use crate::driver::HOTLINE_BACKEND_ID;
use crate::session::{new_id, now_ms};

impl Room {
    pub(in crate::session) fn cancel_peer_exchange(&self, from: &str, to: &str) {
        if let Some(session) = lock(&self.peers.sessions).remove(&(from.into(), to.into())) {
            session.caller_capability.revoke();
            session.target_capability.revoke();
            session.driver.invalidate();
        }
    }
    /// One teammate's message to another, answered.
    ///
    /// Runs the target's peer turn to its end and hands back what it said, so
    /// the tool that asked can return the reply rather than promising one. The
    /// caller may be mid-turn on its own tape while this runs: nothing here
    /// touches the caller's session, only its tape's marker.
    #[cfg(test)]
    pub async fn deliver(
        self: &Arc<Self>,
        from: &str,
        to: &str,
        message: &str,
    ) -> Result<DeliverResult, String> {
        self.deliver_with_capability(from, to, message, None).await
    }

    /// The same delivery with the caller's live session lease when the
    /// request came from a teammate tool. Direct callers use a fresh lease;
    /// the normal tool path binds the temporary grant to the caller session.
    #[cfg(test)]
    pub(crate) async fn deliver_with_capability(
        self: &Arc<Self>,
        from: &str,
        to: &str,
        message: &str,
        caller_capability: Option<CapabilityLease>,
    ) -> Result<DeliverResult, String> {
        let _working = self.working()?;
        let asked = self.ask(from, to, message)?;
        self.exchange(asked, caller_capability).await
    }

    /// The checks a message passes before anything is started for it, and
    /// the claim on its thread.
    pub(in crate::session) fn ask(
        self: &Arc<Self>,
        from: &str,
        to: &str,
        message: &str,
    ) -> Result<Asked, String> {
        let (caller, target, message) = self.checked(from, to, message)?;
        let key = thread_key(&caller.id, &target.id)
            .ok_or_else(|| "Those two teammates cannot share a thread.".to_string())?;
        self.peers.begin(&key, &target.id)?;
        Ok(Asked {
            _answering: Answering {
                room: self.clone(),
                key: key.clone(),
            },
            caller,
            target,
            key,
            message: message.to_string(),
        })
    }

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

    /// The exchange a message starts: approval, the target's peer session,
    /// its turn, and the marker on both tapes as it goes.
    pub(in crate::session) async fn exchange(
        self: &Arc<Self>,
        asked: Asked,
        caller_capability: Option<CapabilityLease>,
    ) -> Result<DeliverResult, String> {
        let Asked {
            caller,
            target,
            key,
            message,
            _answering,
        } = asked;
        let message = message.as_str();
        let caller_capability =
            caller_capability.unwrap_or_else(|| self.capability_lease(&caller.id));
        let target_capability = self.capability_lease(&target.id);
        caller_capability.check()?;
        target_capability.check()?;
        let authorization = self
            .authorize_collaboration(
                &caller,
                &target,
                &caller_capability,
                &target_capability,
                false,
            )
            .await?;
        if let Err(error) = thread::ensure(self.log.root(), &key) {
            return Err(format!("That thread could not be opened: {error}"));
        }

        let session = self
            .peer_session(
                &caller,
                &target,
                &key,
                caller_capability.clone(),
                target_capability.clone(),
                authorization.scope,
            )
            .await?;
        caller_capability.check()?;
        target_capability.check()?;
        if !session.valid() {
            return Err("That peer session's capabilities have been revoked.".to_string());
        }
        if authorization.temporary {
            let scope = authorization
                .scope
                .expect("a temporary collaboration approval has a scope");
            if !self
                .peers
                .grant_session(&caller.id, &target.id, &session, scope)
            {
                return Err(
                    "That collaboration approval expired before the peer session could start."
                        .to_string(),
                );
            }
        }
        self.mark(&session, &caller, &target, PeerStatus::Open);
        self.append_thread(
            &session,
            TranscriptEvent::User {
                id: new_id(),
                ts: now_ms(),
                text: message.to_string(),
                attachments: None,
                reactions: None,
                reply_to: None,
                scheduled: None,
                ring: None,
                receipt: None,
                client: None,
            },
        );

        if !session.valid() {
            self.mark(&session, &caller, &target, PeerStatus::Failed);
            return Err("That peer session's capabilities have been revoked.".to_string());
        }
        // The same funnel as a tape: what the peer says between its tool
        // calls is thinking, and the asker hears the report.
        let driven = crate::session::runner::drive(
            session.driver.as_ref(),
            envelope(&caller, message),
            self.reach_of(&target.id),
            None,
            |event, asked| {
                self.append_thread(&session, event);
                if asked {
                    self.mark(&session, &caller, &target, PeerStatus::Waiting);
                }
            },
        )
        .await;
        // A permission the turn left open on the thread is a button nobody is
        // behind, exactly as on a tape — and no seat is shown a peer card, so
        // the child's own timeout is the only thing that ever answered it.
        if driven.asked {
            let stream = StreamId::Pair(session.thread_key.clone());
            for expired in
                crate::log::expire_orphaned_permissions(&self.log.load(&stream), now_ms())
            {
                if let Err(error) = self.log.append(&stream, &expired) {
                    eprintln!(
                        "the thread {} could not be appended to: {error}",
                        session.thread_key
                    );
                }
            }
        }
        *lock(&session.last_used) = now_ms();
        self.let_go_of_computer(&target.id, &session.holder);

        if !session.valid() {
            self.mark(&session, &caller, &target, PeerStatus::Failed);
            return Err("That peer session's capabilities have been revoked.".to_string());
        }
        if let Some(error) = driven.failure {
            self.mark(&session, &caller, &target, PeerStatus::Failed);
            return Err(format!("{} could not answer: {error}", target.name));
        }
        lock(&session.marker).exchanges += 1;
        self.mark(&session, &caller, &target, PeerStatus::Done);
        Ok(DeliverResult {
            from: target.name,
            reply: driven.replies.join("\n\n"),
        })
    }

    /// Checks the directional collaboration policy before a thread or target
    /// session is created. A Whole machine Hotline Agent caller has implicit
    /// authority; every workspace caller needs either the recipient's stable
    /// sender grant or an approval tied to the two live capability leases.
    pub(in crate::session) async fn authorize_collaboration(
        self: &Arc<Self>,
        caller: &Persona,
        target: &Persona,
        caller_capability: &CapabilityLease,
        target_capability: &CapabilityLease,
        handoff: bool,
    ) -> Result<CollaborationAuthorization, String> {
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
        // Whatever a teammate may do on its own desk, it gains nothing by
        // crossing a link: one on a linked desk is never implicitly trusted.
        let implicit = caller.node.is_none()
            && caller.backend_id == HOTLINE_BACKEND_ID
            && caller.reach.unwrap_or_default() == Reach::Machine;
        let informed = self.log.load(&StreamId::Room).iter().any(|v| {
            v["kind"] == "collaboration_informed"
                && v["id"] == format!("collaboration:{}:{}", caller.id, target.id)
                && v["deleted"] != true
        });
        if (!handoff || informed)
            && (implicit
                || target
                    .allowed_senders
                    .iter()
                    .any(|sender| sender == &caller.id))
        {
            return Ok(CollaborationAuthorization {
                temporary: false,
                scope: None,
            });
        }
        if let Some(scope) = self.peers.session_granted(&caller.id, &target.id) {
            return Ok(CollaborationAuthorization {
                temporary: true,
                scope: Some(scope),
            });
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
        self.write(
            &card_on,
            &TranscriptEvent::Permission {
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
                Ok(CollaborationAuthorization {
                    temporary: true,
                    scope: Some(scope),
                })
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
                Ok(CollaborationAuthorization {
                    temporary: false,
                    scope: None,
                })
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
