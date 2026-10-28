//! Starting a teammate's session and ending it: the start gate, the
//! capability lease a start runs under, and invalidating, stopping and
//! reattaching what is running.

use super::*;

impl Room {
    /// Brings a teammate up, on the driver its backend names. One caller at a
    /// time per teammate; see [`Room::starts`]. A teammate that is already up
    /// is the answer: the second caller — a schedule firing as the window
    /// starts the same teammate — gets the session that exists, not a second
    /// agent on the same tape that only one of them could ever stop.
    pub async fn start(self: &Arc<Self>, persona_id: &str) -> Result<SessionInfo, String> {
        let _working = self.working()?;
        let capability = self.capability_lease(persona_id);
        let gate = self.start_gate(persona_id);
        let _held = gate.lock().await;
        capability.check()?;
        if lock(&self.sessions).contains_key(persona_id) {
            self.session(persona_id)?;
            return Ok(self.info(persona_id));
        }
        self.start_now(persona_id, capability).await
    }

    /// This teammate's start gate, made the first time anyone starts it.
    pub(super) fn start_gate(&self, persona_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        lock(&self.starts)
            .entry(persona_id.to_string())
            .or_default()
            .clone()
    }

    pub(super) fn capability_epoch(&self, persona_id: &str) -> CapabilityEpoch {
        lock(&self.capabilities)
            .entry(persona_id.to_string())
            .or_default()
            .clone()
    }

    pub(super) fn capability_lease(&self, persona_id: &str) -> CapabilityLease {
        self.capability_epoch(persona_id).lease()
    }

    /// What a request between two teammates holds of one of them: its
    /// lasting authority, which a chapter's swap, a parked thread and a job's
    /// firing ending leave alone and a stop or a policy change ends, scoped
    /// so that ending the request ends it alone.
    pub(super) fn request_lease(&self, persona_id: &str) -> CapabilityLease {
        self.capability_epoch(persona_id).lasting().scoped()
    }

    /// Read-only tools bound to the call under the teammate's authority.
    /// The lease is the teammate's own, so changing its grants on the card
    /// refuses the front's next call.
    pub(crate) fn front_reach(
        self: &Arc<Self>,
        persona: &Persona,
        thread: &ThreadId,
    ) -> Result<FrontReach, String> {
        self.validate_call_thread(&persona.id, thread)?;
        let capability = self.capability_lease(&persona.id);
        Ok(FrontReach {
            tools: TeammateTools::new(self, persona.id.clone())
                .with_capability(capability.clone())
                .for_call(thread.clone())
                .for_front(),
            capability,
        })
    }

    pub(super) fn current_session(&self, candidate: &Arc<Session>) -> bool {
        lock(&self.sessions)
            .get(&candidate.persona_id)
            .is_some_and(|current| Arc::ptr_eq(current, candidate))
    }

