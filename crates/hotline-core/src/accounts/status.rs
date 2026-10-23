//! What an agent says about whether a folder is signed in, and how far that is
//! believed.
//!
//! The answer comes from the agent's own process, run with the account's
//! folder, so an environment variable the scrub list does not know could still
//! change whose sign-in answers. That is why the answer is accepted only when it
//! names the folder it was asked about. The raw answer is read and dropped: it
//! never reaches a log, a tape or the wire.

use super::env::env_plan;
use super::resolve::{self, ResolvedAccount};
use super::support::account_support;
use crate::contract::{AccountStatusState, AccountStatusView};
use crate::driver::acp::registry::{self, Launch};
use crate::driver::acp::{self, StartBounds};
use crate::private_dir;
use crate::room::{agent_accounts, roster};
use crate::session::{Room, now_ms};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::Duration;
use tokio::io::AsyncReadExt;

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

pub(crate) const STALE_AFTER_MS: i64 = 600_000;

const STATUS_OUTPUT_LIMIT: u64 = 64 * 1024;

/// What each account's agent last said about its sign-in, in memory only,
/// with at most one status run per account at a time.
pub(crate) struct AccountStatuses {
    bounds: StartBounds,
    entries: Mutex<HashMap<String, Entry>>,
    /// Told of each answer recorded, so a switch the last answer held can go.
    room: OnceLock<Weak<Room>>,
}

#[derive(Default)]
struct Entry {
    /// Held for the whole of a run: a caller that finds it held waits for
    /// that run's answer instead of starting another.
    gate: Arc<tokio::sync::Mutex<()>>,
    view: Option<AccountStatusView>,
    runs: u64,
}

impl AccountStatuses {
    pub(crate) fn new(bounds: StartBounds) -> Self {
        Self {
            bounds,
            entries: Mutex::new(HashMap::new()),
            room: OnceLock::new(),
        }
    }

    pub(crate) fn report_to(&self, room: Weak<Room>) {
        let _ = self.room.set(room);
    }

    pub(crate) async fn check(&self, root: &Path, account: &ResolvedAccount) -> AccountStatusView {
        self.check_with(&account.id, || async {
            self.run_in_checked_folder(root, account)
                .await
                .unwrap_or_else(|_| unavailable())
        })
        .await
    }

    async fn run_in_checked_folder(
        &self,
        root: &Path,
        account: &ResolvedAccount,
    ) -> Result<AccountStatusView, String> {
        let dir = private_dir::checked_account_dir(root, &account.id)
            .map_err(|error| error.to_string())?;
        registry::install(root, &account.backend_id).await?;
        let launch = registry::launch(root, &account.backend_id)?;
        let account = ResolvedAccount {
            dir,
            ..account.clone()
        };
        Ok(run_status(&launch, &account, self.bound(root, &launch)).await)
    }

    /// A launch that has never answered here may be fetching its package, so
    /// it gets the first-start bound. Status never marks a launch started.
    pub(super) fn bound(&self, root: &Path, launch: &Launch) -> Duration {
        if registry::started_before(root, launch) {
            self.bounds.known
        } else {
            self.bounds.first
        }
    }

    pub(super) async fn check_with<F: Future<Output = AccountStatusView>>(
        &self,
        account_id: &str,
        run: impl FnOnce() -> F,
    ) -> AccountStatusView {
        let (gate, runs_before) = self.gate(account_id);
        let running = gate.lock().await;
        if let Some(entry) = lock(&self.entries).get(account_id)
            && entry.runs != runs_before
            && let Some(view) = &entry.view
        {
            return view.clone();
        }
        let view = self.record(account_id, run().await);
        drop(running);
        self.recorded(account_id);
        view
    }

    /// For a question asked after something changed the folder, such as
    /// whether a sign-in that just ended took: an answer from a run already
    /// going predates the change, so this waits its turn and always runs.
    pub(super) async fn recheck_with<F: Future<Output = AccountStatusView>>(
        &self,
        account_id: &str,
        run: impl FnOnce() -> F,
    ) -> AccountStatusView {
        let (gate, _) = self.gate(account_id);
        let running = gate.lock().await;
        let view = self.record(account_id, run().await);
        drop(running);
        self.recorded(account_id);
        view
    }

    /// Only once the gate is let go: until then the account reads as checking.
    fn recorded(&self, account_id: &str) {
        if let Some(room) = self.room.get().and_then(Weak::upgrade) {
            room.retry_switches_to(account_id);
        }
    }

    fn gate(&self, account_id: &str) -> (Arc<tokio::sync::Mutex<()>>, u64) {
        let mut entries = lock(&self.entries);
        let entry = entries.entry(account_id.to_string()).or_default();
        (entry.gate.clone(), entry.runs)
    }

