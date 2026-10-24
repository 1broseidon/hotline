//! Words into a session: the chapter a message lands in, the prompt funnel,
//! deliveries, nudges and retries, and how driver updates become tape events.

use super::*;

impl Room {
    /// The session this message goes to, which is not always the one that was
    /// running.
    ///
    /// A chapter closes while nobody is being spoken to — on idle, on request,
    /// or because the agent asked — and the session that belonged to it still
    /// has the whole of it in its context. So the message before it reaches
    /// the agent is where the swap happens: the old session stops, a fresh one
    /// starts, and starting it opens the chapter this message will land in. A
    /// running session with an open chapter is left alone, which is every
    /// message but the first of a chapter. A chapter that has gone quiet past
    /// the room's idle window closes here first, so the message that ends a
    /// long quiet is the first of the next chapter, not the last of the old.
    ///
    /// The whole of that decision is behind the teammate's start gate. Which
    /// session is running and which chapter it is in are only true together,
    /// and the swap is a stop and a start with a gap in the middle: two
    /// messages arriving on a closed chapter would each perform it, and the
    /// second stop would cancel the session the first had just brought up.
    /// Behind the gate the second message finds the chapter the first opened
    /// and joins it.
    ///
    /// The gate comes back with the session, and the caller holds it until
    /// the message has claimed its turn: a computer swap between turns takes
    /// the same gate, so it can never restart a session under a message that
    /// was just admitted to it.
    pub(super) async fn in_this_chapter(
        self: &Arc<Self>,
        persona_id: &str,
    ) -> Result<(Arc<Session>, tokio::sync::OwnedMutexGuard<()>), String> {
        let held = self.start_gate(persona_id).lock_owned().await;
        // A message after a long quiet starts the next chapter: the stale one
        // closes before the line is written, so the line lands in the chapter
        // it begins rather than in the one the idle sweep would otherwise close
        // under it, taking the agent's memory of the reply along.
        if self.went_quiet(persona_id, now_ms()) && !self.mid_turn(persona_id) {
            self.close_chapter(persona_id, ChapterClose::Idle).await;
        }
        let session = self.session(persona_id)?;
        // Hotline Agent that came up with no provider said so on its tape. A
        // message is refused while that stands, so the words go back to the
        // composer; once a provider is connected, the session starts again
        // on it and the message goes to that one.
        if session.backend_id == HOTLINE_BACKEND_ID && session.driver.startup_failure().is_some() {
            if self.keys.provider_auth().is_empty() {
                return Err(crate::driver::failure::NO_PROVIDER.to_string());
            }
            let capability = self.stop_with_capability(persona_id);
            self.start_now(persona_id, capability).await?;
            return Ok((self.session(persona_id)?, held));
        }
        if self
            .log
            .chapter_activity(&StreamId::Tape(persona_id.to_string()))
            .0
            .is_some()
        {
            return Ok((session, held));
        }
        let capability = self.stop_with_capability(persona_id);
        self.start_now(persona_id, capability).await?;
        Ok((self.session(persona_id)?, held))
    }

    /// Hands the teammate a message and returns at once: the turn runs on its
    /// own task and everything it does arrives as tape events and deltas. A
    /// message sent during a turn steers when the driver supports active input.
    ///
    /// `reply_to` is the id of the message this one answers, and the
    /// attachments are files the teammate is handed alongside the words.
    pub async fn prompt(
        self: &Arc<Self>,
        persona_id: &str,
        text: &str,
        reply_to: Option<String>,
        attachments: Option<Vec<Attachment>>,
    ) -> Result<(), String> {
        let _working = self.working()?;
        if let Some(answered) = &reply_to
            && let Some(from) = self
                .tape(persona_id)
                .iter()
                .find(|event| event["id"] == *answered)
                .and_then(|event| {
                    serde_json::from_value::<crate::contract::DeliveryFrom>(event["from"].clone())
                        .ok()
                })
            && matches!(
                from.kind,
                crate::contract::ThreadKind::Call | crate::contract::ThreadKind::Side
            )
        {
            let thread = from.thread();
            if self
                .link_of(&thread)
                .is_some_and(|link| link.persona_id.as_deref() == Some(persona_id))
            {
                return self
                    .prompt_side_replying(&thread.key, text, None, attachments)
                    .await;
            }
        }
        let (session, _held) = self.in_this_chapter(persona_id).await?;
        self.heard_from_person(persona_id);
        if let Some(answered) = reply_to {
            mark(&session.pending_reply, answered);
        }
        // An empty list is no list: the record should not carry a field
        // saying nothing was attached.
        let attachments = attachments.filter(|attachments| !attachments.is_empty());
        self.say(
            &session,
            Sending {
                shown: text.to_string(),
                wire: Wired {
                    text: self.with_person_name(text),
                    attachments: attachments.clone().unwrap_or_default(),
                    steer: !crate::wire::commands::from_voice(),
                    said: None,
                    from: None,
                    voice: None,
                    spoken: crate::wire::commands::from_voice(),
                    unprompted: None,
                    retry: false,
                    wake: false,
                    request: None,
                },
                attachments,
            },
        )
        .await;
        Ok(())
    }

