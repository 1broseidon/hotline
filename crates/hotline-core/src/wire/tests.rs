//! The wire, driven the way a client drives it: over a real socket.
//!
//! The room behind the door is a stand-in — Phase 0's sessions and vault are
//! being built beside this — so what is proved here is the wire's own half:
//! the token, the framing, the ordering of a subscription, and the roster
//! view the core maintains out of the log.

use super::*;
use crate::contract::{
    ChapterClose, ConfigChoice, Credential, CredentialKind, LoginPrompt, LoginStatus, Persona,
    PersonaDraft, Reach, SessionCapabilities, SessionState,
};
use crate::driver::Driver;
use crate::mcp::server::TeammateTools;
use crate::session::{Agents, ProviderAuth, ProviderKeys, Room};
use crate::{paths, room};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::{MaybeTlsStream, connect_async};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

fn idle(persona_id: &str) -> SessionInfo {
    SessionInfo {
        persona_id: persona_id.to_string(),
        state: SessionState::Idle,
        session_id: None,
        agent_name: None,
        agent_version: None,
        context_restored: false,
        restore_note: None,
        models: Vec::new(),
        current_model_id: None,
        model_label: None,
        modes: Vec::new(),
        current_mode_id: None,
        mode_label: None,
        configs: Vec::new(),
        slash_commands: Vec::new(),
        capabilities: SessionCapabilities {
            active_input: false,
            load_session: false,
            resume: false,
            fork: false,
            mcp_http: false,
            image: false,
        },
        error: None,
    }
}

fn thinking(persona_id: &str) -> SessionInfo {
    let mut info = idle(persona_id);
    info.state = SessionState::Thinking;
    info
}

/// A room where nothing is running unless a test says otherwise: every
/// session is idle until `set_info` names one, and the vault answers with
/// what it was handed.
#[derive(Default)]
struct PendingStop {
    entered: tokio_util::sync::CancellationToken,
    release: tokio_util::sync::CancellationToken,
    completed: tokio_util::sync::CancellationToken,
    dropped: tokio_util::sync::CancellationToken,
}

struct Quiet {
    pending_stop: Option<Arc<PendingStop>>,
    auth_owner: Mutex<Option<tokio_util::sync::CancellationToken>>,
    infos: broadcast::Sender<SessionInfo>,
    deltas: broadcast::Sender<StreamDelta>,
    states: Mutex<HashMap<String, SessionInfo>>,
    info_reads: Mutex<Vec<String>>,
    reattaches: Mutex<Vec<String>>,
    invalidations: Mutex<Vec<String>>,
    computer_changes: Mutex<Vec<String>>,
    /// Teammates with a card waiting in a thread other than their DM.
    thread_cards: Mutex<std::collections::HashSet<String>>,
    policy_updates: Arc<tokio::sync::Mutex<()>>,
}

