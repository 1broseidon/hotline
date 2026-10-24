//! Voice attaches to a working thread; its agent outlives the microphone.
use super::{Post, Room, lock, new_id};
use crate::contract::{End, ThreadId, ThreadKind};
use crate::log::StreamId;
use crate::thread::{Link, ThreadState};
use serde_json::{Value, json};
use std::time::Duration;

pub(super) const CALL_CLOSING_DELAY: Duration = Duration::from_secs(120);

#[derive(Clone)]
pub(super) struct CallClosing {
    ticket: String,
    threads: Vec<ThreadId>,
    posted_after: usize,
}

const SUMMARY_CURSOR: &str = "call-closing-note";

impl Room {
    pub(super) fn call_instruction(&self, thread: &ThreadId) -> String {
        lock(&self.voice)
            .upgrade()
            .and_then(|calls| {
                calls
                    .thread_origin(thread)
                    .map(|origin| calls.turn_instruction(&origin.call_id))
            })
            .unwrap_or_else(|| crate::voice::spoken::CONTRACT.into())
    }

    pub(crate) fn lead_call(&self, persona: &str, thread: &ThreadId) -> bool {
        lock(&self.voice)
            .upgrade()
            .is_some_and(|calls| calls.lead_call(persona, thread))
    }

    pub(crate) fn validate_call_thread(
        &self,
        persona: &str,
        thread: &ThreadId,
    ) -> Result<(), String> {
        if !matches!(thread.kind, ThreadKind::Call | ThreadKind::Side) {
            return Err("Call a work thread or an earlier call.".into());
        }
        match self.link_of(thread) {
            Some(link) if link.persona_id.as_deref() != Some(persona) => {
                Err("That thread belongs to another teammate.".into())
            }
            Some(link) if matches!(link.state, ThreadState::Closed(_)) => {
                Err("Continue that thread before calling it.".into())
            }
            Some(link) if link.opener.is_some() => {
                Err("That thread was handed over by a teammate.".into())
            }
            None => Err("There is no such thread.".into()),
            _ => Ok(()),
        }
    }

    pub(crate) fn call_began(&self, thread: &ThreadId, persona: &str, ts: i64) {
        let mut closings = lock(&self.call_closings);
        let closing = closings
            .entry(persona.into())
            .or_insert_with(|| CallClosing {
                ticket: new_id(),
                threads: Vec::new(),
                posted_after: self.tape(persona).len(),
            });
        closing.ticket = new_id();
        if !closing.threads.contains(thread) {
            closing.threads.push(thread.clone());
        }
        drop(closings);
        if self.link_of(thread).is_some() {
            return;
        }
        self.write_link(&Link {
            id: Link::fresh_id(thread),
            ts,
            thread: thread.clone(),
            persona_id: Some(persona.into()),
            title: "Call".into(),
            state: ThreadState::Live,
            outcome: None,
            at: None,
            note: None,
            binding: None,
            elapsed_ms: None,
            opener: None,
            handoff_ended: false,
        });
    }

    pub(crate) fn call_spoken(
        &self,
        persona: &str,
        thread: &ThreadId,
        id: &str,
        spoken: &str,
    ) -> bool {
        let Some(stream) = StreamId::of_thread(thread) else {
            return false;
        };
        let Some(mut event) = self
            .log
            .load(&stream)
            .into_iter()
            .find(|event| event["kind"] == "agent" && event["id"] == id)
        else {
            return false;
        };
        event["spoken"] = json!(spoken);
        self.threads().write(thread, persona, &event);
        true
    }

    pub(crate) fn call_said(&self, thread: &ThreadId, persona: &str, line: &Value) {
        self.threads().write(thread, persona, line);
    }

    pub(super) fn attached_voice(&self, thread: &ThreadId) -> Option<crate::voice::Origin> {
        lock(&self.voice)
            .upgrade()
            .and_then(|voice| voice.thread_origin(thread))
    }

