//! How a side thread ends: finished for good, parked to come back from its own
//! stream, or archived with a one-line result; and what the room keeps of it
//! afterwards, the session to reopen, the name, the marker and the summary.

use super::*;

impl Room {
    /// Ends a thread for good, until it is continued: its agent stopped, its
    /// authority revoked, its link closed. Safe to call twice.
    pub(super) fn finish_side(&self, side: &Arc<LiveSide>, by: End, result: Option<String>) {
        self.end_side(side, Ending::Close(by, result));
    }

    /// Stops a thread's turn and closes it: what the person pressing Stop on
    /// the exchange that opened it comes to. A thread that is not live is left
    /// as it is.
    pub(in crate::session) fn stop_work(&self, side_id: &str, outcome: &str) {
        let Some(side) = self.sides.get(side_id) else {
            return;
        };
        let dropped: Vec<Line> = lock(&side.turns).waiting.drain(..).collect();
        self.fail_dropped_lines(&side.persona_id, dropped);
        if let Some(driver) = side.driver() {
            driver.cancel();
        }
        self.finish_side(&side, End::Stopped, Some(outcome.to_string()));
    }

    /// Cancels the turn in flight in a live thread when it is the one
    /// answering `request`, and nothing else: the thread stays open, and the
    /// lines waiting behind the turn stay queued.
    pub(in crate::session) fn cancel_turn_answering(&self, side_id: &str, request: &str) {
        let Some(side) = self.sides.get(side_id) else {
            return;
        };
        if side.request().as_deref() != Some(request) {
            return;
        }
        if let Some(driver) = side.driver() {
            driver.cancel();
        }
    }

    /// The requests an interrupt left for the person's next line, in a
    /// thread closed before that line came, have no turn to answer them:
    /// each ends as one whose turn ended without a result. On a task of its
    /// own, since a stop ends a thread under the exchange lock this takes.
    fn end_unanswered(&self, requests: Vec<String>) {
        let Some(room) = lock(&self.me).upgrade() else {
            return;
        };
        tokio::spawn(async move {
            for request in requests {
                let reply = room.ended_without_result(&request).to_string();
                room.finish_request(&request, reply, true);
            }
        });
    }

    /// Lets go of a thread's agent and leaves the thread open.
    pub(super) fn park_side(&self, side: &Arc<LiveSide>) {
        self.end_side(side, Ending::Park);
    }

    pub(in crate::session) fn end_side(&self, side: &Arc<LiveSide>, ending: Ending) {
        if side.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        self.sides.remove(&side.id);
        let dropped: Vec<Line> = lock(&side.turns).waiting.drain(..).collect();
        self.fail_dropped_lines(&side.persona_id, dropped);
        // A parked thread leaves what it carries to the agent that brings it
        // back (`Room::carried_into`): a park ends no request.
        if matches!(ending, Ending::Close(..)) {
            let in_flight = side.request();
            let carried = self.carried_into(&side.id, in_flight.as_deref());
            if !carried.is_empty() {
                self.end_unanswered(carried);
            }
        }
        side.capability.revoke();
        // A thread ended while its agent is still starting has none to stop
        // here: the start sees that it is closed and stops it itself.
        if let Some(up) = side.up.get() {
            up.driver.invalidate();
            self.let_go_of_computer(&side.persona_id, &up.holder);
        }
        let stream = StreamId::of_thread(&side.thread).expect("work has a stream");
        let events = self.log.load(&stream);
        for expired in crate::log::expire_orphaned_permissions(&events, now_ms()) {
            self.threads()
                .write(&side.thread.clone(), &side.persona_id, &expired);
        }
        match ending {
            Ending::Park => self.mark_side(side, ThreadState::Parked, None),
            Ending::Close(by, result) => {
                let said_so = result.is_some();
                let result = result.or_else(|| last_words(&events));
                self.mark_side(side, ThreadState::Closed(by), result);
                self.queue_closing_note(&side.thread.clone(), said_so);
            }
        }
        let _ = self.info_changes.send(self.info(&side.persona_id));
    }

    /// Archives a thread that has no agent: the person ended a parked one.
    pub(super) fn archive_parked(&self, link: Link, by: End, result: Option<String>) {
        let thread = link.thread.clone();
        let Some(persona_id) = link.persona_id.clone() else {
            return;
        };
        let said_so = result.is_some();
        let result = result.or_else(|| {
            last_words(
                &self
                    .log
                    .load(&StreamId::of_thread(&thread).expect("work has a stream")),
            )
        });
        self.write_link(&Link {
            state: ThreadState::Closed(by),
            outcome: result,
            at: Some(now_ms()),
            ..link
        });
        self.queue_closing_note(&thread, said_so);
        let _ = self.info_changes.send(self.info(&persona_id));
    }

    /// Keeps the agent's session id on the marker once a turn has completed on
    /// it, the way a teammate's checkpoint is kept: that is what lets a parked
    /// or archived thread reopen with real recall. A turn that failed, or a
    /// session the driver says is broken, withdraws the promise instead.
    pub(super) fn remember_session(
        &self,
        side: &LiveSide,
        driven: &crate::session::runner::Driven,
    ) {
        let broken = side
            .driver()
            .is_none_or(|driver| !driver.checkpoint_valid())
            || driven.stop_reason.as_deref() == Some("failed");
        let next = if broken {
            None
        } else {
            lock(&side.reported).clone()
        };
        {
            let mut saved = lock(&side.saved);
            if *saved == next {
                return;
            }
            *saved = next;
        }
        self.mark_side(side, ThreadState::Live, None);
    }

