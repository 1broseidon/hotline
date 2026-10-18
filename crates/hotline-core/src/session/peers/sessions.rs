//! The peer sessions behind the threads: the summaries a window reads, the
//! permission card a person answers inside a peer turn, recovery after a
//! restart, and the one session per direction that answers.

use super::*;
use crate::driver::{HOTLINE_BACKEND_ID, acp};
use crate::mcp::server::TeammateTools;
use crate::paths::thread_participants;
use crate::session::now_ms;

impl Room {
    /// Every thread this teammate is in, newest first.
    pub fn peer_threads(&self, persona_id: &str) -> Vec<PeerThreadSummary> {
        let names: HashMap<String, String> = room::roster(&self.log)
            .into_iter()
            .map(|persona| (persona.id, persona.name))
            .collect();
        let named = |id: &str| {
            names
                .get(id)
                .cloned()
                .or_else(|| {
                    linked::is_linked(id)
                        .then(|| {
                            self.participant(id)
                                .ok()
                                .map(|p| crate::session::links::shown(&p))
                        })
                        .flatten()
                })
                .unwrap_or_else(|| "Deleted teammate".to_string())
        };
        let mut summaries: Vec<PeerThreadSummary> = thread::keys_for(self.log.root(), persona_id)
            .into_iter()
            .filter_map(|key| {
                let (a, b) = thread_participants(&key)?;
                let other = if a == persona_id { b } else { a };
                let (user_side, agent_side) = (a.to_string(), b.to_string());
                let events = self.log.load(&StreamId::Pair(key.clone()));
                let last = events
                    .iter()
                    .rfind(|event| matches!(kind_of(event), "user" | "agent"));
                Some(PeerThreadSummary {
                    with_persona_id: other.to_string(),
                    with_name: named(other),
                    exchanges: events
                        .iter()
                        .filter(|event| kind_of(event) == "turn")
                        .count() as i64,
                    last_at: events
                        .iter()
                        .filter_map(|event| event.get("ts").and_then(Value::as_i64))
                        .max()
                        .unwrap_or_default(),
                    waiting: events.iter().any(|event| {
                        kind_of(event) == "permission" && event.get("decision").is_none()
                    }),
                    working_persona_id: self.peers.answering_in(&key),
                    preview: last.map(|event| PeerPreview {
                        from_name: named(if kind_of(event) == "user" {
                            &user_side
                        } else {
                            &agent_side
                        }),
                        text: event
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        at: event.get("ts").and_then(Value::as_i64).unwrap_or_default(),
                    }),
                    thread_key: key,
                })
            })
            .collect();
        summaries.sort_by_key(|summary| std::cmp::Reverse(summary.last_at));
        summaries
    }

    /// Says that these messages have been read, and answers how many of them
    /// that moved. An id naming nothing, or a message that is already read,
    /// moves nothing — which is what makes a repeated receipt harmless.
    pub fn mark_peer_read(&self, key: &str, event_ids: &[String]) -> usize {
        let stream = StreamId::Pair(key.to_string());
        let events = self.log.load(&stream);
        let updates = read_receipt_updates(&events, event_ids);
        let moved = updates.len();
        let answerer = thread_participants(key).map_or("", |(first, _)| first);
        for event in updates {
            self.write_thread(key, answerer, &event);
        }
        moved
    }

    /// Answers a permission card raised in a peer thread, by the one who is
    /// answering there. The card is the thread's own, so the answer is written
    /// to it through the shared path; a card whose turn is over has no one
    /// behind it and is refused.
    pub(crate) async fn answer_peer_permission(
        &self,
        key: &str,
        request_id: &str,
        option_id: &str,
    ) -> Result<(), String> {
        let sessions: Vec<Arc<PeerSession>> = lock(&self.peers.sessions)
            .values()
            .filter(|live| live.thread_key == key && live.valid())
            .cloned()
            .collect();
        let Some(live) = sessions
            .iter()
            .find(|live| live.driver.answer_permission(request_id, option_id))
        else {
            return Err("That request is no longer waiting for an answer.".to_string());
        };
        let id = Value::from(format!("perm:{request_id}"));
        let card = self
            .log
            .load(&StreamId::Pair(key.to_string()))
            .into_iter()
            .find(|event| event.get("id") == Some(&id))
            .and_then(|event| serde_json::from_value::<TranscriptEvent>(event).ok());
        if let Some(TranscriptEvent::Permission {
            id,
            request_id,
            title,
            options,
            ..
        }) = card
        {
            let decided_option_name = options
                .iter()
                .find(|option| option.option_id == option_id)
                .map(|option| option.name.clone());
            self.write_thread(
                key,
                &live.target_id,
                &TranscriptEvent::Permission {
                    id,
                    ts: now_ms(),
                    request_id,
                    title,
                    options,
                    decision: Some(option_id.to_string()),
                    decided_option_name,
                },
            );
        }
        Ok(())
    }

