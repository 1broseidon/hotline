//! Secrets the operator keeps for teammates to use without ever seeing them.
//!
//! A shared secret is a named value — `GITHUB_TOKEN`, `NPM_TOKEN` — kept in
//! the OS credential store through the same opaque-reference records that
//! hold provider keys and MCP tokens. The name is the environment variable a
//! teammate's computer finds the value under, so it is spelled like one. The
//! window stores and deletes by name and lists names; nothing on the wire, in
//! the room stream or in a subscription ever carries a value. The one reader
//! of a value is the grant that hands it to a computer, on its way there.
//!
//! One record per secret, `vault/shared/<NAME>.json`, so storing or taking
//! one never rewrites another, and the reference file's own modification
//! time is when the value last changed.

use super::*;
use crate::contract::SharedSecret;
use std::time::UNIX_EPOCH;

/// A value shorter than this is refused. The computer redacts every granted
/// value from what it answers the model, and a short one would blank out
/// ordinary words wherever they occurred.
const MIN_VALUE_CHARS: usize = 8;
const MAX_VALUE_BYTES: usize = 64 * 1024;
const MAX_NAME_CHARS: usize = 64;

/// Variables the shell, the display and the loader own. Setting one as a
/// secret would change how every job runs rather than hand it a credential.
const RESERVED: [&str; 12] = [
    "PATH",
    "HOME",
    "USER",
    "SHELL",
    "TERM",
    "LANG",
    "TZ",
    "DISPLAY",
    "PWD",
    "TMPDIR",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
];

/// A secret's name is the environment variable it becomes: capital letters,
/// digits and underscores, starting with a letter. Hotline's own variables
/// and the shell's are not for taking.
pub fn check_secret_name(name: &str) -> io::Result<()> {
    let spelled = !name.is_empty()
        && name.len() <= MAX_NAME_CHARS
        && name.starts_with(|c: char| c.is_ascii_uppercase())
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    if !spelled {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "A secret's name is the environment variable it becomes: capital letters, digits and underscores, starting with a letter — GITHUB_TOKEN, not github-token.",
        ));
    }
    if name.starts_with("HOTLINE_") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Names starting with HOTLINE_ are Hotline's own.",
        ));
    }
    if RESERVED.contains(&name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} is the shell's, not a secret's. Pick another name."),
        ));
    }
    Ok(())
}

fn check_secret_value(value: &str) -> io::Result<()> {
    if value.chars().count() < MIN_VALUE_CHARS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "A secret is at least 8 characters; anything shorter would be redacted out of ordinary words.",
        ));
    }
    if value.len() > MAX_VALUE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "A secret is at most 64 KiB.",
        ));
    }
    if value.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "A secret cannot contain a NUL byte.",
        ));
    }
    Ok(())
}

impl Vault {
    pub(super) fn shared_dir(&self) -> PathBuf {
        self.directory().join("shared")
    }

    fn shared_path(&self, name: &str) -> PathBuf {
        self.shared_dir().join(format!("{name}.json"))
    }