impl Quiet {
    fn new() -> Self {
        Self {
            pending_stop: None,
            auth_owner: Mutex::new(None),
            infos: broadcast::channel(16).0,
            deltas: broadcast::channel(16).0,
            states: Mutex::new(HashMap::new()),
            info_reads: Mutex::new(Vec::new()),
            reattaches: Mutex::new(Vec::new()),
            invalidations: Mutex::new(Vec::new()),
            computer_changes: Mutex::new(Vec::new()),
            thread_cards: Mutex::default(),
            policy_updates: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    fn set_info(&self, info: SessionInfo) {
        self.states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(info.persona_id.clone(), info.clone());
        let _ = self.infos.send(info);
    }

    fn reattached(&self) -> Vec<String> {
        self.reattaches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// A real session room behind the WebSocket door, with all desk-only
/// facilities stubbed. This keeps the wire regression independent of a model
/// key while the collaboration card and answer still pass through Room.
struct CoreHandle {
    room: Arc<Room>,
}

struct NoAgents;

#[async_trait]
impl Agents for NoAgents {
    fn agent(
        &self,
        _persona: &Persona,
        _preamble: String,
        _said: Vec<crate::driver::rig::Said>,
        _tools: TeammateTools,
        _extra_mcp: Vec<crate::mcp::McpServer>,
    ) -> Result<Arc<dyn Driver>, String> {
        Err("the denial test must not start a recipient session".to_string())
    }

    async fn complete(
        &self,
        _model_id: &str,
        _system: &str,
        _prompt: &str,
    ) -> Result<String, String> {
        Err("the denial test must not call a model".to_string())
    }
}

struct NoKeys;

impl ProviderKeys for NoKeys {
    fn provider_auth(&self) -> HashMap<String, ProviderAuth> {
        HashMap::new()
    }
}

#[async_trait]
impl RoomHandle for CoreHandle {
    fn threads_waiting(&self, persona_id: &str) -> bool {
        self.room.threads_waiting(persona_id)
    }

    fn policy_update_lock(&self) -> Arc<tokio::sync::Mutex<()>> {
        self.room.policy_update_lock()
    }

    async fn start(&self, persona_id: &str) -> Result<SessionInfo, String> {
        self.room.start(persona_id).await
    }

    fn stop(&self, persona_id: &str) -> Result<(), String> {
        self.room.stop(persona_id)
    }

    fn invalidate(&self, persona_id: &str) -> Result<(), String> {
        self.room.invalidate(persona_id)
    }

    fn invalidate_all(&self) -> Result<(), String> {
        self.room.invalidate_all()
    }

    async fn reattach(&self, persona_id: &str) -> Result<(), String> {
        self.room.reattach(persona_id).await
    }

    async fn reattach_all(&self) -> Result<(), String> {
        self.room.reattach_all().await
    }

    async fn prompt(
        &self,
        persona_id: &str,
        text: &str,
        reply_to: Option<String>,
        attachments: Option<Vec<crate::contract::Attachment>>,
    ) -> Result<(), String> {
        self.room
            .prompt(persona_id, text, reply_to, attachments)
            .await
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

    async fn set_config(
        &self,
        persona_id: &str,
        config_id: &str,
        value: &str,
    ) -> Result<SessionInfo, String> {
        self.room.set_config(persona_id, config_id, value).await
    }

    fn models_efforts(&self, model_id: &str) -> crate::contract::EffortChoices {
        crate::models::effort_choices_with_default(model_id)
    }

    async fn answer_permission(
        &self,
        persona_id: &str,
        request_id: &str,
        option_id: &str,
    ) -> Result<(), String> {
        self.room
            .answer_permission(persona_id, request_id, option_id)
            .await
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

    fn stop_exchange(&self, a: &str, b: &str) -> Result<(), String> {
        self.room.stop_exchange(a, b)
    }

    fn resume_exchange(&self, a: &str, b: &str) -> Result<(), String> {
        self.room.resume_exchange(a, b)
    }

    async fn start_fresh_chapter(
        &self,
        persona_id: &str,
    ) -> Result<crate::contract::ChapterSummary, String> {
        self.room
            .start_fresh_chapter(persona_id, ChapterClose::User)
            .await
    }

    async fn resume_chapter(
        &self,
        persona_id: &str,
    ) -> Result<crate::contract::ChapterSummary, String> {
        self.room.resume_chapter(persona_id).await
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
        _provider_id: &str,
        _label: &str,
        _secret: &str,
    ) -> Result<Credential, String> {
        Err("credentials are unavailable in this wire test".to_string())
    }

    fn credential_revoke(&self, _id: &str) -> Result<(), String> {
        Err("credentials are unavailable in this wire test".to_string())
    }

    fn credential_delete(&self, _id: &str) -> Result<(), String> {
        Err("credentials are unavailable in this wire test".to_string())
    }

    async fn credential_login(&self, _provider_id: &str) -> Result<LoginPrompt, String> {
        Err("credentials are unavailable in this wire test".to_string())
    }

    fn login_status(&self, _login_id: &str) -> Result<LoginStatus, String> {
        Err("credentials are unavailable in this wire test".to_string())
    }

    fn credentials(&self) -> Vec<Credential> {
        Vec::new()
    }

    async fn backends(&self) -> Vec<crate::contract::BackendChoice> {
        Vec::new()
    }

    fn skills(
        &self,
        _persona_id: Option<&str>,
    ) -> Result<Vec<crate::contract::SkillEntry>, String> {
        Ok(Vec::new())
    }

    fn models(&self) -> Vec<ConfigChoice> {
        self.room.models_for_desk()
    }

    fn models_catalog(
        &self,
        _provider_id: &str,
    ) -> Result<Vec<crate::contract::CatalogModel>, String> {
        Err("catalogues are unavailable in this wire test".to_string())
    }

    fn import(&self, _from: &std::path::Path) -> Result<crate::import::Report, String> {
        Err("import is unavailable in this wire test".to_string())
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

    async fn credential_refresh_models(
        &self,
        _provider_id: &str,
    ) -> Result<Vec<crate::contract::CatalogModel>, String> {
        Err("credentials are unavailable in this wire test".to_string())
    }

    fn forget(&self, persona_id: &str) {
        self.room.forget(persona_id);
    }

    async fn computer_runtimes(&self) -> Vec<crate::contract::RuntimeReport> {
        self.room.computer_runtimes().await
    }

    fn computer_releases(&self) -> crate::contract::ComputerReleases {
        self.room.computer_releases()
    }

    async fn computer_releases_check(&self) -> crate::contract::ComputerReleases {
        self.room.computer_releases_check().await
    }

    async fn computer_status(
        &self,
        persona_id: &str,
    ) -> Result<crate::contract::ComputerStatus, String> {
        self.room.computer_status(persona_id).await
    }

    async fn computer_stop(&self, persona_id: &str) -> Result<(), String> {
        self.room.computer_stop(persona_id).await
    }

    async fn computer_remove(&self, persona_id: &str) -> Result<(), String> {
        self.room.computer_remove(persona_id).await
    }
    async fn computer_update(&self, persona_id: &str) -> Result<(), String> {
        self.room.computer_update(persona_id).await
    }
}

#[async_trait::async_trait]
impl RoomHandle for Quiet {
    fn threads_waiting(&self, persona_id: &str) -> bool {
        self.thread_cards.lock().unwrap().contains(persona_id)
    }

    async fn computer_cookies_push(
        &self,
        _persona_id: &str,
        transfer: crate::contract::CookieTransfer,
    ) -> Result<Vec<crate::contract::CookieSite>, String> {
        let cookies = crate::computer::cookies::validate_transfer(&transfer)?;
        Ok(crate::computer::cookies::summarize(&cookies))
    }

    async fn computer_capacity(&self) -> crate::contract::ComputerCapacity {
        crate::contract::ComputerCapacity {
            runtime: Some(crate::contract::ComputerRuntime::Podman),
            cpus: 8,
            memory_bytes: 16 * 1024 * 1024 * 1024,
            source: crate::contract::ComputerCapacitySource::Runtime,
        }
    }

    async fn computer_settings_changed(&self, persona_id: &str) {
        self.computer_changes
            .lock()
            .unwrap()
            .push(persona_id.into());
    }

    async fn agent_auth_start(
        &self,
        persona_id: &str,
        method_id: &str,
        owner: tokio_util::sync::CancellationToken,
    ) -> Result<String, String> {
        assert_eq!(persona_id, "ada");
        assert_eq!(method_id, "fixture");
        *self.auth_owner.lock().unwrap() = Some(owner);
        Ok("fixture-attempt".into())
    }
    fn agent_auth_poll(
        &self,
        persona_id: &str,
        id: &str,
    ) -> Result<crate::driver::auth::AuthStatus, String> {
        assert_eq!((persona_id, id), ("ada", "fixture-attempt"));
        Ok(crate::driver::auth::AuthStatus {
            state: "running",
            output: "ephemeral fixture output".into(),
            error: None,
        })
    }
    fn agent_auth_input(&self, persona_id: &str, id: &str, input: &str) -> Result<(), String> {
        assert_eq!(
            (persona_id, id, input),
            ("ada", "fixture-attempt", "fixture input")
        );
        Ok(())
    }
    fn agent_auth_cancel(&self, persona_id: &str, id: &str) -> Result<(), String> {
        assert_eq!((persona_id, id), ("ada", "fixture-attempt"));
        Ok(())
    }

    fn policy_update_lock(&self) -> Arc<tokio::sync::Mutex<()>> {
        self.policy_updates.clone()
    }

    async fn start(&self, persona_id: &str) -> Result<SessionInfo, String> {
        let mut info = idle(persona_id);
        info.current_model_id = Some("anthropic/claude".to_string());
        self.set_info(info.clone());
        Ok(info)
    }

    fn stop(&self, _persona_id: &str) -> Result<(), String> {
        Ok(())
    }

    fn invalidate(&self, persona_id: &str) -> Result<(), String> {
        self.invalidations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(persona_id.to_string());
        Ok(())
    }

    fn invalidate_all(&self) -> Result<(), String> {
        self.invalidate("*")
    }

    async fn reattach(&self, persona_id: &str) -> Result<(), String> {
        self.reattaches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(persona_id.to_string());
        Ok(())
    }

    async fn reattach_all(&self) -> Result<(), String> {
        let ids: Vec<String> = self
            .states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .keys()
            .cloned()
            .collect();
        for id in ids {
            self.reattach(&id).await?;
        }
        Ok(())
    }

    async fn prompt(
        &self,
        _persona_id: &str,
        _text: &str,
        _reply_to: Option<String>,
        _attachments: Option<Vec<crate::contract::Attachment>>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn cancel(&self, _persona_id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn start_fresh_chapter(
        &self,
        _persona_id: &str,
    ) -> Result<crate::contract::ChapterSummary, String> {
        Err("Nothing runs in this room, so nothing has a chapter.".to_string())
    }

    async fn resume_chapter(
        &self,
        _persona_id: &str,
    ) -> Result<crate::contract::ChapterSummary, String> {
        Err("Nothing runs in this room, so nothing has a chapter.".to_string())
    }

    async fn set_model(&self, persona_id: &str, model_id: &str) -> Result<SessionInfo, String> {
        let mut info = self.info(persona_id);
        info.current_model_id = Some(model_id.to_string());
        self.set_info(info.clone());
        Ok(info)
    }

    async fn set_mode(&self, persona_id: &str, _mode_id: &str) -> Result<SessionInfo, String> {
        Ok(idle(persona_id))
    }

    async fn set_config(
        &self,
        persona_id: &str,
        config_id: &str,
        value: &str,
    ) -> Result<SessionInfo, String> {
        let mut info = self.info(persona_id);
        info.configs = vec![crate::contract::SessionConfig {
            id: config_id.to_string(),
            name: config_id.to_string(),
            category: None,
            current_id: Some(value.to_string()),
            options: Vec::new(),
        }];
        self.set_info(info.clone());
        Ok(info)
    }

    fn models_efforts(&self, model_id: &str) -> crate::contract::EffortChoices {
        crate::models::effort_choices_with_default(model_id)
    }

    async fn answer_permission(
        &self,
        _persona_id: &str,
        _request_id: &str,
        _option_id: &str,
    ) -> Result<(), String> {
        Err("Nothing runs in this room, so nothing is waiting.".to_string())
    }

    fn answer_human(
        &self,
        _persona_id: &str,
        _action_id: &str,
        _status: crate::contract::HumanAnswer,
        _note: Option<String>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn stop_exchange(&self, _a: &str, _b: &str) -> Result<(), String> {
        Ok(())
    }

    fn resume_exchange(&self, _a: &str, _b: &str) -> Result<(), String> {
        Ok(())
    }

    fn info(&self, persona_id: &str) -> SessionInfo {
        self.info_reads.lock().unwrap().push(persona_id.to_string());
        self.states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(persona_id)
            .cloned()
            .unwrap_or_else(|| idle(persona_id))
    }

    fn subscribe_info(&self) -> broadcast::Receiver<SessionInfo> {
        self.infos.subscribe()
    }

    fn subscribe_deltas(&self) -> broadcast::Receiver<StreamDelta> {
        self.deltas.subscribe()
    }

    fn credential_create(
        &self,
        provider_id: &str,
        label: &str,
        _secret: &str,
    ) -> Result<Credential, String> {
        Ok(Credential {
            id: "cred-1".to_string(),
            provider_id: provider_id.to_string(),
            credential_kind: CredentialKind::ApiKey,
            base_url: None,
            custom: None,
            label: label.to_string(),
            revoked: false,
            created_at: 1,
            updated_at: 1,
        })
    }

    fn credential_revoke(&self, _id: &str) -> Result<(), String> {
        Ok(())
    }

    fn credential_delete(&self, _id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn credential_login(&self, provider_id: &str) -> Result<LoginPrompt, String> {
        if let Some(message) = crate::models::login_refusal(provider_id) {
            return Err(message);
        }
        Err("Nothing runs in this room, so nothing signs in.".to_string())
    }

    fn login_status(&self, login_id: &str) -> Result<LoginStatus, String> {
        Err(format!("There is no login {login_id}."))
    }

    /// Hotline Agent, as every real room reports it, plus one harness this
    /// desk knows of but cannot run, for the mobile create's validation to
    /// have something to refuse.
    async fn backends(&self) -> Vec<crate::contract::BackendChoice> {
        vec![
            crate::contract::BackendChoice {
                id: "hotline".to_string(),
                name: "Hotline Agent".to_string(),
                description: "Hotline's own agent. Runs any model from the providers you connect."
                    .to_string(),
                unavailable: None,
            },
            crate::contract::BackendChoice {
                id: "cursor".to_string(),
                name: "Cursor".to_string(),
                description: "An external harness.".to_string(),
                unavailable: Some("Not signed in.".to_string()),
            },
        ]
    }

    fn skills(
        &self,
        _persona_id: Option<&str>,
    ) -> Result<Vec<crate::contract::SkillEntry>, String> {
        Ok(Vec::new())
    }

    fn credentials(&self) -> Vec<Credential> {
        Vec::new()
    }

    fn models(&self) -> Vec<ConfigChoice> {
        vec![
            ConfigChoice {
                id: "anthropic/claude".to_string(),
                name: "Claude".to_string(),
                description: None,
                group: None,
            },
            ConfigChoice {
                id: "openai/gpt".to_string(),
                name: "GPT".to_string(),
                description: None,
                group: None,
            },
        ]
    }

    fn models_catalog(
        &self,
        provider_id: &str,
    ) -> Result<Vec<crate::contract::CatalogModel>, String> {
        if crate::models::wiring(provider_id).is_none() {
            return Err(format!(
                "{provider_id} is not a provider Hotline Agent can use."
            ));
        }
        Ok(crate::models::catalog_models(
            provider_id,
            &HashMap::new(),
            None,
        ))
    }

    fn import(&self, _from: &std::path::Path) -> Result<crate::import::Report, String> {
        Ok(crate::import::Report::default())
    }

    fn teammate_tools(&self, _persona_id: &str) -> Option<crate::contract::TeammateToolLedger> {
        None
    }

    fn schedule_create(
        &self,
        _persona_id: &str,
        _kind: crate::contract::ScheduleKind,
        _when: Option<i64>,
        _every: Option<i64>,
        _prompt: &str,
        _quiet: bool,
    ) -> Result<crate::contract::ScheduledJob, String> {
        Err("Nothing runs in this room, so nothing is scheduled.".to_string())
    }

    fn schedule_list(&self) -> Vec<crate::contract::ScheduledJob> {
        Vec::new()
    }

    fn schedule_cancel(&self, _id: &str) -> Result<(), String> {
        Err("Nothing runs in this room, so nothing is scheduled.".to_string())
    }

    fn schedule_set_quiet(&self, _id: &str, _quiet: bool) -> Result<(), String> {
        Err("Nothing runs in this room, so nothing is scheduled.".to_string())
    }

    fn peer_threads(&self, _persona_id: &str) -> Vec<crate::contract::PeerThreadSummary> {
        Vec::new()
    }

    fn mark_peer_read(&self, _key: &str, _event_ids: &[String]) -> usize {
        0
    }

    async fn credential_refresh_models(
        &self,
        provider_id: &str,
    ) -> Result<Vec<crate::contract::CatalogModel>, String> {
        if let Some(message) = crate::models::login_refusal(provider_id) {
            return Err(message);
        }
        let name = crate::models::catalog()
            .providers
            .get(provider_id)
            .map(|entry| entry.name.as_str())
            .unwrap_or(provider_id);
        Err(format!("There is no sign-in for {name}."))
    }

    fn forget(&self, _persona_id: &str) {}

    fn computer_releases(&self) -> crate::contract::ComputerReleases {
        crate::contract::ComputerReleases {
            floor: "0.0.0".into(),
            repository: "example/computer".into(),
            newest: None,
            releases: Vec::new(),
            checked_at: None,
            error: None,
        }
    }

    async fn computer_releases_check(&self) -> crate::contract::ComputerReleases {
        self.computer_releases()
    }

    async fn computer_runtimes(&self) -> Vec<crate::contract::RuntimeReport> {
        vec![
            crate::contract::RuntimeReport {
                runtime: crate::contract::ComputerRuntime::Podman,
                state: crate::contract::RuntimeState::Ready,
                detail: None,
                rootless: true,
            },
            crate::contract::RuntimeReport {
                runtime: crate::contract::ComputerRuntime::Docker,
                state: crate::contract::RuntimeState::Ready,
                detail: None,
                rootless: false,
            },
        ]
    }

    async fn computer_status(
        &self,
        _persona_id: &str,
    ) -> Result<crate::contract::ComputerStatus, String> {
        Ok(crate::contract::ComputerStatus {
            state: crate::contract::ComputerState::Running,
            url: Some("http://127.0.0.1:18787/mcp".into()),
            viewer: Some("http://127.0.0.1:15800".into()),
            release: None,
            available: None,
            update_failed: None,
        })
    }

    async fn computer_stop(&self, _persona_id: &str) -> Result<(), String> {
        if let Some(pending) = &self.pending_stop {
            let _dropped = pending.dropped.clone().drop_guard();
            pending.entered.cancel();
            pending.release.cancelled().await;
            pending.completed.cancel();
        }
        Ok(())
    }

    async fn computer_remove(&self, _persona_id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn computer_update(&self, _persona_id: &str) -> Result<(), String> {
        Ok(())
    }
}

fn scratch(name: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("hotline-core-wire-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("transcripts")).unwrap();
    root
}

/// A door on a scratch data directory, already serving.
fn door(name: &str) -> (PathBuf, Log, u16) {
    door_with(name, Arc::new(Quiet::new()))
}

fn door_with(name: &str, room: Arc<Quiet>) -> (PathBuf, Log, u16) {
    let root = scratch(name);
    let log = Log::open(&root);
    let door = Door::bind(log.clone(), "desk-token".to_string(), room).unwrap();
    let port = door.port();
    tokio::spawn(door.run());
    (root, log, port)
}

/// A real session room behind the door, with a workspace pair and no model
/// implementation. The denial path must finish before the recipient driver is
/// ever requested, so a driver is unnecessary for this wire proof.
fn core_door(name: &str) -> (PathBuf, Log, u16, Arc<Room>) {
    let root = scratch(name);
    let log = Log::open(&root);
    let persona = |id: &str, name: &str| {
        json!({
            "kind": "persona",
            "id": id,
            "name": name,
            "goal": format!("Role of {name}"),
            "backendId": "hotline",
            "cwd": root.to_string_lossy(),
            "mcpPolicy": { "mode": "none", "serverIds": [] },
            "backgroundWork": false,
            "sessionCheckpoints": [],
            "lastSessionId": null,
            "createdAt": 1,
            "updatedAt": 1,
        })
    };
    log.append(&StreamId::Room, &persona("ada", "Ada")).unwrap();
    log.append(&StreamId::Room, &persona("bob", "Bob")).unwrap();
    let room = Room::with_agents(log.clone(), Arc::new(NoKeys), Arc::new(NoAgents));
    let door = Door::bind(
        log.clone(),
        "desk-token".to_string(),
        Arc::new(CoreHandle { room: room.clone() }),
    )
    .unwrap();
    let port = door.port();
    tokio::spawn(door.run());
    (root, log, port, room)
}

async fn desk(port: u16) -> Socket {
    connect_async(format!("ws://127.0.0.1:{port}/ws?token=desk-token"))
        .await
        .unwrap()
        .0
}

async fn ask(socket: &mut Socket, frame: Value) {
    socket.send(Message::text(frame.to_string())).await.unwrap();
}

async fn heard(socket: &mut Socket) -> Value {
    let message = socket.next().await.unwrap().unwrap();
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}

/// The next frame this test is waiting for. A command's answer and a
/// subscription's frame are queued by different tasks, so a test that wants
/// one of them says which rather than counting.
async fn heard_where(socket: &mut Socket, wanted: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..16 {
        let frame = heard(socket).await;
        if wanted(&frame) {
            return frame;
        }
    }
    panic!("the frame this test was waiting for never arrived");
}

async fn create(socket: &mut Socket, id: i64, name: &str) -> Value {
    ask(
        socket,
        json!({ "id": id, "cmd": "persona.create", "params": { "draft": { "name": name } } }),
    )
    .await;
    let answer = heard_where(socket, |frame| frame["id"] == id).await;
    assert_eq!(answer["ok"], true, "{answer}");
    answer["result"].clone()
}

/// A command's answer, or a failure saying it never came. A command that
/// takes the read loop down with it is answered never, and a test that waits
/// forever for that answer says nothing about why.
async fn answered(socket: &mut Socket, id: i64) -> Value {
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        heard_where(socket, |frame| frame["id"] == id),
    )
    .await
    .unwrap_or_else(|_| panic!("command {id} was never answered"))
}

#[tokio::test]
async fn a_command_is_answered_by_its_id_and_a_command_nobody_has_is_refused() {
    let (_root, _log, port) = door("framing");
    let mut socket = desk(port).await;

    let created = create(&mut socket, 1, "Ada").await;
    assert_eq!(created["name"], "Ada");

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "persona.fly", "params": {} }),
    )
    .await;
    let refused = heard(&mut socket).await;
    assert_eq!(refused["id"], 2);
    assert_eq!(refused["ok"], false);
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("cannot read that command"),
        "{refused}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_wire_answers_a_core_owned_collaboration_card_before_peer_start() {
    let (_root, log, port, room) = core_door("collaboration-wire");
    let mut socket = desk(port).await;
    ask(&mut socket, json!({ "id": 1, "sub": { "tape": "ada" } })).await;
    assert_eq!(heard(&mut socket).await, json!({ "id": 1, "ok": true }));
    let snapshot = heard(&mut socket).await;
    assert_eq!(snapshot["snapshot"], json!([]));

    let delivery = {
        let room = room.clone();
        tokio::spawn(async move { room.deliver("ada", "bob", "read Bob's private file").await })
    };
    let card = heard_where(&mut socket, |frame| {
        frame["sub"] == 1 && frame["event"]["kind"] == "permission"
    })
    .await;
    let request_id = card["event"]["requestId"].as_str().unwrap();
    assert!(request_id.starts_with("collab:"), "{card}");
    assert_eq!(
        card["event"]["title"],
        "Allow Ada to ask Bob to work?\n\nBob can take handoffs in a work thread of its own, use its own context, workspace and enabled tools to fulfill Ada's requests and return results."
    );

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "session.answer_permission",
            "params": { "personaId": "ada", "requestId": request_id, "optionId": "deny" }
        }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    let denied = delivery.await.unwrap().unwrap_err();
    assert!(denied.contains("denied"), "{denied}");
    assert!(log.load(&StreamId::Pair("ada~bob".to_string())).is_empty());
}

#[tokio::test]
async fn the_desk_token_opens_the_door_and_nothing_else_does() {
    let (_root, _log, port) = door("token");

    let wrong = connect_async(format!("ws://127.0.0.1:{port}/ws?token=not-it"))
        .await
        .err()
        .unwrap();
    assert_eq!(status_of(wrong), Some(StatusCode::UNAUTHORIZED));

    let elsewhere = connect_async(format!("ws://127.0.0.1:{port}/pair?token=desk-token"))
        .await
        .err()
        .unwrap();
    assert_eq!(status_of(elsewhere), Some(StatusCode::NOT_FOUND));
}

fn status_of(error: Error) -> Option<StatusCode> {
    match error {
        Error::Http(response) => Some(response.status()),
        _ => None,
    }
}

#[tokio::test]
async fn a_subscription_is_the_fold_and_then_every_event_that_lands_after_it() {
    let (_root, log, port) = door("subscribe");
    let mut socket = desk(port).await;

    let already = json!({ "kind": "setting", "id": "chapterIdleHours", "value": 2 });
    log.append(&StreamId::Room, &already).unwrap();

    ask(&mut socket, json!({ "id": 7, "sub": "room" })).await;
    assert_eq!(heard(&mut socket).await, json!({ "id": 7, "ok": true }));
    assert_eq!(
        heard(&mut socket).await,
        json!({ "sub": 7, "snapshot": [already] })
    );

    let later = json!({ "kind": "setting", "id": "theme", "value": "dark" });
    log.append(&StreamId::Room, &later).unwrap();
    assert_eq!(
        heard(&mut socket).await,
        json!({ "sub": 7, "event": later })
    );

    ask(&mut socket, json!({ "id": 8, "unsub": 7 })).await;
    assert_eq!(heard(&mut socket).await, json!({ "id": 8, "ok": true }));
}

#[tokio::test]
async fn the_roster_is_one_row_per_living_teammate_and_a_tombstone_takes_one_away() {
    let (_root, _log, port) = door("roster");
    let mut socket = desk(port).await;

    let ada = create(&mut socket, 1, "Ada").await;
    let bob = create(&mut socket, 2, "Bob").await;

    ask(&mut socket, json!({ "id": 3, "sub": { "view": "roster" } })).await;
    let snapshot = heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    let rows = snapshot["snapshot"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["persona"], ada);
    assert_eq!(rows[0]["session"]["state"], "idle");
    assert!(rows[0]["preview"].is_null(), "{snapshot}");
    assert_eq!(rows[1]["persona"], bob);

    ask(
        &mut socket,
        json!({ "id": 4, "cmd": "persona.delete", "params": { "id": bob["id"] } }),
    )
    .await;
    let removed = heard_where(&mut socket, |frame| frame["removed"].is_string()).await;
    assert_eq!(removed, json!({ "sub": 3, "removed": bob["id"] }));
}

#[tokio::test]
async fn pinning_orders_the_desks_pins_and_the_roster_rows_carry_their_slot() {
    let (_root, log, port) = door("pins");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let bob = create(&mut socket, 2, "Bob").await;
    let cy = create(&mut socket, 3, "Cy").await;
    let di = create(&mut socket, 4, "Di").await;

    ask(&mut socket, json!({ "id": 5, "sub": { "view": "roster" } })).await;
    let snapshot = heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    assert!(snapshot["snapshot"][0]["pin"].is_null(), "{snapshot}");

    let pin = |id: i64, who: &Value, slot: Value| json!({ "id": id, "cmd": "persona.pin", "params": { "id": who["id"], "slot": slot } });
    ask(&mut socket, pin(6, &ada, json!(0))).await;
    assert_eq!(
        heard_where(&mut socket, |f| f["id"] == 6).await["result"],
        json!([ada["id"]])
    );
    let row = heard_where(&mut socket, |f| f["event"]["persona"]["id"] == ada["id"]).await;
    assert_eq!(row["event"]["pin"], 0, "{row}");

    // Inserting at a slot shifts the rest along; a slot past the end appends.
    ask(&mut socket, pin(7, &bob, json!(0))).await;
    assert_eq!(
        heard_where(&mut socket, |f| f["id"] == 7).await["result"],
        json!([bob["id"], ada["id"]])
    );
    let row = heard_where(&mut socket, |f| {
        f["event"]["persona"]["id"] == ada["id"] && f["event"]["pin"] == 1
    })
    .await;
    assert_eq!(row["event"]["pin"], 1, "{row}");
    ask(&mut socket, pin(8, &cy, json!(9))).await;
    assert_eq!(
        heard_where(&mut socket, |f| f["id"] == 8).await["result"],
        json!([bob["id"], ada["id"], cy["id"]])
    );

    // A fourth is refused and says why; moving a pinned one is not a fourth.
    ask(&mut socket, pin(9, &di, json!(0))).await;
    let refused = heard_where(&mut socket, |f| f["id"] == 9).await;
    assert_eq!(refused["ok"], false);
    assert_eq!(
        refused["error"],
        "A desk pins up to 3 teammates. Unpin one first."
    );
    ask(&mut socket, pin(10, &cy, json!(0))).await;
    assert_eq!(
        heard_where(&mut socket, |f| f["id"] == 10).await["result"],
        json!([cy["id"], bob["id"], ada["id"]])
    );

    ask(&mut socket, pin(11, &bob, Value::Null)).await;
    assert_eq!(
        heard_where(&mut socket, |f| f["id"] == 11).await["result"],
        json!([cy["id"], ada["id"]])
    );
    let row = heard_where(&mut socket, |f| {
        f["event"]["persona"]["id"] == bob["id"] && f["event"]["pin"].is_null()
    })
    .await;
    assert!(row["event"]["pin"].is_null(), "{row}");
    assert_eq!(
        room::pinned_teammates(&log),
        vec![cy["id"].as_str().unwrap(), ada["id"].as_str().unwrap()]
    );

    ask(&mut socket, pin(12, &json!({ "id": "nobody" }), json!(0))).await;
    assert_eq!(
        heard_where(&mut socket, |f| f["id"] == 12).await["error"],
        "There is no teammate nobody."
    );
}

#[tokio::test]
async fn deleting_a_pinned_teammate_frees_its_slot() {
    let (_root, log, port) = door("pins-delete");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let bob = create(&mut socket, 2, "Bob").await;
    for (id, who) in [(3, &ada), (4, &bob)] {
        ask(
            &mut socket,
            json!({ "id": id, "cmd": "persona.pin", "params": { "id": who["id"], "slot": 9 } }),
        )
        .await;
        assert_eq!(
            heard_where(&mut socket, |f| f["id"] == id).await["ok"],
            true
        );
    }
    ask(
        &mut socket,
        json!({ "id": 5, "cmd": "persona.delete", "params": { "id": ada["id"] } }),
    )
    .await;
    assert_eq!(heard_where(&mut socket, |f| f["id"] == 5).await["ok"], true);
    assert_eq!(room::settings(&log)["pinnedTeammates"], json!([bob["id"]]));
}

#[test]
fn only_a_desk_or_owner_seat_may_pin() {
    let pin = Command::PersonaPin {
        id: "ada".to_string(),
        slot: Some(0),
    };
    assert!(Seat::Desk.permits(&pin));
    assert!(Seat::Owner.permits(&pin));
    assert!(!Seat::Phone.permits(&pin));
}

#[tokio::test]
async fn a_line_in_a_tape_is_the_rosters_preview_of_that_teammate() {
    let (_root, log, port) = door("roster-preview");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let persona_id = ada["id"].as_str().unwrap().to_string();

    ask(&mut socket, json!({ "id": 2, "sub": { "view": "roster" } })).await;
    heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;

    log.append(
        &StreamId::Tape(persona_id.clone()),
        &json!({ "kind": "user", "id": "u1", "ts": 5, "text": "morning" }),
    )
    .unwrap();

    let row = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(
        row["event"]["preview"],
        json!({ "from": "me", "text": "morning", "at": 5 })
    );
    assert_eq!(row["event"]["latest"], 5);
}

#[tokio::test]
async fn a_card_waiting_on_the_person_marks_the_row_until_it_is_answered() {
    let (_root, log, port) = door("roster-waiting");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let persona_id = ada["id"].as_str().unwrap().to_string();

    ask(&mut socket, json!({ "id": 2, "sub": { "view": "roster" } })).await;
    let snapshot = heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    assert_eq!(snapshot["snapshot"][0]["waiting"], false, "{snapshot}");

    let tape = StreamId::Tape(persona_id);
    let card = |decision: Option<&str>| {
        let mut card = json!({
            "kind": "permission", "id": "p1", "ts": 5, "requestId": "r1",
            "title": "Run make", "options": [{ "optionId": "allow", "name": "Allow" }],
        });
        if let Some(decision) = decision {
            card["decision"] = json!(decision);
        }
        card
    };
    log.append(&tape, &card(None)).unwrap();
    let asked = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(asked["event"]["waiting"], true, "{asked}");

    log.append(&tape, &card(Some("allow"))).unwrap();
    let answered = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(answered["event"]["waiting"], false, "{answered}");

    log.append(
        &tape,
        &json!({
            "kind": "human_action", "id": "h1", "ts": 6, "actionId": "a1",
            "reason": "Tap the 2FA prompt", "status": "pending",
        }),
    )
    .unwrap();
    let human = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(human["event"]["waiting"], true, "{human}");
}

#[tokio::test]
async fn a_card_waiting_in_a_side_thread_marks_the_row_though_the_tape_has_none() {
    let room = Arc::new(Quiet::new());
    let (_root, _log, port) = door_with("roster-thread-waiting", room.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let persona_id = ada["id"].as_str().unwrap().to_string();
    ask(&mut socket, json!({ "id": 2, "sub": { "view": "roster" } })).await;
    let snapshot = heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    assert_eq!(snapshot["snapshot"][0]["waiting"], false, "{snapshot}");

    room.thread_cards.lock().unwrap().insert(persona_id.clone());
    room.set_info(idle(&persona_id));
    let asked = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(asked["event"]["waiting"], true, "{asked}");
}

#[tokio::test]
async fn an_exchange_pause_updates_the_live_roster_until_resumed_or_stopped() {
    let (_root, log, port) = door("roster-exchange-pause");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let tape = StreamId::Tape(ada["id"].as_str().unwrap().to_string());
    ask(&mut socket, json!({ "id": 2, "sub": { "view": "roster" } })).await;
    heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;

    for status in ["pending", "resumed", "pending", "stopped"] {
        log.append(
            &tape,
            &json!({
                "kind": "exchange_paused", "id": "pause-1", "ts": 5,
                "withPersonaId": "mack", "withName": "Mack", "exchanges": 12,
                "status": status,
            }),
        )
        .unwrap();
        let row = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
        assert_eq!(row["event"]["waiting"], status == "pending", "{row}");
    }
}

#[tokio::test]
async fn the_rosters_latest_is_the_last_message_ts_and_a_tool_does_not_move_it() {
    let (_root, log, port) = door("roster-latest");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let persona_id = ada["id"].as_str().unwrap().to_string();

    ask(&mut socket, json!({ "id": 2, "sub": { "view": "roster" } })).await;
    heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;

    log.append(
        &StreamId::Tape(persona_id.clone()),
        &json!({ "kind": "user", "id": "u1", "ts": 5, "text": "morning" }),
    )
    .unwrap();
    let first = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(first["event"]["latest"], 5);

    log.append(
        &StreamId::Tape(persona_id.clone()),
        &json!({
            "kind": "tool", "id": "t1", "ts": 8, "toolCallId": "c1",
            "title": "Read main.rs", "status": "in_progress",
        }),
    )
    .unwrap();
    let tool = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(tool["event"]["latest"], 5);
    assert!(tool["event"].get("activity").is_none(), "{tool}");

    log.append(
        &StreamId::Tape(persona_id),
        &json!({ "kind": "agent", "id": "a1", "ts": 9, "text": "on it" }),
    )
    .unwrap();
    let second = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(second["event"]["latest"], 9);
}

#[tokio::test(flavor = "current_thread")]
async fn the_roster_recovers_a_session_update_lost_to_lag() {
    let quiet = Arc::new(Quiet::new());
    let (_root, _log, port) = door_with("roster-session-lag", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let bob = create(&mut socket, 2, "Bob").await;
    let ada_id = ada["id"].as_str().unwrap();
    let bob_id = bob["id"].as_str().unwrap();
    quiet.set_info(thinking(ada_id));

    ask(&mut socket, json!({ "id": 3, "sub": { "view": "roster" } })).await;
    let initial = heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    let ada_row = initial["snapshot"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["persona"]["id"] == ada_id)
        .unwrap();
    assert_eq!(ada_row["session"]["state"], "thinking");

    // No await on this single-threaded runtime: Ada's final update falls out
    // of the 16-slot channel before the roster can receive it. Only Bob's
    // updates remain, so replaying those cannot repair Ada's stale state.
    quiet.set_info(idle(ada_id));
    for _ in 0..32 {
        quiet.set_info(thinking(bob_id));
    }

    let recovered = heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    assert_eq!(recovered["sub"], 3);
    let rows = recovered["snapshot"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    for (persona_id, state) in [(ada_id, "idle"), (bob_id, "thinking")] {
        let row = rows
            .iter()
            .find(|row| row["persona"]["id"] == persona_id)
            .unwrap();
        assert_eq!(row["session"]["state"], state);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_roster_tape_burst_reads_each_dirty_teammate_once() {
    let quiet = Arc::new(Quiet::new());
    let (_root, log, port) = door_with("roster-burst", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let bob = create(&mut socket, 2, "Bob").await;
    let ids = [ada["id"].as_str().unwrap(), bob["id"].as_str().unwrap()];
    ask(&mut socket, json!({ "id": 3, "sub": { "view": "roster" } })).await;
    heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    quiet.info_reads.lock().unwrap().clear();

    // Overrun both tape receivers without letting their pumps or the view
    // run. Only the final state matters, not the thousand nudges per row.
    for n in 0..1000 {
        for persona_id in ids {
            log.append(
                &StreamId::Tape(persona_id.to_string()),
                &json!({
                    "kind": "agent", "id": format!("a{n}"), "ts": n, "text": format!("line {n}")
                }),
            )
            .unwrap();
        }
    }
    for _ in 0..2 {
        let frame = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            heard_where(&mut socket, |frame| frame["event"].is_object()),
        )
        .await
        .unwrap();
        assert_eq!(frame["event"]["preview"]["text"], "line 999");
        assert_eq!(frame["event"]["latest"], 999);
    }
    let mut reads = quiet.info_reads.lock().unwrap().clone();
    reads.sort();
    let mut expected = ids.map(str::to_string);
    expected.sort();
    assert_eq!(reads, expected);
}

#[tokio::test(flavor = "current_thread")]
async fn a_roster_session_burst_refreshes_each_teammate_once() {
    let quiet = Arc::new(Quiet::new());
    let (_root, _log, port) = door_with("roster-session-burst", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let bob = create(&mut socket, 2, "Bob").await;
    let ids = [ada["id"].as_str().unwrap(), bob["id"].as_str().unwrap()];
    ask(&mut socket, json!({ "id": 3, "sub": { "view": "roster" } })).await;
    heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    quiet.info_reads.lock().unwrap().clear();
    for _ in 0..8 {
        for id in ids {
            quiet.set_info(thinking(id));
        }
    }
    for _ in 0..2 {
        let row = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
        assert_eq!(row["event"]["session"]["state"], "thinking");
    }
    let mut reads = quiet.info_reads.lock().unwrap().clone();
    reads.sort();
    let mut expected = ids.map(str::to_string);
    expected.sort();
    assert_eq!(reads, expected);
}

#[tokio::test]
async fn roster_updates_use_the_cached_persona_after_a_room_edit() {
    let quiet = Arc::new(Quiet::new());
    let (root, log, port) = door_with("roster-cached-persona", quiet.clone());
    let mut socket = desk(port).await;
    let mut ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap().to_string();
    ask(&mut socket, json!({ "id": 2, "sub": { "view": "roster" } })).await;
    heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    ada["kind"] = json!("persona");
    ada["name"] = json!("Ada updated");
    log.append(&StreamId::Room, &ada).unwrap();
    let updated = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(updated["event"]["persona"]["name"], "Ada updated");

    // Make another room read impossible; a session hint should need only
    // the retained persona and the tape tail, not the room file again.
    std::fs::rename(root.join("room.jsonl"), root.join("room.saved")).unwrap();
    quiet.set_info(thinking(&id));
    let row = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        heard_where(&mut socket, |frame| frame["event"].is_object()),
    )
    .await
    .unwrap();
    assert_eq!(row["event"]["persona"]["name"], "Ada updated");
    assert_eq!(row["event"]["session"]["state"], "thinking");
    std::fs::rename(root.join("room.saved"), root.join("room.jsonl")).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn room_lag_rebuilds_cached_personas_and_late_hints_cannot_restore_a_deletion() {
    let quiet = Arc::new(Quiet::new());
    let (_root, log, port) = door_with("roster-room-lag", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let mut bob = create(&mut socket, 2, "Bob").await;
    let ada_id = ada["id"].as_str().unwrap();
    let bob_id = bob["id"].as_str().unwrap().to_string();
    ask(&mut socket, json!({ "id": 3, "sub": { "view": "roster" } })).await;
    heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    bob["kind"] = json!("persona");
    bob["name"] = json!("Bob updated");
    log.append(&StreamId::Room, &bob).unwrap();
    log.append(
        &StreamId::Room,
        &json!({"kind": "persona", "id": ada_id, "deleted": true}),
    )
    .unwrap();
    for n in 0..1_200 {
        log.append(
            &StreamId::Room,
            &json!({"kind": "setting", "id": "noise", "value": n}),
        )
        .unwrap();
    }
    let recovered = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        heard_where(&mut socket, |frame| frame["snapshot"].is_array()),
    )
    .await
    .unwrap();
    let rows = recovered["snapshot"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["persona"]["name"], "Bob updated");
    quiet.info_reads.lock().unwrap().clear();
    quiet.set_info(thinking(ada_id));
    log.append(
        &StreamId::Tape(ada_id.to_string()),
        &json!({"kind": "agent", "id": "late", "text": "late", "ts": 1}),
    )
    .unwrap();
    quiet.set_info(thinking(&bob_id));
    let row = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(row["event"]["persona"]["id"], bob_id);
    assert_eq!(*quiet.info_reads.lock().unwrap(), [bob_id]);
}

#[tokio::test]
async fn search_failures_are_wire_errors_instead_of_empty_results() {
    let (root, _log, port) = door("search-unavailable");
    let mut socket = desk(port).await;
    for (id, command) in [(1, "search.thread"), (2, "search.all")] {
        ask(
            &mut socket,
            json!({"id": id, "cmd": command,
            "params": {"personaId": "ada", "query": "harbour"}}),
        )
        .await;
        let answer = heard_where(&mut socket, |frame| frame["id"] == id).await;
        assert_eq!(answer["ok"], false);
        assert_eq!(
            answer["error"],
            "Search is unavailable right now. Please try again."
        );
        assert!(answer.get("result").is_none());
    }
    assert!(!paths::index_path(&root).exists());
}

#[tokio::test]
async fn a_tool_in_progress_is_the_rosters_activity_while_the_session_is_thinking() {
    let quiet = Arc::new(Quiet::new());
    let (_root, log, port) = door_with("roster-activity", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let persona_id = ada["id"].as_str().unwrap().to_string();

    ask(&mut socket, json!({ "id": 2, "sub": { "view": "roster" } })).await;
    heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;

    quiet.set_info(thinking(&persona_id));
    let thinking_row = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(thinking_row["event"]["session"]["state"], "thinking");
    assert!(
        thinking_row["event"].get("activity").is_none(),
        "{thinking_row}"
    );

    log.append(
        &StreamId::Tape(persona_id.clone()),
        &json!({
            "kind": "tool", "id": "t1", "ts": 10, "toolCallId": "c1",
            "title": "Read main.rs", "status": "in_progress",
        }),
    )
    .unwrap();
    let running = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(running["event"]["activity"], "Read main.rs");

    log.append(
        &StreamId::Tape(persona_id.clone()),
        &json!({
            "kind": "tool", "id": "t1", "ts": 11, "toolCallId": "c1",
            "title": "Read main.rs", "status": "completed",
        }),
    )
    .unwrap();
    let done = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert!(done["event"].get("activity").is_none(), "{done}");

    log.append(
        &StreamId::Tape(persona_id.clone()),
        &json!({
            "kind": "tool", "id": "t2", "ts": 12, "toolCallId": "c2",
            "title": "Edit lib.rs", "status": "in_progress",
        }),
    )
    .unwrap();
    let next = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(next["event"]["activity"], "Edit lib.rs");

    log.append(
        &StreamId::Tape(persona_id.clone()),
        &json!({ "kind": "turn", "id": "tu1", "ts": 13, "stopReason": "end_turn" }),
    )
    .unwrap();
    let ended = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert!(ended["event"].get("activity").is_none(), "{ended}");

    log.append(
        &StreamId::Tape(persona_id.clone()),
        &json!({
            "kind": "tool", "id": "t3", "ts": 14, "toolCallId": "c3",
            "title": "Run tests", "status": "in_progress",
        }),
    )
    .unwrap();
    let again = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(again["event"]["activity"], "Run tests");

    quiet.set_info(idle(&persona_id));
    let idle_row = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(idle_row["event"]["session"]["state"], "idle");
    assert!(idle_row["event"].get("activity").is_none(), "{idle_row}");
}

#[tokio::test]
async fn a_new_teammate_takes_the_rooms_defaults() {
    let (root, log, port) = door("defaults");
    let mut socket = desk(port).await;

    let created = create(&mut socket, 1, "  Ada  ").await;
    let id = created["id"].as_str().unwrap();
    assert_eq!(created["name"], "Ada");
    assert_eq!(created["goal"], "");
    assert_eq!(created["backendId"], "hotline");
    assert_eq!(
        created["cwd"],
        json!(paths::default_workspace(&root, id).to_string_lossy())
    );
    assert_eq!(
        created["mcpPolicy"],
        json!({ "mode": "none", "serverIds": [] })
    );
    assert_eq!(created["sessionCheckpoints"], json!([]));
    assert_eq!(created["createdAt"], created["updatedAt"]);
    // Absent, not null: the workspace is the wall, and a teammate that never
    // asked for the machine says nothing about reach at all.
    assert!(created.get("reach").is_none(), "{created}");
    assert!(created.get("team").is_none(), "{created}");
    // No room default and no last model used: the driver picks at start.
    assert!(created.get("modelId").is_none(), "{created}");

    // And it is the room's own record, not a value the door made up.
    assert_eq!(json!(room::roster(&log)), json!([created]));
}

#[tokio::test]
async fn a_draft_that_names_things_keeps_them() {
    let (_root, _log, port) = door("draft");
    let mut socket = desk(port).await;

    let draft = PersonaDraft {
        name: "Bob".to_string(),
        goal: Some("  Keep the harbour running.  ".to_string()),
        team: Some("harbour".to_string()),
        backend_id: Some("cursor".to_string()),
        cwd: Some("/tmp/harbour".to_string()),
        reach: Some(crate::contract::Reach::Machine),
        model_id: Some("gpt-5".to_string()),
        effort_id: None,
        computer: None,
        background_work: Some(true),
    };
    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "persona.create", "params": { "draft": draft } }),
    )
    .await;
    let created = heard(&mut socket).await["result"].clone();
    assert_eq!(created["goal"], "Keep the harbour running.");
    assert_eq!(created["team"], "harbour");
    assert_eq!(created["backendId"], "cursor");
    assert_eq!(created["cwd"], "/tmp/harbour");
    assert_eq!(created["reach"], "machine");
    assert_eq!(created["modelId"], "gpt-5");
    assert_eq!(created["backgroundWork"], true);
}

#[tokio::test]
async fn a_patch_is_folded_over_the_teammate_and_the_whole_record_is_written_again() {
    let (_root, log, port) = door("update");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap().to_string();

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "persona.update", "params": { "id": id, "patch": { "name": "Ada Lovelace" } } }),
    )
    .await;
    let updated = heard(&mut socket).await["result"].clone();
    assert_eq!(updated["name"], "Ada Lovelace");
    assert_eq!(updated["goal"], ada["goal"]);
    assert_eq!(updated["createdAt"], ada["createdAt"]);
    assert_eq!(json!(room::roster(&log)), json!([updated]));

    ask(
        &mut socket,
        json!({ "id": 3, "cmd": "persona.update", "params": { "id": "nobody", "patch": {} } }),
    )
    .await;
    let refused = heard(&mut socket).await;
    assert_eq!(refused["ok"], false);
    assert_eq!(refused["error"], "There is no teammate nobody.");
}

#[tokio::test]
async fn a_setting_is_one_event_per_key_and_null_puts_the_default_back() {
    let (_root, log, port) = door("settings");
    let mut socket = desk(port).await;

    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "settings.update", "params": { "patch": { "theme": "dark", "chapterIdleHours": 2 } } }),
    )
    .await;
    let settings = heard(&mut socket).await["result"].clone();
    assert_eq!(settings["theme"], "dark");
    assert_eq!(settings["chapterIdleHours"], 2);

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "settings.update", "params": { "patch": { "chapterIdleHours": null } } }),
    )
    .await;
    let cleared = heard(&mut socket).await["result"].clone();
    assert_eq!(cleared["chapterIdleHours"], 8);
    assert_eq!(json!(room::settings(&log)), cleared);
}

/// Words arriving in a thread, as the room broadcasts them.
fn said_in(thread: ThreadId, message_id: &str, text: &str) -> StreamDelta {
    StreamDelta::ThreadDelta {
        thread,
        message_id: message_id.to_string(),
        kind: DeltaKind::Text,
        text: text.to_string(),
    }
}

/// A socket that has said it reads `threads2`, with the answer it got.
async fn hello_threads2(socket: &mut Socket, id: i64) -> Value {
    ask(
        socket,
        json!({ "id": id, "cmd": "client.hello", "params": { "capabilities": ["threads2"] } }),
    )
    .await;
    answered(socket, id).await
}

/// Opens a subscription and takes its acknowledgement and empty snapshot.
async fn subscribed_empty(socket: &mut Socket, id: i64, target: Value) {
    ask(socket, json!({ "id": id, "sub": target })).await;
    assert_eq!(heard(socket).await, json!({ "id": id, "ok": true }));
    assert_eq!(heard(socket).await, json!({ "sub": id, "snapshot": [] }));
}

#[tokio::test]
async fn a_tape_carries_the_deltas_nobody_writes_down() {
    let quiet = Arc::new(Quiet::new());
    let deltas = quiet.deltas.clone();
    let (_root, _log, port) = door_with("ephemeral", quiet);

    let mut socket = desk(port).await;
    subscribed_empty(&mut socket, 1, json!({ "tape": "ada" })).await;

    // A delta for somebody else's tape is not this subscription's business.
    let _ = deltas.send(said_in(ThreadId::dm("bob"), "m1", "not here"));
    let _ = deltas.send(said_in(ThreadId::dm("ada"), "m2", "hel"));
    assert_eq!(
        heard(&mut socket).await,
        json!({
            "sub": 1,
            "ephemeral": { "type": "agent_delta", "personaId": "ada", "messageId": "m2", "text": "hel" }
        }),
        "a client that did not declare threads2 is sent the shape the DM has always had"
    );
    let _ = deltas.send(StreamDelta::ThreadDelta {
        thread: ThreadId::dm("ada"),
        message_id: "m3".to_string(),
        kind: DeltaKind::Thought,
        text: "hm".to_string(),
    });
    assert_eq!(
        heard(&mut socket).await["ephemeral"],
        json!({ "type": "thought_delta", "personaId": "ada", "messageId": "m3", "text": "hm" })
    );
}

#[tokio::test]
async fn a_side_thread_carries_only_its_own_deltas_and_never_the_teammates() {
    let quiet = Arc::new(Quiet::new());
    let deltas = quiet.deltas.clone();
    let (_root, _log, port) = door_with("side-ephemeral", quiet);

    let mut socket = desk(port).await;
    subscribed_empty(&mut socket, 1, json!({ "side": "s1" })).await;

    // The teammate's main words, and another thread's, are not this one's.
    let _ = deltas.send(said_in(ThreadId::dm("ada"), "m1", "main"));
    let _ = deltas.send(said_in(ThreadId::side("s2"), "m2", "other"));
    let _ = deltas.send(said_in(ThreadId::side("s1"), "m3", "hel"));
    assert_eq!(
        heard(&mut socket).await,
        json!({
            "sub": 1,
            "ephemeral": { "type": "side_agent_delta", "sideId": "s1", "messageId": "m3", "text": "hel" }
        })
    );
}

#[tokio::test]
async fn a_tape_subscription_never_hears_a_side_threads_deltas() {
    let quiet = Arc::new(Quiet::new());
    let deltas = quiet.deltas.clone();
    let (_root, _log, port) = door_with("side-not-on-tape", quiet);

    let mut socket = desk(port).await;
    subscribed_empty(&mut socket, 1, json!({ "tape": "ada" })).await;
    let _ = deltas.send(said_in(ThreadId::side("ada"), "m1", "side"));
    let _ = deltas.send(said_in(ThreadId::dm("ada"), "m2", "main"));
    assert_eq!(
        heard(&mut socket).await["ephemeral"]["text"],
        "main",
        "the side thread's words stayed out of the main tape"
    );
}

#[tokio::test]
async fn a_threads2_client_is_sent_one_thread_delta_for_every_kind() {
    let quiet = Arc::new(Quiet::new());
    let deltas = quiet.deltas.clone();
    let (_root, _log, port) = door_with("thread-delta", quiet);

    let mut socket = desk(port).await;
    hello_threads2(&mut socket, 1).await;
    // The same thread is named by `threadId` on any kind; the older targets
    // for a tape, a side thread and a run are sent the new delta too.
    subscribed_empty(
        &mut socket,
        2,
        json!({ "threadId": { "kind": "dm", "key": "ada" } }),
    )
    .await;
    subscribed_empty(
        &mut socket,
        3,
        json!({ "threadId": { "kind": "side", "key": "s1" } }),
    )
    .await;
    subscribed_empty(&mut socket, 4, json!({ "run": "r1" })).await;

    let _ = deltas.send(said_in(ThreadId::dm("ada"), "m1", "main"));
    let _ = deltas.send(said_in(ThreadId::side("s1"), "m2", "side"));
    let _ = deltas.send(StreamDelta::ThreadDelta {
        thread: ThreadId::run("r1"),
        message_id: "m3".to_string(),
        kind: DeltaKind::Thought,
        text: "run".to_string(),
    });
    let mut heard_by = HashMap::new();
    for _ in 0..3 {
        let frame = heard_where(&mut socket, |frame| frame.get("ephemeral").is_some()).await;
        heard_by.insert(frame["sub"].as_i64().unwrap(), frame["ephemeral"].clone());
    }
    assert_eq!(
        heard_by[&2],
        json!({ "type": "thread_delta", "thread": { "kind": "dm", "key": "ada" },
                "messageId": "m1", "kind": "text", "text": "main" })
    );
    assert_eq!(
        heard_by[&3],
        json!({ "type": "thread_delta", "thread": { "kind": "side", "key": "s1" },
                "messageId": "m2", "kind": "text", "text": "side" })
    );
    assert_eq!(
        heard_by[&4],
        json!({ "type": "thread_delta", "thread": { "kind": "run", "key": "r1" },
                "messageId": "m3", "kind": "thought", "text": "run" })
    );
}

#[tokio::test]
async fn an_old_client_is_sent_no_delta_for_a_kind_that_never_had_one() {
    let quiet = Arc::new(Quiet::new());
    let deltas = quiet.deltas.clone();
    let (_root, _log, port) = door_with("run-delta", quiet);

    let mut socket = desk(port).await;
    subscribed_empty(&mut socket, 1, json!({ "run": "r1" })).await;
    subscribed_empty(&mut socket, 2, json!({ "tape": "ada" })).await;
    let _ = deltas.send(said_in(ThreadId::run("r1"), "m1", "run"));
    let _ = deltas.send(said_in(ThreadId::dm("ada"), "m2", "main"));
    let frame = heard_where(&mut socket, |frame| frame.get("ephemeral").is_some()).await;
    assert_eq!(frame["sub"], 2, "the run's words were not sent to {frame}");
}

#[tokio::test]
async fn a_phone_may_run_side_threads_the_way_it_runs_a_conversation() {
    for command in [
        Command::SideStart {
            persona_id: "ada".to_string(),
            text: "x".to_string(),
        },
        Command::SidePrompt {
            side_id: "s".to_string(),
            text: "x".to_string(),
            attachments: None,
        },
        Command::SideCancel {
            side_id: "s".to_string(),
        },
        Command::SideArchive {
            side_id: "s".to_string(),
        },
        Command::SideList {
            persona_id: "ada".to_string(),
        },
        Command::SideAnswerPermission {
            side_id: "s".to_string(),
            request_id: "r".to_string(),
            option_id: "o".to_string(),
        },
    ] {
        assert!(Seat::Phone.permits(&command), "{command:?}");
    }
    assert!(Seat::Phone.permits_sub(&Target::Side("s".to_string())));
}

#[tokio::test]
async fn human_answer_is_a_command_the_wire_can_read() {
    let (_root, _log, port) = door("human-answer");
    let mut socket = desk(port).await;

    ask(
        &mut socket,
        json!({
            "id": 1,
            "cmd": "human.answer",
            "params": { "personaId": "ada", "actionId": "act-1", "status": "done" },
        }),
    )
    .await;
    let answered = heard(&mut socket).await;
    assert_eq!(answered["id"], 1);
    assert_eq!(answered["ok"], true, "{answered}");

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "human.answer",
            "params": { "personaId": "ada", "actionId": "act-1", "status": "declined", "note": "not now" },
        }),
    )
    .await;
    let declined = heard(&mut socket).await;
    assert_eq!(declined["ok"], true, "{declined}");
}

