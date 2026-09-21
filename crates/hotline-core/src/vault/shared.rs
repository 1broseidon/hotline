//! Secrets the operator keeps for teammates to use without ever seeing them.
//!
//! A shared secret is named like an environment variable — `GITHUB_TOKEN`,
//! `GITHUB`, `GITHUB_PASSKEY` — and kept in the OS credential store through
//! the same opaque-reference records that hold provider keys and MCP tokens.
//! It has a kind, and the kind says where a teammate's computer puts it: a
//! **variable** into the environment of every job; a **login** — sites, a
//! username, a password, perhaps a TOTP seed — typed field by field into a
//! sign-in form on one of its own sites and nowhere else; a **passkey** — a
//! WebAuthn credential the computer's browser made under the operator's
//! arming — into the browser's authenticator, where it signs in by itself.
//! The window stores and deletes by name and lists names, kinds, sites and
//! usernames; nothing on the wire, in the room stream or in a subscription
//! ever carries a value. The one reader of a value is the grant that hands
//! it to a computer, on its way there.
//!
//! One record per secret, `vault/shared/<NAME>.json`, so storing or taking
//! one never rewrites another, and the reference file's own modification
//! time is when the value last changed. A login or a passkey also has
//! `vault/shared/<NAME>.about.json` beside it: its kind and what the
//! listing says of it — sites and username, or site and account name —
//! which is identity, not a secret, so the list never opens the store.

use super::*;
use crate::contract::{SharedSecret, SharedSecretKind};
use std::time::UNIX_EPOCH;

/// A value shorter than this is refused. The computer redacts every granted
/// value from what it answers the model, and a short one would blank out
/// ordinary words wherever they occurred.
const MIN_VALUE_CHARS: usize = 8;
const MAX_VALUE_BYTES: usize = 64 * 1024;
const MAX_NAME_CHARS: usize = 64;
const MAX_SITES: usize = 16;
const MAX_USERNAME_BYTES: usize = 1024;
/// A TOTP seed shorter than this is refused: issuers hand out at least 80
/// bits, which is sixteen base32 characters.
const MIN_SEED_CHARS: usize = 16;

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

/// A stored secret as the keychain holds it and a computer takes it: the
/// same JSON on both sides of the wire, so what the vault reads is what
/// `PUT /secrets` sends. A variable is kept as its bare value, which is
/// what every record from before kinds is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum StoredSecret {
    Variable {
        value: String,
    },
    #[serde(rename_all = "camelCase")]
    Login {
        /// Origins, `https://host[:port]`, or `http://` on localhost.
        sites: Vec<String>,
        username: String,
        password: String,
        /// A base32 TOTP seed, when the sign-in asks for a code.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        totp: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Passkey {
        rp_id: String,
        credential_id: String,
        /// PKCS#8, base64; the one part that is a secret.
        private_key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_handle: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_display_name: Option<String>,
    },
}

impl StoredSecret {
    pub fn kind(&self) -> SharedSecretKind {
        match self {
            StoredSecret::Variable { .. } => SharedSecretKind::Variable,
            StoredSecret::Login { .. } => SharedSecretKind::Login,
            StoredSecret::Passkey { .. } => SharedSecretKind::Passkey,
        }
    }

