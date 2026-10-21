//! What an agent says about whether a folder is signed in, and how far that is
//! believed.
//!
//! The answer comes from the agent's own process, run with the account's
//! folder, so an environment variable the scrub list does not know could still
//! change whose sign-in answers. That is why the answer is accepted only when it
//! names the folder it was asked about (spec §10.2). The raw answer is read and
//! dropped: it never reaches a log, a tape or the wire.

use crate::contract::{AccountStatusState, AccountStatusView};
use serde_json::Value;
use std::path::Path;

/// Reads `claude auth status --json`'s answer for the account folder `dir`.
///
/// Signed in means a first-party subscription login, with no API key standing
/// in for it, in exactly this folder. A logged-out answer is signed out
/// whatever else it says. Anything else, including an answer that cannot be
/// read, is unavailable and says nothing more: a person is shown a fixed
/// sentence, never the answer. `checked_at` is left for whoever keeps the cache.
pub(crate) fn parse_claude_status(stdout: &[u8], dir: &Path) -> AccountStatusView {
    let answer: Value = serde_json::from_slice(stdout.trim_ascii()).unwrap_or(Value::Null);
    let text = |key: &str| answer[key].as_str().map(str::to_string);
    let (state, email, plan) = match answer["loggedIn"] {
        Value::Bool(false) => (AccountStatusState::SignedOut, None, None),
        Value::Bool(true) if is_first_party_login_in(&answer, dir) => (
            AccountStatusState::SignedIn,
            text("email"),
            text("subscriptionType"),
        ),
        _ => (AccountStatusState::Unavailable, None, None),
    };
    AccountStatusView {
        state,
        checked_at: None,
        email,
        plan,
    }
}

/// Whether a logged-in `answer` is the subscription login of the folder `dir`,
/// compared as the real folders the two spellings reach.
fn is_first_party_login_in(answer: &Value, dir: &Path) -> bool {
    answer["apiProvider"] == "firstParty"
        && answer["authMethod"] == "claude.ai"
        && answer["apiKeySource"].is_null()
        && answer["configDirectory"]
            .as_str()
            .and_then(|named| dunce::canonicalize(named).ok())
            .zip(dunce::canonicalize(dir).ok())
            .is_some_and(|(named, wanted)| named == wanted)
}

#[cfg(test)]
mod tests;
