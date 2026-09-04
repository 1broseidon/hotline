//! The vault: provider secrets on this machine's disk, and the room's record
//! that they exist.
//!
//! A secret is never an event. `<root>/vault/secrets.json` is a JSON map from
//! credential id to secret — a `0600` file in a `0700` directory, written
//! through a temporary file, a rename, and an fsync of the directory so no
//! reader ever sees half of one and a crash cannot lose the rename — and a
//! login's tokens live in `<root>/vault/logins/<id>/`, the same modes, the
//! files Rig will write pre-created so a `std::fs::write` cannot leave them
//! world-readable. The room stream carries only the metadata: which provider,
//! what the user called it, whether it is revoked. That is why `list` and
//! `provider_auth` are two different questions. The room knows a credential
//! exists; only this disk knows what it is, which is what lets a stream be
//! read, copied or shipped without carrying a key along with it.
//!
//! The discipline is the previous Toad's `src/bun/store/credentials.ts`,
//! minus the fleet: nothing is replicated and nothing is sealed to another
//! desk, because there are no other desks here.

use crate::contract::{Credential, CredentialKind};
use crate::log::{Log, StreamId};
use crate::models::Client;
use crate::session::ProviderAuth;
use async_trait::async_trait;
use rmcp::transport::auth::{CredentialRefreshGuard, CredentialStore, StoredCredentials};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::{Mutex, PoisonError};
use tokio::sync::Mutex as AsyncMutex;

/// The client identity Toad received from an MCP authorization server. The
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

#[derive(Clone, Deserialize, Serialize)]
struct McpOAuthRecord {
    server_url: String,
    registration: Option<McpOAuthRegistration>,
    credentials: Option<StoredCredentials>,
}

