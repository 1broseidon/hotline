//! Opaque records in the OS credential store. Disk holds only references.
//!
//! Adapted from Prism's native store. Each replacement uses new chunk IDs:
//! failed writes cannot damage the previous token, even when Windows' entry
//! size limit makes a record span several credentials.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

const SERVICE: &str = "team.toad.credentials";
const CHUNK_BYTES: usize = 2000;
const MAX_BYTES: usize = 1024 * 1024;

#[cfg(windows)]
pub(crate) mod windows;

/// Persistence for opaque secret chunks. A missing record is distinct from
/// an unavailable store. Applications normally use [`NativeStore`]; harnesses
/// can inject a disposable store without touching the operator's keychain.
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>>;
    fn set(&self, key: &str, bytes: &[u8]) -> io::Result<()>;
    fn delete(&self, key: &str) -> io::Result<()>;
}

/// macOS Keychain, Windows Credential Manager, or Linux Secret Service.
#[derive(Default)]
pub struct NativeStore;

fn native_error(error: keyring::Error) -> io::Error {
    // Platform errors can include backend details. None belong in the wire.
    match error {
        keyring::Error::NoStorageAccess(_) => io::Error::new(
            io::ErrorKind::PermissionDenied,
            "OS credential storage is locked or inaccessible. Unlock it and retry; Toad does not fall back to plaintext.",
        ),
        _ => io::Error::other(
            "OS credential storage is unavailable. Check your keychain or Secret Service and retry; Toad does not fall back to plaintext.",
        ),
    }
}

impl SecretStore for NativeStore {
    fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
        match keyring::Entry::new(SERVICE, key).and_then(|entry| entry.get_secret()) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(native_error(error)),
        }
    }

    fn set(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        keyring::Entry::new(SERVICE, key)
            .and_then(|entry| entry.set_secret(bytes))
            .map_err(native_error)
    }

    fn delete(&self, key: &str) -> io::Result<()> {
        match keyring::Entry::new(SERVICE, key).and_then(|entry| entry.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(native_error(error)),
        }
    }
}

pub(crate) fn default_store() -> Arc<dyn SecretStore> {
    #[cfg(not(test))]
    {
        Arc::new(NativeStore)
    }
    #[cfg(test)]
    {
        tests::store()
    }
}

#[derive(Clone)]
pub(crate) struct CredentialFiles {
    root: PathBuf,
    store: Arc<dyn SecretStore>,
    writer: Arc<Mutex<()>>,
}

impl CredentialFiles {
    pub(crate) fn new(root: PathBuf, store: Arc<dyn SecretStore>) -> Self {
        Self {
            root,
            store,
            writer: Arc::new(Mutex::new(())),
        }
    }

    pub(crate) fn file(&self, path: PathBuf) -> CredentialFile {
        CredentialFile {
            path,
            files: self.clone(),
        }
    }
}

/// An application-owned secret record. Cloning the handle shares persistence
/// coordination, not a cached secret. Revocation is visible to live clients.
#[derive(Clone)]
pub struct CredentialFile {
    path: PathBuf,
    files: CredentialFiles,
}

impl std::fmt::Debug for CredentialFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialFile").finish_non_exhaustive()
    }
}

impl PartialEq for CredentialFile {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path && Arc::ptr_eq(&self.files.store, &other.files.store)
    }
}
impl Eq for CredentialFile {}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Reference {
    #[serde(rename = "toadCredential")]
    version: u8,
    generation: uuid::Uuid,
    chunks: usize,
    digest: String,
}

fn corrupt() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "The stored credential is incomplete or corrupt. Restore it or sign in again.",
    )
}

fn reference(bytes: &[u8]) -> io::Result<Option<Reference>> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| corrupt())?;
    if value.get("toadCredential").is_none() {
        return Ok(None);
    }
    let reference: Reference = serde_json::from_value(value).map_err(|_| corrupt())?;
    if reference.version != 1
        || reference.chunks == 0
        || reference.chunks > MAX_BYTES.div_ceil(CHUNK_BYTES)
        || reference.digest.len() != 64
    {
        return Err(corrupt());
    }
    Ok(Some(reference))
}

impl CredentialFile {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    fn key(&self, reference: &Reference, chunk: usize) -> io::Result<String> {
        let relative = self
            .path
            .strip_prefix(&self.files.root)
            .map_err(|_| corrupt())?;
        // Exact path binding prevents copied references from resolving another
        // connection's secrets; distinct data roots also have distinct entries.
        let root = self.files.root.canonicalize()?;
        let digest = Sha256::digest(root.join(relative).as_os_str().as_encoded_bytes());
        Ok(format!("{digest:x}/{}/{chunk}", reference.generation))
    }

