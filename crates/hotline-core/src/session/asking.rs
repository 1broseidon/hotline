//! A teammate asking a person: permission requests, `ask_human` cards and
//! their answers, and the cards that expire, are withdrawn or are superseded.

use super::*;
use crate::contract::ThreadKind;

impl Room {
    /// Answers a permission the agent is waiting on.
    ///
    /// Collaboration cards belong to the room, not to a driver. They are
    /// routed here before the normal driver answer path so an ACP child cannot
    /// decide whether another teammate may use its workspace. A standing
    /// grant is appended while the policy lock is held; the lock is acquired
    /// only after the pending wait has been taken, so it is never held while
    /// the delivery waits for the operator.
    pub async fn answer_permission(
        &self,
        persona_id: &str,
        request_id: &str,
        option_id: &str,
    ) -> Result<(), String> {
        if request_id.starts_with(peers::COLLAB_REQUEST_PREFIX) {
            let Some(wait) = self.take_collaboration_wait(request_id, persona_id) else {
                return Err("That request is no longer waiting for an answer.".to_string());
            };
            let Some((decision, option_name)) =
                peers::collaboration_decision(option_id, &wait.caller_name)
            else {
                self.write_collaboration_decision(&wait, "expired", "Invalid answer");
                let _ = wait.sender.send(peers::CollaborationDecision::Deny);
                return Err("That collaboration card has no such answer.".to_string());
            };

            if wait.caller_capability.check().is_err()
                || wait.target_capability.check().is_err()
                || !self
                    .peers
                    .scope_current(&wait.caller_id, &wait.target_id, wait.scope)
                || wait.sender.is_closed()
            {
                self.write_collaboration_decision(&wait, "expired", "Expired unanswered");
                let _ = wait.sender.send(peers::CollaborationDecision::Deny);
                return Err("That request is no longer waiting for an answer.".to_string());
            }

            if decision == peers::CollaborationDecision::Permanent {
                let policy_updates = self.policy_update_lock();
                let _held = policy_updates.lock().await;
                if wait.caller_capability.check().is_err()
                    || wait.target_capability.check().is_err()
                    || !self
                        .peers
                        .scope_current(&wait.caller_id, &wait.target_id, wait.scope)
                    || wait.sender.is_closed()
                {
                    self.write_collaboration_decision(&wait, "expired", "Expired unanswered");
                    let _ = wait.sender.send(peers::CollaborationDecision::Deny);
                    return Err("That request is no longer waiting for an answer.".to_string());
                }
                if let Err(error) = self.allow_sender(&wait.target_id, &wait.caller_id) {
                    self.write_collaboration_decision(&wait, "expired", "Grant failed");
                    let _ = wait.sender.send(peers::CollaborationDecision::Deny);
                    return Err(error);
                }
            }

            self.write_collaboration_decision(&wait, option_id, &option_name);
            let _ = wait.sender.send(decision);
            return Ok(());
        }

        if let Some(TranscriptEvent::Permission {
            from: Some(from), ..
        }) = self.permission_card(persona_id, request_id)
            && from.kind == ThreadKind::Call
        {
            return self
                .answer_side_permission(&from.thread, request_id, option_id)
                .await;
        }

        let session = self.session(persona_id)?;
        if !session.driver.answer_permission(request_id, option_id) {
            return Err("That request is no longer waiting for an answer.".to_string());
        }
        let Some(card) = self.permission_card(persona_id, request_id) else {
            return Ok(());
        };
        let TranscriptEvent::Permission {
            id,
            request_id,
            title,
            options,
            ..
        } = card
        else {
            return Ok(());
        };
        let decided_option_name = options
            .iter()
            .find(|option| option.option_id == option_id)
            .map(|option| option.name.clone());
        self.write(
            persona_id,
            &TranscriptEvent::Permission {
                from: None,
                id,
                ts: now_ms(),
                request_id,
                title,
                options,
                decision: Some(option_id.to_string()),
                decided_option_name,
            },
        );
        Ok(())
    }

