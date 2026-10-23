//! Which account a teammate runs under, resolved before any process starts to
//! the account's checked folder or to a refusal that says why not.
//!
//! The room stores only the account's row; the folder is derived from its id
//! and checked here every time, because a folder swapped for a link would
//! send an account's sign-in somewhere else.

use super::support::{AccountSupport, account_support};
use crate::contract::{AgentAccount, Credential, Persona};
use crate::log::{Log, StreamId};
use crate::vault::Vault;
use crate::{paths, private_dir, room};
use hotline_room::provider_auth::ChosenAccount;
use serde_json::Value;
use std::io;
use std::path::{Path, PathBuf};

/// An account a teammate may run under: its row, and the folder that passed the
/// check just now.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedAccount {
    pub(crate) id: String,
    pub(crate) backend_id: String,
    pub(crate) name: String,
    /// The canonical folder. Use this one and no other spelling of it.
    pub(crate) dir: PathBuf,
    pub(crate) support: &'static AccountSupport,
}

/// Why a teammate cannot start under its account. Two sentences tell it: the
/// private one for the desk and its owner, who may see the account's name and
/// where it lives, and the public one for everyone else, who may not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AccountRefusal {
    /// The desk has no such account, or it belongs to an agent that cannot run
    /// under one.
    UnknownAccount { account_id: String },
    /// The account is on the desk, but its folder is not a real directory where
    /// Hotline made it. `path` is where the folder should be, not where a link
    /// in its place leads.
    FolderUnavailable {
        account_id: String,
        name: String,
        path: PathBuf,
        kind: io::ErrorKind,
    },
}

impl AccountRefusal {
    /// For the desk and its owner. `agent` is the agent's name for people.
    pub(crate) fn private_sentence(&self, teammate: &str, agent: &str) -> String {
        match self {
            Self::UnknownAccount { .. } => format!(
                "Pick a {agent} account for {teammate}: the one it was set to is not on this desk."
            ),
            Self::FolderUnavailable { .. } => self.account_sentence(),
        }
    }

    /// For the desk and its owner when no teammate is involved.
    pub(crate) fn account_sentence(&self) -> String {
        match self {
            Self::UnknownAccount { .. } => super::NOT_ON_THIS_DESK.into(),
            Self::FolderUnavailable {
                name, path, kind, ..
            } => format!(
                "Can't open {name}'s folder ({}): {kind}. Fix the folder or pick another account.",
                path.display()
            ),
        }
    }

    /// For anyone else: the same words for both refusals, and never the
    /// account's name, its path or the reason, which only the desk may know.
    pub(crate) fn public_sentence(&self, teammate: &str, agent: &str) -> String {
        format!("{teammate} can't start: its {agent} account needs attention at the desk.")
    }
}

/// The account `persona` runs under: `None` when it has none, when its agent
/// cannot run under one, or when it is set to another agent's account or to a
/// Hotline Agent credential. The choice stays stored and is ignored, so it
/// applies again if the teammate returns to where it can.
pub(crate) fn resolve(
    root: &Path,
    log: &Log,
    persona: &Persona,
) -> Result<Option<ResolvedAccount>, AccountRefusal> {
    let Some(account_id) = &persona.account_id else {
        return Ok(None);
    };
    if account_support(&persona.backend_id).is_none() {
        return Ok(None);
    }
    match named(&log.load(&StreamId::Room), account_id) {
        Named::Agent(row) if row.backend_id == persona.backend_id => checked(root, row).map(Some),
        Named::Agent(_) | Named::Credential(_) => Ok(None),
        Named::Nothing => Err(AccountRefusal::UnknownAccount {
            account_id: account_id.clone(),
        }),
    }
}

/// The credential a Hotline Agent teammate is set to, revoked or not and of
/// whichever provider: a turn on another provider's model ignores it. An
/// agent's account is ignored here, as a credential is by [`resolve`].
pub(crate) fn hotline(
    log: &Log,
    persona: &Persona,
) -> Result<Option<ChosenAccount>, AccountRefusal> {
    let Some(account_id) = &persona.account_id else {
        return Ok(None);
    };
    match named(&log.load(&StreamId::Room), account_id) {
        Named::Credential(credential) => Ok(Some(ChosenAccount {
            id: credential.id,
            provider_id: credential.provider_id,
        })),
        Named::Agent(_) => Ok(None),
        Named::Nothing => Err(AccountRefusal::UnknownAccount {
            account_id: account_id.clone(),
        }),
    }
}

/// What an account id names on this desk.
pub(crate) enum Named {
    Agent(AgentAccount),
    Credential(Credential),
    Nothing,
}

/// `events` are the room stream's.
pub(crate) fn named(events: &[Value], account_id: &str) -> Named {
    if let Some(row) = room::agent_accounts_from(events)
        .into_iter()
        .find(|row| row.id == account_id)
    {
        return Named::Agent(row);
    }
    Vault::credentials(events)
        .into_iter()
        .find(|credential| credential.id == account_id)
        .map_or(Named::Nothing, Named::Credential)
}

/// The account with `account_id`, for a command about the account itself: its
/// status, a sign-in or its setup.
pub(crate) fn account(
    root: &Path,
    log: &Log,
    account_id: &str,
) -> Result<ResolvedAccount, AccountRefusal> {
    let row = room::agent_accounts(log)
        .into_iter()
        .find(|row| row.id == account_id)
        .ok_or_else(|| AccountRefusal::UnknownAccount {
            account_id: account_id.to_string(),
        })?;
    checked(root, row)
}

/// `row` as an account a teammate may run under, its folder checked now.
fn checked(root: &Path, row: AgentAccount) -> Result<ResolvedAccount, AccountRefusal> {
    let unknown = || AccountRefusal::UnknownAccount {
        account_id: row.id.clone(),
    };
    let support = account_support(&row.backend_id).ok_or_else(unknown)?;
    // Defensive: the room admits only the uuid spelling `agent_account_dir`
    // accepts, so no stored id reaches the `None`.
    let path = paths::agent_account_dir(root, &row.id).ok_or_else(unknown)?;
    let dir = private_dir::checked_account_dir(root, &row.id).map_err(|error| {
        AccountRefusal::FolderUnavailable {
            account_id: row.id.clone(),
            name: row.name.clone(),
            path,
            kind: error.kind(),
        }
    })?;
    Ok(ResolvedAccount {
        id: row.id,
        backend_id: row.backend_id,
        name: row.name,
        dir,
        support,
    })
}

#[cfg(test)]
mod tests;