    fn bytes(&self) -> io::Result<Option<Vec<u8>>> {
        let mut file = match private_read(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_BYTES {
            return Err(corrupt());
        }
        Ok(Some(bytes))
    }

    fn load(&self, reference: &Reference) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        for chunk in 0..reference.chunks {
            let value = self.files.store.get(&self.key(reference, chunk)?)?.ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound,
                    "This credential is missing from the OS store. Restore it or sign in again.")
            })?;
            if value.len() > CHUNK_BYTES {
                return Err(corrupt());
            }
            bytes.extend(value);
        }
        if bytes.len() > MAX_BYTES || format!("{:x}", Sha256::digest(&bytes)) != reference.digest {
            return Err(corrupt());
        }
        Ok(bytes)
    }

    /// Migrate a legacy JSON secret on first use. A failed native write leaves
    /// the original file intact and refuses to use it as a plaintext fallback.
    pub(crate) fn read(&self) -> io::Result<Option<Vec<u8>>> {
        let _writer = self
            .files
            .writer
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let Some(bytes) = self.bytes()? else {
            return Ok(None);
        };
        if let Some(reference) = reference(&bytes)? {
            return self.load(&reference).map(Some);
        }
        self.replace(&bytes, None)?;
        Ok(Some(bytes))
    }

    pub(crate) fn write(&self, bytes: &[u8]) -> io::Result<()> {
        let _writer = self
            .files
            .writer
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let previous = self
            .bytes()?
            .map(|bytes| reference(&bytes))
            .transpose()?
            .flatten();
        self.replace(bytes, previous.as_ref())
    }

    fn replace(&self, bytes: &[u8], previous: Option<&Reference>) -> io::Result<()> {
        if bytes.len() > MAX_BYTES {
            return Err(io::Error::other("A credential record exceeds 1 MiB."));
        }
        let parent = self.path.parent().ok_or_else(corrupt)?;
        // An abandoned login must never be recreated by a pending refresh.
        check_private_path(parent)?;
        let next = Reference {
            version: 1,
            generation: uuid::Uuid::new_v4(),
            chunks: bytes.len().div_ceil(CHUNK_BYTES).max(1),
            digest: format!("{:x}", Sha256::digest(bytes)),
        };
        let staged = (|| {
            for chunk in 0..next.chunks {
                let from = (chunk * CHUNK_BYTES).min(bytes.len());
                let to = ((chunk + 1) * CHUNK_BYTES).min(bytes.len());
                self.files
                    .store
                    .set(&self.key(&next, chunk)?, &bytes[from..to])?;
            }
            if self.load(&next)? != bytes {
                return Err(corrupt());
            }
            Ok(())
        })();
        if let Err(error) = staged {
            let _ = self.remove_chunks(&next);
            return Err(error);
        }
        // After replacement is attempted, its outcome may be uncertain. Keep
        // both generations on error so a failed fsync cannot destroy either.
        atomic_write(
            &self.path,
            &serde_json::to_vec(&next).map_err(io::Error::other)?,
        )?;
        if let Some(previous) = previous {
            let _ = self.remove_chunks(previous);
        }
        Ok(())
    }

    fn remove_chunks(&self, reference: &Reference) -> io::Result<()> {
        for chunk in 0..reference.chunks {
            self.files.store.delete(&self.key(reference, chunk)?)?;
        }
        Ok(())
    }

    pub(crate) fn delete(&self) -> io::Result<()> {
        let _writer = self
            .files
            .writer
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let Some(bytes) = self.bytes()? else {
            return Ok(());
        };
        if let Some(reference) = reference(&bytes)? {
            self.remove_chunks(&reference)?;
        }
        fs::remove_file(&self.path)?;
        sync_parent(&self.path)
    }
}

pub(crate) fn check_private_path(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Credential storage cannot be a reparse point.",
            ));
        }
    }
    if metadata.file_type().is_symlink() || !(metadata.is_dir() || metadata.is_file()) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Credential storage requires a regular file or directory.",
        ));
    }
    Ok(())
}

pub(crate) fn private_read(path: &Path) -> io::Result<fs::File> {
    check_private_path(path)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(corrupt());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Credential storage cannot be a reparse point.",
            ));
        }
    }
    Ok(file)
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(corrupt)?;
    check_private_path(parent)?;
    match check_private_path(path) {
        Ok(()) => (),
        Err(error) if error.kind() == io::ErrorKind::NotFound => (),
        Err(error) => return Err(error),
    }
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    sync_parent(path)
}

