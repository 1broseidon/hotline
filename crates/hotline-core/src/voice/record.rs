//! What a direct call keeps of itself: the call as a thread.
//!
//! A call with a teammate is a thread (see [`crate::thread`]) whose parent is
//! the teammate's DM. Its lines are on `calls/<id>.jsonl`, found by
//! `search_thread`, and one `link` line on the DM stands for it. A call that has
//! ended is read back as any other thread is.
//!
//! The writes are made by a task of the call's own, in the order they were
//! sent, so that the call's path to the person's ear never waits on a file or
//! the search index: saying a line is pushing it on a channel.
//!
//! The teammate's session keeps the person's words in its own conversation
//! too, as the turns they were; this is the call as it was heard and said.
//! With a voice front on the call, the person's lines are what the front
//! heard and the voice's are what the front said; the teammate's tape holds
//! only what was handed to it and what it answered.

use crate::contract::VoiceEndReason;
use crate::session::{Room, now_ms};
use crate::thread::End;
use serde_json::{Value, json};
use std::sync::Weak;
use tokio::sync::mpsc;

/// Who said a line on the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Speaker {
    /// The person on the call.
    Person,
    /// The teammate, the desk or a voice front, speaking on it.
    Voice,
}

/// One thing to be written, with the time it happened.
enum Entry {
    Began {
        ts: i64,
    },
    Line(Value),
    Ended {
        ts: i64,
        end: End,
        outcome: &'static str,
    },
}

/// The sending end of a direct call's thread. Writing stops with the room.
pub(super) struct Record {
    sender: mpsc::UnboundedSender<Entry>,
}

impl Record {
    /// Opens the call's thread under `persona_id`'s DM and starts its writer.
    /// The link is the first thing written.
    pub(super) fn open(room: Weak<Room>, call_id: &str, persona_id: &str) -> Self {
        let (sender, mut entries) = mpsc::unbounded_channel();
        let _ = sender.send(Entry::Began { ts: now_ms() });
        let call_id = call_id.to_string();
        let persona_id = persona_id.to_string();
        tokio::spawn(async move {
            while let Some(entry) = entries.recv().await {
                let Some(room) = room.upgrade() else {
                    return;
                };
                match entry {
                    Entry::Began { ts } => room.call_began(&call_id, &persona_id, ts),
                    Entry::Line(line) => room.call_said(&call_id, &persona_id, &line),
                    Entry::Ended { ts, end, outcome } => {
                        room.call_ended(&call_id, &persona_id, end, outcome, ts);
                    }
                }
            }
        });
        Self { sender }
    }

    /// A line said on the call.
    pub(super) fn said(&self, speaker: Speaker, id: &str, text: &str) {
        let ts = now_ms();
        let kind = match speaker {
            Speaker::Person => "user",
            Speaker::Voice => "agent",
        };
        let line = json!({"kind": kind, "id": id, "ts": ts, "text": text});
        let _ = self.sender.send(Entry::Line(line));
    }

    /// The call is over, for this reason.
    pub(super) fn ended(&self, reason: Option<VoiceEndReason>) {
        let (end, outcome) = match reason {
            Some(VoiceEndReason::Client) | None => (End::Person, "Hung up"),
            Some(VoiceEndReason::Goodbye) => (End::Person, "Said goodbye"),
            Some(VoiceEndReason::Idle) => (End::Idle, "Went quiet"),
            Some(VoiceEndReason::Budget) => (End::Failed, "The voice budget ran out"),
            Some(VoiceEndReason::Error) => (End::Failed, "The voice kept failing"),
            Some(VoiceEndReason::Replaced) => (End::Stopped, "Replaced by a newer call"),
        };
        let _ = self.sender.send(Entry::Ended {
            ts: now_ms(),
            end,
            outcome,
        });
    }
}

/// The most of what the person said, and of what the front answered, that the
/// summary quotes, and how many exchanges it keeps: the newest.
const ASKED_CHARS: usize = 160;
const ANSWERED_CHARS: usize = 240;
const EXCHANGES: usize = 6;

