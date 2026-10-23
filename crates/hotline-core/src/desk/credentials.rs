//! A Hotline Agent account on the desk: a key made, a server connected, a
//! sign-in begun and where it lands, and a change made in place. Each is
//! named here, so a name is checked or settled before anything is written.

use super::{Desk, LoginOutcome};
use crate::contract::{AccountSection, Credential, CredentialKind, LoginPrompt};
use crate::log::{Log, StreamId};
use crate::models::Client;
use crate::providers::rig_login;
use crate::vault::{CANNOT_CHANGE, CANNOT_REPLACE, CredentialChange, Vault};
use crate::wire::{CredentialUpdate, RoomHandle};
use serde_json::json;
use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

/// What a finished sign-in becomes.
enum Landing {
    /// A new account, named `label` or else its provider's next free name.
    New {
        provider_id: String,
        label: Option<String>,
    },
    /// New tokens for the account `into`, which keeps its id, name and place.
    Again { into: String },
}

/// A sign-in under way in its own fresh folder, `id`.
struct SignIn {
    vault: Arc<Vault>,
    logins: Arc<Mutex<HashMap<String, LoginOutcome>>>,
    log: Log,
    id: String,
    client: Client,
    landing: Landing,
}

impl Desk {
    pub(super) fn create_key(
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
        let section = Vault::section_of(provider_id, CredentialKind::ApiKey);
        let label = self.vault.check_label(provider_id, section, label, None)?;
        self.vault
            .create(provider_id, &label, secret.trim())
            .map_err(|error| error.to_string())
    }

    pub(super) async fn connect_server(
        &self,
        base_url: &str,
        label: Option<&str>,
    ) -> Result<Credential, String> {
        let section = Vault::section_of("ollama", CredentialKind::Local);
        let label = label
            .map(|label| self.vault.check_label("ollama", section, label, None))
            .transpose()?;
        let base_url = crate::providers::ollama_url(base_url)?;
        let ids = crate::providers::ollama_models(&base_url, "").await?;
        let label = label.unwrap_or_else(|| self.vault.free_label("ollama", "Ollama Local"));
        self.vault
            .connect_local(&base_url, &ids, &label)
            .map_err(|error| error.to_string())
    }

    /// Every part is checked before any is written, and a server's models
    /// are found before its URL changes.
    pub(super) async fn change_credential(
        &self,
        id: &str,
        change: CredentialUpdate,
    ) -> Result<Credential, String> {
        let credential = self.vault.find(id).map_err(|error| error.to_string())?;
        let section = Vault::section_of(&credential.provider_id, credential.credential_kind);
        let label = change
            .label
            .map(|label| {
                self.vault
                    .check_label(&credential.provider_id, section, &label, Some(id))
            })
            .transpose()?;
        let secret = match change.secret.as_deref().map(str::trim) {
            Some("") => return Err("Enter an API key.".into()),
            secret => secret.map(str::to_string),
        };
        let base_url = match change.base_url {
            None => None,
            Some(_) if credential.custom.is_some() || section != AccountSection::Server => {
                return Err(CANNOT_CHANGE.into());
            }
            Some(base_url) => {
                let base_url = crate::providers::ollama_url(&base_url)?;
                let ids = crate::providers::ollama_models(&base_url, "").await?;
                Some((base_url, ids))
            }
        };
        let change = CredentialChange {
            label,
            secret,
            base_url,
        };
        self.vault
            .update(id, change)
            .map_err(|error| error.to_string())
    }