/// An id off the wire is not a teammate this room has, and one with no
/// characters in it names the transcripts directory rather than a tape in it.
/// It is answered like any other stranger, because a command that panics is a
/// command that is never answered at all — and it takes the socket with it.
#[tokio::test]
async fn a_command_naming_a_teammate_with_no_id_is_answered() {
    let (_root, log, port) = door("blank-id");
    let _indexer = crate::store::search::Indexer::open(&log).unwrap();
    let mut socket = desk(port).await;

    for (id, cmd, params) in [
        (1, "chapter.list", json!({ "personaId": "" })),
        (2, "peers.list", json!({ "personaId": "" })),
        (
            3,
            "search.thread",
            json!({ "personaId": "", "query": "crane" }),
        ),
        (4, "teammate.tools", json!({ "personaId": "" })),
    ] {
        ask(
            &mut socket,
            json!({ "id": id, "cmd": cmd, "params": params }),
        )
        .await;
        let answer = answered(&mut socket, id).await;
        assert_eq!(answer["ok"], true, "{cmd}: {answer}");
    }

    // The socket is still the one that answered the first of them.
    let created = create(&mut socket, 5, "Ada").await;
    assert_eq!(created["name"], "Ada");
}

/// A subscription to a tape that cannot exist is still a subscription: it is
/// acknowledged, snapshotted empty, and closed by an unsubscribe.
#[tokio::test]
async fn a_tape_subscription_for_a_teammate_with_no_id_snapshots_nothing() {
    let (_root, _log, port) = door("blank-id-sub");
    let mut socket = desk(port).await;

    ask(&mut socket, json!({ "id": 1, "sub": { "tape": "" } })).await;
    let opened = answered(&mut socket, 1).await;
    assert_eq!(opened["ok"], true, "{opened}");
    let snapshot = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        heard_where(&mut socket, |frame| frame["sub"] == 1),
    )
    .await
    .expect("the snapshot never arrived");
    assert_eq!(snapshot["snapshot"], json!([]));
}

#[test]
fn the_desk_seat_may_do_everything_the_room_can_do() {
    assert!(Seat::Desk.permits(&Command::ModelsList {}));
    assert!(Seat::Desk.permits(&Command::ModelsCatalog {
        provider_id: "anthropic".to_string()
    }));
    assert!(Seat::Desk.permits(&Command::PersonaDelete {
        id: "ada".to_string()
    }));
    assert!(Seat::Desk.permits_sub(&Target::Room));
    assert!(Seat::Desk.permits_sub(&Target::View(ViewName::Roster)));
}

#[test]
fn the_phone_seat_may_watch_and_stop_a_computer_but_not_remove_it() {
    let persona_id = "ada".to_string();
    assert!(Seat::Phone.permits(&Command::ComputerStatus {
        persona_id: persona_id.clone()
    }));
    assert!(Seat::Phone.permits(&Command::ComputerStop {
        persona_id: persona_id.clone()
    }));
    assert!(!Seat::Phone.permits(&Command::ComputerRemove {
        persona_id: persona_id.clone()
    }));
    assert!(!Seat::Phone.permits(&Command::ComputerRuntimes {}));
}

/// Who a teammate is — its name and goal — and whether it stays are the
/// person's to change from anywhere; the patch that reaches its grants is
/// not, so the phone gets a narrow edit and never `persona.update`.
#[test]
fn the_phone_seat_renames_and_deletes_a_teammate_but_never_patches_one() {
    assert!(Seat::Phone.permits(&Command::MobilePersonaUpdate {
        id: "ada".to_string(),
        name: Some("Ada Lovelace".to_string()),
        goal: None,
    }));
    assert!(Seat::Phone.permits(&Command::PersonaDelete {
        id: "ada".to_string()
    }));
    assert!(!Seat::Phone.permits(&Command::PersonaUpdate {
        id: "ada".to_string(),
        patch: json!({ "name": "Ada Lovelace" }),
    }));
}

/// A teammate's standing access is the owner phone's to change and never a
/// companion's, and only through the narrow command.
#[test]
fn only_the_owner_phone_changes_a_teammates_access() {
    let access = Command::MobilePersonaAccess {
        id: "ada".to_string(),
        reach: Some(Reach::Machine),
        mode_id: None,
        background_work: Some(true),
    };
    assert!(Seat::Owner.permits(&access));
    assert!(!Seat::Phone.permits(&access));
    {
        let seat = Seat::Phone;
        assert!(!seat.permits(&Command::PersonaUpdate {
            id: "ada".to_string(),
            patch: json!({ "reach": "machine" }),
        }));
        assert!(!seat.permits(&Command::SessionSetMode {
            persona_id: "ada".to_string(),
            mode_id: "bypassPermissions".to_string(),
        }));
    }
    assert!(Seat::Owner.capabilities().contains(&"personaAccess"));
    assert!(!Seat::Phone.capabilities().contains(&"personaAccess"));
}

/// Reach and background work are written for a Hotline Agent teammate, and
/// a mode is refused for one; a harness teammate keeps a mode for its next
/// start, and is refused a reach. Nothing to change is refused too.
#[tokio::test]
async fn mobile_persona_access_sets_reach_mode_and_background_work() {
    let (_root, log, port) = door("mobile-access");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "mobile.persona_access",
            "params": { "id": ada, "reach": "machine", "backgroundWork": true },
        }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["result"]["reach"], "machine");
    assert_eq!(answer["result"]["backgroundWork"], true);

    ask(
        &mut socket,
        json!({
            "id": 3,
            "cmd": "persona.create",
            "params": { "draft": { "name": "Bea", "backendId": "claude-acp" } },
        }),
    )
    .await;
    let bea = answered(&mut socket, 3).await["result"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    ask(
        &mut socket,
        json!({
            "id": 4,
            "cmd": "mobile.persona_access",
            "params": { "id": bea, "modeId": "plan", "backgroundWork": true },
        }),
    )
    .await;
    let answer = answered(&mut socket, 4).await;
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["result"]["modeId"], "plan");
    assert_eq!(answer["result"]["backgroundWork"], true);

    for (n, params, needle) in [
        (5, json!({ "id": ada, "modeId": "plan" }), "no mode"),
        (6, json!({ "id": bea, "reach": "machine" }), "has a reach"),
        (7, json!({ "id": ada }), "Nothing to change"),
        (8, json!({ "id": bea, "modeId": " " }), "needs an id"),
    ] {
        ask(
            &mut socket,
            json!({ "id": n, "cmd": "mobile.persona_access", "params": params }),
        )
        .await;
        let refused = answered(&mut socket, n).await;
        assert_eq!(refused["ok"], false, "{refused}");
        assert!(
            refused["error"].as_str().unwrap().contains(needle),
            "{refused}"
        );
    }
    let kept = room::roster(&log);
    let ada = kept.iter().find(|persona| persona.name == "Ada").unwrap();
    assert_eq!(ada.reach, Some(Reach::Machine));
    assert!(ada.mode_id.is_none());
}

/// The phone's edit writes a name and a goal and nothing else, whatever
/// else its JSON carries; a blank name, an empty edit and a teammate the
/// room does not hold are each refused with a sentence.
#[tokio::test]
async fn mobile_persona_update_changes_only_the_name_and_goal() {
    let (_root, log, port) = door("mobile-update");
    let mut socket = desk(port).await;
    let created = create(&mut socket, 1, "Ada").await;
    let id = created["id"].as_str().unwrap().to_string();

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "mobile.persona_update",
            "params": {
                "id": id,
                "name": "  Ada Lovelace ",
                "goal": "Keep the harbour running",
                "reach": "machine",
                "cwd": "/etc",
                "backgroundWork": true,
                "computer": { "enabled": true },
            },
        }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    let updated = &answer["result"];
    assert_eq!(updated["name"], "Ada Lovelace");
    assert_eq!(updated["goal"], "Keep the harbour running");
    assert!(updated.get("reach").is_none(), "{updated}");
    assert!(updated.get("computer").is_none(), "{updated}");
    assert_eq!(updated["backgroundWork"], false);
    assert_eq!(updated["cwd"], created["cwd"]);

    // An absent field is left alone; an empty goal clears it.
    ask(
        &mut socket,
        json!({ "id": 3, "cmd": "mobile.persona_update", "params": { "id": id, "goal": " " } }),
    )
    .await;
    let cleared = answered(&mut socket, 3).await["result"].clone();
    assert_eq!(cleared["name"], "Ada Lovelace");
    assert_eq!(cleared["goal"], "");

    for (n, params, needle) in [
        (4, json!({ "id": id, "name": "   " }), "needs a name"),
        (5, json!({ "id": id }), "Nothing to change"),
        (6, json!({ "id": "nobody", "name": "Bob" }), ""),
    ] {
        ask(
            &mut socket,
            json!({ "id": n, "cmd": "mobile.persona_update", "params": params }),
        )
        .await;
        let refused = answered(&mut socket, n).await;
        assert_eq!(refused["ok"], false, "{refused}");
        assert!(
            refused["error"].as_str().unwrap().contains(needle),
            "{refused}"
        );
    }
    let kept = room::roster(&log);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].name, "Ada Lovelace");
}

/// The phone gets a narrow create of its own, and a read of what harnesses
/// this desk can run — but never the full `persona.create`, which can name
/// a reach, a path and a computer the phone posture must not touch.
#[test]
fn the_phone_seat_creates_a_teammate_narrowly_but_not_with_persona_create() {
    assert!(Seat::Phone.permits(&Command::MobilePersonaCreate {
        request_id: "3fbb7d63-0a3e-4c6a-9c0e-8f6f8e8e6b39".to_string(),
        name: "Ada".to_string(),
        goal: None,
        backend_id: None,
        model_id: None,
        effort_id: None,
    }));
    assert!(Seat::Phone.permits(&Command::BackendsList {}));
    assert!(!Seat::Phone.permits(&Command::PersonaCreate {
        draft: PersonaDraft {
            name: "Ada".to_string(),
            goal: None,
            team: None,
            backend_id: None,
            cwd: None,
            reach: None,
            model_id: None,
            effort_id: None,
            computer: None,
            background_work: None,
        }
    }));
}

/// A file a teammate sent is part of the conversation the phone reads, so
/// the phone reads the file too — by its message, never by a path.
#[test]
fn the_phone_seat_reads_a_sent_file_by_its_message() {
    assert!(Seat::Phone.permits(&Command::FileRead {
        persona_id: "ada".to_string(),
        event_id: "e1".to_string(),
        index: None,
        offset: 0,
        size: None,
    }));
}

#[tokio::test]
async fn a_sent_file_is_read_by_its_message_a_part_at_a_time() {
    let (root, _log, port) = door("file-read");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap();
    let kept = paths::sent_file_dir(&root, id, "e1").unwrap();
    std::fs::create_dir_all(&kept).unwrap();
    std::fs::write(kept.join("report.txt"), b"quarterly numbers\n").unwrap();

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "file.read", "params": { "personaId": id, "eventId": "e1" } }),
    )
    .await;
    let read = answered(&mut socket, 2).await;
    assert_eq!(read["ok"], true, "{read}");
    assert_eq!(read["result"]["name"], "report.txt");
    assert_eq!(read["result"]["mimeType"], "text/plain");
    assert_eq!(read["result"]["size"], 18);
    assert_eq!(read["result"]["offset"], 0);
    assert_eq!(read["result"]["data"], "cXVhcnRlcmx5IG51bWJlcnMK");
    assert_eq!(read["result"].get("next"), None);

    ask(
        &mut socket,
        json!({ "id": 3, "cmd": "file.read", "params": { "personaId": id, "eventId": "e1", "offset": 10 } }),
    )
    .await;
    let rest = answered(&mut socket, 3).await;
    assert_eq!(rest["result"]["offset"], 10);
    assert_eq!(rest["result"]["data"], "bnVtYmVycwo=");

    for (n, params, error) in [
        (
            4,
            json!({ "personaId": id, "eventId": "e2" }),
            "That message has no file.",
        ),
        (
            5,
            json!({ "personaId": id, "eventId": "../e1" }),
            "That message has no file.",
        ),
        (
            6,
            json!({ "personaId": id, "eventId": "e1", "offset": 19 }),
            "The file is 18 bytes; 19 is not a place in it.",
        ),
        (
            7,
            json!({ "personaId": "nobody", "eventId": "e1" }),
            "There is no teammate nobody.",
        ),
        (
            8,
            json!({ "personaId": id, "eventId": "e1", "index": 1 }),
            "A teammate's file has only attachment index zero.",
        ),
    ] {
        ask(
            &mut socket,
            json!({ "id": n, "cmd": "file.read", "params": params }),
        )
        .await;
        let refused = answered(&mut socket, n).await;
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(refused["error"], error);
    }
}

/// A picture is no more private than the name beside it, so every seat reads
/// one, and only by the hash the roster names.
#[tokio::test]
async fn a_teammates_picture_is_read_by_its_hash_and_by_every_seat() {
    let command = Command::AvatarRead {
        persona_id: "ada".to_string(),
        hash: "0".repeat(64),
        offset: 0,
        size: None,
    };
    for seat in [Seat::Desk, Seat::Owner, Seat::Phone] {
        assert!(seat.permits(&command));
    }

    let (root, _log, port) = door("avatar-read");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap();
    let hash = "ab".repeat(32);
    let path = paths::avatar_path(&root, id, &hash).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(b"pixels");
    std::fs::write(&path, &png).unwrap();

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "avatar.read", "params": { "personaId": id, "hash": hash } }),
    )
    .await;
    let read = answered(&mut socket, 2).await;
    assert_eq!(read["ok"], true, "{read}");
    assert_eq!(read["result"]["mimeType"], "image/png");
    assert_eq!(read["result"]["size"], png.len());
    assert_eq!(read["result"].get("next"), None);

    for (n, params, error) in [
        (
            3,
            json!({ "personaId": id, "hash": "../../room.jsonl" }),
            "A picture is named by 64 lowercase hex digits.",
        ),
        (
            4,
            json!({ "personaId": id, "hash": "AB".repeat(32) }),
            "A picture is named by 64 lowercase hex digits.",
        ),
        (
            5,
            json!({ "personaId": id, "hash": "cd".repeat(32) }),
            "That teammate has no such picture.",
        ),
        (
            6,
            json!({ "personaId": "nobody", "hash": hash }),
            "There is no teammate nobody.",
        ),
    ] {
        ask(
            &mut socket,
            json!({ "id": n, "cmd": "avatar.read", "params": params }),
        )
        .await;
        let refused = answered(&mut socket, n).await;
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(refused["error"], error);
    }
}

/// The owner's "Use initial" clears a picture, and can set nothing else there.
#[tokio::test]
async fn a_patch_can_clear_a_picture_but_not_invent_one() {
    let (root, _log, port) = door("avatar-clear");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap();
    let kept = paths::avatar_path(&root, id, &"ab".repeat(32)).unwrap();
    std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
    std::fs::write(&kept, b"png").unwrap();

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "persona.update", "params": { "id": id, "patch": {
            "avatar": { "hash": "ab".repeat(32), "by": "person", "updatedAt": "now" } } } }),
    )
    .await;
    let refused = answered(&mut socket, 2).await;
    assert_eq!(refused["ok"], false, "{refused}");

    ask(
        &mut socket,
        json!({ "id": 3, "cmd": "persona.update", "params": { "id": id, "patch": { "avatar": null } } }),
    )
    .await;
    let cleared = answered(&mut socket, 3).await;
    assert_eq!(cleared["ok"], true, "{cleared}");
    assert!(!kept.exists());
}

/// Cookie import reads the person's own machine, so it is the desk's alone:
/// the phone cannot list host browsers, preview, or import, and there is no
/// agent tool for any of it. This is the enforcement point behind the promise
/// that the agent can never pull cookies itself.
#[test]
fn only_the_desk_seat_may_import_host_cookies() {
    let browsers = Command::ComputerBrowsersList {};
    let preview = Command::ComputerCookiesPreview {
        browser_id: "chrome".to_string(),
        profile_id: "Default".to_string(),
    };
    let import = Command::ComputerCookiesImport {
        persona_id: "ada".to_string(),
        browser_id: "chrome".to_string(),
        profile_id: "Default".to_string(),
        domains: vec!["example.com".to_string()],
    };
    let list = Command::ComputerCookiesList {
        persona_id: "ada".to_string(),
    };
    let forget = Command::ComputerCookiesForget {
        persona_id: "ada".to_string(),
        browser_id: "chrome".to_string(),
        profile_id: "Default".to_string(),
        domain: Some("example.com".to_string()),
    };
    for command in [&browsers, &preview, &import, &list, &forget] {
        assert!(Seat::Desk.permits(command), "{command:?}");
        assert!(!Seat::Phone.permits(command), "{command:?}");
    }
}

/// Stored secrets are the desk's alone: the phone can neither list, store
/// nor delete one, and there is no agent tool for any of it. With the grant
/// itself living on `persona.update`, which the phone may not send either,
/// nothing a model says can put a secret in front of a teammate.
#[test]
fn only_the_desk_seat_may_touch_stored_secrets() {
    let list = Command::SecretsList {};
    let set = Command::SecretsSet {
        name: "GITHUB_TOKEN".to_string(),
        value: "ghp_notarealtoken0001".to_string(),
    };
    let delete = Command::SecretsDelete {
        name: "GITHUB_TOKEN".to_string(),
    };
    let login = Command::SecretsLoginSet {
        name: "GITHUB_LOGIN".to_string(),
        sites: vec!["https://github.com".to_string()],
        username: "george".to_string(),
        password: "correct-horse-battery".to_string(),
        totp: None,
    };
    let register = Command::SecretsPasskeyRegister {
        name: "GITHUB_PASSKEY".to_string(),
        persona_id: "ada".to_string(),
        rp_id: "github.com".to_string(),
    };
    let registration = Command::SecretsPasskeyRegistration {
        persona_id: "ada".to_string(),
    };
    let cancel = Command::SecretsPasskeyCancel {
        persona_id: "ada".to_string(),
    };
    for command in [
        &list,
        &set,
        &delete,
        &login,
        &register,
        &registration,
        &cancel,
    ] {
        assert!(Seat::Desk.permits(command), "{command:?}");
        assert!(!Seat::Phone.permits(command), "{command:?}");
    }
    assert!(!Seat::Phone.permits(&Command::PersonaUpdate {
        id: "ada".to_string(),
        patch: serde_json::json!({"computer": {"enabled": true, "secrets": ["GITHUB_TOKEN"]}}),
    }));
}

/// Only person seats stop or resume automatic exchanges.
#[test]
fn the_phone_seat_resumes_and_stops_exchanges_for_the_person() {
    let (a, b) = ("ada".to_string(), "bob".to_string());
    for command in [
        Command::TeammatesExchangeStop {
            a: a.clone(),
            b: b.clone(),
        },
        Command::TeammatesExchangeResume { a, b },
    ] {
        assert!(Seat::Desk.permits(&command), "{command:?}");
        assert!(Seat::Phone.permits(&command), "{command:?}");
    }
}

/// A phone answers what a teammate is waiting on and sets how it thinks.
/// What it may reach, and any standing posture, stay at the desk.
#[test]
fn the_phone_seat_answers_for_the_person_but_never_grants_a_standing_one() {
    let persona_id = "ada".to_string();
    assert!(Seat::Phone.permits(&Command::HumanAnswer {
        persona_id: persona_id.clone(),
        action_id: "act".to_string(),
        status: crate::contract::HumanAnswer::Done,
        note: None,
    }));
    // A passkey card is one answer to one request the person armed for at
    // the desk; the arming, and the register that starts it, stay there.
    assert!(Seat::Phone.permits(&Command::SecretsPasskeyAnswer {
        persona_id: persona_id.clone(),
        ask_id: "ask-1".to_string(),
        approved: true,
    }));
    assert!(Seat::Phone.permits(&Command::SessionSetModel {
        persona_id: persona_id.clone(),
        model_id: "anthropic/claude".to_string(),
    }));
    assert!(Seat::Phone.permits(&Command::SessionSetConfig {
        persona_id: persona_id.clone(),
        config_id: "effort".to_string(),
        value: "high".to_string(),
    }));
    // One answer to one request, wherever the person is standing.
    assert!(Seat::Phone.permits(&Command::SessionAnswerPermission {
        persona_id: persona_id.clone(),
        request_id: "req".to_string(),
        option_id: "allow".to_string(),
    }));
    // What a resting teammate could be set to is a list, not a grant.
    assert!(Seat::Phone.permits(&Command::ModelsList {}));
    assert!(Seat::Phone.permits(&Command::ModelsEfforts {
        model_id: "anthropic/claude".to_string(),
    }));
    // A standing grant is a different thing, and a harness mode is one.
    assert!(!Seat::Phone.permits(&Command::SessionSetMode {
        persona_id: persona_id.clone(),
        mode_id: "bypassPermissions".to_string(),
    }));
    assert!(!Seat::Phone.permits(&Command::PersonaUpdate {
        id: persona_id,
        patch: Default::default(),
    }));
}

