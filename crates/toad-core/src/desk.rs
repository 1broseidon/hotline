//! The desk: the room, its vault and its log, standing behind the wire.
//!
//! The wire asks a [`RoomHandle`] for what it does not own — a session, a
//! secret, the models a key can reach — and the session room asks a
//! [`ProviderKeys`] for keys. This is the one place those two seams are
//! joined to the real things, so the shell and the headless harness open a
//! desk the same way and get the same room.

use crate::contract::{
    Attachment, BackendChoice, ChapterClose, ChapterSummary, ConfigChoice, Credential, SessionInfo,
    StreamDelta,
};
use crate::driver::{PI_BACKEND_ID, acp};
use crate::log::Log;
use crate::session::{ProviderKeys, Room};
use crate::vault::Vault;
use crate::wire::RoomHandle;
use async_trait::async_trait;
use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::broadcast;

impl ProviderKeys for Vault {
    fn provider_keys(&self) -> HashMap<String, String> {
        Vault::provider_keys(self)
    }
}

/// Everything that runs behind one data directory.
pub struct Desk {
    pub log: Log,
    room: Arc<Room>,
    vault: Arc<Vault>,
}

impl Desk {
    /// Opens the log, the vault and the room over one data directory.
    pub fn open(root: &Path) -> io::Result<Desk> {
        let log = Log::open(root);
        let vault = Arc::new(Vault::open(root, log.clone())?);
        let room = Room::new(log.clone(), vault.clone());
        Ok(Desk { log, room, vault })
    }
}

#[async_trait]
impl RoomHandle for Desk {
    async fn start(&self, persona_id: &str) -> Result<SessionInfo, String> {
        self.room.start(persona_id).await
    }

    fn stop(&self, persona_id: &str) -> Result<(), String> {
        self.room.stop(persona_id)
    }

    async fn prompt(
        &self,
        persona_id: &str,
        text: &str,
        reply_to: Option<String>,
        attachments: Option<Vec<Attachment>>,
    ) -> Result<(), String> {
        self.room
            .prompt(persona_id, text, reply_to, attachments)
            .await
    }

    /// The desk is the person: a chapter closed from the window was asked for
    /// by the user, never by the agent.
    async fn start_fresh_chapter(&self, persona_id: &str) -> Result<ChapterSummary, String> {
        self.room
            .start_fresh_chapter(persona_id, ChapterClose::User)
            .await
    }

    async fn resume_chapter(&self, persona_id: &str) -> Result<ChapterSummary, String> {
        self.room.resume_chapter(persona_id).await
    }

    fn cancel(&self, persona_id: &str) -> Result<(), String> {
        self.room.cancel(persona_id)
    }

    async fn set_model(&self, persona_id: &str, model_id: &str) -> Result<SessionInfo, String> {
        self.room.set_model(persona_id, model_id).await
    }

    async fn set_mode(&self, persona_id: &str, mode_id: &str) -> Result<SessionInfo, String> {
        self.room.set_mode(persona_id, mode_id).await
    }

    fn answer_permission(
        &self,
        persona_id: &str,
        request_id: &str,
        option_id: &str,
    ) -> Result<(), String> {
        self.room
            .answer_permission(persona_id, request_id, option_id)
    }

    fn answer_human(
        &self,
        persona_id: &str,
        action_id: &str,
        status: crate::contract::HumanAnswer,
        note: Option<String>,
    ) -> Result<(), String> {
        self.room.answer_human(persona_id, action_id, status, note)
    }

    fn info(&self, persona_id: &str) -> SessionInfo {
        self.room.info(persona_id)
    }

    fn subscribe_info(&self) -> broadcast::Receiver<SessionInfo> {
        self.room.subscribe_info()
    }

    fn subscribe_deltas(&self) -> broadcast::Receiver<StreamDelta> {
        self.room.subscribe_deltas()
    }

    fn credential_create(
        &self,
        provider_id: &str,
        label: &str,
        secret: &str,
    ) -> Result<Credential, String> {
        self.vault
            .create(provider_id, label, secret)
            .map_err(|error| error.to_string())
    }

    fn credential_revoke(&self, id: &str) -> Result<(), String> {
        self.vault.revoke(id).map_err(|error| error.to_string())
    }

    fn credential_delete(&self, id: &str) -> Result<(), String> {
        self.vault.delete(id).map_err(|error| error.to_string())
    }

    /// Toad Agent first, then whatever the ACP catalogue and the PATH say.
    async fn backends(&self) -> Vec<BackendChoice> {
        let mut choices = vec![BackendChoice {
            id: PI_BACKEND_ID.to_string(),
            name: "Toad Agent".to_string(),
            description: "Built in: runs on the desk's provider keys.".to_string(),
            unavailable: None,
        }];
        for backend in acp::registry::backends(self.log.root()).await {
            choices.push(BackendChoice {
                id: backend.id,
                name: backend.name,
                description: backend.description,
                unavailable: backend.unavailable,
            });
        }
        choices
    }

    fn credentials(&self) -> Vec<Credential> {
        self.vault.list()
    }

    fn models(&self) -> Vec<ConfigChoice> {
        self.room.models_for_desk()
    }

    fn import(&self, from: &Path) -> Result<crate::import::Report, String> {
        crate::import::import(from, &self.log, &self.vault).map_err(|error| error.to_string())
    }

    fn teammate_tools(&self, persona_id: &str) -> Option<crate::contract::TeammateToolLedger> {
        self.room.teammate_tools(persona_id)
    }

    fn schedule_create(
        &self,
        persona_id: &str,
        kind: crate::contract::ScheduleKind,
        when: Option<i64>,
        every: Option<i64>,
        prompt: &str,
        quiet: bool,
    ) -> Result<crate::contract::ScheduledJob, String> {
        self.room
            .schedule_create(persona_id, kind, when, every, prompt, quiet)
    }

    fn schedule_list(&self) -> Vec<crate::contract::ScheduledJob> {
        self.room.schedule_list()
    }

    fn schedule_cancel(&self, id: &str) -> Result<(), String> {
        self.room.schedule_cancel(id)
    }

    fn schedule_set_quiet(&self, id: &str, quiet: bool) -> Result<(), String> {
        self.room.schedule_set_quiet(id, quiet)
    }

    fn peer_threads(&self, persona_id: &str) -> Vec<crate::contract::PeerThreadSummary> {
        self.room.peer_threads(persona_id)
    }

    fn mark_peer_read(&self, key: &str, event_ids: &[String]) -> usize {
        self.room.mark_peer_read(key, event_ids)
    }

    fn forget(&self, persona_id: &str) {
        self.room.forget(persona_id);
    }
}
