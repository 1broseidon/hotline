use super::*;
use crate::credentials::{MemoryStore, SecretStore};
use crate::driver::failure::{Failure, Kind};
use std::sync::atomic::AtomicBool;

pub(super) struct LoginDir {
    _root: tempfile::TempDir,
    vault: crate::vault::Vault,
    id: String,
    dir: PathBuf,
}

impl LoginDir {
    pub(super) fn new(store: Arc<dyn SecretStore>) -> Self {
        let root = tempfile::tempdir().unwrap();
        let log = crate::log::Log::open(root.path());
        let vault = crate::vault::Vault::open_with_store(root.path(), log, store).unwrap();
        let (id, dir) = vault.begin_login().unwrap();
        Self {
            _root: root,
            vault,
            id,
            dir,
        }
    }

    pub(super) fn path(&self) -> &Path {
        &self.dir
    }
}

fn reference(dir: &LoginDir, name: &str) -> Value {
    let value: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join(name)).unwrap()).unwrap();
    assert_eq!(value["hotlineCredential"], 1);
    assert!(value.get("access_token").is_none());
    assert!(value.get("github_token").is_none());
    value
}

#[tokio::test]
async fn rig_vault_plaintext_auth_becomes_a_reference_on_first_read() {
    let memory = Arc::new(MemoryStore::default());
    let dir = LoginDir::new(memory.clone());
    let written = chatgpt_0_42(dir.path(), "private-access", i64::MAX);
    let store = LoginDirStore::new(dir.path());
    assert_eq!(
        store.load(chatgpt::PROVIDER_NAME).await.unwrap().unwrap(),
        written
    );
    let first = reference(&dir, CHATGPT_RECORD);
    assert!(!memory.0.lock().unwrap().is_empty());
    assert_eq!(
        store.load(chatgpt::PROVIDER_NAME).await.unwrap().unwrap(),
        written
    );
    assert_eq!(
        reference(&dir, CHATGPT_RECORD),
        first,
        "migration runs once"
    );
}

/// Fail after one chunk has reached memory, so migration must clean up the
/// partial generation as well as keep the original plaintext.
#[derive(Default)]
struct LockedStore {
    memory: MemoryStore,
    locked: AtomicBool,
    writes: AtomicUsize,
}

impl SecretStore for LockedStore {
    fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
        self.memory.get(key)
    }

    fn set(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        if self.locked.load(Ordering::SeqCst) && self.writes.fetch_add(1, Ordering::SeqCst) > 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "keychain locked",
            ));
        }
        self.memory.set(key, bytes)
    }

    fn delete(&self, key: &str) -> io::Result<()> {
        self.memory.delete(key)
    }
}

#[tokio::test]
async fn rig_vault_keychain_failure_keeps_plaintext_and_asks_to_unlock() {
    let backend = Arc::new(LockedStore::default());
    let dir = LoginDir::new(backend.clone());
    let written = chatgpt_0_42(dir.path(), &"private-access".repeat(500), i64::MAX);
    backend.locked.store(true, Ordering::SeqCst);
    let error = chatgpt(dir.path()).auth_context(&http()).await.unwrap_err();
    assert!(matches!(error, chatgpt::auth::AuthError::Store(_)));
    assert_eq!(
        std::fs::read_to_string(dir.path().join(CHATGPT_RECORD)).unwrap(),
        written
    );
    assert!(backend.memory.0.lock().unwrap().is_empty());

    // Outgoing wraps authentication errors in Rig's HTTP error before the
    // driver sees them. That wrapper must not turn this into a network retry.
    let error = rig::ProviderError::Http(Arc::new(http_client::Error::instance(io::Error::other(
        error.to_string(),
    ))));
    let failure = Failure::provider(error, "request");
    assert_eq!(failure.kind, Kind::Keychain);
    assert!(failure.summary.contains("Unlock your keychain"));
    assert!(!failure.summary.contains("Sign in again"));
    assert!(failure.sign_in.is_none());

    backend.locked.store(false, Ordering::SeqCst);
    assert_eq!(
        LoginDirStore::new(dir.path())
            .load(chatgpt::PROVIDER_NAME)
            .await
            .unwrap()
            .unwrap(),
        written
    );
    reference(&dir, CHATGPT_RECORD);
}

