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

/// The most of one line the summary quotes.
const QUOTE_CHARS: usize = 160;

/// What a front call has done since its last post to the teammate's DM, for
/// the one summary that stands for it there. The call's own thread keeps what
/// was said; the DM gets this, never the turns.
#[derive(Default)]
pub(super) struct Digest {
    asked: Vec<String>,
    /// Tool names with how often each ran, in the order first run.
    tools: Vec<(String, u32)>,
    handed: Vec<String>,
}

impl Digest {
    /// The person asked this.
    pub(super) fn asked(&mut self, words: &str) {
        self.asked.push(words.trim().to_string());
    }

    /// The front ran a desk-side tool.
    pub(super) fn ran(&mut self, tool: &str) {
        match self.tools.iter_mut().find(|(name, _)| name == tool) {
            Some((_, count)) => *count += 1,
            None => self.tools.push((tool.to_string(), 1)),
        }
    }

    /// The front handed these words to the working session.
    pub(super) fn handed(&mut self, words: &str) {
        self.handed.push(words.trim().to_string());
    }

    /// The summary of what has happened since the last one, tagged as from
    /// the call, or `None` when nothing has. `open` is what the person last
    /// asked that nothing has answered.
    pub(super) fn summary(&mut self, open: Option<&str>) -> Option<String> {
        if self.asked.is_empty() && self.tools.is_empty() && self.handed.is_empty() {
            return None;
        }
        let quote = |text: &String| format!("\"{}\"", crate::driver::clip(text, QUOTE_CHARS));
        let mut lines = vec!["From the call:".to_string()];
        if !self.asked.is_empty() {
            let asked: Vec<String> = self.asked.iter().map(quote).collect();
            lines.push(format!("Asked: {}", asked.join("; ")));
        }
        let mut done: Vec<String> = self
            .tools
            .iter()
            .map(|(name, count)| match count {
                1 => format!("ran {name}"),
                count => format!("ran {name} ×{count}"),
            })
            .collect();
        done.extend(
            self.handed
                .iter()
                .map(|words| format!("handed to the working session: {}", quote(words))),
        );
        if !done.is_empty() {
            lines.push(format!("Done: {}", done.join("; ")));
        }
        if let Some(open) = open {
            lines.push(format!("Left open: {}", quote(&open.to_string())));
        }
        *self = Self::default();
        Some(lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_digest_says_what_was_asked_done_and_left_open_once() {
        let mut digest = Digest::default();
        assert_eq!(digest.summary(None), None);
        digest.asked("what is on the PR?");
        digest.ran("search_thread");
        digest.ran("search_thread");
        digest.handed("check the failing PR");
        let summary = digest.summary(Some("and the release?")).unwrap();
        assert!(summary.starts_with("From the call:"), "{summary}");
        assert!(summary.contains("Asked: \"what is on the PR?\""));
        assert!(summary.contains("ran search_thread ×2"));
        assert!(summary.contains("handed to the working session: \"check the failing PR\""));
        assert!(summary.contains("Left open: \"and the release?\""));
        assert_eq!(digest.summary(None), None, "a summary is given once");
    }
}
