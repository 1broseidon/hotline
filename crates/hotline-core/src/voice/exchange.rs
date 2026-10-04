//! What was said on one direct call: the person's words, the voice's replies,
//! and what the teammate's session reported back and the voice relayed.
//!
//! The call remembers this for itself. The teammate's tape only holds what was
//! handed to its session, so without it the voice would forget the sentence
//! before, and the session would never learn what was said on the way.
//!
//! The call's thread keeps the whole conversation (`calls/<id>.jsonl`, see
//! [`super::record`]). This is the part of it the voice is given: the newest
//! lines under the caps below, which [`Exchange::from_thread`] rebuilds from
//! the thread when a call is picked up again.

use serde_json::Value;
use std::collections::VecDeque;

/// The most lines kept; the oldest go first.
const MAX_LINES: usize = 30;
/// The most characters one line keeps.
const MAX_LINE_CHARS: usize = 800;
/// The most characters all lines keep together.
const MAX_CHARS: usize = 6_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speaker {
    /// The person on the call.
    Person,
    /// The voice, answering for itself.
    Voice,
    /// The voice, saying what the teammate's session had just reported. The
    /// session already knows these words.
    Relayed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub speaker: Speaker,
    pub text: String,
}

/// A call's recent lines, oldest first, and how many of the newest the
/// teammate's session has not yet been told.
#[derive(Default)]
pub struct Exchange {
    lines: VecDeque<Line>,
    unseen: usize,
}

impl Exchange {
    pub fn push(&mut self, speaker: Speaker, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        // One reply said sentence by sentence is one line.
        if speaker != Speaker::Person
            && let Some(last) = self.lines.back_mut()
            && last.speaker == speaker
        {
            last.text.push(' ');
            last.text.push_str(text);
            last.text = clip(&last.text);
            self.trim();
            return;
        }
        self.lines.push_back(Line {
            speaker,
            text: clip(text),
        });
        self.unseen += 1;
        self.trim();
    }

    /// What a call's thread says was said, under the same caps as it was kept
    /// under live. Every line is taken as told to the session: what the last
    /// process's session was not told is not known, and the voice keeps its own
    /// memory of it in any case.
    pub fn from_thread(events: &[Value]) -> Self {
        let mut exchange = Self::default();
        for event in events {
            let speaker = match event.get("kind").and_then(Value::as_str) {
                Some("user") => Speaker::Person,
                Some("agent") if event.get("relayed").and_then(Value::as_bool) == Some(true) => {
                    Speaker::Relayed
                }
                Some("agent") => Speaker::Voice,
                _ => continue,
            };
            if let Some(text) = event.get("text").and_then(Value::as_str) {
                exchange.push(speaker, text);
            }
        }
        exchange.told();
        exchange
    }

    /// Every line, oldest first.
    pub fn lines(&self) -> Vec<Line> {
        self.lines.iter().cloned().collect()
    }

    /// The lines the session has not been told, oldest first.
    pub fn unseen(&self) -> Vec<Line> {
        self.lines
            .iter()
            .skip(self.lines.len() - self.unseen.min(self.lines.len()))
            .cloned()
            .collect()
    }

    /// The session has now been told everything so far.
    pub fn told(&mut self) {
        self.unseen = 0;
    }

    fn trim(&mut self) {
        let mut chars: usize = self.lines.iter().map(|line| line.text.len()).sum();
        while self.lines.len() > MAX_LINES || (chars > MAX_CHARS && self.lines.len() > 1) {
            let Some(gone) = self.lines.pop_front() else {
                break;
            };
            chars -= gone.text.len();
        }
        self.unseen = self.unseen.min(self.lines.len());
    }
}

fn clip(text: &str) -> String {
    if text.chars().count() <= MAX_LINE_CHARS {
        return text.to_string();
    }
    let mut kept: String = text.chars().take(MAX_LINE_CHARS).collect();
    kept.push('…');
    kept
}

