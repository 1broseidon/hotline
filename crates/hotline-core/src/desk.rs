//! The desk: the room, its vault and its log, standing behind the wire.
//!
//! The wire asks a [`RoomHandle`] for what it does not own — a session, a
//! secret, the models a key can reach — and the session room asks a
//! [`ProviderKeys`] for credentials. This is the one place those two seams
//! are joined to the real things, so the shell and the headless harness open
//! a desk the same way and get the same room.

use crate::contract::{
    Attachment, BackendChoice, CatalogModel, ChapterClose, ChapterSummary, ConfigChoice,
    Credential, CredentialKind, LoginPrompt, LoginState, LoginStatus, SessionInfo, SkillEntry,
    SkillSource, StreamDelta,
};
use crate::driver::{HOTLINE_BACKEND_ID, acp};
use crate::log::{Log, StreamId};
use crate::mcp::{McpOAuthService, McpServer};
use crate::models::Client;
use crate::session::{ProviderAuth, ProviderKeys, Room};
use crate::vault::Vault;
use crate::wire::RoomHandle;
use crate::{paths, room, skills};
use async_trait::async_trait;
use rig::providers::{chatgpt, copilot};
use serde_json::json;
use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::{broadcast, oneshot};
use tokio_util::sync::CancellationToken;

/// The vault's keys and the room's model settings, as one seam.
///
/// The driver builds `SessionInfo.models` from [`ProviderKeys`] alone and
/// has no log. Reading the filter and the preferred model here, each time,
/// is how a saved choice is in force on the next turn without a restart.
struct DeskCredentials {
    vault: Arc<Vault>,
    log: Log,
}

impl ProviderKeys for DeskCredentials {
    fn provider_auth(&self) -> HashMap<String, crate::session::ProviderAuth> {
        Vault::provider_auth(&self.vault)
    }

    fn enabled_models(&self) -> HashMap<String, Vec<String>> {
        crate::models::enabled_models(&crate::room::settings(&self.log))
    }

    fn preferred_model(&self) -> Option<String> {
        crate::models::preferred_model(&crate::room::settings(&self.log))
    }

    fn model_metadata(&self) -> HashMap<String, CatalogModel> {
        self.vault.model_metadata()
    }

    fn account_models(&self) -> HashMap<String, Vec<String>> {
        self.vault.account_models()
    }
}

/// How far an in-flight device-code login has got. Lives only in this
/// process: a finished login stays queryable until Hotline exits.
enum LoginOutcome {
    Pending(CancellationToken),
    Done(Credential),
    Failed(String),
}

/// Everything that runs behind one data directory.
pub struct Desk {
    pub log: Log,
    room: Arc<Room>,
    vault: Arc<Vault>,
    logins: Arc<Mutex<HashMap<String, LoginOutcome>>>,
    mcp_oauth: Arc<McpOAuthService>,
}

impl Desk {
    /// Opens the log, the vault and the room over one data directory.
    pub fn open(root: &Path) -> io::Result<Desk> {
        Self::open_with_store(root, crate::credentials::default_store())
    }

    /// Open against an explicitly supplied credential backend.
    pub fn open_with_store(
        root: &Path,
        store: Arc<dyn crate::credentials::SecretStore>,
    ) -> io::Result<Desk> {
        let log = Log::open(root);
        log.migrate_backend_id()?;
        let vault = Arc::new(Vault::open_with_store(root, log.clone(), store)?);
        let keys = Arc::new(DeskCredentials {
            vault: vault.clone(),
            log: log.clone(),
        });
        let room = Room::new_with_mcp(log.clone(), keys, vault.clone());
        let mcp_oauth = Arc::new(McpOAuthService::new(vault.clone()));
        let room_for_oauth = Arc::downgrade(&room);
        mcp_oauth.set_on_complete(Arc::new(move || {
            let Some(room) = room_for_oauth.upgrade() else {
                return;
            };
            tokio::spawn(async move {
                let gate = room.policy_update_lock();
                let _held = gate.lock().await;
                if room.invalidate_all().is_ok() {
                    let _ = room.reattach_all().await;
                }
            });
        }));
        Ok(Desk {
            log,
            room,
            vault,
            logins: Arc::new(Mutex::new(HashMap::new())),
            mcp_oauth,
        })
    }

