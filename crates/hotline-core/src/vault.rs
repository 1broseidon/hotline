//! Credential metadata belongs to the room; secret values belong to the OS store.
//! Rig-owned ChatGPT/Copilot caches remain private files until Rig exposes a store hook.

use crate::contract::{Credential, CredentialKind};
use crate::credentials::{CredentialFile, CredentialFiles, SecretStore};
use crate::log::{Log, StreamId};
use crate::models::Client;
use crate::providers::discovery::{self, ListedModel};
use crate::session::ProviderAuth;
use async_trait::async_trait;
use rmcp::transport::auth::{CredentialRefreshGuard, CredentialStore, StoredCredentials};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::{Mutex, PoisonError};
use tokio::sync::Mutex as AsyncMutex;

mod launch;
mod shared;

pub use shared::{StoredSecret, check_rp_id, check_secret_name, check_site};

/// The client identity Hotline received from an MCP authorization server. The
/// secret is kept beside the token in the vault record; this type never crosses
/// the room or wire boundary.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct McpOAuthRegistration {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub redirect_uri: String,
    pub issuer: Option<String>,
    pub resource: String,
    pub scopes: Vec<String>,
}

/// Everything the vault keeps for one HTTP MCP server, bound to the URL it
/// was saved for. OAuth fills the registration and credentials; a bearer or
/// header server fills the secret. One file per server, so forgetting a
/// server's login forgets its token too.
#[derive(Clone, Deserialize, Serialize)]
struct McpRecord {
    server_url: String,
    registration: Option<McpOAuthRegistration>,
    credentials: Option<StoredCredentials>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    secret: Option<String>,
}

/// The vault over one data root.
///
/// It holds the log rather than opening one of its own, because a credential
/// is a room event like any other and there is exactly one writer to the room
/// stream.
pub struct Vault {
    root: PathBuf,
    log: Log,
    files: CredentialFiles,
    /// The one writer of `secrets.json`. Create and delete are each a read of
    /// the whole map, one entry changed, and the whole map written back, so
    /// two of them at once — two sockets, or a socket and a teammate's tool —
    /// would each write a map that never saw the other's key. It is also what
    /// makes the temporary file safe to name after the process: inside this
    /// process only one write is ever using it.
    writer: Mutex<()>,
    /// Refreshes for one MCP server serialize across all sessions in this
    /// process. The guard is held while rmcp reads, refreshes and saves the
    /// rotated token, so two long-lived agents cannot spend the same refresh
    /// token concurrently.
    mcp_refresh_guards: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    /// Serializes each manager's expiry check with the refresh it may trigger.
    /// This is separate from `mcp_refresh_guards`: rmcp acquires the latter
    /// from inside `get_access_token`, so reusing it here would deadlock.
    mcp_token_locks: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
}

impl Vault {
    /// A saved key belongs to an exact endpoint. Changing the URL requires
    /// explicitly supplying a new key or choosing a keyless connection.
    pub(crate) fn custom_key(
        &self,
        id: Option<&str>,
        base_url: &str,
        secret: Option<&str>,
    ) -> io::Result<Option<String>> {
        let previous = id.map(|id| self.custom_credential(id)).transpose()?;
        if let Some(secret) = secret {
            let secret = secret.trim();
            if secret.is_empty() {
                return Ok(None);
            }
            if secret.chars().any(char::is_control) {
                return Err(io::Error::other(
                    "The API key cannot contain control characters.",
                ));
            }
            return Ok(Some(secret.into()));
        }
        let Some(previous) = previous.filter(|c| c.credential_kind == CredentialKind::ApiKey)
        else {
            return Ok(None);
        };
        if previous.base_url.as_deref() != Some(base_url) {
            return Err(io::Error::other(
                "Enter the API key again when changing the server URL, or turn off API key authentication.",
            ));
        }
        read_custom_key(&self.login_tokens(&previous.id), base_url).map(Some)
    }

    fn custom_credential(&self, id: &str) -> io::Result<Credential> {
        let credential = self.find(id)?;
        if credential.custom.is_none() || credential.revoked {
            return Err(io::Error::other("This is not an active custom connection."));
        }
        Ok(credential)
    }