    pub(super) fn handoff_queued(&self, persona_id: &str, id: &str) -> bool {
        self.session(persona_id).is_ok_and(|session| {
            lock(&session.unread).iter().any(|v| v == id)
                || lock(&session.turns)
                    .waiting
                    .iter()
                    .any(|w| w.said.as_deref() == Some(id))
        })
    }

    /// Something for a teammate, into its own conversation: written to its
    /// tape first, so it survives a restart and is heard once, and then handed
    /// to the driver behind the turn in flight, or on a turn of its own if
    /// there is none. It never cuts into a turn, and a teammate that is not
    /// running is started for it. `request` is the ask its turn answers.
    pub(super) async fn deliver_identified(
        self: &Arc<Self>,
        persona_id: &str,
        id: &str,
        cause: DeliveryCause,
        text: String,
        request: Option<turns::RequestLine>,
    ) -> Result<(), String> {
        let _working = self.working()?;
        if self.handoff_queued(persona_id, id) {
            return Ok(());
        }
        let existing = self.tape(persona_id).into_iter().find(|v| v["id"] == id);
        if existing.as_ref().is_some_and(|v| v["receipt"] == "read") {
            return Ok(());
        }
        self.start(persona_id).await?;
        let (session, _held) = self.in_this_chapter(persona_id).await?;
        if lock(&session.dispatched_deliveries).contains(id) {
            return Ok(());
        }
        let ts = existing
            .as_ref()
            .and_then(|v| v["ts"].as_i64())
            .unwrap_or_else(now_ms);
        let wire = peers::delivery_wire(&cause, &text);
        let from = self.delivery_source(persona_id, &cause);
        if existing.is_none() {
            let event = TranscriptEvent::Delivery {
                id: id.into(),
                ts,
                from: Some(from.clone()),
                cause,
                text,
                receipt: Some(Receipt::Sent),
            };
            let value = serde_json::to_value(event).map_err(|e| e.to_string())?;
            self.log
                .append(&StreamId::Tape(persona_id.into()), &value)
                .map_err(|e| e.to_string())?;
        }
        let mut wired = Wired::words(timed(ts, &wire));
        wired.said = Some(id.into());
        wired.from = Some(from);
        wired.request = request;
        lock(&session.dispatched_deliveries).insert(id.into());
        self.dispatch(session, wired);
        Ok(())
    }

    /// Hotline's own words to a running teammate — a reopened chapter told what
    /// was said while it was away, never something a person typed.
    ///
    /// Queued like a prompt and never written down: the driver hears it, the
    /// record does not.
    pub fn nudge(self: &Arc<Self>, persona_id: &str, text: &str) -> Result<(), String> {
        let _working = self.working()?;
        let session = self.session(persona_id)?;
        self.dispatch(session, Wired::words(timed(now_ms(), text)));
        Ok(())
    }

    /// The person's Retry on a turn that failed: the same line goes to the
    /// driver again, and nothing new is written for it, so the conversation
    /// keeps the one message and its failure and gains only the new reply.
    /// Only the latest turn, when it failed and nothing was said after it,
    /// can be retried.
    pub async fn retry(self: &Arc<Self>, persona_id: &str) -> Result<(), String> {
        let Some(line) = failed_line(&self.tape(persona_id)) else {
            return Err("There is no failed turn to retry.".into());
        };
        let _working = self.working()?;
        self.start(persona_id).await?;
        let (session, _held) = self.in_this_chapter(persona_id).await?;
        self.dispatch(
            session,
            Wired {
                text: timed_from(line.ts, line.client, &line.text),
                attachments: line.attachments,
                retry: true,
                ..Wired::words(String::new())
            },
        );
        Ok(())
    }

