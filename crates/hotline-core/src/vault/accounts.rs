//! A credential as an account: which one is its provider's default, the auth
//! a chosen one gives a turn, its name and section, and the changes that keep
//! its id, so the default and the teammates set to it never move.

use super::*;
use crate::contract::AccountSection;
use crate::driver::failure::ACCOUNT_SIGNED_OUT;
use crate::providers::rig_login::{self, COPILOT_RECORD};
use hotline_room::provider_auth::ChosenAccount;

pub(crate) const CANNOT_CHANGE: &str = "This account cannot be changed that way.";
pub(crate) const CANNOT_REPLACE: &str = "This sign-in cannot replace that account's.";

pub(crate) struct CredentialChange {
    pub label: Option<String>,
    pub secret: Option<String>,
    pub base_url: Option<(String, Vec<String>)>,
}

impl Vault {
    pub(crate) fn default_ids(&self) -> HashMap<String, String> {
        self.connections()
            .into_iter()
            .map(|(provider_id, (credential, _))| (provider_id, credential.id))
            .collect()
    }

    pub(crate) fn default_account(&self, provider_id: &str) -> Option<(String, bool)> {
        let events = self.log.load(&StreamId::Room);
        let (credential, _) = self
            .connections_from(&events, &self.read_secrets())
            .remove(provider_id)?;
        let live = Self::credentials(&events)
            .iter()
            .filter(|credential| credential.provider_id == provider_id && !credential.revoked)
            .count();
        Some((credential.id, live > 1))
    }

    pub fn provider_auth_for(
        &self,
        account: Option<&ChosenAccount>,
    ) -> HashMap<String, ProviderAuth> {
        let events = self.log.load(&StreamId::Room);
        let secrets = self.read_secrets();
        let mut auth: HashMap<String, ProviderAuth> = self
            .connections_from(&events, &secrets)
            .into_iter()
            .map(|(provider_id, (_, auth))| (provider_id, auth))
            .collect();
        if let Some(account) = account {
            auth.insert(
                account.provider_id.clone(),
                self.chosen_auth(&events, &secrets, account),
            );
        }
        auth
    }

    fn chosen_auth(
        &self,
        events: &[Value],
        secrets: &io::Result<BTreeMap<String, String>>,
        account: &ChosenAccount,
    ) -> ProviderAuth {
        let signed_out = || ProviderAuth::Unavailable(ACCOUNT_SIGNED_OUT.into());
        let Some(credential) = Self::credentials(events).into_iter().find(|credential| {
            credential.id == account.id
                && credential.provider_id == account.provider_id
                && !credential.revoked
        }) else {
            return signed_out();
        };
        match self.credential_auth(&credential, secrets) {
            Ok(Some(auth)) => auth,
            Ok(None) => signed_out(),
            Err(error) => ProviderAuth::Unavailable(error.to_string()),
        }
    }