    pub(crate) fn save_custom(
        &self,
        id: Option<&str>,
        draft: crate::contract::CustomProviderDraft,
    ) -> io::Result<Credential> {
        let _one_writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        let previous = id.map(|id| self.custom_credential(id)).transpose()?;
        let key = self.custom_key(id, &draft.base_url, draft.secret.as_deref())?;
        let id = previous
            .as_ref()
            .map(|c| c.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        self.check_layout()?;
        let dir = self.login_dir(&id);
        make_private_directory(&dir)?;
        if let Some(key) = &key {
            let bytes = serde_json::to_vec(&CustomKey {
                base_url: draft.base_url.clone(),
                key: key.clone(),
            })
            .map_err(io::Error::other)?;
            self.login_tokens(&id).write(&bytes)?;
        } else {
            self.login_tokens(&id).delete()?;
        }
        let now = now_ms();
        let credential = Credential {
            provider_id: format!("custom-{id}"),
            id,
            credential_kind: if key.is_some() {
                CredentialKind::ApiKey
            } else {
                CredentialKind::Local
            },
            base_url: Some(draft.base_url),
            custom: Some(crate::contract::CustomProvider {
                api: draft.api,
                models: draft.models,
            }),
            label: draft.name,
            revoked: false,
            created_at: previous.map_or(now, |c| c.created_at),
            updated_at: now,
        };
        self.log.append(&StreamId::Room, &event(&credential))?;
        Ok(credential)
    }

    /// Opens the vault over a data root, refusing one whose directory or file
    /// is not the plain directory and plain file it must be.
    ///
    /// Nothing is created here. Establishing the layout is a write-time
    /// obligation: a `0700` `vault/` on a machine that has never held a secret
    /// would be a private-looking folder saying something untrue about the
    /// machine.
    pub fn open(root: impl Into<PathBuf>, log: Log) -> io::Result<Vault> {
        Self::open_with_store(root, log, crate::credentials::default_store())
    }

    /// Supply an isolated credential backend for a harness or embedding application.
    pub fn open_with_store(
        root: impl Into<PathBuf>,
        log: Log,
        store: Arc<dyn SecretStore>,
    ) -> io::Result<Vault> {
        let root = root.into();
        let vault = Vault {
            files: CredentialFiles::new(root.clone(), store),
            root,
            log,
            writer: Mutex::new(()),
            mcp_refresh_guards: Mutex::new(HashMap::new()),
            mcp_token_locks: Mutex::new(HashMap::new()),
        };
        vault.check_layout()?;
        // A locked keychain must not prevent Settings from opening. Resolution
        // retries migration and surfaces its error before starting a server.
        let _ = vault.migrate_mcp_settings();
        Ok(vault)
    }

    /// Records a new credential: the secret to disk first, then the fact of it
    /// on the room stream.
    ///
    /// That order is the safe one. A crash between the two leaves a secret
    /// nobody can name, which is invisible and unusable; the other order would
    /// leave the room claiming a credential this machine cannot use.
    pub fn create(&self, provider_id: &str, label: &str, secret: &str) -> io::Result<Credential> {
        let now = now_ms();
        let credential = Credential {
            id: uuid::Uuid::new_v4().to_string(),
            provider_id: provider_id.to_string(),
            credential_kind: CredentialKind::ApiKey,
            base_url: None,
            custom: None,
            label: label.to_string(),
            revoked: false,
            created_at: now,
            updated_at: now,
        };
        let _one_writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        let mut secrets = self.read_secrets()?;
        secrets.insert(credential.id.clone(), secret.to_string());
        self.write_secrets(&secrets)?;
        self.log.append(&StreamId::Room, &event(&credential))?;
        Ok(credential)
    }

    /// Marks a credential dead, everywhere that asks for a usable key.
    ///
    /// The secret stays on disk until `delete` takes it, because the two are
    /// different acts: revoking is the room's record that a key is not to be
    /// used, and the row keeps its label so the user can find the key they
    /// still have to rotate at the provider. Grok OAuth tokens are removed
    /// immediately so live clients cannot refresh a revoked login.
    pub fn revoke(&self, id: &str) -> io::Result<()> {
        let _one_writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        let mut credential = self.find(id)?;
        // Live Grok clients reread this directory before sending or refreshing.
        // Removing it also invalidates clients already held by a teammate.
        if credential.provider_id == "xai" && credential.credential_kind == CredentialKind::Oauth {
            self.abandon_login(id)?;
        }
        credential.revoked = true;
        credential.updated_at = now_ms();
        self.log.append(&StreamId::Room, &event(&credential))?;
        Ok(())
    }

    /// Takes both halves away: the secret or login directory off the disk,
    /// then the tombstone on the stream. Files first, so a crash between them
    /// cannot leave a usable credential behind a row that says it is gone.
    pub fn delete(&self, id: &str) -> io::Result<()> {
        let _one_writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        let credential = self.find(id)?;
        if credential.credential_kind == CredentialKind::ApiKey {
            let mut secrets = self.read_secrets()?;
            if secrets.remove(&credential.id).is_some() {
                self.write_secrets(&secrets)?;
            }
        }
        self.abandon_login(id)?;
        self.log.append(
            &StreamId::Room,
            &json!({ "kind": "credential", "id": credential.id, "deleted": true }),
        )?;
        Ok(())
    }

    /// Makes the directory and files for a fresh login, and returns the
    /// credential id and that directory. No event yet — the row lands only
    /// once the login succeeds, the same order `create` keeps (secret first,
    /// then the fact of it).
    pub fn begin_login(&self, provider_id: &str) -> io::Result<(String, PathBuf)> {
        let id = uuid::Uuid::new_v4().to_string();
        let dir = self.login_dir(&id);
        let _one_writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        self.check_layout()?;
        make_private_directory(&dir)?;
        match crate::models::wiring(provider_id).map(|wiring| wiring.client) {
            Some(Client::ChatGpt) => write_private(&dir.join("auth.json"), b"{}")?,
            Some(Client::Copilot) => {
                write_private(&dir.join("api-key.json"), b"{}")?;
                write_private(&dir.join("access-token"), b"")?;
            }
            _ => {}
        }
        Ok((id, dir))
    }

    /// Records a finished login on the room stream. The files are already on
    /// disk from [`Self::begin_login`]; this is the fact of them.
    pub fn finish_login(&self, id: &str, provider_id: &str, label: &str) -> io::Result<Credential> {
        let now = now_ms();
        let credential = Credential {
            id: id.to_string(),
            provider_id: provider_id.to_string(),
            credential_kind: CredentialKind::Oauth,
            base_url: None,
            custom: None,
            label: label.to_string(),
            revoked: false,
            created_at: now,
            updated_at: now,
        };
        self.log.append(&StreamId::Room, &event(&credential))?;
        Ok(credential)
    }

    /// Removes a login that did not finish, so a failed attempt leaves nothing.
    pub fn abandon_login(&self, id: &str) -> io::Result<()> {
        let dir = self.login_dir(id);
        self.login_tokens(id).delete()?;
        match fs::remove_dir_all(&dir) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Every credential the room knows about, in the order they were created.
    /// A tombstoned one is not one.
    pub fn list(&self) -> Vec<Credential> {
        self.log
            .load(&StreamId::Room)
            .into_iter()
            .filter(|event| event.get("kind").and_then(Value::as_str) == Some("credential"))
            .filter(|event| event.get("deleted").and_then(Value::as_bool) != Some(true))
            .filter_map(|event| serde_json::from_value(event).ok())
            .collect()
    }

    /// One usable credential per provider, keyed by provider id: the first
    /// credential created for a provider wins.
    ///
    /// A revoked row is not auth, and neither is a key whose secret is
    /// missing or a login whose directory is gone — that is what a create
    /// torn between its two writes looks like, and what a stream restored
    /// without its vault looks like. Handing the provider nothing at all
    /// beats handing it an empty string.
    pub fn provider_auth(&self) -> HashMap<String, ProviderAuth> {
        self.connections()
            .into_iter()
            .map(|(id, (_, auth))| (id, auth))
            .collect()
    }

    pub(crate) fn connection(&self, provider_id: &str) -> Option<(Credential, ProviderAuth)> {
        self.connections().remove(provider_id)
    }

    fn connections(&self) -> HashMap<String, (Credential, ProviderAuth)> {
        let secrets = self.read_secrets();
        let mut connections = HashMap::new();
        for credential in self.list() {
            if credential.revoked {
                continue;
            }
            let auth = match self.credential_auth(&credential, &secrets) {
                Ok(Some(auth)) => auth,
                Ok(None) => continue,
                Err(error) => ProviderAuth::Unavailable(error.to_string()),
            };
            connections
                .entry(credential.provider_id.clone())
                .or_insert((credential, auth));
        }
        connections
    }

    fn credential_auth(
        &self,
        credential: &Credential,
        secrets: &io::Result<BTreeMap<String, String>>,
    ) -> io::Result<Option<ProviderAuth>> {
        if let Some(config) = &credential.custom {
            let Some(base_url) = credential.base_url.clone() else {
                return Ok(None);
            };
            let key = if credential.credential_kind == CredentialKind::ApiKey {
                Some(read_custom_key(
                    &self.login_tokens(&credential.id),
                    &base_url,
                )?)
            } else {
                None
            };
            return Ok(Some(ProviderAuth::Custom {
                name: credential.label.clone(),
                base_url,
                api_key: key,
                config: config.clone(),
            }));
        }
        Ok(match credential.credential_kind {
            CredentialKind::ApiKey => match secrets {
                Ok(secrets) => secrets
                    .get(&credential.id)
                    .cloned()
                    .map(ProviderAuth::ApiKey),
                Err(error) => return Err(io::Error::new(error.kind(), error.to_string())),
            },
            CredentialKind::Oauth => {
                let dir = self.login_dir(&credential.id);
                if !dir.is_dir() {
                    return Ok(None);
                }
                if matches!(credential.provider_id.as_str(), "openrouter" | "xai") {
                    Some(ProviderAuth::StoredLogin {
                        tokens: self.login_tokens(&credential.id),
                    })
                } else {
                    Some(ProviderAuth::Login { token_dir: dir })
                }
            }
            CredentialKind::Local => credential
                .base_url
                .clone()
                .map(|base_url| ProviderAuth::Local { base_url }),
        })
    }

    /// Discovery and manual additions belong to the active credential. A new
    /// account or local server never inherits the previous connection's ids.
    pub fn account_models(&self) -> HashMap<String, Vec<String>> {
        self.connections()
            .into_iter()
            .filter_map(|(provider_id, (credential, _))| {
                if let Some(custom) = credential.custom {
                    return Some((provider_id, custom.models));
                }
                let discovered = self.read_discovery(&credential.id);
                let manual = self.read_manual_models(&credential.id);
                let mut ids = match discovered {
                    Some(models) => models.into_iter().map(|model| model.id).collect::<Vec<_>>(),
                    None if manual.is_empty() => return None,
                    None => crate::models::catalog()
                        .providers
                        .get(&provider_id)
                        .map(|entry| entry.models.keys().cloned().collect())
                        .unwrap_or_default(),
                };
                // Copilot's successful account response remains authoritative,
                // including when an older manual addition is no longer offered.
                if provider_id != "github-copilot" {
                    ids.extend(manual);
                }
                ids.sort();
                ids.dedup();
                Some((provider_id, ids))
            })
            .collect()
    }

    /// The same resolved metadata feeds Settings, pickers, and request limits.
    pub(crate) fn model_metadata(&self) -> HashMap<String, crate::contract::CatalogModel> {
        let mut result = HashMap::new();
        for (provider_id, (credential, _)) in self.connections() {
            let discovered: BTreeMap<_, _> = self
                .read_discovery(&credential.id)
                .unwrap_or_default()
                .into_iter()
                .map(|model| (model.id.clone(), model))
                .collect();
            let manual: BTreeSet<_> = self
                .read_manual_models(&credential.id)
                .into_iter()
                .collect();
            let ids: Vec<_> = discovered
                .keys()
                .chain(manual.iter())
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            for mut model in
                crate::models::catalog_models(&provider_id, &HashMap::new(), Some(&ids))
            {
                model.manual = manual.contains(&model.id);
                if let Some(listed) = discovered.get(&model.id) {
                    if let Some(name) = &listed.name {
                        model.name = name.clone();
                    }
                    model.context_limit = listed.context_limit.or(model.context_limit);
                    model.output_limit = listed.output_limit.or(model.output_limit);
                }
                result.insert(format!("{provider_id}/{}", model.id), model);
            }
        }
        result
    }

    pub(crate) fn read_discovery(&self, credential_id: &str) -> Option<Vec<ListedModel>> {
        let dir = self.checked_model_directory(credential_id).ok()?;
        if let Ok(bytes) = read_model_file(&dir.join("discovery.json"))
            && let Ok(models) = serde_json::from_slice::<Vec<ListedModel>>(&bytes)
            && discovery::validate(&models).is_ok()
        {
            return Some(models);
        }
        // Pre-discovery releases wrote only ids. Reading that file never
        // rewrites it, so installing a newer bundle preserves original data.
        let bytes = read_model_file(&dir.join("models.json")).ok()?;
        let ids: Vec<String> = serde_json::from_slice(&bytes).ok()?;
        let ids = discovery::validate_ids(&ids).ok()?;
        Some(
            ids.into_iter()
                .map(|id| ListedModel {
                    id,
                    name: None,
                    context_limit: None,
                    output_limit: None,
                })
                .collect(),
        )
    }

    pub(crate) fn read_manual_models(&self, credential_id: &str) -> Vec<String> {
        let read = || {
            let dir = self.checked_model_directory(credential_id).ok()?;
            let bytes = read_model_file(&dir.join("manual-models.json")).ok()?;
            let ids: Vec<String> = serde_json::from_slice(&bytes).ok()?;
            discovery::validate_ids(&ids).ok()
        };
        read().unwrap_or_default()
    }

    fn checked_model_directory(&self, credential_id: &str) -> io::Result<PathBuf> {
        if uuid::Uuid::parse_str(credential_id).is_err() {
            return Err(io::Error::other("Invalid model-cache connection id."));
        }
        self.check_layout()?;
        let dir = self.login_dir(credential_id);
        if let Ok(entry) = dir.symlink_metadata()
            && !entry.is_dir()
        {
            return Err(io::Error::other("Model cache must be a real directory."));
        }
        Ok(dir)
    }

    pub(crate) fn cache_discovery(
        &self,
        credential_id: &str,
        models: &[ListedModel],
    ) -> io::Result<()> {
        discovery::validate(models).map_err(io::Error::other)?;
        let body = serde_json::to_vec(models).map_err(io::Error::other)?;
        self.write_models_file(credential_id, "discovery.json", &body)
    }

    pub(crate) fn set_manual_models(&self, credential_id: &str, ids: &[String]) -> io::Result<()> {
        let ids = discovery::validate_ids(ids).map_err(io::Error::other)?;
        let body = serde_json::to_vec(&ids).map_err(io::Error::other)?;
        self.write_models_file(credential_id, "manual-models.json", &body)
    }

    fn write_models_file(
        &self,
        credential_id: &str,
        filename: &str,
        body: &[u8],
    ) -> io::Result<()> {
        let _one_writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        if body.len() > discovery::MAX_BYTES {
            return Err(io::Error::other("Model cache is too large."));
        }
        if self.find(credential_id)?.revoked {
            return Err(io::Error::other(
                "This connection was revoked during model discovery.",
            ));
        }
        let dir = self.checked_model_directory(credential_id)?;
        make_private_directory(&dir)?;
        let dest = dir.join(filename);
        if let Ok(entry) = dest.symlink_metadata()
            && !entry.is_file()
        {
            return Err(io::Error::other("Model cache must be a regular file."));
        }
        let temporary = dir.join(format!(".models-{}.tmp", uuid::Uuid::new_v4()));
        persist_renamed(&temporary, &dest, &dir, body)
    }

    pub(crate) fn connect_local(&self, base_url: &str, ids: &[String]) -> io::Result<Credential> {
        let (id, dir) = self.begin_login("ollama")?;
        let now = now_ms();
        let credential = Credential {
            id: id.clone(),
            provider_id: "ollama".into(),
            credential_kind: CredentialKind::Local,
            base_url: Some(base_url.into()),
            custom: None,
            label: "Ollama Local".into(),
            revoked: false,
            created_at: now,
            updated_at: now,
        };
        let result = write_account_models(&dir, ids).and_then(|()| {
            self.log
                .append(&StreamId::Room, &event(&credential))
                .map(|_| ())
        });
        if let Err(error) = result {
            let _ = self.abandon_login(&id);
            return Err(error);
        }
        Ok(credential)
    }

    /// Returns the rmcp credential store bound to one configured server URL.
    /// The store is intentionally scoped by both id and URL: changing the URL
    /// in settings cannot cause a token minted for the old origin to be sent
    /// to the new one.
    pub(crate) fn mcp_credential_store(
        self: &Arc<Self>,
        server_id: &str,
        server_url: &str,
    ) -> McpCredentialStore {
        let refresh_guard = self.mcp_refresh_lock(server_id);
        McpCredentialStore {
            vault: self.clone(),
            server_id: server_id.to_string(),
            server_url: server_url.to_string(),
            refresh_guard,
        }
    }

    /// Serializes the complete token read/refresh/save operation for one
    /// configured server. rmcp's refresh guard protects an individual
    /// `refresh_token` call; this outer lock also prevents two independent
    /// managers from both deciding that the same expired token needs refresh.
    pub(crate) fn mcp_token_lock(&self, server_id: &str) -> Arc<AsyncMutex<()>> {
        let guard_key = mcp_path_component(server_id);
        {
            let mut guards = self
                .mcp_token_locks
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            guards
                .entry(guard_key)
                .or_insert_with(|| Arc::new(AsyncMutex::new(())))
                .clone()
        }
    }

    fn mcp_refresh_lock(&self, server_id: &str) -> Arc<AsyncMutex<()>> {
        let guard_key = mcp_path_component(server_id);
        let mut guards = self
            .mcp_refresh_guards
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        guards
            .entry(guard_key)
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    /// Reads the saved client registration for a server. Registration metadata
    /// is protected by the same boundary as its tokens and is never returned
    /// through the normal settings or wire snapshots.
    pub(crate) fn mcp_oauth_registration(
        &self,
        server_id: &str,
        server_url: &str,
    ) -> io::Result<Option<McpOAuthRegistration>> {
        Ok(self
            .read_mcp_record(server_id, server_url)?
            .and_then(|record| record.registration))
    }

    /// Saves registration metadata before the browser flow starts, so a
    /// denied consent can be retried without registering a second client.
    pub(crate) fn save_mcp_oauth_registration(
        &self,
        server_id: &str,
        server_url: &str,
        registration: McpOAuthRegistration,
    ) -> io::Result<()> {
        self.update_mcp_record(server_id, server_url, |record| {
            record.registration = Some(registration);
        })
    }

    /// The token a bearer or header server sends, if one was saved for this
    /// URL. A token saved for another URL is not this server's token.
    pub(crate) fn mcp_secret(
        &self,
        server_id: &str,
        server_url: &str,
    ) -> io::Result<Option<String>> {
        Ok(self
            .read_mcp_record_any_url(server_id)?
            .filter(|record| record.server_url == server_url)
            .and_then(|record| record.secret))
    }

    /// Saves the token for a URL. A record bound to a different URL is
    /// replaced whole: a token minted for the old origin, and any OAuth
    /// material with it, must not follow the server to a new one.
    pub(crate) fn set_mcp_secret(
        &self,
        server_id: &str,
        server_url: &str,
        secret: &str,
    ) -> io::Result<()> {
        let _one_writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        let mut record = self
            .read_mcp_record_any_url(server_id)?
            .filter(|record| record.server_url == server_url)
            .unwrap_or_else(|| McpRecord {
                server_url: server_url.to_string(),
                registration: None,
                credentials: None,
                secret: None,
            });
        record.secret = Some(secret.to_string());
        self.write_mcp_record(server_id, &record)
    }

    /// Deletes all OAuth state for a server, including a saved client secret
    /// and refresh token, and a saved bearer or header token. Sign-out uses
    /// this after invalidating live leases.
    pub(crate) fn clear_mcp_oauth(&self, server_id: &str) -> io::Result<()> {
        let _one_writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        self.check_layout()?;
        self.files.file(self.mcp_path(server_id)).delete()
    }

    fn directory(&self) -> PathBuf {
        self.root.join("vault")
    }

    fn secrets_path(&self) -> PathBuf {
        self.directory().join("secrets.json")
    }

    fn logins_dir(&self) -> PathBuf {
        self.directory().join("logins")
    }

    fn mcp_dir(&self) -> PathBuf {
        self.directory().join("mcp")
    }

    fn mcp_path(&self, server_id: &str) -> PathBuf {
        self.mcp_dir()
            .join(format!("{}.json", mcp_path_component(server_id)))
    }

    fn login_dir(&self, id: &str) -> PathBuf {
        self.logins_dir().join(id)
    }

    fn find(&self, id: &str) -> io::Result<Credential> {
        self.list()
            .into_iter()
            .find(|credential| credential.id == id)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("there is no credential {id}"),
                )
            })
    }