    /// The link on the teammate's tape and at the head of the thread's own
    /// stream.
    /// An untitled thread takes its name from the first line said in it.
    pub(super) fn name_side(&self, side: &LiveSide, text: &str) {
        let title = title_of(text);
        {
            let mut current = lock(&side.title);
            if !current.is_empty() || title.is_empty() {
                return;
            }
            *current = title;
        }
        self.mark_side(side, ThreadState::Live, None);
        let _ = self.info_changes.send(self.info(&side.persona_id));
    }

    /// A live mark on a thread that has ended writes nothing: the end's own
    /// link stands.
    pub(super) fn mark_side(&self, side: &LiveSide, state: ThreadState, outcome: Option<String>) {
        let _marking = lock(&side.marking);
        if state == ThreadState::Live && side.closed.load(Ordering::SeqCst) {
            return;
        }
        let thread = side.thread.clone();
        let saved = lock(&side.saved).clone();
        self.write_link(&Link {
            id: self.link_id(&thread),
            ts: side.started,
            thread,
            persona_id: Some(side.persona_id.clone()),
            title: lock(&side.title).clone(),
            state,
            outcome,
            at: matches!(state, ThreadState::Closed(_)).then(now_ms),
            note: None,
            binding: saved.map(|session_id| AgentBinding {
                backend_id: side.backend_id.clone(),
                session_id,
            }),
            elapsed_ms: None,
            opener: side.opener.clone(),
            handoff_ended: side.opener.is_some() && self.handoff_ended(&side.id),
        });
    }

    /// Writes on each thread's link whether the delegation it serves still
    /// stands, where that has changed ([`Room::handoff_ended`]): a live
    /// thread's from its agent's record, one with no agent from its link.
    pub(in crate::session) fn mark_handoffs(&self, threads: &[String]) {
        for side_id in threads {
            let Some(link) = self.link_of(&self.work_thread(side_id)) else {
                continue;
            };
            let ended = link.opener.is_some() && self.handoff_ended(side_id);
            if link.handoff_ended == ended {
                continue;
            }
            match self.sides.get(side_id) {
                Some(side) => self.mark_side(&side, ThreadState::Live, None),
                None => self.write_link(&Link {
                    handoff_ended: ended,
                    ..link
                }),
            }
        }
    }

    pub(super) fn side_summary(&self, side: &LiveSide) -> SideThreadSummary {
        let events = self
            .log
            .load(&StreamId::of_thread(&side.thread).expect("work has a stream"));
        SideThreadSummary {
            side_id: side.id.clone(),
            persona_id: side.persona_id.clone(),
            title: lock(&side.title).clone(),
            status: SideStatus::Live,
            started_at: side.started,
            last_at: last_at(&events).max(side.started),
            working: side.turn_running(),
            waiting: waiting_on(&events),
            preview: preview_line(&events),
            result: None,
            archived_by: None,
            archived_at: None,
            opened_by: side.opener.clone().map(Into::into),
        }
    }

    /// Pending human cards remain reachable when a work or call thread parks.
    pub(in crate::session) fn cards_in_work(&self, persona_id: &str) -> Vec<(Value, ThreadId)> {
        crate::thread::ThreadStore::new(&self.log)
            .list(persona_id)
            .into_iter()
            .filter(|thread| {
                matches!(thread.id.kind, ThreadKind::Side | ThreadKind::Call)
                    && !matches!(thread.state, ThreadState::Closed(_))
            })
            .flat_map(|thread| {
                self.log
                    .load(&StreamId::of_thread(&thread.id).expect("a work thread has a stream"))
                    .into_iter()
                    .filter(|event| {
                        matches!(
                            event["kind"].as_str(),
                            Some("human_action" | "credential_ask")
                        )
                    })
                    .map(move |event| (event, thread.id.clone()))
            })
            .collect()
    }

    pub(super) fn stored_side_ids(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(crate::paths::sides_dir(self.log.root())) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                name.strip_suffix(".jsonl").map(str::to_string)
            })
            .collect()
    }

    /// A thread with no agent, as its link and stream say it is.
    pub(super) fn stored_summary(
        &self,
        side_id: &str,
        persona_id: &str,
    ) -> Option<SideThreadSummary> {
        let events = self.log.load(&StreamId::Side(side_id.to_string()));
        let link = Link::find(&events, &ThreadId::side(side_id))?;
        if link.persona_id.as_deref() != Some(persona_id) {
            return None;
        }
        // A stream whose link still says live belongs to a process that is
        // gone, and the next start parks it: it is never listed as live from
        // here.
        let (status, archived_by) = match link.state {
            ThreadState::Live => return None,
            ThreadState::Parked => (SideStatus::Parked, None),
            ThreadState::Closed(end) => (
                SideStatus::Archived,
                Some(match end {
                    End::Agent => SideEnd::Agent,
                    End::Idle => SideEnd::Idle,
                    End::Stopped | End::Failed | End::Cancelled => SideEnd::Stopped,
                    _ => SideEnd::Person,
                }),
            ),
        };
        Some(SideThreadSummary {
            side_id: side_id.to_string(),
            persona_id: persona_id.to_string(),
            title: link.title,
            status,
            started_at: link.ts,
            last_at: last_at(&events).max(link.ts),
            working: false,
            waiting: false,
            preview: preview_line(&events),
            result: link.outcome,
            archived_by,
            archived_at: link.at,
            opened_by: link.opener.map(Into::into),
        })
    }
}
