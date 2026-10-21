//! One door to the chat (ADR 0008 §3).
//!
//! The DM is the log of truth: whatever a teammate gives the person reaches
//! it, wherever the teammate was when it said so. Anything not said in a turn
//! of the DM itself comes in through [`Room::post`] — a call's summary or what
//! the call was asked to put in the chat, a job's report or reminder, a file
//! sent from a work thread — as one `agent` message stamped with where it
//! came from, so a client draws "From the call" from the stamp rather than
//! from words in the text.
//!
//! An agent asks for it with `post_to_chat`, one of Hotline's own tools, so an
//! ACP child reaches it over the desk's MCP server and Hotline Agent through
//! the same handler in process. The tool is listed for every session, because
//! a child's tool list is fixed when its session starts; whether a call may
//! post is decided here, outside the model, by the thread's kind and the
//! turn's modality ([`Room::post_to_chat`]). Every turn of a thread's agent
//! opens a [`Posting`] while it runs, and a call that lands outside one is
//! refused.

use super::{Room, Wired, lock, new_id, now_ms, timed};
use crate::contract::{
    Attachment, DeliveryFrom, ScheduledRun, ThreadId, ThreadKind, TranscriptEvent,
};
use std::sync::Arc;

/// The most one post to the chat may say; a longer account is a thread's to
/// keep, and the post can point at it.
pub(crate) const POST_CHARS: usize = 2_000;

/// What `post_to_chat` says when the DM's own turn asks: the person is
/// reading that turn's reply.
const LOUD: &str = "Your reply here is already the chat: say it in your reply.";

/// What `post_to_chat` says to a second call in one turn.
pub(super) const AGAIN: &str =
    "Already posted: one per turn. Put everything in that one next time.";

/// What a turn of a thread's agent may post, and whether it has.
pub(super) struct Posting {
    /// The job whose thread the turn is on: its posts carry the job's stamp.
    job: Option<ScheduledRun>,
    posted: bool,
    /// Voice detached while this turn was running; report once it finishes.
    pub(super) detached: bool,
    /// A file is a work result even when its closing words are short prose.
    pub(super) file_sent: bool,
}