    /// A symlink where the vault should be is a secret written somewhere
    /// nobody chose, so anything that is not the plain directory and plain
    /// file this writes is refused rather than followed.
    fn check_layout(&self) -> io::Result<()> {
        let directory = self.directory();
        #[cfg(windows)]
        if directory.exists() {
            check_vault_tree(&directory)?;
        }
        if let Ok(entry) = directory.symlink_metadata()
            && !entry.is_dir()
        {
            return Err(io::Error::other(format!(
                "{} must be a real directory owned by this user",
                directory.display()
            )));
        }
        let secrets = self.secrets_path();
        if let Ok(entry) = secrets.symlink_metadata()
            && !entry.is_file()
        {
            return Err(io::Error::other(format!(
                "{} must be a regular owner-only file",
                secrets.display()
            )));
        }
        let logins = self.logins_dir();
        if let Ok(entry) = logins.symlink_metadata()
            && !entry.is_dir()
        {
            return Err(io::Error::other(format!(
                "{} must be a real directory owned by this user",
                logins.display()
            )));
        }
        let mcp = self.mcp_dir();
        if let Ok(entry) = mcp.symlink_metadata()
            && !entry.is_dir()
        {
            return Err(io::Error::other(format!(
                "{} must be a real directory owned by this user",
                mcp.display()
            )));
        }
        let shared = self.shared_dir();
        if let Ok(entry) = shared.symlink_metadata()
            && !entry.is_dir()
        {
            return Err(io::Error::other(format!(
                "{} must be a real directory owned by this user",
                shared.display()
            )));
        }
        Ok(())
    }

