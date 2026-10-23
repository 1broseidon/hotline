//! An agent account: a named sign-in the operator keeps for an agent, which a
//! teammate's agent process runs under instead of the agent's own default.
//!
//! An account is a folder under the data directory that the agent is pointed
//! at. Everything here is the same for every agent and parameterized by that
//! agent's row in [`support`]; a new agent joins by getting a row, a parser for
//! its status answer and the tests of both, and nothing else in this module
//! changes.

pub(crate) mod env;
pub(crate) mod login;
pub(crate) mod resolve;
pub(crate) mod setup;
pub(crate) mod status;
pub(crate) mod support;
pub(crate) mod switch;

use crate::contract::Persona;
use crate::driver::HOTLINE_BACKEND_ID;
use crate::log::{Log, StreamId};
use crate::room;
use resolve::{Named, ResolvedAccount};
use serde_json::Value;

pub(crate) const NOT_ON_THIS_DESK: &str = "That account is not on this desk.";
pub(crate) const NO_ACCOUNTS: &str = "That agent does not use accounts.";
pub(crate) const SIGNED_OUT: &str = "That account is signed out. Sign in again first.";
pub(crate) const NOT_FOR_THIS_MODEL: &str = "That account is not for this teammate's model.";

/// Hotline Agent runs on a credential of its model's provider; a harness on
/// one of its own accounts, if it has a row.
pub(crate) fn uses_accounts(backend_id: &str) -> bool {
    backend_id == HOTLINE_BACKEND_ID || support::account_support(backend_id).is_some()
}

/// The account `persona` runs under. On Hotline Agent that is the credential
/// its driver holds, whatever its provider, though only turns on that provider
/// use it; an agent's account there, or anything else its agent cannot run on,
/// stays stored and reads as none.
pub(crate) fn account_of(log: &Log, persona: &Persona) -> Option<String> {
    persona.account_id.as_ref()?;
    account_of_from(&log.load(&StreamId::Room), persona)
}

/// The same, from room events the caller has already loaded.
pub(crate) fn account_of_from(events: &[Value], persona: &Persona) -> Option<String> {
    let account_id = persona.account_id.as_deref()?;
    let runs_on_it = match resolve::named(events, account_id) {
        Named::Agent(row) => {
            row.backend_id == persona.backend_id
                && support::account_support(&persona.backend_id).is_some()
        }
        Named::Credential(_) => persona.backend_id == HOTLINE_BACKEND_ID,
        Named::Nothing => false,
    };
    runs_on_it.then(|| account_id.to_string())
}

/// Whether a teammate on `backend_id` may be given `account_id`; none is
/// always allowed. On Hotline Agent it must be a live credential of
/// `model_id`'s provider, and with no model the room's preferred one counts.
pub(crate) fn check_choice(
    log: &Log,
    backend_id: &str,
    model_id: Option<&str>,
    account_id: Option<&str>,
) -> Result<(), String> {
    let Some(account_id) = account_id else {
        return Ok(());
    };
    if !uses_accounts(backend_id) {
        return Err(NO_ACCOUNTS.into());
    }
    let events = log.load(&StreamId::Room);
    match resolve::named(&events, account_id) {
        Named::Agent(row) if row.backend_id == backend_id => Ok(()),
        Named::Credential(credential) if backend_id == HOTLINE_BACKEND_ID => {
            if credential.revoked {
                return Err(SIGNED_OUT.into());
            }
            let preferred = crate::models::preferred_model(&room::settings_from_events(&events));
            if model_provider(model_id, preferred.as_deref())
                != Some(credential.provider_id.as_str())
            {
                return Err(NOT_FOR_THIS_MODEL.into());
            }
            Ok(())
        }
        _ => Err(NOT_ON_THIS_DESK.into()),
    }
}

/// The provider a Hotline Agent teammate's turns run on: its model's, or with
/// no model the room's `preferred` one's.
pub(crate) fn model_provider<'a>(
    model_id: Option<&'a str>,
    preferred: Option<&'a str>,
) -> Option<&'a str> {
    let (provider_id, _) = model_id.or(preferred)?.split_once('/')?;
    Some(provider_id)
}

/// The account with its folder checked, for the desk and its owner: the
/// refusal names the account and where its folder is.
pub(crate) fn resolved(log: &Log, account_id: &str) -> Result<ResolvedAccount, String> {
    resolve::account(log.root(), log, account_id).map_err(|refusal| refusal.account_sentence())
}
