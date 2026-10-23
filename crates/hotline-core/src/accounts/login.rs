//! Signing an account in with its agent's own sign-in, run in the account's
//! folder under the account's environment.
//!
//! An attempt succeeds only when the process exits 0 and the status run that
//! follows says this folder is signed in: the browser can authorize another
//! account, and a window closed by hand can still exit 0.

use super::env::env_plan;
use super::resolve::{AccountRefusal, ResolvedAccount};
use super::status::{AccountStatuses, run_status, unavailable};
use super::support::AccountSupport;
use crate::contract::{AccountStatusState, AccountStatusView, LoginAttemptState, LoginAttemptView};
use crate::driver::acp::registry::{self, Launch};
use crate::driver::auth::{Attempt, COMMAND_FAILED};
use crate::room::{agent_accounts, clear_account_checkpoints, write_agent_accounts};
use crate::session::{Room, now_ms};
use crate::wire::AuthStatus;
use crate::{paths, private_dir};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const LOGIN_TIMEOUT: Duration = Duration::from_mins(10);
const CANCELLED: &str = "Sign-in cancelled.";
const DISCONNECTED: &str = "Sign-in cancelled when the desktop disconnected.";
const TIMED_OUT: &str = "Sign-in timed out. You can try again.";
const GONE: &str = "That sign-in is no longer available.";
const NOT_RECORDED: &str = "Signed in, but Hotline could not record it. Try again.";

/// On Windows the sign-in is a console window, which only the desk's own
/// screen shows.
pub(crate) fn can_sign_in_here(seat_is_desk: bool) -> bool {
    cfg!(unix) || seat_is_desk
}

pub(crate) struct AccountLogins {
    statuses: Arc<AccountStatuses>,
    /// Kept after it ends, so the last attempt can still be read.
    attempts: Mutex<HashMap<String, Arc<Attempt>>>,
}

struct Run {
    launch: Launch,
    status_bound: Duration,
    account: ResolvedAccount,
    args: Vec<String>,
    agent: String,
    timeout: Duration,
}

