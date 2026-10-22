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
use crate::log::Log;
use resolve::ResolvedAccount;

pub(crate) const NOT_ON_THIS_DESK: &str = "That account is not on this desk.";
pub(crate) const NO_ACCOUNTS: &str = "That agent does not use accounts.";

/// The account `persona` runs under. A choice stays stored on an agent that
/// cannot run under one, and is ignored there.
pub(crate) fn account_of(persona: &Persona) -> Option<&str> {
    persona
        .agent_account_id
        .as_deref()
        .filter(|_| support::account_support(&persona.backend_id).is_some())
}

/// Whether a teammate on `backend_id` may be given `account_id`; none is
/// always allowed.
pub(crate) fn check_choice(
    log: &Log,
    backend_id: &str,
    account_id: Option<&str>,
) -> Result<(), String> {
    let Some(account_id) = account_id else {
        return Ok(());
    };
    if support::account_support(backend_id).is_none() {
        return Err(NO_ACCOUNTS.into());
    }
    if !crate::room::agent_accounts(log)
        .iter()
        .any(|account| account.id == account_id && account.backend_id == backend_id)
    {
        return Err(NOT_ON_THIS_DESK.into());
    }
    Ok(())
}

/// The account with its folder checked, for the desk and its owner: the
/// refusal names the account and where its folder is.
pub(crate) fn resolved(log: &Log, account_id: &str) -> Result<ResolvedAccount, String> {
    resolve::account(log.root(), log, account_id).map_err(|refusal| refusal.account_sentence())
}
