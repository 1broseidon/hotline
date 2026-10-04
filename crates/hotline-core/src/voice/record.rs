//! What a direct call keeps of itself: the call as a thread.
//!
//! A call with a teammate is a thread (see [`crate::thread`]) whose parent is
//! the teammate's DM. Its lines are on `calls/<id>.jsonl`, found by
//! `search_thread`, and one `link` line on the DM stands for it. A call that has
//! ended is read back as any other thread is.
//!
//! The writes are made by a task of the call's own, in the order they were
//! sent, so that the call's path to the person's ear never waits on a file or
//! the search index: saying a line is pushing it on a channel. The call's
//! [`Exchange`](super::exchange::Exchange) is what the voice is given from
//! what was said; this is where all of it is kept.

use super::exchange::Speaker;
use crate::contract::VoiceEndReason;
use crate::session::{Room, now_ms};
use crate::thread::End;
use serde_json::{Value, json};
use std::sync::Weak;
use tokio::sync::mpsc;

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

    /// A line said on the call. A relayed line is the teammate's own report,
    /// retold by the voice.
    pub(super) fn said(&self, speaker: Speaker, id: &str, text: &str) {
        let ts = now_ms();
        let line = match speaker {
            Speaker::Person => json!({"kind": "user", "id": id, "ts": ts, "text": text}),
            Speaker::Voice => json!({"kind": "agent", "id": id, "ts": ts, "text": text}),
            Speaker::Relayed => {
                json!({"kind": "agent", "id": id, "ts": ts, "text": text, "relayed": true})
            }
        };
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
