//! Voice never infers authority from a card's title or a model's choice.
//! Only the core's live collaboration wait may grant a session approval.

use super::*;
use hotline_voice::cards::{Card, Notice};

impl Room {
    pub(crate) fn call_card_notice(&self, card: &Card, first_person: bool) -> Option<Notice> {
        let persona = self.persona(&card.persona).ok()?;
        if card.kind == "human_action"
            && let Some((
                TranscriptEvent::HumanAction {
                    reason,
                    question: Some(true),
                    status: HumanActionStatus::Pending,
                    expires_at,
                    ..
                },
                _,
            )) = self.human_card(&card.persona, &card.request)
            && expires_at.is_none_or(|expires| expires > now_ms())
            && lock(&self.voice_questions)
                .get(&card.request)
                .is_some_and(CapabilityLease::is_current)
        {
            return Some(Notice {
                text: if first_person {
                    format!("A question for you: {reason} Say yes or no.")
                } else {
                    format!("{} asks: {reason} Say yes or no.", persona.name)
                },
                answerable: true,
            });
        }
        if card.kind == "permission" && self.peers.voice_approval(&card.persona, &card.request) {
            let TranscriptEvent::Permission {
                title,
                decision: None,
                ..
            } = self.permission_card(&card.persona, &card.request)?
            else {
                return None;
            };
            return Some(Notice {
                text: format!("{title} Say yes to allow this session, or no to decline."),
                answerable: true,
            });
        }
        // Opaque driver approvals, action requests and passkeys cannot be
        // promoted by names, options or an argument supplied by a model.
        let action = match card.kind.as_str() {
            "permission" => "an approval",
            "human_action" => "an action",
            "passkey_ask" => "a passkey request",
            _ => return None,
        };
        let detail = match card.kind.as_str() {
            "human_action" => match self.human_card(&card.persona, &card.request)?.0 {
                TranscriptEvent::HumanAction {
                    reason,
                    status: HumanActionStatus::Pending,
                    ..
                } => format!(": {}", reason.trim_end_matches(['.', ' '])),
                _ => return None,
            },
            "permission" => match self.permission_card(&card.persona, &card.request) {
                Some(TranscriptEvent::Permission {
                    title,
                    decision: None,
                    ..
                }) => format!(
                    ": {}",
                    hotline_room::text::clip(&title, 500).trim_end_matches(['.', ' '])
                ),
                _ => String::new(),
            },
            _ => String::new(),
        };
        Some(Notice {
            text: format!(
                "{} for {action}{detail}. Please handle it at the desk; it cannot be answered by voice.",
                if first_person {
                    "I need you".to_string()
                } else {
                    format!("{} needs you", persona.name)
                }
            ),
            answerable: false,
        })
    }

    pub(crate) async fn answer_call_card(
        self: &Arc<Self>,
        card: &Card,
        call: &str,
        yes: bool,
    ) -> Result<(), String> {
        let calls = lock(&self.voice).upgrade().ok_or("The call has ended.")?;
        calls.check_card_call(call, &card.persona)?;
        if card.kind == "human_action" {
            let Some((
                TranscriptEvent::HumanAction {
                    question: Some(true),
                    status: HumanActionStatus::Pending,
                    expires_at,
                    ..
                },
                _,
            )) = self.human_card(&card.persona, &card.request)
            else {
                return Err("An action card needs the desk.".into());
            };
            if expires_at.is_some_and(|expires| expires <= now_ms()) {
                return Err("That question has expired.".into());
            }
            let capability = lock(&self.voice_questions)
                .get(&card.request)
                .cloned()
                .ok_or("This question no longer has a live session.")?;
            capability.check()?;
            self.answer_human(
                &card.persona,
                &card.request,
                if yes {
                    HumanAnswer::Done
                } else {
                    HumanAnswer::Declined
                },
                // The settled card in the home chat is the record. The person
                // reads this note, and the agent's delivery carries only the
                // note, so it must say which answer was given.
                Some(format!(
                    "Answered by voice on the call: {}.",
                    if yes { "yes" } else { "no" }
                )),
            )?;
            return Ok(());
        }
        if card.kind != "permission" || !self.peers.voice_approval(&card.persona, &card.request) {
            return Err("This card needs the desk; voice cannot answer it.".into());
        }
        let Some(TranscriptEvent::Permission { .. }) =
            self.permission_card(&card.persona, &card.request)
        else {
            return Err("The approval card is no longer available.".into());
        };
        self.answer_permission(
            &card.persona,
            &card.request,
            if yes {
                peers::ALLOW_SESSION
            } else {
                peers::DENY
            },
        )
        .await?;
        Ok(())
    }
}

/// Only cards carry a mirrored status, never arbitrary model output.
pub(super) fn is_card(event: &Value) -> bool {
    matches!(
        event["kind"].as_str(),
        Some("permission" | "human_action" | "passkey_ask")
    )
}

impl Room {
    /// Preserve the trusted origin when a typed handler supersedes a card.
    pub(super) fn card_with_origin<'a>(
        &self,
        persona: &str,
        event: &'a Value,
    ) -> std::borrow::Cow<'a, Value> {
        if is_card(event)
            && event["from"].is_null()
            && let Some(old) = self
                .tape(persona)
                .into_iter()
                .find(|old| old["id"] == event["id"])
            && !old["from"].is_null()
        {
            let mut stamped = event.clone();
            stamped["from"] = old["from"].clone();
            return std::borrow::Cow::Owned(stamped);
        }
        std::borrow::Cow::Borrowed(event)
    }
}
