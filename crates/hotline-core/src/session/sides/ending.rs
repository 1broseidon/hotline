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

    pub(in crate::session) fn end_side(&self, side: &Arc<LiveSide>, ending: Ending) {
        if !side.closed.claim(&ending) {
            return;
        }
        self.carry_out(side, ending);
    }

    /// Lets go of a thread's agent only if it is idle, and says whether it is
    /// let go of (or already gone). The look and the claim of the ending are
    /// one step under the thread's turn lock, so a line that began a turn
    /// first is never cut by a park decided on a look taken before it, and
    /// one that comes after finds the thread ending and reopens it
    /// ([`Room::queue_in_side`]). `quiet_since` also refuses a thread said to
    /// since that time.
    pub(in crate::session) fn park_if_idle(
        &self,
        side: &Arc<LiveSide>,
        quiet_since: Option<i64>,
    ) -> bool {
        {
            let turns = lock(&side.turns);
            if turns.running {
                return false;
            }
            if quiet_since.is_some_and(|cutoff| side.last_used() > cutoff) {
                return false;
            }
            if !side.closed.claim(&Ending::Park) {
                return true;
            }
        }
        self.carry_out(side, Ending::Park);
        true
    }

    /// What an ending claimed does. The thread stays in the room's map, ending,
    /// until its link says where it went, so nothing that asks sees a live
    /// link with no thread behind it, or a thread with a link that still says
    /// live: whoever needs it settled waits for [`Sides::ended`].
    fn carry_out(&self, side: &Arc<LiveSide>, ending: Ending) {
        // A colleague's question still waiting here fails to its asker.
        let dropped: Vec<Line> = lock(&side.turns).waiting.drain(..).collect();
        self.fail_dropped_lines(&side.persona_id, dropped);
        // A parked thread leaves what an Interrupt left it carrying to the
        // agent that brings it back: only a close ends that, and the
        // person's End thread ends it as theirs (`Room::person_ended`).
        if let Ending::Close(by, _) = &ending
            && *by != End::Person
        {
            self.end_carried(&side.id, side.request().as_deref());
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
        self.sides.at("ending");
        let mut note = None;
        match ending {
            Ending::Park => self.mark_side(side, ThreadState::Parked, None),
            Ending::Close(by, result) => {
                let said_so = result.is_some();
                let result = result.or_else(|| last_words(&events));
                self.mark_side(side, ThreadState::Closed(by), result);
                note = Some(said_so);
            }
            Ending::Restore(prior) => self.write_link(&prior),
        }
        // Only now does the thread leave the map: its link and its place in
        // the room change together, as far as anyone asking can tell.
        self.sides.leave(side);
        if let Some(said_so) = note {
            self.queue_closing_note(&side.thread.clone(), said_so);
        }
        let _ = self.info_changes.send(self.info(&side.persona_id));
    }

    /// Archives a thread that has no agent: a parked one ended, and with it
    /// what an Interrupt left it carrying, as a live thread's close ends it.
    /// Says whether it did: a thread that woke or was archived since `link`
    /// was read is left as it is now, and the caller looks again.
    pub(super) fn archive_parked(&self, link: Link, by: End, result: Option<String>) -> bool {
        let thread = link.thread.clone();
        let Some(persona_id) = link.persona_id else {
            return false;
        };
        let said_so = result.is_some();
        let result = result.or_else(|| {
            last_words(
                &self
                    .log
                    .load(&StreamId::of_thread(&thread).expect("work has a stream")),
            )
        });
        let archived = self.amend_link(&thread, |fresh| {
            (fresh.state == ThreadState::Parked && self.sides.get(&thread.key).is_none()).then(
                || Link {
                    state: ThreadState::Closed(by),
                    outcome: result,
                    at: Some(now_ms()),
                    ..fresh
                },
            )
        });
        if !archived {
            return false;
        }
        if by != End::Person {
            self.end_carried(&thread.key, None);
        }
        self.queue_closing_note(&thread, said_so);
        let _ = self.info_changes.send(self.info(&persona_id));
        true
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
        if state == ThreadState::Live && side.closed.is_set() {
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
            asked_in: side.asked_in.clone(),
            run_kind: None,
        });
    }

    /// Writes on each thread's link whether the delegation it serves still
    /// stands, where that has changed ([`Room::handoff_ended`]): a live
    /// thread's from its agent's record, one with no agent from its link.
    pub(in crate::session) fn mark_handoffs(&self, threads: &[String]) {
        for side_id in threads {
            let thread = self.work_thread(side_id);
            let Some(link) = self.link_of(&thread) else {
                continue;
            };
            let ended = link.opener.is_some() && self.handoff_ended(side_id);
            self.sides.at("handoffs");
            if link.handoff_ended == ended {
                continue;
            }
            if let Some(side) = self.sides.get(side_id) {
                self.mark_side(&side, ThreadState::Live, None);
            } else {
                // Changed on the link as it stands when it is written, not as
                // it was read: the thread may have woken or ended since.
                self.amend_link(&thread, |fresh| {
                    (fresh.handoff_ended != ended).then_some(Link {
                        handoff_ended: ended,
                        ..fresh
                    })
                });
                // It woke meanwhile: its own mark says where it stands.
                if let Some(side) = self.sides.get(side_id) {
                    self.mark_side(&side, ThreadState::Live, None);
                }
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
                            Some("human_action" | "credential_ask" | "cookie_ask")
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
