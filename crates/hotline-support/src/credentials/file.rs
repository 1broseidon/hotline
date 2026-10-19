//! Secret chunks as owner-only files, for a desk with no OS credential store.
//!
//! A server has no session bus and no unlocked Secret Service, so `hotline
//! serve --store file` keeps the chunks [`super::CredentialFiles`] writes as
//! files under `<data>/secrets/`: the directory is 0700, every file 0600,
//! both created that way rather than narrowed afterwards. This is chosen by
//! the operator, never reached by falling back from the native store, and a
//! room records which one it uses (`store.json`, see [`super::claim_backend`])
//! so a desk started on the other one refuses instead of starting empty.
//!
//! What protects these files is the account that owns them and the disk they
//! are on. Anyone who is root on the machine, or holds a copy of its disk or
//! its backups, can read them — as they can the login tokens Rig already
//! keeps in `vault/logins/`. `docs/security.md` says so.
//!
//! A replacement is written beside the old file, flushed, and renamed over
//! it, so a failed write leaves the previous record whole. A name is the
//! SHA-256 of the key, so no key can climb out of the directory. A symlink or
//! anything else that is not a plain file is refused, as is a directory that
//! someone else owns or can read.

use super::SecretStore;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

pub const SECRETS_DIR: &str = "secrets";
const MAX_FILE: u64 = 64 * 1024;

pub struct FileStore {
    dir: PathBuf,
    writer: Mutex<()>,
}

impl FileStore {
    /// Opens, creating if needed, `<root>/secrets`.
    pub fn open(root: &Path) -> io::Result<FileStore> {
        let dir = root.join(SECRETS_DIR);
        match fs::DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(unavailable(&dir, error)),
        }
        let store = FileStore {
            dir,
            writer: Mutex::new(()),
        };
        store.check_dir()?;
        Ok(store)
    }

    /// The directory must still be ours alone, every time it is used: a
    /// directory swapped for a symlink, or opened up to other accounts,
    /// stops the store rather than being written through.
    fn check_dir(&self) -> io::Result<()> {
        let meta =
            fs::symlink_metadata(&self.dir).map_err(|error| unavailable(&self.dir, error))?;
        if !meta.file_type().is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "{} is not a plain directory. Hotline keeps secrets only in a directory it created.",
                    self.dir.display()
                ),
            ));
        }
        // SAFETY: getuid cannot fail and touches no memory.
        let uid = unsafe { libc::getuid() };
        if meta.uid() != uid || meta.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "{} must belong to this account and be closed to every other (mode 0700).",
                    self.dir.display()
                ),
            ));
        }
        Ok(())
    }

    /// A record's name is the SHA-256 of its key: fixed length whatever the
    /// key, nothing a key says reaches the file system, and no key can name
    /// a path outside this directory.
    fn path(&self, key: &str) -> PathBuf {
        use sha2::{Digest, Sha256};
        self.dir
            .join(format!("{:x}", Sha256::digest(key.as_bytes())))
    }

    fn sync_dir(&self) -> io::Result<()> {
        File::open(&self.dir)?.sync_all()
    }
}

fn unavailable(path: &Path, error: io::Error) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "The secret store at {} is unavailable: {error}. Hotline does not fall back to anywhere else.",
            path.display()
        ),
    )
}

