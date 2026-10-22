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
use crate::contract::{SecretUse, SharedSecret, SharedSecretKind};
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
/// shape on both sides of the wire; the sidecar supplies its current use
/// policy before `PUT /secrets` sends it. A variable is kept as its bare value, which is
/// what every record from before kinds is.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum StoredSecret {
    Variable {
        value: String,
    },
    #[serde(rename_all = "camelCase")]
    Login {
        #[serde(default)]
        r#use: SecretUse,
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
        #[serde(default)]
        r#use: SecretUse,
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

impl std::fmt::Debug for StoredSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredSecret")
            .field("kind", &self.kind())
            .finish_non_exhaustive()
    }
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
                r#use,
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
                    r#use,
                    sites,
                    username,
                    password,
                    totp,
                })
            }
            StoredSecret::Passkey {
                r#use,
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
                    r#use,
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
                r#use,
                ..
            } => Some(About::Login {
                r#use: *r#use,
                sites: sites.clone(),
                username: username.clone(),
                totp: totp.is_some(),
            }),
            StoredSecret::Passkey {
                rp_id,
                user_name,
                r#use,
                ..
            } => Some(About::Passkey {
                r#use: *r#use,
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
        #[serde(default)]
        r#use: SecretUse,
        sites: Vec<String>,
        username: String,
        totp: bool,
    },
    #[serde(rename_all = "camelCase")]
    Passkey {
        #[serde(default)]
        r#use: SecretUse,
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
            #[expect(
                clippy::case_sensitive_file_extension_comparisons,
                reason = "`.about` is the vault's own lowercase suffix, not an extension a person chose"
            )]
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
            r#use: SecretUse::Auto,
            sites: None,
            username: None,
            totp: None,
            rp_id: None,
            user_name: None,
        };
        match about {
            None => {}
            Some(About::Login {
                r#use,
                sites,
                username,
                totp,
            }) => {
                secret.r#use = r#use;
                secret.kind = SharedSecretKind::Login;
                secret.sites = Some(sites);
                secret.username = Some(username);
                secret.totp = Some(totp);
            }
            Some(About::Passkey {
                rp_id,
                user_name,
                r#use,
            }) => {
                secret.r#use = r#use;
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
            Some(about) => crate::credentials::atomic_write(
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

    /// Policy is metadata: changing it neither reads nor rewrites a private key.
    pub fn set_secret_use(&self, name: &str, usage: SecretUse) -> io::Result<()> {
        check_secret_name(name)?;
        let _one_writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        self.check_layout()?;
        if !self.shared_path(name).symlink_metadata()?.is_file() {
            return Err(io::Error::other(
                "The secret is not a regular vault record.",
            ));
        }
        let mut about: About =
            serde_json::from_reader(crate::credentials::private_read(&self.about_path(name))?)
                .map_err(io::Error::other)?;
        match &mut about {
            About::Login { r#use, .. } | About::Passkey { r#use, .. } => *r#use = usage,
        }
        crate::credentials::atomic_write(
            &self.about_path(name),
            &serde_json::to_vec(&about).map_err(io::Error::other)?,
        )
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
                Some(mut secret) => {
                    // The sidecar owns policy, so a metadata-only change reaches
                    // the next delivery without rewriting the keychain value.
                    match &mut secret {
                        StoredSecret::Login { r#use, .. } | StoredSecret::Passkey { r#use, .. } => {
                            *r#use =
                                match crate::credentials::private_read(&self.about_path(name)) {
                                    Ok(file) => match serde_json::from_reader::<_, About>(file)
                                        .map_err(io::Error::other)?
                                    {
                                        About::Login { r#use, .. }
                                        | About::Passkey { r#use, .. } => r#use,
                                    },
                                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                                        SecretUse::Auto
                                    }
                                    Err(error) => return Err(error),
                                };
                        }
                        StoredSecret::Variable { .. } => {}
                    }
                    values.insert(name.clone(), secret);
                }
                None => missing.push(name.clone()),
            }
        }
        Ok((values, missing))
    }
}

#[cfg(test)]
mod tests;
