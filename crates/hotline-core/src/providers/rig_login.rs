//! ChatGPT and GitHub Copilot sign-ins, kept in the OS credential store
//! behind a reference file in the credential's login directory.
//!
//! Rig signs in, refreshes and reads the tokens. [`LoginDirStore`] is the
//! store it does that through: each record is a [`CredentialFile`] of the
//! vault that holds the directory, so a sign-in made before Rig 0.45, kept
//! there in plain text, moves into the keychain the first time it is read
//! and is the same sign-in after it. [`chatgpt`] and [`copilot`] hand every
//! caller of one credential (turns, voice, image generation, discovery) the
//! same authenticator, which shares one refresh and one in-memory token
//! among them; the store's per-credential lock covers any other
//! authenticator over the same directory, such as a sign-in. A rotated
//! refresh token is spent once.

use crate::credentials::{CredentialFile, CredentialFiles};
use rig::error::BoxError;
use rig::http_client::{HttpClientExt, ReqwestClient};
use rig::providers::chatgpt::auth::{
    AuthContext, DeviceCodeHandler, DeviceCodePrompt, StoreLock, TokenStore,
};
use rig::providers::{chatgpt, copilot};
use rig::wasm_compat::WasmBoxedFuture;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

/// ChatGPT's record: the file Rig 0.42 wrote, whose fields Rig's record
/// still has, so the plain text it holds moves into the keychain as it
/// stands.
pub(crate) const CHATGPT_RECORD: &str = "auth.json";
/// Copilot's one record: the GitHub token and the Copilot API token beside it.
pub(crate) const COPILOT_RECORD: &str = "copilot.json";
/// Rig 0.42's Copilot layout: the GitHub token as plain text, and the API
/// token it was exchanged for. Read once, into [`COPILOT_RECORD`].
const LEGACY_GITHUB_TOKEN: &str = "access-token";
const LEGACY_API_KEY: &str = "api-key.json";

/// The credential files of every open vault, by the directory its logins
/// are made in. A login is named to its callers by its directory alone
/// (`ProviderAuth::Login`), and the vault that made it is the one whose
/// store holds its tokens: a desk on the file store keeps them there, not
/// in the keychain.
static VAULTS: LazyLock<Mutex<HashMap<PathBuf, (CredentialFiles, crate::log::Log)>>> =
    LazyLock::new(Mutex::default);

/// Keeps the records of every login directory in `logins` in `files`. The
/// vault says so when it opens.
pub(crate) fn keep_logins_in(logins: PathBuf, files: CredentialFiles, log: crate::log::Log) {
    VAULTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(logins, (files, log));
}

/// Rig's records for one login directory: the key Rig names a record by
/// picks its file there.
pub(crate) struct LoginDirStore {
    dir: PathBuf,
}

impl LoginDirStore {
    pub(crate) fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
        }
    }

    /// The record kept in the file `name`, in the store of the vault that
    /// holds this directory. A directory no open vault holds, which only a
    /// test makes, is kept in the process's own store.
    fn file(&self, name: &str) -> CredentialFile {
        let held = self.dir.parent().and_then(|logins| {
            VAULTS
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(logins)
                .map(|(files, _)| files.clone())
        });
        held.unwrap_or_else(|| {
            CredentialFiles::new(self.dir.clone(), crate::vault::default_store())
        })
        .file(self.dir.join(name))
    }

    /// The file the record Rig names `key` is kept in.
    fn record(key: &str) -> Result<&'static str, BoxError> {
        match key {
            chatgpt::PROVIDER_NAME => Ok(CHATGPT_RECORD),
            copilot::PROVIDER_NAME => Ok(COPILOT_RECORD),
            other => Err(format!("a login directory keeps no {other} record").into()),
        }
    }
}

impl TokenStore for LoginDirStore {
    fn load<'a>(&'a self, key: &'a str) -> WasmBoxedFuture<'a, Result<Option<String>, BoxError>> {
        Box::pin(async move {
            let record = self.file(Self::record(key)?);
            let copilot = key == copilot::PROVIDER_NAME;
            let dir = self.dir.clone();
            let bytes = blocking(move || {
                let lock = login_lock(&dir);
                let _writing = lock.lock().unwrap_or_else(PoisonError::into_inner);
                if copilot {
                    migrate_copilot(&dir, &record)?;
                }
                record.read()
            })
            .await?;
            Ok(bytes.map(String::from_utf8).transpose()?)
        })
    }