    /// The secrets on disk. A vault that is not there yet holds nothing; one
    /// that cannot be parsed is an error, because overwriting it would throw
    /// away keys this cannot see.
    fn read_secrets(&self) -> io::Result<BTreeMap<String, String>> {
        let path = self.secrets_path();
        let Some(bytes) = self.files.file(path).read()? else {
            return Ok(BTreeMap::new());
        };
        serde_json::from_slice(&bytes).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "The provider credential record is unreadable; Hotline will not overwrite it.",
            )
        })
    }

    fn write_secrets(&self, secrets: &BTreeMap<String, String>) -> io::Result<()> {
        self.check_layout()?;
        make_private_directory(&self.directory())?;
        let _ = fs::remove_file(
            self.directory()
                .join(format!("secrets.json.{}.tmp", std::process::id())),
        );
        self.files
            .file(self.secrets_path())
            .write(&serde_json::to_vec(secrets).map_err(io::Error::other)?)
    }

    fn read_mcp_record(&self, server_id: &str, server_url: &str) -> io::Result<Option<McpRecord>> {
        let Some(record) = self.read_mcp_record_any_url(server_id)? else {
            return Ok(None);
        };
        if record.server_url != server_url {
            return Err(io::Error::other(
                "saved MCP OAuth credentials belong to a different server URL; sign in again",
            ));
        }
        Ok(Some(record))
    }

    fn read_mcp_record_any_url(&self, server_id: &str) -> io::Result<Option<McpRecord>> {
        self.check_layout()?;
        let path = self.mcp_path(server_id);
        let Some(bytes) = self.files.file(path).read()? else {
            return Ok(None);
        };
        let record: McpRecord = serde_json::from_slice(&bytes).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "The MCP credential record is unreadable.",
            )
        })?;
        Ok(Some(record))
    }

    fn update_mcp_record(
        &self,
        server_id: &str,
        server_url: &str,
        update: impl FnOnce(&mut McpRecord),
    ) -> io::Result<()> {
        let _one_writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        let mut record = self
            .read_mcp_record(server_id, server_url)?
            .unwrap_or_else(|| McpRecord {
                server_url: server_url.to_string(),
                registration: None,
                credentials: None,
                secret: None,
            });
        update(&mut record);
        self.write_mcp_record(server_id, &record)
    }

    fn write_mcp_record(&self, server_id: &str, record: &McpRecord) -> io::Result<()> {
        self.check_layout()?;
        let directory = self.mcp_dir();
        make_private_directory(&directory)?;
        self.files
            .file(self.mcp_path(server_id))
            .write(&serde_json::to_vec(record).map_err(io::Error::other)?)
    }

    pub(crate) fn login_tokens(&self, id: &str) -> CredentialFile {
        self.files.file(self.login_dir(id).join("auth.json"))
    }
}