/// A phone reads one teammate's schedules and nothing it could change them
/// or the room with: no job made, cancelled or quieted, and no room stream,
/// which is where a setting would reach it. A thread between two teammates
/// and a subagent's run are read like a tape: they hold what was said.
#[test]
fn the_phone_seat_reads_a_teammates_schedules_but_changes_none_of_them() {
    use crate::contract::ScheduleKind;
    assert!(Seat::Phone.permits_sub(&Target::Schedules("ada".to_string())));
    assert!(Seat::Phone.permits_sub(&Target::Tape("ada".to_string())));
    assert!(Seat::Phone.permits_sub(&Target::Thread("ada~bob".to_string())));
    assert!(Seat::Phone.permits_sub(&Target::View(ViewName::Roster)));
    assert!(Seat::Phone.permits_sub(&Target::Run("run-1".to_string())));
    assert!(!Seat::Phone.permits_sub(&Target::Room));
    let create = Command::ScheduleCreate {
        persona_id: "ada".to_string(),
        kind: ScheduleKind::Loop,
        when: None,
        every: Some(60_000),
        prompt: "sweep the inbox".to_string(),
        quiet: None,
    };
    let cancel = Command::ScheduleCancel {
        id: "job-1".to_string(),
    };
    let quiet = Command::ScheduleSetQuiet {
        id: "job-1".to_string(),
        quiet: true,
    };
    // The whole room's list is every teammate's; a phone asks for one.
    let list = Command::ScheduleList {};
    let settings = Command::SettingsUpdate {
        patch: Default::default(),
    };
    for command in [&create, &cancel, &quiet, &list, &settings] {
        assert!(Seat::Desk.permits(command), "{command:?}");
        assert!(!Seat::Phone.permits(command), "{command:?}");
    }
    assert!(Seat::Desk.permits_sub(&Target::Schedules("ada".to_string())));
}

fn scheduled(
    id: &str,
    persona_id: &str,
    kind: crate::contract::ScheduleKind,
    next_at: i64,
) -> crate::contract::ScheduledJob {
    use crate::contract::ScheduleKind;
    crate::contract::ScheduledJob {
        id: id.to_string(),
        persona_id: persona_id.to_string(),
        kind,
        when: (kind == ScheduleKind::Schedule).then_some(next_at),
        every: (kind == ScheduleKind::Loop).then_some(60_000),
        prompt: format!("{id}, please"),
        quiet: None,
        operator_created: true,
        next_at,
        created_at: 1_700_000_000_000,
    }
}

#[tokio::test]
async fn a_teammates_schedules_are_the_whole_list_on_opening_and_again_on_each_change() {
    use crate::contract::ScheduleKind;
    let (_root, log, port) = door("schedules-view");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let bob = create(&mut socket, 2, "Bob").await;
    let ada_id = ada["id"].as_str().unwrap();
    let bob_id = bob["id"].as_str().unwrap();

    // Nothing scheduled is an answer, and it is said: `ok`, then an empty
    // list.
    ask(
        &mut socket,
        json!({ "id": 3, "sub": { "schedules": ada_id } }),
    )
    .await;
    assert_eq!(heard(&mut socket).await, json!({ "id": 3, "ok": true }));
    assert_eq!(
        heard(&mut socket).await,
        json!({ "sub": 3, "snapshot": [] })
    );

    let once = scheduled(
        "job-once",
        ada_id,
        ScheduleKind::Schedule,
        1_700_000_900_000,
    );
    room::append_schedule(&log, &once).unwrap();
    let first = heard(&mut socket).await;
    assert_eq!(
        first,
        json!({ "sub": 3, "snapshot": [{
            "id": "job-once",
            "personaId": ada_id,
            "kind": "schedule",
            "prompt": "job-once, please",
            "when": 1_700_000_900_000_i64,
            "nextAt": 1_700_000_900_000_i64,
        }] })
    );

    let mut sweep = scheduled("job-loop", ada_id, ScheduleKind::Loop, 1_700_000_100_000);
    sweep.quiet = Some(true);
    room::append_schedule(&log, &sweep).unwrap();
    let both = heard(&mut socket).await;
    let ids: Vec<&str> = both["snapshot"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["job-loop", "job-once"], "{both}");
    assert_eq!(both["snapshot"][0]["every"], 60_000);
    assert_eq!(both["snapshot"][0]["quiet"], true);
    assert!(both["snapshot"][0].get("when").is_none(), "{both}");

    // Another teammate's job changes nothing here, so nothing is sent: the
    // next frame is the loop's next run moving, once.
    room::append_schedule(
        &log,
        &scheduled("job-bob", bob_id, ScheduleKind::Loop, 1_700_000_050_000),
    )
    .unwrap();
    sweep.next_at += 60_000;
    room::append_schedule(&log, &sweep).unwrap();
    let moved = heard(&mut socket).await;
    assert_eq!(moved["snapshot"].as_array().unwrap().len(), 2, "{moved}");
    assert_eq!(moved["snapshot"][0]["nextAt"], 1_700_000_160_000_i64);
    assert!(!moved.to_string().contains("job-bob"), "{moved}");

    // A one-shot that fired, or was cancelled, is a tombstone that names no
    // teammate; the list without it is the answer either way.
    room::tombstone_schedule(&log, "job-once").unwrap();
    let fired = heard(&mut socket).await;
    assert_eq!(fired["snapshot"].as_array().unwrap().len(), 1, "{fired}");
    assert_eq!(fired["snapshot"][0]["id"], "job-loop");

    // A teammate deleted takes its list with it, and the view ends.
    ask(
        &mut socket,
        json!({ "id": 4, "cmd": "persona.delete", "params": { "id": ada_id } }),
    )
    .await;
    let (mut deleted, mut removed) = (None, None);
    while deleted.is_none() || removed.is_none() {
        let frame = heard(&mut socket).await;
        if frame["id"] == 4 {
            deleted = Some(frame);
        } else if frame["sub"] == 3 {
            removed = Some(frame);
        }
    }
    assert_eq!(deleted.unwrap()["ok"], true);
    assert_eq!(removed.unwrap(), json!({ "sub": 3, "removed": ada_id }));
}

#[tokio::test]
async fn a_teammate_the_room_does_not_hold_or_cannot_read_is_refused_rather_than_empty() {
    let (root, _log, port) = door("schedules-refused");
    let mut socket = desk(port).await;

    ask(
        &mut socket,
        json!({ "id": 1, "sub": { "schedules": "nobody" } }),
    )
    .await;
    assert_eq!(
        heard(&mut socket).await,
        json!({
            "id": 1,
            "ok": false,
            "error": "There is no teammate nobody.",
            "code": "unknown_teammate",
        })
    );

    // A room stream that is there but cannot be read has no answer to give.
    std::fs::create_dir_all(root.join("room.jsonl")).unwrap();
    ask(
        &mut socket,
        json!({ "id": 2, "sub": { "schedules": "ada" } }),
    )
    .await;
    let unreadable = heard(&mut socket).await;
    assert_eq!(unreadable["ok"], false, "{unreadable}");
    assert_eq!(unreadable["code"], "unreadable", "{unreadable}");

    // Neither refusal left a subscription behind to reuse the id.
    let _ = std::fs::remove_dir(root.join("room.jsonl"));
    create(&mut socket, 3, "Ada").await;
    ask(
        &mut socket,
        json!({ "id": 1, "sub": { "schedules": "nobody" } }),
    )
    .await;
    assert_eq!(heard(&mut socket).await["code"], "unknown_teammate");
}

/// Behind on the room stream, the view reads the list again rather than
/// missing a change; unable to read it later, it says so and ends, since a
/// list it cannot refresh is no longer one to show as current.
#[tokio::test]
async fn a_schedules_view_catches_up_after_falling_behind_and_ends_when_it_cannot_read() {
    use crate::contract::ScheduleKind;
    let root = scratch("schedules-lag");
    let log = Log::open(&root);
    log.append(
        &StreamId::Room,
        &json!({"kind": "persona", "id": "ada", "name": "Ada", "goal": "", "backendId": "hotline", "cwd": "/tmp", "mcpPolicy": {"mode": "none", "serverIds": []}, "sessionCheckpoints": [], "createdAt": 1, "updatedAt": 1}),
    )
    .unwrap();
    room::append_schedule(
        &log,
        &scheduled("job-1", "ada", ScheduleKind::Loop, 1_700_000_100_000),
    )
    .unwrap();

    let (events, room_events) = broadcast::channel(1);
    for n in 0..3 {
        events.send(json!({"kind": "setting", "id": n})).unwrap();
    }
    let (tx, mut frames) = mpsc::unbounded_channel();
    let outbox = Outbox {
        auth_attempts: Arc::default(),
        pairing: Arc::default(),
        uploads: Arc::default(),
        threads2: Arc::default(),
        lean: Arc::default(),
        sender: Outgoing::Desk(tx),
        cancel: tokio_util::sync::CancellationToken::new(),
        max: usize::MAX,
    };
    let view = tokio::spawn(schedules::view(
        9,
        "ada".to_string(),
        log.clone(),
        room_events,
        Vec::new(),
        outbox,
    ));
    let caught_up: Value = serde_json::from_str(&frames.recv().await.unwrap()).unwrap();
    assert_eq!(caught_up["sub"], 9);
    assert_eq!(caught_up["snapshot"][0]["id"], "job-1", "{caught_up}");

    std::fs::remove_file(root.join("room.jsonl")).unwrap();
    std::fs::create_dir_all(root.join("room.jsonl")).unwrap();
    events
        .send(json!({"kind": "schedule", "id": "job-1", "deleted": true}))
        .unwrap();
    let ended: Value = serde_json::from_str(&frames.recv().await.unwrap()).unwrap();
    assert_eq!(ended["sub"], 9);
    assert_eq!(ended["code"], "unreadable", "{ended}");
    tokio::time::timeout(std::time::Duration::from_secs(5), view)
        .await
        .expect("the view ended")
        .unwrap();
}

/// The seat lets `session.set_config` through, so the narrowing happens where
/// the categories are known: effort is the one a phone may set, whatever a
/// harness happens to call it, and anything else a harness serves is not.
#[test]
fn only_a_config_the_session_calls_effort_is_a_phones_to_set() {
    use crate::contract::{SessionConfig, SessionConfigCategory};
    let room = Arc::new(Quiet::new());
    let mut info = idle("ada");
    info.configs = vec![
        SessionConfig {
            id: "reasoning".to_string(),
            name: "Reasoning".to_string(),
            category: Some(SessionConfigCategory::Effort),
            current_id: Some("high".to_string()),
            options: Vec::new(),
        },
        SessionConfig {
            id: "approval".to_string(),
            name: "Approvals".to_string(),
            category: None,
            current_id: Some("ask".to_string()),
            options: Vec::new(),
        },
    ];
    room.set_info(info);
    let handle: Arc<dyn RoomHandle> = room;
    assert!(super::effort_config(&handle, "ada", "reasoning"));
    assert!(!super::effort_config(&handle, "ada", "approval"));
    assert!(!super::effort_config(
        &handle,
        "ada",
        "nothing-by-that-name"
    ));
    assert!(!super::effort_config(&handle, "bob", "reasoning"));
}

/// A frame the phone can afford travels as a small JPEG; one it cannot
/// decode is stripped as before, and text is untouched either way.
#[test]
fn a_computer_frame_reaches_the_phone_as_a_small_jpeg_or_not_at_all() {
    use base64::{Engine, prelude::BASE64_STANDARD};
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbImage::from_fn(1600, 900, |x, y| {
        image::Rgb([(x % 256) as u8, (y % 256) as u8, 90])
    })
    .write_to(&mut png, image::ImageFormat::Png)
    .unwrap();
    let frame = json!({
        "kind": "computer_frame", "id": "f1", "ts": 1,
        "dataUrl": format!("data:image/png;base64,{}", BASE64_STANDARD.encode(png.into_inner())),
    });
    let small = phone_event(frame);
    let data_url = small["dataUrl"].as_str().unwrap();
    assert!(data_url.starts_with("data:image/jpeg;base64,"));
    let jpeg = BASE64_STANDARD
        .decode(&data_url["data:image/jpeg;base64,".len()..])
        .unwrap();
    assert!(jpeg.len() <= 96 * 1024);
    let decoded = image::load_from_memory(&jpeg).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (720, 405));
    assert!(small.get("mobileTruncated").is_none());

    let junk = json!({"kind": "computer_frame", "id": "f2", "ts": 2, "dataUrl": "data:image/png;base64,AAAA"});
    let stripped = phone_event(junk);
    assert!(stripped.get("dataUrl").is_none());
    assert_eq!(stripped["mobileTruncated"], true);
}

#[test]
fn a_hung_up_client_and_an_interrupted_accept_are_waited_out() {
    assert_eq!(
        accept_again(&std::io::Error::from(std::io::ErrorKind::ConnectionAborted)),
        Some(AcceptAgain::Now)
    );
    assert_eq!(
        accept_again(&std::io::Error::from(std::io::ErrorKind::Interrupted)),
        Some(AcceptAgain::Now)
    );
    assert_eq!(
        accept_again(&std::io::Error::from(std::io::ErrorKind::NotConnected)),
        None
    );
}

#[cfg(unix)]
#[test]
fn too_many_open_files_pauses_and_a_gone_listener_does_not() {
    assert_eq!(
        accept_again(&std::io::Error::from_raw_os_error(libc::ECONNABORTED)),
        Some(AcceptAgain::Now)
    );
    assert_eq!(
        accept_again(&std::io::Error::from_raw_os_error(libc::EINTR)),
        Some(AcceptAgain::Now)
    );
    assert_eq!(
        accept_again(&std::io::Error::from_raw_os_error(libc::EMFILE)),
        Some(AcceptAgain::AfterPause)
    );
    assert_eq!(
        accept_again(&std::io::Error::from_raw_os_error(libc::ENFILE)),
        Some(AcceptAgain::AfterPause)
    );
    assert_eq!(
        accept_again(&std::io::Error::from_raw_os_error(libc::EBADF)),
        None
    );
}

#[cfg(windows)]
#[test]
fn winsock_resource_pressure_pauses_without_losing_the_listener() {
    use windows_sys::Win32::Networking::WinSock::{
        WSAECONNRESET, WSAEMFILE, WSAENOBUFS, WSAENOTSOCK,
    };
    for code in [WSAEMFILE, WSAENOBUFS] {
        assert_eq!(
            accept_again(&std::io::Error::from_raw_os_error(code)),
            Some(AcceptAgain::AfterPause)
        );
    }
    assert_eq!(
        accept_again(&std::io::Error::from_raw_os_error(WSAECONNRESET)),
        Some(AcceptAgain::Now)
    );
    assert_eq!(
        accept_again(&std::io::Error::from_raw_os_error(WSAENOTSOCK)),
        None
    );
}

/// A stream subscription that ends without an unsubscribe used to keep its
/// id forever, so the next client that reused the number was told it was
/// already open for a task that would never deliver.
#[tokio::test]
async fn a_subscription_id_is_free_once_its_task_has_ended() {
    let (_root, log, port) = door("sub-ended");
    let mut socket = desk(port).await;

    ask(&mut socket, json!({ "id": 7, "sub": "room" })).await;
    assert_eq!(heard(&mut socket).await, json!({ "id": 7, "ok": true }));
    assert_eq!(
        heard(&mut socket).await,
        json!({ "sub": 7, "snapshot": [] })
    );

    log.close_broadcasts();

    let reused = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            ask(&mut socket, json!({ "id": 7, "sub": "room" })).await;
            let answer = answered(&mut socket, 7).await;
            if answer["ok"] == true {
                return answer;
            }
            let error = answer["error"].as_str().unwrap_or("");
            assert!(
                error.contains("already open"),
                "waiting for the ended task to free the id, got {answer}"
            );
        }
    })
    .await
    .expect("the ended subscription never freed its id");
    assert_eq!(reused["ok"], true, "{reused}");
}

#[tokio::test]
async fn teammate_tools_answers_null_as_a_result_not_a_void() {
    let (_root, _log, port) = door("tools-null");
    let mut socket = desk(port).await;
    let created = create(&mut socket, 1, "Ada").await;
    let persona_id = created["id"].as_str().unwrap();

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "teammate.tools", "params": { "personaId": persona_id } }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    let fields = answer.as_object().expect("an answer is an object");
    assert!(
        fields.contains_key("result"),
        "teammate.tools with no ledger omitted result: {answer}"
    );
    assert_eq!(fields.get("result"), Some(&Value::Null));
}

#[tokio::test]
async fn a_key_provider_cannot_start_a_device_login() {
    let (_root, _log, port) = door("login-key");
    let mut socket = desk(port).await;
    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "credential.login", "params": { "providerId": "anthropic" } }),
    )
    .await;
    let refused = answered(&mut socket, 1).await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(
        refused["error"].as_str(),
        Some("Anthropic takes an API key, not a sign-in.")
    );
}

#[tokio::test]
async fn an_unknown_login_id_is_an_error() {
    let (_root, _log, port) = door("login-status");
    let mut socket = desk(port).await;
    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "credential.login_status", "params": { "loginId": "nobody" } }),
    )
    .await;
    let refused = answered(&mut socket, 1).await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"].as_str(), Some("There is no login nobody."));
}

#[tokio::test]
async fn credential_refresh_models_needs_a_login_and_refuses_a_key() {
    let (_root, _log, port) = door("refresh-models");
    let mut socket = desk(port).await;

    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "credential.refresh_models", "params": { "providerId": "anthropic" } }),
    )
    .await;
    let key = answered(&mut socket, 1).await;
    assert_eq!(key["ok"], false, "{key}");
    assert_eq!(
        key["error"].as_str(),
        Some("Anthropic takes an API key, not a sign-in.")
    );

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "credential.refresh_models", "params": { "providerId": "github-copilot" } }),
    )
    .await;
    let unsigned = answered(&mut socket, 2).await;
    assert_eq!(unsigned["ok"], false, "{unsigned}");
    assert_eq!(
        unsigned["error"].as_str(),
        Some("There is no sign-in for GitHub Copilot.")
    );
}

#[tokio::test]
async fn models_catalog_lists_a_wired_provider_and_refuses_an_unwired_one() {
    let (_root, _log, port) = door("models-catalog");
    let mut socket = desk(port).await;

    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "models.catalog", "params": { "providerId": "nope" } }),
    )
    .await;
    let refused = answered(&mut socket, 1).await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(
        refused["error"].as_str(),
        Some("nope is not a provider Hotline Agent can use.")
    );

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "models.catalog", "params": { "providerId": "anthropic" } }),
    )
    .await;
    let listed = answered(&mut socket, 2).await;
    assert_eq!(listed["ok"], true, "{listed}");
    let models = listed["result"].as_array().expect("a catalogue is a list");
    assert!(!models.is_empty());
    assert!(
        models.iter().all(|model| model["enabled"] == true),
        "{listed}"
    );
    assert!(
        models
            .iter()
            .all(|model| model["id"].as_str().is_some_and(|id| !id.is_empty())),
        "{listed}"
    );
}

/// Setting a model on an idle Hotline Agent teammate writes the choice on the
/// persona and remembers it as the last model used, without needing a live
/// session to hold it.
#[tokio::test]
async fn session_set_model_on_an_idle_hotline_agent_writes_the_persona_and_last_used() {
    let (_root, log, port) = door("set-model-idle");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap();
    assert!(ada.get("modelId").is_none(), "{ada}");

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "session.set_model",
            "params": { "personaId": id, "modelId": "openai/gpt" },
        }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["result"]["state"], "idle");
    assert_eq!(answer["result"]["personaId"], id);

    let roster = room::roster(&log);
    assert_eq!(roster[0].model_id.as_deref(), Some("openai/gpt"));
    assert_eq!(room::settings(&log)["lastModelId"], "openai/gpt");
}

#[tokio::test]
async fn session_set_model_refuses_a_hotline_agent_id_the_desk_cannot_reach() {
    let (_root, _log, port) = door("set-model-unknown");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap();

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "session.set_model",
            "params": { "personaId": id, "modelId": "nope/nope" },
        }),
    )
    .await;
    let refused = answered(&mut socket, 2).await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(
        refused["error"].as_str(),
        Some("nope/nope is not a model this desk can reach.")
    );
}

/// An ACP harness names its own models. The wire stores what it is given
/// and lets the child refuse an id it does not know, once it is live.
#[tokio::test]
async fn session_set_model_accepts_an_arbitrary_id_on_an_acp_teammate() {
    let (_root, log, port) = door("set-model-acp");
    let mut socket = desk(port).await;
    let draft = PersonaDraft {
        name: "Bob".to_string(),
        goal: None,
        team: None,
        backend_id: Some("cursor".to_string()),
        cwd: None,
        reach: None,
        model_id: None,
        effort_id: None,
        computer: None,
        background_work: None,
    };
    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "persona.create", "params": { "draft": draft } }),
    )
    .await;
    let created = answered(&mut socket, 1).await["result"].clone();
    let id = created["id"].as_str().unwrap();

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "session.set_model",
            "params": { "personaId": id, "modelId": "cursor-special" },
        }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(
        room::roster(&log)[0].model_id.as_deref(),
        Some("cursor-special")
    );
    assert!(
        room::settings(&log).get("lastModelId").is_none(),
        "an ACP choice is not the room's last Hotline Agent model"
    );
}

fn a_model_with_efforts() -> (String, Vec<String>) {
    crate::models::catalog()
        .providers
        .iter()
        .find_map(|(provider, entry)| {
            entry.models.iter().find_map(|(id, model)| {
                (!model.efforts.is_empty())
                    .then(|| (format!("{provider}/{id}"), model.efforts.clone()))
            })
        })
        .expect("the snapshot has a model with an effort list")
}

/// The idle picker's reply names the effort a teammate with none stored
/// runs at, so the strip shows `high` before any session has said so.
#[tokio::test]
async fn models_efforts_names_the_default_a_blank_teammate_runs_at() {
    let (_root, _log, port) = door("efforts-default");
    let mut socket = desk(port).await;
    let (model, offered) = a_model_with_efforts();
    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "models.efforts", "params": { "modelId": model } }),
    )
    .await;
    let answer = answered(&mut socket, 1).await;
    assert_eq!(answer["ok"], true, "{answer}");
    let ids: Vec<&str> = answer["result"]["choices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|one| one["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, offered.iter().map(String::as_str).collect::<Vec<_>>());
    let expected = offered.iter().any(|one| one == "high").then_some("high");
    assert_eq!(answer["result"]["defaultId"].as_str(), expected, "{answer}");
}

/// Setting effort on an idle Hotline Agent teammate writes it on the persona
/// and answers idle info, without needing a live session to hold it.
#[tokio::test]
async fn session_set_config_on_an_idle_hotline_agent_writes_effort_id() {
    let (_root, log, port) = door("set-config-idle");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap();
    assert!(ada.get("effortId").is_none(), "{ada}");

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "session.set_config",
            "params": { "personaId": id, "configId": "effort", "value": "high" },
        }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["result"]["state"], "idle");
    assert_eq!(room::roster(&log)[0].effort_id.as_deref(), Some("high"));

    ask(
        &mut socket,
        json!({
            "id": 3,
            "cmd": "session.set_config",
            "params": { "personaId": id, "configId": "effort", "value": "" },
        }),
    )
    .await;
    let cleared = answered(&mut socket, 3).await;
    assert_eq!(cleared["ok"], true, "{cleared}");
    assert!(
        room::roster(&log)[0].effort_id.is_none(),
        "{:?}",
        room::roster(&log)[0]
    );
}

#[tokio::test]
async fn session_set_config_refuses_an_effort_the_model_does_not_list() {
    let (model, listed) = a_model_with_efforts();
    let (_root, _log, port) = door("set-config-unknown");
    let mut socket = desk(port).await;
    let draft = PersonaDraft {
        name: "Ada".to_string(),
        goal: None,
        team: None,
        backend_id: None,
        cwd: None,
        reach: None,
        model_id: Some(model.clone()),
        effort_id: None,
        computer: None,
        background_work: None,
    };
    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "persona.create", "params": { "draft": draft } }),
    )
    .await;
    let created = answered(&mut socket, 1).await["result"].clone();
    let id = created["id"].as_str().unwrap();

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "session.set_config",
            "params": { "personaId": id, "configId": "effort", "value": "nope" },
        }),
    )
    .await;
    let refused = answered(&mut socket, 2).await;
    assert_eq!(refused["ok"], false, "{refused}");
    let error = refused["error"].as_str().unwrap();
    assert!(error.contains("nope is not an effort"), "{error}");
    assert!(
        listed.iter().all(|id| id != "nope"),
        "the fixture must pick a model that does not list nope"
    );
}

/// The phone's own create builds exactly the draft it cannot express: a
/// workspace reach, this desk's default workspace, no computer and no
/// background work — proved by reading the fields back off the written
/// persona, not by trusting the params the phone sent. Unknown fields the
/// phone JSON might carry, `reach`, `cwd` and `computer` among them, are
/// ignored on this command the same way they are on every other mobile one:
/// none of the `Command` variants sets `deny_unknown_fields`.
#[tokio::test]
async fn mobile_persona_create_builds_a_confined_draft_the_phone_could_not_express() {
    let (root, log, port) = door("mobile-create-confined");
    let mut socket = desk(port).await;
    let request_id = "b1f0c8b2-1c2b-4e2a-9a3d-6e4b5b6a7c8d";
    ask(
        &mut socket,
        json!({
            "id": 1,
            "cmd": "mobile.persona_create",
            "params": {
                "requestId": request_id,
                "name": "Ada",
                "goal": "Keep the harbour running",
                "reach": "machine",
                "cwd": "/etc",
                "computer": { "enabled": true },
            },
        }),
    )
    .await;
    let answer = answered(&mut socket, 1).await;
    assert_eq!(answer["ok"], true, "{answer}");
    let created = answer["result"].clone();
    assert_eq!(created["id"], request_id);
    assert_eq!(created["name"], "Ada");
    assert_eq!(created["goal"], "Keep the harbour running");
    assert_eq!(created["backendId"], "hotline");
    assert!(created.get("reach").is_none(), "{created}");
    assert!(created.get("computer").is_none(), "{created}");
    assert_eq!(created["backgroundWork"], false);
    assert_eq!(
        created["cwd"],
        json!(paths::default_workspace(&root, request_id).to_string_lossy())
    );
    assert_eq!(room::roster(&log).len(), 1);
}

/// A retried `requestId` after a lost acknowledgement returns the teammate
/// already made rather than a second one.
#[tokio::test]
async fn a_repeated_request_id_does_not_duplicate_the_teammate() {
    let (_root, log, port) = door("mobile-create-idempotent");
    let mut socket = desk(port).await;
    let request_id = "c2a1d9c3-2d3c-4f3b-8b4e-7f5c6c7b8d9e";
    let make = json!({
        "id": 1,
        "cmd": "mobile.persona_create",
        "params": { "requestId": request_id, "name": "Ada" },
    });
    ask(&mut socket, make.clone()).await;
    let first = answered(&mut socket, 1).await["result"].clone();

    let mut retry = make.clone();
    retry["id"] = json!(2);
    ask(&mut socket, retry).await;
    let second = answered(&mut socket, 2).await["result"].clone();

    assert_eq!(first, second);
    assert_eq!(room::roster(&log).len(), 1);
}