/// The vault over one data root.
///
/// It holds the log rather than opening one of its own, because a credential
/// is a room event like any other and there is exactly one writer to the room
/// stream.
pub struct Vault {
    root: PathBuf,
    log: Log,
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
    /// Opens the vault over a data root, refusing one whose directory or file
    /// is not the plain directory and plain file it must be.
    ///
    /// Nothing is created here. Establishing the layout is a write-time
    /// obligation: a `0700` `vault/` on a machine that has never held a secret
    /// would be a private-looking folder saying something untrue about the
    /// machine.
    pub fn open(root: impl Into<PathBuf>, log: Log) -> io::Result<Vault> {
        let vault = Vault {
            root: root.into(),
            log,
            writer: Mutex::new(()),
            mcp_refresh_guards: Mutex::new(HashMap::new()),
            mcp_token_locks: Mutex::new(HashMap::new()),
        };
        vault.check_layout()?;
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
    /// still have to rotate at the provider.
    pub fn revoke(&self, id: &str) -> io::Result<()> {
        let mut credential = self.find(id)?;
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
        match credential.credential_kind {
            CredentialKind::Oauth => {
                let dir = self.login_dir(&credential.id);
                match fs::remove_dir_all(&dir) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
            CredentialKind::ApiKey => {
                let mut secrets = self.read_secrets()?;
                if secrets.remove(&credential.id).is_some() {
                    self.write_secrets(&secrets)?;
                }
            }
        }
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
        // A vault that cannot be read is no keys rather than a failure: the
        // place where refusing matters is the write, where a key could be lost.
        let secrets = self.read_secrets().unwrap_or_default();
        let mut auth = HashMap::new();
        for credential in self.list() {
            if credential.revoked {
                continue;
            }
            let value = match credential.credential_kind {
                CredentialKind::ApiKey => {
                    let Some(secret) = secrets.get(&credential.id) else {
                        continue;
                    };
                    ProviderAuth::ApiKey(secret.clone())
                }
                CredentialKind::Oauth => {
                    let dir = self.login_dir(&credential.id);
                    if !dir.is_dir() {
                        continue;
                    }
                    ProviderAuth::Login { token_dir: dir }
                }
            };
            auth.entry(credential.provider_id).or_insert(value);
        }
        auth
    }

    /// The model ids each held login can run, when they have been written
    /// beside it. A login with no list, an unreadable one, or a revoked
    /// login is absent from the map, which the picker treats as the whole
    /// catalogue — a missing list is not an empty one. Read from the login
    /// directory `ProviderAuth` already carries rather than a second field
    /// on the credential.
    pub fn account_models(&self) -> HashMap<String, Vec<String>> {
        self.provider_auth()
            .into_iter()
            .filter_map(|(provider_id, auth)| match auth {
                ProviderAuth::Login { token_dir } => {
                    let text = fs::read_to_string(token_dir.join("models.json")).ok()?;
                    let ids: Vec<String> = serde_json::from_str(&text).ok()?;
                    Some((provider_id, ids))
                }
                ProviderAuth::ApiKey(_) => None,
            })
            .collect()
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

    /// Deletes all OAuth state for a server, including a saved client secret
    /// and refresh token. Sign-out uses this after invalidating live leases.
    pub(crate) fn clear_mcp_oauth(&self, server_id: &str) -> io::Result<()> {
        let _one_writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        self.check_layout()?;
        let path = self.mcp_path(server_id);
        match fs::symlink_metadata(&path) {
            Ok(entry) if !entry.is_file() => Err(io::Error::other(format!(
                "{} is not a regular file",
                path.display()
            ))),
            Ok(_) => {
                fs::remove_file(&path)?;
                sync_directory(&self.mcp_dir())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
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
        Ok(())
    }

    /// The secrets on disk. A vault that is not there yet holds nothing; one
    /// that cannot be parsed is an error, because overwriting it would throw
    /// away keys this cannot see.
    fn read_secrets(&self) -> io::Result<BTreeMap<String, String>> {
        let path = self.secrets_path();
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(error) => return Err(error),
        };
        serde_json::from_str(&text).map_err(|error| {
            io::Error::other(format!(
                "{} is not the map of secrets Toad writes ({error}). Fix or remove it; Toad will not overwrite provider credentials it cannot read.",
                path.display()
            ))
        })
    }

    /// Atomic, owner-only persistence. No backup: a rotated key is not worth
    /// keeping.
    fn write_secrets(&self, secrets: &BTreeMap<String, String>) -> io::Result<()> {
        self.check_layout()?;
        let directory = self.directory();
        make_private_directory(&directory)?;
        // A crash can leave this process's own temporary behind. Removing the
        // directory entry first is safe even if something replaced it, because
        // `create_new` then refuses the race instead of following it.
        let temporary = directory.join(format!("secrets.json.{}.tmp", std::process::id()));
        let _ = fs::remove_file(&temporary);
        let text = serde_json::to_string_pretty(secrets).map_err(io::Error::other)?;
        persist_renamed(
            &temporary,
            &self.secrets_path(),
            &directory,
            format!("{text}\n").as_bytes(),
        )
    }

    fn read_mcp_record(
        &self,
        server_id: &str,
        server_url: &str,
    ) -> io::Result<Option<McpOAuthRecord>> {
        self.check_layout()?;
        let path = self.mcp_path(server_id);
        let text = match fs::symlink_metadata(&path) {
            Ok(entry) if !entry.is_file() => {
                return Err(io::Error::other(format!(
                    "{} is not a regular file",
                    path.display()
                )));
            }
            Ok(_) => fs::read_to_string(&path)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let record: McpOAuthRecord = serde_json::from_str(&text).map_err(|error| {
            io::Error::other(format!(
                "{} is not a readable MCP OAuth record ({error})",
                path.display()
            ))
        })?;
        if record.server_url != server_url {
            return Err(io::Error::other(
                "saved MCP OAuth credentials belong to a different server URL; sign in again",
            ));
        }
        Ok(Some(record))
    }

    fn update_mcp_record(
        &self,
        server_id: &str,
        server_url: &str,
        update: impl FnOnce(&mut McpOAuthRecord),
    ) -> io::Result<()> {
        let _one_writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        let mut record = self
            .read_mcp_record(server_id, server_url)?
            .unwrap_or_else(|| McpOAuthRecord {
                server_url: server_url.to_string(),
                registration: None,
                credentials: None,
            });
        update(&mut record);
        self.write_mcp_record(server_id, &record)
    }

    fn write_mcp_record(&self, server_id: &str, record: &McpOAuthRecord) -> io::Result<()> {
        self.check_layout()?;
        let directory = self.mcp_dir();
        make_private_directory(&directory)?;
        let temporary = directory.join(format!(
            "{}.{}.tmp",
            mcp_path_component(server_id),
            std::process::id()
        ));
        let _ = fs::remove_file(&temporary);
        let text = serde_json::to_string_pretty(record).map_err(io::Error::other)?;
        persist_renamed(
            &temporary,
            &self.mcp_path(server_id),
            &directory,
            format!("{text}\n").as_bytes(),
        )
    }
}

/// rmcp's OAuth store adapter. It carries only a vault handle and server
/// binding; the token itself is loaded for the duration of one SDK operation
/// and is never represented in a Toad event, setting or descriptor.
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

/// The room event contains the credential metadata, never its secret.
fn event(credential: &Credential) -> Value {
    crate::room::room_event(
        "credential",
        serde_json::to_value(credential)
            .expect("a credential is a struct of strings, bools and numbers"),
    )
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

/// Windows has no mode bits, so the ACL is the boundary: the directory's
/// inherited ACEs have to be removed and full control granted to the current
/// user alone — `icacls <dir> /inheritance:r /grant:r *<SID>:(OI)(CI)F`, the
/// discipline `../toad/src/bun/store/credentials.ts` runs. That is not built
/// here yet, and a vault that cannot prove its directory is private must not
/// write a secret into it, so this refuses instead of pretending.
#[cfg(windows)]
fn make_private_directory(path: &Path) -> io::Result<()> {
    Err(io::Error::other(format!(
        "Could not make {} private to the current Windows user; provider credentials were not written",
        path.display()
    )))
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
fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut file = create_private_file(path)?;
    file.write_all(contents)?;
    file.sync_all()?;
    Ok(())
}

/// The account's model ids as a JSON array of bare catalogue ids, 0600
/// beside the login. Replaces a list already there so Refresh can rewrite
/// without leaving the previous bytes behind a `create_new` refusal.
pub(crate) fn write_account_models(token_dir: &Path, ids: &[String]) -> io::Result<()> {
    let path = token_dir.join("models.json");
    let mut body = serde_json::to_vec(ids).map_err(io::Error::other)?;
    body.push(b'\n');
    if path.exists() {
        fs::remove_file(&path)?;
    }
    write_private(&path, &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("toad-core-vault-{name}-{}", std::process::id()));
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
        assert!(secrets(&vault).contains("sk-ant-secret"));
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
        assert!(secrets(&vault).contains("sk-ant-001"));

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