#[tokio::test]
async fn rig_vault_delete_removes_every_chunk() {
    for key in [chatgpt::PROVIDER_NAME, copilot::PROVIDER_NAME] {
        let memory = Arc::new(MemoryStore::default());
        let dir = LoginDir::new(memory.clone());
        let store = LoginDirStore::new(dir.path());
        let body = json!({"token": "private-token".repeat(500)}).to_string();
        store.save(key, &body).await.unwrap();
        assert!(memory.0.lock().unwrap().len() > 1);
        assert_eq!(store.load(key).await.unwrap().unwrap(), body);
        store.delete(key).await.unwrap();
        assert!(memory.0.lock().unwrap().is_empty());
        assert!(store.load(key).await.unwrap().is_none());
    }
}

#[tokio::test]
async fn rig_vault_abandon_removes_copilot_chunks_and_reference() {
    let memory = Arc::new(MemoryStore::default());
    let dir = LoginDir::new(memory.clone());
    LoginDirStore::new(dir.path())
        .save(
            copilot::PROVIDER_NAME,
            &json!({"github_token": "secret".repeat(1000)}).to_string(),
        )
        .await
        .unwrap();
    assert!(memory.0.lock().unwrap().len() > 1);
    dir.vault.abandon_login(&dir.id).unwrap();
    assert!(memory.0.lock().unwrap().is_empty());
    assert!(!dir.path().exists());
}

#[tokio::test]
async fn rig_vault_0_42_copilot_migrates_straight_into_the_keychain() {
    let memory = Arc::new(MemoryStore::default());
    let dir = LoginDir::new(memory.clone());
    let mut expected = copilot_0_42(dir.path(), "private-github");
    expected["github_token"] = json!("private-github");
    let loaded = LoginDirStore::new(dir.path())
        .load(copilot::PROVIDER_NAME)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&loaded).unwrap(), expected);
    reference(&dir, COPILOT_RECORD);
    assert!(!memory.0.lock().unwrap().is_empty());
    assert!(!dir.path().join(LEGACY_GITHUB_TOKEN).exists());
    assert!(!dir.path().join(LEGACY_API_KEY).exists());
}

#[tokio::test]
async fn rig_vault_s1_plaintext_copilot_migrates_into_the_keychain() {
    let memory = Arc::new(MemoryStore::default());
    let dir = LoginDir::new(memory.clone());
    let body = json!({"github_token": "s1-github", "token": "s1-copilot"}).to_string();
    std::fs::write(dir.path().join(COPILOT_RECORD), &body).unwrap();
    assert_eq!(
        LoginDirStore::new(dir.path())
            .load(copilot::PROVIDER_NAME)
            .await
            .unwrap()
            .unwrap(),
        body
    );
    reference(&dir, COPILOT_RECORD);
    assert!(!memory.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn rig_vault_failed_legacy_copilot_migration_keeps_both_files() {
    let backend = Arc::new(LockedStore::default());
    let dir = LoginDir::new(backend.clone());
    let token = "github-token".repeat(500);
    let api = copilot_0_42(dir.path(), &token);
    backend.locked.store(true, Ordering::SeqCst);
    let error = copilot(dir.path()).auth_context(&http()).await.unwrap_err();
    assert!(matches!(error, chatgpt::auth::AuthError::Store(_)));
    assert_eq!(
        std::fs::read_to_string(dir.path().join(LEGACY_GITHUB_TOKEN)).unwrap(),
        format!("{token}\n")
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join(LEGACY_API_KEY)).unwrap(),
        api.to_string()
    );
    assert!(!dir.path().join(COPILOT_RECORD).exists());
    assert!(backend.memory.0.lock().unwrap().is_empty());
}
