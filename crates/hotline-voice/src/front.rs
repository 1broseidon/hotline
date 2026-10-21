//! Live conversation and work on the call’s own thread.

use super::Calls;
use super::front_tools::FrontTools;
use hotline_contract::{VoiceEvent, VoiceState};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::broadcast;

use super::realtime::WORK_ON_IT;
pub const REMAIN_SILENT: &str = "remain_silent";

/// One function the front may call. `parameters` is its JSON Schema.
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// The front's whole reach.
pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: WORK_ON_IT.into(),
            description: "I write, change or run what was asked on this call. I keep working while we talk. Further instructions change my current work; stop cancels it. Use the whole call as context when asked to write that down.".into(),
            parameters: json!({
                "type": "object",
                "properties": {"words": {"type": "string", "description": "The task or change, in the person’s words."}, "action": {"type": "string", "enum": ["work", "stop"]}},
                "required": ["words"], "additionalProperties": false,
            }),
        },
        Tool {
            name: REMAIN_SILENT.into(),
            description: "Say nothing this time: the person is not talking to you, or there is nothing to add.".into(),
            parameters: json!({"type": "object", "properties": {}}),
        },
    ]
}

use super::realtime::SAY_THIS;

/// How many recent spoken results are checked for accidental echoes.
const NOTES_KEPT: usize = 8;
/// The shortest text an echo is looked for in.
const ECHO_CHARS: usize = 16;

/// The call's own lines that the front said, as against the teammate's.
pub const FRONT_LINE_PREFIX: &str = "front-";

/// Who the front speaks as, which words its fixed lines: on a direct call it
/// is the teammate, in the first person; on a desk call it speaks for the
/// desk and the teammates behind it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speaks {
    AsTeammate,
    ForDesk,
}

/// Admission is not completion. The front already acknowledged the request.
fn accepted(speaks: Speaks) -> &'static str {
    match speaks {
        Speaks::AsTeammate => {
            "Work accepted on this call. Nothing is done yet: say nothing of the result until you are given it, and do not acknowledge it again."
        }
        Speaks::ForDesk => {
            "Work accepted. Nothing is done yet: say nothing of the result until you are given it, and do not acknowledge it again."
        }
    }
}

pub struct Front {
    calls: Arc<Calls>,
    call_id: String,
    speaks: Speaks,
    events: broadcast::Receiver<VoiceEvent>,
    /// The last thing sent, by sequence number, which must rise.
    seq: u32,
    /// How much of each said line has been given out already; a line is sent
    /// again, grown, as its reply streams.
    said: HashMap<String, usize>,
    /// The latest lines given to the front to say, newest last. A front that
    /// relays one back would have the teammate answer its own reply, and the
    /// front say that, round and round.
    notes: std::collections::VecDeque<String>,
    /// The teammate's desk-side tools, on a direct call.
    desk_tools: Option<Arc<FrontTools>>,
    dictation_open: bool,
}

impl Front {
    /// The front of a live call to a teammate that takes finalized text.
    pub fn open(calls: &Arc<Calls>, call_id: &str, speaks: Speaks) -> Result<Self, String> {
        let (_, events) = calls.subscribe(call_id)?;
        Ok(Self {
            calls: calls.clone(),
            call_id: call_id.into(),
            speaks,
            events,
            seq: 0,
            said: HashMap::new(),
            notes: std::collections::VecDeque::new(),
            desk_tools: None,
            dictation_open: false,
        })
    }

    pub fn with_desk_tools(mut self, tools: Option<Arc<FrontTools>>) -> Self {
        self.desk_tools = tools;
        self
    }

    /// The desk-side tools a call to a teammate holds, which run beside the
    /// call rather than through [`Front::tool_call`].
    pub fn desk_tools(&self) -> Option<Arc<FrontTools>> {
        self.desk_tools.clone()
    }

    /// Provider turn boundaries are not sentence boundaries. Only caller words
    /// can close an unfinished dictation, never the front's invented completion.
    pub(crate) fn heard(&mut self, text: &str) {
        self.dictation_open = unfinished_dictation(text, self.dictation_open);
    }

    /// Answers one tool call with the output the front is given back, and
    /// whether it should speak after it. A failure is an output too, worded so
    /// the front does not take the words for sent, and is spoken to. A send
    /// is not: the front acknowledged it as it made it, and a second word
    /// would only say it again.
    pub async fn tool_call(&mut self, name: &str, arguments: &str) -> (String, bool) {
        let tool_id = uuid::Uuid::new_v4().to_string();
        self.calls
            .record_tool(&self.call_id, &tool_id, name, arguments, None);
        let result = match name {
            REMAIN_SILENT => (String::new(), false),
            WORK_ON_IT if self.dictation_open && !stops_work(arguments) => (
                "Work has not started: the caller's dictation is unfinished. Remain silent and wait for them to finish; then call work_on_it with the complete request. Do not invent the missing words.".into(),
                false,
            ),
            WORK_ON_IT => match self.work(arguments).await {
                Ok(Some(stopped)) => (stopped, true),
                Ok(None) => (accepted(self.speaks).into(), false),
                Err(error) => (format!("Work could not start: {error}"), true),
            },
            other => (format!("There is no tool called {other}."), true),
        };
        self.calls
            .record_tool(&self.call_id, &tool_id, name, arguments, Some(&result.0));
        result
    }