    /// The record as it is kept, or the refusal. Sites, seeds and names come
    /// out in one spelling, so the computer's origin rule and the person's
    /// listing agree whichever way they were typed.
    fn checked(self, name: &str) -> io::Result<StoredSecret> {
        let refuse = |text: String| io::Error::new(io::ErrorKind::InvalidInput, text);
        match self {
            StoredSecret::Variable { value } => {
                check_secret_value(&value)?;
                Ok(StoredSecret::Variable { value })
            }
            StoredSecret::Login {
                sites,
                username,
                password,
                totp,
            } => {
                let sites: Vec<String> = sites
                    .iter()
                    .map(|site| site.trim())
                    .filter(|site| !site.is_empty())
                    .map(check_site)
                    .collect::<io::Result<_>>()?;
                if sites.is_empty() {
                    return Err(refuse(format!(
                        "{name} needs at least one site: the origin its sign-in form is on, such as https://github.com. A login is typed there and nowhere else."
                    )));
                }
                if sites.len() > MAX_SITES {
                    return Err(refuse(format!("{name} names more than {MAX_SITES} sites.")));
                }
                let username = username.trim().to_owned();
                if username.is_empty()
                    || username.len() > MAX_USERNAME_BYTES
                    || username.contains(['\n', '\r', '\0'])
                {
                    return Err(refuse(format!("{name} needs a username, on one line.")));
                }
                check_secret_value(&password)
                    .map_err(|error| refuse(format!("{name}'s password: {error}")))?;
                let totp = totp
                    .as_deref()
                    .map(str::trim)
                    .filter(|seed| !seed.is_empty())
                    .map(check_totp_seed)
                    .transpose()?;
                Ok(StoredSecret::Login {
                    sites,
                    username,
                    password,
                    totp,
                })
            }
            StoredSecret::Passkey {
                rp_id,
                credential_id,
                private_key,
                user_handle,
                user_name,
                user_display_name,
            } => {
                check_rp_id(&rp_id)?;
                let bytes = |field: &str, text: &str| {
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, text)
                        .map_err(|_| refuse(format!("{name}'s {field} is not base64.")))
                };
                if bytes("credentialId", &credential_id)?.is_empty() {
                    return Err(refuse(format!("{name} has no credential id.")));
                }
                if bytes("privateKey", &private_key)?.len() < 32 {
                    return Err(refuse(format!(
                        "{name}'s private key is too short to be one."
                    )));
                }
                if let Some(handle) = &user_handle {
                    bytes("userHandle", handle)?;
                }
                Ok(StoredSecret::Passkey {
                    rp_id,
                    credential_id,
                    private_key,
                    user_handle,
                    user_name,
                    user_display_name,
                })
            }
        }
    }

    /// What the listing says of it beside the name: never a value.
    fn about(&self) -> Option<About> {
        match self {
            StoredSecret::Variable { .. } => None,
            StoredSecret::Login {
                sites,
                username,
                totp,
                ..
            } => Some(About::Login {
                sites: sites.clone(),
                username: username.clone(),
                totp: totp.is_some(),
            }),
            StoredSecret::Passkey {
                rp_id, user_name, ..
            } => Some(About::Passkey {
                rp_id: rp_id.clone(),
                user_name: user_name.clone(),
            }),
        }
    }

    /// The bytes the keychain keeps: a variable's bare value, so a record
    /// from before kinds reads the same; anything else as JSON.
    fn bytes(&self) -> io::Result<Vec<u8>> {
        match self {
            StoredSecret::Variable { value } => Ok(value.as_bytes().to_vec()),
            typed => serde_json::to_vec(typed).map_err(io::Error::other),
        }
    }

    fn from_bytes(bytes: Vec<u8>) -> Option<StoredSecret> {
        let value = String::from_utf8(bytes).ok()?;
        if value.starts_with('{')
            && let Ok(typed) = serde_json::from_str::<StoredSecret>(&value)
        {
            return Some(typed);
        }
        Some(StoredSecret::Variable { value })
    }
}

/// The sidecar beside a login's or a passkey's reference: the part of the
/// record the listing shows. A variable has none.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum About {
    Login {
        sites: Vec<String>,
        username: String,
        totp: bool,
    },
    #[serde(rename_all = "camelCase")]
    Passkey {
        rp_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_name: Option<String>,
    },
}

/// A secret's name is the environment variable it becomes: capital letters,
/// digits and underscores, starting with a letter. Hotline's own variables
/// and the shell's are not for taking. Every kind is named the same way, so
/// one list holds them all and a computer's `state info` reads as one.
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