impl SecretStore for FileStore {
    fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
        self.check_dir()?;
        let path = self.path(key);
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) if error.raw_os_error() == Some(libc::ELOOP) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "A secret record is a symlink, and Hotline does not follow one.",
                ));
            }
            Err(error) => return Err(unavailable(&path, error)),
        };
        let meta = file.metadata()?;
        if !meta.file_type().is_file() || meta.len() > MAX_FILE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "A secret record is not a plain file of a secret's size.",
            ));
        }
        let mut bytes = Vec::with_capacity(meta.len() as usize);
        file.take(MAX_FILE).read_to_end(&mut bytes)?;
        Ok(Some(bytes))
    }

    fn set(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() as u64 > MAX_FILE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "A secret chunk is larger than a record holds.",
            ));
        }
        let _held = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        self.check_dir()?;
        let path = self.path(key);
        let mut nonce = [0u8; 8];
        getrandom::fill(&mut nonce).map_err(|error| io::Error::other(error.to_string()))?;
        let staged = self.dir.join(format!(".{}.tmp", hex::encode(nonce)));
        let written = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&staged)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            fs::rename(&staged, &path)?;
            self.sync_dir()
        })();
        if written.is_err() {
            let _ = fs::remove_file(&staged);
        }
        written.map_err(|error| unavailable(&path, error))
    }

    fn delete(&self, key: &str) -> io::Result<()> {
        let _held = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        self.check_dir()?;
        let path = self.path(key);
        match fs::remove_file(&path) {
            Ok(()) => self.sync_dir(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(unavailable(&path, error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn records_are_owner_only_and_replaced_whole() {
        let root = tempfile::tempdir().unwrap();
        let store = FileStore::open(root.path()).unwrap();
        assert_eq!(store.get("chunk/1").unwrap(), None);
        store.set("chunk/1", b"first").unwrap();
        store.set("chunk/1", b"second").unwrap();
        assert_eq!(
            store.get("chunk/1").unwrap().as_deref(),
            Some(&b"second"[..])
        );
        assert_eq!(mode(&root.path().join(SECRETS_DIR)), 0o700);
        let files: Vec<_> = fs::read_dir(root.path().join(SECRETS_DIR))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        // One record, no staging file left behind, and the key never appears
        // as a path: `chunk/1` cannot make a subdirectory.
        assert_eq!(files.len(), 1);
        assert_eq!(mode(&files[0]), 0o600);
        assert!(!files[0].to_string_lossy().contains("chunk"));
        store.delete("chunk/1").unwrap();
        store.delete("chunk/1").unwrap();
        assert_eq!(store.get("chunk/1").unwrap(), None);
    }

    #[test]
    fn a_symlinked_record_is_refused_not_followed() {
        let root = tempfile::tempdir().unwrap();
        let store = FileStore::open(root.path()).unwrap();
        let elsewhere = root.path().join("elsewhere");
        fs::write(&elsewhere, b"not a secret").unwrap();
        std::os::unix::fs::symlink(&elsewhere, store.path("key")).unwrap();
        let refused = store.get("key").unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_directory_open_to_others_or_swapped_for_a_link_stops_the_store() {
        let root = tempfile::tempdir().unwrap();
        let store = FileStore::open(root.path()).unwrap();
        let dir = root.path().join(SECRETS_DIR);
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            store.set("key", b"x").unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(FileStore::open(root.path()).is_err());

        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        fs::rename(&dir, root.path().join("moved")).unwrap();
        std::os::unix::fs::symlink(root.path().join("moved"), &dir).unwrap();
        assert_eq!(
            store.get("key").unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn a_credential_round_trips_through_the_file_store() {
        // The same reference-and-chunks layer the keychain sits under.
        let root = tempfile::tempdir().unwrap();
        let store: std::sync::Arc<dyn SecretStore> =
            std::sync::Arc::new(FileStore::open(root.path()).unwrap());
        let files = super::super::CredentialFiles::new(root.path().to_path_buf(), store);
        fs::DirBuilder::new()
            .mode(0o700)
            .create(root.path().join("vault"))
            .unwrap();
        let record = files.file(root.path().join("vault").join("key.json"));
        let secret = vec![7u8; 5000];
        record.write(&secret).unwrap();
        assert_eq!(record.read().unwrap().as_deref(), Some(&secret[..]));
    }

    #[test]
    fn a_room_refuses_the_store_it_was_not_made_with() {
        use super::super::{Backend, claim_backend};
        let fresh = tempfile::tempdir().unwrap();
        claim_backend(fresh.path(), Backend::File).unwrap();
        claim_backend(fresh.path(), Backend::File).unwrap();
        let refused = claim_backend(fresh.path(), Backend::Native).unwrap_err();
        assert!(refused.to_string().contains("file store"));

        // A room from before the marker, with a vault, was the keychain's.
        let older = tempfile::tempdir().unwrap();
        fs::create_dir_all(older.path().join("vault")).unwrap();
        assert!(claim_backend(older.path(), Backend::File).is_err());
        assert!(!older.path().join(super::super::BACKEND_FILE).exists());
        claim_backend(older.path(), Backend::Native).unwrap();
        assert!(older.path().join(super::super::BACKEND_FILE).exists());
    }
}