    /// Posts a `human_action` card and returns: the asking turn goes on,
    /// and the person's answer comes back to the teammate as a delivery
    /// whenever they give it (see [`Room::answer_human`]). The card is the
    /// record, so it outlives the turn, a stop, and a restart. It stays up
    /// [`ASK_HOURS`], or the `hours` the teammate asked for up to
    /// [`ASK_MAX_HOURS`], and the teammate is handed its id so it can take
    /// the card back with [`Room::withdraw_human`].
    pub fn ask_human(
        &self,
        persona_id: &str,
        reason: &str,
        home: &ThreadId,
        hours: Option<u32>,
    ) -> Result<String, String> {
        self.ask_card(persona_id, reason, home, hours, None)
    }

    pub(crate) fn ask_question(
        &self,
        persona_id: &str,
        reason: &str,
        home: &ThreadId,
        capability: CapabilityLease,
    ) -> Result<String, String> {
        capability.check()?;
        self.ask_card(
            persona_id,
            reason,
            home,
            None,
            Some(Question {
                capability,
                person_name: None,
            }),
        )
    }

    pub(super) fn ask_card(
        &self,
        persona_id: &str,
        reason: &str,
        home: &ThreadId,
        hours: Option<u32>,
        question: Option<Question>,
    ) -> Result<String, String> {
        self.persona(persona_id)?;
        let reason = reason.trim();
        if reason.len() < 3 {
            return Err("request_human needs a `reason` of at least three characters.".to_string());
        }
        let reason: String = reason.chars().take(500).collect();
        let hours = hours.unwrap_or(ASK_HOURS).clamp(1, ASK_MAX_HOURS);
        let ts = now_ms();
        let action_id = if question.as_ref().is_some_and(|q| q.person_name.is_some()) {
            format!("person-name:{}", new_id())
        } else {
            new_id()
        };
        let work =
            matches!(home.kind, ThreadKind::Side | ThreadKind::Call).then_some(home.key.as_str());
        self.link_handoff_human(work, &action_id)?;
        let informational = question.is_some();
        if let Some(question) = question {
            let mut questions = lock(&self.voice_questions);
            questions.retain(|_, question| question.capability.is_current());
            questions.insert(action_id.clone(), question);
        }
        self.write_home(
            persona_id,
            home,
            &TranscriptEvent::HumanAction {
                from: None,
                id: format!("human:{action_id}"),
                ts,
                action_id: action_id.clone(),
                reason: reason.clone(),
                question: informational.then_some(true),
                status: HumanActionStatus::Pending,
                note: None,
                delivers: Some(true),
                expires_at: Some(ts + i64::from(hours) * 60 * 60_000),
                thread: work.map(str::to_string),
            },
        );
        self.push.notify_in(
            Some(&if home.kind == ThreadKind::Call {
                ThreadId::dm(persona_id)
            } else {
                home.clone()
            }),
            &self.needs_you(persona_id),
            &reason,
            persona_id,
            Some(crate::push::Waiting::Human {
                action_id: action_id.clone(),
            }),
        );
        let lasts = if hours == 1 {
            "an hour".to_string()
        } else {
            format!("{hours} hours")
        };
        Ok(format!(
            "Asked. The card is in front of the person now, and stays up for {lasts} unless they answer it. Their answer, and anything they type with it, will arrive later as its own message in this conversation. Carry on with whatever does not depend on it; if nothing does, say what you are waiting for and end your reply. This request's id is `{action_id}`: if you stop needing it, because you found another way or the work moved on, call withdraw_request with that id so the person is no longer asked."
        ))
    }