    fn save<'a>(
        &'a self,
        key: &'a str,
        record: &'a str,
    ) -> WasmBoxedFuture<'a, Result<(), BoxError>> {
        Box::pin(async move {
            let file = self.file(Self::record(key)?);
            let bytes = record.as_bytes().to_vec();
            let dir = self.dir.clone();
            blocking(move || {
                let lock = login_lock(&dir);
                let _writing = lock.lock().unwrap_or_else(PoisonError::into_inner);
                live(&dir).map_err(io::Error::other)?;
                file.write(&bytes)
            })
            .await
        })
    }

    fn delete<'a>(&'a self, key: &'a str) -> WasmBoxedFuture<'a, Result<(), BoxError>> {
        Box::pin(async move {
            let file = self.file(Self::record(key)?);
            blocking(move || file.delete()).await
        })
    }

    /// One lock per record file, never per provider: a Copilot device flow
    /// holds its credential's lock for as long as the person takes to
    /// approve it, up to fifteen minutes, and no other credential waits.
    fn lock<'a>(&'a self, key: &'a str) -> WasmBoxedFuture<'a, Result<StoreLock, BoxError>> {
        Box::pin(async move {
            let lock = record_lock(&self.dir.join(Self::record(key)?));
            Ok(StoreLock::new(lock.lock_owned().await))
        })
    }
}

/// Serialize the local write with revoke/delete. Rig's async record lock
/// spans network refresh; this lock only spans disk/keychain work, so a
/// revoked refresh cannot recreate a record after local deletion.
pub(crate) fn login_lock(dir: &Path) -> Arc<Mutex<()>> {
    static LOCKS: LazyLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = LazyLock::new(Mutex::default);
    LOCKS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .entry(dir.to_path_buf())
        .or_default()
        .clone()
}

/// Every record lock of the login in `dir`. A refresh holds its record's
/// from reading the tokens to saving the new ones, so tokens written while
/// these are held are never overwritten by a refresh of the ones before.
pub(crate) async fn hold_records(dir: &Path) -> [OwnedMutexGuard<()>; 2] {
    [
        record_lock(&dir.join(CHATGPT_RECORD)).lock_owned().await,
        record_lock(&dir.join(COPILOT_RECORD)).lock_owned().await,
    ]
}

/// The lock every store over the record at `path` takes.
fn record_lock(path: &Path) -> Arc<AsyncMutex<()>> {
    static LOCKS: LazyLock<Mutex<HashMap<PathBuf, Arc<AsyncMutex<()>>>>> =
        LazyLock::new(Mutex::default);
    LOCKS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .entry(path.to_path_buf())
        .or_default()
        .clone()
}

/// A keychain call off the async workers: one can wait on the person
/// unlocking the keychain or allowing Hotline into it.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> Result<T, BoxError> {
    Ok(tokio::task::spawn_blocking(work).await??)
}

