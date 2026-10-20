//! ChatGPT and GitHub Copilot sign-ins, kept where Hotline has always kept
//! them: in the credential's login directory under the vault.
//!
//! Rig signs in, refreshes and reads the tokens. [`LoginDirStore`] is the
//! store it does that through, so a login made before Rig 0.45 is the same
//! login after it. [`chatgpt`] and [`copilot`] hand every caller of one
//! credential (turns, voice, image generation, discovery) the same
//! authenticator, which shares one refresh and one in-memory token among
//! them; the store's per-credential lock covers any other authenticator over
//! the same directory, such as a sign-in. A rotated refresh token is spent
//! once.

use rig::error::BoxError;
use rig::http_client::ReqwestClient;
use rig::providers::chatgpt::auth::{
    AuthContext, DeviceCodeHandler, DeviceCodePrompt, StoreLock, TokenStore,
};
use rig::providers::{chatgpt, copilot};
use rig::wasm_compat::WasmBoxedFuture;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use tokio::sync::Mutex as AsyncMutex;

/// ChatGPT's record: the file Rig 0.42 wrote, whose fields Rig's record
/// still has, so it is read as it stands.
pub(crate) const CHATGPT_RECORD: &str = "auth.json";
/// Copilot's one record: the GitHub token and the Copilot API token beside it.
pub(crate) const COPILOT_RECORD: &str = "copilot.json";
/// Rig 0.42's Copilot layout: the GitHub token as plain text, and the API
/// token it was exchanged for. Read once, into [`COPILOT_RECORD`].
const LEGACY_GITHUB_TOKEN: &str = "access-token";
const LEGACY_API_KEY: &str = "api-key.json";

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

    fn file(&self, key: &str) -> Result<PathBuf, BoxError> {
        let name = match key {
            chatgpt::PROVIDER_NAME => CHATGPT_RECORD,
            copilot::PROVIDER_NAME => COPILOT_RECORD,
            other => return Err(format!("a login directory keeps no {other} record").into()),
        };
        Ok(self.dir.join(name))
    }
}

impl TokenStore for LoginDirStore {
    fn load<'a>(&'a self, key: &'a str) -> WasmBoxedFuture<'a, Result<Option<String>, BoxError>> {
        Box::pin(async move {
            let path = self.file(key)?;
            if key == copilot::PROVIDER_NAME {
                migrate_copilot(&self.dir)?;
            }
            match crate::vault::read_model_file(&path) {
                Ok(bytes) => Ok(Some(String::from_utf8(bytes)?)),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error.into()),
            }
        })
    }

    fn save<'a>(
        &'a self,
        key: &'a str,
        record: &'a str,
    ) -> WasmBoxedFuture<'a, Result<(), BoxError>> {
        Box::pin(async move {
            let path = self.file(key)?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            crate::vault::write_beside_login(&self.dir, name, record.as_bytes())?;
            Ok(())
        })
    }

    fn delete<'a>(&'a self, key: &'a str) -> WasmBoxedFuture<'a, Result<(), BoxError>> {
        Box::pin(async move {
            remove_if_present(&self.file(key)?)?;
            Ok(())
        })
    }

    /// One lock per record file, never per provider: a Copilot device flow
    /// holds its credential's lock for as long as the person takes to
    /// approve it, up to fifteen minutes, and no other credential waits.
    fn lock<'a>(&'a self, key: &'a str) -> WasmBoxedFuture<'a, Result<StoreLock, BoxError>> {
        Box::pin(async move {
            let lock = record_lock(&self.file(key)?);
            Ok(StoreLock::new(lock.lock_owned().await))
        })
    }
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

/// Moves a Rig 0.42 Copilot sign-in into [`COPILOT_RECORD`]: the GitHub
/// token from `access-token`, and the API token record `api-key.json` holds
/// beside it, field for field. The old files are removed only once the new
/// record is on disk, so a failed write leaves the sign-in where it was. A
/// directory with a record already, or with no token in either old file,
/// has nothing to move.
fn migrate_copilot(dir: &Path) -> io::Result<()> {
    if dir.join(COPILOT_RECORD).symlink_metadata().is_ok() {
        return Ok(());
    }
    let mut record = match crate::vault::read_model_file(&dir.join(LEGACY_API_KEY)) {
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
        record.insert("github_token".into(), token.into());
    }
    let api_token = record
        .get("token")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|token| !token.trim().is_empty());
    if !api_token && !record.contains_key("github_token") {
        return Ok(());
    }
    let body = serde_json::to_vec(&record).map_err(io::Error::other)?;
    crate::vault::write_beside_login(dir, COPILOT_RECORD, &body)?;
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

/// The ChatGPT authenticator for the login in `dir`, shared by everything
/// that uses that login. It never starts a sign-in: with no usable tokens
/// it fails, naming the sign-in.
pub(crate) fn chatgpt(dir: &Path) -> chatgpt::auth::Authenticator {
    static SHARED: LazyLock<Mutex<HashMap<PathBuf, chatgpt::auth::Authenticator>>> =
        LazyLock::new(Mutex::default);
    SHARED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .entry(dir.to_path_buf())
        .or_insert_with(|| {
            chatgpt::auth::Authenticator::new(
                chatgpt::auth::AuthSource::OAuth,
                LoginDirStore::new(dir),
                DeviceCodeHandler::default(),
                false,
            )
        })
        .clone()
}

/// The Copilot authenticator for the login in `dir`, shared the same way.
pub(crate) fn copilot(dir: &Path) -> copilot::auth::Authenticator {
    static SHARED: LazyLock<Mutex<HashMap<PathBuf, copilot::auth::Authenticator>>> =
        LazyLock::new(Mutex::default);
    SHARED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .entry(dir.to_path_buf())
        .or_insert_with(|| {
            copilot::auth::Authenticator::new(
                copilot::auth::AuthSource::OAuth,
                LoginDirStore::new(dir),
                DeviceCodeHandler::default(),
                false,
            )
        })
        .clone()
}

/// Signs the new login in `dir` in to ChatGPT with a device code, which
/// `show` puts in front of the person. The tokens land in the login's
/// record, and the login's shared authenticator holds them from then on.
pub(crate) async fn chatgpt_sign_in(
    dir: &Path,
    show: impl Fn(DeviceCodePrompt) + Send + Sync + 'static,
) -> Result<AuthContext, String> {
    chatgpt(dir)
        .with_device_code_handler(DeviceCodeHandler::new(show))
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
pub(crate) enum Login {
    ChatGpt(chatgpt::auth::Authenticator),
    Copilot(copilot::auth::Authenticator),
}

impl Login {
    /// The login's live token, refreshed first when it has expired, or why
    /// there is none.
    pub(crate) async fn context(&self) -> Result<AuthContext, String> {
        match self {
            Self::ChatGpt(auth) => auth.auth_context(&http()).await,
            Self::Copilot(auth) => auth.auth_context(&http()).await,
        }
        .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests;
