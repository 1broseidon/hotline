//! The wire, driven the way a client drives it: over a real socket.
//!
//! The room behind the door is a stand-in — Phase 0's sessions and vault are
//! being built beside this — so what is proved here is the wire's own half:
//! the token, the framing, the ordering of a subscription, and the roster
//! view the core maintains out of the log.

use super::door::{AcceptAgain, accept_again};
use super::phone::{effort_config, phone_event, snapshot_shaped};
use super::seat::{PRIVATE_REASON, Seat};
use super::socket::{FORBIDDEN, Outgoing, answer};
use super::*;
use crate::contract::{
    ChapterClose, Command, ConfigChoice, Credential, CredentialKind, DeltaKind, LoginPrompt,
    LoginStatus, Persona, PersonaDraft, Reach, SessionCapabilities, SessionInfo, SessionState,
    StreamDelta, Target, ThreadId, ThreadKind, TranscriptEvent, ViewName,
};
use crate::driver::Driver;
use crate::log::{Log, StreamId};
use crate::mcp::server::TeammateTools;
use crate::session::{Agents, ProviderAuth, ProviderKeys, Room};
use crate::{paths, room};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::{Error, Message};
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
            additional_directories: false,
        },
        error: None,
        awaiting_subagents: false,
        processes: Vec::new(),
        plan_usage: None,
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
    pending_login: Option<Arc<PendingStop>>,
    /// When set, `capabilities.options` answers only once it is cancelled:
    /// a provider slow to say what it can do.
    slow_read: Option<tokio_util::sync::CancellationToken>,
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
    /// What each harness last offered, by backend id.
    offers: Mutex<HashMap<String, crate::contract::BackendOffer>>,
    /// The effort picker a live session reports once on each model, as an
    /// ACP session does after `session/set_model`.
    session_efforts: Mutex<HashMap<String, crate::contract::SessionConfig>>,
    /// Each `set_model` and `set_config` a live session was sent, in order.
    calls: Mutex<Vec<String>>,
}

impl Quiet {
    fn new() -> Self {
        Self {
            pending_stop: None,
            pending_login: None,
            slow_read: None,
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
            offers: Mutex::default(),
            session_efforts: Mutex::default(),
            calls: Mutex::default(),
        }
    }

    /// A live session on `model_id` reports this effort picker.
    fn reporting(self, model_id: &str, picker: crate::contract::SessionConfig) -> Self {
        self.session_efforts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(model_id.to_string(), picker);
        self
    }

    fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn offering(self, backend_id: &str, offer: crate::contract::BackendOffer) -> Self {
        self.offers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(backend_id.to_string(), offer);
        self
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
pub(crate) struct CoreHandle {
    pub(crate) room: Arc<Room>,
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
    async fn secrets_capture_answer(
        &self,
        answer: crate::contract::CredentialAnswer,
    ) -> Result<(), String> {
        self.room.answer_credential(answer).await
    }

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

    async fn credential_delete(&self, _id: &str) -> Result<(), String> {
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

    fn import(&self, _from: &std::path::Path) -> Result<crate::contract::Report, String> {
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
        reminder: bool,
    ) -> Result<crate::contract::ScheduledJob, String> {
        self.room.schedule_create(
            persona_id,
            crate::session::Brief::job(kind, when, every, prompt, quiet)?,
            reminder,
        )
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

    async fn computer_restart(&self, persona_id: &str) -> Result<(), String> {
        self.room.computer_restart(persona_id).await
    }
}

#[async_trait::async_trait]
impl RoomHandle for Quiet {
    fn backend_offer(&self, backend_id: &str) -> Option<crate::contract::BackendOffer> {
        self.offers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(backend_id)
            .cloned()
    }

    fn threads_waiting(&self, persona_id: &str) -> bool {
        self.thread_cards.lock().unwrap().contains(persona_id)
    }

    async fn capability_options(&self) -> Result<crate::contract::CapabilityOptions, String> {
        if let Some(release) = &self.slow_read {
            release.cancelled().await;
        }
        Err("This room cannot list what its providers can do.".into())
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
    ) -> Result<crate::wire::AuthStatus, String> {
        assert_eq!((persona_id, id), ("ada", "fixture-attempt"));
        Ok(crate::wire::AuthStatus {
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
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(format!("set_model {model_id}"));
        let mut info = self.info(persona_id);
        info.current_model_id = Some(model_id.to_string());
        if let Some(picker) = self
            .session_efforts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(model_id)
        {
            info.configs.retain(|config| config.id != picker.id);
            info.configs.push(picker.clone());
        }
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
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(format!("set_config {config_id} {value}"));
        let mut info = self.info(persona_id);
        // A picker the session reports keeps its levels; any other is said
        // back as set.
        match info
            .configs
            .iter_mut()
            .find(|config| config.id == config_id)
        {
            Some(config) if !config.options.is_empty() => {
                config.current_id = Some(value.to_string());
            }
            _ => {
                info.configs = vec![crate::contract::SessionConfig {
                    id: config_id.to_string(),
                    name: config_id.to_string(),
                    category: None,
                    current_id: Some(value.to_string()),
                    options: Vec::new(),
                    note: None,
                }];
            }
        }
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

    async fn credential_delete(&self, _id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn credential_login(&self, provider_id: &str) -> Result<LoginPrompt, String> {
        if let Some(pending) = &self.pending_login {
            let _dropped = pending.dropped.clone().drop_guard();
            pending.entered.cancel();
            pending.release.cancelled().await;
            pending.completed.cancel();
        }
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
                models: None,
                sign_in: None,
            },
            crate::contract::BackendChoice {
                id: "cursor".to_string(),
                name: "Cursor".to_string(),
                description: "An external harness.".to_string(),
                unavailable: Some("Not signed in.".to_string()),
                models: None,
                sign_in: None,
            },
            crate::contract::BackendChoice {
                id: "codex".to_string(),
                name: "Codex".to_string(),
                description: "An external harness, ready.".to_string(),
                unavailable: None,
                models: None,
                sign_in: None,
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
                account: None,
                efforts: None,
                default_effort: None,
                fast: None,
            },
            ConfigChoice {
                id: "openai/gpt".to_string(),
                name: "GPT".to_string(),
                description: None,
                group: None,
                account: None,
                efforts: None,
                default_effort: None,
                fast: None,
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

    fn import(&self, _from: &std::path::Path) -> Result<crate::contract::Report, String> {
        Ok(crate::contract::Report::default())
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
        _reminder: bool,
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
            .map_or(provider_id, |entry| entry.name.as_str());
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
            limits_changed: false,
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

    async fn computer_restart(&self, _persona_id: &str) -> Result<(), String> {
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
    let door = Door::bind(
        log.clone(),
        "desk-token".to_string(),
        room,
        Parts::default(),
    )
    .unwrap();
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
        Parts::default(),
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

/// A socket that has said it reads `threads2`, with the answer it got.
async fn hello_threads2(socket: &mut Socket, id: i64) -> Value {
    ask(
        socket,
        json!({ "id": id, "cmd": "client.hello", "params": { "capabilities": ["threads2"] } }),
    )
    .await;
    answered(socket, id).await
}

/// Exercise the same frame reader and seat gate as both socket transports.
async fn remote_control_answer(
    seat: Seat,
    room: &Arc<dyn RoomHandle>,
    log: &Log,
    parts: &Parts,
    frame: Value,
) -> Value {
    let (tx, mut frames) = mpsc::unbounded_channel();
    let outbox = Outbox {
        call_owner: "desk".into(),
        pairing_cancel: tokio_util::sync::CancellationToken::new(),
        auth_attempts: Arc::default(),
        pairing: Arc::default(),
        uploads: Arc::default(),
        threads2: Arc::default(),
        roster_options: Arc::default(),
        lean: Arc::default(),
        sender: Outgoing::Desk(tx),
        cancel: tokio_util::sync::CancellationToken::new(),
        max: usize::MAX,
    };
    let mut subscriptions = HashMap::new();
    let desk = crate::wire::Agents {
        log: log.clone(),
        room: room.clone(),
        parts: parts.clone(),
    };
    answer(&frame, seat, &desk, &outbox, &mut subscriptions, None).await;
    let response = serde_json::from_str(&frames.recv().await.unwrap()).unwrap();
    for handle in subscriptions.into_values() {
        handle.abort();
    }
    response
}

#[tokio::test]
async fn a_slow_read_answers_after_the_writes_behind_it_which_keep_their_order() {
    let release = tokio_util::sync::CancellationToken::new();
    let mut room = Quiet::new();
    room.slow_read = Some(release.clone());
    let (_root, log, port) = door_with("slow-read", Arc::new(room));
    let mut socket = desk(port).await;

    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "capabilities.options" }),
    )
    .await;
    for (id, theme) in [(2, "dark"), (3, "light")] {
        ask(
            &mut socket,
            json!({ "id": id, "cmd": "settings.update", "params": { "patch": { "theme": theme } } }),
        )
        .await;
    }
    ask(&mut socket, json!({ "id": 4, "cmd": "ping" })).await;

    // The writes and the ping come back in the order they were asked,
    // while the read ahead of them is still waiting on its provider.
    let mut order = Vec::new();
    while order.len() < 3 {
        let frame = tokio::time::timeout(Duration::from_secs(5), heard(&mut socket))
            .await
            .expect("the commands behind a slow read were held up");
        if let Some(id) = frame["id"].as_i64() {
            assert_eq!(frame["ok"], true, "{frame}");
            order.push(id);
        }
    }
    assert_eq!(order, [2, 3, 4]);
    assert_eq!(room::settings(&log)["theme"], "light");

    release.cancel();
    let read = answered(&mut socket, 1).await;
    assert_eq!(read["ok"], false, "{read}");
}

mod asks;
mod capabilities;
mod capture;
#[cfg(unix)]
mod cookie_offers;
mod deltas;
mod door;
mod files;
mod linked;
mod mobile_computer;
mod models;
mod owner_handler;
mod personas;
mod phone_pressure;
mod roster;
mod schedules;
mod seats;
#[cfg(unix)]
mod secret_uses;
mod settings;
mod threads;
mod windows;

fn snapshot_for_seat(log: &Log, stream: &StreamId, seat: Seat, threads2: bool) -> Vec<Value> {
    snapshot_shaped(
        log.load(stream),
        stream,
        seat == Seat::Phone,
        None,
        threads2,
    )
}

mod concurrency;