    /// The user's line onto the tape, and then the driver's turn. The order is
    /// the invariant: what was said is a fact the moment somebody said it, and
    /// a turn that fails must not lose the message that started it.
    async fn say(self: &Arc<Self>, session: &Arc<Session>, sending: Sending) {
        // Desk calls retain their provenance until task-203 replaces the dispatcher.
        let id = if crate::wire::commands::from_voice() {
            format!("voice:{}", new_id())
        } else {
            new_id()
        };
        if let Some(attachments) = &sending.attachments {
            let root = self.log.root().to_path_buf();
            let persona_id = session.persona_id.clone();
            let event_id = id.clone();
            let attachments = attachments.clone();
            // Keep the bytes at send time, never on the phone's first read:
            // the source path may hold a different picture by then.
            if let Err(error) = tokio::task::spawn_blocking(move || {
                crate::sent::retain_user_images(&root, &persona_id, &event_id, &attachments);
            })
            .await
            {
                eprintln!("Could not retain user images for {id}: {error}");
            }
        }
        let ts = now_ms();
        let client = crate::wire::commands::prompt_client();
        self.append(
            session,
            TranscriptEvent::User {
                id: id.clone(),
                ts,
                text: sending.shown,
                attachments: sending.attachments,
                reactions: None,
                reply_to: None,
                scheduled: None,
                ring: None,
                receipt: Some(Receipt::Sent),
                client,
            },
        );
        let mut wire = sending.wire;
        let posts = std::mem::take(&mut *lock(&session.pending_posts));
        if !posts.is_empty() {
            wire.text = super::post::posted_since(&posts, &wire.text);
        }
        wire.text = timed_from(ts, client, &wire.text);
        wire.said = Some(id);
        self.dispatch(session.clone(), wire);
    }

    /// Stamps every line the driver has taken as read, now that the agent has
    /// produced something with it in context. Nothing un-reads.
    pub(super) fn mark_read(&self, session: &Session) {
        let taken: Vec<String> = std::mem::take(&mut *lock(&session.unread));
        if taken.is_empty() {
            return;
        }
        let tape = self.tape(&session.persona_id);
        for id in taken {
            let Some(line) = tape.iter().rev().find(|event| event["id"] == id.as_str()) else {
                continue;
            };
            let Ok(event) = serde_json::from_value::<TranscriptEvent>(line.clone()) else {
                continue;
            };
            self.write(&session.persona_id, &peers::stamped(event, Receipt::Read));
        }
    }

    /// The agent's reaction to what the person last said: one emoji on that
    /// message, never a message of its own. A reaction the message already
    /// carries is left as it is.
    pub fn react(&self, persona_id: &str, emoji: &str) -> Result<(), String> {
        let emoji = emoji.trim();
        if emoji.is_empty() || emoji.chars().count() > 4 || emoji.chars().any(char::is_alphanumeric)
        {
            return Err("react needs one emoji.".to_string());
        }
        let session = self.session(persona_id)?;
        let tape = self.tape(&session.persona_id);
        let line = tape
            .iter()
            .rev()
            .find(|event| event["kind"] == "user" && event.get("scheduled").is_none())
            .ok_or_else(|| "There is no message from the person to react to.".to_string())?;
        let TranscriptEvent::User {
            id,
            ts,
            text,
            attachments,
            reactions,
            reply_to,
            scheduled,
            ring,
            receipt,
            client,
        } = serde_json::from_value::<TranscriptEvent>(line.clone())
            .map_err(|error| format!("The last message could not be read: {error}"))?
        else {
            return Err("There is no message from the person to react to.".to_string());
        };
        let mut reactions = reactions.unwrap_or_default();
        if reactions.iter().any(|had| had == emoji) {
            return Ok(());
        }
        reactions.push(emoji.to_string());
        self.write(
            &session.persona_id,
            &TranscriptEvent::User {
                id,
                ts,
                text,
                attachments,
                reactions: Some(reactions),
                reply_to,
                scheduled,
                ring,
                receipt,
                client,
            },
        );
        Ok(())
    }

    /// Opens a web link in the person's own browser on this computer. Only
    /// while they are here: their last message came from the desktop app, so
    /// a tab never opens on a screen nobody is looking at.
    pub async fn open_link(&self, persona_id: &str, link: &str) -> Result<String, String> {
        let link = url::Url::parse(link.trim())
            .ok()
            .filter(|link| matches!(link.scheme(), "http" | "https"))
            .ok_or("open_link needs a full http or https link.")?;
        let session = self.session(persona_id)?;
        let here = self
            .tape(&session.persona_id)
            .iter()
            .rev()
            .find(|event| event["kind"] == "user" && event.get("scheduled").is_none())
            .is_some_and(|event| event.get("client").and_then(Value::as_str) == Some("desktop"));
        if !here {
            return Err("The person is not at this computer, so nothing was opened. Put the link in your reply instead.".into());
        }
        open_in_browser(link.as_str()).await?;
        Ok("Opened in the person's browser on this computer.".into())
    }