    /// Hold through installation and restart; release on failure to allow work again.
    pub fn prepare_restart(&self) -> Result<tokio::sync::OwnedRwLockWriteGuard<()>, String> {
        self.room.prepare_restart()
    }

    fn mcp_server(&self, server_id: &str) -> Result<McpServer, String> {
        crate::mcp::servers(&crate::room::settings(&self.log))
            .into_iter()
            .find(|server| server.id == server_id)
            .ok_or_else(|| format!("There is no MCP server named {server_id}."))
    }

    fn logins(&self) -> std::sync::MutexGuard<'_, HashMap<String, LoginOutcome>> {
        self.logins.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn login_error(&self, login_id: &str) -> String {
        match self.logins().get(login_id) {
            Some(LoginOutcome::Failed(error)) => error.clone(),
            _ => "Sign-in failed before a code arrived.".to_string(),
        }
    }
}

impl Drop for Desk {
    fn drop(&mut self) {
        for outcome in self.logins().values() {
            if let LoginOutcome::Pending(cancel) = outcome {
                cancel.cancel();
            }
        }
    }
}

#[async_trait]
impl RoomHandle for Desk {
    fn protect_mcp_settings(&self, value: &serde_json::Value) -> Result<serde_json::Value, String> {
        self.vault
            .protect_mcp_settings(value)
            .map_err(|error| error.to_string())
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
        let wiring = crate::models::wiring(provider_id)
            .ok_or_else(|| format!("Unknown provider {provider_id}."))?;
        if wiring.client == Client::CustomOpenAi {
            return Err("Use the custom connection form to choose its URL and API.".into());
        }
        if !wiring.credential_kinds.contains(&CredentialKind::ApiKey) {
            return Err(format!("{provider_id} does not accept an API key."));
        }
        if secret.trim().is_empty() {
            return Err("Enter an API key.".into());
        }
        self.vault
            .create(provider_id, label, secret.trim())
            .map_err(|error| error.to_string())
    }

    fn credential_revoke(&self, id: &str) -> Result<(), String> {
        self.vault.revoke(id).map_err(|error| error.to_string())
    }

    fn credential_delete(&self, id: &str) -> Result<(), String> {
        self.vault.delete(id).map_err(|error| error.to_string())
    }