    /// The start itself. The caller is holding this teammate's gate.
    ///
    /// The agent is the DM thread's, built and started by
    /// [`Room::thread_agent`] under the Dm policy. What is left here is the
    /// room's part: the session it publishes, the watches it hands the driver's
    /// streams to, and the chapter the conversation is in.
    pub(super) async fn start_now(
        self: &Arc<Self>,
        persona_id: &str,
        capability: CapabilityLease,
    ) -> Result<SessionInfo, String> {
        let _working = self.working()?;
        capability.check()?;
        let persona = self.persona(persona_id)?;
        capability.check()?;
        let mut agent = self
            .thread_agent(agent::Opening {
                thread: ThreadId::dm(&persona.id),
                persona,
                title: "the main conversation".to_string(),
                opener: None,
                lease: capability.clone(),
                heard_from: None,
            })
            .await?;
        let persona = agent.view.clone();
        let driver = agent.driver.clone();
        let has_computer = agent.computer;
        let reported = agent.started.clone();
        let (info_updates, unprompted, subagents, wakes) = match agent.watches.take() {
            Some(watches) => (
                watches.info,
                watches.unprompted,
                watches.subagents,
                watches.wakes,
            ),
            None => (None, None, None, None),
        };
        let mut info = idle_info(&persona.id);
        info.state = if driver.startup_failure().is_some() {
            SessionState::Error
        } else {
            SessionState::Ready
        };
        if let Some(text) = driver.startup_failure() {
            self.write(
                &persona.id,
                &TranscriptEvent::Notice {
                    id: new_id(),
                    ts: now_ms(),
                    level: NoticeLevel::Error,
                    text,
                },
            );
        }
        info.agent_name = Some(reported.agent_name);
        info.agent_version = reported.agent_version;
        info.session_id.clone_from(&reported.session_id);
        info.context_restored = reported.context_restored;
        info.models = reported.models;
        info.current_model_id = Some(reported.current_model_id);
        info.model_label = reported.model_label;
        info.modes = reported.modes;
        info.current_mode_id = reported.current_mode_id;
        info.mode_label = reported.mode_label;
        info.configs = reported.configs;
        info.capabilities = reported.capabilities;
        let session = Arc::new(Session {
            persona_id: persona.id.clone(),
            computer: has_computer,
            holder: agent.holder.clone(),
            capability: capability.clone(),
            backend_id: persona.backend_id.clone(),
            driver,
            info: Mutex::new(info.clone()),
            turns: Mutex::new(Turns::default()),
            stopped_by_person: std::sync::atomic::AtomicBool::new(false),
            interrupted_by_person: std::sync::atomic::AtomicBool::new(false),
            rebuild_for_sign_in: std::sync::atomic::AtomicBool::new(false),
            input_ready: Notify::new(),
            pending_reply: Mutex::new(None),
            pending_posts: Mutex::new(Vec::new()),
            unread: Mutex::new(Vec::new()),
            voice_origin: Mutex::new(None),
            dispatched_deliveries: Mutex::new(HashSet::default()),
            glance: Mutex::new(None),
            pending_checkpoint: Mutex::new(
                reported.session_id.filter(|_| !reported.context_restored),
            ),
        });
        {
            let _lifecycle = lock(&self.lifecycle);
            // Dropped, the agent is stopped: the session never joined the room.
            capability.check()?;
            lock(&self.sessions).insert(persona.id.clone(), session.clone());
            agent.keep();
            let _ = self.info_changes.send(info.clone());
        }
        if let Some(info_updates) = info_updates {
            self.watch_driver_info(&session, info_updates);
        }
        if let Some(unprompted) = unprompted {
            self.watch_unprompted(&session, unprompted);
        }
        if let Some(subagents) = subagents {
            self.watch_subagents(persona.id.clone(), subagents);
        }
        if let Some(wakes) = wakes {
            self.watch_wakes(&session, wakes);
        }
        if let Some(processes) = session.driver.processes() {
            self.watch_processes(&session, &processes);
        }
        // Nothing said is outside a chapter: a session that starts on a tape
        // whose last chapter is closed — or that has none at all — opens one.
        self.begin_chapter(&persona.id, &persona.backend_id);
        // Hotline draws the permission cards but does not decide whether the
        // agent sends the requests, and somebody who believes they are behind
        // a gate that is not there should be told.
        if let Some(text) = acp::containment_notice(&persona.backend_id) {
            self.write(
                &persona.id,
                &TranscriptEvent::Notice {
                    id: new_id(),
                    ts: now_ms(),
                    level: NoticeLevel::Warn,
                    text,
                },
            );
        }
        Ok(info)
    }