/// Why a harness is unavailable can name the machine's accounts and folders:
/// the owner reads it, a companion reads only that it is unavailable.
#[tokio::test]
async fn a_companion_reads_that_a_harness_is_unavailable_not_why() {
    let handle: Arc<dyn RoomHandle> = Arc::new(Quiet::new());
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let request = json!({"id": 1, "cmd": "backends.list", "params": {}});
    let reason = |answer: &Value| answer["result"][1]["unavailable"].clone();
    let owner = remote_control_answer(Seat::Owner, &handle, &log, request.clone()).await;
    assert_eq!(reason(&owner), "Not signed in.");
    let phone = remote_control_answer(Seat::Phone, &handle, &log, request).await;
    assert_eq!(phone["ok"], true, "{phone}");
    assert_eq!(reason(&phone), super::PRIVATE_REASON);
    assert!(phone["result"][0].get("unavailable").is_none());
}

/// A bad uuid, a blank name, an unknown backend and one `backends.list`
/// reports unavailable are each refused with a sentence a phone can show.
#[tokio::test]
async fn mobile_persona_create_refuses_a_bad_uuid_a_blank_name_and_a_backend_that_is_not_ready() {
    let (_root, _log, port) = door("mobile-create-refusals");
    let mut socket = desk(port).await;
    for (n, params, needle) in [
        (
            1,
            json!({ "requestId": "not-a-uuid", "name": "Ada" }),
            "requestId must be a uuid",
        ),
        (
            2,
            json!({ "requestId": "d3b2e0d4-3e4d-405c-9c5f-8a6d7d8c9e0f", "name": "   " }),
            "needs a name",
        ),
        (
            3,
            json!({
                "requestId": "e4c3f1e5-4f5e-416d-ad6a-9b7e8e9dafa0",
                "name": "Ada",
                "backendId": "nope",
            }),
            "no harness nope",
        ),
        (
            4,
            json!({
                "requestId": "f5d4a2f6-5a6f-427e-be7b-ac8f9fabab11",
                "name": "Ada",
                "backendId": "cursor",
            }),
            "not ready",
        ),
    ] {
        ask(
            &mut socket,
            json!({ "id": n, "cmd": "mobile.persona_create", "params": params }),
        )
        .await;
        let refused = answered(&mut socket, n).await;
        assert_eq!(refused["ok"], false, "{refused}");
        let error = refused["error"].as_str().unwrap();
        assert!(error.contains(needle), "{error}");
    }
}

#[tokio::test]
async fn session_set_config_on_an_acp_teammate_goes_to_the_room() {
    let (_root, _log, port) = door("set-config-acp");
    let mut socket = desk(port).await;
    let draft = PersonaDraft {
        name: "Bob".to_string(),
        goal: None,
        team: None,
        backend_id: Some("cursor".to_string()),
        cwd: None,
        reach: None,
        model_id: None,
        effort_id: None,
        computer: None,
        background_work: None,
    };
    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "persona.create", "params": { "draft": draft } }),
    )
    .await;
    let created = answered(&mut socket, 1).await["result"].clone();
    let id = created["id"].as_str().unwrap();

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "session.set_config",
            "params": { "personaId": id, "configId": "effort", "value": "high" },
        }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["result"]["configs"][0]["id"], "effort");
    assert_eq!(answer["result"]["configs"][0]["currentId"], "high");
}

#[tokio::test]
async fn persona_create_fills_model_id_from_the_room_default() {
    let (_root, _log, port) = door("create-default-model");
    let mut socket = desk(port).await;
    ask(
        &mut socket,
        json!({
            "id": 1,
            "cmd": "settings.update",
            "params": { "patch": { "defaultModelId": "anthropic/claude" } },
        }),
    )
    .await;
    assert_eq!(answered(&mut socket, 1).await["ok"], true);

    let created = create(&mut socket, 2, "Ada").await;
    assert_eq!(created["modelId"], "anthropic/claude");
}

#[tokio::test]
async fn persona_create_fills_model_id_from_the_last_used_when_no_default() {
    let (_root, _log, port) = door("create-last-model");
    let mut socket = desk(port).await;
    ask(
        &mut socket,
        json!({
            "id": 1,
            "cmd": "settings.update",
            "params": { "patch": { "lastModelId": "openai/gpt" } },
        }),
    )
    .await;
    assert_eq!(answered(&mut socket, 1).await["ok"], true);

    let created = create(&mut socket, 2, "Ada").await;
    assert_eq!(created["modelId"], "openai/gpt");
}

#[tokio::test]
async fn persona_create_leaves_model_id_absent_when_the_room_has_no_preference() {
    let (_root, _log, port) = door("create-no-model");
    let mut socket = desk(port).await;
    let created = create(&mut socket, 1, "Ada").await;
    assert!(created.get("modelId").is_none(), "{created}");
}

/// The fake room's start reports a model, which is enough for the wire to
/// remember it as the last one used.
#[tokio::test]
async fn session_start_writes_last_model_id_from_the_sessions_info() {
    let (_root, log, port) = door("start-last-model");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap();

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "session.start", "params": { "personaId": id } }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(room::settings(&log)["lastModelId"], "anthropic/claude");
}

/// A harness picks its own model and only says so once running, so what a
/// start reports is written to the teammate: the band can name it before
/// the child is started again. The room's last model is Hotline Agent's alone.
#[tokio::test]
async fn session_start_writes_the_reported_model_on_an_acp_teammate() {
    let (_root, log, port) = door("start-acp-model");
    let mut socket = desk(port).await;
    let draft = PersonaDraft {
        name: "Bob".to_string(),
        goal: None,
        team: None,
        backend_id: Some("cursor".to_string()),
        cwd: None,
        reach: None,
        model_id: None,
        effort_id: None,
        computer: None,
        background_work: None,
    };
    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "persona.create", "params": { "draft": draft } }),
    )
    .await;
    let id = answered(&mut socket, 1).await["result"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "session.start", "params": { "personaId": id } }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    let bob = room::roster(&log)
        .into_iter()
        .find(|persona| persona.id == id)
        .unwrap();
    assert_eq!(bob.model_id.as_deref(), Some("anthropic/claude"));
    assert!(
        room::settings(&log).get("lastModelId").is_none(),
        "a harness's model is the teammate's, not the room's last"
    );
}

/// A patch that changes what a teammate can use restarts it; a patch of
/// only the name does not, because the driver is not built from the name.
#[tokio::test]
async fn persona_update_of_reach_reattaches_and_a_name_patch_does_not() {
    let quiet = Arc::new(Quiet::new());
    let (_root, _log, port) = door_with("reattach-persona", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap().to_string();

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "persona.update",
            "params": { "id": id, "patch": { "reach": "machine" } },
        }),
    )
    .await;
    assert_eq!(answered(&mut socket, 2).await["ok"], true);
    assert_eq!(quiet.reattached(), vec![id.clone()]);
    assert_eq!(*quiet.invalidations.lock().unwrap(), vec![id.clone()]);

    ask(
        &mut socket,
        json!({
            "id": 3,
            "cmd": "persona.update",
            "params": { "id": id, "patch": { "name": "Ada Lovelace" } },
        }),
    )
    .await;
    assert_eq!(answered(&mut socket, 3).await["ok"], true);
    assert_eq!(
        quiet.reattached(),
        vec![id.clone()],
        "a name patch reattached a second time"
    );
    assert_eq!(*quiet.invalidations.lock().unwrap(), vec![id.clone()]);

    ask(
        &mut socket,
        json!({
            "id": 4,
            "cmd": "persona.update",
            "params": { "id": id, "patch": { "reach": "invalid" } },
        }),
    )
    .await;
    assert_eq!(answered(&mut socket, 4).await["ok"], false);
    assert_eq!(
        *quiet.invalidations.lock().unwrap(),
        vec![id],
        "an invalid patch must not revoke a working session"
    );
}

/// Turning the computer on or off restarts the teammate, because it gains
/// or loses the tools. Its limits, mounts and secrets do not: they wait for
/// the container to be made again, and a turn in flight is not cut short.
#[tokio::test]
async fn computer_details_do_not_restart_the_teammate_but_turning_it_on_does() {
    let quiet = Arc::new(Quiet::new());
    let (_root, _log, port) = door_with("computer-details", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let ada_id = ada["id"].as_str().unwrap().to_string();

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "persona.update", "params": { "id": ada_id, "patch": {
            "computer": { "enabled": true },
        } } }),
    )
    .await;
    assert_eq!(answered(&mut socket, 2).await["ok"], true);
    assert_eq!(quiet.reattached(), vec![ada_id.clone()]);

    ask(
        &mut socket,
        json!({ "id": 3, "cmd": "persona.update", "params": { "id": ada_id, "patch": {
            "computer": { "enabled": true, "memory": "8g", "secrets": ["GITHUB_TOKEN"] },
        } } }),
    )
    .await;
    let answer = answered(&mut socket, 3).await;
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["result"]["computer"]["memory"], "8g");
    assert_eq!(
        quiet.reattached(),
        vec![ada_id.clone()],
        "no second restart"
    );
    assert_eq!(*quiet.invalidations.lock().unwrap(), vec![ada_id.clone()]);

    ask(
        &mut socket,
        json!({ "id": 4, "cmd": "persona.update", "params": { "id": ada_id, "patch": {
            "computer": { "enabled": false, "memory": "8g" },
        } } }),
    )
    .await;
    assert_eq!(answered(&mut socket, 4).await["ok"], true);
    assert_eq!(quiet.reattached(), vec![ada_id.clone(), ada_id]);
}

/// Removing a tool source is the decision, so it leaves every teammate's
/// grant as well: no policy is left naming a server that is gone, including
/// one an older build left behind. A setting that does not name servers
/// leaves grants alone.
#[tokio::test]
async fn removing_a_tool_source_takes_it_out_of_every_grant() {
    let quiet = Arc::new(Quiet::new());
    let (_root, log, port) = door_with("forget-servers", quiet.clone());
    log.append(
        &StreamId::Room,
        &json!({ "kind": "setting", "id": "mcpServers", "value": [
            { "id": "prism", "type": "http", "name": "Prism", "url": "http://127.0.0.1:9086/mcp" },
            { "id": "ketch", "type": "stdio", "name": "ketch", "command": "ketch", "args": [] },
        ] }),
    )
    .unwrap();
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let ada_id = ada["id"].as_str().unwrap().to_string();
    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "persona.update", "params": { "id": ada_id, "patch": {
            "mcpPolicy": { "mode": "some", "serverIds": ["prism", "ketch", "ghost"] },
        } } }),
    )
    .await;
    assert_eq!(answered(&mut socket, 2).await["ok"], true);
    let granted = |log: &Log| {
        room::roster(log)
            .into_iter()
            .find(|persona| persona.id == ada_id)
            .unwrap()
            .mcp_policy
            .server_ids
    };

    ask(
        &mut socket,
        json!({ "id": 3, "cmd": "settings.update", "params": { "patch": { "chapterIdleHours": 2 } } }),
    )
    .await;
    assert_eq!(answered(&mut socket, 3).await["ok"], true);
    assert_eq!(granted(&log), ["prism", "ketch", "ghost"]);

    ask(
        &mut socket,
        json!({ "id": 4, "cmd": "settings.update", "params": { "patch": { "mcpServers": [] } } }),
    )
    .await;
    assert_eq!(answered(&mut socket, 4).await["ok"], true);
    assert!(granted(&log).is_empty());
}

/// A server that no longer exists is settled, not an error, even when a
/// policy still names its id: its missing rows leave the ledger the last
/// start published. A server that exists and failed, and the computer, stay.
#[tokio::test]
async fn a_server_that_no_longer_exists_leaves_no_missing_rows() {
    use crate::contract::{AgentKind, ToolSourceKind, ToolState};
    use crate::session::ledger::ToolLedger;
    let root = scratch("stale-tool-rows");
    let log = Log::open(&root);
    // The ledger store is shared by every test; this teammate is only here.
    let id = format!("stale-{}", uuid::Uuid::new_v4());
    let persona = |server_ids: &[&str]| {
        json!({
            "kind": "persona", "id": id, "name": "Ada", "goal": "Tools",
            "backendId": "hotline", "cwd": root.to_string_lossy(),
            "mcpPolicy": { "mode": "some", "serverIds": server_ids },
            "backgroundWork": false, "sessionCheckpoints": [], "lastSessionId": null,
            "createdAt": 1, "updatedAt": 1,
        })
    };
    let servers = |ids: &[&str]| {
        let list: Vec<Value> = ids
            .iter()
            .map(|id| json!({ "id": id, "type": "stdio", "name": id, "command": id, "args": [] }))
            .collect();
        json!({ "kind": "setting", "id": "mcpServers", "value": list })
    };
    log.append(&StreamId::Room, &servers(&["prism", "ketch"]))
        .unwrap();
    log.append(&StreamId::Room, &persona(&["prism", "ketch", "ghost"]))
        .unwrap();
    let room = Room::with_agents(log.clone(), Arc::new(NoKeys), Arc::new(NoAgents));
    let mut ledger = ToolLedger::new(id.clone(), AgentKind::Hotline, "hotline");
    ledger
        .verified(ToolSourceKind::Builtin, "hotline", "send_file", "built in")
        .absent(ToolSourceKind::Mcp, "prism", "prism", "failed to start")
        .declared(ToolSourceKind::Mcp, "ketch", "search", "handed over")
        .absent(ToolSourceKind::Mcp, "ghost", "ghost", "no longer exists")
        .absent(
            ToolSourceKind::Mcp,
            crate::computer::SERVER_ID,
            "computer",
            "stopped",
        );
    ledger.publish();
    let rows = |room: &Room| {
        let mut rows: Vec<String> = room
            .teammate_tools(&id)
            .unwrap()
            .rows
            .into_iter()
            .map(|row| row.origin)
            .collect();
        rows.sort();
        rows
    };
    // Ghost is gone though the policy names it; prism exists and failed.
    assert_eq!(rows(&room), ["computer", "hotline", "ketch", "prism"]);

    // Prism is deleted too: its failure is no longer news.
    log.append(&StreamId::Room, &servers(&["ketch"])).unwrap();
    assert_eq!(rows(&room), ["computer", "hotline", "ketch"]);
    assert!(
        room.teammate_tools(&id)
            .unwrap()
            .rows
            .iter()
            .any(|row| row.origin == "computer" && row.state == ToolState::Absent)
    );
    crate::session::ledger::forget(&id);
}

/// The room's server list is what every policy of "all" includes, so every
/// live session restarts. A setting that does not name servers leaves them.
#[tokio::test]
async fn settings_update_of_mcp_servers_reattaches_every_live_session() {
    let quiet = Arc::new(Quiet::new());
    let (_root, _log, port) = door_with("reattach-servers", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let bob = create(&mut socket, 2, "Bob").await;
    let ada_id = ada["id"].as_str().unwrap().to_string();
    let bob_id = bob["id"].as_str().unwrap().to_string();

    ask(
        &mut socket,
        json!({ "id": 3, "cmd": "session.start", "params": { "personaId": ada_id } }),
    )
    .await;
    assert_eq!(answered(&mut socket, 3).await["ok"], true);
    ask(
        &mut socket,
        json!({ "id": 4, "cmd": "session.start", "params": { "personaId": bob_id } }),
    )
    .await;
    assert_eq!(answered(&mut socket, 4).await["ok"], true);

    ask(
        &mut socket,
        json!({
            "id": 5,
            "cmd": "settings.update",
            "params": { "patch": { "mcpServers": [] } },
        }),
    )
    .await;
    assert_eq!(answered(&mut socket, 5).await["ok"], true);
    let mut got = quiet.reattached();
    got.sort();
    let mut want = vec![ada_id, bob_id];
    want.sort();
    assert_eq!(got, want);
    assert_eq!(*quiet.invalidations.lock().unwrap(), vec!["*"]);

    ask(
        &mut socket,
        json!({
            "id": 6,
            "cmd": "settings.update",
            "params": { "patch": { "chapterIdleHours": 2 } },
        }),
    )
    .await;
    assert_eq!(answered(&mut socket, 6).await["ok"], true);
    let mut after = quiet.reattached();
    after.sort();
    assert_eq!(after, want, "a chapterIdleHours patch reattached again");
    assert_eq!(*quiet.invalidations.lock().unwrap(), vec!["*"]);
}

#[tokio::test]
async fn computer_commands_and_the_runtime_setting() {
    let quiet = Arc::new(Quiet::new());
    let (_root, _log, port) = door_with("computer-wire", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap().to_string();

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "computer.runtimes", "params": {} }),
    )
    .await;
    let runtimes = answered(&mut socket, 2).await;
    assert_eq!(runtimes["ok"], true, "{runtimes}");
    assert_eq!(runtimes["result"][0]["runtime"], "podman");
    assert_eq!(runtimes["result"][0]["state"], "ready");
    assert_eq!(runtimes["result"][0]["rootless"], true);
    assert_eq!(runtimes["result"][1]["runtime"], "docker");

    ask(
        &mut socket,
        json!({ "id": 3, "cmd": "computer.status", "params": { "personaId": id } }),
    )
    .await;
    let status = answered(&mut socket, 3).await;
    assert_eq!(status["ok"], true, "{status}");
    assert_eq!(status["result"]["state"], "running");
    assert_eq!(status["result"]["url"], "http://127.0.0.1:18787/mcp");
    assert_eq!(status["result"]["viewer"], "http://127.0.0.1:15800");

    ask(
        &mut socket,
        json!({ "id": 4, "cmd": "computer.stop", "params": { "personaId": id } }),
    )
    .await;
    let stopped = answered(&mut socket, 4).await;
    assert_eq!(stopped["ok"], true, "{stopped}");
    assert!(stopped.get("result").is_none(), "{stopped}");

    ask(
        &mut socket,
        json!({ "id": 5, "cmd": "computer.remove", "params": { "personaId": id } }),
    )
    .await;
    let removed = answered(&mut socket, 5).await;
    assert_eq!(removed["ok"], true, "{removed}");
    assert!(removed.get("result").is_none(), "{removed}");

    ask(
        &mut socket,
        json!({
            "id": 6,
            "cmd": "settings.update",
            "params": { "patch": { "computerRuntime": "podman" } },
        }),
    )
    .await;
    let settings = answered(&mut socket, 6).await;
    assert_eq!(settings["ok"], true, "{settings}");
    assert_eq!(settings["result"]["computerRuntime"], "podman");
    assert!(
        quiet.reattached().is_empty(),
        "picking a runtime does not reattach: {:?}",
        quiet.reattached()
    );

    ask(
        &mut socket,
        json!({
            "id": 7,
            "cmd": "computer.status",
            "params": { "personaId": "nobody" },
        }),
    )
    .await;
    let missing = answered(&mut socket, 7).await;
    assert_eq!(missing["ok"], false, "{missing}");
    assert!(
        missing["error"]
            .as_str()
            .unwrap()
            .contains("There is no teammate nobody"),
        "{missing}"
    );
}

#[tokio::test]
async fn auth_wire_allows_only_desktop_and_disconnect_revokes_its_owner() {
    let quiet = Arc::new(Quiet::new());
    let (_root, log, port) = door_with("auth-wire", quiet.clone());
    let room: Arc<dyn RoomHandle> = quiet.clone();
    let (tx, mut frames) = mpsc::unbounded_channel();
    let outbox = Outbox {
        auth_attempts: Arc::default(),
        pairing: Arc::default(),
        uploads: Arc::default(),
        threads2: Arc::default(),
        lean: Arc::default(),
        sender: Outgoing::Desk(tx),
        cancel: tokio_util::sync::CancellationToken::new(),
        max: usize::MAX,
    };
    let commands = [
        (
            "agent.auth.start",
            json!({"personaId":"ada","methodId":"fixture", "command":"ignored-ui-command", "args":["ignored"]}),
        ),
        (
            "agent.auth.poll",
            json!({"personaId":"ada","id":"fixture-attempt"}),
        ),
        (
            "agent.auth.input",
            json!({"personaId":"ada","id":"fixture-attempt","input":"fixture input"}),
        ),
        (
            "agent.auth.cancel",
            json!({"personaId":"ada","id":"fixture-attempt"}),
        ),
    ];
    for (cmd, params) in &commands {
        let frame = json!({"id":1,"cmd":cmd,"params":params}).to_string();
        answer(
            &frame,
            Seat::Phone,
            &log,
            &room,
            &outbox,
            &mut HashMap::new(),
            None,
        )
        .await;
        let response: Value = serde_json::from_str(&frames.recv().await.unwrap()).unwrap();
        assert_eq!(response["ok"], false, "{response}");
        assert_eq!(response["code"], FORBIDDEN, "{response}");
    }
    assert!(
        quiet.auth_owner.lock().unwrap().is_none(),
        "phone never reached auth"
    );
    let mut socket = desk(port).await;
    for (index, (cmd, params)) in commands.iter().enumerate() {
        let id = index as i64 + 1;
        ask(&mut socket, json!({"id":id,"cmd":cmd,"params":params})).await;
        let response = heard_where(&mut socket, |frame| frame["id"] == id).await;
        assert_eq!(response["ok"], true, "{response}");
        if index == 0 {
            assert_eq!(response["result"]["id"], "fixture-attempt");
        }
        if index == 1 {
            assert_eq!(response["result"]["output"], "ephemeral fixture output");
        }
    }
    // A second authenticated desk knows the UUID but cannot use it as authority.
    let mut other = desk(port).await;
    for (index, (cmd, params)) in commands.iter().enumerate().skip(1) {
        let id = index as i64 + 10;
        ask(&mut other, json!({"id":id,"cmd":cmd,"params":params})).await;
        let response = answered(&mut other, id).await;
        assert_eq!(response["ok"], false, "{response}");
        assert!(
            response["error"]
                .as_str()
                .unwrap()
                .contains("another desktop")
        );
    }
    ask(&mut socket, json!({"id":20,"cmd":"agent.auth.poll","params":{"personaId":"ada","id":"fixture-attempt"}})).await;
    let response = answered(&mut socket, 20).await;
    assert_eq!(response["result"]["output"], "ephemeral fixture output");
    let owner = quiet.auth_owner.lock().unwrap().clone().unwrap();
    assert!(!owner.is_cancelled());
    socket.close(None).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), owner.cancelled())
        .await
        .unwrap();
    assert!(log.load(&StreamId::Tape("ada".into())).is_empty());
}

/// Exercise the same frame reader and seat gate as both socket transports.
async fn remote_control_answer(
    seat: Seat,
    room: &Arc<dyn RoomHandle>,
    log: &Log,
    frame: Value,
) -> Value {
    let (tx, mut frames) = mpsc::unbounded_channel();
    let outbox = Outbox {
        auth_attempts: Arc::default(),
        pairing: Arc::default(),
        uploads: Arc::default(),
        threads2: Arc::default(),
        lean: Arc::default(),
        sender: Outgoing::Desk(tx),
        cancel: tokio_util::sync::CancellationToken::new(),
        max: usize::MAX,
    };
    let mut subscriptions = HashMap::new();
    answer(
        &frame.to_string(),
        seat,
        log,
        room,
        &outbox,
        &mut subscriptions,
        None,
    )
    .await;
    let response = serde_json::from_str(&frames.recv().await.unwrap()).unwrap();
    for handle in subscriptions.into_values() {
        handle.abort();
    }
    response
}

#[tokio::test]
async fn remote_controls_are_denied_to_companions() {
    use crate::credentials::tests::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::default());
    let desk = Arc::new(crate::desk::Desk::open_with_store(root.path(), store.clone()).unwrap());
    let room: Arc<dyn RoomHandle> = desk.clone();
    let remote =
        crate::remote::Remote::open_with_store(root.path(), desk.log.clone(), room.clone(), store)
            .unwrap();
    desk.set_remote(&remote);

    {
        let seat = Seat::Phone;
        for (cmd, params) in [
            ("remote.status", json!({})),
            ("remote.configure", json!({"enabled":true,"host":"all"})),
            ("remote.devices", json!({})),
            ("remote.revoke", json!({"deviceId":"any"})),
            ("remote.relay", json!({"deskId":"any"})),
            ("remote.pairing", json!({})),
            ("remote.pairing", json!({"role":"companion"})),
            ("remote.pairing", json!({"id":"any"})),
            ("remote.pairing", json!({"id":"any","cancel":true})),
            (
                "persona.update",
                json!({"id":"any","patch":{"reach":"machine"}}),
            ),
            ("settings.update", json!({"patch":{"remote":true}})),
        ] {
            let response = remote_control_answer(
                seat,
                &room,
                &desk.log,
                json!({"id":1,"cmd":cmd,"params":params}),
            )
            .await;
            assert_eq!(response["code"], FORBIDDEN, "{seat:?} {cmd}: {response}");
        }
        let response =
            remote_control_answer(seat, &room, &desk.log, json!({"id":1,"sub":"room"})).await;
        assert_eq!(response["code"], FORBIDDEN, "{seat:?}: {response}");
        // Both roles retain the same useful, read-only phone command.
        let response =
            remote_control_answer(seat, &room, &desk.log, json!({"id":1,"cmd":"models.list"}))
                .await;
        assert_eq!(response["ok"], true, "{seat:?}: {response}");
    }
    assert!(
        !remote.status().enabled,
        "denied configure must not reach Remote"
    );
    assert!(remote.devices().is_empty());

    for (cmd, params) in [
        ("remote.status", json!({})),
        ("remote.configure", json!({"enabled":false,"host":"all"})),
        ("remote.devices", json!({})),
        ("remote.revoke", json!({"deviceId":"already-absent"})),
    ] {
        let response = remote_control_answer(
            Seat::Desk,
            &room,
            &desk.log,
            json!({"id":1,"cmd":cmd,"params":params}),
        )
        .await;
        assert_eq!(response["ok"], true, "{cmd}: {response}");
    }
    let weak = Arc::downgrade(&remote);
    drop(remote);
    assert!(
        weak.upgrade().is_none(),
        "desk controls must not form a strong cycle"
    );
    assert!(desk.remote().is_none());
    let response = remote_control_answer(
        Seat::Desk,
        &room,
        &desk.log,
        json!({"id":1,"cmd":"remote.status"}),
    )
    .await;
    assert_eq!(response["ok"], false);
    assert!(
        response["error"]
            .as_str()
            .unwrap()
            .contains("not available")
    );
}

