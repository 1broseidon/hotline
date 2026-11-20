//! What a direct call keeps of itself: the call as a thread.
//!
//! A call with a teammate is a thread (as kept by the room) whose parent is
//! the teammate's DM. Its lines are on `calls/<id>.jsonl`, found by
//! `search_thread`, and one `link` line on the DM stands for it. A call that has
//! ended is read back as any other thread is.
//!
//! Speech is written before the next call event is handled, so work can read
//! the complete conversation that preceded its request. Ending is queued:
//! summary generation must not hold up disconnecting the microphone.

use crate::call_room::CallRoom;
use crate::heard::{Playback, Stop};
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
    persona: Option<String>,
    client: Option<hotline_contract::Client>,
}

impl Record {
    /// Opens the call's record, under its teammate's DM when it has one.
    /// The link is the first thing written.
    pub(super) fn open(
        room: Arc<dyn CallRoom>,
        call_id: &hotline_contract::ThreadId,
        persona_id: Option<&str>,
    ) -> Self {
        let (sender, mut entries) = mpsc::unbounded_channel();
        room.call_began(call_id, persona_id, now_ms());
        let record = Self {
            sender,
            room: room.clone(),
            thread: call_id.clone(),
            persona: persona_id.map(str::to_string),
            client: crate::call_room::prompt_client(),
        };
        let call_id = call_id.clone();
        let persona_id = persona_id.map(str::to_string);
        tokio::spawn(async move {
            while let Some(entry) = entries.recv().await {
                let Some(room) = room.upgrade() else {
                    return;
                };
                match entry {
                    Entry::Ended { ts, end, outcome } => {
                        room.call_ended(&call_id, persona_id.as_deref(), end, outcome, ts);
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
        let mut line = json!({"kind": kind, "id": id, "ts": ts, "text": text});
        if speaker == Speaker::Person {
            line["client"] = json!(self.client);
        }
        self.line(&line);
    }

    pub(super) fn line(&self, line: &Value) {
        self.room
            .call_said(&self.thread, self.persona.as_deref(), line);
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

impl super::Call {
    /// Finalize live text before a boundary; unchanged text is never appended twice.
    pub(super) fn keep_front_speech(&mut self) {
        if self.front.is_none() {
            return;
        }
        let Some(line) = self.speech_line.current_mut() else {
            return;
        };
        if !line.recorded
            && !line.text.is_empty()
            && let Some(record) = &self.record
        {
            record.said(Speaker::Voice, &line.id, &line.text);
            self.conversation.remember(&line.text);
            line.recorded = true;
        }
    }

    /// The call's speech was stopped: the cut notice, kept on its thread,
    /// says what the caller heard of it ([`super::heard`]).
    pub(super) fn cut_speech(&mut self, stop: Stop, playback: Playback) {
        // A front's speech is one stream with no clips: a clip position
        // names nothing in it.
        let playback = match playback {
            Playback::Clip(_) if self.front.is_some() => Playback::Unreported,
            playback => playback,
        };
        self.keep_front_speech();
        if let Some(line) = self.speech_line.interrupt(stop, playback)
            && let Some(record) = &self.record
        {
            record.line(&line);
        }
    }
}