    /// Revokes a teammate's current generation before its replacement policy
    /// is written. Keeping the session record until reattach lets a live
    /// caller restart it, while the inactive epoch blocks starts that race
    /// inside the wire's invalidate-to-append window.
    pub fn invalidate(&self, persona_id: &str) -> Result<(), String> {
        self.persona(persona_id)?;
        self.forget_informed_collaboration(persona_id);
        {
            let _lifecycle = lock(&self.lifecycle);
            self.capability_epoch(persona_id).invalidate();
            let session = lock(&self.sessions).get(persona_id).cloned();
            if let Some(session) = &session {
                self.revoke_session(session);
            }
        }
        self.peers.expire_grants(persona_id);
        self.drop_sides(persona_id);
        self.let_go_of_jobs(Some(persona_id));
        self.revoke_exchanges(persona_id);
        self.settle_collaboration(persona_id);
        self.settle_permissions(persona_id);
        self.computers.mark_idle(persona_id, now_ms());
        Ok(())
    }

    /// The caller holds the lifecycle lock and has closed the epoch. Keep
    /// the record for reattachment, but never present revoked execution as
    /// ready if persisting its replacement policy fails.
    fn revoke_session(&self, session: &Arc<Session>) {
        lock(&session.turns).waiting.clear();
        session.driver.invalidate();
        let info = {
            let mut info = lock(&session.info);
            info.state = SessionState::Stopped;
            info.awaiting_subagents = false;
            // `invalidate` killed them.
            info.processes.clear();
            info.clone()
        };
        let _ = self.info_changes.send(info);
    }

    /// Revokes every live and cached session when room-wide infrastructure
    /// changes. All authorities remain quarantined until `reattach_all`
    /// activates the generations against the new settings.
    pub fn invalidate_all(&self) -> Result<(), String> {
        let roster_ids: Vec<String> = room::roster(&self.log)
            .into_iter()
            .map(|persona| persona.id)
            .collect();
        let sessions = {
            let _lifecycle = lock(&self.lifecycle);
            for persona_id in &roster_ids {
                self.capability_epoch(persona_id).invalidate();
            }
            let sessions: Vec<Arc<Session>> = lock(&self.sessions).values().cloned().collect();
            for session in &sessions {
                self.revoke_session(session);
            }
            sessions
        };
        for persona_id in &roster_ids {
            self.revoke_exchanges(persona_id);
        }
        self.peers.expire_all_grants();
        self.drop_all_sides();
        self.let_go_of_jobs(None);
        self.settle_all_collaboration();
        for session in sessions {
            self.settle_permissions(&session.persona_id);
            self.computers.mark_idle(&session.persona_id, now_ms());
        }
        Ok(())
    }

    /// A deleted teammate: its own session stopped, with the collaboration
    /// grants it held or was given, and its start gate let go, because nothing
    /// should wait behind — or be kept for — an id that names nobody any more.
    pub fn forget(&self, persona_id: &str) {
        let _ = self.stop(persona_id);
        lock(&self.capabilities).remove(persona_id);
        lock(&self.starts).remove(persona_id);
        let computers = self.computers.clone();
        let id = persona_id.to_string();
        tokio::spawn(async move {
            let _ = computers.remove(&id, None).await;
        });
    }

    /// Ends the session. The teammate keeps its tape; what stops is the agent.
    ///
    /// The person stopping a teammate ends its side threads with it: they are
    /// its agent, and nothing is left to answer in them. Its jobs' agents are
    /// let go of too; the jobs go on firing.
    pub fn stop(&self, persona_id: &str) -> Result<(), String> {
        *lock(&self.stops).entry(persona_id.to_string()).or_default() += 1;
        self.stop_session(persona_id);
        self.drop_sides(persona_id);
        self.let_go_of_jobs(Some(persona_id));
        Ok(())
    }

    /// The person's stop of the main session alone. It ends what the
    /// teammate's requests hold of it as well, which a chapter's swap leaves
    /// standing.
    pub(super) fn stop_session(&self, persona_id: &str) {
        self.stop_with_capability(persona_id);
        self.capability_epoch(persona_id).stop();
        self.revoke_exchanges(persona_id);
    }