/// A login's site as the computer will check it: an `https://` origin —
/// scheme and host, a port if not the default, nothing after — or `http://`
/// on localhost alone. Answers it in one spelling.
pub fn check_site(site: &str) -> io::Result<String> {
    let refuse = |text: String| io::Error::new(io::ErrorKind::InvalidInput, text);
    let site = site.trim();
    let (scheme, rest) = site.split_once("://").ok_or_else(|| {
        refuse(format!(
            "A site is an origin such as https://github.com, not {site:?}."
        ))
    })?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "https" && scheme != "http" {
        return Err(refuse(format!("A site is an https address, not {site:?}.")));
    }
    let (authority, tail) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
    if !matches!(tail, "" | "/") {
        return Err(refuse(format!(
            "A site is the origin alone, with nothing after the host: {scheme}://{authority}, not {site:?}."
        )));
    }
    if authority.contains('@') {
        return Err(refuse(format!("A site carries no user name: {site:?}.")));
    }
    let default_port: u16 = if scheme == "https" { 443 } else { 80 };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if host.ends_with(']') || !host.contains(':') => {
            let port: u16 = port
                .parse()
                .ok()
                .filter(|port| *port != 0)
                .ok_or_else(|| refuse(format!("{site:?} has no such port.")))?;
            (host, port)
        }
        _ => (authority, default_port),
    };
    let host = host.to_ascii_lowercase();
    let bracketed = host.starts_with('[') && host.ends_with(']');
    let named = !host.is_empty()
        && !host.starts_with('.')
        && !host.ends_with('.')
        && host
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-');
    if !(named || bracketed) {
        return Err(refuse(format!("{site:?} has no host.")));
    }
    let local = host == "localhost"
        || host.ends_with(".localhost")
        || host == "127.0.0.1"
        || host == "[::1]";
    if scheme == "http" && !local {
        return Err(refuse(format!(
            "{site:?} is plain http. A login goes to an https site, or to http on localhost alone."
        )));
    }
    Ok(if port == default_port {
        format!("{scheme}://{host}")
    } else {
        format!("{scheme}://{host}:{port}")
    })
}

/// A passkey's site is a relying party id: a host name in lower case, such
/// as `github.com`, never an address with a scheme, a port or a path.
pub fn check_rp_id(rp_id: &str) -> io::Result<()> {
    let shaped = !rp_id.is_empty()
        && rp_id.len() <= 253
        && !rp_id.starts_with('.')
        && !rp_id.ends_with('.')
        && !rp_id.contains("..")
        && rp_id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-');
    if shaped {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "A passkey's site is a host name in lower case, such as github.com — not {rp_id:?}."
            ),
        ))
    }
}

/// A TOTP seed in one spelling: base32 in capitals, without the spaces,
/// dashes and padding issuers print it with.
fn check_totp_seed(seed: &str) -> io::Result<String> {
    let cleaned: String = seed
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '=')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if cleaned.len() < MIN_SEED_CHARS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "A TOTP seed has at least 16 base32 characters: the letters A to Z and the digits 2 to 7, as the site shows it when setting up an authenticator app.",
        ));
    }
    if !cleaned
        .chars()
        .all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "A TOTP seed is base32: the letters A to Z and the digits 2 to 7, as the site shows it when setting up an authenticator app.",
        ));
    }
    Ok(cleaned)
}

impl Vault {
    pub(super) fn shared_dir(&self) -> PathBuf {
        self.directory().join("shared")
    }

    fn shared_path(&self, name: &str) -> PathBuf {
        self.shared_dir().join(format!("{name}.json"))
    }

    fn about_path(&self, name: &str) -> PathBuf {
        self.shared_dir().join(format!("{name}.about.json"))
    }