    fn record(&self, account_id: &str, mut view: AccountStatusView) -> AccountStatusView {
        view.checked_at = Some(now_ms());
        let mut entries = lock(&self.entries);
        let entry = entries.entry(account_id.to_string()).or_default();
        entry.view = Some(view.clone());
        entry.runs += 1;
        view
    }

    /// How many checks of the account hold its gate or wait for it: each one
    /// keeps a clone of the gate for as long as it runs.
    #[cfg(test)]
    pub(super) fn checks_in_line(&self, account_id: &str) -> usize {
        lock(&self.entries)
            .get(account_id)
            .map_or(0, |entry| Arc::strong_count(&entry.gate) - 1)
    }

    pub(crate) fn cached(&self, account_id: &str) -> AccountStatusView {
        let last = self.last_answer(account_id);
        let checking = lock(&self.entries)
            .get(account_id)
            .is_some_and(|entry| entry.gate.try_lock().is_err());
        if checking {
            AccountStatusView {
                state: AccountStatusState::Checking,
                ..last
            }
        } else {
            last
        }
    }

    /// Without the `Checking` a run in flight shows.
    pub(crate) fn last_answer(&self, account_id: &str) -> AccountStatusView {
        lock(&self.entries)
            .get(account_id)
            .and_then(|entry| entry.view.clone())
            .unwrap_or_else(not_checked)
    }

    /// Never-checked is not stale: such an account waits for a person to ask.
    pub(crate) fn stale(&self, account_id: &str, now_ms: i64) -> bool {
        let entries = lock(&self.entries);
        entries.get(account_id).is_some_and(|entry| {
            entry.gate.try_lock().is_ok()
                && entry
                    .view
                    .as_ref()
                    .and_then(|view| view.checked_at)
                    .is_some_and(|checked_at| now_ms.saturating_sub(checked_at) > STALE_AFTER_MS)
        })
    }

    pub(crate) fn refresh(self: &Arc<Self>, root: PathBuf, account: ResolvedAccount) {
        let statuses = self.clone();
        tokio::spawn(async move {
            statuses.check(&root, &account).await;
        });
    }

    pub(crate) fn refresh_in_use(self: &Arc<Self>, room: &Room) {
        let log = room.log();
        let root = log.root().to_path_buf();
        let accounts = agent_accounts(log);
        // A credential is not an agent's sign-in, so the agent has nothing to say of it.
        let in_use: BTreeSet<String> = roster(log)
            .into_iter()
            .filter(|persona| account_support(&persona.backend_id).is_some())
            .filter_map(|persona| persona.account_id)
            .filter(|account_id| accounts.iter().any(|account| &account.id == account_id))
            .collect();
        for account_id in in_use {
            if let Ok(account) = resolve::account(&root, log, &account_id) {
                self.refresh(root.clone(), account);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn put(&self, account_id: &str, status: AccountStatusView) {
        lock(&self.entries)
            .entry(account_id.to_string())
            .or_default()
            .view = Some(status);
    }
}

pub(super) async fn run_status(
    launch: &Launch,
    account: &ResolvedAccount,
    bound: Duration,
) -> AccountStatusView {
    let mut command = tokio::process::Command::new(&launch.command);
    command.args(&launch.args).args(account.support.status_args);
    env_plan(&launch.env, &[], Some(account)).apply(&mut command);
    command
        .current_dir(&account.dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(windows)]
    crate::process_windows::prepare(&mut command);
    let Ok(mut child) = command.spawn() else {
        return unavailable();
    };
    #[cfg(unix)]
    if let Some(id) = child.id() {
        lock(&acp::GROUPS).push(id as libc::pid_t);
    }
    #[cfg(windows)]
    let Ok(job) = crate::process_windows::Job::attach(child.id()) else {
        return unavailable();
    };
    let Some(stdout) = child.stdout.take() else {
        acp::end_group(&child);
        return unavailable();
    };
    let mut answer = Vec::new();
    let answered = tokio::time::timeout(
        bound,
        stdout.take(STATUS_OUTPUT_LIMIT).read_to_end(&mut answer),
    )
    .await;
    acp::end_group(&child);
    #[cfg(windows)]
    if answered.is_err() {
        let _ = job.terminate();
    }
    match answered {
        Ok(Ok(_)) => (account.support.parse_status)(&answer, &account.dir),
        _ => unavailable(),
    }
}

fn not_checked() -> AccountStatusView {
    only(AccountStatusState::NotChecked)
}

pub(super) fn unavailable() -> AccountStatusView {
    only(AccountStatusState::Unavailable)
}

fn only(state: AccountStatusState) -> AccountStatusView {
    AccountStatusView {
        state,
        checked_at: None,
        email: None,
        plan: None,
    }
}

fn lock<T>(held: &Mutex<T>) -> MutexGuard<'_, T> {
    held.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
