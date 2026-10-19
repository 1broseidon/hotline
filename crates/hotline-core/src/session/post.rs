//! One door to the chat (ADR 0008 §3).
//!
//! The DM is the log of truth: whatever a teammate gives the person reaches
//! it, wherever the teammate was when it said so. Anything not said in a turn
//! of the DM itself comes in through [`Room::post`] — a call's summary or what
//! the call was asked to put in the chat, a quiet run's note, a file sent from
//! a work thread — as one `agent` message stamped with where it came from, so
//! a client draws "From the call" from the stamp rather than from words in
//! the text.
//!
//! It never passes through the DM's quiet window. That window turns the
//! teammate's words into thinking by kind, and this is the one way a quiet
//! run is heard.

use super::{Room, lock, new_id, now_ms};
use crate::contract::{
    Attachment, DeliveryFrom, ScheduledRun, ThreadId, ThreadKind, TranscriptEvent,
};

/// How many posts from elsewhere ride on the next line of a live session.
const MAX_PENDING_POSTS: usize = 6;

/// What a thread posts to the chat.
pub(crate) struct Post {
    /// The message's id: a file is kept under it before it is posted.
    pub id: String,
    pub text: String,
    pub attachments: Option<Vec<Attachment>>,
    /// The job, when a scheduled run posted it.
    pub scheduled: Option<ScheduledRun>,
}

impl Post {
    pub(crate) fn words(text: &str) -> Self {
        Self {
            id: new_id(),
            text: text.to_string(),
            attachments: None,
            scheduled: None,
        }
    }
}

impl Room {
    /// Writes `post` to the teammate's DM as the teammate's own message,
    /// `from` the thread it was said on, and says whether it landed.
    ///
    /// It wakes the person as a reply does, except from a call they are on.
    /// It does not wake the home agent: everything posted so far was asked
    /// for by the person (they placed the call, opened the work thread,
    /// scheduled the job), so the post is for them. A live home session is
    /// handed what its other threads posted ahead of the person's next line,
    /// so a typed follow-up does not start cold, and a session that starts
    /// later reads it from the tape. A post from the DM's own turn, which is
    /// a quiet run's, is already in that session's context.
    pub(crate) fn post(&self, persona_id: &str, from: &ThreadId, post: Post) -> Result<(), String> {
        let elsewhere = from.kind != ThreadKind::Dm;
        let event = TranscriptEvent::Agent {
            id: post.id,
            ts: now_ms(),
            text: post.text,
            attachments: post.attachments,
            reactions: None,
            ring: None,
            receipt: None,
            spoken: None,
            from: elsewhere.then(|| DeliveryFrom::new(from, None)),
            scheduled: post.scheduled,
        };
        let event = serde_json::to_value(&event).map_err(|error| error.to_string())?;
        self.try_write_value(persona_id, &event)
            .map_err(|error| format!("The chat could not be written to: {error}"))?;
        let said = crate::sent::message_text(&event);
        if elsewhere && let Ok(session) = self.session(persona_id) {
            let origin = posted_from(&event).unwrap_or_default();
            let mut posts = lock(&session.pending_posts);
            posts.push(format!("{origin}: {said}"));
            let excess = posts.len().saturating_sub(MAX_PENDING_POSTS);
            posts.drain(..excess);
        }
        if !self.on_the_call(from) {
            let name = self
                .persona(persona_id)
                .map_or_else(|_| "Hotline".to_string(), |persona| persona.name);
            self.push.notify(&name, &said, persona_id, None);
        }
        Ok(())
    }

    /// Whether `thread` is a call the person is on now.
    fn on_the_call(&self, thread: &ThreadId) -> bool {
        thread.kind == ThreadKind::Call
            && lock(&self.voice)
                .upgrade()
                .is_some_and(|voice| voice.is_up(&thread.key))
    }
}

/// Where a post on the tape came from, as the teammate is told it: "From
/// your call". `None` for a message the DM's own turn said.
pub(super) fn posted_from(event: &serde_json::Value) -> Option<String> {
    let field = |key: &str| event.get(key).cloned().unwrap_or_default();
    if let Ok(run) = serde_json::from_value::<ScheduledRun>(field("scheduled")) {
        return Some(format!("From your scheduled job \"{}\"", run.name));
    }
    let from = serde_json::from_value::<DeliveryFrom>(field("from")).ok()?;
    Some(
        match from.kind {
            ThreadKind::Dm => return None,
            ThreadKind::Call => "From your call",
            ThreadKind::Side => "From your work thread",
            ThreadKind::Pair => "From your thread with a colleague",
            ThreadKind::Run => "From your subagent",
        }
        .to_string(),
    )
}