    /// Every stored secret by name, with its kind, what the listing may say
    /// of it, and when its value last changed, in name order. Never a value,
    /// and never a look into the OS store. A vault with no shared secrets,
    /// or no vault at all yet, is an empty list.
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
                .filter(|name| !name.ends_with(".about"))
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
            secrets.push(self.describe(name, updated_at)?);
        }
        secrets.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(secrets)
    }

    /// The listing's row for one record: its sidecar, if it has one, folded
    /// onto the name. A sidecar this vault cannot read is a variable's
    /// absence, not an error: the value behind it is still there to hand
    /// over, and the person can replace the record.
    fn describe(&self, name: &str, updated_at: i64) -> io::Result<SharedSecret> {
        let about = match fs::symlink_metadata(self.about_path(name)) {
            Ok(metadata) if metadata.is_file() => fs::read(self.about_path(name))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<About>(&bytes).ok()),
            _ => None,
        };
        let mut secret = SharedSecret {
            name: name.to_owned(),
            updated_at,
            kind: SharedSecretKind::Variable,
            sites: None,
            username: None,
            totp: None,
            rp_id: None,
            user_name: None,
        };
        match about {
            None => {}
            Some(About::Login {
                sites,
                username,
                totp,
            }) => {
                secret.kind = SharedSecretKind::Login;
                secret.sites = Some(sites);
                secret.username = Some(username);
                secret.totp = Some(totp);
            }
            Some(About::Passkey { rp_id, user_name }) => {
                secret.kind = SharedSecretKind::Passkey;
                secret.rp_id = Some(rp_id);
                secret.user_name = user_name;
            }
        }
        Ok(secret)
    }

    /// Stores a variable `value` under `name`, replacing what was there
    /// whatever its kind was.
    pub fn set_shared_secret(&self, name: &str, value: &str) -> io::Result<()> {
        self.set_shared(
            name,
            StoredSecret::Variable {
                value: value.to_owned(),
            },
        )
    }

    /// Stores `secret` under `name`, replacing what was there. The value goes
    /// to the OS store and only a reference to disk, beside what the listing
    /// may say of it; a store that refuses leaves the previous record in
    /// place.
    pub fn set_shared(&self, name: &str, secret: StoredSecret) -> io::Result<()> {
        check_secret_name(name)?;
        let secret = secret.checked(name)?;
        let _one_writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        self.check_layout()?;
        make_private_directory(&self.shared_dir())?;
        self.files
            .file(self.shared_path(name))
            .write(&secret.bytes()?)?;
        match secret.about() {
            Some(about) => write_private(
                &self.about_path(name),
                &serde_json::to_vec(&about).map_err(io::Error::other)?,
            ),
            None => match fs::remove_file(self.about_path(name)) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            },
        }
    }

    /// Takes a secret away: its chunks out of the OS store and its records
    /// off the disk. A name that is not stored is an error, so the window
    /// can say so rather than pretend.
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
        self.files.file(path).delete()?;
        match fs::remove_file(self.about_path(name)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// The records behind `names`, for handing to a computer — the one way a
    /// value leaves the store — and the names among them that are not stored
    /// any more, so the teammate's tape can say so. A record this vault
    /// refuses to read is an error, not a missing name: the layout under it
    /// is not what this wrote.
    pub fn shared_secret_values(
        &self,
        names: &[String],
    ) -> io::Result<(BTreeMap<String, StoredSecret>, Vec<String>)> {
        self.check_layout()?;
        let mut values = BTreeMap::new();
        let mut missing = Vec::new();
        for name in names {
            if check_secret_name(name).is_err() {
                missing.push(name.clone());
                continue;
            }
            match self
                .files
                .file(self.shared_path(name))
                .read()?
                .and_then(StoredSecret::from_bytes)
            {
                Some(secret) => {
                    values.insert(name.clone(), secret);
                }
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

    fn login(sites: &[&str], totp: Option<&str>) -> StoredSecret {
        StoredSecret::Login {
            sites: sites.iter().map(|site| (*site).to_owned()).collect(),
            username: "george".to_owned(),
            password: "correct horse battery".to_owned(),
            totp: totp.map(str::to_owned),
        }
    }

    pub(crate) const KEY_BASE64: &str = "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgeE7T2PDJPCRfPvTUFvVcQI7KFSDmnKbrRfEjRGbtV9WhRANCAATfeasaWHkMKJ4oCcdDzVX9c2xUUkC7Uiuqu8tS0LXtRJ8pCk+gNSvvqWaB3WgFNn4rvQ8wS1bH+dOjfgZoq2gz";

    pub(crate) fn passkey(rp_id: &str) -> StoredSecret {
        StoredSecret::Passkey {
            rp_id: rp_id.to_owned(),
            credential_id: "AQID".to_owned(),
            private_key: KEY_BASE64.to_owned(),
            user_handle: Some("dGVhbW1hdGUtMQ==".to_owned()),
            user_name: Some("teammate".to_owned()),
            user_display_name: None,
        }
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
        assert_eq!(listed[0].kind, SharedSecretKind::Variable);
        assert!(listed[0].updated_at > 0);

        // The disk holds a reference; the value is in the store alone.
        let on_disk =
            fs::read_to_string(root.path().join("vault/shared/GITHUB_TOKEN.json")).unwrap();
        assert!(!on_disk.contains("ghp_notarealtoken0001"), "{on_disk}");
        assert!(on_disk.contains("hotlineCredential"), "{on_disk}");
        assert!(
            !root
                .path()
                .join("vault/shared/GITHUB_TOKEN.about.json")
                .exists()
        );
        let room = fs::read_to_string(root.path().join("room.jsonl")).unwrap_or_default();
        assert!(!room.contains("GITHUB_TOKEN"), "no room event: {room}");

        let (values, missing) = vault
            .shared_secret_values(&["GITHUB_TOKEN".to_string(), "NPM_TOKEN".to_string()])
            .unwrap();
        assert_eq!(
            values["GITHUB_TOKEN"],
            StoredSecret::Variable {
                value: "ghp_notarealtoken0001".to_owned()
            }
        );
        assert_eq!(missing, ["NPM_TOKEN"]);
    }

    #[test]
    fn a_login_and_a_passkey_are_listed_by_what_they_are_for_and_never_by_value() {
        let root = tempfile::tempdir().unwrap();
        let vault = vault(root.path());
        vault
            .set_shared(
                "GITHUB",
                StoredSecret::Login {
                    sites: vec![
                        "https://GitHub.com/".to_owned(),
                        " http://localhost:8123 ".to_owned(),
                        "".to_owned(),
                    ],
                    username: " george ".to_owned(),
                    password: "correct horse battery".to_owned(),
                    totp: Some("gezd gnbv-gy3t qojq gezd gnbv gy3t qojq".to_owned()),
                },
            )
            .unwrap();
        vault
            .set_shared("GITHUB_PASSKEY", passkey("github.com"))
            .unwrap();
        vault
            .set_shared_secret("GITHUB_TOKEN", "ghp_notarealtoken0001")
            .unwrap();

        let listed = vault.shared_secrets().unwrap();
        let by_name = |name: &str| listed.iter().find(|one| one.name == name).unwrap().clone();
        let github = by_name("GITHUB");
        assert_eq!(github.kind, SharedSecretKind::Login);
        assert_eq!(
            github.sites.as_deref(),
            Some(
                &[
                    "https://github.com".to_owned(),
                    "http://localhost:8123".to_owned()
                ][..]
            )
        );
        assert_eq!(github.username.as_deref(), Some("george"));
        assert_eq!(github.totp, Some(true));
        let passkey = by_name("GITHUB_PASSKEY");
        assert_eq!(passkey.kind, SharedSecretKind::Passkey);
        assert_eq!(passkey.rp_id.as_deref(), Some("github.com"));
        assert_eq!(passkey.user_name.as_deref(), Some("teammate"));
        assert_eq!(by_name("GITHUB_TOKEN").kind, SharedSecretKind::Variable);
        let listing = serde_json::to_string(&listed).unwrap();
        assert!(!listing.contains("battery"), "{listing}");
        assert!(!listing.contains("GEZDGNBV"), "{listing}");
        assert!(!listing.contains(KEY_BASE64), "{listing}");

        // The sidecar says what the listing says, and nothing more.
        let about = fs::read_to_string(root.path().join("vault/shared/GITHUB.about.json")).unwrap();
        assert!(about.contains("\"username\":\"george\""), "{about}");
        assert!(!about.contains("battery"), "{about}");
        assert!(!about.contains("GEZDGNBV"), "{about}");
        let about =
            fs::read_to_string(root.path().join("vault/shared/GITHUB_PASSKEY.about.json")).unwrap();
        assert!(!about.contains(KEY_BASE64), "{about}");

        // What is handed to a computer is the whole record, in one spelling.
        let (values, missing) = vault
            .shared_secret_values(&["GITHUB".to_string(), "GITHUB_PASSKEY".to_string()])
            .unwrap();
        assert!(missing.is_empty());
        assert_eq!(
            values["GITHUB"],
            StoredSecret::Login {
                sites: vec![
                    "https://github.com".to_owned(),
                    "http://localhost:8123".to_owned()
                ],
                username: "george".to_owned(),
                password: "correct horse battery".to_owned(),
                totp: Some("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ".to_owned()),
            }
        );
        assert_eq!(
            values["GITHUB_PASSKEY"],
            super::tests::passkey("github.com")
        );

        // Replacing a login with a variable takes its sidecar away.
        vault
            .set_shared_secret("GITHUB", "now-a-variable-value")
            .unwrap();
        assert_eq!(
            by_name("GITHUB").kind,
            SharedSecretKind::Login,
            "the earlier listing"
        );
        assert_eq!(
            vault
                .shared_secrets()
                .unwrap()
                .iter()
                .find(|one| one.name == "GITHUB")
                .unwrap()
                .kind,
            SharedSecretKind::Variable
        );
        assert!(!root.path().join("vault/shared/GITHUB.about.json").exists());
        vault.delete_shared_secret("GITHUB_PASSKEY").unwrap();
        assert!(
            !root
                .path()
                .join("vault/shared/GITHUB_PASSKEY.about.json")
                .exists()
        );
        assert_eq!(names(&vault), ["GITHUB", "GITHUB_TOKEN"]);
    }

    #[test]
    fn a_login_needs_a_site_of_its_own_and_a_passkey_a_key() {
        let root = tempfile::tempdir().unwrap();
        let vault = vault(root.path());
        for (what, secret, expect) in [
            ("no site", login(&[], None), "at least one site"),
            (
                "a site with a path",
                login(&["https://github.com/login"], None),
                "nothing after the host",
            ),
            (
                "plain http",
                login(&["http://github.com"], None),
                "plain http",
            ),
            (
                "no scheme",
                login(&["github.com"], None),
                "an origin such as",
            ),
            (
                "a short seed",
                login(&["https://github.com"], Some("GEZDGNBV")),
                "at least 16",
            ),
            (
                "a seed that is not base32",
                login(&["https://github.com"], Some("GEZDGNBVGY3TQOJQ1890")),
                "base32",
            ),
            ("a passkey on no site", passkey(""), "host name"),
            (
                "a passkey on an address",
                passkey("https://github.com"),
                "host name",
            ),
        ] {
            let refused = vault.set_shared("LOGIN", secret).unwrap_err();
            assert_eq!(refused.kind(), io::ErrorKind::InvalidInput, "{what}");
            assert!(refused.to_string().contains(expect), "{what}: {refused}");
        }
        let short = StoredSecret::Login {
            sites: vec!["https://github.com".to_owned()],
            username: "george".to_owned(),
            password: "hunter2".to_owned(),
            totp: None,
        };
        let refused = vault.set_shared("LOGIN", short).unwrap_err();
        assert!(
            refused.to_string().contains("LOGIN's password"),
            "{refused}"
        );
        let nobody = StoredSecret::Login {
            sites: vec!["https://github.com".to_owned()],
            username: "  ".to_owned(),
            password: "correct horse battery".to_owned(),
            totp: None,
        };
        assert!(vault.set_shared("LOGIN", nobody).is_err());
        let StoredSecret::Passkey {
            rp_id,
            credential_id,
            user_handle,
            user_name,
            user_display_name,
            ..
        } = passkey("github.com")
        else {
            unreachable!()
        };
        let keyless = StoredSecret::Passkey {
            rp_id,
            credential_id,
            private_key: "AQID".to_owned(),
            user_handle,
            user_name,
            user_display_name,
        };
        assert!(
            vault
                .set_shared("LOGIN", keyless)
                .unwrap_err()
                .to_string()
                .contains("too short")
        );
        assert!(names(&vault).is_empty(), "nothing refused was stored");
        for site in [
            "https://github.com",
            "https://GitHub.com:443/",
            "https://[::1]:8443",
            "http://localhost:8123",
            "http://app.localhost",
        ] {
            check_site(site).unwrap_or_else(|error| panic!("{site}: {error}"));
        }
        assert_eq!(
            check_site("https://GitHub.com:443/").unwrap(),
            "https://github.com"
        );
        assert_eq!(
            check_site("https://[::1]:8443").unwrap(),
            "https://[::1]:8443"
        );
        for rp in ["github.com", "localhost", "login.example.co.uk"] {
            check_rp_id(rp).unwrap_or_else(|error| panic!("{rp}: {error}"));
        }
        for rp in [
            "",
            "GitHub.com",
            "github.com/",
            "github.com:443",
            ".github.com",
            "a..b",
        ] {
            assert!(check_rp_id(rp).is_err(), "{rp:?}");
        }
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
        // A variable that happens to look like a record is still a variable.
        vault
            .set_shared_secret(
                "JSON",
                r#"{"kind":"login","sites":[],"username":"","password":""}"#,
            )
            .unwrap();
        let (values, _) = vault.shared_secret_values(&["JSON".to_string()]).unwrap();
        assert!(
            matches!(&values["JSON"], StoredSecret::Login { .. }),
            "a value that parses as a record is taken as one; the desk never stores such a variable through the window, which types it"
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
        assert_eq!(
            values["NPM_TOKEN"],
            StoredSecret::Variable {
                value: "npm_second_value".to_owned()
            }
        );

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
        vault
            .set_shared("GITHUB", login(&["https://github.com"], None))
            .unwrap();
        let mode = |path: PathBuf| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(vault.shared_dir()), 0o700);
        assert_eq!(mode(vault.shared_dir().join("GITHUB_TOKEN.json")), 0o600);
        assert_eq!(mode(vault.shared_dir().join("GITHUB.about.json")), 0o600);

        // A link where a record should be is refused rather than followed,
        // and it is not listed as a secret either.
        let elsewhere = root.path().join("elsewhere.json");
        fs::write(&elsewhere, "{}").unwrap();
        std::os::unix::fs::symlink(&elsewhere, vault.shared_dir().join("PLANTED.json")).unwrap();
        std::os::unix::fs::symlink(
            &elsewhere,
            vault.shared_dir().join("GITHUB_TOKEN.about.json"),
        )
        .unwrap();
        assert_eq!(names(&vault), ["GITHUB", "GITHUB_TOKEN"]);
        assert_eq!(
            vault.shared_secrets().unwrap()[1].kind,
            SharedSecretKind::Variable,
            "a planted sidecar is not read"
        );
        assert!(
            vault
                .shared_secret_values(&["PLANTED".to_string()])
                .is_err(),
            "a planted link is refused, not read"
        );
    }
}
