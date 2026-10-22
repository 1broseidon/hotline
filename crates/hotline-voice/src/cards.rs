//! Cards wait for a gap in the conversation. Only finalized caller speech
//! can answer the one card just announced; the model has no answer tool.

use crate::{Calls, lock};
use hotline_contract::{VoiceEvent, VoiceState};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Card {
    pub persona: String,
    pub kind: String,
    pub request: String,
}

pub struct Notice {
    pub text: String,
    pub answerable: bool,
}

#[derive(Default)]
pub(crate) struct Queue {
    pending: VecDeque<Card>,
    seen: HashMap<Card, bool>,
    pub active: Option<Card>,
    pub answered: bool,
    awaiting_answer: bool,
}

impl Calls {
    pub fn check_card_call(&self, id: &str, persona: &str) -> Result<(), String> {
        self.change(id, |call| {
            if call.state == VoiceState::Held
                || call.reconnecting
                || call.connection.as_ref().is_some_and(|c| c.dropped)
                || !(call.desk || call.target.as_deref() == Some(persona))
            {
                return Err("This call cannot answer that teammate's card.".into());
            }
            Ok(())
        })
    }
    pub fn card(&self, persona: &str, event: &Value) {
        let kind = event["kind"].as_str().unwrap_or_default();
        let (key, pending) = match kind {
            "permission" => ("requestId", event["decision"].is_null()),
            "human_action" | "credential_ask" => ("actionId", event["status"] == "pending"),
            "secret_use_ask" => ("useId", event["status"] == "pending"),
            "passkey_ask" => ("askId", event["status"] == "pending"),
            _ => return,
        };
        let Some(request) = event[key].as_str() else {
            return;
        };
        let card = Card {
            persona: persona.into(),
            kind: kind.into(),
            request: request.into(),
        };
        for call in lock(&self.calls).iter_mut().filter(|call| {
            call.state != VoiceState::Ended
                && (call.desk || call.target.as_deref() == Some(persona))
        }) {
            if !pending {
                call.cards.seen.insert(card.clone(), false);
                let _ = call.events.send(VoiceEvent::Card {
                    persona_id: persona.into(),
                    request_id: request.into(),
                    kind: kind.into(),
                    resolved: Some(true),
                });
                call.cards.pending.retain(|queued| queued != &card);
                if call.cards.active.as_ref() == Some(&card) {
                    call.cards.active = None;
                }
                if kind == "credential_ask" && event["status"] == "stored" {
                    // Completion is a second, value-free notice, not another
                    // answerable card. Its own key deduplicates both seats.
                    let stored = Card {
                        kind: "credential_stored".into(),
                        ..card.clone()
                    };
                    if let std::collections::hash_map::Entry::Vacant(entry) =
                        call.cards.seen.entry(stored.clone())
                    {
                        entry.insert(true);
                        call.cards.pending.push_back(stored);
                    }
                }
            } else if let std::collections::hash_map::Entry::Vacant(entry) =
                call.cards.seen.entry(card.clone())
            {
                entry.insert(true);
                call.cards.pending.push_back(card.clone());
                let _ = call.events.send(VoiceEvent::Card {
                    persona_id: persona.into(),
                    request_id: request.into(),
                    kind: kind.into(),
                    resolved: None,
                });
            }
        }
    }

    /// Used by notification routing: a held or disconnected call is not a
    /// substitute for waking the person at the desk.
    pub fn carries_cards(&self, persona: &str) -> bool {
        lock(&self.calls).iter().any(|call| {
            !matches!(call.state, VoiceState::Ended | VoiceState::Held)
                && !call.reconnecting
                && !call.connection.as_ref().is_some_and(|c| c.dropped)
                && (call.desk || call.target.as_deref() == Some(persona))
        })
    }

    pub(crate) fn next_card(&self, id: &str) -> Option<String> {
        loop {
            let card = self
                .change(id, |call| {
                    if call.caller_speaking
                        || call.input.is_some()
                        || call.utterance_pending
                        || matches!(call.state, VoiceState::Held | VoiceState::Speaking)
                        || call.reconnecting
                        || call.connection.as_ref().is_some_and(|c| c.dropped)
                        || call.front.as_ref().is_some_and(|front| front.responding)
                        || (call.cards.active.is_some() && call.cards.awaiting_answer)
                    {
                        return Ok(None);
                    }
                    Ok(call.cards.pending.pop_front().map(|card| {
                        let first_person =
                            !call.desk && call.target.as_deref() == Some(card.persona.as_str());
                        (card, first_person)
                    }))
                })
                .ok()??;
            let (card, first_person) = card;
            let Some(notice) = self.room.card_notice(&card, first_person) else {
                continue;
            };
            if self.change(id, |call| {
                if call.cards.seen.get(&card) != Some(&true) {
                    return Ok(false);
                }
                call.cards.active = Some(card);
                call.cards.awaiting_answer = notice.answerable;
                if let Some(front) = &mut call.front {
                    front.responding = true;
                }
                Ok(true)
            }) != Ok(true)
            {
                continue;
            }
            return Some(notice.text);
        }
    }

    /// Returns a spoken acknowledgement when a card consumed this input.
    /// Neither a model tool nor a client-supplied card id enters this path.
    pub(crate) async fn answer_card_words(&self, id: &str, words: &str) -> Option<String> {
        let yes = match words
            .trim()
            .trim_end_matches(['.', '!', '?'])
            .to_ascii_lowercase()
            .as_str()
        {
            "yes" => true,
            "no" => false,
            _ => {
                let _ = self.change(id, |call| {
                    call.cards.active = None;
                    Ok(())
                });
                return None;
            }
        };
        let card = self
            .change(id, |call| {
                if call.state == VoiceState::Held {
                    return Ok(None);
                }
                let card = call.cards.active.take();
                call.cards.answered = card.is_some();
                Ok(card)
            })
            .ok()??;
        Some(match self.room.answer_card(&card, id, yes).await {
            Ok(()) => if yes {
                "Yes, recorded from this call."
            } else {
                "No, recorded from this call."
            }
            .into(),
            Err(_) => {
                "That card could not be answered on the call. Please check it at the desk.".into()
            }
        })
    }
}