    /// Hands the driver a line: on the turn in flight if there is one, on a
    /// new turn if there is not.
    pub(super) fn dispatch(self: &Arc<Self>, session: Arc<Session>, wire: Wired) {
        let working = self
            .lease()
            .expect("a dispatch caller holds a read lease, so restart cannot hold the write lease");
        // Joining the queue and claiming an idle driver are one decision under
        // one lock, so a line can never be filed behind a turn that has
        // already stopped coming back for it.
        let Some(wire) = lock(&session.turns).claim(wire) else {
            session.input_ready.notify_one();
            return;
        };
        let room = self.clone();
        tokio::spawn(async move {
            let _working = working;
            room.run_queue(session, wire).await;
        });
    }

    /// The person's Stop of the turn in flight: as [`Room::cut_turn`], and a
    /// question the turn was answering ends as their stop of it.
    pub fn cancel(&self, persona_id: &str) -> Result<(), String> {
        let session = self.session(persona_id)?;
        session
            .stopped_by_person
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.cut_turn(&session, false);
        Ok(())
    }

    /// The person's Interrupt of the turn in flight, to say something
    /// instead: as [`Room::cut_turn`], and a question the turn was answering
    /// ends with that said, for its asker to ask again if it still needs to,
    /// never as their stop of it. The main conversation has no thread to
    /// carry the answer on, and the person's next line must never become it.
    pub fn interrupt(&self, persona_id: &str) -> Result<(), String> {
        let session = self.session(persona_id)?;
        session
            .interrupted_by_person
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.cut_turn(&session, true);
        Ok(())
    }

    /// Stops the turn in flight and drops whatever was waiting behind it,
    /// a colleague's question included, which fails to its asker. A question
    /// the turn was answering ends as one whose turn ended without a result,
    /// unless the person asked for the cut. A first-contact card involving
    /// the teammate expires with the cut, unless it is a `redirect`, the
    /// person's Interrupt, which decides nothing about a colleague.
    pub(super) fn cut_turn(&self, session: &Session, redirect: bool) {
        let persona_id = session.persona_id.as_str();
        let dropped: Vec<Wired> = lock(&session.turns).waiting.drain(..).collect();
        lock(&session.unread).clear();
        lock(&session.dispatched_deliveries).clear();
        session.driver.cancel();
        if redirect {
            self.settle_permissions_of_a_redirect(persona_id);
        } else {
            self.settle_collaboration(persona_id);
            self.settle_permissions(persona_id);
        }
        self.fail_dropped_questions(
            persona_id,
            dropped
                .iter()
                .filter_map(|line| line.request.as_ref())
                .map(|line| line.request.as_str()),
        );
    }
}