    async fn credential_login(&self, provider_id: &str) -> Result<LoginPrompt, String> {
        if let Some(message) = crate::models::login_refusal(provider_id) {
            return Err(message);
        }
        let wiring = crate::models::wiring(provider_id)
            .ok_or_else(|| format!("{provider_id} is not a provider Hotline Agent can use."))?;
        let label = crate::models::catalog()
            .providers
            .get(wiring.id)
            .map(|entry| entry.name.clone())
            .unwrap_or_else(|| wiring.id.to_string());
        let (id, token_dir) = self
            .vault
            .begin_login(provider_id)
            .map_err(|error| error.to_string())?;
        let tokens = self.vault.login_tokens(&id);
        let cancel = CancellationToken::new();
        self.logins()
            .insert(id.clone(), LoginOutcome::Pending(cancel.clone()));
        let (prompt_tx, prompt_rx) = oneshot::channel();
        let prompt_tx = Arc::new(Mutex::new(Some(prompt_tx)));
        let login_id = id.clone();
        let emit = move |user_code: String, verification_uri: String| {
            if let Some(tx) = prompt_tx
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
            {
                let _ = tx.send(LoginPrompt {
                    login_id: login_id.clone(),
                    user_code,
                    verification_uri,
                });
            }
        };
        let vault = self.vault.clone();
        let logins = self.logins.clone();
        let task_id = id.clone();
        let provider_id = provider_id.to_string();
        let log = self.log.clone();
        let mut task = tokio::spawn(async move {
            let authorize = async {
                match wiring.client {
                    Client::ChatGpt => chatgpt::Client::builder()
                        .oauth()
                        .auth_file(token_dir.join("auth.json"))
                        .on_device_code(move |prompt| {
                            emit(prompt.user_code, prompt.verification_uri)
                        })
                        .allow_device_flow(true)
                        .build()
                        .map_err(|error| error.to_string())?
                        .authorize()
                        .await
                        .map_err(|error| error.to_string()),
                    Client::Copilot => copilot::Client::builder()
                        .oauth()
                        .token_dir(&token_dir)
                        .on_device_code(move |prompt| {
                            emit(prompt.user_code, prompt.verification_uri)
                        })
                        .allow_device_flow(true)
                        .build()
                        .map_err(|error| error.to_string())?
                        .authorize()
                        .await
                        .map_err(|error| error.to_string()),
                    Client::OpenRouter => crate::providers::openrouter_login(&tokens, emit).await,
                    Client::XAi => crate::providers::xai::login(&tokens, emit).await,
                    _ => Err(format!("{provider_id} does not support sign-in.")),
                }
            };
            let result = tokio::select! {
                biased;
                _ = cancel.cancelled() => Err("Sign-in cancelled.".to_string()),
                result = tokio::time::timeout(Duration::from_secs(10 * 60), authorize) =>
                    result.unwrap_or_else(|_| Err("Sign-in timed out. Try again.".to_string())),
            };
            let recorded = record_login(vault, logins, task_id, provider_id, label, result);
            if recorded && wiring.client == Client::Copilot {
                store_copilot_account_models(&token_dir, &log).await;
            }
        });
        tokio::select! {
            biased;
            prompt = prompt_rx => match prompt {
                Ok(prompt) => Ok(prompt),
                Err(_) => {
                    let _ = (&mut task).await;
                    Err(self.login_error(&id))
                }
            },
            _ = &mut task => Err(self.login_error(&id)),
            _ = tokio::time::sleep(Duration::from_secs(30)) => {
                self.credential_login_cancel(&id)?;
                Err("Timed out waiting for sign-in instructions.".to_string())
            }
        }
    }

    fn credential_login_cancel(&self, login_id: &str) -> Result<(), String> {
        let mut logins = self.logins();
        let outcome = logins
            .get_mut(login_id)
            .ok_or_else(|| format!("There is no login {login_id}."))?;
        if let LoginOutcome::Pending(cancel) = outcome {
            cancel.cancel();
            *outcome = LoginOutcome::Failed("Sign-in cancelled.".into());
        }
        Ok(())
    }

    async fn credential_connect_local(&self, base_url: &str) -> Result<Credential, String> {
        let base_url = crate::providers::ollama_url(base_url)?;
        let ids = crate::providers::ollama_models(&base_url, "").await?;
        self.vault
            .connect_local(&base_url, &ids)
            .map_err(|error| error.to_string())
    }

    fn credential_custom_save(
        &self,
        id: Option<&str>,
        draft: crate::contract::CustomProviderDraft,
    ) -> Result<Credential, String> {
        let draft = crate::providers::custom::validate(draft)?;
        self.vault
            .save_custom(id, draft)
            .map_err(|error| error.to_string())
    }

    async fn credential_custom_models(
        &self,
        id: Option<&str>,
        base_url: &str,
        secret: Option<&str>,
    ) -> Result<Vec<String>, String> {
        let base_url = crate::providers::custom::server_url(base_url)?;
        let key = self
            .vault
            .custom_key(id, &base_url, secret)
            .map_err(|error| error.to_string())?;
        crate::providers::custom::discover(&base_url, key.as_deref()).await
    }

