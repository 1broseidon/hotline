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
//!
//! An agent asks for it with `post_to_chat`, one of Hotline's own tools, so an
//! ACP child reaches it over the desk's MCP server and Hotline Agent through
//! the same handler in process. The tool is listed for every session, because
//! a child's tool list is fixed when its session starts; whether a call may
//! post is decided here, outside the model, by the thread's kind and the
//! turn's modality ([`Room::post_to_chat`]). Every turn of a thread's agent
//! opens a [`Posting`] while it runs, and a call that lands outside one is
//! refused.

use super::{Room, Wired, lock, new_id, now_ms, quiet, timed};
use crate::contract::{
    Attachment, DeliveryFrom, ScheduledRun, ThreadId, ThreadKind, TranscriptEvent,
};
use std::sync::Arc;

/// How many posts from elsewhere ride on the next line of a live session.
const MAX_PENDING_POSTS: usize = 6;

/// The most one post to the chat may say; a longer account is a thread's to
/// keep, and the post can point at it.
pub(crate) const POST_CHARS: usize = 2_000;

/// What `post_to_chat` says when the DM's own turn asks: the person is
/// reading that turn's reply.
const LOUD: &str = "Your reply here is already the chat: say it in your reply.";

/// What `post_to_chat` says to a second call in one turn, the wording the
/// quiet run's one note always had.
const AGAIN: &str = "Already posted: one per turn. Put everything in that one next time.";

/// What a turn of a thread's agent may post, and whether it has.
pub(super) struct Posting {
    /// The job a quiet scheduled run of the DM is: the one DM turn whose
    /// replies nobody sees, and so the one that may post. Its post carries the
    /// job's stamp.
    quiet: Option<ScheduledRun>,
    posted: bool,
}

/// A thread's turn, open for `post_to_chat` until it is dropped.
pub(super) struct PostingTurn<'a> {
    room: &'a Room,
    thread: ThreadId,
}

impl Drop for PostingTurn<'_> {
    fn drop(&mut self) {
        lock(&self.room.posting).remove(&self.thread);
    }
}

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
    /// Opens `thread`'s turn for `post_to_chat`, for as long as the answer is
    /// held. `quiet` is the job when the turn is a quiet scheduled run of the
    /// DM. A thread takes one turn at a time, so the turn opened here is the
    /// thread's only one.
    pub(super) fn open_posting(
        &self,
        thread: &ThreadId,
        quiet: Option<ScheduledRun>,
    ) -> PostingTurn<'_> {
        lock(&self.posting).insert(
            thread.clone(),
            Posting {
                quiet,
                posted: false,
            },
        );
        PostingTurn {
            room: self,
            thread: thread.clone(),
        }
    }

    /// `post_to_chat(text, wake)` from the turn of `thread` in flight: the
    /// gate, then [`Room::post`].
    ///
    /// Any thread of the teammate's may post but two. The DM's own turn is
    /// refused, because its reply is already the chat, unless it is a quiet
    /// scheduled run, whose replies nobody sees. A subagent run is refused,
    /// because it reports to the agent that launched it, which decides what
    /// the person hears. One post a turn, and so one wake; the post goes to
    /// the teammate's own chat, clipped at [`POST_CHARS`]. The lease is the
    /// caller's to check as the call lands, as it is for every tool.
    ///
    /// `wake` is for a thread that needs the home agent to act on what it
    /// posted, not only the person to know: the DM is handed the post on a
    /// turn of its own, and its reply is what the person hears next. The DM
    /// is the home agent, so it has nobody to wake.
    pub(crate) async fn post_to_chat(
        self: &Arc<Self>,
        persona_id: &str,
        thread: &ThreadId,
        text: &str,
        wake: bool,
    ) -> Result<String, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("post_to_chat needs the `text` to post.".to_string());
        }
        if thread.kind == ThreadKind::Run {
            return Err(SUBAGENT.to_string());
        }
        if wake && thread.kind == ThreadKind::Dm {
            return Err(
                "You are the main conversation: there is nobody to wake. Post without `wake`."
                    .to_string(),
            );
        }
        // A quiet run the person has spoken into since is answering them, out
        // loud: its window is what says whether anyone sees its replies.
        let unseen = thread.kind != ThreadKind::Dm
            || self
                .session(persona_id)
                .is_ok_and(|session| quiet::mutes_deltas(lock(&session.quiet).as_ref(), now_ms()));
        let scheduled = {
            let mut posting = lock(&self.posting);
            let turn = posting.get_mut(thread).ok_or_else(|| {
                "post_to_chat posts from a turn in progress, and this thread has none.".to_string()
            })?;
            if thread.kind == ThreadKind::Dm && (turn.quiet.is_none() || !unseen) {
                return Err(LOUD.to_string());
            }
            if turn.posted {
                return Err(AGAIN.to_string());
            }
            turn.posted = true;
            turn.quiet.clone()
        };
        let posted = self
            .post_and_wake(persona_id, thread, text, scheduled, wake)
            .await;
        if posted.is_err()
            && let Some(turn) = lock(&self.posting).get_mut(thread)
        {
            turn.posted = false;
        }
        posted
    }

    /// The post the gate let through, and the DM woken for it when asked.
    async fn post_and_wake(
        self: &Arc<Self>,
        persona_id: &str,
        thread: &ThreadId,
        text: &str,
        scheduled: Option<ScheduledRun>,
        wake: bool,
    ) -> Result<String, String> {
        let post = Post {
            scheduled,
            ..Post::words(&crate::driver::clip(text, POST_CHARS))
        };
        if !wake {
            self.post(persona_id, thread, post)?;
            return Ok("Posted to the chat.".to_string());
        }
        // The DM is brought up and into its chapter first, so the post lands
        // in the chapter that hears it and is handed to the session that
        // acts on it.
        let _working = self.working()?;
        self.start(persona_id).await?;
        let (session, _held) = self.in_this_chapter(persona_id).await?;
        self.post(persona_id, thread, post)?;
        let posts = std::mem::take(&mut *lock(&session.pending_posts));
        let line = format!(
            "(What you posted to this chat from elsewhere since you last spoke here:\n{})\n\n\
             The last of these asks you to act on it now. Do, and tell the person what came of it.",
            posts.join("\n")
        );
        self.dispatch(session, Wired::words(timed(now_ms(), &line)));
        Ok("Posted to the chat, and your main conversation is acting on it.".to_string())
    }

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

/// What `post_to_chat` says to a subagent run.
pub(crate) const SUBAGENT: &str = "A subagent reports to the teammate that launched it, which \
decides what the person hears: put it in your report.";

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
