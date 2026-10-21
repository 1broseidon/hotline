//! What a direct call keeps of itself: the call as a thread.
//!
//! A call with a teammate is a thread (as kept by the room) whose parent is
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

use crate::call_room::CallRoom;
use hotline_contract::End;
use hotline_contract::VoiceEndReason;
fn now_ms() -> i64 {
    chrono::Local::now().timestamp_millis()
}
use serde_json::{Value, json};
use std::sync::Arc;
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
    room: Arc<dyn CallRoom>,
    thread: hotline_contract::ThreadId,
}

impl Record {
    /// Opens the call's thread under `persona_id`'s DM and starts its writer.
    /// The link is the first thing written.
    pub(super) fn open(
        room: Arc<dyn CallRoom>,
        call_id: &hotline_contract::ThreadId,
        persona_id: &str,
    ) -> Self {
        let (sender, mut entries) = mpsc::unbounded_channel();
        room.call_began(call_id, persona_id, now_ms());
        let record = Self {
            sender,
            room: room.clone(),
            thread: call_id.clone(),
        };
        let call_id = call_id.clone();
        let persona_id = persona_id.to_string();
        tokio::spawn(async move {
            while let Some(entry) = entries.recv().await {
                let Some(room) = room.upgrade() else {
                    return;
                };
                match entry {
                    Entry::Line(line) => room.call_said(&call_id, &persona_id, &line),
                    Entry::Ended { ts, end, outcome } => {
                        room.call_ended(&call_id, &persona_id, end, outcome, ts);
                    }
                }
            }
        });
        record
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
        self.room.call_detached(&self.thread);
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

/// Generated words are not proof of playback. Keep the interrupted line,
/// without guessing a word boundary from elapsed audio time.
#[derive(Default)]
pub(super) struct SpeechLine {
    id: String,
    text: String,
    cut: bool,
    pub(super) pending: Option<String>,
}

impl SpeechLine {
    pub(super) fn update(&mut self, id: &str, text: &str) {
        if self.id != id {
            self.id = id.into();
            self.cut = false;
        }
        self.text = text.into();
    }

    pub(super) fn interrupt(&mut self, played_ms: Option<u32>) -> Option<Value> {
        if self.cut || self.text.is_empty() {
            return None;
        }
        self.cut = true;
        let timing =
            played_ms.map_or_else(String::new, |ms| format!(" Playback stopped at {ms} ms."));
        let text = format!(
            "Speech interrupted.{timing} The caller did not hear this line to the end: {}\nThe exact last word heard is unknown. Answer the caller's interruption; do not resume the unfinished speech.",
            self.text
        );
        self.pending = Some(text.clone());
        Some(
            json!({"kind": "notice", "id": uuid::Uuid::new_v4().to_string(),
            "ts": now_ms(), "level": "info", "text": text, "interrupted": true,
            "speechId": self.id, "playedMs": played_ms}),
        )
    }
}

impl super::Call {
    pub(super) fn cut_speech(&mut self, played_ms: Option<u32>) {
        if let Some(line) = self.speech_line.interrupt(played_ms)
            && let Some(record) = &self.record
        {
            let _ = record.sender.send(Entry::Line(line));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cut_keeps_the_generated_line_without_claiming_it_was_heard() {
        let mut line = SpeechLine::default();
        line.update("reply", "First sentence. Unheard ending.");
        let cut = line.interrupt(Some(420)).unwrap();
        let _: hotline_contract::TranscriptEvent = serde_json::from_value(cut.clone()).unwrap();
        assert_eq!(cut["speechId"], "reply");
        assert_eq!(cut["playedMs"], 420);
        assert_eq!(cut["interrupted"], true);
        assert!(cut["text"].as_str().unwrap().contains("Unheard ending."));
        assert!(line.pending.as_ref().unwrap().contains("do not resume"));
        line.update("reply", "A late tail.");
        assert!(line.interrupt(Some(0)).is_none());
        line.update("next", "The new answer.");
        assert!(line.interrupt(None).is_some());
    }
}