    fn login_status(&self, login_id: &str) -> Result<LoginStatus, String> {
        match self.logins().get(login_id) {
            Some(LoginOutcome::Pending(_)) => Ok(LoginStatus {
                state: LoginState::Pending,
                credential: None,
                error: None,
            }),
            Some(LoginOutcome::Done(credential)) => Ok(LoginStatus {
                state: LoginState::Done,
                credential: Some(credential.clone()),
                error: None,
            }),
            Some(LoginOutcome::Failed(error)) => Ok(LoginStatus {
                state: LoginState::Failed,
                credential: None,
                error: Some(error.clone()),
            }),
            None => Err(format!("There is no login {login_id}.")),
        }
    }

    /// Built-ins first, then the gateway with its invalid entries named, then
    /// the teammate's own — the ones Hotline copied there are the grant, so
    /// they are not listed twice.
    fn skills(&self, persona_id: Option<&str>) -> Result<Vec<SkillEntry>, String> {
        let mut entries = skills::builtin_entries();
        entries.extend(skills::read_folder(
            &paths::skills_path(self.log.root()),
            SkillSource::Gateway,
            false,
        ));
        if let Some(persona_id) = persona_id {
            let persona = room::roster(&self.log)
                .into_iter()
                .find(|persona| persona.id == persona_id)
                .ok_or_else(|| "There is no such teammate in this room.".to_string())?;
            entries.extend(skills::read_folder(
                &Path::new(&persona.cwd).join(skills::DIRECTORY),
                SkillSource::Workspace,
                true,
            ));
            entries.extend(skills::computer_entry(Path::new(&persona.cwd)));
        }
        Ok(entries)
    }