    /// Capture the turn at disconnection, before the asynchronous call log
    /// can catch a later typed turn instead.
    pub(crate) fn call_detached(&self, thread: &ThreadId) {
        if let Some(turn) = lock(&self.posting).get_mut(thread) {
            turn.detached = true;
        }
    }

    pub(crate) fn call_ended(
        &self,
        thread: &ThreadId,
        persona: &str,
        _end: End,
        _outcome: &str,
        _ts: i64,
    ) {
        let Some(room) = self.this() else { return };
        let ticket = new_id();
        {
            let mut closings = lock(&self.call_closings);
            let Some(closing) = closings.get_mut(persona) else {
                return;
            };
            closing.ticket.clone_from(&ticket);
        }
        let thread = thread.clone();
        let persona = persona.to_string();
        let due = tokio::time::Instant::now() + CALL_CLOSING_DELAY;
        self.threads().write(&thread, &persona, &json!({"kind":"notice", "id":new_id(), "ts":super::now_ms(), "level":"info", "text":"Voice disconnected. This thread stays open."}));
        tokio::spawn(async move {
            tokio::time::sleep_until(due).await;
            loop {
                let Some(closing) = lock(&room.call_closings)
                    .get(&persona)
                    .cloned()
                    .filter(|closing| closing.ticket == ticket)
                else {
                    return;
                };
                if closing
                    .threads
                    .iter()
                    .any(|thread| room.on_the_call(thread) || room.work_thread_working(thread))
                {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
                let intervals: Vec<_> = closing
                    .threads
                    .iter()
                    .map(|thread| room.call_interval(thread))
                    .collect();
                let mut slice: Vec<_> = intervals.iter().flatten().cloned().collect();
                let posted = room.call_posts(&persona, &closing);
                if !posted.is_empty() {
                    slice.push(json!({"kind":"user", "text":format!("Already delivered to the person's chat (complete written results, independent of speech playback): {}", posted.iter().map(crate::sent::message_text).collect::<Vec<_>>().join("\n\n"))}));
                }
                let note = if slice.is_empty() {
                    None
                } else {
                    let Ok(persona_view) = room.persona(&persona) else {
                        return;
                    };
                    let mut note = room.call_note(&persona_view, &slice).await.unwrap_or_else(|| super::chapters::Note {
                        title: "Call".into(),
                        note: "The calls ended, but I could not prepare their summary. Open the linked calls for the conversation and finished work.".into(),
                        status: crate::contract::ChapterStatus::InProgress,
                        tags: Vec::new(),
                    });
                    // A model cannot infer missing work from an interrupted spoken rendition.
                    if !posted.is_empty() && claims_missing_result(&note.note) {
                        note.note = "The calls finished. The written results are already in this chat; the linked calls keep the conversation.".into();
                    }
                    Some(note)
                };
                let _admission = room.sides.waking.lock().await;
                if closing
                    .threads
                    .iter()
                    .any(|thread| room.on_the_call(thread) || room.work_thread_working(thread))
                    || closing
                        .threads
                        .iter()
                        .zip(&intervals)
                        .any(|(thread, before)| room.call_interval(thread) != *before)
                    || room.call_posts(&persona, &closing) != posted
                {
                    continue;
                }
                // Claim publication atomically with call admission, without holding this
                // lock across voice lookups (call start holds the voice lock first).
                {
                    let mut closings = lock(&room.call_closings);
                    if closings
                        .get(&persona)
                        .is_none_or(|closing| closing.ticket != ticket)
                    {
                        return;
                    }
                    closings.remove(&persona);
                }
                if let Some(note) = &note {
                    let post = Post {
                        sources: closing.threads.clone(),
                        ..Post::words(&note.note)
                    };
                    if room.post(&persona, &thread, post).is_err() {
                        return;
                    }
                    for source in &closing.threads {
                        room.threads().write(source, &persona, &json!({"kind":"notice", "id":format!("{SUMMARY_CURSOR}:{}", new_id()), "ts":super::now_ms(), "level":"info", "text":note.note}));
                    }
                }
                for source in &closing.threads {
                    if !room.on_the_call(source) {
                        room.park_after_call(source, note.clone());
                    }
                }
                return;
            }
        });
    }

    fn call_interval(&self, thread: &ThreadId) -> Vec<Value> {
        let Some(stream) = StreamId::of_thread(thread) else {
            return Vec::new();
        };
        let all = self.log.load(&stream);
        let after = all
            .iter()
            .rposition(|event| {
                event["id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with(SUMMARY_CURSOR))
            })
            .map_or(0, |at| at + 1);
        all.into_iter().skip(after).filter(call_content).collect()
    }

    fn call_posts(&self, persona: &str, closing: &CallClosing) -> Vec<Value> {
        self.tape(persona)
            .into_iter()
            .skip(closing.posted_after)
            .filter(|event| {
                event["kind"] == "agent"
                    && closing.threads.iter().any(|thread| {
                        event["from"]["thread"] == thread.key
                            && event["from"]["kind"] == thread.kind.name()
                    })
            })
            .collect()
    }

    /// Only the turn caught by hang-up reports automatically. Its complete
    /// written answer stays on the thread, even if preparing a summary fails.
    pub(super) async fn post_detached_reply(
        &self,
        persona: &str,
        thread: &ThreadId,
        subject: &str,
        replies: &[Value],
    ) {
        let detached = lock(&self.posting)
            .get(thread)
            .is_some_and(|turn| turn.detached);
        let closing = lock(&self.call_closings).get(persona).cloned();
        let already_delivered = closing.as_ref().is_some_and(|closing| {
            self.call_posts(persona, closing)
                .iter()
                .any(|event| event["from"]["thread"] == thread.key)
        });
        if !detached || replies.is_empty() || already_delivered {
            return;
        }
        let text = replies
            .iter()
            .filter_map(|event| event["text"].as_str())
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        if text.is_empty() {
            return;
        }
        let text = if short_call_post(&text) {
            text
        } else {
            let mut slice = vec![json!({"kind":"user", "text":subject})];
            slice.extend_from_slice(replies);
            let summary = match self.persona(persona) {
                Ok(persona) => self.call_reply_note(&persona, &slice).await,
                Err(_) => None,
            };
            summary.unwrap_or_else(|| {
                let subject = self.link_of(thread).map(|link| link.title)
                    .filter(|title| !title.is_empty() && title != "Call")
                    .unwrap_or_else(|| subject.lines().next().unwrap_or("Call work").to_owned());
                format!("{}: the reply is ready in the call thread. I could not prepare its short summary.", crate::driver::clip(&subject, 160))
            })
        };
        let _ = self.post(persona, thread, Post::words(&text));
    }

    fn park_after_call(&self, thread: &ThreadId, note: Option<super::chapters::Note>) {
        self.end_thread(thread, super::sides::Ending::Park);
        if let Some(mut link) = self.link_of(thread)
            && !matches!(link.state, ThreadState::Closed(_))
        {
            link.state = ThreadState::Parked;
            if let Some(note) = note {
                if thread.kind == ThreadKind::Call {
                    link.title = note.title;
                }
                link.note = Some(note.note);
            }
            self.write_link(&link);
        }
    }
}

/// Three short paragraphs fit in chat; longer answers belong behind the link.
fn short_call_post(text: &str) -> bool {
    text.chars().count() <= 900
        && text
            .split("\n\n")
            .filter(|part| !part.trim().is_empty())
            .count()
            <= 3
}

/// Finished work belongs in the closing account even when no voice read it aloud.
fn call_content(event: &Value) -> bool {
    crate::store::chapters::is_message(event) || event["toolKind"] == "work_result"
}

fn claims_missing_result(note: &str) -> bool {
    let lower = note.to_lowercase();
    [
        "cut off",
        "cut short",
        "truncat",
        "incomplete",
        "missing",
        "not delivered",
    ]
    .iter()
    .any(|claim| lower.contains(claim))
}
