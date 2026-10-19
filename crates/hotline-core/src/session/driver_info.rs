//! What a running driver says about itself: its models, modes and config,
//! the wakes and unprompted turns it raises, and the desk's changes to them.

use super::*;

impl Room {
    /// Keeps ACP's live picker metadata in the roster without putting a
    /// protocol notification on the tape. Weak references let the watcher
    /// disappear with a stopped session instead of retaining a room/session
    /// cycle while the driver is being torn down.
    pub(super) fn watch_driver_info(
        self: &Arc<Self>,
        session: &Arc<Session>,
        mut updates: watch::Receiver<DriverInfo>,
    ) {
        let room = Arc::downgrade(self);
        let session = Arc::downgrade(session);
        tokio::spawn(async move {
            while updates.changed().await.is_ok() {
                let Some(room) = room.upgrade() else {
                    break;
                };
                let Some(session) = session.upgrade() else {
                    break;
                };
                let reported = updates.borrow_and_update().clone();
                if room.apply_driver_info(&session, &reported).is_err() {
                    break;
                }
            }
        });
    }

    /// Wakes the teammate when a command it left running ends on its own: the
    /// result is a line of Hotline's, never written as the person's, queued
    /// behind whatever is waiting. On a call to the teammate the turn is
    /// said on the call as well as written, as a reply to the person's own
    /// line would be.
    pub(super) fn watch_wakes(
        self: &Arc<Self>,
        session: &Arc<Session>,
        mut wakes: mpsc::UnboundedReceiver<String>,
    ) {
        let room = Arc::downgrade(self);
        let session = Arc::downgrade(session);
        tokio::spawn(async move {
            while let Some(text) = wakes.recv().await {
                let Some(room) = room.upgrade() else {
                    break;
                };
                let Some(session) = session.upgrade() else {
                    break;
                };
                if !session.capability.is_current() || !room.current_session(&session) {
                    break;
                }
                // A room preparing to restart takes no new work, and the
                // process went with the session that held it.
                let Ok(_working) = room.working() else {
                    continue;
                };
                let mut wire = Wired::words(text);
                wire.voice = lock(&room.voice)
                    .upgrade()
                    .and_then(|voice| voice.on_call(&session.persona_id));
                room.dispatch(session, wire);
            }
        });
    }

    /// Runs what the agent does by itself between turns as a turn of its own,
    /// ahead of anything waiting, so its reply reaches the chat, and the
    /// phone, when it is said rather than behind the next message.
    pub(super) fn watch_unprompted(
        self: &Arc<Self>,
        session: &Arc<Session>,
        mut activities: mpsc::UnboundedReceiver<mpsc::Receiver<Update>>,
    ) {
        let room = Arc::downgrade(self);
        let session = Arc::downgrade(session);
        tokio::spawn(async move {
            while let Some(updates) = activities.recv().await {
                let Some(room) = room.upgrade() else {
                    break;
                };
                let Some(session) = session.upgrade() else {
                    break;
                };
                if !session.capability.is_current() || !room.current_session(&session) {
                    break;
                }
                // A room preparing to restart takes no new work; the agent's
                // words are then dropped, as they were before it could speak.
                let Ok(working) = room.lease() else {
                    continue;
                };
                let wire = Wired::unprompted(updates);
                let claimed = {
                    let mut turns = lock(&session.turns);
                    if turns.running {
                        turns.waiting.push_front(wire);
                        None
                    } else {
                        turns.running = true;
                        Some(wire)
                    }
                };
                let Some(wire) = claimed else {
                    continue;
                };
                let room = room.clone();
                tokio::spawn(async move {
                    let _working = working;
                    room.run_queue(session, wire).await;
                });
            }
        });
    }

    /// Reads each live session's picker again from its driver. Only a driver
    /// whose choices come from the room answers; a harness keeps what it
    /// reported.
    fn refresh_session_models(&self) {
        let sessions: Vec<Arc<Session>> = lock(&self.sessions).values().cloned().collect();
        for session in sessions {
            if let Some(reported) = session.driver.current_info() {
                let _ = self.apply_driver_info(&session, &reported);
            }
        }
    }

    /// Applies only mutable picker metadata from a driver response. The
    /// lifecycle and lease checks happen while the session is still captured,
    /// so a setter response or ACP notification from a replaced driver cannot
    /// overwrite the current session's state, identity, or checkpoint.
    fn apply_driver_info(
        &self,
        session: &Arc<Session>,
        reported: &DriverInfo,
    ) -> Result<SessionInfo, String> {
        let _lifecycle = lock(&self.lifecycle);
        session.capability.check()?;
        if !lock(&self.sessions)
            .get(&session.persona_id)
            .is_some_and(|current| Arc::ptr_eq(current, session))
        {
            return Err("That session is no longer current.".to_string());
        }
        let info = {
            let mut info = lock(&session.info);
            if session.driver.take_auth_success() && info.state == SessionState::Error {
                info.state = SessionState::Ready;
                info.error = None;
            }
            if info.session_id != reported.session_id {
                lock(&session.pending_checkpoint).clone_from(&reported.session_id);
                info.session_id.clone_from(&reported.session_id);
                info.context_restored = reported.context_restored;
            }
            info.models.clone_from(&reported.models);
            info.current_model_id = Some(reported.current_model_id.clone());
            info.model_label.clone_from(&reported.model_label);
            info.modes.clone_from(&reported.modes);
            info.current_mode_id.clone_from(&reported.current_mode_id);
            info.mode_label.clone_from(&reported.mode_label);
            info.configs.clone_from(&reported.configs);
            info.clone()
        };
        let _ = self.info_changes.send(info.clone());
        Ok(info)
    }

    pub async fn set_model(&self, persona_id: &str, model_id: &str) -> Result<SessionInfo, String> {
        let session = self.session(persona_id)?;
        let reported = session.driver.set_model(model_id).await?;
        self.apply_driver_info(&session, &reported)
    }

    pub async fn set_config(
        &self,
        persona_id: &str,
        config_id: &str,
        value: &str,
    ) -> Result<SessionInfo, String> {
        let session = self.session(persona_id)?;
        let reported = session.driver.set_config(config_id, value).await?;
        self.apply_driver_info(&session, &reported)
    }

    pub async fn set_mode(&self, persona_id: &str, mode_id: &str) -> Result<SessionInfo, String> {
        let session = self.session(persona_id)?;
        let reported = session.driver.set_mode(mode_id).await?;
        self.apply_driver_info(&session, &reported)
    }
}

/// Keeps each live session's model picker in step with the room. A key
/// added, a list refreshed or a filter saved changes what Hotline Agent can
/// run, and that must not wait for the session to restart. A lagged
/// receiver may have missed one of those events, so it refreshes too.
pub(super) fn follow_model_changes(room: Weak<Room>, mut events: broadcast::Receiver<Value>) {
    tokio::spawn(async move {
        loop {
            let changed = match events.recv().await {
                Ok(event) => crate::room::changes_models(&event),
                Err(broadcast::error::RecvError::Lagged(_)) => true,
                Err(broadcast::error::RecvError::Closed) => return,
            };
            if !changed {
                continue;
            }
            let Some(room) = room.upgrade() else {
                return;
            };
            room.refresh_session_models();
        }
    });
}