    /// Hotline Agent first, then whatever the ACP catalogue and the PATH say.
    async fn backends(&self) -> Vec<BackendChoice> {
        let mut choices = vec![BackendChoice {
            id: HOTLINE_BACKEND_ID.to_string(),
            name: "Hotline Agent".to_string(),
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

    async fn mcp_auth_start(&self, server_id: &str) -> Result<serde_json::Value, String> {
        let server = self.mcp_server(server_id)?;
        self.mcp_oauth.start(&server).await
    }

    async fn mcp_auth_callback(
        &self,
        login_id: &str,
        callback_url: &str,
    ) -> Result<serde_json::Value, String> {
        self.mcp_oauth.complete(login_id, callback_url).await
    }

    async fn mcp_auth_status(&self, server_id: &str) -> Result<serde_json::Value, String> {
        let server = self.mcp_server(server_id)?;
        self.mcp_oauth.status(&server).await
    }

    async fn mcp_auth_sign_out(&self, server_id: &str) -> Result<(), String> {
        self.mcp_server(server_id)?;
        self.mcp_oauth.sign_out(server_id).await
    }

    /// The server may not be in settings yet: the window saves the token
    /// first so the settings write that follows reattaches with it.
    fn mcp_secret_set(&self, server_id: &str, url: &str, secret: &str) -> Result<(), String> {
        if server_id.is_empty() || url.is_empty() || secret.is_empty() {
            return Err("A saved MCP token needs a server id, a URL and the token.".to_string());
        }
        self.vault
            .set_mcp_secret(server_id, url, secret)
            .map_err(|error| format!("could not save the MCP token: {error}"))
    }

    fn models(&self) -> Vec<ConfigChoice> {
        self.room.models_for_desk()
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
        let mut models = crate::models::catalog_models(
            provider_id,
            &crate::models::enabled_models(&crate::room::settings(&self.log)),
            self.vault
                .account_models()
                .get(provider_id)
                .map(Vec::as_slice),
        );
        let metadata = self.vault.model_metadata();
        for model in &mut models {
            if let Some(resolved) = metadata.get(&format!("{provider_id}/{}", model.id)) {
                let enabled = model.enabled;
                *model = resolved.clone();
                model.enabled = enabled;
            }
        }
        Ok(models)
    }

    fn models_manual_set(
        &self,
        provider_id: &str,
        model_ids: &[String],
    ) -> Result<Vec<CatalogModel>, String> {
        if crate::models::wiring(provider_id).is_none()
            || crate::models::is_custom(provider_id)
            || provider_id == "openai-compatible"
        {
            return Err("Choose an existing native provider connection; edit custom IDs in its connection form.".into());
        }
        let (credential, _) = self
            .vault
            .connection(provider_id)
            .ok_or_else(|| format!("There is no connection for {provider_id}."))?;
        let ids = crate::providers::discovery::validate_ids(model_ids)?;
        if provider_id == "github-copilot" && !ids.is_empty() {
            let account = self.vault.read_discovery(&credential.id).ok_or_else(|| {
                "Refresh Copilot's account models before adding a manual ID.".to_string()
            })?;
            if ids
                .iter()
                .any(|id| !account.iter().any(|model| &model.id == id))
            {
                return Err("Copilot manual IDs must be offered by the signed-in account.".into());
            }
        }
        self.vault
            .set_manual_models(&credential.id, &ids)
            .map_err(|error| error.to_string())?;
        self.models_catalog(provider_id)
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

    async fn credential_refresh_models(
        &self,
        provider_id: &str,
    ) -> Result<Vec<CatalogModel>, String> {
        let wiring = crate::models::wiring(provider_id)
            .ok_or_else(|| format!("Unknown provider {provider_id}."))?;
        let (credential, auth) = self
            .vault
            .connection(provider_id)
            .ok_or_else(|| format!("There is no connection for {provider_id}."))?;
        let discovered = match (wiring.client, auth) {
            (_, ProviderAuth::Unavailable(error)) => return Err(error),
            (Client::Ollama, ProviderAuth::Local { base_url }) => {
                crate::providers::discovery::ollama_models(&base_url, "").await?
            }
            (Client::OllamaCloud, ProviderAuth::ApiKey(key)) => {
                crate::providers::discovery::ollama_models(crate::providers::OLLAMA_CLOUD_URL, &key)
                    .await?
            }
            (Client::Copilot, ProviderAuth::Login { token_dir }) => {
                crate::providers::discovery::copilot_models(&token_dir).await?
            }
            (Client::OpenRouter, ProviderAuth::StoredLogin { tokens }) => {
                let key = crate::providers::openrouter_key(&tokens)?;
                crate::providers::discovery::api_models(Client::OpenRouter, &key).await?
            }
            (client, ProviderAuth::ApiKey(key)) if crate::models::supports_discovery(client) => {
                crate::providers::discovery::api_models(client, &key).await?
            }
            _ => {
                return Err(format!(
                    "{provider_id} does not offer model discovery for this connection. Add an exact model ID manually."
                ));
            }
        };
        self.vault
            .cache_discovery(&credential.id, &discovered)
            .map_err(|error| error.to_string())?;
        self.models_catalog(provider_id)
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

    async fn computer_browsers(&self) -> Vec<crate::contract::HostBrowser> {
        self.room.computer_browsers().await
    }

    async fn computer_cookies_preview(
        &self,
        browser_id: &str,
        profile_id: &str,
    ) -> Result<Vec<crate::contract::CookieSite>, String> {
        self.room
            .computer_cookies_preview(browser_id, profile_id)
            .await
    }

    async fn computer_cookies_import(
        &self,
        persona_id: &str,
        browser_id: &str,
        profile_id: &str,
        domains: &[String],
    ) -> Result<Vec<crate::contract::CookieSite>, String> {
        self.room
            .computer_cookies_import(persona_id, browser_id, profile_id, domains)
            .await
    }

    fn secrets_list(&self) -> Result<Vec<crate::contract::SharedSecret>, String> {
        self.vault
            .shared_secrets()
            .map_err(|error| error.to_string())
    }

    /// The vault write is the fact; a computer that cannot be handed the new
    /// value says so on its teammate's tape, and the command still succeeds.
    async fn secrets_set(
        &self,
        name: &str,
        value: &str,
    ) -> Result<crate::contract::SharedSecret, String> {
        self.vault
            .set_shared_secret(name, value)
            .map_err(|error| error.to_string())?;
        let stored = self
            .vault
            .shared_secrets()
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|secret| secret.name == name)
            .ok_or_else(|| format!("{name} was stored but is not listed; check the vault."))?;
        self.room.secrets_changed().await;
        Ok(stored)
    }

    async fn secrets_delete(&self, name: &str) -> Result<(), String> {
        self.vault
            .delete_shared_secret(name)
            .map_err(|error| error.to_string())?;
        self.room.secrets_changed().await;
        Ok(())
    }
}

fn record_login(
    vault: Arc<Vault>,
    logins: Arc<Mutex<HashMap<String, LoginOutcome>>>,
    id: String,
    provider_id: String,
    label: String,
    result: Result<(), String>,
) -> bool {
    let mut logins = logins.lock().unwrap_or_else(PoisonError::into_inner);
    if !matches!(logins.get(&id), Some(LoginOutcome::Pending(cancel)) if !cancel.is_cancelled()) {
        let _ = vault.abandon_login(&id);
        return false;
    }
    let outcome = match result {
        Ok(()) => match vault.finish_login(&id, &provider_id, &label) {
            Ok(credential) => LoginOutcome::Done(credential),
            Err(error) => {
                let _ = vault.abandon_login(&id);
                LoginOutcome::Failed(error.to_string())
            }
        },
        Err(error) => {
            let _ = vault.abandon_login(&id);
            LoginOutcome::Failed(error)
        }
    };
    let recorded = matches!(outcome, LoginOutcome::Done(_));
    logins.insert(id, outcome);
    recorded
}

async fn store_copilot_account_models(token_dir: &Path, log: &Log) {
    match fetch_copilot_account_models(token_dir).await {
        Ok(ids) => {
            if crate::vault::write_account_models(token_dir, &ids).is_err() {
                notice_unread_account_models(log);
            }
        }
        Err(_) => notice_unread_account_models(log),
    }
}

fn notice_unread_account_models(log: &Log) {
    let _ = log.append(
        &StreamId::Room,
        &crate::room::room_event(
            "notice",
            json!({
                "id": uuid::Uuid::new_v4().to_string(),
                "ts": chrono::Utc::now().timestamp_millis(),
                "level": "warn",
                "text": "The GitHub Copilot model list could not be read. Refresh under Settings → Providers retries.",
            }),
        ),
    );
}

/// The Copilot account's model ids, from Rig. Tests never call this: they
/// write `models.json` beside a login themselves.
async fn fetch_copilot_account_models(token_dir: &Path) -> Result<Vec<String>, String> {
    Ok(crate::providers::discovery::copilot_models(token_dir)
        .await?
        .into_iter()
        .map(|model| model.id)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::Desk;
    use crate::contract::{Attachment, McpPolicy, Persona, PolicyMode, Reach};
    use crate::driver::rig::Said;
    use crate::driver::{Driver, DriverInfo, Update};
    use crate::log::{Log, StreamId};
    use crate::mcp::server::TeammateTools;
    use crate::session::{Agents, ProviderAuth, ProviderKeys, Room};
    use crate::vault::Vault;
    use crate::wire::Door;
    use async_trait::async_trait;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{Value, json};
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpStream;
    use tokio::sync::mpsc;
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

    const TOKEN: &str = "desk-revocation-test-token";

    /// A deterministic agent seam for the real Desk and Door. The room still
    /// builds and owns the session tools; this test agent only keeps a clone
    /// so the wire can prove that a policy update revokes an old handle.
    #[derive(Default)]
    struct RetainingAgents {
        tools: Arc<Mutex<Vec<TeammateTools>>>,
    }

    struct TestDriver;

    #[async_trait]
    impl Driver for TestDriver {
        async fn start(&self, _persona: &Persona) -> Result<DriverInfo, String> {
            Ok(DriverInfo {
                agent_name: "Test agent".to_string(),
                ..DriverInfo::default()
            })
        }

        async fn prompt(
            &self,
            _text: String,
            _attachments: Vec<Attachment>,
            _reach: Reach,
        ) -> mpsc::Receiver<Update> {
            let (_sender, receiver) = mpsc::channel(1);
            receiver
        }

        fn cancel(&self) {}

        async fn set_model(&self, model_id: &str) -> Result<DriverInfo, String> {
            Ok(DriverInfo {
                agent_name: "Test agent".to_string(),
                current_model_id: model_id.to_string(),
                ..DriverInfo::default()
            })
        }
    }

    #[async_trait]
    impl Agents for RetainingAgents {
        fn agent(
            &self,
            _persona: &Persona,
            _preamble: String,
            _said: Vec<Said>,
            tools: TeammateTools,
            _extra_mcp: Vec<crate::mcp::McpServer>,
        ) -> Result<Arc<dyn Driver>, String> {
            self.tools.lock().unwrap().push(tools);
            Ok(Arc::new(TestDriver))
        }

        async fn complete(
            &self,
            _model_id: &str,
            _system: &str,
            _prompt: &str,
        ) -> Result<String, String> {
            Err("the desk revocation test does not summarize chapters".to_string())
        }
    }

    struct TestKeys;

    impl ProviderKeys for TestKeys {
        fn provider_auth(&self) -> HashMap<String, ProviderAuth> {
            HashMap::new()
        }
    }

    fn scratch() -> PathBuf {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "hotline-desk-revocation-{}-{suffix}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn persona(root: &Path, id: &str, name: &str) -> Persona {
        Persona {
            node: None,
            id: id.to_string(),
            name: name.to_string(),
            goal: "Keep the revocation test deterministic.".to_string(),
            face: None,
            team: None,
            backend_id: "hotline".to_string(),
            cwd: root.join(id).to_string_lossy().into_owned(),
            reach: Some(Reach::Workspace),
            model_id: None,
            mode_id: None,
            effort_id: None,
            harness_override: None,
            hop_notice: None,
            mcp_policy: McpPolicy {
                mode: PolicyMode::All,
                server_ids: Vec::new(),
            },
            skill_policy: Default::default(),
            background_work: false,
            allowed_senders: Vec::new(),
            web_search_policy: None,
            computer: None,
            subagents: None,
            session_checkpoints: Vec::new(),
            last_session_id: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn append_persona(log: &Log, persona: &Persona) {
        let event = crate::room::room_event("persona", json!(persona));
        log.append(&StreamId::Room, &event).unwrap();
    }

    /// The smallest wire client needed for a command response. The Desk's
    /// session broadcasts are intentionally ignored: command responses carry
    /// the synchronization point this test needs.
    struct Client {
        socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
        next_id: i64,
    }

    impl Client {
        async fn connect(port: u16) -> Self {
            let (socket, _) = connect_async(format!("ws://127.0.0.1:{port}/ws?token={TOKEN}"))
                .await
                .unwrap();
            Self { socket, next_id: 1 }
        }

        async fn read(&mut self) -> Value {
            loop {
                match self.socket.next().await.expect("the Door closed") {
                    Ok(Message::Text(text)) => return serde_json::from_str(&text).unwrap(),
                    Ok(_) => {}
                    Err(error) => panic!("the WebSocket failed: {error}"),
                }
            }
        }

        async fn call(&mut self, command: &str, params: Value) -> Value {
            let id = self.next_id;
            self.next_id += 1;
            self.socket
                .send(Message::text(
                    json!({ "id": id, "cmd": command, "params": params }).to_string(),
                ))
                .await
                .unwrap();
            loop {
                let frame = tokio::time::timeout(std::time::Duration::from_secs(5), self.read())
                    .await
                    .expect("the Door did not answer within five seconds");
                if frame.get("id").and_then(Value::as_i64) == Some(id) {
                    return frame;
                }
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn persona_policy_update_over_live_desk_wire_revokes_old_tools() {
        let root = scratch();
        let log = Log::open(root.clone());
        append_persona(&log, &persona(&root, "ada", "Ada"));
        append_persona(&log, &persona(&root, "bob", "Bob"));

        let agents = Arc::new(RetainingAgents::default());
        let room = Room::with_agents(log.clone(), Arc::new(TestKeys), agents.clone());
        let vault = Arc::new(Vault::open(&root, log.clone()).unwrap());
        let desk = Desk {
            log: log.clone(),
            room,
            mcp_oauth: Arc::new(crate::mcp::McpOAuthService::new(vault.clone())),
            vault,
            logins: Arc::new(Mutex::new(HashMap::new())),
        };
        let door = Door::bind(log, TOKEN.to_string(), Arc::new(desk)).unwrap();
        let port = door.port();
        tokio::spawn(door.run());
        let mut client = Client::connect(port).await;

        let started = client
            .call("session.start", json!({ "personaId": "ada" }))
            .await;
        assert_eq!(started["ok"], true, "{started}");

        let old_tools = agents.tools.lock().unwrap().first().cloned().unwrap();
        let before = old_tools.call("list_teammates", &json!({})).await.unwrap();
        assert!(before.contains("bob"), "{before}");

        let updated = client
            .call(
                "persona.update",
                json!({
                    "id": "ada",
                    "patch": { "mcpPolicy": { "mode": "none", "serverIds": [] } },
                }),
            )
            .await;
        assert_eq!(updated["ok"], true, "{updated}");

        let refused = old_tools
            .call("list_teammates", &json!({}))
            .await
            .expect_err("a policy update must revoke the retained handle");
        assert!(refused.to_lowercase().contains("revoked"), "{refused}");

        let fresh_tools = agents.tools.lock().unwrap().last().cloned().unwrap();
        let after = fresh_tools
            .call("list_teammates", &json!({}))
            .await
            .unwrap();
        assert!(after.contains("bob"), "{after}");

        let configured = client
            .call(
                "settings.update",
                json!({ "patch": { "mcpServers": [{
                "id": "oauth", "name": "OAuth", "type": "http",
                "url": "https://example.test/mcp", "auth": { "mode": "oauth" }
            }] } }),
            )
            .await;
        assert_eq!(configured["ok"], true, "{configured}");
        let active_tools = agents.tools.lock().unwrap().last().cloned().unwrap();
        let built = agents.tools.lock().unwrap().len();
        // A damaged vault makes sign-out fail. The request still revokes
        // authority and must not silently rebuild a usable session afterward.
        std::fs::create_dir_all(root.join("vault")).unwrap();
        std::fs::write(root.join("vault/mcp"), "not a directory").unwrap();
        let signed_out = client
            .call("mcp.auth_sign_out", json!({ "serverId": "oauth" }))
            .await;
        assert_eq!(signed_out["ok"], false, "{signed_out}");
        assert!(
            active_tools
                .call("list_teammates", &json!({}))
                .await
                .is_err()
        );
        assert_eq!(agents.tools.lock().unwrap().len(), built);
        std::fs::remove_file(root.join("vault/mcp")).unwrap();
        let retried = client
            .call("mcp.auth_sign_out", json!({ "serverId": "oauth" }))
            .await;
        assert_eq!(retried["ok"], true, "{retried}");
        assert!(agents.tools.lock().unwrap().len() > built);
    }
}