    /// Stops every peer session this teammate is a side of. A teammate that
    /// has been deleted has no more colleagues to answer.
    pub(crate) fn drop_peer_sessions(&self, persona_id: &str) {
        self.peers.invalidate(persona_id);
        self.settle_invalid_collaboration();
    }

    /// What a restart left undone, put right once when the room opens.
    ///
    /// A delivery on a tape that the agent never read is handed to it again,
    /// so an answer that landed as the desk went down is still heard, once.
    /// An exchange whose peer turn died with the desk has a marker still
    /// open on both tapes: it is closed as failed, and a sender still inside
    /// [`RECOVER_WITHIN`] of it is told its message went unanswered, rather
    /// than waiting for an answer nothing is going to send.
    pub(in crate::session) async fn recover_exchanges(self: &Arc<Self>) {
        self.recover_queued_exchanges();
        let now = now_ms();
        for persona in room::roster(&self.log) {
            let tape = self.tape(&persona.id);
            for (id, _ts, cause, text) in unheard(&tape) {
                if matches!(cause, DeliveryCause::Handoff { .. }) {
                    continue;
                }
                // Use the same idempotent dispatch as the durable result worker:
                // either may see the saved result first at startup.
                if let Err(error) = self.deliver_identified(&persona.id, &id, cause, text).await {
                    eprintln!(
                        "{}'s delivery could not be handed on: {error}",
                        persona.name
                    );
                }
            }
            for marker in cut_off(&tape) {
                let TranscriptEvent::Peer {
                    id,
                    ts,
                    thread_key,
                    with_persona_id,
                    with_name,
                    exchanges,
                    ..
                } = marker
                else {
                    continue;
                };
                if self.owns_exchange_thread(&thread_key)
                    || self.peers.answering_in(&thread_key).is_some()
                {
                    continue;
                }
                for (whose, other_id, other_name, role) in [
                    (&persona.id, &with_persona_id, &with_name, PeerRole::Caller),
                    (
                        &with_persona_id,
                        &persona.id,
                        &persona.name,
                        PeerRole::Target,
                    ),
                ] {
                    self.write(
                        whose,
                        &TranscriptEvent::Peer {
                            id: id.clone(),
                            ts,
                            thread_key: thread_key.clone(),
                            with_persona_id: other_id.clone(),
                            with_name: other_name.clone(),
                            role,
                            exchanges,
                            status: PeerStatus::Failed,
                            seat: None,
                        },
                    );
                }
                let last = self
                    .log
                    .load(&StreamId::Pair(thread_key.clone()))
                    .iter()
                    .filter_map(|event| event.get("ts").and_then(Value::as_i64))
                    .max()
                    .unwrap_or(ts);
                if now - last > RECOVER_WITHIN {
                    continue;
                }
                let cause = DeliveryCause::Peer {
                    request_id: None,
                    persona_id: with_persona_id,
                    name: with_name,
                    thread_key,
                    status: PeerStatus::Failed,
                    about: String::new(),
                };
                let text = "Hotline restarted before they answered.".to_string();
                if let Err(error) = self.deliver_into(&persona.id, cause, text).await {
                    eprintln!("{}'s delivery could not be written: {error}", persona.name);
                }
            }
        }
    }

    /// The peer sessions the room is holding, for the idle sweep: one row per
    /// session, quiet since it last answered, working while it is mid-turn.
    pub(in crate::session) fn quiet_peers(&self) -> Vec<(ThreadId, i64, bool)> {
        lock(&self.peers.sessions)
            .values()
            .map(|live| {
                (
                    ThreadId::pair(&live.thread_key),
                    *lock(&live.last_used),
                    self.peers.answering_in(&live.thread_key).is_some(),
                )
            })
            .collect()
    }

