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
use super::front_tools::FrontTools;
use crate::contract::{VoiceEvent, VoiceState};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

pub const SEND_TO_TEAMMATE: &str = "send_to_teammate";
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
            name: SEND_TO_TEAMMATE.into(),
            description: "Pass the person's own words to the teammate, exactly as they said them: do not rephrase, summarize or add to them. The teammate does the work; sending is not doing it.".into(),
            parameters: json!({
                "type": "object",
                "properties": {"words": {"type": "string", "description": "What the person said, verbatim."}},
                "required": ["words"],
            }),
        },
        Tool {
            name: REMAIN_SILENT.into(),
            description: "Say nothing this time: the person is not talking to you, or there is nothing to add.".into(),
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
}

use super::realtime::SAY_THIS;

/// How many of the latest notes a relay is checked against.
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

/// Taken in turn, so a long piece of work does not repeat one line.
const TEAMMATE_PROGRESS: [&str; 4] = [
    "Still on it.",
    "Still digging, bear with me.",
    "Still working on it, one moment.",
    "Bear with me, still going.",
];
const DESK_PROGRESS: [&str; 3] = [
    "Still working on it.",
    "Still waiting on that.",
    "Still going, bear with me.",
];
/// A call says it is still thinking every 15 seconds, and again whenever the
/// front stops talking; the person hears it at most this often.
const PROGRESS_EVERY: Duration = Duration::from_secs(45);
/// Each progress line waits twice as long as the last, up to this.
const PROGRESS_MOST: Duration = Duration::from_secs(180);

/// A pending request is not described: its details are the desk's to show.
fn needs_you(speaks: Speaks, kind: &str) -> String {
    let approve = matches!(kind, "permission" | "passkey_ask");
    match (speaks, approve) {
        (Speaks::AsTeammate, true) => "I need you to approve something on the desk or your phone.",
        (Speaks::AsTeammate, false) => "I need you to do something on the desk or your phone.",
        (Speaks::ForDesk, true) => {
            "The teammate needs you to approve something on the desk or phone."
        }
        (Speaks::ForDesk, false) => "The teammate needs you to do something on the desk or phone.",
    }
    .into()
}

/// What the front hears back once the person's words are handed over, with
/// no reply asked for: it acknowledged them as it sent them.
fn handed(speaks: Speaks) -> &'static str {
    match speaks {
        Speaks::AsTeammate => {
            "Sent to your working session. Nothing is done yet: say nothing of the result until you are given it, and do not acknowledge it again."
        }
        Speaks::ForDesk => {
            "Handed to the desk. Nothing is done yet: say nothing of the result until you are given it, and do not acknowledge it again."
        }
    }
}

pub struct Front {
    calls: Arc<Calls>,
    call_id: String,
    speaks: Speaks,
    /// Progress lines said so far, to take the next in turn.
    progressed: usize,
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
    /// When the person last heard something of the teammate's turn.
    last_told: Instant,
    /// The latest lines given to the front to say, newest last. A front that
    /// relays one back would have the teammate answer its own reply, and the
    /// front say that, round and round.
    notes: std::collections::VecDeque<String>,
    /// The teammate's desk-side tools, on a direct call.
    desk_tools: Option<Arc<FrontTools>>,
}

