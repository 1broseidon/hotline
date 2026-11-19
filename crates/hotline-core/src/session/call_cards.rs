//! Voice never infers authority from a card's title or a model's choice.
//! Core-owned name cards and live collaboration waits carry bounded authority.

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
                .is_some_and(|question| question.capability.is_current())
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
                text: format!("{title} Say yes to allow it this once, or no to decline."),
                answerable: true,
            });
        }
        if card.kind == "cookie_ask" {
            let pending = self.tape(&card.persona).iter().any(|event| {
                event["kind"] == "cookie_ask"
                    && event["id"] == card.request
                    && event["status"] == "pending"
            });
            return pending.then(|| Notice {
                text: "A sign-in ask waits on the desk and phone. Please answer it there.".into(),
                answerable: false,
            });
        }
        if card.kind == "secret_use_ask" {
            let pending = self.tape(&card.persona).iter().any(|event| {
                event["kind"] == "secret_use_ask"
                    && event["useId"] == card.request
                    && event["status"] == "pending"
            });
            return pending.then(|| Notice {
                text: "A key-use approval waits on the desk and phone. Please answer it there."
                    .into(),
                answerable: false,
            });
        }
        if card.kind == "credential_ask" {
            let pending = self.tape(&card.persona).iter().any(|e| {
                e["kind"] == "credential_ask"
                    && e["actionId"] == card.request
                    && e["status"] == "pending"
            });
            return pending.then(|| Notice {
                text: "A credential card waits on the desk and phone. Please fill it there; never say the values on this call.".into(),
                answerable: false,
            });
        }
        if card.kind == "credential_stored" {
            let tape = self.tape(&card.persona);
            let event = tape.iter().find(|e| {
                e["kind"] == "credential_ask"
                    && e["actionId"] == card.request
                    && e["status"] == "stored"
            })?;
            return Some(Notice {
                text: format!("Stored as {}.", event["storedAs"].as_str()?),
                answerable: false,
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
                }) => match spoken_title(&title) {
                    short if short.is_empty() => String::new(),
                    short => format!(": {short}"),
                },
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
                .map(|question| question.capability.clone())
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
            if yes { peers::ALLOW } else { peers::DENY },
        )
        .await?;
        Ok(())
    }
}

/// Only cards carry a mirrored status, never arbitrary model output.
/// How many words of an approval's title a call says.
const SPOKEN_TITLE_WORDS: usize = 8;

/// An approval's title as a call says it. A driver titles its approval with
/// the tool's own line, a whole command or a path from the root, which a
/// voice reads out letter by letter. So a path is said as its last name, a
/// link as its host, flags and shell punctuation are left out, a command
/// stops at its first pipe or `&&`, and only the first few words are said:
/// the full line is on the card at the desk, which is where it is answered.
pub(super) fn spoken_title(title: &str) -> String {
    let line = title.lines().next().unwrap_or_default();
    let mut words: Vec<String> = Vec::new();
    let mut cut = false;
    for token in line.split_whitespace() {
        let token = token.trim_matches(|ch: char| matches!(ch, '`' | '"' | '\'' | '(' | ')' | ','));
        if matches!(token, "|" | "||" | "&&" | ";" | ">" | ">>" | "<") {
            cut = true;
            break;
        }
        if token.starts_with('-') || !token.chars().any(char::is_alphanumeric) {
            continue;
        }
        let word = if let Some((_, rest)) = token.split_once("://") {
            rest.split(['/', '?', '#']).next().unwrap_or_default()
        } else if token.contains(['/', '\\']) {
            token
                .split(['/', '\\'])
                .rfind(|part| !part.is_empty())
                .unwrap_or_default()
        } else {
            token
        };
        let word = word.trim_end_matches(['.', ':', ';']);
        if word.is_empty() {
            continue;
        }
        if words.len() == SPOKEN_TITLE_WORDS {
            cut = true;
            break;
        }
        words.push(word.to_string());
    }
    let said = words.join(" ");
    if cut && !said.is_empty() {
        format!("{said}, and more")
    } else {
        said
    }
}

pub(super) fn is_card(event: &Value) -> bool {
    matches!(
        event["kind"].as_str(),
        Some("permission" | "human_action" | "credential_ask" | "passkey_ask" | "cookie_ask")
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

#[cfg(test)]
mod tests {
    use super::spoken_title;

    #[test]
    fn a_call_says_an_approvals_title_short_with_no_path_flag_or_pipe_read_out() {
        assert_eq!(
            spoken_title("Read /Users/ada/Projects/hotline/crates/hotline-voice/src/turn.rs"),
            "Read turn.rs"
        );
        assert_eq!(
            spoken_title("Run `cargo test -p hotline-voice --lib -- --nocapture | tail -5`"),
            "Run cargo test hotline-voice, and more"
        );
        assert_eq!(
            spoken_title("Fetch https://www.nhl.com/devils/schedule?month=12"),
            "Fetch www.nhl.com"
        );
        assert_eq!(
            spoken_title("Edit C:\\Users\\ada\\notes\\plan.md\nsecond line"),
            "Edit plan.md"
        );
        assert_eq!(
            spoken_title("one two three four five six seven eight nine ten"),
            "one two three four five six seven eight, and more"
        );
        assert_eq!(spoken_title("Search the web."), "Search the web");
        assert_eq!(spoken_title(" -- | "), "");
    }
}