/// What a front call has done since its last post to the teammate's DM, for
/// the one summary that stands for it there. The call's own thread keeps what
/// was said word for word; the DM gets this, in the teammate's voice.
#[derive(Default)]
pub(super) struct Digest {
    exchanges: Vec<Exchange>,
    /// Whether the front looked anything up for the person.
    looked_up: bool,
    handed: Vec<String>,
}

/// What the person asked, and what the front said back.
struct Exchange {
    asked: String,
    answered: Vec<String>,
}

impl Digest {
    /// The person asked this.
    pub(super) fn asked(&mut self, words: &str) {
        self.exchanges.push(Exchange {
            asked: words.trim().to_string(),
            answered: Vec::new(),
        });
    }

    /// The front said this to the person. A line before anyone asked is
    /// not an answer.
    pub(super) fn answered(&mut self, words: &str) {
        if let Some(exchange) = self.exchanges.last_mut() {
            exchange.answered.push(words.trim().to_string());
        }
    }

    /// The front looked something up with a desk-side tool.
    pub(super) fn looked_up(&mut self) {
        self.looked_up = true;
    }

    /// The front handed these words to the working session.
    pub(super) fn handed(&mut self, words: &str) {
        self.handed.push(words.trim().to_string());
    }

    /// The summary of what has happened since the last one, as the teammate
    /// would say it, tagged as from the call, or `None` when nothing has.
    /// `open` is what the person last asked that nothing has answered. Words
    /// are scrubbed and clipped as the front's own starting context is, since
    /// the summary becomes part of the teammate's history.
    pub(super) fn summary(&mut self, open: Option<&str>) -> Option<String> {
        if self.exchanges.is_empty() && !self.looked_up && self.handed.is_empty() {
            return None;
        }
        let quote = |text: &str, limit: usize| {
            format!(
                "\"{}\"",
                super::front_prompt::clip(&super::front_prompt::scrub(text), limit)
            )
        };
        let mut sentences = Vec::new();
        let skipped = self.exchanges.len().saturating_sub(EXCHANGES);
        for exchange in self.exchanges.iter().skip(skipped) {
            let asked = quote(&exchange.asked, ASKED_CHARS);
            let answered = exchange.answered.join(" ");
            sentences.push(if answered.trim().is_empty() {
                format!("You asked {asked}.")
            } else {
                format!(
                    "You asked {asked} and I said {}.",
                    quote(&answered, ANSWERED_CHARS)
                )
            });
        }
        if self.looked_up {
            sentences.push("I looked through our earlier conversations to answer.".to_string());
        }
        for words in &self.handed {
            sentences.push(format!(
                "I'm still working on this: {}.",
                quote(words, ASKED_CHARS)
            ));
        }
        if let Some(open) = open {
            sentences.push(format!(
                "I have not answered {} yet.",
                quote(open, ASKED_CHARS)
            ));
        }
        *self = Self::default();
        Some(format!("From the call: {}", sentences.join(" ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_digest_is_the_teammates_account_of_the_call_given_once() {
        let mut digest = Digest::default();
        assert_eq!(digest.summary(None), None);
        digest.asked("what is on the PR?");
        digest.looked_up();
        digest.looked_up();
        digest.answered("The scheduler fix is in.");
        digest.handed("check the failing PR");
        let summary = digest.summary(Some("and the release?")).unwrap();
        assert!(summary.starts_with("From the call: You asked"), "{summary}");
        assert!(summary.contains("\"what is on the PR?\" and I said \"The scheduler fix is in.\""));
        assert!(summary.contains("I'm still working on this: \"check the failing PR\""));
        assert!(summary.contains("I have not answered \"and the release?\" yet."));
        assert!(!summary.contains("search_thread") && !summary.contains("session"));
        assert_eq!(digest.summary(None), None, "a summary is given once");
    }

    #[test]
    fn a_summary_scrubs_and_keeps_only_the_newest_exchanges() {
        let mut digest = Digest::default();
        for n in 0..8 {
            digest.asked(&format!("question {n}?"));
            digest.answered("sk-live-abcdefghijklmnopqrstuvwxyz0123456789");
        }
        let summary = digest.summary(None).unwrap();
        assert!(!summary.contains("question 1?") && summary.contains("question 2?"));
        assert!(
            !summary.contains("abcdefghijklmnopqrstuvwxyz0123456789"),
            "{summary}"
        );
    }
}