/// What the teammate and its agent have said to each other in the chapter the
/// agent is joining, for a driver that starts back into the conversation.
///
/// A chapter is one working context, so a session hears its own chapter and
/// not the ones before it: the wake block is what carries those. A tape with
/// no marker at all was written before this room divided anything, and reads
/// as one implicit chapter. A chapter that reopened an earlier one is that
/// earlier stretch plus anything said since, because Hotline Agent has no
/// checkpoint and the tape is its memory of the work.
///
/// The model said one thing; the tape may show it as several bubbles. Consecutive
/// agent events collapse back into one [`Said::Agent`], so the model sees one
/// thing again. A reply said on a call is shown to it as both its versions
/// ([`voiced`]). The Rig history is built from this, not from the tape, so it
/// does not need a second fold.
pub(super) fn said(events: &[Value]) -> Vec<Said> {
    let within: Vec<&Value> = match chapter_view::open_chapter(events) {
        Some(open) => {
            let mut lines = Vec::new();
            if let Some(from) = open.get("resumedFrom").and_then(Value::as_str)
                && let Some(origin) = events
                    .iter()
                    .find(|event| event.get("id").and_then(Value::as_str) == Some(from))
            {
                lines.extend(chapter_view::slice_of(events, origin));
            }
            lines.extend(chapter_view::slice_of(events, open));
            lines
        }
        None if chapter_view::chapters_of(events).is_empty() => events.iter().collect(),
        None => Vec::new(),
    };
    // A file sent without a caption is a message with no words; the model
    // remembers it by name.
    fold_said(voiced(within.iter().filter_map(|event| {
        event.get("text")?.as_str()?;
        let text = crate::sent::message_text(event);
        let spoken = event
            .get("spoken")
            .and_then(Value::as_str)
            .map(str::to_string);
        let said = match event.get("kind")?.as_str()? {
            "user" => Some(Said::User(match event.get("ts").and_then(Value::as_i64) {
                Some(ts) => timed_from(
                    ts,
                    event
                        .get("client")
                        .and_then(|client| serde_json::from_value(client.clone()).ok()),
                    &text,
                ),
                None => text,
            })),
            // A post from another thread is remembered with where it came
            // from, which its text no longer says.
            "agent" => Some(Said::Agent(match super::post::posted_from(event) {
                Some(origin) => format!("({origin}) {text}"),
                None => text,
            })),
            // What came back is remembered as it was heard.
            "delivery" => match serde_json::from_value((*event).clone()).ok()? {
                TranscriptEvent::Delivery {
                    ts,
                    cause,
                    text,
                    receipt,
                    ..
                } => {
                    // Queued work is not history: replay must pass exchange
                    // admission, so a restart cannot resurrect a stopped request.
                    if receipt != Some(Receipt::Read)
                        && matches!(
                            cause,
                            DeliveryCause::Handoff { .. }
                                | DeliveryCause::Question { .. }
                                | DeliveryCause::Peer {
                                    request_id: Some(_),
                                    ..
                                }
                        )
                    {
                        return None;
                    }
                    Some(Said::User(timed(ts, &peers::delivery_wire(&cause, &text))))
                }
                _ => None,
            },
            _ => None,
        }?;
        Some((said, spoken))
    })))
}

/// A reply said on a call, as the model is shown it again: its spoken
/// version and its written one, tagged as the call asked for them, so a
/// follow-up or a barge-in knows what was heard and what was only read. The
/// spoken version is kept on the reply's first bubble, and the bubbles after
/// it are the rest of the written version.
fn voiced(lines: impl IntoIterator<Item = (Said, Option<String>)>) -> Vec<Said> {
    let mut replies: Vec<(Said, Option<String>)> = Vec::new();
    for (line, spoken) in lines {
        match (replies.last_mut(), line) {
            (Some((Said::Agent(written), Some(_))), Said::Agent(next)) if spoken.is_none() => {
                written.push_str("\n\n");
                written.push_str(&next);
            }
            (_, line) => replies.push((line, spoken)),
        }
    }
    replies
        .into_iter()
        .map(|reply| match reply {
            (Said::Agent(written), Some(spoken)) => {
                Said::Agent(crate::voice::spoken::both(&spoken, &written))
            }
            (line, _) => line,
        })
        .collect()
}

/// Consecutive agent events are one thing the model said, shown as several
/// bubbles. Fold here so a teammate's tape and a peer thread put the pieces
/// back the same way.
pub(super) fn fold_said(lines: impl IntoIterator<Item = Said>) -> Vec<Said> {
    let mut out = Vec::new();
    for line in lines {
        match (out.last_mut(), line) {
            (Some(Said::Agent(already)), Said::Agent(next)) => {
                already.push_str("\n\n");
                already.push_str(&next);
            }
            (_, line) => out.push(line),
        }
    }
    out
}