    /// Every stored secret by name, with when its value last changed, in name
    /// order. Never a value. A vault with no shared secrets, or no vault at
    /// all yet, is an empty list.
    pub fn shared_secrets(&self) -> io::Result<Vec<SharedSecret>> {
        self.check_layout()?;
        let entries = match fs::read_dir(self.shared_dir()) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut secrets = Vec::new();
        for entry in entries {
            let entry = entry?;
            // The listing does not follow a link, so one planted here is not
            // a secret whatever it is named.
            if !entry.file_type()?.is_file() {
                continue;
            }
            let file_name = entry.file_name();
            let Some(name) = file_name
                .to_str()
                .and_then(|file_name| file_name.strip_suffix(".json"))
            else {
                continue;
            };
            if check_secret_name(name).is_err() {
                continue;
            }
            let updated_at = entry
                .metadata()?
                .modified()?
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_millis() as i64);
            secrets.push(SharedSecret {
                name: name.to_owned(),
                updated_at,
            });
        }
        secrets.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(secrets)
    }

    /// Stores `value` under `name`, replacing what was there. The value goes
    /// to the OS store and only a reference to disk; a store that refuses
    /// leaves the previous value in place.
    pub fn set_shared_secret(&self, name: &str, value: &str) -> io::Result<()> {
        check_secret_name(name)?;
        check_secret_value(value)?;
        let _one_writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        self.check_layout()?;
        make_private_directory(&self.shared_dir())?;
        self.files
            .file(self.shared_path(name))
            .write(value.as_bytes())
    }

    /// Takes a secret away: its chunks out of the OS store and its reference
    /// off the disk. A name that is not stored is an error, so the window can
    /// say so rather than pretend.
    pub fn delete_shared_secret(&self, name: &str) -> io::Result<()> {
        check_secret_name(name)?;
        let _one_writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        self.check_layout()?;
        let path = self.shared_path(name);
        if path.symlink_metadata().is_err() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("There is no secret named {name}."),
            ));
        }
        self.files.file(path).delete()
    }

    /// The values behind `names`, for handing to a computer — the one way a
    /// value leaves the store — and the names among them that are not stored
    /// any more, so the teammate's tape can say so. A record this vault
    /// refuses to read is an error, not a missing name: the layout under it
    /// is not what this wrote.
    pub fn shared_secret_values(
        &self,
        names: &[String],
    ) -> io::Result<(BTreeMap<String, String>, Vec<String>)> {
        self.check_layout()?;
        let mut values = BTreeMap::new();
        let mut missing = Vec::new();
        for name in names {
            if check_secret_name(name).is_err() {
                missing.push(name.clone());
                continue;
            }
            match self.files.file(self.shared_path(name)).read()? {
                Some(bytes) => match String::from_utf8(bytes) {
                    Ok(value) => {
                        values.insert(name.clone(), value);
                    }
                    Err(_) => missing.push(name.clone()),
                },
                None => missing.push(name.clone()),
            }
        }
        Ok((values, missing))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::tests::MemoryStore;

    fn vault(root: &Path) -> Vault {
        Vault::open_with_store(root, Log::open(root), Arc::new(MemoryStore::default())).unwrap()
    }

    fn names(vault: &Vault) -> Vec<String> {
        vault
            .shared_secrets()
            .unwrap()
            .into_iter()
            .map(|secret| secret.name)
            .collect()
    }

    #[test]
    fn a_shared_secret_is_listed_by_name_and_never_by_value() {
        let root = tempfile::tempdir().unwrap();
        let vault = vault(root.path());
        assert!(names(&vault).is_empty(), "nothing stored yet");

        vault
            .set_shared_secret("GITHUB_TOKEN", "ghp_notarealtoken0001")
            .unwrap();
        let listed = vault.shared_secrets().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "GITHUB_TOKEN");
        assert!(listed[0].updated_at > 0);

        // The disk holds a reference; the value is in the store alone.
        let on_disk =
            fs::read_to_string(root.path().join("vault/shared/GITHUB_TOKEN.json")).unwrap();
        assert!(!on_disk.contains("ghp_notarealtoken0001"), "{on_disk}");
        assert!(on_disk.contains("hotlineCredential"), "{on_disk}");
        let room = fs::read_to_string(root.path().join("room.jsonl")).unwrap_or_default();
        assert!(!room.contains("GITHUB_TOKEN"), "no room event: {room}");

        let (values, missing) = vault
            .shared_secret_values(&["GITHUB_TOKEN".to_string(), "NPM_TOKEN".to_string()])
            .unwrap();
        assert_eq!(values["GITHUB_TOKEN"], "ghp_notarealtoken0001");
        assert_eq!(missing, ["NPM_TOKEN"]);
    }

    #[test]
    fn a_name_is_an_environment_variable_and_hotlines_own_are_refused() {
        for good in ["GITHUB_TOKEN", "A", "X1_2_3", "NPM_TOKEN_2026"] {
            check_secret_name(good).unwrap_or_else(|error| panic!("{good}: {error}"));
        }
        for bad in [
            "",
            "github_token",
            "Github_Token",
            "1TOKEN",
            "_TOKEN",
            "GITHUB-TOKEN",
            "GITHUB TOKEN",
            "TOKEN=1",
            "../ESCAPE",
            "HOTLINE_COMPUTER_TOKEN",
            "HOTLINE_ANYTHING",
            "PATH",
            "HOME",
            "LD_PRELOAD",
        ] {
            assert!(check_secret_name(bad).is_err(), "{bad:?} is not a name");
        }
        let root = tempfile::tempdir().unwrap();
        let vault = vault(root.path());
        assert!(vault.set_shared_secret("PATH", "/usr/bin:/bin").is_err());
        assert!(vault.delete_shared_secret("../x").is_err());
        assert!(names(&vault).is_empty());
    }

    #[test]
    fn a_value_is_at_least_eight_characters() {
        let root = tempfile::tempdir().unwrap();
        let vault = vault(root.path());
        let refused = vault.set_shared_secret("PIN", "1234").unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::InvalidInput);
        assert!(refused.to_string().contains("at least 8"), "{refused}");
        assert!(vault.set_shared_secret("NUL", "abc\0defgh").is_err());
        assert!(
            vault
                .set_shared_secret("KEY", "-----BEGIN KEY-----\nline\n-----END KEY-----\n")
                .is_ok(),
            "a multi-line value, such as a private key, is a value"
        );
    }

    #[test]
    fn replacing_keeps_one_record_and_deleting_takes_it() {
        let root = tempfile::tempdir().unwrap();
        let vault = vault(root.path());
        vault
            .set_shared_secret("NPM_TOKEN", "npm_first_value")
            .unwrap();
        vault
            .set_shared_secret("NPM_TOKEN", "npm_second_value")
            .unwrap();
        assert_eq!(names(&vault), ["NPM_TOKEN"]);
        let (values, _) = vault
            .shared_secret_values(&["NPM_TOKEN".to_string()])
            .unwrap();
        assert_eq!(values["NPM_TOKEN"], "npm_second_value");

        vault.delete_shared_secret("NPM_TOKEN").unwrap();
        assert!(names(&vault).is_empty());
        assert!(!root.path().join("vault/shared/NPM_TOKEN.json").exists());
        let (values, missing) = vault
            .shared_secret_values(&["NPM_TOKEN".to_string()])
            .unwrap();
        assert!(values.is_empty());
        assert_eq!(missing, ["NPM_TOKEN"]);
        assert_eq!(
            vault.delete_shared_secret("NPM_TOKEN").unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_shared_directory_and_its_records_are_private_and_a_planted_link_is_not_a_secret() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let vault = vault(root.path());
        vault
            .set_shared_secret("GITHUB_TOKEN", "ghp_notarealtoken0001")
            .unwrap();
        let mode = |path: PathBuf| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(vault.shared_dir()), 0o700);
        assert_eq!(mode(vault.shared_dir().join("GITHUB_TOKEN.json")), 0o600);

        // A link where a record should be is refused rather than followed,
        // and it is not listed as a secret either.
        let elsewhere = root.path().join("elsewhere.json");
        fs::write(&elsewhere, "{}").unwrap();
        std::os::unix::fs::symlink(&elsewhere, vault.shared_dir().join("PLANTED.json")).unwrap();
        assert_eq!(names(&vault), ["GITHUB_TOKEN"]);
        assert!(
            vault
                .shared_secret_values(&["PLANTED".to_string()])
                .is_err(),
            "a planted link is refused, not read"
        );
    }
}