fn sync_parent(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    fs::File::open(path.parent().ok_or_else(corrupt)?)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::OnceLock;

    #[derive(Default)]
    pub(crate) struct MemoryStore(pub Mutex<HashMap<String, Vec<u8>>>);
    impl SecretStore for MemoryStore {
        fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
            Ok(self.0.lock().unwrap().get(key).cloned())
        }
        fn set(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
            self.0.lock().unwrap().insert(key.into(), bytes.into());
            Ok(())
        }
        fn delete(&self, key: &str) -> io::Result<()> {
            self.0.lock().unwrap().remove(key);
            Ok(())
        }
    }
    pub(crate) fn store() -> Arc<dyn SecretStore> {
        static STORE: OnceLock<Arc<MemoryStore>> = OnceLock::new();
        STORE.get_or_init(Default::default).clone()
    }

    #[derive(Default)]
    struct FailingStore {
        memory: MemoryStore,
        writes: std::sync::atomic::AtomicUsize,
        fail_at: std::sync::atomic::AtomicUsize,
        locked: std::sync::atomic::AtomicBool,
    }
    impl SecretStore for FailingStore {
        fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
            if self.locked.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "store locked",
                ));
            }
            self.memory.get(key)
        }
        fn set(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
            use std::sync::atomic::Ordering::SeqCst;
            let write = self.writes.fetch_add(1, SeqCst) + 1;
            if self.locked.load(SeqCst) || self.fail_at.load(SeqCst) == write {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "store write failed",
                ));
            }
            self.memory.set(key, bytes)
        }
        fn delete(&self, key: &str) -> io::Result<()> {
            self.memory.delete(key)
        }
    }

    #[test]
    fn failed_chunk_rotation_preserves_the_previous_credential() {
        use std::sync::atomic::Ordering::SeqCst;
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(FailingStore::default());
        let file = CredentialFiles::new(root.path().into(), store.clone())
            .file(root.path().join("auth.json"));
        let previous = vec![b'a'; 4500];
        file.write(&previous).unwrap();
        let reference = fs::read(file.path()).unwrap();
        store.fail_at.store(5, SeqCst); // fail on the second chunk of the replacement
        assert!(file.write(&vec![b'b'; 6500]).is_err());
        assert_eq!(fs::read(file.path()).unwrap(), reference);
        assert_eq!(file.read().unwrap().unwrap(), previous);
        assert_eq!(store.memory.0.lock().unwrap().len(), 3);
    }

    #[test]
    fn migration_verifies_storage_before_replacing_plaintext_and_never_falls_back() {
        use std::sync::atomic::Ordering::SeqCst;
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(FailingStore::default());
        let file = CredentialFiles::new(root.path().into(), store.clone())
            .file(root.path().join("auth.json"));
        let legacy = br#"{"key":"legacy-secret"}"#;
        fs::write(file.path(), legacy).unwrap();
        store.locked.store(true, SeqCst);
        assert_eq!(
            file.read().unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(fs::read(file.path()).unwrap(), legacy);
        store.locked.store(false, SeqCst);
        assert_eq!(file.read().unwrap().unwrap(), legacy);
        assert!(
            !fs::read_to_string(file.path())
                .unwrap()
                .contains("legacy-secret")
        );
        store.locked.store(true, SeqCst);
        assert_eq!(
            file.read().unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        store.locked.store(false, SeqCst);
        store.memory.0.lock().unwrap().clear();
        assert_eq!(file.read().unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn a_copied_reference_cannot_resolve_another_connections_secret() {
        let root = tempfile::tempdir().unwrap();
        let files = CredentialFiles::new(root.path().into(), Arc::new(MemoryStore::default()));
        let first = files.file(root.path().join("first.json"));
        let second = files.file(root.path().join("second.json"));
        first.write(b"secret-one").unwrap();
        fs::copy(first.path(), second.path()).unwrap();
        assert_eq!(second.read().unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(first.read().unwrap().unwrap(), b"secret-one");
    }

    #[test]
    fn revoked_record_cannot_be_recreated_by_a_live_handle() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("login");
        fs::create_dir(&dir).unwrap();
        let file = CredentialFiles::new(root.path().into(), Arc::new(MemoryStore::default()))
            .file(dir.join("auth.json"));
        file.write(b"token").unwrap();
        file.delete().unwrap();
        fs::remove_dir(&dir).unwrap();
        assert!(file.write(b"rotated-token").is_err());
        assert!(!dir.exists());
    }

    #[test]
    #[ignore = "writes one disposable entry to the native OS credential store"]
    fn native_store_roundtrip() {
        let key = format!("native-test-{}", uuid::Uuid::new_v4());
        let store = NativeStore;
        assert!(store.get(&key).unwrap().is_none());
        store.set(&key, b"disposable-toad-credential").unwrap();
        let read = store.get(&key);
        let deleted = store.delete(&key);
        assert_eq!(read.unwrap().unwrap(), b"disposable-toad-credential");
        deleted.unwrap();
        assert!(store.get(&key).unwrap().is_none());
    }
}
