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