    /// The new event keeps the credential's place in the stream, which is
    /// what keeps it its provider's default.
    pub(crate) fn update(&self, id: &str, change: CredentialChange) -> io::Result<Credential> {
        let _one_writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        let mut credential = self.find(id)?;
        let custom = credential.custom.is_some();
        let fits = |kind: CredentialKind| !custom && credential.credential_kind == kind;
        let empty = change.label.is_none() && change.secret.is_none() && change.base_url.is_none();
        if empty
            || (change.secret.is_some() && !fits(CredentialKind::ApiKey))
            || (change.base_url.is_some() && !fits(CredentialKind::Local))
        {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, CANNOT_CHANGE));
        }
        if let Some(secret) = change.secret {
            let mut secrets = self.read_secrets()?;
            secrets.insert(credential.id.clone(), secret);
            self.write_secrets(&secrets)?;
        }
        if let Some((base_url, ids)) = change.base_url {
            self.check_layout()?;
            let dir = self.login_dir(&credential.id);
            make_private_directory(&dir)?;
            write_account_models(&dir, &ids)?;
            self.forget_discovery(&credential.id)?;
            credential.base_url = Some(base_url);
        }
        if let Some(label) = change.label {
            credential.label = label;
        }
        credential.updated_at = now_ms();
        self.log.append(&StreamId::Room, &event(&credential))?;
        Ok(credential)
    }

    /// Moves a finished sign-in from its own fresh directory into the
    /// credential it signs in again. Until this runs the old tokens stand, so
    /// a sign-in that is cancelled or fails changes nothing.
    pub(crate) fn adopt_login(&self, from: &str, into: &str) -> io::Result<Credential> {
        let _one_writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        let refused = || io::Error::new(io::ErrorKind::InvalidInput, CANNOT_REPLACE);
        let mut credential = self
            .find(into)
            .ok()
            .filter(|credential| {
                credential.credential_kind == CredentialKind::Oauth && !credential.revoked
            })
            .ok_or_else(refused)?;
        // `from` names a directory this removes, so it must be a sign-in's own.
        if uuid::Uuid::parse_str(from).is_err() || self.find(from).is_ok() {
            return Err(refused());
        }
        let from_dir = self.login_dir(from);
        let into_dir = self.login_dir(into);
        let tokens = self.login_tokens(from).read()?;
        let copilot = self.files.file(from_dir.join(COPILOT_RECORD)).read()?;
        let kept = if credential.provider_id == "github-copilot" {
            &copilot
        } else {
            &tokens
        };
        if kept.is_none() {
            return Err(refused());
        }
        self.check_layout()?;
        make_private_directory(&into_dir)?;
        {
            let lock = rig_login::login_lock(&into_dir);
            let _login_write = lock.lock().unwrap_or_else(PoisonError::into_inner);
            for (record, bytes) in [
                (self.login_tokens(into), tokens),
                (self.files.file(into_dir.join(COPILOT_RECORD)), copilot),
            ] {
                match bytes {
                    Some(bytes) => record.write(&bytes)?,
                    None => record.delete()?,
                }
            }
            for name in ["models.json", "endpoints.json"] {
                match read_model_file(&from_dir.join(name)) {
                    Ok(body) => write_beside_login(&into_dir, name, &body)?,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        remove_if_present(&into_dir.join(name))?;
                    }
                    Err(error) => return Err(error),
                }
            }
            rig_login::forget(&into_dir);
        }
        self.forget_discovery(into)?;
        self.abandon_login(from)?;
        credential.updated_at = now_ms();
        self.log.append(&StreamId::Room, &event(&credential))?;
        Ok(credential)
    }

    /// The models an earlier server or sign-in listed are not this one's.
    fn forget_discovery(&self, id: &str) -> io::Result<()> {
        remove_if_present(&self.login_dir(id).join("discovery.json"))
    }

    pub(crate) fn free_label(&self, provider_id: &str, base: &str) -> String {
        let taken: Vec<String> = self
            .list()
            .into_iter()
            .filter(|credential| credential.provider_id == provider_id && !credential.revoked)
            .map(|credential| credential.label.to_lowercase())
            .collect();
        let mut label = base.to_string();
        let mut number = 1;
        while taken.contains(&label.to_lowercase()) {
            number += 1;
            label = format!("{base} {number}");
        }
        label
    }

    pub(crate) fn check_label(
        &self,
        provider_id: &str,
        section: AccountSection,
        label: &str,
        except: Option<&str>,
    ) -> Result<String, String> {
        let label = label.trim();
        if !(1..=64).contains(&label.chars().count()) {
            return Err("Give the account a name of 1 to 64 characters.".into());
        }
        let wanted = label.to_lowercase();
        let taken = self.list().iter().any(|credential| {
            credential.provider_id == provider_id
                && !credential.revoked
                && Some(credential.id.as_str()) != except
                && Self::section_of(&credential.provider_id, credential.credential_kind) == section
                && credential.label.to_lowercase() == wanted
        });
        if taken {
            let provider = crate::models::catalog()
                .providers
                .get(provider_id)
                .map_or(provider_id, |entry| entry.name.as_str());
            return Err(format!("{provider} already has an account named {label}."));
        }
        Ok(label.to_string())
    }

    pub(crate) fn section_of(provider_id: &str, kind: CredentialKind) -> AccountSection {
        match kind {
            CredentialKind::Local => AccountSection::Server,
            // An OpenRouter sign-in mints a key that is billed per use.
            CredentialKind::Oauth if provider_id == "openrouter" => AccountSection::ApiKey,
            CredentialKind::Oauth => AccountSection::Subscription,
            CredentialKind::ApiKey => AccountSection::ApiKey,
        }
    }
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests;