enum Outcome {
    Exited(Result<(), String>),
    Stopped(&'static str),
}

impl AccountLogins {
    pub(crate) fn new(statuses: Arc<AccountStatuses>) -> Self {
        Self {
            statuses,
            attempts: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) async fn start(
        self: &Arc<Self>,
        room: &Arc<Room>,
        account: &ResolvedAccount,
        email: Option<String>,
        owner: CancellationToken,
        seat_is_desk: bool,
    ) -> Result<String, String> {
        if !can_sign_in_here(seat_is_desk) {
            return Err("Sign in from the desk itself.".into());
        }
        if owner.is_cancelled() {
            return Err("The desktop connection has closed.".into());
        }
        let root = room.log().root();
        let agent = registry::name_of(root, &account.backend_id)
            .unwrap_or_else(|| account.backend_id.clone());
        let dir = private_dir::checked_account_dir(root, &account.id).map_err(|error| {
            AccountRefusal::FolderUnavailable {
                account_id: account.id.clone(),
                name: account.name.clone(),
                path: paths::agent_account_dir(root, &account.id)
                    .expect("a resolved account's id names a folder"),
                kind: error.kind(),
            }
            .private_sentence(&account.name, &agent)
        })?;
        registry::install(root, &account.backend_id).await?;
        let launch = registry::launch(root, &account.backend_id)?;
        let attempt = self.admit(&account.id, owner)?;
        let id = attempt.id.clone();
        let run = Run {
            status_bound: self.statuses.bound(root, &launch),
            launch,
            account: ResolvedAccount {
                dir,
                ..account.clone()
            },
            args: login_args(account.support, email.as_deref()),
            agent,
            timeout: LOGIN_TIMEOUT,
        };
        let (logins, room) = (self.clone(), room.clone());
        tokio::spawn(async move { logins.run(&room, run, attempt).await });
        Ok(id)
    }

    fn admit(&self, account_id: &str, owner: CancellationToken) -> Result<Arc<Attempt>, String> {
        // The check and the insert are one critical section, or two callers
        // could both find no attempt running.
        let mut attempts = lock(&self.attempts);
        if attempts
            .get(account_id)
            .is_some_and(|attempt| attempt.running())
        {
            return Err("A sign-in is already running for this account.".into());
        }
        let attempt = Attempt::new(owner);
        attempts.insert(account_id.to_string(), attempt.clone());
        Ok(attempt)
    }

    fn attempt(&self, account_id: &str, id: &str) -> Result<Arc<Attempt>, String> {
        lock(&self.attempts)
            .get(account_id)
            .filter(|attempt| attempt.id == id)
            .cloned()
            .ok_or_else(|| GONE.to_string())
    }

    pub(crate) fn poll(&self, account_id: &str, id: &str) -> Result<AuthStatus, String> {
        Ok(self.attempt(account_id, id)?.poll())
    }

    pub(crate) fn input(&self, account_id: &str, id: &str, input: &str) -> Result<(), String> {
        self.attempt(account_id, id)?.input(input)
    }

    pub(crate) fn cancel(&self, account_id: &str, id: &str) -> Result<(), String> {
        self.attempt(account_id, id)?.cancel.cancel();
        Ok(())
    }

    pub(crate) fn view(&self, account_id: &str) -> Option<LoginAttemptView> {
        let attempt = lock(&self.attempts).get(account_id).cloned()?;
        let (state, error) = attempt.state();
        let state = match state {
            "running" => LoginAttemptState::Running,
            "succeeded" => LoginAttemptState::Succeeded,
            _ => LoginAttemptState::Failed,
        };
        Some(LoginAttemptView {
            id: attempt.id.clone(),
            state,
            error,
        })
    }

    async fn run(self: &Arc<Self>, room: &Arc<Room>, run: Run, attempt: Arc<Attempt>) {
        let outcome = sign_in(&run, &attempt).await;
        self.conclude(room, &run, &attempt, outcome).await;
    }

    async fn conclude(&self, room: &Arc<Room>, run: &Run, attempt: &Attempt, outcome: Outcome) {
        let root = room.log().root();
        let status = self
            .statuses
            .recheck_with(&run.account.id, || {
                // A stop fails whatever the status says, so it is failed as
                // soon as the recheck holds the gate, not after it.
                if let Outcome::Stopped(sentence) = outcome {
                    attempt.finish(Err(sentence.to_string()));
                }
                async move {
                    match private_dir::checked_account_dir(root, &run.account.id) {
                        Ok(dir) if dir == run.account.dir => {
                            run_status(&run.launch, &run.account, run.status_bound).await
                        }
                        _ => unavailable(),
                    }
                }
            })
            .await;
        let Outcome::Exited(exited) = outcome else {
            room.retry_switches_to(&run.account.id);
            return;
        };
        let exited = exited.map_err(|error| {
            if error == COMMAND_FAILED {
                format!("{} closed before it was signed in.", run.agent)
            } else {
                error
            }
        });
        let result = settle(room, &run.account, exited, &status, &run.agent).await;
        let confirmed = result.is_ok();
        attempt.finish(result);
        // Only now: a running attempt holds every switch to this account.
        if confirmed {
            room.account_signed_in(&run.account.id);
        } else {
            room.retry_switches_to(&run.account.id);
        }
    }
}

/// The sign-in's arguments, with the email only when it is plain enough that
/// no shell or `cmd.exe` between here and the agent reads anything into it.
fn login_args(support: &AccountSupport, email: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = support.login_args.iter().map(ToString::to_string).collect();
    if let (Some(flag), Some(email)) = (
        support.email_flag,
        email.filter(|email| is_plain_email(email)),
    ) {
        args.push(flag.to_string());
        args.push(email.to_string());
    }
    args
}

fn is_plain_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    let local_ok = (1..=64).contains(&local.len())
        && local
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
        && !local.starts_with(['-', '.'])
        && !local.contains("..");
    let labels: Vec<&str> = domain.split('.').collect();
    let domain_ok = labels.len() >= 2
        && labels.iter().all(|label| {
            (1..=63).contains(&label.len())
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        });
    email.len() <= 254 && local_ok && domain_ok
}

#[cfg(unix)]
async fn sign_in(run: &Run, attempt: &Arc<Attempt>) -> Outcome {
    let env = env_plan(
        &run.launch.env,
        &[("TERM".into(), "xterm-256color".into())],
        Some(&run.account),
    );
    let mut terminal = tokio::spawn(crate::driver::auth::terminal(
        run.launch.clone(),
        run.account.dir.clone(),
        run.args.clone(),
        env,
        attempt.clone(),
    ));
    // The terminal ends with the same sentence for a cancel and a disconnect,
    // so which one it was is read from the tokens first.
    let stopped = tokio::select! {
        biased;
        () = attempt.cancel.cancelled() => CANCELLED,
        () = attempt.owner.cancelled() => DISCONNECTED,
        () = tokio::time::sleep(run.timeout) => TIMED_OUT,
        exited = &mut terminal => {
            return Outcome::Exited(exited.unwrap_or_else(|_| {
                Err("The sign-in terminal stopped unexpectedly.".into())
            }));
        }
    };
    attempt.cancel.cancel();
    let _ = terminal.await;
    Outcome::Stopped(stopped)
}

#[cfg(windows)]
async fn sign_in(run: &Run, attempt: &Arc<Attempt>) -> Outcome {
    let not_started = || {
        Outcome::Exited(Err(format!(
            "{}'s sign-in could not be started.",
            run.agent
        )))
    };
    let mut command = tokio::process::Command::new(&run.launch.command);
    command.args(&run.launch.args).args(&run.args);
    env_plan(&run.launch.env, &[], Some(&run.account)).apply(&mut command);
    command.current_dir(&run.account.dir);
    crate::process_windows::prepare_visible(&mut command);
    let Ok(mut child) = command.spawn() else {
        return not_started();
    };
    let Ok(job) = crate::process_windows::Job::attach_without_kill_on_close(child.id()) else {
        // Still suspended, and `prepare_visible` does not kill on drop.
        let _ = child.kill().await;
        return not_started();
    };
    let waited = tokio::select! {
        biased;
        () = attempt.cancel.cancelled() => Err(CANCELLED),
        () = attempt.owner.cancelled() => Err(DISCONNECTED),
        () = tokio::time::sleep(run.timeout) => Err(TIMED_OUT),
        exited = child.wait() => Ok(exited),
    };
    match waited {
        Ok(Ok(status)) if status.success() => Outcome::Exited(Ok(())),
        Ok(_) => Outcome::Exited(Err(COMMAND_FAILED.into())),
        Err(stopped) => {
            let _ = job.terminate();
            // Ends a child the job did not, and does nothing to one it did.
            let _ = child.kill().await;
            Outcome::Stopped(stopped)
        }
    }
}

async fn settle(
    room: &Arc<Room>,
    account: &ResolvedAccount,
    process: Result<(), String>,
    status: &AccountStatusView,
    agent: &str,
) -> Result<(), String> {
    process?;
    if status.state != AccountStatusState::SignedIn {
        return Err(format!(
            "{agent} finished, but {} is not signed in.",
            account.name
        ));
    }
    signed_in(room, account).await
}

async fn signed_in(room: &Arc<Room>, account: &ResolvedAccount) -> Result<(), String> {
    let policy = room.policy_update_lock();
    let _held = policy.lock().await;
    let mut accounts = agent_accounts(room.log());
    let Some(row) = accounts.iter_mut().find(|row| row.id == account.id) else {
        return Ok(());
    };
    row.login_at = Some(now_ms());
    write_agent_accounts(room.log(), &accounts).map_err(|_| NOT_RECORDED.to_string())?;
    clear_account_checkpoints(room.log(), &account.id).map_err(|_| NOT_RECORDED.to_string())
}

fn lock<T>(held: &Mutex<T>) -> MutexGuard<'_, T> {
    held.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
