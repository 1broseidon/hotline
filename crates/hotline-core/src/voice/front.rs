//! The bridge between a call's voice front and the teammate it speaks for.
//!
//! The front is a realtime model that hears the person and talks to them. It
//! has two tools and no reach into the room: `send_to_teammate` hands the
//! person's words to the teammate, and `remain_silent` says the front has
//! nothing to add. The teammate does the work. [`Front::tool_call`] answers
//! the front's tool calls, and [`Front::next_note`] turns what the call then
//! says and asks of the person into [`Note`]s for the front to speak.
//!
//! The words go in by the path a typed message takes (the call's own text
//! input, so the teammate's session, grants and open chapter, and a second
//! send while its turn runs steers that turn), marked as said on the call.
//! What the front itself says is never sent anywhere: the teammate's tape
//! holds the person's words alone.

use super::Calls;
use crate::contract::{VoiceEvent, VoiceState};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::broadcast;

pub const SEND_TO_TEAMMATE: &str = "send_to_teammate";
pub const REMAIN_SILENT: &str = "remain_silent";

/// One function the front may call. `parameters` is its JSON Schema.
pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
}

/// The front's whole reach.
pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: SEND_TO_TEAMMATE,
            description: "Pass the person's own words to the teammate, exactly as they said them: do not rephrase, summarize or add to them. The teammate does the work; sending is not doing it.",
            parameters: json!({
                "type": "object",
                "properties": {"words": {"type": "string", "description": "What the person said, verbatim."}},
                "required": ["words"],
            }),
        },
        Tool {
            name: REMAIN_SILENT,
            description: "Say nothing this time: the person is not talking to you, or there is nothing to add.",
            parameters: json!({"type": "object", "properties": {}}),
        },
    ]
}

/// Something for the front to say to the person. The text is the core's, to
/// be said as given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// The next words of the teammate's `<spoken>` version of its reply.
    Spoken(String),
    /// The teammate's turn is still running.
    Progress(String),
    /// The teammate waits on the person at the desk or phone.
    NeedsYou(String),
}

impl Note {
    pub fn text(&self) -> &str {
        match self {
            Self::Spoken(text) | Self::Progress(text) | Self::NeedsYou(text) => text,
        }
    }

    /// The text to give a front model so that it says the note, word for word.
    pub fn instruction(&self) -> String {
        format!(
            "Say exactly this to the person, word for word, and add nothing: {}",
            self.text()
        )
    }
}

const PROGRESS_LINE: &str = "Still working on it.";

/// A pending request is not described: its details are the desk's to show.
fn needs_you(kind: &str) -> String {
    match kind {
        "permission" | "passkey_ask" => "It needs you to approve something on the desk or phone.",
        _ => "It needs you to do something on the desk or phone.",
    }
    .into()
}

pub struct Front {
    calls: Arc<Calls>,
    call_id: String,
    events: broadcast::Receiver<VoiceEvent>,
    /// The last thing sent, by sequence number, which must rise.
    seq: u32,
    /// How much of each said line has been given out already; a line is sent
    /// again, grown, as its reply streams.
    said: HashMap<String, usize>,
    /// Words are with the teammate and its turn has not ended, so a call that
    /// says it is still thinking is progress.
    waiting: bool,
    /// The call says it is thinking as soon as the words are handed over; the
    /// front already knows, so only the repeats are progress.
    first_thinking_unreported: bool,
}

impl Front {
    /// The front of a live call to a teammate that takes finalized text.
    pub fn open(calls: &Arc<Calls>, call_id: &str) -> Result<Self, String> {
        let (_, events) = calls.subscribe(call_id)?;
        Ok(Self {
            calls: calls.clone(),
            call_id: call_id.into(),
            events,
            seq: 0,
            said: HashMap::new(),
            waiting: false,
            first_thinking_unreported: false,
        })
    }

    /// Answers one tool call with the output the front is given back. A
    /// failure is an output too, worded so the front does not take the words
    /// for sent.
    pub fn tool_call(&mut self, name: &str, arguments: &str) -> String {
        match name {
            REMAIN_SILENT => String::new(),
            SEND_TO_TEAMMATE => match self.send(arguments) {
                Ok(()) => "Handed to the teammate. Nothing is done yet: do not say it is, and say nothing of the result until you are given it.".into(),
                Err(error) => format!("Not sent: {error}"),
            },
            other => format!("There is no tool called {other}."),
        }
    }

    fn send(&mut self, arguments: &str) -> Result<(), String> {
        let words = serde_json::from_str::<Value>(arguments)
            .ok()
            .and_then(|arguments| arguments["words"].as_str().map(str::to_string))
            .ok_or("send_to_teammate needs the person's words.")?;
        self.calls.relay(&self.call_id, self.seq + 1, &words)?;
        self.seq += 1;
        self.waiting = true;
        self.first_thinking_unreported = true;
        Ok(())
    }

    /// The next thing to tell the person, or `None` once the call has ended.
    pub async fn next_note(&mut self) -> Option<Note> {
        loop {
            let event = match self.events.recv().await {
                Ok(event) => event,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            };
            match event {
                VoiceEvent::Said { id, text } => {
                    let given = self.said.entry(id).or_insert(0);
                    let fresh = text.get(*given..).unwrap_or_default().trim();
                    if !fresh.is_empty() {
                        let note = Note::Spoken(fresh.to_string());
                        *given = text.len();
                        return Some(note);
                    }
                }
                VoiceEvent::State {
                    state: VoiceState::Ended,
                    ..
                } => return None,
                VoiceEvent::State {
                    state: VoiceState::Thinking,
                    listening: true,
                    ..
                } => {
                    if std::mem::take(&mut self.first_thinking_unreported) || !self.waiting {
                        continue;
                    }
                    return Some(Note::Progress(PROGRESS_LINE.into()));
                }
                VoiceEvent::State {
                    state: VoiceState::Listening,
                    ..
                } => self.waiting = false,
                VoiceEvent::Card { kind, .. } => return Some(Note::NeedsYou(needs_you(&kind))),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_front_has_two_tools_and_neither_reaches_the_room() {
        let names: Vec<_> = tools().iter().map(|tool| tool.name).collect();
        assert_eq!(names, [SEND_TO_TEAMMATE, REMAIN_SILENT]);
    }

    #[test]
    fn a_pending_request_is_told_as_needing_the_person_not_described() {
        assert!(needs_you("permission").contains("approve"));
        assert!(needs_you("human_action").contains("desk or phone"));
    }

    #[test]
    fn a_note_is_given_to_the_front_to_say_as_it_is() {
        let note = Note::Spoken("The checks passed.".into());
        assert!(note.instruction().ends_with("The checks passed."));
    }
}
