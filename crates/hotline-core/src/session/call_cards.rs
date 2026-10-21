//! Voice never infers authority from a card's title or a model's choice.
//! Only the core's live collaboration wait may grant a session approval.

use super::*;
use hotline_voice::cards::{Card, Notice};

impl Room {
    pub(crate) fn call_card_notice(&self, card: &Card) -> Option<Notice> {
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
                text: format!("{} asks: {reason} Say yes or no.", persona.name),
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
                } => format!(": {reason}"),
                _ => return None,
            },
            "permission" => match self.permission_card(&card.persona, &card.request) {
                Some(TranscriptEvent::Permission {
                    title,
                    decision: None,
                    ..
                }) => format!(": {}", hotline_room::text::clip(&title, 500)),
                _ => String::new(),
            },
            _ => String::new(),
        };
        Some(Notice {
            text: format!(
                "{} needs you for {action}{detail}. Please handle it at the desk; it cannot be answered by voice.",
                persona.name
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
                    reason,
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
                Some(format!(
                    "From call {call}: {}. Informational answer only; no operation is authorized.",
                    if yes { "Yes" } else { "No" }
                )),
            )?;
            return self.record_call_answer(card, call, yes, &reason);
        }
        if card.kind != "permission" || !self.peers.voice_approval(&card.persona, &card.request) {
            return Err("This card needs the desk; voice cannot answer it.".into());
        }
        let Some(TranscriptEvent::Permission { title, .. }) =
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
        self.record_call_answer(card, call, yes, &title)
    }

    fn record_call_answer(
        &self,
        card: &Card,
        call: &str,
        yes: bool,
        summary: &str,
    ) -> Result<(), String> {
        self.try_write_value(
            &card.persona,
            &serde_json::json!({
                "kind": "notice", "id": format!("call-answer:{call}:{}", card.request),
                "ts": now_ms(), "level": "info",
                "text": format!("From call {call}: {} — {summary}",
                    if yes { "Yes" } else { "No" }),
            }),
        )
        .map_err(|error| format!("The answer's record could not be saved: {error}"))
    }
}