    /// Replaces the main session, for a chapter swap: its handles end, and
    /// the requests the teammate sent or was given, which hold its lasting
    /// authority, go on. A side thread has a conversation of its own and
    /// goes on through one.
    ///
    /// The replacement authority is captured in the same critical section as
    /// stopping the old session, so a later stop also cancels a replacement
    /// that has not reached its first await yet.
    pub(super) fn stop_with_capability(&self, persona_id: &str) -> CapabilityLease {
        self.swap_out(persona_id).0
    }

    /// [`Self::stop_with_capability`], and whether a session was running.
    fn swap_out(&self, persona_id: &str) -> (CapabilityLease, bool) {
        let (session, capability) = {
            let _lifecycle = lock(&self.lifecycle);
            // A quarantined epoch stays closed, so a concurrent policy update
            // is not reopened by the swap.
            let epoch = self.capability_epoch(persona_id);
            epoch.replace_session();
            (lock(&self.sessions).remove(persona_id), epoch.lease())
        };
        self.peers.expire_grants(persona_id);
        let Some(session) = session else {
            self.settle_collaboration(persona_id);
            self.settle_permissions(persona_id);
            self.computers.mark_idle(persona_id, now_ms());
            return (capability, false);
        };
        lock(&session.turns).waiting.clear();
        session.driver.invalidate();
        self.settle_collaboration(persona_id);
        self.settle_permissions(persona_id);
        self.computers.mark_idle(persona_id, now_ms());
        let mut info = idle_info(persona_id);
        info.state = SessionState::Stopped;
        let _lifecycle = lock(&self.lifecycle);
        if !lock(&self.sessions).contains_key(persona_id) {
            let _ = self.info_changes.send(info);
        }
        (capability, true)
    }

    /// Rebuilds a live session from the teammate's current record, so a change
    /// to what it can use takes effect without waiting for the next start.
    ///
    /// Behind the start gate: two reattaches, or a reattach racing a chapter
    /// swap, would otherwise stop the session the other had just brought up.
    /// An old turn cannot delay the change: its handles are revoked before
    /// replacement, and its remaining updates cannot change the new session.
    pub async fn reattach(self: &Arc<Self>, persona_id: &str) -> Result<(), String> {
        let _working = self.working()?;
        let gate = self.start_gate(persona_id);
        let _held = gate.lock().await;
        self.reattach_held(persona_id).await
    }

    /// [`Self::reattach`] for a caller already holding the start gate.
    pub(super) async fn reattach_held(self: &Arc<Self>, persona_id: &str) -> Result<(), String> {
        let capability = {
            // Already behind the start gate, so taking the policy lock here
            // keeps the one order `start_gate -> policy_update_lock`. It also
            // makes this wait for any policy update in flight to finish its
            // own invalidate and durable append before the epoch is activated
            // and the log is read to start: a change half-written cannot be
            // started on. A second policy update arriving while this waits
            // invalidates under the lock and quarantines the epoch again, for
            // the reattach that update will run.
            let policy_gate = self.policy_update_lock();
            let _policy_held = policy_gate.lock().await;
            // The wire invalidates before appending. Repeating it also gives
            // direct callers the same stale-driver boundary.
            self.invalidate(persona_id)?;
            let _lifecycle = lock(&self.lifecycle);
            let was_running = lock(&self.sessions).remove(persona_id).is_some();
            let epoch = self.capability_epoch(persona_id);
            epoch.activate();
            self.schedule_changed.notify_one();
            was_running.then(|| epoch.lease())
        };
        let Some(capability) = capability else {
            return Ok(());
        };
        self.start_again(persona_id, capability).await
    }

    /// Rebuilds a running main session from the teammate's current record
    /// the way a chapter's swap does: the old session's handles end, and the
    /// teammate's threads, its requests and its authority stand. For what
    /// changes the session and not what the teammate may do: its computer
    /// joining or made again, or a sign-in to a server its policy grants.
    /// What narrows reach goes through [`Self::reattach`].
    pub(super) async fn rebuild(self: &Arc<Self>, persona_id: &str) -> Result<(), String> {
        let _working = self.working()?;
        let gate = self.start_gate(persona_id);
        let _held = gate.lock().await;
        self.rebuild_held(persona_id).await
    }