/// What the session is told ahead of the person's exact words on a handoff:
/// the call lines it has not seen, framed as the call's. None when there are
/// none, and then the words go alone.
pub fn handoff_preamble(unseen: &[Line]) -> Option<String> {
    let told: Vec<String> = unseen
        .iter()
        .filter_map(|line| match line.speaker {
            Speaker::Person => Some(format!("The person: {}", line.text)),
            Speaker::Voice => Some(format!("Your voice: {}", line.text)),
            Speaker::Relayed => None,
        })
        .collect();
    if told.is_empty() {
        return None;
    }
    Some(format!(
        "Earlier on this voice call, answered by your voice without passing it to you:\n{}\n\nThe person now says, by voice:",
        told.join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_call_keeps_only_its_newest_lines_and_characters() {
        let mut exchange = Exchange::default();
        for number in 0..45 {
            exchange.push(Speaker::Person, &format!("line {number}"));
            exchange.push(Speaker::Voice, &format!("reply {number}"));
        }
        let lines = exchange.lines();
        assert_eq!(lines.len(), MAX_LINES);
        assert_eq!(lines.last().unwrap().text, "reply 44");
        assert!(exchange.unseen().len() <= MAX_LINES);

        let mut exchange = Exchange::default();
        for _ in 0..20 {
            exchange.push(Speaker::Person, &"x".repeat(5_000));
        }
        let lines = exchange.lines();
        assert!(lines.iter().all(|line| line.text.chars().count() <= 801));
        assert!(lines.iter().map(|line| line.text.len()).sum::<usize>() <= MAX_CHARS + 4);
    }

    #[test]
    fn a_reply_said_in_sentences_is_one_line_and_the_person_breaks_it() {
        let mut exchange = Exchange::default();
        exchange.push(Speaker::Voice, "First.");
        exchange.push(Speaker::Voice, "Second.");
        exchange.push(Speaker::Person, "Hm.");
        exchange.push(Speaker::Voice, "Third.");
        let said: Vec<_> = exchange.lines().into_iter().map(|l| l.text).collect();
        assert_eq!(said, ["First. Second.", "Hm.", "Third."]);
    }

    #[test]
    fn the_exchange_is_rebuilt_from_the_thread_under_the_same_caps() {
        let said = |kind: &str, text: &str, relayed: bool| {
            let mut line = serde_json::json!({"kind": kind, "id": text, "ts": 1, "text": text});
            if relayed {
                line["relayed"] = true.into();
            }
            line
        };
        let mut events = vec![
            serde_json::json!({"kind": "link", "id": "link:call:c", "ts": 1}),
            said("user", "Is the winch fixed?", false),
            said("agent", "Not yet.", false),
            said("agent", "Still waiting on the part.", true),
        ];
        let rebuilt = Exchange::from_thread(&events);
        let lines = rebuilt.lines();
        assert_eq!(
            lines.iter().map(|line| line.speaker).collect::<Vec<_>>(),
            [Speaker::Person, Speaker::Voice, Speaker::Relayed]
        );
        assert!(rebuilt.unseen().is_empty());

        for number in 0..45 {
            events.push(said("user", &format!("line {number}"), false));
            events.push(said("agent", &format!("reply {number}"), false));
        }
        let rebuilt = Exchange::from_thread(&events);
        assert_eq!(rebuilt.lines().len(), MAX_LINES);
        assert_eq!(rebuilt.lines().last().unwrap().text, "reply 44");
    }

    #[test]
    fn only_lines_after_the_last_telling_are_unseen() {
        let mut exchange = Exchange::default();
        exchange.push(Speaker::Person, "One.");
        exchange.push(Speaker::Voice, "Two.");
        assert_eq!(exchange.unseen().len(), 2);
        exchange.told();
        assert!(exchange.unseen().is_empty());
        exchange.push(Speaker::Person, "Three.");
        assert_eq!(exchange.unseen()[0].text, "Three.");
    }

    #[test]
    fn a_handoff_is_told_the_unseen_call_lines_the_session_did_not_write() {
        let unseen = [
            Line {
                speaker: Speaker::Person,
                text: "I'm thinking about the launch.".into(),
            },
            Line {
                speaker: Speaker::Voice,
                text: "What's worrying you?".into(),
            },
            Line {
                speaker: Speaker::Relayed,
                text: "The session's own report.".into(),
            },
        ];
        let told = handoff_preamble(&unseen).unwrap();
        assert!(told.contains("The person: I'm thinking about the launch."));
        assert!(told.contains("Your voice: What's worrying you?"));
        assert!(!told.contains("own report"));
        assert!(told.ends_with("by voice:"));
        assert_eq!(handoff_preamble(&[]), None);
        assert_eq!(handoff_preamble(&unseen[2..]), None);
    }
}