/// rmcp's OAuth store adapter. It carries only a vault handle and server
/// binding; the token itself is loaded for the duration of one SDK operation
/// and is never represented in a Hotline event, setting or descriptor.
#[derive(Clone)]
pub(crate) struct McpCredentialStore {
    vault: Arc<Vault>,
    server_id: String,
    server_url: String,
    refresh_guard: Arc<AsyncMutex<()>>,
}

#[async_trait]
impl CredentialStore for McpCredentialStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, rmcp::transport::auth::AuthError> {
        self.vault
            .read_mcp_record(&self.server_id, &self.server_url)
            .map(|record| record.and_then(|record| record.credentials))
            .map_err(|error| {
                rmcp::transport::auth::AuthError::CredentialStoreError(error.to_string())
            })
    }

    async fn save(
        &self,
        credentials: StoredCredentials,
    ) -> Result<(), rmcp::transport::auth::AuthError> {
        self.vault
            .update_mcp_record(&self.server_id, &self.server_url, |record| {
                record.credentials = Some(credentials);
            })
            .map_err(|error| {
                rmcp::transport::auth::AuthError::CredentialStoreError(error.to_string())
            })
    }

    async fn clear(&self) -> Result<(), rmcp::transport::auth::AuthError> {
        self.vault
            .clear_mcp_oauth(&self.server_id)
            .map_err(|error| {
                rmcp::transport::auth::AuthError::CredentialStoreError(error.to_string())
            })
    }

    async fn acquire_refresh_guard(
        &self,
    ) -> Result<Option<CredentialRefreshGuard>, rmcp::transport::auth::AuthError> {
        Ok(Some(CredentialRefreshGuard::new(
            self.refresh_guard.clone().lock_owned().await,
        )))
    }
}

/// Keep user supplied ids inside one vault directory while retaining enough
/// of the id to make a manually inspected vault understandable. The stable
/// FNV suffix prevents traversal and collisions across process restarts.
fn mcp_path_component(id: &str) -> String {
    let mut readable = String::with_capacity(id.len().min(48));
    for character in id.chars().take(48) {
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
            readable.push(character);
        } else {
            readable.push('_');
        }
    }
    if readable.is_empty() {
        readable.push_str("server");
    }
    let mut hash = 0xcbf29ce484222325u64;
    for byte in id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{readable}-{hash:016x}")
}

#[cfg(windows)]
fn check_vault_tree(path: &Path) -> io::Result<()> {
    crate::credentials::check_private_path(path)?;
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    if metadata.is_dir() {
        crate::credentials::windows::private_directory(path)?;
    } else {
        crate::credentials::windows::private_file(path)?;
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            check_vault_tree(&entry?.path())?;
        }
    }
    Ok(())
}

/// The room event contains the credential metadata, never its secret.
fn event(credential: &Credential) -> Value {
    crate::room::room_event(
        "credential",
        serde_json::to_value(credential)
            .expect("a credential is a struct of strings, bools and numbers"),
    )
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CustomKey {
    base_url: String,
    key: String,
}

fn read_custom_key(file: &CredentialFile, base_url: &str) -> io::Result<String> {
    let key: CustomKey = serde_json::from_slice(&file.read()?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "The custom connection key is missing.",
        )
    })?)
    .map_err(|_| io::Error::other("The custom connection's key is unreadable."))?;
    if key.base_url != base_url || key.key.is_empty() {
        return Err(io::Error::other(
            "The saved key does not belong to this endpoint. Enter it again.",
        ));
    }
    Ok(key.key)
}

/// Bytes onto a temporary, fsync, rename, fsync the directory. A failure
/// removes the temporary so a later write is not stepping over a half-written
/// file this process still names.
fn persist_renamed(
    temporary: &Path,
    dest: &Path,
    directory: &Path,
    contents: &[u8],
) -> io::Result<()> {
    let result = (|| {
        let mut file = create_private_file(temporary)?;
        file.write_all(contents)?;
        // The rename is only atomic over bytes that reached the disk.
        file.sync_all()?;
        drop(file);
        fs::rename(temporary, dest)?;
        // The rename is a directory entry. Fsync of the file is not fsync of
        // that entry; without this, a crash can lose the key we just wrote.
        sync_directory(directory)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        open_directory(path)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Creates the vault directory private to this user, and says so again on a
/// directory that was already there — a boundary is only a boundary if it is
/// re-established every time.
#[cfg(unix)]
fn make_private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    // `mkdir` masks the mode it is given by the process umask, which can only
    // take permissions away, so the directory is never wider than 0700 — not
    // even for the instant between creating it and setting the mode again.
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    // chmod on the path follows a symlink. The fd does not, so a vault/
    // swapped for a link cannot have its target's mode changed through us.
    open_directory(path)?.set_permissions(fs::Permissions::from_mode(0o700))
}

/// Opens a directory without following a symlink at the last component.
#[cfg(unix)]
fn open_directory(path: &Path) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
}

/// Windows files inherit the private DACL established at directory creation.
#[cfg(windows)]
fn make_private_directory(path: &Path) -> io::Result<()> {
    crate::credentials::windows::private_directory(path)
}

/// Creates a file only this user can read. On Windows the file's privacy is
/// the directory's ACL, which `make_private_directory` refuses to establish,
/// so nothing reaches here on that platform.
fn create_private_file(path: &Path) -> io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// A 0600 file whose contents Rig will overwrite in place, so the mode it
/// is created with is the mode it keeps. JSON records start as `{}` because
/// Rig treats an empty file as a parse error, not as absent.
pub(crate) fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut file = create_private_file(path)?;
    file.write_all(contents)?;
    file.sync_all()?;
    Ok(())
}

