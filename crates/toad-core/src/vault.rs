//! The vault: provider secrets on this machine's disk, and the room's record
//! that they exist.
//!
//! A secret is never an event. `<root>/vault/secrets.json` is a JSON map from
//! credential id to secret — a `0600` file in a `0700` directory, written
//! through a temporary file and a rename so no reader ever sees half of one —
//! and the room stream carries only the metadata: which provider, what the
//! user called it, whether it is revoked. That is why `list` and
//! `provider_keys` are two different questions. The room knows a credential
//! exists; only this disk knows what it is, which is what lets a stream be
//! read, copied or shipped without carrying a key along with it.
//!
//! The discipline is the previous Toad's `src/bun/store/credentials.ts`,
//! minus the fleet: nothing is replicated and nothing is sealed to another
//! desk, because there are no other desks here.

use crate::contract::{Credential, CredentialKind};
use crate::log::{Log, StreamId};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

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

    /// Takes both halves away: the secret off the disk, then the tombstone on
    /// the stream. Secret first, so a crash between them cannot leave a usable
    /// key behind a row that says it is gone.
    pub fn delete(&self, id: &str) -> io::Result<()> {
        let _one_writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        let credential = self.find(id)?;
        let mut secrets = self.read_secrets()?;
        if secrets.remove(&credential.id).is_some() {
            self.write_secrets(&secrets)?;
        }
        self.log.append(
            &StreamId::Room,
            &json!({ "kind": "credential", "id": credential.id, "deleted": true }),
        )?;
        Ok(())
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

    /// One usable API key per provider, keyed by provider id: the first
    /// credential created for a provider wins.
    ///
    /// A revoked row is not a key, and neither is a row whose secret is
    /// missing — that is what a `create` torn between its two writes looks
    /// like, and what a stream restored without its vault looks like. Handing
    /// the provider nothing at all beats handing it an empty string.
    pub fn provider_keys(&self) -> HashMap<String, String> {
        // A vault that cannot be read is no keys rather than a failure: the
        // place where refusing matters is the write, where a key could be lost.
        let secrets = self.read_secrets().unwrap_or_default();
        let mut keys = HashMap::new();
        for credential in self.list() {
            if credential.revoked {
                continue;
            }
            let Some(secret) = secrets.get(&credential.id) else {
                continue;
            };
            keys.entry(credential.provider_id)
                .or_insert_with(|| secret.clone());
        }
        keys
    }

    fn directory(&self) -> PathBuf {
        self.root.join("vault")
    }

    fn secrets_path(&self) -> PathBuf {
        self.directory().join("secrets.json")
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
        let mut file = create_private_file(&temporary)?;
        let text = serde_json::to_string_pretty(secrets).map_err(io::Error::other)?;
        file.write_all(format!("{text}\n").as_bytes())?;
        // The rename is only atomic over bytes that reached the disk.
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, self.secrets_path())
    }
}

/// The room event a credential is. `kind` leads, the way every event on the
/// stream does, and the rest is the credential exactly as the contract spells
/// it.
fn event(credential: &Credential) -> Value {
    let Ok(Value::Object(fields)) = serde_json::to_value(credential) else {
        unreachable!("a credential is a struct of strings, bools and numbers");
    };
    let mut event = serde_json::Map::new();
    event.insert("kind".to_string(), Value::from("credential"));
    event.extend(fields);
    Value::Object(event)
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
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
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
    /// which reads to `provider_keys` as a key the user never sees again.
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

        let keys = vault.provider_keys();
        assert_eq!(keys.len(), 8, "{keys:?}");
        for writer in 0..8 {
            assert_eq!(
                keys.get(&format!("provider-{writer}")).map(String::as_str),
                Some(format!("key-{writer}").as_str())
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
            vault.provider_keys().get("anthropic").map(String::as_str),
            Some("sk-ant-001")
        );

        vault.revoke(&credential.id).unwrap();
        assert!(vault.provider_keys().is_empty());
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
            vault.provider_keys(),
            HashMap::from([
                ("anthropic".to_string(), "sk-ant-first".to_string()),
                ("openai".to_string(), "sk-oai".to_string()),
            ])
        );

        vault.revoke(&first.id).unwrap();
        assert_eq!(
            vault.provider_keys().get("anthropic").map(String::as_str),
            Some("sk-ant-second")
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
        assert!(vault.provider_keys().is_empty());
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

        let keys = vault.provider_keys();
        assert_eq!(
            keys.get("anthropic").map(String::as_str),
            Some("sk-ant-001")
        );
        assert_eq!(keys.get("openai").map(String::as_str), Some("sk-oai"));
        assert!(!secrets(&vault).contains("sk-ghost"));
        assert!(!mine.exists());
    }
}