    async fn work(&mut self, arguments: &str) -> Result<Option<String>, String> {
        let arguments: Value =
            serde_json::from_str(arguments).map_err(|_| "work_on_it needs a JSON object.")?;
        let words = arguments["words"]
            .as_str()
            .ok_or("work_on_it needs the person's words.")?;
        if self.echoes_a_note(words) {
            return Err("those are words you were given to say, not the person's. Say nothing more about them.".into());
        }
        let stop = match arguments.get("action").and_then(Value::as_str) {
            None | Some("work") => false,
            Some("stop") => true,
            Some(_) => return Err("work_on_it action must be work or stop.".into()),
        };
        if stop {
            self.dictation_open = false;
            let stopped = self.calls.stop_work(&self.call_id)?;
            return Ok(Some(stopped));
        }
        if let Some(tools) = &self.desk_tools {
            tools.check()?;
        }
        self.calls
            .work_on_it(&self.call_id, self.seq + 1, words)
            .await?;
        self.seq += 1;
        Ok(None)
    }

    /// The next thing to tell the person, or `None` once the call has ended.
    pub async fn next_note(&mut self) -> Option<String> {
        loop {
            let event = match self.events.recv().await {
                Ok(event) => event,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            };
            match event {
                VoiceEvent::Said { id, .. } if id.starts_with(FRONT_LINE_PREFIX) => {}
                VoiceEvent::Said { id, text } => {
                    let given = self.said.entry(id).or_insert(0);
                    let fresh = text.get(*given..).unwrap_or_default().trim();
                    if !fresh.is_empty() {
                        let note = fresh.to_string();
                        *given = text.len();
                        return self.told(note);
                    }
                }
                VoiceEvent::State {
                    state: VoiceState::Ended,
                    ..
                } => return None,
                _ => {}
            }
        }
    }

    fn echoes_a_note(&self, words: &str) -> bool {
        let words = words.trim();
        // Short pieces are left alone: the person may well say "yes" or
        // "the tests" that a line also held.
        let long = |text: &str| text.chars().count() >= ECHO_CHARS;
        words.contains(SAY_THIS.trim())
            || self.notes.iter().any(|note| {
                (long(note) && words.contains(note.as_str()))
                    || (long(words) && note.contains(words))
            })
    }

    /// Keeps a note for the echo check, then hands it out.
    fn told(&mut self, note: String) -> Option<String> {
        if self.notes.len() == NOTES_KEPT {
            self.notes.pop_front();
        }
        self.notes.push_back(note.trim().to_string());
        Some(note)
    }
}

pub(crate) fn stops_work(arguments: &str) -> bool {
    serde_json::from_str::<Value>(arguments).is_ok_and(|arguments| arguments["action"] == "stop")
}

fn unfinished_dictation(text: &str, already_open: bool) -> bool {
    let words: Vec<_> = text
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect();
    if words.is_empty() {
        return already_open;
    }
    let dictating = already_open
        || words
            .iter()
            .any(|word| matches!(word.as_str(), "draft" | "say" | "write"));
    // Punctuation from STT is not evidence of completion. These dangling
    // words need their object or clause even when STT adds a full stop.
    dictating
        && words.last().is_some_and(|word| {
            matches!(
                word.as_str(),
                "draft"
                    | "say"
                    | "write"
                    | "saying"
                    | "that"
                    | "the"
                    | "a"
                    | "an"
                    | "to"
                    | "for"
                    | "with"
                    | "about"
                    | "and"
                    | "or"
                    | "but"
                    | "because"
                    | "if"
                    | "when"
                    | "which"
                    | "whose"
                    | "should"
                    | "could"
                    | "would"
                    | "will"
                    | "is"
                    | "are"
                    | "was"
                    | "be"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfinished_dictation_uses_caller_words_not_stt_punctuation() {
        for text in [
            "Draft a message saying that.",
            "Please write",
            "Say that the",
            "Write a note about",
        ] {
            assert!(unfinished_dictation(text, false), "{text}");
        }
        assert!(unfinished_dictation("the fog horn should", true));
        for text in [
            "Draft a harbour note.",
            "Write that down.",
            "Say hello.",
            "What is that?",
        ] {
            assert!(!unfinished_dictation(text, false), "{text}");
        }
        assert!(!unfinished_dictation(
            "the fog horn should be tested on Friday.",
            true
        ));
        assert!(!unfinished_dictation("Never mind, drop it.", true));
    }

    #[test]
    fn the_front_has_two_tools_and_neither_reaches_the_room() {
        let names: Vec<_> = tools().iter().map(|tool| tool.name.clone()).collect();
        assert_eq!(names, [WORK_ON_IT, REMAIN_SILENT]);
    }

    #[test]
    fn a_direct_call_speaks_its_fixed_lines_as_the_teammate() {
        let direct = accepted(Speaks::AsTeammate);
        assert!(direct.contains("do not acknowledge it again"));
        assert!(direct.contains("Nothing is done yet"));
        assert!(!direct.contains("Handed to"));
        assert!(accepted(Speaks::ForDesk).contains("Nothing is done yet"));
    }
}
