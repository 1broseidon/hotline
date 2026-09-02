//! The desk: the room, its vault and its log, standing behind the wire.
//!
//! The wire asks a [`RoomHandle`] for what it does not own — a session, a
//! secret, the models a key can reach — and the session room asks a
//! [`ProviderKeys`] for credentials. This is the one place those two seams
//! are joined to the real things, so the shell and the headless harness open
//! a desk the same way and get the same room.

use crate::contract::{
    Attachment, BackendChoice, ChapterClose, ChapterSummary, ConfigChoice, Credential, LoginPrompt,
    LoginState, LoginStatus, SessionInfo, StreamDelta,
};
use crate::driver::{PI_BACKEND_ID, acp};
use crate::log::Log;
use crate::models::Client;
use crate::session::{ProviderKeys, Room};
use crate::vault::Vault;
use crate::wire::RoomHandle;
use async_trait::async_trait;
use rig::providers::{chatgpt, copilot};
use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::{broadcast, oneshot};

/// The vault's keys and the room's model filter, as one seam.
///
/// The driver builds `SessionInfo.models` from [`ProviderKeys`] alone and
/// has no log. Reading the setting here, each time, is how a saved filter
/// is in force on the next turn without a restart.
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
}

/// How far an in-flight device-code login has got. Lives only in this
/// process: a finished login stays queryable until Toad exits.
enum LoginOutcome {
    Pending,
    Done(Credential),
    Failed(String),
}

/// Everything that runs behind one data directory.
pub struct Desk {
    pub log: Log,
    room: Arc<Room>,
    vault: Arc<Vault>,
    logins: Arc<Mutex<HashMap<String, LoginOutcome>>>,
}

impl Desk {
    /// Opens the log, the vault and the room over one data directory.
    pub fn open(root: &Path) -> io::Result<Desk> {
        let log = Log::open(root);
        let vault = Arc::new(Vault::open(root, log.clone())?);
        let keys = Arc::new(DeskCredentials {
            vault: vault.clone(),
            log: log.clone(),
        });
        let room = Room::new(log.clone(), keys);
        Ok(Desk {
            log,
            room,
            vault,
            logins: Arc::new(Mutex::new(HashMap::new())),
        })
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

    async fn credential_login(&self, provider_id: &str) -> Result<LoginPrompt, String> {
        if let Some(message) = crate::models::login_refusal(provider_id) {
            return Err(message);
        }
        let wiring = crate::models::wiring(provider_id)
            .ok_or_else(|| format!("{provider_id} is not a provider Toad Agent can use."))?;
        let label = crate::models::catalog()
            .providers
            .get(wiring.id)
            .map(|entry| entry.name.clone())
            .unwrap_or_else(|| wiring.id.to_string());
        let (id, token_dir) = self
            .vault
            .begin_login(provider_id)
            .map_err(|error| error.to_string())?;
        self.logins().insert(id.clone(), LoginOutcome::Pending);

        let (prompt_tx, prompt_rx) = oneshot::channel();
        let prompt_tx = Arc::new(Mutex::new(Some(prompt_tx)));
        let login_id = id.clone();
        let emit = {
            let prompt_tx = prompt_tx.clone();
            let login_id = login_id.clone();
            move |user_code: String, verification_uri: String| {
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
            }
        };

        let vault = self.vault.clone();
        let logins = self.logins.clone();
        let id_for_task = id.clone();
        let provider_for_task = provider_id.to_string();
        let label_for_task = label.clone();

        let mut task = match wiring.client {
            Client::ChatGpt => {
                let client = chatgpt::Client::builder()
                    .oauth()
                    .auth_file(token_dir.join("auth.json"))
                    .on_device_code(move |prompt| emit(prompt.user_code, prompt.verification_uri))
                    .allow_device_flow(true)
                    .build()
                    .map_err(|error| error.to_string());
                let client = match client {
                    Ok(client) => client,
                    Err(error) => {
                        let _ = self.vault.abandon_login(&id);
                        self.logins().remove(&id);
                        return Err(error);
                    }
                };
                tokio::spawn(async move {
                    record_login(
                        vault,
                        logins,
                        id_for_task,
                        provider_for_task,
                        label_for_task,
                        client.authorize().await.map_err(|error| error.to_string()),
                    );
                })
            }
            Client::Copilot => {
                let client = copilot::Client::builder()
                    .oauth()
                    .token_dir(&token_dir)
                    .on_device_code(move |prompt| emit(prompt.user_code, prompt.verification_uri))
                    .allow_device_flow(true)
                    .build()
                    .map_err(|error| error.to_string());
                let client = match client {
                    Ok(client) => client,
                    Err(error) => {
                        let _ = self.vault.abandon_login(&id);
                        self.logins().remove(&id);
                        return Err(error);
                    }
                };
                tokio::spawn(async move {
                    record_login(
                        vault,
                        logins,
                        id_for_task,
                        provider_for_task,
                        label_for_task,
                        client.authorize().await.map_err(|error| error.to_string()),
                    );
                })
            }
            _ => {
                let _ = self.vault.abandon_login(&id);
                self.logins().remove(&id);
                return Err(format!("{provider_id} takes an API key, not a sign-in."));
            }
        };

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
                task.abort();
                let _ = self.vault.abandon_login(&id);
                let message = "Timed out waiting for a sign-in code.".to_string();
                self.logins().insert(id, LoginOutcome::Failed(message.clone()));
                Err(message)
            }
        }
    }

    fn login_status(&self, login_id: &str) -> Result<LoginStatus, String> {
        match self.logins().get(login_id) {
            Some(LoginOutcome::Pending) => Ok(LoginStatus {
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

    fn models_catalog(
        &self,
        provider_id: &str,
    ) -> Result<Vec<crate::contract::CatalogModel>, String> {
        if crate::models::wiring(provider_id).is_none() {
            return Err(format!(
                "{provider_id} is not a provider Toad Agent can use."
            ));
        }
        Ok(crate::models::catalog_models(
            provider_id,
            &crate::models::enabled_models(&crate::room::settings(&self.log)),
        ))
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

fn record_login(
    vault: Arc<Vault>,
    logins: Arc<Mutex<HashMap<String, LoginOutcome>>>,
    id: String,
    provider_id: String,
    label: String,
    result: Result<(), String>,
) {
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
    logins
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(id, outcome);
}
