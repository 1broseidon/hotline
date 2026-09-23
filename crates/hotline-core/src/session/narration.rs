//! How a turn's running commentary stays off the screen.
//!
//! An agent with a tool in hand says what it is about to do, does it, says
//! what it found, does the next thing — and every one of those sentences
//! reached the tape as chat, so a person on a phone watched five bubbles land
//! for one answer. The house style asks for silence during the work, and
//! asking works about as well here as it does for a quiet schedule (see
//! [`super::quiet`]): a model told not to narrate narrates its restraint.
//!
//! So nothing here asks. Once a turn has called a tool, an agent message is
//! held until the next thing the driver sends. If that is another tool call,
//! the message was narration and is written as a `thought`. Anything that
//! puts something in front of a person — the end of the turn, a permission
//! card, a notice, a chapter boundary, another message — makes it the report,
//! written as chat. The rule reads the *kind* of the next update and never
//! the words, so no phrasing changes the answer; the agent is never told, so
//! it has nothing to announce; and the words are not lost, only demoted, so
//! the transcript still shows the work under thinking.
//!
//! What is never held: the first message of a turn, before any tool has been
//! called. "On it" is the one line the house style asks for — the word that
//! says you were heard — and an answer that needs no tool is a message
//! nothing follows anyway.

use crate::driver::{MessageKind, Update};

/// The voice of one turn: what it has said, and the one line it is holding.
#[derive(Debug, Default)]
pub struct Voice {
    /// A tool has been called this turn.
    worked: bool,
    /// A message went out before any tool was called: the acknowledgement.
    spoken: bool,
    /// An agent message waiting to learn whether it was narration.
    held: Option<(String, String)>,
}

impl Voice {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a live agent delta should stream as thinking instead.
    ///
    /// After the acknowledgement, a message may turn out to be narration, and
    /// the window must not type a bubble that then vanishes; it shows thinking
    /// and the report lands whole.
    pub fn mutes_deltas(&self) -> bool {
        self.worked || self.spoken
    }

    /// One update in, the updates to write in its place, in order.
    pub fn step(&mut self, update: Update) -> Vec<Update> {
        match update {
            Update::Message {
                kind: MessageKind::Agent,
                id,
                text,
            } => {
                let mut out = self.release(MessageKind::Agent);
                if !self.worked && !self.spoken {
                    self.spoken = true;
                    out.push(Update::Message {
                        kind: MessageKind::Agent,
                        id,
                        text,
                    });
                } else {
                    self.held = Some((id, text));
                }
                out
            }
            update @ Update::ToolCall { .. } => {
                self.worked = true;
                let mut out = self.release(MessageKind::Thought);
                out.push(update);
                out
            }
            update @ Update::Turn { .. } => {
                let mut out = self.release(MessageKind::Agent);
                out.push(update);
                *self = Self::new();
                out
            }
            // A turn waiting on its jobs has said what it has to say for now:
            // the held line is the report, and whatever wakes it — a job's
            // result, the person — starts a new stretch of voice, whose first
            // words are heard as they come.
            update @ Update::Parked => {
                let mut out = self.release(MessageKind::Agent);
                out.push(update);
                *self = Self::new();
                out
            }
            // The agent has not spoken again: a held line keeps waiting.
            update @ (Update::ToolResult { .. }
            | Update::Delta { .. }
            | Update::Message {
                kind: MessageKind::Thought,
                ..
            }) => vec![update],
            // A card, a notice or a chapter boundary is about to be seen; what
            // was said before it is said.
            update => {
                let mut out = self.release(MessageKind::Agent);
                out.push(update);
                out
            }
        }
    }

    /// The turn ended without saying so — the driver stopped — and a held
    /// line is the last thing it said.
    pub fn finish(&mut self) -> Vec<Update> {
        let out = self.release(MessageKind::Agent);
        *self = Self::new();
        out
    }

