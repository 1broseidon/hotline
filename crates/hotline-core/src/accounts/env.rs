//! The environment of a process that runs under an account, as one pure plan
//! and two appliers.
//!
//! The order is the launch's environment, then the login method's, then the
//! removal of every inherited credential, then the account's folder last. The
//! removal comes after everything that could bring a removed variable back, and
//! the folder after the removal, so that nothing but the account's own sign-in
//! is left for the agent to find.
//!
//! Without an account the plan removes nothing and adds nothing: a launch runs
//! exactly as it did before accounts existed.

use super::resolve::ResolvedAccount;
use std::ffi::OsString;

/// What to do to a process's environment before it starts: first remove every
/// name in `remove`, then set every pair in `set`, in order.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct EnvPlan {
    pub(crate) remove: Vec<OsString>,
    pub(crate) set: Vec<(OsString, OsString)>,
}

/// The plan for a launch whose catalogue entry asks for `catalogue` and whose
/// login method asks for `method`, run under `account` or, with none, as the
/// agent's own default.
///
/// A name the account scrubs, or its folder variable, is dropped from both
/// lists whatever its case: Windows ignores the case of an environment name,
/// so a lowercase spelling of a scrubbed name would otherwise bring it back.
pub(crate) fn env_plan(
    catalogue: &[(String, String)],
    method: &[(String, String)],
    account: Option<&ResolvedAccount>,
) -> EnvPlan {
    let asked = catalogue.iter().chain(method);
    let Some(account) = account else {
        return EnvPlan {
            remove: Vec::new(),
            set: asked
                .map(|(name, value)| (name.into(), value.into()))
                .collect(),
        };
    };
    let support = account.support;
    let is_replaced = |name: &str| {
        support
            .scrub
            .iter()
            .chain([&support.env_var])
            .any(|replaced| replaced.eq_ignore_ascii_case(name))
    };
    let mut set: Vec<(OsString, OsString)> = asked
        .filter(|(name, _)| !is_replaced(name))
        .map(|(name, value)| (name.into(), value.into()))
        .collect();
    set.push((support.env_var.into(), account.dir.clone().into_os_string()));
    EnvPlan {
        remove: support.scrub.iter().map(OsString::from).collect(),
        set,
    }
}

impl EnvPlan {
    /// Applies the plan to a command, every removal before any set: the
    /// account's folder variable is in both lists, and it must survive.
    pub(crate) fn apply(&self, command: &mut tokio::process::Command) {
        for name in &self.remove {
            command.env_remove(name);
        }
        for (name, value) in &self.set {
            command.env(name, value);
        }
    }

    /// The same for a pseudo-terminal command. Only the terminal sign-in uses
    /// it, and that exists only on Unix.
    #[cfg(unix)]
    pub(crate) fn apply_pty(&self, command: &mut portable_pty::CommandBuilder) {
        for name in &self.remove {
            command.env_remove(name);
        }
        for (name, value) in &self.set {
            command.env(name, value);
        }
    }
}

#[cfg(test)]
mod tests;