    /// Takes down a card that delivers because the teammate that raised it
    /// no longer needs the person. The card is looked up among the caller's
    /// own, so a teammate can withdraw nothing but its own; it is superseded
    /// as withdrawn and nothing is delivered back. Held under the same lock
    /// as an answer, so an answer and a withdrawal racing settle it once.
    pub fn withdraw_human(&self, persona_id: &str, action_id: &str) -> Result<(), String> {
        let _answering = lock(&self.answering_later);
        let Some((
            TranscriptEvent::HumanAction {
                status: HumanActionStatus::Pending,
                delivers: Some(true),
                ..
            },
            _,
        )) = self.human_card(persona_id, action_id)
        else {
            return Err("No request of yours is waiting under that id: it was answered, expired or withdrawn already, or it is not an id request_human gave you.".to_string());
        };
        self.unlink_handoff_human(persona_id, action_id)?;
        self.supersede_human(persona_id, action_id, HumanActionStatus::Withdrawn, None);
        Ok(())
    }

    /// Resolves a `request_human` card and supersedes it.
    ///
    /// The card is answered off the tape, so it can be answered after a
    /// restart, and the answer goes to the teammate as a delivery. Refused
    /// when nothing is behind that id any more — it expired, was withdrawn,
    /// or somebody else answered first — so a stale button cannot quietly
    /// settle a card that is already gone.
    pub fn answer_human(
        self: &Arc<Self>,
        persona_id: &str,
        action_id: &str,
        status: HumanAnswer,
        note: Option<String>,
    ) -> Result<(), String> {
        let status = match status {
            HumanAnswer::Done => HumanActionStatus::Done,
            HumanAnswer::Declined => HumanActionStatus::Dismissed,
        };
        let note = note
            .map(|note| note.trim().chars().take(2_000).collect::<String>())
            .filter(|note| !note.is_empty());
        self.answer_later(persona_id, action_id, status, note)
    }

    /// The answer to a card that delivers: superseded on the tape, then
    /// handed to the teammate. The card is the one lock, so two answers
    /// racing each other settle it once.
    fn answer_later(
        self: &Arc<Self>,
        persona_id: &str,
        action_id: &str,
        status: HumanActionStatus,
        note: Option<String>,
    ) -> Result<(), String> {
        let _answering = lock(&self.answering_later);
        let Some((
            TranscriptEvent::HumanAction {
                reason,
                status: HumanActionStatus::Pending,
                delivers: Some(true),
                expires_at,
                ..
            },
            home,
        )) = self.human_card(persona_id, action_id)
        else {
            return Err("That request is no longer waiting for an answer.".to_string());
        };
        if action_id.starts_with("person-name:") {
            self.settle_person_name(action_id, status, expires_at)?;
        }
        self.supersede_human(persona_id, action_id, status, note.clone());
        self.deliver_answer(persona_id, action_id, &reason, status, note, home);
        Ok(())
    }

    /// An answer, or the lack of one, on its way to the teammate that asked: to
    /// the thread the card was raised in.
    fn deliver_answer(
        self: &Arc<Self>,
        persona_id: &str,
        action_id: &str,
        reason: &str,
        status: HumanActionStatus,
        note: Option<String>,
        home: ThreadId,
    ) {
        let room = self.clone();
        let persona_id = persona_id.to_string();
        let action_id = action_id.to_string();
        let reason = reason.to_string();
        tokio::spawn(async move {
            if let Err(error) = room
                .hand_over_answer(&persona_id, &action_id, &reason, status, note, home)
                .await
            {
                eprintln!("an answer to {persona_id}'s request could not be delivered: {error}");
            }
        });
    }