/// The turn in flight on a thread, as [`Room::take_post`] finds it.
pub(super) enum Turn {
    /// None is.
    Closed,
    /// The DM's own turn, which the person reads: its reply is the chat.
    Loud,
    /// A turn whose one post has just been taken, with the job when it is a
    /// job's.
    Taken(Option<ScheduledRun>),
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
    /// The job, when a job posted it.
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
    /// held. `job` is the job when the thread is a job's. A thread takes one
    /// turn at a time, so the turn opened here is the thread's only one.
    pub(super) fn open_posting(
        &self,
        thread: &ThreadId,
        job: Option<ScheduledRun>,
    ) -> PostingTurn<'_> {
        lock(&self.posting).insert(
            thread.clone(),
            Posting {
                job,
                posted: false,
                detached: false,
                file_sent: false,
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
    /// refused, because its reply is already the chat. A subagent run is
    /// refused, because it reports to the agent that launched it, which
    /// decides what the person hears; a job, a subagent on a schedule that
    /// the person asked for, posts. One post a turn, and so one wake; the post goes to
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
        let scheduled = match self.take_post(thread)? {
            Turn::Closed => {
                return Err(
                    "post_to_chat posts from a turn in progress, and this thread has none."
                        .to_string(),
                );
            }
            Turn::Loud => return Err(LOUD.to_string()),
            Turn::Taken(scheduled) => scheduled,
        };
        let posted = self
            .post_and_wake(persona_id, thread, text, scheduled, wake)
            .await;
        if posted.is_err() {
            self.give_post_back(thread);
        }
        posted
    }

    /// Takes the one post of the turn in flight on `thread`, unless it is
    /// the DM's own turn, which posts nothing. A second is refused.
    pub(super) fn take_post(&self, thread: &ThreadId) -> Result<Turn, String> {
        let mut posting = lock(&self.posting);
        let Some(turn) = posting.get_mut(thread) else {
            return Ok(Turn::Closed);
        };
        if thread.kind == ThreadKind::Dm {
            return Ok(Turn::Loud);
        }
        if turn.posted {
            return Err(AGAIN.to_string());
        }
        turn.posted = true;
        Ok(Turn::Taken(turn.job.clone()))
    }

    /// Gives the turn in flight on `thread` back the post that never landed.
    pub(super) fn give_post_back(&self, thread: &ThreadId) {
        if let Some(turn) = lock(&self.posting).get_mut(thread) {
            turn.posted = false;
        }
    }

    /// Whether the turn in flight on `thread` has had its one post.
    pub(super) fn has_posted(&self, thread: &ThreadId) -> bool {
        lock(&self.posting)
            .get(thread)
            .is_some_and(|turn| turn.posted)
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
        let line = posted_since(
            &posts,
            "The last of these asks you to act on it now. Do, and tell the person what came of it.",
        );
        self.dispatch(session, Wired::words(timed(now_ms(), &line)));
        Ok("Posted to the chat, and your main conversation is acting on it.".to_string())
    }

    /// Writes `post` to the teammate's DM as the teammate's own message,
    /// `from` the thread it was said on, and says whether it landed.
    ///
    /// It wakes the person as a reply does, except from a call they are on.
    /// It does not wake the home agent: a post the person asked for (they
    /// placed the call, opened the work thread, scheduled the job) is for
    /// them, and only a thread that asks to wake it, through
    /// [`Room::post_to_chat`], hands it a turn. A live home session is
    /// handed what its other threads posted ahead of the person's next line,
    /// so a typed follow-up does not start cold, and a session that starts
    /// later reads it from the tape. A job's post, and a reminder's, which is
    /// the DM's own, are noted on the job for its listing.
    pub(crate) fn post(&self, persona_id: &str, from: &ThreadId, post: Post) -> Result<(), String> {
        let elsewhere = from.kind != ThreadKind::Dm;
        let job = post.scheduled.as_ref().map(|run| run.job_id.clone());
        let event = TranscriptEvent::Agent {
            id: post.id,
            ts: now_ms(),
            text: post.text,
            attachments: post.attachments,
            reactions: None,
            ring: None,
            receipt: None,
            spoken: None,
            silent: self.on_the_call(from).then_some(true),
            from: elsewhere.then(|| {
                let mut source = DeliveryFrom::new(from, None);
                source.title = self.link_of(from).map(|link| link.title).filter(|title| {
                    !(title.trim().is_empty() || from.kind == ThreadKind::Call && title == "Call")
                });
                source
            }),
            scheduled: post.scheduled,
        };
        let event = serde_json::to_value(&event).map_err(|error| error.to_string())?;
        self.try_write_value(persona_id, &event)
            .map_err(|error| format!("The chat could not be written to: {error}"))?;
        if let Some(attachments) = event["attachments"]
            .as_array()
            .filter(|items| !items.is_empty())
        {
            if let Some(turn) = lock(&self.posting).get_mut(from) {
                turn.file_sent = true;
            }
            if let Some(calls) = lock(&self.voice).upgrade() {
                for attachment in attachments {
                    calls.file_posted(from, attachment["name"].as_str().unwrap_or("sent file"));
                }
            }
        }
        let said = crate::sent::message_text(&event);
        if let Some(job) = &job {
            self.job_posted(job, &said);
        }
        if (elsewhere || job.is_some())
            && let Ok(session) = self.session(persona_id)
        {
            let origin = posted_from(&event).unwrap_or_default();
            let mut posts = lock(&session.pending_posts);
            posts.push(format!("{origin}: {said}"));
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
    pub(super) fn on_the_call(&self, thread: &ThreadId) -> bool {
        lock(&self.voice)
            .upgrade()
            .is_some_and(|voice| voice.thread_is_up(thread))
    }
}

/// What `post_to_chat` says to a subagent run.
pub(crate) const SUBAGENT: &str = "A subagent reports to the teammate that launched it, which \
decides what the person hears: put it in your report.";

/// `line` for a live home session, behind what its other threads posted to the
/// chat since it last spoke there: the posts are in the chat, not in its
/// context.
pub(super) fn posted_since(posts: &[String], line: &str) -> String {
    format!(
        "(What you posted to this chat from elsewhere since you last spoke here:\n{})\n\n{line}",
        posts.join("\n")
    )
}

/// Where a post on the tape came from, as the teammate is told it: "From
/// your call". `None` for a message the DM's own turn said.
pub(super) fn posted_from(event: &serde_json::Value) -> Option<String> {
    let field = |key: &str| event.get(key).cloned().unwrap_or_default();
    if let Ok(run) = serde_json::from_value::<ScheduledRun>(field("scheduled")) {
        return Some(format!(
            "From your scheduled job {} (thread job:{})",
            run.name, run.job_id
        ));
    }
    let from = serde_json::from_value::<DeliveryFrom>(field("from")).ok()?;
    Some(format!(
        "{} (thread {}:{})",
        match from.kind {
            ThreadKind::Dm => return None,
            ThreadKind::Call =>
                "From your call; this thread IS its transcript — read_thread reads it in full",
            ThreadKind::Side => "From your work thread",
            ThreadKind::Pair => "From your thread with a colleague",
            ThreadKind::Run => "From your subagent",
            ThreadKind::Job => "From your job",
        },
        from.kind.name(),
        from.thread
    ))
}