#[tokio::test]
async fn desk_pairing_commands_start_poll_and_cancel_sealed_pairing() {
    use crate::credentials::tests::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::default());
    let desk = Arc::new(crate::desk::Desk::open_with_store(root.path(), store.clone()).unwrap());
    let room: Arc<dyn RoomHandle> = desk.clone();
    let remote =
        crate::remote::Remote::open_with_store(root.path(), desk.log.clone(), room.clone(), store)
            .unwrap();
    desk.set_remote(&remote);
    let response = remote_control_answer(
        Seat::Desk,
        &room,
        &desk.log,
        json!({"id":1,"cmd":"remote.configure","params":{"enabled":true,"host":"all"}}),
    )
    .await;
    assert_eq!(response["ok"], true, "{response}");
    for params in [json!({}), json!({"role":"companion"})] {
        let response = remote_control_answer(
            Seat::Desk,
            &room,
            &desk.log,
            json!({"id":1,"cmd":"remote.pairing","params":params}),
        )
        .await;
        assert_eq!(response["ok"], true, "{response}");
        assert!(
            response["result"]["url"]
                .as_str()
                .unwrap()
                .starts_with("hotline://pair?v=2")
        );
        assert!(response["result"].get("manual").is_none());
        let id = response["result"]["id"].as_str().unwrap();
        let pending = remote_control_answer(
            Seat::Desk,
            &room,
            &desk.log,
            json!({"id":1,"cmd":"remote.pairing","params":{"id":id}}),
        )
        .await;
        assert_eq!(pending, json!({"id":1,"ok":true,"result":null}));
        let cancelled = remote_control_answer(
            Seat::Desk,
            &room,
            &desk.log,
            json!({"id":1,"cmd":"remote.pairing","params":{"id":id,"cancel":true}}),
        )
        .await;
        assert_eq!(cancelled, json!({"id":1,"ok":true}));
    }
    for params in [json!({"cancel":true}), json!({"id":"any","role":"owner"})] {
        let refused = remote_control_answer(
            Seat::Desk,
            &room,
            &desk.log,
            json!({"id":1,"cmd":"remote.pairing","params":params}),
        )
        .await;
        assert_eq!(refused["ok"], false, "{refused}");
    }
    remote.configure(false, "all").await.unwrap();
}

#[tokio::test]
async fn a_desktop_disconnect_does_not_drop_an_inflight_mutation() {
    let pending = Arc::new(PendingStop::default());
    let mut room = Quiet::new();
    room.pending_stop = Some(pending.clone());
    let (_root, _log, port) = door_with("desk-pending-stop", Arc::new(room));
    let mut socket = desk(port).await;
    let persona = create(&mut socket, 0, "Ada").await;
    ask(
        &mut socket,
        json!({"id":1,"cmd":"computer.stop","params":{"personaId":persona["id"]}}),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(3), pending.entered.cancelled())
        .await
        .unwrap();
    socket.close(None).await.unwrap();
    drop(socket);
    // Unlike a revoked phone, the local desk's mutation must finish.
    assert!(
        tokio::time::timeout(Duration::from_millis(50), pending.dropped.cancelled())
            .await
            .is_err()
    );
    pending.release.cancel();
    tokio::time::timeout(Duration::from_secs(3), pending.dropped.cancelled())
        .await
        .unwrap();
    assert!(pending.completed.is_cancelled());
}

#[test]
fn mobile_persona_computer_contract_distinguishes_absent_null_and_number() {
    for (params, expected) in [
        (json!({"id":"ada"}), None),
        (json!({"id":"ada","cpus":null}), Some(None)),
        (json!({"id":"ada","cpus":1.5}), Some(Some(1.5))),
    ] {
        let command: Command = serde_json::from_value(json!({
            "cmd":"mobile.persona_computer", "params":params,
        }))
        .unwrap();
        let Command::MobilePersonaComputer { cpus, .. } = &command else {
            panic!("wrong command")
        };
        assert_eq!(*cpus, expected);
        assert_eq!(serde_json::to_value(&command).unwrap()["params"], params);
        assert!(Seat::Owner.permits(&command));
        assert!(Seat::Desk.permits(&command));
        assert!(!Seat::Phone.permits(&command));
    }
    assert!(Seat::Owner.capabilities().contains(&"personaComputer"));
    assert!(!Seat::Phone.capabilities().contains(&"personaComputer"));
    for seat in [Seat::Desk, Seat::Owner, Seat::Phone] {
        assert!(seat.permits(&Command::ComputerCapacity {}));
    }
    use ts_rs::TS;
    let command_ts = Command::decl(&ts_rs::Config::default());
    let computer_ts = command_ts
        .split("mobile.persona_computer")
        .nth(1)
        .unwrap()
        .split(" | { \"cmd\"")
        .next()
        .unwrap();
    assert!(
        computer_ts.contains("cpus?: number | null,"),
        "{computer_ts}"
    );
    let capacity_ts = crate::contract::ComputerCapacity::decl(&ts_rs::Config::default());
    assert!(capacity_ts.contains("memoryBytes: number"), "{capacity_ts}");
    assert!(
        capacity_ts.contains("runtime: ComputerRuntime | null"),
        "{capacity_ts}"
    );
    let capacity = crate::contract::ComputerCapacity {
        runtime: None,
        cpus: 2,
        memory_bytes: 4 * 1024 * 1024 * 1024,
        source: crate::contract::ComputerCapacitySource::Default,
    };
    assert_eq!(
        json!(capacity),
        json!({"runtime":null,"cpus":2,"memoryBytes":4294967296_u64,"source":"default"})
    );
}

#[tokio::test]
async fn mobile_persona_computer_owner_controls_preserve_other_settings() {
    let quiet = Arc::new(Quiet::new());
    let (_root, log, port) = door_with("mobile-computer", quiet.clone());
    let handle: Arc<dyn RoomHandle> = quiet.clone();
    let mut socket = desk(port).await;
    let id = create(&mut socket, 1, "Ada").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let request = |params| json!({"id":2,"cmd":"mobile.persona_computer","params":params});
    let denied = remote_control_answer(
        Seat::Phone,
        &handle,
        &log,
        request(json!({"id":id,"enabled":true})),
    )
    .await;
    assert_eq!(denied["code"], FORBIDDEN, "{denied}");
    assert!(room::roster(&log)[0].computer.is_none());
    let enabled = remote_control_answer(
        Seat::Owner,
        &handle,
        &log,
        request(json!({"id":id,"enabled":true})),
    )
    .await;
    assert_eq!(enabled["ok"], true, "{enabled}");
    assert_eq!(enabled["result"]["id"], id);
    assert_eq!(enabled["result"]["computer"], json!({"enabled":true}));
    assert_eq!(quiet.reattached(), vec![id.clone()]);
    let full = json!({"enabled":true,"image":"pinned:test","memory":"4g","cpus":2.0,"pids":2048,
        "mounts":[{"host":"/projects","path":"/work","readonly":true}],"secrets":["TOKEN"]});
    let seed = remote_control_answer(
        Seat::Desk,
        &handle,
        &log,
        json!({"id":3,"cmd":"persona.update","params":{"id":id,"patch":{"computer":full}}}),
    )
    .await;
    assert_eq!(seed["ok"], true, "{seed}");
    let updated = remote_control_answer(Seat::Owner, &handle, &log, request(json!({
        "id":id,"memory":"4608m","cpus":1.5,"image":"untrusted","pids":1,"secrets":[],"mounts":[],"reach":"machine"
    }))).await;
    assert_eq!(updated["ok"], true, "{updated}");
    let mut wanted = full.clone();
    wanted["memory"] = json!("4608m");
    wanted["cpus"] = json!(1.5);
    assert_eq!(updated["result"]["computer"], wanted);
    assert!(updated["result"].get("reach").is_none());
    let absent = remote_control_answer(
        Seat::Owner,
        &handle,
        &log,
        request(json!({"id":id,"memory":"5g"})),
    )
    .await;
    assert_eq!(absent["result"]["computer"]["cpus"], 1.5);
    let cleared = remote_control_answer(
        Seat::Owner,
        &handle,
        &log,
        request(json!({"id":id,"cpus":null})),
    )
    .await;
    assert_eq!(cleared["ok"], true, "{cleared}");
    assert!(cleared["result"]["computer"].get("cpus").is_none());
    assert_eq!(cleared["result"]["computer"]["image"], "pinned:test");
    assert_eq!(
        quiet.reattached(),
        vec![id.clone()],
        "resource edits must not restart"
    );
    assert_eq!(quiet.computer_changes.lock().unwrap().len(), 4);
    let disabled = remote_control_answer(
        Seat::Owner,
        &handle,
        &log,
        request(json!({"id":id,"enabled":false})),
    )
    .await;
    assert_eq!(disabled["result"]["computer"]["enabled"], false);
    assert_eq!(quiet.reattached(), vec![id.clone(), id]);
}

#[tokio::test]
async fn mobile_persona_computer_validates_limits_atomically_against_fake_capacity() {
    let quiet = Arc::new(Quiet::new());
    let (_root, log, port) = door_with("mobile-computer-limits", quiet.clone());
    let handle: Arc<dyn RoomHandle> = quiet.clone();
    let mut socket = desk(port).await;
    let id = create(&mut socket, 1, "Ada").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    for seat in [Seat::Owner, Seat::Phone, Seat::Desk] {
        let answer = remote_control_answer(
            seat,
            &handle,
            &log,
            json!({"id":2,"cmd":"computer.capacity","params":{}}),
        )
        .await;
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(
            answer["result"],
            json!({"runtime":"podman","cpus":8,"memoryBytes":17179869184_u64,"source":"runtime"})
        );
    }
    let empty = remote_control_answer(
        Seat::Owner,
        &handle,
        &log,
        json!({"id":3,"cmd":"mobile.persona_computer","params":{"id":id}}),
    )
    .await;
    assert_eq!(empty["ok"], false);
    assert!(
        empty["error"]
            .as_str()
            .unwrap()
            .contains("Nothing to change")
    );
    for patch in [
        json!({"cpus":0}),
        json!({"cpus":-0.5}),
        json!({"cpus":0.25}),
        json!({"cpus":8.5}),
        json!({"memory":"256m"}),
        json!({"memory":"513m"}),
        json!({"memory":"17g"}),
        json!({"memory":"1.5g"}),
        json!({"memory":"512.0m"}),
        json!({"memory":"+512m"}),
        json!({"memory":"512"}),
        json!({"memory":"512mb"}),
        json!({"memory":" 512m"}),
        json!({"memory":"18446744073709551615g"}),
        json!({"memory":""}),
        json!({"memory":"512m","cpus":9}),
        json!({"memory":"17g","cpus":1}),
    ] {
        let mut params = patch.clone();
        params["id"] = json!(id);
        params["enabled"] = json!(true);
        let refused = remote_control_answer(
            Seat::Owner,
            &handle,
            &log,
            json!({"id":4,"cmd":"mobile.persona_computer","params":params}),
        )
        .await;
        assert_eq!(refused["ok"], false, "{patch}: {refused}");
        assert!(
            room::roster(&log)[0].computer.is_none(),
            "invalid edits must not enable or partly save"
        );
    }
    for (cpus, memory) in [(0.5, "512m"), (8.0, "16g"), (1.0, "1024M"), (2.5, "2G")] {
        let answer = remote_control_answer(Seat::Owner, &handle, &log, json!({"id":5,"cmd":"mobile.persona_computer","params":{"id":id,"cpus":cpus,"memory":memory}})).await;
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(answer["result"]["computer"]["cpus"], cpus);
        assert_eq!(answer["result"]["computer"]["memory"], memory);
        assert_eq!(
            answer["result"]["computer"]["enabled"], false,
            "limits alone are not an enable grant"
        );
    }
    // JSON cannot encode these, but the typed boundary must also reject them.
    for cpus in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let command = Command::MobilePersonaComputer {
            id: id.clone(),
            enabled: None,
            memory: None,
            cpus: Some(Some(cpus)),
        };
        assert!(
            commands::run(command, &log, &handle)
                .await
                .unwrap_err()
                .contains("CPUs")
        );
    }
    assert!(quiet.reattached().is_empty());
}

#[tokio::test]
async fn owner_commands_and_room_subscription_reach_the_real_handler() {
    use crate::credentials::tests::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    let room: Arc<dyn RoomHandle> = desk.clone();
    for seat in [Seat::Owner, Seat::Desk] {
        let answer = remote_control_answer(seat, &room, &desk.log,
            json!({"id":1,"cmd":"settings.update","params":{"patch":{"skillsHome":"/tmp/owner-skills"}}})).await;
        assert_eq!(answer["ok"], true, "{answer}");
        let answer =
            remote_control_answer(seat, &room, &desk.log, json!({"id":2,"sub":"room"})).await;
        assert_eq!(answer["ok"], true, "{answer}");
    }
    let answer = remote_control_answer(Seat::Phone, &room, &desk.log,
        json!({"id":3,"cmd":"settings.update","params":{"patch":{"skillsHome":"/tmp/companion-skills"}}})).await;
    assert_eq!(answer["code"], FORBIDDEN);
    assert_eq!(
        crate::room::settings(&desk.log)["skillsHome"],
        "/tmp/owner-skills"
    );
}

#[tokio::test]
async fn voice_is_owner_only_through_the_real_handler() {
    use crate::credentials::tests::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_voice_services(
            root.path(),
            Arc::new(MemoryStore::default()),
            Some(crate::voice::tests::services()),
        )
        .unwrap(),
    );
    let room: Arc<dyn RoomHandle> = desk.clone();
    let call = uuid::Uuid::new_v4().to_string();
    for (command, params) in [
        ("voice.status", json!({})),
        ("voice.status", json!({"inputMode":"text"})),
        ("voice.call_start", json!({"callId":call})),
        (
            "voice.call_start",
            json!({"callId":call,"personaId":"ada","streamAudio":true}),
        ),
        (
            "voice.call_start",
            json!({"callId":call,"inputMode":"text"}),
        ),
        (
            "voice.text",
            json!({"callId":call,"seq":1,"text":"Check the PR."}),
        ),
        (
            "voice.audio",
            json!({"callId":call,"seq":1,"index":0,"data":"AAA=","final":true}),
        ),
        (
            "voice.utterance",
            json!({"callId":call,"seq":1,"mimeType":"audio/wav","data":"","durationMs":100}),
        ),
        ("voice.interrupt", json!({"callId":call})),
        ("voice.hold", json!({"callId":call,"hold":true})),
        ("voice.call_end", json!({"callId":call})),
    ] {
        let denied = remote_control_answer(
            Seat::Phone,
            &room,
            &desk.log,
            json!({"id":1,"cmd":command,"params":params}),
        )
        .await;
        assert_eq!(denied["code"], FORBIDDEN, "{denied}");
    }
    let denied = remote_control_answer(
        Seat::Phone,
        &room,
        &desk.log,
        json!({"id":2,"sub":{"call":call}}),
    )
    .await;
    assert_eq!(denied["code"], FORBIDDEN);
    let hidden = remote_control_answer(
        Seat::Phone,
        &room,
        &desk.log,
        json!({"id":20,"sub":{"tape":crate::voice::TAPE_ID}}),
    )
    .await;
    assert_eq!(hidden["code"], FORBIDDEN);
    for seat in [Seat::Desk, Seat::Owner] {
        let allowed = remote_control_answer(
            seat,
            &room,
            &desk.log,
            json!({"id":3,"cmd":"voice.call_start","params":{"callId":call}}),
        )
        .await;
        assert_eq!(allowed["ok"], true, "{allowed}");
    }
    for seat in [Seat::Desk, Seat::Owner] {
        let text_call = uuid::Uuid::new_v4().to_string();
        let started = remote_control_answer(seat, &room, &desk.log,
            json!({"id":4,"cmd":"voice.call_start","params":{"callId":text_call,"inputMode":"text"}})).await;
        assert_eq!(started["ok"], true, "{started}");
        assert_eq!(started["result"]["inputMode"], "text");
        assert_eq!(started["result"]["input"], json!(["text/plain"]));
        let committed = remote_control_answer(seat, &room, &desk.log,
            json!({"id":5,"cmd":"voice.text","params":{"callId":text_call,"seq":1,"text":"Check the PR."}})).await;
        assert_eq!(committed["ok"], true, "{committed}");
        let replay = remote_control_answer(seat, &room, &desk.log,
            json!({"id":6,"cmd":"voice.text","params":{"callId":text_call,"seq":1,"text":"Check the PR."}})).await;
        assert_eq!(replay["ok"], false, "{replay}");
    }
    assert!(
        Seat::Owner
            .capabilities_for(room.as_ref())
            .contains(&"voiceDirectCalls")
    );
    assert!(
        !Seat::Phone
            .capabilities_for(room.as_ref())
            .contains(&"voiceDirectCalls")
    );
    assert!(
        Seat::Owner
            .capabilities_for(room.as_ref())
            .contains(&"voiceTextInput")
    );
    assert!(
        !Seat::Phone
            .capabilities_for(room.as_ref())
            .contains(&"voiceTextInput")
    );
}

#[tokio::test]
async fn the_desks_speech_models_are_the_owners_through_the_real_handler() {
    use crate::credentials::tests::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    let room: Arc<dyn RoomHandle> = desk.clone();
    let model = "parakeet-tdt-110m-en";
    // A companion can neither see, fetch, remove nor hear with them. None of
    // these is answered, so nothing here reaches the network.
    for (command, params) in [
        ("voice.models", json!({})),
        ("voice.model_install", json!({"modelId":model})),
        ("voice.model_cancel", json!({"modelId":model})),
        ("voice.model_remove", json!({"modelId":model})),
        (
            "voice.transcribe",
            json!({"mimeType":"audio/wav","data":"UklGRg=="}),
        ),
    ] {
        let denied = remote_control_answer(
            Seat::Phone,
            &room,
            &desk.log,
            json!({"id":1,"cmd":command,"params":params}),
        )
        .await;
        assert_eq!(denied["code"], FORBIDDEN, "{command}: {denied}");
    }
    for seat in [Seat::Desk, Seat::Owner] {
        let listed = remote_control_answer(
            seat,
            &room,
            &desk.log,
            json!({"id":2,"cmd":"voice.models","params":{}}),
        )
        .await;
        assert_eq!(listed["ok"], true, "{listed}");
        // Nothing is installed until the owner asks.
        let states: Vec<&str> = listed["result"]
            .as_array()
            .unwrap()
            .iter()
            .map(|model| model["state"].as_str().unwrap())
            .collect();
        assert_eq!(states, ["available", "available"]);
        let unknown = remote_control_answer(
            seat,
            &room,
            &desk.log,
            json!({"id":3,"cmd":"voice.model_install","params":{"modelId":"../../vault"}}),
        )
        .await;
        assert_eq!(unknown["ok"], false, "{unknown}");
        let unheard = remote_control_answer(
            seat,
            &room,
            &desk.log,
            json!({"id":4,"cmd":"voice.transcribe","params":{"mimeType":"audio/wav","data":"UklGRg=="}}),
        )
        .await;
        assert_eq!(unheard["ok"], false, "{unheard}");
        assert!(
            unheard["error"]
                .as_str()
                .is_some_and(|error| error.contains("Download a speech model")),
            "{unheard}"
        );
    }
}

#[tokio::test]
async fn direct_voice_readiness_does_not_require_a_dispatcher_but_requires_speech_and_budget() {
    use crate::credentials::tests::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    let room: Arc<dyn RoomHandle> = desk.clone();
    desk.credential_create("openai", "Speech fixture", "unused-test-key")
        .unwrap();
    desk.log
        .append(
            &StreamId::Room,
            &crate::room::room_event(
                "setting",
                json!({
                    "id":"voice", "value":{"dispatcher":{"provider":"not-connected"}}
                }),
            ),
        )
        .unwrap();
    for params in [json!({}), json!({"inputMode":"text"})] {
        let answer = remote_control_answer(
            Seat::Owner,
            &room,
            &desk.log,
            json!({"id":1,"cmd":"voice.status","params":params}),
        )
        .await;
        assert_eq!(answer["ok"], true);
        assert_eq!(answer["result"]["available"], false);
        assert_eq!(answer["result"]["directAvailable"], true);
        if params.get("inputMode").is_some() {
            assert!(answer["result"].get("stt").is_none());
        } else {
            assert_eq!(answer["result"]["stt"]["providerId"], "openai");
        }
        assert_eq!(answer["result"]["tts"]["providerId"], "openai");
        assert!(answer["result"].get("dispatcher").is_none());
    }
    for seat in [Seat::Owner, Seat::Desk] {
        let capabilities = seat.capabilities_for(room.as_ref());
        assert!(capabilities.contains(&"voice"));
        assert!(capabilities.contains(&"voiceDirectCalls"));
        assert!(capabilities.contains(&"voiceTextInput"));
    }
    assert!(
        !Seat::Phone
            .capabilities_for(room.as_ref())
            .contains(&"voiceDirectCalls")
    );
    desk.log
        .append(
            &StreamId::Room,
            &crate::room::room_event(
                "setting",
                json!({
                    "id":"spending", "value":{"dayUsd":0,"monthUsd":0}
                }),
            ),
        )
        .unwrap();
    assert!(!desk.voice().unwrap().status().direct_available);
    assert!(
        !desk
            .voice()
            .unwrap()
            .status_for(crate::contract::VoiceInputMode::Text)
            .direct_available
    );
    assert!(
        !Seat::Owner
            .capabilities_for(room.as_ref())
            .contains(&"voice")
    );
    desk.log
        .append(
            &StreamId::Room,
            &crate::room::room_event(
                "setting",
                json!({
                    "id":"spending", "value":{"dayUsd":2,"monthUsd":20}
                }),
            ),
        )
        .unwrap();
    desk.log
        .append(
            &StreamId::Room,
            &crate::room::room_event(
                "setting",
                json!({
                    "id":"voice", "value":{
                        "stt":{"provider":"not-connected"},
                        "dispatcher":{"provider":"not-connected"}
                    }
                }),
            ),
        )
        .unwrap();
    assert!(!desk.voice().unwrap().status().direct_available);
    let text = remote_control_answer(
        Seat::Owner,
        &room,
        &desk.log,
        json!({"id":2,"cmd":"voice.status","params":{"inputMode":"text"}}),
    )
    .await;
    assert_eq!(text["ok"], true);
    assert_eq!(text["result"]["available"], false);
    assert_eq!(text["result"]["directAvailable"], true);
    assert!(text["result"].get("stt").is_none());
    for seat in [Seat::Owner, Seat::Desk] {
        let capabilities = seat.capabilities_for(room.as_ref());
        assert!(capabilities.contains(&"voice"));
        assert!(capabilities.contains(&"voiceDirectCalls"));
        assert!(capabilities.contains(&"voiceTextInput"));
    }
    desk.log
        .append(
            &StreamId::Room,
            &crate::room::room_event(
                "setting",
                json!({"id":"voice", "value":{"tts":{"provider":"not-connected"}}}),
            ),
        )
        .unwrap();
    for mode in [
        crate::contract::VoiceInputMode::Audio,
        crate::contract::VoiceInputMode::Text,
    ] {
        assert!(!desk.voice().unwrap().status_for(mode).direct_available);
    }
    assert!(
        !Seat::Owner
            .capabilities_for(room.as_ref())
            .contains(&"voiceDirectCalls")
    );
}

#[tokio::test]
async fn cookie_push_is_operator_only_through_the_real_handler() {
    let quiet: Arc<dyn RoomHandle> = Arc::new(Quiet::new());
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let request = json!({"id":1,"cmd":"computer.cookies.push","params":{"personaId":"ada","transfer":{
        "sourceId":"laptop", "browserId":"firefox", "profileId":"default", "domains":["example.com"],
        "cookies":[{"domain":".example.com","name":"session","value":"private-cookie-value","path":"/"}]
    }}});
    for seat in [Seat::Desk, Seat::Owner, Seat::Phone] {
        let answer = remote_control_answer(seat, &quiet, &log, request.clone()).await;
        assert_eq!(answer["ok"], seat != Seat::Phone, "{answer}");
        if seat == Seat::Phone {
            assert_eq!(answer["code"], FORBIDDEN);
        }
        assert!(!answer.to_string().contains("private-cookie-value"));
    }
    let mut invalid = request;
    invalid["params"]["transfer"]["domains"] = json!(["other.com"]);
    assert_eq!(
        remote_control_answer(Seat::Owner, &quiet, &log, invalid).await["ok"],
        false
    );
    assert!(log.load(&StreamId::Room).is_empty());
}

#[tokio::test]
async fn images_status_is_operator_only_and_mock_rooms_are_unavailable() {
    let handle: Arc<dyn RoomHandle> = Arc::new(Quiet::new());
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let request = json!({"id": 1, "cmd": "images.status", "params": {}});
    for seat in [Seat::Desk, Seat::Owner, Seat::Phone] {
        let answer = remote_control_answer(seat, &handle, &log, request.clone()).await;
        if seat == Seat::Phone {
            assert_eq!(answer["ok"], false, "{answer}");
            assert_eq!(answer["code"], FORBIDDEN);
            assert!(answer.get("result").is_none());
        } else {
            assert_eq!(answer["ok"], true, "{answer}");
            assert_eq!(
                answer["result"],
                json!({
                    "available": false, "unavailable": "This room cannot make images."
                })
            );
        }
    }
    assert!(log.load(&StreamId::Room).is_empty());
}

#[tokio::test]
async fn capabilities_options_are_for_the_desk_and_owner_and_never_a_companion() {
    use crate::credentials::tests::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    let handle: Arc<dyn RoomHandle> = desk.clone();
    let request = json!({"id": 1, "cmd": "capabilities.options", "params": {}});
    for seat in [Seat::Desk, Seat::Owner, Seat::Phone] {
        let answer = remote_control_answer(seat, &handle, &desk.log, request.clone()).await;
        if seat == Seat::Phone {
            assert_eq!(answer["code"], FORBIDDEN, "{answer}");
            assert!(answer.get("result").is_none());
        } else {
            assert_eq!(answer["ok"], true, "{answer}");
        }
    }
    let quiet: Arc<dyn RoomHandle> = Arc::new(Quiet::new());
    let answer = remote_control_answer(Seat::Owner, &quiet, &desk.log, request).await;
    assert_eq!(answer["ok"], false, "{answer}");
}