    fn release(&mut self, kind: MessageKind) -> Vec<Update> {
        self.held
            .take()
            .map(|(id, text)| Update::Message { kind, id, text })
            .into_iter()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    //! The property worth pinning is negative: there is no message text that
    //! changes any answer here. The narration in these cases is the kind an
    //! agent actually produces, and the report says the same words.

    use super::*;

    fn says(id: &str, text: &str) -> Update {
        Update::Message {
            kind: MessageKind::Agent,
            id: id.into(),
            text: text.into(),
        }
    }

    fn calls(id: &str) -> Update {
        Update::ToolCall {
            call_id: id.into(),
            title: "ls".into(),
            kind: "ls".into(),
        }
    }

    fn returns(id: &str) -> Update {
        Update::ToolResult {
            call_id: id.into(),
            ok: true,
            output: String::new(),
            images: Vec::new(),
        }
    }

    fn turn() -> Update {
        Update::Turn {
            stop_reason: "end_turn".into(),
            usage: None,
        }
    }

    fn kinds(updates: &[Update]) -> Vec<String> {
        updates
            .iter()
            .map(|update| match update {
                Update::Message {
                    kind: MessageKind::Agent,
                    text,
                    ..
                } => format!("agent:{text}"),
                Update::Message {
                    kind: MessageKind::Thought,
                    text,
                    ..
                } => format!("thought:{text}"),
                Update::ToolCall { call_id, .. } => format!("call:{call_id}"),
                Update::ToolResult { call_id, .. } => format!("result:{call_id}"),
                Update::Turn { .. } => "turn".into(),
                other => format!("{other:?}"),
            })
            .collect()
    }

    fn run(voice: &mut Voice, updates: Vec<Update>) -> Vec<String> {
        let mut out = Vec::new();
        for update in updates {
            out.extend(voice.step(update));
        }
        kinds(&out)
    }

    #[test]
    fn the_acknowledgement_is_said_the_narration_is_thought_and_the_report_is_said() {
        let mut voice = Voice::new();
        let out = run(
            &mut voice,
            vec![
                says("m1", "on it"),
                calls("c1"),
                returns("c1"),
                says("m2", "Found the file, editing it now."),
                calls("c2"),
                returns("c2"),
                says("m3", "Running the tests."),
                calls("c3"),
                returns("c3"),
                says("m4", "Done, all green."),
                turn(),
            ],
        );
        assert_eq!(
            out,
            [
                "agent:on it",
                "call:c1",
                "result:c1",
                "thought:Found the file, editing it now.",
                "call:c2",
                "result:c2",
                "thought:Running the tests.",
                "call:c3",
                "result:c3",
                "agent:Done, all green.",
                "turn",
            ]
        );
    }

    #[test]
    fn the_same_words_are_chat_when_nothing_follows_them_and_thought_when_a_tool_does() {
        let line = "Done, all green.";
        let mut voice = Voice::new();
        let spoken = run(&mut voice, vec![calls("c1"), says("m1", line), turn()]);
        assert_eq!(spoken, ["call:c1", &format!("agent:{line}"), "turn"]);

        let mut voice = Voice::new();
        let demoted = run(
            &mut voice,
            vec![calls("c1"), says("m1", line), calls("c2"), turn()],
        );
        assert_eq!(
            demoted,
            ["call:c1", &format!("thought:{line}"), "call:c2", "turn"]
        );
    }

    #[test]
    fn an_answer_with_no_tool_is_said_at_once() {
        let mut voice = Voice::new();
        assert!(!voice.mutes_deltas());
        let out = run(&mut voice, vec![says("m1", "41, all .rs"), turn()]);
        assert_eq!(out, ["agent:41, all .rs", "turn"]);
    }

    #[test]
    fn only_one_line_is_said_before_the_work_and_deltas_are_thinking_after_it() {
        let mut voice = Voice::new();
        let out = run(&mut voice, vec![says("m1", "on it")]);
        assert_eq!(out, ["agent:on it"]);
        assert!(
            voice.mutes_deltas(),
            "after the acknowledgement the window shows thinking"
        );
        let out = run(
            &mut voice,
            vec![says("m2", "Let me look at the config first."), calls("c1")],
        );
        assert_eq!(out, ["thought:Let me look at the config first.", "call:c1"]);
    }

    #[test]
    fn a_held_line_waits_through_results_and_thoughts_and_is_said_before_a_card() {
        let mut voice = Voice::new();
        let out = run(
            &mut voice,
            vec![
                calls("c1"),
                says("m1", "I need to push this."),
                returns("c1"),
                Update::Message {
                    kind: MessageKind::Thought,
                    id: "t1".into(),
                    text: "checking".into(),
                },
            ],
        );
        assert_eq!(out, ["call:c1", "result:c1", "thought:checking"]);
        let out = run(
            &mut voice,
            vec![Update::Permission {
                request_id: "p1".into(),
                title: "Push to origin?".into(),
                options: Vec::new(),
            }],
        );
        assert_eq!(out[0], "agent:I need to push this.");
        assert!(out[1].starts_with("Permission"));
    }

    #[test]
    fn a_turn_waiting_on_its_jobs_says_its_last_line_and_the_next_stretch_is_heard() {
        let mut voice = Voice::new();
        let out = run(
            &mut voice,
            vec![
                says("m1", "on it"),
                calls("c1"),
                says("m2", "Handed that to a subagent; I'll report back."),
                Update::Parked,
            ],
        );
        assert_eq!(
            out,
            [
                "agent:on it",
                "call:c1",
                "agent:Handed that to a subagent; I'll report back.",
                "Parked",
            ]
        );
        assert!(
            !voice.mutes_deltas(),
            "an answer to the person while the job runs streams as speech"
        );
        let out = run(
            &mut voice,
            vec![says("m3", "Still running, about halfway."), Update::Parked],
        );
        assert_eq!(out, ["agent:Still running, about halfway.", "Parked"]);
        let out = run(
            &mut voice,
            vec![returns("c1"), says("m4", "It's done: three bugs."), turn()],
        );
        assert_eq!(out, ["result:c1", "agent:It's done: three bugs.", "turn"]);
    }

    #[test]
    fn a_driver_that_stops_mid_turn_still_says_the_last_line() {
        let mut voice = Voice::new();
        let out = run(&mut voice, vec![calls("c1"), says("m1", "Almost there.")]);
        assert_eq!(out, ["call:c1"]);
        assert_eq!(kinds(&voice.finish()), ["agent:Almost there."]);
        assert!(!voice.mutes_deltas(), "the next turn starts fresh");
    }

    #[test]
    fn a_turn_ends_the_hold_and_the_next_turn_may_acknowledge_again() {
        let mut voice = Voice::new();
        run(&mut voice, vec![says("m1", "on it"), calls("c1"), turn()]);
        let out = run(
            &mut voice,
            vec![says("m2", "sure"), calls("c2"), says("m3", "done"), turn()],
        );
        assert_eq!(out, ["agent:sure", "call:c2", "agent:done", "turn"]);
    }
}
