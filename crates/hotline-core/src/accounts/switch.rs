//! A teammate's move from the account its session runs on to the one it is
//! set to, which waits while its work runs or the new account cannot sign in.

use crate::contract::{
    AccountHold, AccountStatusState, AccountStatusView, LoginAttemptState, LoginAttemptView,
    SessionAccount,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingAccount {
    pub(crate) from: Option<String>,
    pub(crate) wanted: Option<String>,
    /// The same account signed in again: its sessions restart, and its other
    /// accounts' checkpoints stay.
    pub(crate) relogin: bool,
}

/// Read live each time, never stored: a hold ends the moment the status or
/// the sign-in that caused it does.
pub(crate) fn hold(
    status: &AccountStatusView,
    attempt: Option<&LoginAttemptView>,
) -> Option<AccountHold> {
    if attempt.is_some_and(|attempt| attempt.state == LoginAttemptState::Running) {
        Some(AccountHold::SigningIn)
    } else if status.state == AccountStatusState::SignedOut {
        Some(AccountHold::NotSignedIn)
    } else {
        None
    }
}

pub(crate) fn session_account(
    running: Option<String>,
    pending: Option<&PendingAccount>,
    held: Option<AccountHold>,
) -> Option<SessionAccount> {
    match pending {
        Some(pending) => Some(SessionAccount {
            running,
            wanted: pending.wanted.clone(),
            held,
        }),
        None => running.map(|running| SessionAccount {
            wanted: Some(running.clone()),
            running: Some(running),
            held: None,
        }),
    }
}

#[cfg(test)]
mod tests;