    /// Hands the person's answer to a card to the agent that raised it: in its
    /// work thread when the card was raised in one (waking it if it was parked,
    /// and carrying on a handoff still waiting behind it; once that handoff is
    /// stopped, the answer is heard as a plain one), in its job's thread while
    /// the job lives, else in its DM. Handing over the same answer twice
    /// delivers it once.
    pub(super) async fn hand_over_answer(
        self: &Arc<Self>,
        persona_id: &str,
        action_id: &str,
        reason: &str,
        status: HumanActionStatus,
        note: Option<String>,
        home: ThreadId,
    ) -> Result<(), String> {
        let cause = DeliveryCause::Answer {
            action_id: action_id.to_string(),
            status,
            about: peers::about(reason),
        };
        let delivery_id = format!("human-answer:{action_id}");
        // A tap with no note still says how it came out: on a call or work
        // thread the delivery text is all the voice side sees of it.
        let outcome = match status {
            HumanActionStatus::Done => "The person answered yes (marked it done).",
            HumanActionStatus::Dismissed => "The person answered no (declined it).",
            _ => "",
        };
        let text = match home.kind {
            ThreadKind::Side | ThreadKind::Call => note.unwrap_or_else(|| outcome.to_string()),
            _ => note.unwrap_or_default(),
        };
        let from = DeliveryFrom::new(&home, Some(action_id.to_string()));
        match home.kind {
            ThreadKind::Side | ThreadKind::Call => {
                let request = self
                    .handoff_awaiting(persona_id, action_id)
                    .filter(|handoff| self.handoff_is_open(handoff))
                    .map(|request| turns::RequestLine {
                        request,
                        answer: Some(action_id.to_string()),
                    });
                self.deliver_into_work(&home.key, &delivery_id, cause, from, text, request)
                    .await
            }
            ThreadKind::Job if self.job_lives(&home.key) => {
                self.deliver_into_job(&home.key, &delivery_id, cause, from, text, None)
            }
            _ => {
                self.deliver_identified(persona_id, &delivery_id, cause, text, None)
                    .await
            }
        }
    }

    /// One line onto the thread of the teammate's a card is raised in: its
    /// tape for the DM, else the thread's own stream.
    pub(crate) fn write_home(&self, persona_id: &str, home: &ThreadId, event: &TranscriptEvent) {
        if home.kind == ThreadKind::Dm {
            self.write(persona_id, event);
        } else {
            self.threads().write(home, persona_id, event);
        }
    }

    /// Cards that deliver and have outlived their lifetime without an
    /// answer: expired, and the teammate told, so neither the card nor the
    /// teammate's "waiting on you" stays up forever. A card from before
    /// cards carried `expiresAt` lives [`ASK_HOURS`] from when it was asked.
    pub(crate) fn expire_stale_asks(self: &Arc<Self>, now: i64) {
        self.expire_orphaned_credentials();
        for persona in crate::room::roster(&self.log) {
            for (event, home) in self.cards_of(&persona.id) {
                let Ok(TranscriptEvent::HumanAction {
                    ts,
                    action_id,
                    reason,
                    status: HumanActionStatus::Pending,
                    delivers: Some(true),
                    expires_at,
                    ..
                }) = serde_json::from_value::<TranscriptEvent>(event)
                else {
                    continue;
                };
                let expires_at = expires_at.unwrap_or(ts + i64::from(ASK_HOURS) * 60 * 60_000);
                if now < expires_at {
                    continue;
                }
                let _answering = lock(&self.answering_later);
                if !matches!(
                    self.human_card(&persona.id, &action_id),
                    Some((
                        TranscriptEvent::HumanAction {
                            status: HumanActionStatus::Pending,
                            ..
                        },
                        _
                    ))
                ) {
                    continue;
                }
                self.supersede_human(&persona.id, &action_id, HumanActionStatus::Expired, None);
                self.deliver_answer(
                    &persona.id,
                    &action_id,
                    &reason,
                    HumanActionStatus::Expired,
                    None,
                    home,
                );
            }
        }
    }

    /// The card this request wrote, read back off the tape it was written to.
    pub(super) fn permission_card(
        &self,
        persona_id: &str,
        request_id: &str,
    ) -> Option<TranscriptEvent> {
        let id = Value::from(format!("perm:{request_id}"));
        self.tape(persona_id)
            .into_iter()
            .find(|event| event.get("id") == Some(&id))
            .and_then(|event| serde_json::from_value(event).ok())
    }