fn read_model_file(path: &Path) -> io::Result<Vec<u8>> {
    if !path.symlink_metadata()?.is_file() {
        return Err(io::Error::other("Model cache must be a regular file."));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > discovery::MAX_BYTES as u64 {
        return Err(io::Error::other(
            "Model cache is too large or is not a regular file.",
        ));
    }
    let mut bytes = Vec::new();
    file.take(discovery::MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > discovery::MAX_BYTES {
        return Err(io::Error::other("Model cache is too large."));
    }
    Ok(bytes)
}

/// The account's model ids as a JSON array of bare catalogue ids, 0600
/// beside the login. Replaces a list already there so Refresh can rewrite
/// without leaving the previous bytes behind a `create_new` refusal.
pub(crate) fn write_account_models(token_dir: &Path, ids: &[String]) -> io::Result<()> {
    let ids = discovery::validate_ids(ids).map_err(io::Error::other)?;
    let path = token_dir.join("models.json");
    let mut body = serde_json::to_vec(&ids).map_err(io::Error::other)?;
    body.push(b'\n');
    let temporary = token_dir.join(format!(".models-{}.tmp", uuid::Uuid::new_v4()));
    persist_renamed(&temporary, &path, token_dir, &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(id: &str) -> ListedModel {
        ListedModel {
            id: id.into(),
            name: None,
            context_limit: None,
            output_limit: None,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("hotline-core-vault-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn vault(name: &str) -> Vault {
        let root = scratch(name);
        Vault::open(&root, Log::open(&root)).unwrap()
    }

    fn ids(vault: &Vault) -> Vec<String> {
        vault
            .list()
            .into_iter()
            .map(|credential| credential.id)
            .collect()
    }

    fn room(vault: &Vault) -> String {
        fs::read_to_string(vault.root.join("room.jsonl")).unwrap()
    }

    fn secrets(vault: &Vault) -> String {
        fs::read_to_string(vault.secrets_path()).unwrap()
    }

    #[test]
    fn model_discovery_belongs_to_the_connection_and_cannot_restore_a_deleted_one() {
        let vault = vault("provider-model-cache");
        let first = vault.create("ollama-cloud", "First", "first-key").unwrap();
        let second = vault
            .create("ollama-cloud", "Second", "second-key")
            .unwrap();
        vault
            .cache_discovery(&first.id, &[listed("first-model")])
            .unwrap();
        vault
            .cache_discovery(&second.id, &[listed("second-model")])
            .unwrap();
        assert_eq!(vault.account_models()["ollama-cloud"], ["first-model"]);
        vault.delete(&first.id).unwrap();
        assert_eq!(vault.account_models()["ollama-cloud"], ["second-model"]);
        assert!(
            vault
                .cache_discovery(&first.id, &[listed("stale-result")])
                .is_err()
        );
        assert!(!vault.login_dir(&first.id).exists());
        vault.cache_discovery(&second.id, &[]).unwrap();
        assert!(vault.account_models()["ollama-cloud"].is_empty());
        vault.revoke(&second.id).unwrap();
        assert!(vault.account_models().is_empty());
        assert!(
            vault
                .cache_discovery(&second.id, &[listed("stale-result")])
                .is_err()
        );
    }

    #[test]
    fn invalid_discovery_caches_keep_legacy_bytes_and_refuse_unsafe_paths() {
        let vault = vault("untrusted-model-cache");
        let credential = vault.create("anthropic", "Test", "secret").unwrap();
        let dir = vault.checked_model_directory(&credential.id).unwrap();
        make_private_directory(&dir).unwrap();
        write_account_models(&dir, &["legacy-coder".into()]).unwrap();
        let original = fs::read(dir.join("models.json")).unwrap();
        for content in [
            br#"[{"id":"../escape"}]"#.to_vec(),
            br#"[{"id":"coder","output_limit":0}]"#.to_vec(),
            br#"[{"id":"coder","context_limit":100000001}]"#.to_vec(),
            br#"[{"id":"coder","endpoint":"https://other.example"}]"#.to_vec(),
            vec![b'x'; discovery::MAX_BYTES + 1],
        ] {
            fs::write(dir.join("discovery.json"), &content).unwrap();
            assert_eq!(vault.account_models()["anthropic"], ["legacy-coder"]);
            assert_eq!(fs::read(dir.join("discovery.json")).unwrap(), content);
            assert_eq!(fs::read(dir.join("models.json")).unwrap(), original);
        }
        vault
            .cache_discovery(&credential.id, &[listed("new-coder")])
            .unwrap();
        let good = fs::read(dir.join("discovery.json")).unwrap();
        assert!(
            vault
                .cache_discovery(&credential.id, &[listed("bad\nmodel")])
                .is_err()
        );
        assert_eq!(fs::read(dir.join("discovery.json")).unwrap(), good);
        assert!(vault.read_discovery("../escape").is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let outside = vault.root.join("untouched.json");
            fs::write(&outside, b"[\"outside-model\"]").unwrap();
            fs::remove_file(dir.join("discovery.json")).unwrap();
            symlink(&outside, dir.join("discovery.json")).unwrap();
            assert_eq!(vault.account_models()["anthropic"], ["legacy-coder"]);
            assert!(
                vault
                    .cache_discovery(&credential.id, &[listed("attempt")])
                    .is_err()
            );
            symlink(&outside, dir.join("manual-models.json")).unwrap();
            assert!(vault.read_manual_models(&credential.id).is_empty());
            assert!(
                vault
                    .set_manual_models(&credential.id, &["manual".into()])
                    .is_err()
            );
            assert_eq!(fs::read(&outside).unwrap(), b"[\"outside-model\"]");
        }
    }

    #[test]
    fn manual_models_follow_the_active_connection_and_copilot_cannot_expand_its_account() {
        let vault = vault("manual-model-account");
        let first = vault.create("anthropic", "First", "first-key").unwrap();
        let second = vault.create("anthropic", "Second", "second-key").unwrap();
        vault
            .cache_discovery(&first.id, &[listed("first-model")])
            .unwrap();
        vault
            .set_manual_models(&first.id, &["manual-first".into()])
            .unwrap();
        vault
            .cache_discovery(&second.id, &[listed("second-model")])
            .unwrap();
        assert_eq!(
            vault.account_models()["anthropic"],
            ["first-model", "manual-first"]
        );
        vault.delete(&first.id).unwrap();
        assert_eq!(vault.account_models()["anthropic"], ["second-model"]);
        assert!(
            vault
                .set_manual_models(&first.id, &["stale".into()])
                .is_err()
        );
        let (id, _) = vault.begin_login("github-copilot").unwrap();
        vault
            .finish_login(&id, "github-copilot", "Copilot")
            .unwrap();
        vault
            .cache_discovery(&id, &[listed("current-account")])
            .unwrap();
        vault
            .set_manual_models(&id, &["previous-account".into()])
            .unwrap();
        assert_eq!(
            vault.account_models()["github-copilot"],
            ["current-account"]
        );
    }

    #[test]
    fn a_pasted_token_is_bound_to_its_url_and_replaced_whole_on_a_new_one() {
        let vault = vault("mcp-secret");
        assert_eq!(vault.mcp_secret("s1", "https://a.test/mcp").unwrap(), None);
        vault
            .set_mcp_secret("s1", "https://a.test/mcp", "tok-a")
            .unwrap();
        assert_eq!(
            vault
                .mcp_secret("s1", "https://a.test/mcp")
                .unwrap()
                .as_deref(),
            Some("tok-a")
        );
        assert_eq!(vault.mcp_secret("s1", "https://b.test/mcp").unwrap(), None);
        vault
            .set_mcp_secret("s1", "https://b.test/mcp", "tok-b")
            .unwrap();
        assert_eq!(vault.mcp_secret("s1", "https://a.test/mcp").unwrap(), None);
        assert_eq!(
            vault
                .mcp_secret("s1", "https://b.test/mcp")
                .unwrap()
                .as_deref(),
            Some("tok-b")
        );
        vault.clear_mcp_oauth("s1").unwrap();
        assert_eq!(vault.mcp_secret("s1", "https://b.test/mcp").unwrap(), None);
    }

    /// A credential is a read of the whole map, one entry added, and the map
    /// written back. Two of those at once used to keep whichever finished
    /// last: the other's row was on the room stream with no secret behind it,
    /// which reads to `provider_auth` as a key the user never sees again.
    #[test]
    fn every_credential_written_at_once_keeps_its_secret() {
        let vault = vault("concurrent-create");
        std::thread::scope(|scope| {
            for writer in 0..8 {
                let vault = &vault;
                scope.spawn(move || {
                    vault
                        .create(
                            &format!("provider-{writer}"),
                            "personal",
                            &format!("key-{writer}"),
                        )
                        .unwrap();
                });
            }
        });

        let keys = vault.provider_auth();
        assert_eq!(keys.len(), 8, "{keys:?}");
        for writer in 0..8 {
            assert_eq!(
                keys.get(&format!("provider-{writer}")),
                Some(&ProviderAuth::ApiKey(format!("key-{writer}")))
            );
        }
        assert_eq!(ids(&vault).len(), 8);
    }

    #[cfg(unix)]
    #[test]
    fn the_vault_is_a_private_directory_holding_a_private_file() {
        use std::os::unix::fs::PermissionsExt;

        let vault = vault("modes");
        vault.create("anthropic", "work", "sk-ant-001").unwrap();
        let mode = |path: PathBuf| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(vault.directory()), 0o700);
        assert_eq!(mode(vault.secrets_path()), 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn mcp_oauth_records_are_private_and_bound_to_the_server_url() {
        use std::os::unix::fs::PermissionsExt;

        let vault = vault("mcp-oauth");
        vault
            .save_mcp_oauth_registration(
                "server-id",
                "https://mcp.example.test/mcp",
                McpOAuthRegistration {
                    client_id: "client-id".to_string(),
                    client_secret: Some("client-secret".to_string()),
                    redirect_uri: "http://127.0.0.1:4321/callback".to_string(),
                    issuer: Some("https://auth.example.test".to_string()),
                    resource: "https://mcp.example.test/mcp".to_string(),
                    scopes: vec!["mcp".to_string()],
                },
            )
            .unwrap();
        let mode = |path: PathBuf| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(vault.mcp_dir()), 0o700);
        assert_eq!(mode(vault.mcp_path("server-id")), 0o600);
        assert!(
            vault
                .mcp_oauth_registration("server-id", "https://mcp.example.test/mcp")
                .unwrap()
                .is_some()
        );
        assert!(
            vault
                .mcp_oauth_registration("server-id", "https://mcp.example.test/other")
                .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_vault_directory_that_is_a_symlink_is_refused_rather_than_followed() {
        let root = scratch("symlink");
        let elsewhere = root.join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, root.join("vault")).unwrap();
        assert!(Vault::open(&root, Log::open(&root)).is_err());
    }

    #[test]
    fn a_secret_never_reaches_the_room_stream() {
        let vault = vault("stream");
        let credential = vault.create("anthropic", "work", "sk-ant-secret").unwrap();
        let stream = room(&vault);
        assert!(!stream.contains("sk-ant-secret"), "{stream}");
        assert!(stream.contains(&credential.id), "{stream}");
        assert!(!secrets(&vault).contains("sk-ant-secret"));
        assert_eq!(
            vault.read_secrets().unwrap().values().next().unwrap(),
            "sk-ant-secret"
        );
    }

    #[test]
    fn a_credential_event_is_the_shape_the_room_stream_promises() {
        let vault = vault("shape");
        let credential = vault.create("openai", "personal", "sk-oai").unwrap();
        let written: Value = serde_json::from_str(room(&vault).trim()).unwrap();
        assert_eq!(
            written,
            json!({
                "kind": "credential",
                "id": credential.id,
                "providerId": "openai",
                "credentialKind": "api_key",
                "label": "personal",
                "revoked": false,
                "createdAt": credential.created_at,
                "updatedAt": credential.updated_at,
            })
        );
    }

    #[test]
    fn revoking_hides_a_key_and_deleting_takes_both_halves() {
        let vault = vault("revoke");
        let credential = vault.create("anthropic", "work", "sk-ant-001").unwrap();
        assert_eq!(
            vault.provider_auth().get("anthropic"),
            Some(&ProviderAuth::ApiKey("sk-ant-001".to_string()))
        );

        vault.revoke(&credential.id).unwrap();
        assert!(vault.provider_auth().is_empty());
        // The row stays with its label, because the user still has a key to
        // rotate at the provider, and the secret stays until it is deleted.
        let listed = vault.list();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].revoked);
        assert!(!secrets(&vault).contains("sk-ant-001"));
        assert!(
            vault
                .read_secrets()
                .unwrap()
                .values()
                .any(|key| key == "sk-ant-001")
        );

        vault.delete(&credential.id).unwrap();
        assert!(vault.list().is_empty());
        assert!(!secrets(&vault).contains("sk-ant-001"));
        // Both halves are gone, and asking again says so.
        assert!(vault.revoke(&credential.id).is_err());
        assert!(vault.delete(&credential.id).is_err());
    }

    #[test]
    fn one_key_per_provider_and_the_first_one_created_wins() {
        let vault = vault("keys");
        let first = vault.create("anthropic", "work", "sk-ant-first").unwrap();
        vault.create("anthropic", "spare", "sk-ant-second").unwrap();
        vault.create("openai", "personal", "sk-oai").unwrap();
        assert_eq!(
            vault.provider_auth(),
            HashMap::from([
                (
                    "anthropic".to_string(),
                    ProviderAuth::ApiKey("sk-ant-first".to_string())
                ),
                (
                    "openai".to_string(),
                    ProviderAuth::ApiKey("sk-oai".to_string())
                ),
            ])
        );

        vault.revoke(&first.id).unwrap();
        assert_eq!(
            vault.provider_auth().get("anthropic"),
            Some(&ProviderAuth::ApiKey("sk-ant-second".to_string()))
        );
    }

    #[test]
    fn a_row_whose_secret_is_missing_is_not_a_key() {
        let vault = vault("orphan");
        // What a `create` torn between its two writes leaves behind, and what
        // a stream restored without its vault looks like: metadata with
        // nothing on the disk under it.
        let torn = Credential {
            id: "torn".to_string(),
            provider_id: "anthropic".to_string(),
            credential_kind: CredentialKind::ApiKey,
            base_url: None,
            custom: None,
            label: "torn".to_string(),
            revoked: false,
            created_at: 1,
            updated_at: 1,
        };
        vault.log.append(&StreamId::Room, &event(&torn)).unwrap();

        assert_eq!(ids(&vault), vec!["torn".to_string()]);
        assert!(vault.provider_auth().is_empty());
    }

    #[test]
    fn the_list_is_creation_order_and_a_tombstone_is_the_last_word() {
        let vault = vault("list");
        let first = vault.create("anthropic", "work", "a").unwrap();
        let second = vault.create("openai", "personal", "b").unwrap();
        let third = vault.create("openrouter", "spare", "c").unwrap();

        // A second line for an id supersedes the first and keeps its place.
        vault.revoke(&first.id).unwrap();
        assert_eq!(
            ids(&vault),
            vec![first.id.clone(), second.id.clone(), third.id.clone()]
        );

        vault.delete(&second.id).unwrap();
        assert_eq!(ids(&vault), vec![first.id, third.id]);
    }

    #[test]
    fn a_failed_write_does_not_leave_its_temporary_behind() {
        let root = scratch("failed-write");
        let directory = root.join("vault");
        fs::create_dir_all(&directory).unwrap();
        let dest = directory.join("secrets.json");
        // A directory where the file should land makes the rename fail after
        // the temporary has been created, which is the error path a test can
        // see: the temporary must not still be there.
        fs::create_dir(&dest).unwrap();
        let temporary = directory.join(format!("secrets.json.{}.tmp", std::process::id()));
        let err = persist_renamed(&temporary, &dest, &directory, b"{}\n");
        assert!(err.is_err(), "rename onto a directory should fail");
        assert!(
            !temporary.exists(),
            "the temporary was left behind after a failed write"
        );
    }

    #[cfg(unix)]
    #[test]
    fn setting_the_directory_private_does_not_follow_a_symlink() {
        use std::os::unix::fs::PermissionsExt;

        let root = scratch("chmod-symlink");
        let target = root.join("elsewhere");
        fs::create_dir_all(&target).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
        let vault_dir = root.join("vault");
        std::os::unix::fs::symlink(&target, &vault_dir).unwrap();

        assert!(make_private_directory(&vault_dir).is_err());
        let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o755,
            "chmod followed the symlink and changed the target"
        );
    }

    #[test]
    fn a_credential_line_starts_with_kind() {
        let vault = vault("kind-leads");
        vault.create("openai", "personal", "sk-oai").unwrap();
        let line = room(&vault);
        let line = line.trim();
        assert!(
            line.starts_with("{\"kind\":\"credential\""),
            "kind was not the leading key: {line}"
        );
    }

    #[test]
    fn a_leftover_temporary_file_is_ignored_because_the_rename_is_the_write() {
        let vault = vault("torn-write");
        vault.create("anthropic", "work", "sk-ant-001").unwrap();

        // A write interrupted before its rename leaves a temporary behind:
        // this process's own name, or one from a process that is gone. The
        // vault is what the rename made, so neither is ever read, and the next
        // write steps over its own.
        let mine = vault
            .directory()
            .join(format!("secrets.json.{}.tmp", std::process::id()));
        fs::write(&mine, "{ half a wri").unwrap();
        let stranger = vault.directory().join("secrets.json.999999.tmp");
        fs::write(&stranger, "{\"ghost\":\"sk-ghost\"}").unwrap();

        vault.create("openai", "personal", "sk-oai").unwrap();

        let keys = vault.provider_auth();
        assert_eq!(
            keys.get("anthropic"),
            Some(&ProviderAuth::ApiKey("sk-ant-001".to_string()))
        );
        assert_eq!(
            keys.get("openai"),
            Some(&ProviderAuth::ApiKey("sk-oai".to_string()))
        );
        assert!(!secrets(&vault).contains("sk-ghost"));
        assert!(!mine.exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_login_is_a_private_directory_that_lands_only_once_it_succeeds() {
        use std::os::unix::fs::PermissionsExt;

        let vault = vault("login-roundtrip");
        let (id, dir) = vault.begin_login("openai-codex").unwrap();
        let mode = |path: PathBuf| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(dir.clone()), 0o700);
        assert_eq!(mode(dir.join("auth.json")), 0o600);
        assert_eq!(fs::read_to_string(dir.join("auth.json")).unwrap(), "{}");
        assert!(vault.list().is_empty(), "no event until the login finishes");
        assert!(vault.provider_auth().is_empty());

        let credential = vault.finish_login(&id, "openai-codex", "ChatGPT").unwrap();
        assert_eq!(credential.id, id);
        assert_eq!(credential.credential_kind, CredentialKind::Oauth);
        assert_eq!(credential.label, "ChatGPT");
        match vault.provider_auth().get("openai-codex") {
            Some(ProviderAuth::Login { token_dir }) => assert_eq!(token_dir, &dir),
            other => panic!("finished login should be auth: {other:?}"),
        }
        assert!(!room(&vault).contains("access_token"), "{:?}", room(&vault));
    }

    #[cfg(unix)]
    #[test]
    fn a_copilot_login_precreates_the_files_rig_will_write() {
        use std::os::unix::fs::PermissionsExt;

        let vault = vault("login-copilot");
        let (_id, dir) = vault.begin_login("github-copilot").unwrap();
        let mode = |path: PathBuf| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(dir.clone()), 0o700);
        assert_eq!(mode(dir.join("api-key.json")), 0o600);
        assert_eq!(mode(dir.join("access-token")), 0o600);
        assert_eq!(fs::read_to_string(dir.join("api-key.json")).unwrap(), "{}");
        assert_eq!(fs::read_to_string(dir.join("access-token")).unwrap(), "");
    }

    #[test]
    fn abandoning_a_login_leaves_nothing() {
        let vault = vault("login-abandon");
        let (id, dir) = vault.begin_login("openai-codex").unwrap();
        vault.abandon_login(&id).unwrap();
        assert!(!dir.exists());
        assert!(vault.list().is_empty());
        assert!(vault.provider_auth().is_empty());
    }

    #[test]
    fn deleting_a_login_removes_its_directory() {
        let vault = vault("login-delete");
        let (id, dir) = vault.begin_login("openai-codex").unwrap();
        vault.finish_login(&id, "openai-codex", "ChatGPT").unwrap();
        assert!(dir.is_dir());
        vault.delete(&id).unwrap();
        assert!(!dir.exists());
        assert!(vault.list().is_empty());
        assert!(vault.provider_auth().is_empty());
    }

    #[test]
    fn a_revoked_login_is_not_auth() {
        let vault = vault("login-revoke");
        let (id, dir) = vault.begin_login("openai-codex").unwrap();
        vault.finish_login(&id, "openai-codex", "ChatGPT").unwrap();
        vault.revoke(&id).unwrap();
        assert!(dir.is_dir(), "revoke leaves the files, as for keys");
        assert!(vault.provider_auth().is_empty());
        let listed = vault.list();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].revoked);
    }

    fn copilot_login(vault: &Vault) -> (String, PathBuf) {
        let (id, dir) = vault.begin_login("github-copilot").unwrap();
        vault
            .finish_login(&id, "github-copilot", "GitHub Copilot")
            .unwrap();
        (id, dir)
    }

    #[test]
    fn account_models_reads_the_list_beside_a_login() {
        let vault = vault("account-models-present");
        let (_id, dir) = copilot_login(&vault);
        write_account_models(&dir, &["gpt-5.5".into(), "gpt-4.1".into()]).unwrap();
        write_account_models(&dir, &["gpt-5.5".into()]).unwrap();
        assert_eq!(
            vault.account_models().get("github-copilot"),
            Some(&vec!["gpt-5.5".to_string()])
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.join("models.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn account_models_is_none_when_the_file_is_absent() {
        let vault = vault("account-models-absent");
        assert_eq!(vault.account_models().get("github-copilot"), None);
        copilot_login(&vault);
        assert_eq!(vault.account_models().get("github-copilot"), None);
    }

    #[test]
    fn account_models_is_none_when_the_file_is_unparsable() {
        let vault = vault("account-models-junk");
        let (_id, dir) = copilot_login(&vault);
        fs::write(dir.join("models.json"), "not a list").unwrap();
        assert_eq!(vault.account_models().get("github-copilot"), None);
        fs::write(dir.join("models.json"), "[1, \"gpt-5.5\"]").unwrap();
        assert_eq!(vault.account_models().get("github-copilot"), None);
    }

    #[test]
    fn account_models_is_none_for_a_revoked_login() {
        let vault = vault("account-models-revoked");
        let (id, dir) = copilot_login(&vault);
        write_account_models(&dir, &["gpt-5.5".into()]).unwrap();
        vault.revoke(&id).unwrap();
        assert_eq!(vault.account_models().get("github-copilot"), None);
        assert!(dir.join("models.json").is_file(), "revoke leaves the files");
    }
}