    /// Whether a peer turn this teammate is answering in has a card waiting on
    /// the person.
    pub(in crate::session) fn peer_cards_waiting(&self, persona_id: &str) -> bool {
        let keys: Vec<String> = lock(&self.peers.sessions)
            .values()
            .filter(|live| live.target_id == persona_id)
            .map(|live| live.thread_key.clone())
            .collect();
        keys.iter().any(|key| {
            self.peers.answering_in(key).is_some()
                && crate::session::sides::waiting_on(&self.log.load(&StreamId::Pair(key.clone())))
        })
    }

    /// Lets go of this pair's peer sessions: the agent behind each is stopped,
    /// and the next exchange starts them again, reading the thread.
    pub(in crate::session) fn park_peer(&self, key: &str) {
        let mut removed = Vec::new();
        lock(&self.peers.sessions).retain(|_, live| {
            if live.thread_key != key {
                return true;
            }
            removed.push(live.clone());
            false
        });
        for live in removed {
            live.caller_capability.revoke();
            live.target_capability.revoke();
            live.driver.invalidate();
        }
    }

    /// What follows every sweep of the pairs: sessions whose leases have gone
    /// are dropped, and the collaboration grants that named them with them.
    pub(in crate::session) fn tidy_peers(&self) {
        self.peers.prune_invalid_sessions();
        self.settle_invalid_collaboration();
    }

    /// The peer session for this direction, started if it is not up.
    pub(super) async fn peer_session(
        self: &Arc<Self>,
        caller: &Persona,
        target: &Persona,
        key: &str,
        caller_capability: CapabilityLease,
        target_capability: CapabilityLease,
        scope: Option<CollaborationScope>,
    ) -> Result<Arc<PeerSession>, String> {
        let caller_id = caller.id.clone();
        let target_id = target.id.clone();
        let pair = (caller_id.clone(), target_id.clone());
        caller_capability.check()?;
        target_capability.check()?;
        if let Some(scope) = scope
            && !self.peers.scope_current(&caller_id, &target_id, scope)
        {
            return Err("That collaboration approval expired before work started.".to_string());
        }
        let cached = { lock(&self.peers.sessions).get(&pair).cloned() };
        if let Some(live) = cached {
            if live.valid() {
                return Ok(live);
            }
            let stale = { lock(&self.peers.sessions).remove(&pair) };
            if let Some(stale) = stale {
                stale.caller_capability.revoke();
                stale.target_capability.revoke();
                stale.driver.invalidate();
            }
            return Err("That peer session's capabilities have been revoked.".to_string());
        }
        caller_capability.check()?;
        target_capability.check()?;
        // The roster snapshots used to find this pair may have gone stale
        // while a policy update was being applied. Re-read them only after
        // taking the generation leases, then use the matching records below.
        let caller = self.participant(&caller_id)?;
        let target = self.persona(&target_id)?;
        caller_capability.check()?;
        target_capability.check()?;
        if let Some(scope) = scope
            && !self.peers.scope_current(&caller_id, &target_id, scope)
        {
            return Err("That collaboration approval expired before work started.".to_string());
        }
        std::fs::create_dir_all(&target.cwd).map_err(|error| {
            format!(
                "{}'s working directory {} could not be made: {error}",
                target.name, target.cwd
            )
        })?;
        // A peer conversation is its own: an agent that reopened the
        // teammate's saved session would answer its colleague inside the
        // user's context, and say so.
        let mut view = target.clone();
        view.session_checkpoints = Vec::new();
        view.last_session_id = None;
        let in_process = view.backend_id == HOTLINE_BACKEND_ID;
        let peer_caller_capability = caller_capability.scoped();
        let target_capability = target_capability.with_dependency(&peer_caller_capability);
        peer_caller_capability.check()?;
        if !in_process {
            caller_capability.check()?;
            acp::materialize_agents_md_with_capability(&view, Some(target_capability.clone()))
                .map_err(|error| {
                    format!("{}'s AGENTS.md could not be written: {error}", view.name)
                })?;
        }
        let flip = thread_participants(key).is_some_and(|(user_side, _)| user_side != caller.id);
        let driving = crate::session::Driving::new(
            format!("pair:{key}"),
            format!("a request from {}", crate::session::links::shown(&caller)),
            &target_capability,
        );
        let extra_mcp = self.grant_computer(&view, &driving).await?;
        caller_capability.check()?;
        target_capability.check()?;
        peer_caller_capability.check()?;
        if let Some(scope) = scope
            && !self.peers.scope_current(&caller_id, &target_id, scope)
        {
            return Err("That collaboration approval expired before work started.".to_string());
        }
        let driver = self.agents.agent(
            &view,
            peer_preamble(
                &caller,
                &view,
                in_process.then(|| view.reach.unwrap_or_default()),
                &self.stored_secrets(),
            ),
            said_in(&self.log.load(&StreamId::Pair(key.to_string())), flip),
            TeammateTools::new(self, &view.id)
                .with_capability(target_capability.clone())
                .for_peer(),
            extra_mcp,
        )?;
        let mut starting = StartingPeer(Some(driver.clone()));
        driver.start(&view).await?;
        caller_capability.check()?;
        peer_caller_capability.check()?;
        target_capability.check()?;
        if let Some(scope) = scope
            && !self.peers.scope_current(&caller_id, &target_id, scope)
        {
            return Err("That collaboration approval expired before work started.".to_string());
        }

        let now = now_ms();
        let live = Arc::new(PeerSession {
            driver,
            caller_capability: peer_caller_capability,
            target_capability,
            thread_key: key.to_string(),
            holder: driving.holder().clone(),
            target_id: target_id.clone(),
            flip,
            last_used: Mutex::new(now),
            window: Mutex::new(None),
            marker: Mutex::new(Marker {
                id: format!("xthread:{key}:{now}"),
                ts: now,
                exchanges: 0,
            }),
        });
        let _lifecycle = lock(&self.lifecycle);
        if !live.valid() {
            return Err("That peer session's capabilities have been revoked.".to_string());
        }
        if let Some(scope) = scope
            && !self.peers.scope_current(&caller_id, &target_id, scope)
        {
            return Err("That collaboration approval expired before work started.".to_string());
        }
        if let Some(existing) = lock(&self.peers.sessions).get(&pair).cloned() {
            if existing.valid() {
                live.caller_capability.revoke();
                live.target_capability.revoke();
                return Ok(existing);
            }
            lock(&self.peers.sessions).remove(&pair);
            existing.caller_capability.revoke();
            existing.target_capability.revoke();
            existing.driver.invalidate();
        }
        lock(&self.peers.sessions).insert(pair, live.clone());
        starting.0 = None;
        Ok(live)
    }