/// Moves a Rig 0.42 Copilot sign-in into `record`, in the keychain: the
/// GitHub token from `access-token`, and the API token record
/// `api-key.json` holds beside it, field for field. The old files are
/// removed only once the record is written, so a keychain that refuses the
/// write leaves the sign-in where it was. A directory with a record already
/// (a plain-text `copilot.json` moves on its own first read), or with no token in
/// either old file, has nothing to move.
fn migrate_copilot(dir: &Path, record: &CredentialFile) -> io::Result<()> {
    if record.path().symlink_metadata().is_ok() {
        return Ok(());
    }
    let mut fields = match crate::vault::read_model_file(&dir.join(LEGACY_API_KEY)) {
        Ok(bytes) => serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(&bytes)
            .unwrap_or_default(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => serde_json::Map::new(),
        Err(error) => return Err(error),
    };
    let github_token = match crate::vault::read_model_file(&dir.join(LEGACY_GITHUB_TOKEN)) {
        Ok(bytes) => Some(String::from_utf8_lossy(&bytes).trim().to_string())
            .filter(|token| !token.is_empty()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    if let Some(token) = github_token {
        fields.insert("github_token".into(), token.into());
    }
    let api_token = fields
        .get("token")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|token| !token.trim().is_empty());
    if !api_token && !fields.contains_key("github_token") {
        return Ok(());
    }
    record.write(&serde_json::to_vec(&fields).map_err(io::Error::other)?)?;
    remove_if_present(&dir.join(LEGACY_GITHUB_TOKEN))?;
    remove_if_present(&dir.join(LEGACY_API_KEY))
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// What every ChatGPT and Copilot authentication request is sent over.
pub(crate) fn http() -> ReqwestClient {
    ReqwestClient::default()
}

static SHARED: LazyLock<Mutex<HashMap<PathBuf, Login>>> = LazyLock::new(Mutex::default);

/// The shared, revocable credential used by every caller of this login.
pub(crate) fn chatgpt(dir: &Path) -> Login {
    shared(dir, || {
        Authenticator::ChatGpt(chatgpt::auth::Authenticator::new(
            chatgpt::auth::AuthSource::OAuth,
            LoginDirStore::new(dir),
            DeviceCodeHandler::default(),
            false,
        ))
    })
}

pub(crate) fn copilot(dir: &Path) -> Login {
    shared(dir, || {
        Authenticator::Copilot(copilot::auth::Authenticator::new(
            copilot::auth::AuthSource::OAuth,
            LoginDirStore::new(dir),
            DeviceCodeHandler::default(),
            false,
        ))
    })
}

fn shared(dir: &Path, make: impl FnOnce() -> Authenticator) -> Login {
    SHARED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .entry(dir.to_path_buf())
        .or_insert_with(|| Login {
            dir: dir.to_path_buf(),
            auth: Arc::new(Mutex::new(Some(make()))),
        })
        .clone()
}

/// Retained transports lose their authenticator as well as the registry.
/// Rig exposes forget for ChatGPT; Copilot's cache dies with its last clone.
pub(crate) fn forget(dir: &Path) {
    if let Some(login) = SHARED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(dir)
        && let Some(Authenticator::ChatGpt(auth)) = login
            .auth
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    {
        auth.forget();
    }
}

/// Check the room too: a new registry entry must not revive a revoked row.
#[expect(
    clippy::result_large_err,
    reason = "Rig authentication errors stay typed for failure classification"
)]
fn live(dir: &Path) -> Result<(), chatgpt::auth::AuthError> {
    let log = dir.parent().and_then(|parent| {
        VAULTS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(parent)
            .map(|(_, log)| log.clone())
    });
    let revoked = if let Some(log) = log {
        let id = dir.file_name().and_then(|name| name.to_str());
        log.try_load(&crate::log::StreamId::Room)
            .map_err(|error| chatgpt::auth::AuthError::Store(error.into()))?
            .iter()
            .any(|row| {
                row["kind"] == "credential"
                    && row["id"].as_str() == id
                    && (row["revoked"] == true || row["deleted"] == true)
            })
    } else {
        false
    };
    if revoked || !dir.is_dir() {
        Err(chatgpt::auth::AuthError::Message(
            "This sign-in is no longer live. Sign in again under Settings → Providers.".into(),
        ))
    } else {
        Ok(())
    }
}

/// Signs the new login in `dir` in to ChatGPT with a device code, which
/// `show` puts in front of the person. The tokens land in the login's
/// record, and the login's shared authenticator holds them from then on.
pub(crate) async fn chatgpt_sign_in(
    dir: &Path,
    show: impl Fn(DeviceCodePrompt) + Send + Sync + 'static,
) -> Result<AuthContext, String> {
    let Authenticator::ChatGpt(auth) = chatgpt(dir).authenticator()? else {
        unreachable!("ChatGPT sign-in uses a ChatGPT authenticator")
    };
    auth.with_device_code_handler(DeviceCodeHandler::new(show))
        .sign_in_with_device_code(&http())
        .await
        .map_err(|error| error.to_string())
}

/// Signs the new login in `dir` in to GitHub Copilot with a device code.
/// Copilot's sign-in is its first token exchange, made by an authenticator
/// that may start the device flow, over the same record and lock.
pub(crate) async fn copilot_sign_in(
    dir: &Path,
    show: impl Fn(DeviceCodePrompt) + Send + Sync + 'static,
) -> Result<AuthContext, String> {
    copilot::auth::Authenticator::new(
        copilot::auth::AuthSource::OAuth,
        LoginDirStore::new(dir),
        DeviceCodeHandler::new(show),
        true,
    )
    .auth_context(&http())
    .await
    .map_err(|error| error.to_string())
}

/// One login's shared authenticator, whichever provider it signs in to.
#[derive(Clone)]
enum Authenticator {
    ChatGpt(chatgpt::auth::Authenticator),
    Copilot(copilot::auth::Authenticator),
}

/// A retained client checks both the current room record and its own
/// invalidation before and after a refresh, so deletion during a refresh
/// cannot sign the next request with the token that just came back.
#[derive(Clone)]
pub(crate) struct Login {
    dir: PathBuf,
    auth: Arc<Mutex<Option<Authenticator>>>,
}

impl Login {
    fn authenticator(&self) -> Result<Authenticator, String> {
        self.auth
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or_else(|| "This sign-in is no longer live.".into())
    }

    #[expect(
        clippy::result_large_err,
        reason = "CredentialSource returns Rig authentication errors unchanged"
    )]
    pub(crate) async fn auth_context<H: HttpClientExt>(
        &self,
        http: &H,
    ) -> Result<AuthContext, chatgpt::auth::AuthError> {
        live(&self.dir)?;
        let auth = self
            .authenticator()
            .map_err(chatgpt::auth::AuthError::Message)?;
        let context = match auth {
            Authenticator::ChatGpt(auth) => auth.auth_context(http).await,
            Authenticator::Copilot(auth) => auth.auth_context(http).await,
        }?;
        live(&self.dir)?;
        self.authenticator()
            .map_err(chatgpt::auth::AuthError::Message)?;
        Ok(context)
    }

    pub(crate) async fn context(&self) -> Result<AuthContext, String> {
        use rig::client::signed_in::CredentialSource;
        CredentialSource::auth_context(self, &rig::http_client::DynHttpClient::new(http()))
            .await
            .map_err(|error| error.to_string())
    }
}

impl rig::client::signed_in::CredentialSource for Login {
    async fn auth_context(
        &self,
        http: &rig::http_client::DynHttpClient,
    ) -> Result<AuthContext, chatgpt::auth::AuthError> {
        self.auth_context(http).await
    }
}

pub(crate) mod revoke;

#[cfg(test)]
mod tests;