impl Front {
    /// The front of a live call to a teammate that takes finalized text.
    pub fn open(calls: &Arc<Calls>, call_id: &str, speaks: Speaks) -> Result<Self, String> {
        let (_, events) = calls.subscribe(call_id)?;
        Ok(Self {
            calls: calls.clone(),
            call_id: call_id.into(),
            speaks,
            progressed: 0,
            events,
            seq: 0,
            said: HashMap::new(),
            waiting: false,
            first_thinking_unreported: false,
            last_told: Instant::now(),
            notes: std::collections::VecDeque::new(),
            desk_tools: None,
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

    /// The sequence number the next line handed to the teammate takes.
    pub fn next_seq(&self) -> u32 {
        self.seq + 1
    }

    /// Answers one tool call with the output the front is given back, and
    /// whether it should speak after it. A failure is an output too, worded so
    /// the front does not take the words for sent, and is spoken to. A send
    /// is not: the front acknowledged it as it made it, and a second word
    /// would only say it again.
    pub fn tool_call(&mut self, name: &str, arguments: &str) -> (String, bool) {
        match name {
            REMAIN_SILENT => (String::new(), false),
            SEND_TO_TEAMMATE => match self.send(arguments) {
                Ok(()) => (handed(self.speaks).into(), false),
                Err(error) => (format!("Not sent: {error}"), true),
            },
            other => (format!("There is no tool called {other}."), true),
        }
    }

    fn send(&mut self, arguments: &str) -> Result<(), String> {
        let words = serde_json::from_str::<Value>(arguments)
            .ok()
            .and_then(|arguments| arguments["words"].as_str().map(str::to_string))
            .ok_or("send_to_teammate needs the person's words.")?;
        if self.echoes_a_note(&words) {
            return Err("those are words you were given to say, not the person's. Say nothing more about them.".into());
        }
        self.calls.relay(&self.call_id, self.seq + 1, &words)?;
        self.seq += 1;
        self.waiting = true;
        self.first_thinking_unreported = true;
        self.last_told = Instant::now();
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
                VoiceEvent::Said { id, .. } if id.starts_with(FRONT_LINE_PREFIX) => {}
                VoiceEvent::Said { id, text } => {
                    let given = self.said.entry(id).or_insert(0);
                    let fresh = text.get(*given..).unwrap_or_default().trim();
                    if !fresh.is_empty() {
                        let note = Note::Spoken(fresh.to_string());
                        *given = text.len();
                        self.last_told = Instant::now();
                        return self.told(note);
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
                    if std::mem::take(&mut self.first_thinking_unreported)
                        || !self.waiting
                        || self.last_told.elapsed() < self.progress_wait()
                    {
                        continue;
                    }
                    self.last_told = Instant::now();
                    let line = self.progress_line();
                    return self.told(Note::Progress(line.into()));
                }
                VoiceEvent::State {
                    state: VoiceState::Listening,
                    ..
                } => self.waiting = false,
                VoiceEvent::Card { kind, .. } => {
                    let line = needs_you(self.speaks, &kind);
                    return self.told(Note::NeedsYou(line));
                }
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
    fn told(&mut self, note: Note) -> Option<Note> {
        if self.notes.len() == NOTES_KEPT {
            self.notes.pop_front();
        }
        self.notes.push_back(note.text().trim().to_string());
        Some(note)
    }

    /// 45 s before the first progress line, then 90 s, then every 3 min.
    fn progress_wait(&self) -> Duration {
        PROGRESS_EVERY
            .saturating_mul(1 << self.progressed.min(4))
            .min(PROGRESS_MOST)
    }

    fn progress_line(&mut self) -> &'static str {
        let lines: &[&str] = match self.speaks {
            Speaks::AsTeammate => &TEAMMATE_PROGRESS,
            Speaks::ForDesk => &DESK_PROGRESS,
        };
        let line = lines[self.progressed % lines.len()];
        self.progressed += 1;
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_front_has_two_tools_and_neither_reaches_the_room() {
        let names: Vec<_> = tools().iter().map(|tool| tool.name.clone()).collect();
        assert_eq!(names, [SEND_TO_TEAMMATE, REMAIN_SILENT]);
    }

    #[test]
    fn a_pending_request_is_told_as_needing_the_person_not_described() {
        assert!(needs_you(Speaks::ForDesk, "permission").contains("approve"));
        assert!(needs_you(Speaks::ForDesk, "human_action").contains("desk or phone"));
    }

    #[test]
    fn a_direct_call_speaks_its_fixed_lines_as_the_teammate() {
        assert!(needs_you(Speaks::AsTeammate, "permission").starts_with("I need you to approve"));
        assert!(needs_you(Speaks::ForDesk, "permission").starts_with("The teammate needs you"));
        let direct = handed(Speaks::AsTeammate);
        assert!(direct.contains("do not acknowledge it again"));
        assert!(direct.contains("Nothing is done yet"));
        assert!(!direct.contains("Handed to"));
        assert!(handed(Speaks::ForDesk).contains("Nothing is done yet"));
    }
}