#[tokio::test]
async fn capabilities_options_list_only_connected_providers_and_what_automatic_picks() {
    use crate::credentials::tests::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    let handle: Arc<dyn RoomHandle> = desk.clone();
    let request = json!({"id": 1, "cmd": "capabilities.options", "params": {}});
    let empty = remote_control_answer(Seat::Owner, &handle, &desk.log, request.clone()).await;
    assert_eq!(empty["ok"], true, "{empty}");
    let result = &empty["result"];
    for job in ["images", "stt", "tts"] {
        assert_eq!(result[job]["options"], json!([]), "{job}");
        assert!(
            result[job]["unavailable"]
                .as_str()
                .unwrap()
                .contains("Connect")
        );
        assert!(result[job].get("automatic").is_none());
        assert!(result[job].get("selected").is_none());
    }
    assert_eq!(
        result["spending"],
        json!({"dayUsd": 2.0, "monthUsd": 20.0, "spentDayUsd": 0.0, "spentMonthUsd": 0.0})
    );

    let secret = "capabilities-private-credential";
    desk.credential_create("anthropic", "Chat only", secret)
        .unwrap();
    desk.credential_create("groq", "Speech", secret).unwrap();
    desk.credential_create("openai", "Everything", secret)
        .unwrap();
    let answer = remote_control_answer(Seat::Owner, &handle, &desk.log, request.clone()).await;
    assert!(!answer.to_string().contains(secret));
    let result = &answer["result"];
    let names = |job: &str| -> Vec<String> {
        result[job]["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|provider| provider["providerName"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(names("images"), ["OpenAI"]);
    assert_eq!(names("stt"), ["Groq", "OpenAI"]);
    assert_eq!(names("tts"), ["Groq", "OpenAI"]);
    assert!(names("dispatcher").contains(&"OpenAI".to_string()));
    assert!(
        result["images"]["options"][0]["models"]
            .as_array()
            .unwrap()
            .iter()
            .all(|model| model["id"].as_str().unwrap().contains("image"))
    );
    assert_eq!(
        result["images"]["automatic"],
        json!({"providerId": "openai", "providerName": "OpenAI", "modelId": "gpt-image-2.5-flare"})
    );
    assert_eq!(
        result["stt"]["automatic"]["modelId"],
        "whisper-large-v3-turbo"
    );
    assert_eq!(result["tts"]["automatic"]["providerId"], "groq");
    assert_eq!(result["tts"]["automatic"]["voice"], "hannah");
    assert_eq!(
        result["tts"]["options"][1]["models"][0]["voices"][0],
        "marin"
    );

    // A choice is reported as made, and automatic still says what it would be.
    let selected = remote_control_answer(
        Seat::Owner,
        &handle,
        &desk.log,
        json!({"id": 2, "cmd": "settings.update", "params": {"patch": {
            "images": {"provider": "openai", "model": "gpt-image-1-mini"},
            "voice": {"tts": {"provider": "openai", "voice": "cedar"}},
            "spending": {"dayUsd": 0.5, "monthUsd": 5.0}
        }}}),
    )
    .await;
    assert_eq!(selected["ok"], true, "{selected}");
    let answer = remote_control_answer(Seat::Desk, &handle, &desk.log, request).await;
    let result = &answer["result"];
    assert_eq!(result["images"]["selected"]["modelId"], "gpt-image-1-mini");
    assert_eq!(result["tts"]["selected"]["providerName"], "OpenAI");
    assert_eq!(result["tts"]["selected"]["voice"], "cedar");
    assert_eq!(result["tts"]["automatic"]["providerId"], "groq");
    assert!(result["stt"].get("selected").is_none());
    assert_eq!(result["spending"]["dayUsd"], 0.5);
    assert_eq!(result["spending"]["monthUsd"], 5.0);
}

#[tokio::test]
async fn capabilities_options_add_the_image_and_voice_tallies_into_one_spent_figure() {
    use crate::credentials::tests::MemoryStore;
    use crate::spending::{SpendLedger, SpendingSettings};
    let root = tempfile::tempdir().unwrap();
    SpendLedger::new(root.path().to_path_buf())
        .reserve(&SpendingSettings::default(), 0.25)
        .unwrap()
        .charge(0.125)
        .unwrap();
    std::fs::write(
        root.path().join("voice-ledger.json"),
        json!({
            "day": chrono::Local::now().format("%Y-%m-%d").to_string(),
            "month": chrono::Local::now().format("%Y-%m").to_string(),
            "daySpend": {"stt": 0.0, "tts": 0.25, "dispatcher": 0.0},
            "monthSpend": {"stt": 0.0, "tts": 0.5, "dispatcher": 0.0}
        })
        .to_string(),
    )
    .unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    let handle: Arc<dyn RoomHandle> = desk.clone();
    let answer = remote_control_answer(
        Seat::Owner,
        &handle,
        &desk.log,
        json!({"id": 1, "cmd": "capabilities.options", "params": {}}),
    )
    .await;
    assert_eq!(
        answer["result"]["spending"]["spentDayUsd"], 0.375,
        "{answer}"
    );
    assert_eq!(answer["result"]["spending"]["spentMonthUsd"], 0.625);
}

#[tokio::test]
async fn images_status_resolves_the_desks_vault_without_exposing_credentials() {
    use crate::credentials::tests::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    let handle: Arc<dyn RoomHandle> = desk.clone();
    let request = json!({"id": 1, "cmd": "images.status", "params": {}});
    let missing = remote_control_answer(Seat::Desk, &handle, &desk.log, request.clone()).await;
    assert_eq!(missing["ok"], true, "{missing}");
    assert_eq!(missing["result"]["available"], false);
    assert!(
        missing["result"]["unavailable"]
            .as_str()
            .unwrap()
            .contains("Connect")
    );
    assert!(missing["result"].get("provider").is_none());
    assert!(missing["result"].get("model").is_none());

    let secret = "image-status-private-credential";
    desk.credential_create("anthropic", "No image provider", secret)
        .unwrap();
    let unsupported = remote_control_answer(Seat::Owner, &handle, &desk.log, request.clone()).await;
    assert_eq!(unsupported["result"]["available"], false, "{unsupported}");
    let openai = desk
        .credential_create("openai", "Image provider", secret)
        .unwrap();
    desk.credential_create("google", "Second image provider", secret)
        .unwrap();
    let before = desk.log.load(&StreamId::Room);
    for seat in [Seat::Desk, Seat::Owner] {
        let available = remote_control_answer(seat, &handle, &desk.log, request.clone()).await;
        assert_eq!(
            available["result"],
            json!({
                "available": true, "provider": "openai", "model": "gpt-image-2.5-flare",
                "spending": {"dayUsd": 0.0, "monthUsd": 0.0}
            })
        );
        assert!(!available.to_string().contains(secret));
    }
    assert_eq!(desk.log.load(&StreamId::Room), before);

    let selected = remote_control_answer(
        Seat::Owner,
        &handle,
        &desk.log,
        json!({
            "id": 2, "cmd": "settings.update", "params": {"patch": {
                "images": {"provider": "google", "model": "gemini-3.1-flash-lite-image"}
            }}
        }),
    )
    .await;
    assert_eq!(selected["ok"], true, "{selected}");
    let available = remote_control_answer(Seat::Owner, &handle, &desk.log, request.clone()).await;
    assert_eq!(
        available["result"],
        json!({
            "available": true, "provider": "google", "model": "gemini-3.1-flash-lite-image",
            "spending": {"dayUsd": 0.0, "monthUsd": 0.0}
        })
    );

    let selected = remote_control_answer(Seat::Desk, &handle, &desk.log, json!({
        "id": 3, "cmd": "settings.update", "params": {"patch": {"images": {"provider": "openai"}}}
    })).await;
    assert_eq!(selected["ok"], true, "{selected}");
    desk.credential_revoke(&openai.id).unwrap();
    let before = desk.log.load(&StreamId::Room);
    let revoked = remote_control_answer(Seat::Owner, &handle, &desk.log, request.clone()).await;
    assert_eq!(revoked["result"]["available"], false, "{revoked}");
    assert!(
        revoked["result"]["unavailable"]
            .as_str()
            .unwrap()
            .contains("isn't connected")
    );
    assert!(revoked["result"].get("provider").is_none());
    assert!(revoked["result"].get("model").is_none());
    assert_eq!(desk.log.load(&StreamId::Room), before);

    let reset = remote_control_answer(
        Seat::Desk,
        &handle,
        &desk.log,
        json!({
            "id": 4, "cmd": "settings.update", "params": {"patch": {"images": null}}
        }),
    )
    .await;
    assert_eq!(reset["ok"], true, "{reset}");
    let available = remote_control_answer(Seat::Owner, &handle, &desk.log, request).await;
    assert_eq!(available["result"]["available"], true, "{available}");
    assert_eq!(available["result"]["provider"], "google");
    let denied = remote_control_answer(
        Seat::Phone,
        &handle,
        &desk.log,
        json!({"id": 5, "cmd": "images.status", "params": {}}),
    )
    .await;
    assert_eq!(denied["code"], FORBIDDEN);
    assert!(denied.get("result").is_none());
    for answer in [missing, unsupported, selected, available, revoked, reset] {
        assert!(!answer.to_string().contains(secret), "{answer}");
    }
    assert!(
        !serde_json::to_string(&desk.log.load(&StreamId::Room))
            .unwrap()
            .contains(secret)
    );
}

#[tokio::test]
async fn chatgpt_images_status_is_automatic_without_checking_entitlement() {
    use crate::credentials::tests::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::default());
    let log = crate::log::Log::open(root.path());
    let vault = crate::vault::Vault::open_with_store(root.path(), log, store.clone()).unwrap();
    let (id, token_dir) = vault.begin_login("openai-codex").unwrap();
    vault.finish_login(&id, "openai-codex", "ChatGPT").unwrap();
    drop(vault);
    let desk = Arc::new(crate::desk::Desk::open_with_store(root.path(), store).unwrap());
    let handle: Arc<dyn RoomHandle> = desk.clone();
    let request = json!({"id": 1, "cmd": "images.status", "params": {}});
    let automatic = remote_control_answer(Seat::Desk, &handle, &desk.log, request.clone()).await;
    assert_eq!(automatic["result"]["available"], true);
    assert_eq!(automatic["result"]["provider"], "openai-codex");
    let selected = remote_control_answer(
        Seat::Owner,
        &handle,
        &desk.log,
        json!({
            "id": 2, "cmd": "settings.update", "params": {"patch": {
                "images": {"provider": "openai-codex"}
            }}
        }),
    )
    .await;
    assert_eq!(selected["ok"], true);
    let denied = remote_control_answer(Seat::Phone, &handle, &desk.log, request.clone()).await;
    assert_eq!(denied["code"], FORBIDDEN);
    for seat in [Seat::Owner, Seat::Desk] {
        let status = remote_control_answer(seat, &handle, &desk.log, request.clone()).await;
        assert_eq!(status["result"]["available"], true);
        assert_eq!(status["result"]["provider"], "openai-codex");
        assert_eq!(status["result"]["model"], "gpt-image-2");
        assert!(
            !status
                .to_string()
                .contains(&token_dir.to_string_lossy().to_string())
        );
    }
    assert_eq!(
        std::fs::read_to_string(token_dir.join("auth.json")).unwrap(),
        "{}"
    );
}

#[tokio::test]
async fn image_and_spending_settings_are_typed_validated_and_resettable() {
    let handle: Arc<dyn RoomHandle> = Arc::new(Quiet::new());
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let defaults = remote_control_answer(
        Seat::Desk,
        &handle,
        &log,
        json!({
            "id": 1, "cmd": "settings.update", "params": {"patch": {}}
        }),
    )
    .await;
    assert_eq!(defaults["result"]["images"], json!({}));
    assert_eq!(
        defaults["result"]["spending"],
        json!({"dayUsd": 2.0, "monthUsd": 20.0})
    );

    let updated = remote_control_answer(Seat::Owner, &handle, &log, json!({
        "id": 2, "cmd": "settings.update", "params": {"patch": {
            "images": {"provider": "openai", "model": "gpt-image-1-mini", "secret": "not-a-setting"},
            "spending": {"dayUsd": 0, "monthUsd": 0},
            "futureSetting": {"enabled": true}
        }}
    })).await;
    assert_eq!(updated["ok"], true, "{updated}");
    assert_eq!(
        updated["result"]["images"],
        json!({"provider": "openai", "model": "gpt-image-1-mini"})
    );
    assert_eq!(
        updated["result"]["spending"],
        json!({"dayUsd": 0.0, "monthUsd": 0.0})
    );
    assert_eq!(updated["result"]["futureSetting"], json!({"enabled": true}));
    assert!(
        !serde_json::to_string(&log.load(&StreamId::Room))
            .unwrap()
            .contains("not-a-setting")
    );

    let partial = remote_control_answer(
        Seat::Desk,
        &handle,
        &log,
        json!({
            "id": 3, "cmd": "settings.update", "params": {"patch": {
                "images": {"model": "gpt-image-1-mini"}, "spending": {"monthUsd": 7.5}
            }}
        }),
    )
    .await;
    assert_eq!(partial["ok"], true, "{partial}");
    assert_eq!(
        partial["result"]["images"],
        json!({"model": "gpt-image-1-mini"})
    );
    assert_eq!(
        partial["result"]["spending"],
        json!({"dayUsd": 2.0, "monthUsd": 7.5})
    );

    let reset = remote_control_answer(Seat::Owner, &handle, &log, json!({
        "id": 4, "cmd": "settings.update", "params": {"patch": {"images": null, "spending": null}}
    })).await;
    assert_eq!(reset["ok"], true, "{reset}");
    assert_eq!(reset["result"]["images"], json!({}));
    assert_eq!(
        reset["result"]["spending"],
        json!({"dayUsd": 2.0, "monthUsd": 20.0})
    );
    assert_eq!(reset["result"]["futureSetting"], json!({"enabled": true}));
    let before = log.load(&StreamId::Room);
    let denied = remote_control_answer(
        Seat::Phone,
        &handle,
        &log,
        json!({
            "id": 5, "cmd": "settings.update", "params": {"patch": {"spending": {"dayUsd": 0}}}
        }),
    )
    .await;
    assert_eq!(denied["code"], FORBIDDEN);
    assert_eq!(log.load(&StreamId::Room), before);
}

#[tokio::test]
async fn images_status_reports_recorded_spending_without_creating_or_writing_a_ledger() {
    use crate::credentials::tests::MemoryStore;
    use crate::spending::{SpendLedger, SpendingSettings};
    let root = tempfile::tempdir().unwrap();
    {
        let ledger = SpendLedger::new(root.path().to_path_buf());
        ledger
            .reserve(&SpendingSettings::default(), 0.25)
            .unwrap()
            .charge(0.125)
            .unwrap();
    }
    let ledger_path = root.path().join("spending.json");
    let before = std::fs::read(&ledger_path).unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    let handle: Arc<dyn RoomHandle> = desk.clone();
    let answer = remote_control_answer(
        Seat::Owner,
        &handle,
        &desk.log,
        json!({"id": 1, "cmd": "images.status", "params": {}}),
    )
    .await;
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(
        answer["result"]["spending"],
        json!({"dayUsd": 0.125, "monthUsd": 0.125})
    );
    assert!(answer["result"].get("spendingUnavailable").is_none());
    assert_eq!(std::fs::read(&ledger_path).unwrap(), before);

    let fresh_root = tempfile::tempdir().unwrap();
    let fresh_desk = Arc::new(
        crate::desk::Desk::open_with_store(fresh_root.path(), Arc::new(MemoryStore::default()))
            .unwrap(),
    );
    let fresh_handle: Arc<dyn RoomHandle> = fresh_desk.clone();
    let answer = remote_control_answer(
        Seat::Desk,
        &fresh_handle,
        &fresh_desk.log,
        json!({"id": 2, "cmd": "images.status", "params": {}}),
    )
    .await;
    assert_eq!(
        answer["result"]["spending"],
        json!({"dayUsd": 0.0, "monthUsd": 0.0})
    );
    assert!(!fresh_root.path().join("spending.json").exists());
}

#[tokio::test]
async fn images_status_preserves_the_shared_ledgers_failure_state_without_leaking_contents() {
    use crate::credentials::tests::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let ledger_path = root.path().join("spending.json");
    std::fs::write(&ledger_path, b"private-corrupt-ledger-content").unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    desk.credential_create("openai", "Image provider", "private-image-key")
        .unwrap();
    let handle: Arc<dyn RoomHandle> = desk.clone();
    for seat in [Seat::Desk, Seat::Owner] {
        let answer = remote_control_answer(
            seat,
            &handle,
            &desk.log,
            json!({"id": 1, "cmd": "images.status", "params": {}}),
        )
        .await;
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(answer["result"]["available"], true);
        assert!(answer["result"].get("spending").is_none());
        assert!(
            answer["result"]["spendingUnavailable"]
                .as_str()
                .unwrap()
                .contains("Spending is blocked")
        );
        assert!(
            !answer
                .to_string()
                .contains("private-corrupt-ledger-content")
        );
        assert!(!answer.to_string().contains("private-image-key"));
        if ledger_path.exists() {
            std::fs::remove_file(&ledger_path).unwrap();
        }
    }
    assert!(!ledger_path.exists());
}

#[tokio::test]
async fn images_status_refuses_malformed_saved_selections_and_marks_invalid_caps_unavailable() {
    use crate::credentials::tests::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    desk.credential_create("openai", "Image provider", "private-image-key")
        .unwrap();
    let handle: Arc<dyn RoomHandle> = desk.clone();
    for images in [json!({"provider": 17}), json!([]), json!({"model": " "})] {
        desk.log
            .append(
                &StreamId::Room,
                &json!({
                    "kind": "setting", "id": "images", "value": images
                }),
            )
            .unwrap();
        let answer = remote_control_answer(
            Seat::Desk,
            &handle,
            &desk.log,
            json!({"id": 1, "cmd": "images.status", "params": {}}),
        )
        .await;
        assert_eq!(answer["result"]["available"], false, "{answer}");
        assert!(answer["result"]["unavailable"].is_string());
        assert!(answer["result"].get("provider").is_none());
        assert_eq!(crate::room::settings(&desk.log)["images"], images);
    }
    desk.log
        .append(
            &StreamId::Room,
            &json!({
                "kind": "setting", "id": "images", "deleted": true
            }),
        )
        .unwrap();
    let spending = json!({"dayUsd": "disabled", "monthUsd": 0});
    desk.log
        .append(
            &StreamId::Room,
            &json!({
                "kind": "setting", "id": "spending", "value": spending
            }),
        )
        .unwrap();
    let answer = remote_control_answer(
        Seat::Owner,
        &handle,
        &desk.log,
        json!({"id": 2, "cmd": "images.status", "params": {}}),
    )
    .await;
    assert_eq!(answer["result"]["available"], true, "{answer}");
    assert!(answer["result"].get("spending").is_none());
    assert!(answer["result"]["spendingUnavailable"].is_string());
    assert_eq!(crate::room::settings(&desk.log)["spending"], spending);
    assert!(serde_json::from_value::<crate::spending::SpendingSettings>(spending).is_err());
}

#[tokio::test]
async fn invalid_image_or_spending_settings_do_not_write_any_patch_key() {
    let quiet = Arc::new(Quiet::new());
    let handle: Arc<dyn RoomHandle> = quiet.clone();
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    for (key, value) in [
        ("images", json!("not-an-object")),
        ("images", json!([])),
        ("images", json!({"provider": 17})),
        ("images", json!({"model": false})),
        ("images", json!({"provider": " "})),
        ("images", json!({"model": ""})),
        ("spending", json!([])),
        ("spending", json!({"dayUsd": -1})),
        ("spending", json!({"monthUsd": -1})),
        ("spending", json!({"dayUsd": "2"})),
        ("spending", json!({"monthUsd": null})),
    ] {
        let mut patch = serde_json::Map::new();
        patch.insert("theme".into(), json!("dark"));
        patch.insert("mcpServers".into(), json!([]));
        patch.insert(key.into(), value);
        let refused = remote_control_answer(
            Seat::Owner,
            &handle,
            &log,
            json!({
                "id": 1, "cmd": "settings.update", "params": {"patch": patch}
            }),
        )
        .await;
        assert_eq!(refused["ok"], false, "{refused}");
        assert!(refused["error"].is_string());
        assert!(log.load(&StreamId::Room).is_empty());
        assert!(quiet.invalidations.lock().unwrap().is_empty());
        assert!(quiet.reattached().is_empty());
    }
}

#[tokio::test]
async fn images_status_refuses_corrupt_saved_zero_caps_without_exposing_room_contents() {
    use crate::credentials::tests::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    desk.credential_create("openai", "Image provider", "private-image-key")
        .unwrap();
    let handle: Arc<dyn RoomHandle> = desk.clone();
    let disabled = remote_control_answer(
        Seat::Owner,
        &handle,
        &desk.log,
        json!({
            "id": 1, "cmd": "settings.update", "params": {"patch": {
                "spending": {"dayUsd": 0, "monthUsd": 0}
            }}
        }),
    )
    .await;
    assert_eq!(disabled["ok"], true, "{disabled}");
    std::fs::write(
        crate::paths::room_path(root.path()),
        "{\"kind\":\"setting\",\"id\":\"spending\",\"value\":{\"dayUsd\":0,\"monthUsd\":0},\"private\":\"corrupt-room-canary\"\n",
    ).unwrap();
    for seat in [Seat::Desk, Seat::Owner, Seat::Phone] {
        let answer = remote_control_answer(
            seat,
            &handle,
            &desk.log,
            json!({"id": 2, "cmd": "images.status", "params": {}}),
        )
        .await;
        if seat == Seat::Phone {
            assert_eq!(answer["code"], FORBIDDEN);
            assert!(answer.get("result").is_none());
        } else {
            assert_eq!(answer["ok"], true, "{answer}");
            assert_eq!(answer["result"]["available"], false);
            assert!(
                answer["result"]["unavailable"]
                    .as_str()
                    .unwrap()
                    .contains("could not be safely read")
            );
            assert_eq!(
                answer["result"]["spendingUnavailable"],
                answer["result"]["unavailable"]
            );
            for field in ["provider", "model", "spending"] {
                assert!(answer["result"].get(field).is_none());
            }
        }
        assert!(!answer.to_string().contains("corrupt-room-canary"));
        assert!(!answer.to_string().contains("private-image-key"));
    }
    assert!(!root.path().join("spending.json").exists());
}

/// A thread's link is what is stored, and a client on any build is sent the
/// marker its kind has always had: in a snapshot, as an event, and in a page.
#[tokio::test]
async fn a_thread_link_reaches_every_client_as_the_marker_its_kind_always_had() {
    let (_root, log, port) = door("link-wire");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let persona_id = ada["id"].as_str().unwrap().to_string();
    let tape = StreamId::Tape(persona_id.clone());
    let link = |state: &str| {
        json!({
            "kind": "link", "id": "link:side:s1", "ts": 5, "thread": "s1",
            "threadKind": "side", "personaId": persona_id, "title": "Mend the crane",
            "state": state
        })
    };
    log.append(&tape, &link("live")).unwrap();
    log.append(&StreamId::Side("s1".into()), &link("live"))
        .unwrap();
    // One written before links, which is sent as it is.
    let old = json!({
        "kind": "subagent", "id": "subagent:r1", "ts": 6, "runId": "r1",
        "title": "Look it up", "status": "done"
    });
    log.append(&tape, &old).unwrap();

    ask(
        &mut socket,
        json!({ "id": 2, "sub": { "tape": persona_id } }),
    )
    .await;
    let snapshot = heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    let lines = snapshot["snapshot"].as_array().unwrap();
    assert_eq!(lines[0]["kind"], "side");
    assert_eq!(lines[0]["id"], "link:side:s1");
    assert_eq!(lines[0]["sideId"], "s1");
    assert_eq!(lines[0]["status"], "live");
    assert_eq!(lines[1], old);

    // Rewritten as the thread goes: the event a client folds by id.
    log.append(&tape, &link("parked")).unwrap();
    let event = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(event["event"]["kind"], "side");
    assert_eq!(event["event"]["status"], "parked");

    ask(&mut socket, json!({ "id": 3, "sub": { "side": "s1" } })).await;
    let own = heard_where(&mut socket, |frame| {
        frame["sub"] == 3 && frame["snapshot"].is_array()
    })
    .await;
    assert_eq!(own["snapshot"][0]["kind"], "side");
    assert_eq!(own["snapshot"][0]["personaId"], persona_id);

    log.append(
        &tape,
        &json!({ "kind": "user", "id": "u1", "ts": 9, "text": "later" }),
    )
    .unwrap();
    ask(
        &mut socket,
        json!({ "id": 4, "cmd": "tape.page", "params": { "personaId": persona_id, "before": "u1" } }),
    )
    .await;
    let page = answered(&mut socket, 4).await;
    assert_eq!(page["result"]["events"][0]["kind"], "side");
    assert_eq!(page["result"]["events"][0]["status"], "parked");
}

/// The same stored lines, for a client that said it reads `threads2`: every
/// link is one `link` shape, an old marker included, and it keeps its id.
#[tokio::test]
async fn a_threads2_client_is_sent_every_link_as_the_link_itself() {
    let (_root, log, port) = door("link-wire-threads2");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let persona_id = ada["id"].as_str().unwrap().to_string();
    let tape = StreamId::Tape(persona_id.clone());
    let link = |state: &str| {
        json!({
            "kind": "link", "id": "link:side:s1", "ts": 5, "thread": "s1",
            "threadKind": "side", "personaId": persona_id, "title": "Mend the crane",
            "state": state
        })
    };
    log.append(&tape, &link("live")).unwrap();
    log.append(
        &tape,
        &json!({
            "kind": "subagent", "id": "subagent:r1", "ts": 6, "runId": "r1",
            "title": "Look it up", "status": "done"
        }),
    )
    .unwrap();

    let hello = hello_threads2(&mut socket, 2).await;
    assert!(
        hello["result"]["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("threads2")),
        "{hello}"
    );
    ask(
        &mut socket,
        json!({ "id": 3, "sub": { "tape": persona_id } }),
    )
    .await;
    let snapshot = heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    let lines = snapshot["snapshot"].as_array().unwrap();
    assert_eq!(lines[0], link("live"), "a link is sent as it is stored");
    assert_eq!(lines[1]["kind"], "link");
    assert_eq!(lines[1]["id"], "subagent:r1");
    assert_eq!(lines[1]["threadKind"], "run");
    assert_eq!(lines[1]["thread"], "r1");
    assert_eq!(lines[1]["end"], "done");
    // What a client is sent parses as the contract's `link`.
    for line in lines {
        let typed: TranscriptEvent = serde_json::from_value(line.clone()).unwrap();
        assert!(matches!(typed, TranscriptEvent::Link { .. }), "{line}");
    }

    log.append(&tape, &link("parked")).unwrap();
    let event = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(event["event"]["kind"], "link");
    assert_eq!(event["event"]["state"], "parked");

    log.append(
        &tape,
        &json!({ "kind": "user", "id": "u1", "ts": 9, "text": "later" }),
    )
    .unwrap();
    ask(
        &mut socket,
        json!({ "id": 4, "cmd": "thread.page",
                "params": { "thread": { "kind": "dm", "key": persona_id }, "before": "u1" } }),
    )
    .await;
    let page = answered(&mut socket, 4).await;
    assert_eq!(page["result"]["events"][0]["kind"], "link");
    assert_eq!(page["result"]["events"][0]["state"], "parked");
    assert_eq!(page["result"]["more"], false);

    // The old name pages the same lines in the shape this socket reads.
    ask(
        &mut socket,
        json!({ "id": 5, "cmd": "tape.page", "params": { "personaId": persona_id, "before": "u1" } }),
    )
    .await;
    assert_eq!(
        answered(&mut socket, 5).await["result"],
        page["result"],
        "tape.page is thread.page on a DM"
    );

    // A second socket that never said so is still sent the marker.
    let mut old = desk(port).await;
    ask(&mut old, json!({ "id": 1, "sub": { "tape": persona_id } })).await;
    let snapshot = heard_where(&mut old, |frame| frame["snapshot"].is_array()).await;
    assert_eq!(snapshot["snapshot"][0]["kind"], "side");
    assert_eq!(snapshot["snapshot"][1]["kind"], "subagent");
}

/// A teammate's threads, of every kind, as one list: what each kind's own
/// record says, in one shape.
#[tokio::test]
async fn thread_list_reads_every_kind_as_one_summary() {
    let (_root, log, port) = door("thread-list");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let bob = create(&mut socket, 2, "Bob").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let tape = StreamId::Tape(ada.clone());
    log.append(
        &tape,
        &json!({ "kind": "user", "id": "u1", "ts": 100, "text": "morning" }),
    )
    .unwrap();
    log.append(
        &tape,
        &json!({ "kind": "agent", "id": "a1", "ts": 110, "text": "Good morning." }),
    )
    .unwrap();

    let link = |kind: &str, key: &str, title: &str, state: &str, extra: Value| {
        let mut line = json!({
            "kind": "link", "id": format!("link:{kind}:{key}"), "ts": 200, "thread": key,
            "threadKind": kind, "personaId": ada, "title": title, "state": state,
        });
        for (name, value) in extra.as_object().unwrap() {
            line[name] = value.clone();
        }
        line
    };
    // A work thread a colleague opened, closed with an outcome.
    let side = link(
        "side",
        "s1",
        "Mend the crane",
        "closed",
        json!({ "end": "agent", "outcome": "It was the cache.", "at": 300,
                "openerId": bob, "openerName": "Bob" }),
    );
    log.append(&tape, &side).unwrap();
    log.append(&StreamId::Side("s1".into()), &side).unwrap();
    log.append(
        &StreamId::Side("s1".into()),
        &json!({ "kind": "agent", "id": "sa", "ts": 250, "text": "Checked the cache." }),
    )
    .unwrap();
    // A parked one, with a card waiting.
    let parked = link("side", "s2", "Sweep the docks", "parked", json!({}));
    log.append(&tape, &parked).unwrap();
    log.append(&StreamId::Side("s2".into()), &parked).unwrap();
    log.append(
        &StreamId::Side("s2".into()),
        &json!({ "kind": "permission", "id": "perm:r", "ts": 400, "requestId": "r",
                 "title": "run it", "options": [] }),
    )
    .unwrap();
    let run = link(
        "run",
        "r1",
        "Look it up",
        "closed",
        json!({ "end": "done" }),
    );
    log.append(&tape, &run).unwrap();
    log.append(&StreamId::Run("r1".into()), &run).unwrap();
    let call = link(
        "call",
        "c1",
        "Call",
        "closed",
        json!({ "end": "idle", "at": 260 }),
    );
    log.append(&tape, &call).unwrap();
    log.append(&StreamId::Call("c1".into()), &call).unwrap();
    let pair = paths::thread_key(&ada, &bob).unwrap();
    crate::log::thread::ensure(log.root(), &pair).unwrap();
    log.append(
        &StreamId::Pair(pair.clone()),
        &json!({ "kind": "user", "id": "pu", "ts": 150, "text": "Got a minute?" }),
    )
    .unwrap();

    ask(
        &mut socket,
        json!({ "id": 3, "cmd": "thread.list", "params": { "personaId": ada } }),
    )
    .await;
    let listed = answered(&mut socket, 3).await;
    assert_eq!(listed["ok"], true, "{listed}");
    let rows = listed["result"].as_array().unwrap();
    let find = |kind: &str, key: &str| {
        rows.iter()
            .find(|row| row["thread"] == json!({ "kind": kind, "key": key }))
            .unwrap_or_else(|| panic!("no {kind} {key} in {rows:?}"))
            .clone()
    };
    assert_eq!(rows.len(), 6, "{rows:?}");
    // Live first, then parked, then closed; newest first inside each.
    let order: Vec<&str> = rows
        .iter()
        .map(|row| row["state"].as_str().unwrap())
        .collect();
    assert_eq!(
        order,
        ["live", "parked", "parked", "closed", "closed", "closed"],
        "{order:?}"
    );

    let dm = find("dm", &ada);
    assert_eq!(dm["personaId"], ada.as_str());
    assert_eq!(dm["preview"], "Good morning.");
    assert_eq!(dm["state"], "live");
    assert_eq!(dm["working"], false);

    let closed = find("side", "s1");
    assert_eq!(closed["title"], "Mend the crane");
    assert_eq!(closed["state"], "closed");
    assert_eq!(closed["end"], "agent");
    assert_eq!(closed["outcome"], "It was the cache.");
    assert_eq!(closed["preview"], "Checked the cache.");
    assert_eq!(closed["opener"], json!({ "personaId": bob, "name": "Bob" }));
    assert_eq!(closed["updatedAt"], 250);
    assert_eq!(closed["waiting"], false);

    let waiting = find("side", "s2");
    assert_eq!(waiting["state"], "parked");
    assert_eq!(waiting["waiting"], true);

    assert_eq!(find("run", "r1")["end"], "done");
    assert_eq!(find("call", "c1")["title"], "Call");
    let pair_row = find("pair", &pair);
    assert_eq!(pair_row["state"], "parked");
    assert_eq!(pair_row["preview"], "Got a minute?");
    assert!(
        [ada.as_str(), bob.as_str()].contains(&pair_row["personaId"].as_str().unwrap())
            && [ada.as_str(), bob.as_str()].contains(&pair_row["withPersonaId"].as_str().unwrap())
            && pair_row["personaId"] != pair_row["withPersonaId"],
        "{pair_row}"
    );
    for row in rows {
        serde_json::from_value::<crate::contract::ThreadSummary>(row.clone()).unwrap();
    }

    // Room-wide: Bob's DM joins, and the pair is still listed once.
    ask(
        &mut socket,
        json!({ "id": 4, "cmd": "thread.list", "params": {} }),
    )
    .await;
    let all = answered(&mut socket, 4).await["result"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(all.len(), 7);
    assert_eq!(
        all.iter()
            .filter(|row| row["thread"]["kind"] == "pair")
            .count(),
        1
    );

    // A teammate nobody holds has no threads.
    ask(
        &mut socket,
        json!({ "id": 5, "cmd": "thread.list", "params": { "personaId": "nobody" } }),
    )
    .await;
    assert_eq!(answered(&mut socket, 5).await["result"], json!([]));
}

/// A companion is listed the threads it may read, which leaves out a call:
/// its summary's preview is what was spoken in it.
#[tokio::test]
async fn a_companions_thread_list_leaves_out_calls_and_pairs() {
    let (_root, log, port) = door("thread-list-companion");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let bob = create(&mut socket, 2, "Bob").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let tape = StreamId::Tape(ada.clone());
    let link = |kind: &str, key: &str| {
        json!({
            "kind": "link", "id": format!("link:{kind}:{key}"), "ts": 200, "thread": key,
            "threadKind": kind, "personaId": ada, "title": key, "state": "closed",
            "end": "idle", "at": 300,
        })
    };
    for (kind, key, stream) in [
        ("side", "s1", StreamId::Side("s1".into())),
        ("call", "c1", StreamId::Call("c1".into())),
    ] {
        log.append(&tape, &link(kind, key)).unwrap();
        log.append(&stream, &link(kind, key)).unwrap();
        log.append(
            &stream,
            &json!({ "kind": "agent", "id": "x", "ts": 250, "text": format!("said in {key}") }),
        )
        .unwrap();
    }
    let pair = paths::thread_key(&ada, &bob).unwrap();
    crate::log::thread::ensure(log.root(), &pair).unwrap();
    log.append(
        &StreamId::Pair(pair.clone()),
        &json!({ "kind": "user", "id": "pu", "ts": 150, "text": "Got a minute?" }),
    )
    .unwrap();

    let handle: Arc<dyn RoomHandle> = Arc::new(Quiet::new());
    let kinds = |answer: &Value| -> Vec<String> {
        let mut kinds: Vec<String> = answer["result"]
            .as_array()
            .unwrap_or_else(|| panic!("{answer}"))
            .iter()
            .map(|row| row["thread"]["kind"].as_str().unwrap().to_string())
            .collect();
        kinds.sort();
        kinds
    };
    for params in [json!({ "personaId": ada }), json!({})] {
        let request = json!({ "id": 1, "cmd": "thread.list", "params": params });
        let phone = remote_control_answer(Seat::Phone, &handle, &log, request.clone()).await;
        assert!(
            !phone.to_string().contains("said in c1"),
            "a call's words are not a companion's to list: {phone}"
        );
        assert!(
            !kinds(&phone)
                .iter()
                .any(|kind| kind == "call" || kind == "pair")
        );
        assert!(kinds(&phone).iter().any(|kind| kind == "side"));
        let owner = remote_control_answer(Seat::Owner, &handle, &log, request).await;
        assert!(kinds(&owner).iter().any(|kind| kind == "call"));
        assert!(kinds(&owner).iter().any(|kind| kind == "pair"));
    }
}

#[tokio::test]
async fn a_thread_subscription_by_id_reads_the_stream_its_kind_keeps() {
    let (_root, log, port) = door("thread-id-sub");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let bob = create(&mut socket, 2, "Bob").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let pair = paths::thread_key(&ada, &bob).unwrap();
    crate::log::thread::ensure(log.root(), &pair).unwrap();
    for (stream, line) in [
        (StreamId::Tape(ada.clone()), "dm"),
        (StreamId::Side("s1".into()), "side"),
        (StreamId::Run("r1".into()), "run"),
        (StreamId::Call("c1".into()), "call"),
        (StreamId::Pair(pair.clone()), "pair"),
    ] {
        log.append(
            &stream,
            &json!({ "kind": "user", "id": line, "ts": 1, "text": line }),
        )
        .unwrap();
    }
    for (n, (kind, key)) in [
        ("dm", ada.as_str()),
        ("side", "s1"),
        ("run", "r1"),
        ("call", "c1"),
        ("pair", pair.as_str()),
    ]
    .into_iter()
    .enumerate()
    {
        let id = 10 + n as i64;
        ask(
            &mut socket,
            json!({ "id": id, "sub": { "threadId": { "kind": kind, "key": key } } }),
        )
        .await;
        assert_eq!(heard(&mut socket).await, json!({ "id": id, "ok": true }));
        let snapshot = heard(&mut socket).await;
        assert_eq!(snapshot["snapshot"][0]["text"], kind, "{snapshot}");
    }
    // `{thread: key}` still means a pair, for the clients that know no other.
    ask(&mut socket, json!({ "id": 20, "sub": { "thread": pair } })).await;
    assert_eq!(heard(&mut socket).await, json!({ "id": 20, "ok": true }));
    assert_eq!(heard(&mut socket).await["snapshot"][0]["text"], "pair");
}

/// Each seat may do what it could do under the older names, and no more.
#[test]
fn the_phone_seat_runs_thread_commands_as_it_ran_the_old_ones() {
    let thread = |kind: ThreadKind| ThreadId::new(kind, "k");
    let prompt = |kind| Command::ThreadPrompt {
        thread: thread(kind),
        text: "x".to_string(),
        reply_to: None,
        attachments: None,
    };
    let answer = |kind| Command::ThreadAnswer {
        thread: thread(kind),
        answer: crate::contract::ThreadAnswer::Permission {
            request_id: "r".to_string(),
            option_id: "o".to_string(),
        },
    };
    // A phone speaks to a teammate through `mobile.prompt`, as before, and in
    // a work thread through this.
    assert!(Seat::Phone.permits(&prompt(ThreadKind::Side)));
    for kind in [
        ThreadKind::Dm,
        ThreadKind::Pair,
        ThreadKind::Run,
        ThreadKind::Call,
    ] {
        assert!(!Seat::Phone.permits(&prompt(kind)), "{kind:?}");
    }
    // `peers.answer_permission` is the owner's, so a pair's card is.
    for kind in [
        ThreadKind::Dm,
        ThreadKind::Side,
        ThreadKind::Run,
        ThreadKind::Call,
    ] {
        assert!(Seat::Phone.permits(&answer(kind)), "{kind:?}");
    }
    assert!(!Seat::Phone.permits(&answer(ThreadKind::Pair)));
    for command in [
        Command::ThreadList { persona_id: None },
        Command::ThreadOpen {
            persona_id: "ada".to_string(),
            text: "x".to_string(),
        },
        Command::ThreadCancel {
            thread: thread(ThreadKind::Side),
        },
        Command::ThreadPark {
            thread: thread(ThreadKind::Side),
        },
        Command::ThreadClose {
            thread: thread(ThreadKind::Side),
        },
        Command::ThreadContinue {
            thread: thread(ThreadKind::Side),
        },
        Command::ClientHello {
            capabilities: Vec::new(),
        },
    ] {
        assert!(Seat::Phone.permits(&command), "{command:?}");
    }
    // The voice dispatcher's tape and a call are not the phone's to read.
    let page = |thread| Command::ThreadPage {
        thread,
        before: "b".to_string(),
        limit: None,
        through: None,
    };
    assert!(Seat::Phone.permits(&page(thread(ThreadKind::Pair))));
    assert!(!Seat::Phone.permits(&page(thread(ThreadKind::Call))));
    assert!(!Seat::Phone.permits(&page(ThreadId::dm(crate::voice::TAPE_ID))));
    for kind in [
        ThreadKind::Dm,
        ThreadKind::Side,
        ThreadKind::Pair,
        ThreadKind::Run,
    ] {
        assert!(
            Seat::Phone.permits_sub(&Target::ThreadId(ThreadId::new(kind, "k"))),
            "{kind:?}"
        );
    }
    assert!(!Seat::Phone.permits_sub(&Target::ThreadId(thread(ThreadKind::Call))));
    assert!(!Seat::Phone.permits_sub(&Target::ThreadId(ThreadId::dm(crate::voice::TAPE_ID))));
    // The desk and an owner may do all of it.
    for seat in [Seat::Desk, Seat::Owner] {
        assert!(seat.permits(&prompt(ThreadKind::Dm)));
        assert!(seat.permits(&answer(ThreadKind::Pair)));
        assert!(seat.permits_sub(&Target::ThreadId(thread(ThreadKind::Call))));
    }
}

/// The verbs a kind does have reach what the older command reached: a request
/// for the person is answered through the teammate, a pair's cancel and
/// continue are its exchange's stop and resume.
#[tokio::test]
async fn thread_verbs_reach_the_handlers_the_older_commands_did() {
    let (_root, log, port) = door("thread-verbs-reach");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let bob = create(&mut socket, 2, "Bob").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let pair = paths::thread_key(&ada, &bob).unwrap();
    crate::log::thread::ensure(log.root(), &pair).unwrap();

    let human =
        json!({ "answer": { "kind": "human", "actionId": "a", "status": "done", "note": "ok" } });
    for (id, cmd, params) in [
        (
            3,
            "thread.answer",
            json!({ "thread": { "kind": "dm", "key": ada }, "answer": human["answer"] }),
        ),
        (
            4,
            "thread.cancel",
            json!({ "thread": { "kind": "pair", "key": pair } }),
        ),
    ] {
        ask(
            &mut socket,
            json!({ "id": id, "cmd": cmd, "params": params }),
        )
        .await;
        let answer = answered(&mut socket, id).await;
        assert_eq!(answer["ok"], true, "{cmd}: {answer}");
        assert!(
            answer.get("result").is_none(),
            "{cmd} answers nothing: {answer}"
        );
    }
    ask(
        &mut socket,
        json!({ "id": 5, "cmd": "thread.continue", "params": { "thread": { "kind": "pair", "key": pair } } }),
    )
    .await;
    let resumed = answered(&mut socket, 5).await;
    assert_eq!(resumed["ok"], true, "{resumed}");
    assert_eq!(
        resumed["result"]["thread"],
        json!({ "kind": "pair", "key": pair })
    );
    // A thread the room has no record of is refused, not answered as nothing.
    ask(
        &mut socket,
        json!({ "id": 6, "cmd": "thread.answer", "params": { "thread": { "kind": "side", "key": "ghost" }, "answer": human["answer"] } }),
    )
    .await;
    assert_eq!(answered(&mut socket, 6).await["ok"], false);
}

/// A verb a kind has no meaning for is refused in a sentence, never silently.
#[tokio::test]
async fn a_thread_verb_the_kind_has_no_meaning_for_is_refused() {
    let (_root, _log, port) = door("thread-verbs");
    let mut socket = desk(port).await;
    let mut id = 0;
    for (cmd, kind, params) in [
        ("thread.prompt", "pair", json!({ "text": "hi" })),
        ("thread.prompt", "run", json!({ "text": "hi" })),
        ("thread.prompt", "call", json!({ "text": "hi" })),
        ("thread.park", "dm", json!({})),
        ("thread.close", "dm", json!({})),
        ("thread.close", "run", json!({})),
        ("thread.continue", "dm", json!({})),
        ("thread.cancel", "run", json!({})),
        (
            "thread.answer",
            "run",
            json!({ "answer": { "kind": "permission", "requestId": "r", "optionId": "o" } }),
        ),
        (
            "thread.answer",
            "call",
            json!({ "answer": { "kind": "human", "actionId": "a", "status": "done" } }),
        ),
        (
            "thread.answer",
            "pair",
            json!({ "answer": { "kind": "human", "actionId": "a", "status": "done" } }),
        ),
    ] {
        id += 1;
        let mut params = params;
        params["thread"] = json!({ "kind": kind, "key": "k" });
        ask(
            &mut socket,
            json!({ "id": id, "cmd": cmd, "params": params }),
        )
        .await;
        let answer = answered(&mut socket, id).await;
        assert_eq!(answer["ok"], false, "{cmd} {kind}: {answer}");
        assert!(
            answer["error"].as_str().unwrap().ends_with('.'),
            "{cmd} {kind}: {answer}"
        );
    }
}

#[tokio::test]
async fn the_window_opens_on_a_tapes_last_lines_and_pages_back_to_the_first() {
    let (_root, log, port) = door("tape-window");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let persona_id = ada["id"].as_str().unwrap().to_string();
    let tape = StreamId::Tape(persona_id.clone());
    for n in 0..1_000 {
        log.append(
            &tape,
            &json!({ "kind": "user", "id": format!("m{n}"), "ts": n, "text": "line" }),
        )
        .unwrap();
    }

    ask(
        &mut socket,
        json!({ "id": 2, "sub": { "tape": persona_id } }),
    )
    .await;
    let snapshot = heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    let lines = snapshot["snapshot"].as_array().unwrap();
    assert_eq!(lines.len(), 400);
    assert_eq!(lines[0]["id"], "m600");
    assert_eq!(lines[399]["id"], "m999");

    let page = |id: i64, params: Value| json!({ "id": id, "cmd": "tape.page", "params": params });
    ask(
        &mut socket,
        page(3, json!({ "personaId": persona_id, "before": "m600" })),
    )
    .await;
    let older = answered(&mut socket, 3).await;
    assert_eq!(older["ok"], true, "{older}");
    let events = older["result"]["events"].as_array().unwrap();
    assert_eq!(
        (events.len(), &events[0]["id"], &events[399]["id"]),
        (400, &json!("m200"), &json!("m599"))
    );
    assert_eq!(older["result"]["more"], true);

    ask(
        &mut socket,
        page(4, json!({ "personaId": persona_id, "before": "m200" })),
    )
    .await;
    let first = answered(&mut socket, 4).await;
    assert_eq!(first["result"]["events"].as_array().unwrap().len(), 200);
    assert_eq!(first["result"]["more"], false);

    // A search hit far up: one page reaches it, with some context above.
    ask(
        &mut socket,
        page(
            5,
            json!({ "personaId": persona_id, "before": "m600", "through": "m50" }),
        ),
    )
    .await;
    let reached = answered(&mut socket, 5).await;
    let events = reached["result"]["events"].as_array().unwrap();
    assert_eq!(events[0]["id"], "m10");
    assert_eq!(events.last().unwrap()["id"], "m599");
    assert_eq!(reached["result"]["more"], true);

    // A line the tape no longer holds ends the window rather than failing it.
    ask(
        &mut socket,
        page(6, json!({ "personaId": persona_id, "before": "gone" })),
    )
    .await;
    let gone = answered(&mut socket, 6).await;
    assert_eq!(gone["result"], json!({ "events": [], "more": false }));
}

#[test]
fn an_owner_device_opens_a_tape_on_the_same_window_as_the_desk() {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let tape = StreamId::Tape("ada".into());
    for n in 0..1_000 {
        log.append(
            &tape,
            &json!({ "kind": "user", "id": format!("m{n}"), "ts": n, "text": "line" }),
        )
        .unwrap();
    }
    for seat in [Seat::Desk, Seat::Owner] {
        let lines = snapshot_for_seat(&log, &tape, seat, false);
        assert_eq!((lines.len(), &lines[0]["id"]), (400, &json!("m600")));
    }
    assert_eq!(
        snapshot_for_seat(&log, &tape, Seat::Phone, false).len(),
        200
    );
}

#[test]
fn a_long_turn_of_steps_still_opens_a_tape_on_the_last_message() {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let tape = StreamId::Tape("ada".into());
    log.append(
        &tape,
        &json!({ "kind": "user", "id": "ask", "ts": 0, "text": "go" }),
    )
    .unwrap();
    for n in 1..=500 {
        log.append(
            &tape,
            &json!({ "kind": "thought", "id": format!("t{n}"), "ts": n, "text": "hm" }),
        )
        .unwrap();
    }
    for seat in [Seat::Desk, Seat::Owner, Seat::Phone] {
        let lines = snapshot_for_seat(&log, &tape, seat, false);
        assert_eq!(lines[0]["id"], "ask", "{seat:?}");
        assert_eq!(lines.last().unwrap()["id"], "t500", "{seat:?}");
    }
}

#[tokio::test]
async fn capabilities_options_offer_chatgpt_images_as_the_automatic_subscription() {
    use crate::credentials::tests::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::default());
    let log = crate::log::Log::open(root.path());
    let vault = crate::vault::Vault::open_with_store(root.path(), log, store.clone()).unwrap();
    let (id, token_dir) = vault.begin_login("openai-codex").unwrap();
    vault.finish_login(&id, "openai-codex", "ChatGPT").unwrap();
    drop(vault);
    let desk = Arc::new(crate::desk::Desk::open_with_store(root.path(), store).unwrap());
    let handle: Arc<dyn RoomHandle> = desk.clone();
    let request = json!({"id": 1, "cmd": "capabilities.options", "params": {}});
    for seat in [Seat::Desk, Seat::Owner] {
        let offered = remote_control_answer(seat, &handle, &desk.log, request.clone()).await;
        assert_eq!(offered["ok"], true, "{offered}");
        assert_eq!(
            offered["result"]["images"]["options"],
            json!([{
                "providerId": "openai-codex",
                "providerName": "Codex (ChatGPT subscription)",
                "models": [{"id": "gpt-image-2"}]
            }])
        );
        assert_eq!(
            offered["result"]["images"]["automatic"]["providerId"],
            "openai-codex"
        );
        assert!(offered["result"]["images"].get("selected").is_none());
        assert!(
            !offered
                .to_string()
                .contains(&token_dir.to_string_lossy().to_string())
        );
    }
    desk.credential_create("openai", "API", "options-test-key")
        .unwrap();
    let selected = remote_control_answer(
        Seat::Owner,
        &handle,
        &desk.log,
        json!({"id": 2, "cmd": "settings.update", "params": {"patch": {
            "images": {"provider": "openai-codex", "model": "gpt-image-2"}
        }}}),
    )
    .await;
    assert_eq!(selected["ok"], true, "{selected}");
    let offered = remote_control_answer(Seat::Desk, &handle, &desk.log, request.clone()).await;
    assert_eq!(
        offered["result"]["images"]["selected"],
        json!({
            "providerId": "openai-codex",
            "providerName": "Codex (ChatGPT subscription)",
            "modelId": "gpt-image-2"
        })
    );
    // A subscription stays the automatic pick when a paid key joins it.
    assert_eq!(
        offered["result"]["images"]["automatic"]["providerId"],
        "openai-codex"
    );
    let denied = remote_control_answer(Seat::Phone, &handle, &desk.log, request).await;
    assert_eq!(denied["code"], FORBIDDEN);
    assert_eq!(
        std::fs::read_to_string(token_dir.join("auth.json")).unwrap(),
        "{}"
    );
}

/// The Tools pane's web search commands are the desk's: a keys-and-switches
/// surface a companion phone must not reach, and a key is never answered.
#[test]
fn only_the_desk_and_owner_seats_may_touch_web_search_settings() {
    use crate::contract::WebSearchProvider;
    let status = Command::WebsearchStatus {};
    let enable = Command::WebsearchSetEnabled {
        provider: WebSearchProvider::Exa,
        enabled: false,
    };
    let key = Command::WebsearchSetKey {
        provider: WebSearchProvider::Exa,
        key: Some("exa-key-0123456789".to_string()),
    };
    for command in [&status, &enable, &key] {
        assert!(Seat::Desk.permits(command), "{command:?}");
        assert!(Seat::Owner.permits(command), "{command:?}");
        assert!(!Seat::Phone.permits(command), "{command:?}");
    }
}

#[tokio::test]
async fn web_search_switches_and_keys_round_trip_without_the_key_ever_leaving() {
    use crate::credentials::tests::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    let handle: Arc<dyn RoomHandle> = desk.clone();
    let log = &desk.log;
    let ask = |cmd: &str, params: Value| json!({"id": 1, "cmd": cmd, "params": params});
    let key = "exa-private-key-0123456789";

    let fresh =
        remote_control_answer(Seat::Desk, &handle, log, ask("websearch.status", json!({}))).await;
    assert_eq!(fresh["ok"], true, "{fresh}");
    assert_eq!(
        fresh["result"]["providers"],
        json!([
            {"provider": "parallel", "name": "Parallel", "enabled": true, "hasKey": false},
            {"provider": "exa", "name": "Exa", "enabled": true, "hasKey": false},
            {"provider": "keenable", "name": "Keenable", "enabled": true, "hasKey": false},
            {"provider": "firecrawl", "name": "Firecrawl", "enabled": true, "hasKey": false},
        ])
    );

    let off = remote_control_answer(
        Seat::Owner,
        &handle,
        log,
        ask(
            "websearch.set_enabled",
            json!({"provider": "keenable", "enabled": false}),
        ),
    )
    .await;
    assert_eq!(off["result"]["providers"][2]["enabled"], false, "{off}");
    assert_eq!(
        crate::room::settings(log)["webSearch"],
        json!({"disabled": ["keenable"]})
    );

    let saved = remote_control_answer(
        Seat::Desk,
        &handle,
        log,
        ask("websearch.set_key", json!({"provider": "exa", "key": key})),
    )
    .await;
    assert_eq!(saved["result"]["providers"][1]["hasKey"], true, "{saved}");
    assert!(!saved.to_string().contains(key));
    assert_eq!(
        desk.saved_web_search_key(crate::contract::WebSearchProvider::Exa),
        Some(key.to_string())
    );
    assert!(!format!("{:?}", log.load(&StreamId::Room)).contains(key));

    let on = remote_control_answer(
        Seat::Desk,
        &handle,
        log,
        ask(
            "websearch.set_enabled",
            json!({"provider": "keenable", "enabled": true}),
        ),
    )
    .await;
    assert_eq!(on["result"]["providers"][2]["enabled"], true);
    assert_eq!(
        crate::room::settings(log)["webSearch"],
        json!({"disabled": []})
    );

    let cleared = remote_control_answer(
        Seat::Desk,
        &handle,
        log,
        ask("websearch.set_key", json!({"provider": "exa", "key": null})),
    )
    .await;
    assert_eq!(
        cleared["result"]["providers"][1]["hasKey"], false,
        "{cleared}"
    );

    let refused = remote_control_answer(
        Seat::Phone,
        &handle,
        log,
        ask("websearch.status", json!({})),
    )
    .await;
    assert_eq!(refused["code"], FORBIDDEN, "{refused}");
    let bad = remote_control_answer(
        Seat::Desk,
        &handle,
        log,
        ask(
            "websearch.set_key",
            json!({"provider": "brave", "key": key}),
        ),
    )
    .await;
    assert_eq!(bad["ok"], false, "{bad}");
}