    /// Supersedes every card that still claims to be live.
    ///
    /// A turn that has ended and a session that has stopped are the same fact
    /// for a permission: the agent is no longer waiting, so the button has
    /// nothing behind it and the transcript should not draw one.
    pub(super) fn settle_permissions(&self, persona_id: &str) {
        self.expire_open_cards(persona_id, |_| true);
    }

    /// As [`Self::settle_permissions`], but a first-contact card stays up: its
    /// wait is a colleague's request, not the turn's, and the person's
    /// Interrupt of a turn redirects it and answers no colleague.
    pub(super) fn settle_permissions_of_a_redirect(&self, persona_id: &str) {
        self.expire_open_cards(persona_id, |event| {
            !event["requestId"]
                .as_str()
                .is_some_and(|id| id.starts_with(peers::COLLAB_REQUEST_PREFIX))
        });
    }

    fn expire_open_cards(&self, persona_id: &str, expiring: impl Fn(&Value) -> bool) {
        let home_cards: Vec<_> = self
            .tape(persona_id)
            .into_iter()
            .filter(|event| event["from"]["kind"] != "call" && expiring(event))
            .collect();
        for expired in crate::log::expire_orphaned_permissions(&home_cards, now_ms()) {
            self.write_value(persona_id, &expired);
        }
    }

    pub(super) fn supersede_human(
        &self,
        persona_id: &str,
        action_id: &str,
        status: HumanActionStatus,
        note: Option<String>,
    ) {
        let Some((card, home)) = self.human_card(persona_id, action_id) else {
            return;
        };
        let TranscriptEvent::HumanAction {
            id,
            reason,
            question,
            delivers,
            expires_at,
            thread,
            ..
        } = card
        else {
            return;
        };
        lock(&self.voice_questions).remove(action_id);
        self.write_home(
            persona_id,
            &home,
            &TranscriptEvent::HumanAction {
                from: None,
                id,
                ts: now_ms(),
                action_id: action_id.to_string(),
                reason,
                question,
                status,
                note,
                delivers,
                expires_at,
                thread,
            },
        );
    }

    /// The card this request raised, and the thread it was raised in: the
    /// tape, or one of the teammate's work threads or jobs. Where the card is
    /// is where its answer goes.
    pub(super) fn human_card(
        &self,
        persona_id: &str,
        action_id: &str,
    ) -> Option<(TranscriptEvent, ThreadId)> {
        let id = Value::from(format!("human:{action_id}"));
        self.cards_of(persona_id)
            .into_iter()
            .find(|(event, _)| event.get("id") == Some(&id))
            .and_then(|(event, home)| Some((serde_json::from_value(event).ok()?, home)))
    }

    /// Every human-action card of the teammate's, with the thread it is on.
    pub(super) fn cards_of(&self, persona_id: &str) -> Vec<(Value, ThreadId)> {
        let dm = ThreadId::dm(persona_id);
        self.tape(persona_id)
            .into_iter()
            .map(|event| {
                let home = if event["from"]["kind"] == "call"
                    && let Some(key) = event["from"]["thread"].as_str()
                {
                    ThreadId {
                        kind: ThreadKind::Call,
                        key: key.to_string(),
                    }
                } else {
                    dm.clone()
                };
                (event, home)
            })
            .chain(
                self.cards_in_work(persona_id)
                    .into_iter()
                    .filter(|(event, _)| event["from"]["kind"] != "call"),
            )
            .chain(self.cards_in_jobs(persona_id))
            .collect()
    }
}

/// Authority held by the core, never reconstructed from card prose or saved flags.
pub(super) struct Question {
    pub capability: CapabilityLease,
    pub person_name: Option<String>,
}