    pub(super) async fn sign_in(
        &self,
        provider_id: &str,
        label: Option<&str>,
        into: Option<&str>,
    ) -> Result<LoginPrompt, String> {
        if let Some(message) = crate::models::login_refusal(provider_id) {
            return Err(message);
        }
        let wiring = crate::models::wiring(provider_id)
            .ok_or_else(|| format!("{provider_id} is not a provider Hotline Agent can use."))?;
        let landing = if let Some(into) = into {
            // Before the person signs in rather than after; and the provider
            // is the one part of this `adopt_login` cannot check.
            let again = self.vault.find(into).is_ok_and(|credential| {
                credential.provider_id == wiring.id
                    && credential.credential_kind == CredentialKind::Oauth
                    && !credential.revoked
            });
            if !again {
                return Err(CANNOT_REPLACE.into());
            }
            Landing::Again {
                into: into.to_string(),
            }
        } else {
            let section = Vault::section_of(wiring.id, CredentialKind::Oauth);
            Landing::New {
                provider_id: wiring.id.to_string(),
                label: label
                    .map(|label| self.vault.check_label(wiring.id, section, label, None))
                    .transpose()?,
            }
        };
        let (id, token_dir) = self
            .vault
            .begin_login()
            .map_err(|error| error.to_string())?;
        let tokens = self.vault.login_tokens(&id);
        let cancel = CancellationToken::new();
        self.logins()
            .insert(id.clone(), LoginOutcome::Pending(cancel.clone()));
        let (prompt_tx, prompt_rx) = oneshot::channel();
        let prompt_tx = Arc::new(Mutex::new(Some(prompt_tx)));
        let login_id = id.clone();
        let emit = move |user_code: String, verification_uri: String| {
            let tx = prompt_tx
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(tx) = tx {
                let _ = tx.send(LoginPrompt {
                    login_id: login_id.clone(),
                    user_code,
                    verification_uri,
                });
            }
        };
        let sign_in = SignIn {
            vault: self.vault.clone(),
            logins: self.logins.clone(),
            log: self.log.clone(),
            id: id.clone(),
            client: wiring.client,
            landing,
        };
        let provider_id = provider_id.to_string();
        let mut task = tokio::spawn(async move {
            let authorize = async {
                match wiring.client {
                    Client::ChatGpt => rig_login::chatgpt_sign_in(&token_dir, move |prompt| {
                        emit(prompt.user_code, prompt.verification_uri);
                    })
                    .await
                    .map(drop),
                    Client::Copilot => rig_login::copilot_sign_in(&token_dir, move |prompt| {
                        emit(prompt.user_code, prompt.verification_uri);
                    })
                    .await
                    .map(drop),
                    Client::OpenRouter => crate::providers::openrouter_login(&tokens, emit).await,
                    Client::XAi => crate::providers::xai::login(&tokens, emit).await,
                    _ => Err(format!("{provider_id} does not support sign-in.")),
                }
            };
            let result = tokio::select! {
                biased;
                () = cancel.cancelled() => Err("Sign-in cancelled.".to_string()),
                result = tokio::time::timeout(Duration::from_mins(10), authorize) =>
                    result.unwrap_or_else(|_| Err("Sign-in timed out. Try again.".to_string())),
            };
            sign_in.land(result).await;
        });
        tokio::select! {
            biased;
            prompt = prompt_rx => if let Ok(prompt) = prompt { Ok(prompt) } else {
                let _ = (&mut task).await;
                Err(self.login_error(&id))
            },
            _ = &mut task => Err(self.login_error(&id)),
            () = tokio::time::sleep(Duration::from_secs(30)) => {
                self.credential_login_cancel(&id)?;
                Err("Timed out waiting for sign-in instructions.".to_string())
            }
        }
    }
}

impl SignIn {
    /// A Copilot account's models are fetched into the folder that holds its
    /// tokens once the sign-in has landed, which for one signed in again is
    /// the account's own and no longer the sign-in's.
    async fn land(self, result: Result<(), String>) {
        // A refresh in flight on `into`'s old tokens saves before the new
        // ones land. Let go before the models are fetched, which may refresh.
        let records = match (&self.landing, &result) {
            (Landing::Again { into }, Ok(())) => {
                Some(rig_login::hold_records(&self.vault.login_dir(into)).await)
            }
            _ => None,
        };
        let landed = self.record(result);
        drop(records);
        if self.client == Client::Copilot
            && let Some(id) = landed
        {
            store_copilot_account_models(&self.vault.login_dir(&id), &self.log).await;
        }
    }

    /// Under the logins lock, so a cancel that came first wins and two new
    /// accounts never settle on the same free name. Answers the id of the
    /// account that holds the tokens now.
    fn record(&self, result: Result<(), String>) -> Option<String> {
        let mut logins = self.logins.lock().unwrap_or_else(PoisonError::into_inner);
        if !matches!(logins.get(&self.id), Some(LoginOutcome::Pending(cancel)) if !cancel.is_cancelled())
        {
            let _ = self.vault.abandon_login(&self.id);
            return None;
        }
        let finished = result.and_then(|()| self.landed().map_err(|error| error.to_string()));
        let outcome = match finished {
            Ok(credential) => LoginOutcome::Done(credential),
            Err(error) => {
                let _ = self.vault.abandon_login(&self.id);
                LoginOutcome::Failed(error)
            }
        };
        let landed = match &outcome {
            LoginOutcome::Done(credential) => Some(credential.id.clone()),
            _ => None,
        };
        logins.insert(self.id.clone(), outcome);
        landed
    }

    fn landed(&self) -> io::Result<Credential> {
        match &self.landing {
            Landing::New { provider_id, label } => {
                let label = label.clone().unwrap_or_else(|| {
                    let name = crate::models::catalog()
                        .providers
                        .get(provider_id)
                        .map_or_else(|| provider_id.clone(), |entry| entry.name.clone());
                    self.vault.free_label(provider_id, &name)
                });
                self.vault.finish_login(&self.id, provider_id, &label)
            }
            Landing::Again { into } => self.vault.adopt_login(&self.id, into),
        }
    }
}

async fn store_copilot_account_models(token_dir: &Path, log: &Log) {
    match fetch_copilot_account_models(token_dir).await {
        Ok(ids) => {
            if crate::vault::write_account_models(token_dir, &ids).is_err() {
                notice_unread_account_models(log);
            } else {
                crate::room::models_changed(log);
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

async fn fetch_copilot_account_models(token_dir: &Path) -> Result<Vec<String>, String> {
    Ok(crate::providers::discovery::copilot_models(token_dir)
        .await?
        .into_iter()
        .map(|model| model.id)
        .collect())
}

#[cfg(test)]
mod tests;