    /// One event into the thread: through the receipts, the way round this
    /// thread is stored, and onto the stream.
    pub(super) fn append_thread(&self, session: &PeerSession, event: TranscriptEvent) {
        let step = {
            let mut window = lock(&session.window);
            through_receipts(&mut window, event)
        };
        if let Some(read) = step.read {
            self.write_thread(
                &session.thread_key,
                &session.target_id,
                &oriented(read, session.flip),
            );
        }
        self.write_thread(
            &session.thread_key,
            &session.target_id,
            &oriented(step.event, session.flip),
        );
    }

    /// One line onto the thread's stream, through the shared write path: it
    /// is indexed under both teammates, and a card in it is pushed to the
    /// person and counted for `answerer`, who is the one it is waiting on.
    pub(super) fn write_thread(&self, key: &str, answerer: &str, event: &TranscriptEvent) {
        self.threads().write(&ThreadId::pair(key), answerer, event);
    }

    /// The marker on both sides' tapes, at whatever the exchange has reached.
    pub(super) fn mark(
        &self,
        session: &PeerSession,
        caller: &Persona,
        target: &Persona,
        status: PeerStatus,
    ) {
        let marker = lock(&session.marker);
        for (whose, other, role) in [
            (caller, target, PeerRole::Caller),
            (target, caller, PeerRole::Target),
        ] {
            self.write(
                &whose.id,
                &TranscriptEvent::Peer {
                    id: marker.id.clone(),
                    ts: marker.ts,
                    thread_key: session.thread_key.clone(),
                    with_persona_id: other.id.clone(),
                    with_name: other.name.clone(),
                    role,
                    exchanges: marker.exchanges,
                    status,
                    // Both sides are teammates of this room: a seat belongs to
                    // a citizen from outside it, and this build has none.
                    seat: None,
                },
            );
        }
    }
}