/// What one driver update is, written down.
///
/// The one place an update becomes events, because a teammate's tape and a
/// peer thread must record the same turn the same way — the shapes are the
/// previous edition's, and there is nowhere for a second copy of them to drift
/// to. An empty vec is the one update that is never written: a delta, which
/// the message that follows it makes durable. An agent's message is split
/// into bubbles here so both kinds of agent and a peer thread get the same
/// ones. What is written of a message is its written version, without a tag
/// of the two versions a call asks for, whenever they come
/// ([`crate::voice::spoken`]); on a turn said on a call (`on_call`), the
/// reply's first bubble also keeps the version written to be heard.
pub(super) fn event_of(
    update: Update,
    in_flight: &mut HashMap<String, PendingTool>,
    on_call: bool,
    keep_written: bool,
) -> Vec<TranscriptEvent> {
    match update {
        Update::Chapter { boundary } => {
            boundary.finish(None);
            Vec::new()
        }
        Update::Delta { .. } | Update::Parked => Vec::new(),
        Update::Message { kind, id, text } => {
            let mut versions = crate::voice::spoken::versions(&text);
            if on_call
                && !keep_written
                && !crate::voice::spoken::carries_more(&versions.written)
                && let Some(spoken) = &versions.spoken
            {
                versions.written.clone_from(spoken);
                versions.lazy = None;
            }
            match kind {
                MessageKind::Agent => {
                    let mut spoken = versions.spoken.filter(|_| on_call);
                    if spoken.is_some()
                        && let Some(reason) = versions.lazy
                    {
                        eprintln!("[voice] the written version of {id} {reason}");
                    }
                    let ts = now_ms();
                    pacing::paced(&versions.written)
                        .into_iter()
                        .enumerate()
                        .map(|(i, text)| TranscriptEvent::Agent {
                            id: if i == 0 {
                                id.clone()
                            } else {
                                format!("{id}-{}", i + 1)
                            },
                            ts,
                            text,
                            attachments: None,
                            reactions: None,
                            ring: None,
                            receipt: None,
                            spoken: spoken.take(),
                            silent: None,
                            from: None,
                            scheduled: None,
                        })
                        .collect()
                }
                MessageKind::Thought => vec![TranscriptEvent::Thought {
                    id,
                    ts: now_ms(),
                    text: versions.written,
                }],
            }
        }
        Update::ToolFiles { call_id, files } => {
            if let Some(pending) = in_flight.get_mut(&call_id) {
                pending.files = Some(files);
            }
            Vec::new()
        }
        Update::ToolCall {
            call_id,
            title,
            kind,
        } => {
            let pending = PendingTool {
                ts: now_ms(),
                files: None,
                title,
                kind,
            };
            let event = pending.event(&call_id, ToolStatus::InProgress, None);
            in_flight.insert(call_id, pending);
            vec![event]
        }
        Update::ToolResult {
            call_id,
            ok,
            output,
            images,
        } => {
            let Some(pending) = in_flight.remove(&call_id) else {
                return Vec::new();
            };
            let status = if ok {
                ToolStatus::Completed
            } else {
                ToolStatus::Failed
            };
            let output = ToolOutput::Text {
                text: clip(&output, TOOL_OUTPUT_CHARS),
            };
            let mut events = vec![pending.event(&call_id, status, Some(output))];
            if crate::computer::is_computer_tool(&pending.kind, &pending.title) {
                for image in images {
                    events.push(TranscriptEvent::ComputerFrame {
                        id: new_id(),
                        ts: now_ms(),
                        data_url: image.data_url(),
                    });
                }
            }
            events
        }
        Update::Permission {
            request_id,
            title,
            options,
        } => vec![TranscriptEvent::Permission {
            from: None,
            // The card is superseded by this id when it is answered, so the
            // decision lands on the line already drawn rather than adding a
            // second one below it.
            id: format!("perm:{request_id}"),
            ts: now_ms(),
            request_id,
            title,
            options,
            decision: None,
            decided_option_name: None,
        }],
        Update::Turn { stop_reason, usage } => vec![TranscriptEvent::Turn {
            id: new_id(),
            ts: now_ms(),
            stop_reason,
            usage,
        }],
        Update::Notice { level, text } => vec![TranscriptEvent::Notice {
            id: new_id(),
            ts: now_ms(),
            level,
            text,
        }],
    }
}

/// A tool call the agent has made and not yet heard back about. The tape event
/// is written again by id when it does, so the call's own timestamp and title
/// have to outlive the call.
pub(super) struct PendingTool {
    ts: i64,
    title: String,
    pub(super) kind: String,
    files: Option<Vec<crate::contract::ChangedFile>>,
}

impl PendingTool {
    pub(super) fn event(
        &self,
        call_id: &str,
        status: ToolStatus,
        output: Option<ToolOutput>,
    ) -> TranscriptEvent {
        TranscriptEvent::Tool {
            id: format!("tool:{call_id}"),
            ts: self.ts,
            tool_call_id: call_id.to_string(),
            title: self.title.clone(),
            tool_kind: Some(self.kind.clone()),
            status,
            locations: None,
            files: self.files.clone(),
            output: output.map(|output| vec![output]),
        }
    }
}

/// What the tape writes down in place of the event the room handed it: a
/// person's line, with the message it answers.
pub(super) fn stamped(session: &Session, mut event: TranscriptEvent, now: i64) -> TranscriptEvent {
    if let TranscriptEvent::User { reply_to, .. } = &mut event {
        *reply_to = take_fresh(&session.pending_reply, now);
    }
    event
}