    /// [`Self::rebuild`] for a caller already holding the start gate.
    pub(super) async fn rebuild_held(self: &Arc<Self>, persona_id: &str) -> Result<(), String> {
        let (capability, was_running) = self.swap_out(persona_id);
        if !was_running {
            return Ok(());
        }
        self.start_again(persona_id, capability).await
    }

    /// Starts the main session again on `capability`. A start that fails (a
    /// key gone, a harness missing) leaves the teammate stopped, and the band
    /// has to say why, because nobody pressed Start to be handed the error.
    async fn start_again(
        self: &Arc<Self>,
        persona_id: &str,
        capability: CapabilityLease,
    ) -> Result<(), String> {
        if let Err(error) = self.start_now(persona_id, capability).await {
            let mut info = idle_info(persona_id);
            info.state = SessionState::Stopped;
            info.error = Some(error.clone());
            let _ = self.info_changes.send(info);
            return Err(error);
        }
        Ok(())
    }

    /// A sign-in to an MCP server has completed. It adds reach the policies
    /// already grant and narrows none, so each running main session is
    /// rebuilt to connect with it once its turn is over, and nothing a
    /// teammate holds or was given ends; a sign-out, a server removed or a
    /// policy change ends all of it, through [`Self::invalidate_all`].
    pub(crate) async fn signed_in_to_mcp(self: &Arc<Self>) {
        let running: Vec<Arc<Session>> = {
            let gate = self.policy_update_lock();
            let _held = gate.lock().await;
            lock(&self.sessions).values().cloned().collect()
        };
        // Never under the policy lock: a rebuild takes the start gate, and a
        // message holds that gate while closing a chapter takes the lock.
        for session in running {
            session
                .rebuild_for_sign_in
                .store(true, std::sync::atomic::Ordering::SeqCst);
            self.rebuild_for_sign_in_when_idle(&session);
        }
    }

    /// Rebuilds a main session a sign-in completed under, between turns: a
    /// turn in flight is never cut for it, and its end calls this again (see
    /// [`Room::run_queue`]). Behind the start gate a message has claimed its
    /// turn already, or waits and lands on the session this brings up. A
    /// session stopped, revoked or replaced meanwhile is left alone: what
    /// starts its teammate again connects with the sign-in.
    pub(super) fn rebuild_for_sign_in_when_idle(self: &Arc<Self>, session: &Arc<Session>) {
        use std::sync::atomic::Ordering::SeqCst;
        if !session.rebuild_for_sign_in.load(SeqCst) || lock(&session.turns).running {
            return;
        }
        let room = self.clone();
        let session = session.clone();
        tokio::spawn(async move {
            let gate = room.start_gate(&session.persona_id);
            let _held = gate.lock().await;
            if lock(&session.turns).running
                || !session.capability.is_current()
                || !room.current_session(&session)
                || !session.rebuild_for_sign_in.swap(false, SeqCst)
            {
                return;
            }
            let Ok(_working) = room.working() else {
                return;
            };
            // A rebuild that fails says why on the teammate's band itself.
            let _ = room.rebuild_held(&session.persona_id).await;
        });
    }

    /// Every live session, because a policy of "all" includes every server
    /// and a narrower one is cheap to restart anyway.
    pub async fn reattach_all(self: &Arc<Self>) -> Result<(), String> {
        let mut ids: HashSet<String> = room::roster(&self.log)
            .into_iter()
            .map(|persona| persona.id)
            .collect();
        ids.extend(lock(&self.sessions).keys().cloned());
        let mut first_error = None;
        for id in ids {
            if let Err(error) = self.reattach(&id).await {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}
