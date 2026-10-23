use super::*;
use crate::accounts::support::CLAUDE;
use crate::contract::AccountStatusState::{Checking, SignedIn, SignedOut, Unavailable};
use crate::contract::LoginAttemptState::{Failed, Running, Succeeded};
use crate::contract::{AccountHold, Persona, SessionAccount};
use crate::driver::acp::StartBounds;
use crate::log::Log;
use crate::room::{checkpoint_session, roster};
use crate::session::tests::{DeskKeys, enrol, persona};
use crate::test_support::{PATIENCE, account_on_disk, acp_fixture_launch, link, until};
use std::future::Future;
use tempfile::TempDir;
use tokio::sync::oneshot;

const A: &str = "3f6c2a1e-8b4d-4e7a-9c15-2d8e7f0a6b31";
const B: &str = "a7d94e02-1c5b-4f83-b6e9-0f2c3d4a5b68";

struct Desk {
    _tmp: TempDir,
    log: Log,
    room: Arc<Room>,
    logins: Arc<AccountLogins>,
    a: ResolvedAccount,
}

fn logins() -> Arc<AccountLogins> {
    Arc::new(AccountLogins::new(Arc::new(AccountStatuses::new(
        StartBounds::default(),
    ))))
}

fn desk() -> Desk {
    let tmp = tempfile::tempdir().unwrap();
    let log = Log::open(tmp.path().join("Conta Teste ção"));
    let a = account_on_disk(&log, A, "Account 1", Some(5));
    account_on_disk(&log, B, "Account 2", Some(5));
    enrol(
        &log,
        &Persona {
            backend_id: "claude-acp".to_string(),
            account_id: Some(A.to_string()),
            ..persona("ada")
        },
    );
    for (account, session) in [(Some(A), "s1"), (Some(B), "s2"), (None, "s3")] {
        checkpoint_session(&log, "ada", "claude-acp", account, session).unwrap();
    }
    let room = Room::new(log.clone(), Arc::new(DeskKeys));
    Desk {
        _tmp: tmp,
        log,
        logins: room.account_logins().clone(),
        room,
        a,
    }
}

impl Desk {
    fn login_at(&self, id: &str) -> Option<i64> {
        agent_accounts(&self.log)
            .into_iter()
            .find(|row| row.id == id)
            .and_then(|row| row.login_at)
    }

    fn sessions(&self) -> Vec<String> {
        roster(&self.log)
            .into_iter()
            .find(|persona| persona.id == "ada")
            .unwrap()
            .session_checkpoints
            .into_iter()
            .map(|checkpoint| checkpoint.session_id)
            .collect()
    }

    fn outcome(&self) -> Option<(LoginAttemptState, Option<String>)> {
        self.logins.view(A).map(|view| (view.state, view.error))
    }

    fn plant_credentials(&self) {
        std::fs::write(
            self.a.dir.join(".credentials.json"),
            r#"{"email":"ada@example.com"}"#,
        )
        .unwrap();
    }

    fn sign(
        &self,
        mode: &str,
        timeout: Duration,
        owner: CancellationToken,
    ) -> impl Future<Output = ()> + Send + use<> {
        let attempt = self.logins.admit(A, owner).unwrap();
        let run = self.run(mode, timeout);
        let (logins, room) = (self.logins.clone(), self.room.clone());
        async move { logins.run(&room, run, attempt).await }
    }

    fn run(&self, mode: &str, timeout: Duration) -> Run {
        Run {
            launch: acp_fixture_launch(&[("FIXTURE_LOGIN_MODE", mode)]),
            status_bound: PATIENCE,
            account: self.a.clone(),
            args: login_args(&CLAUDE, Some("ada@example.com")),
            agent: "Claude Code".to_string(),
            timeout,
        }
    }

    fn reported(&self, run: &str) -> Vec<String> {
        let path = self.a.dir.join(format!("fixture-{run}.json"));
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    async fn switch_to_a(&self, held: AccountHold) {
        let mut ada = roster(&self.log)
            .into_iter()
            .find(|persona| persona.id == "ada")
            .unwrap();
        ada.account_id = Some(B.to_string());
        crate::room::append_persona(&self.log, &ada).unwrap();
        assert_eq!(
            self.room.assign_account("ada", Some(A.to_string())).await,
            Ok(SessionAccount {
                running: None,
                wanted: Some(A.to_string()),
                held: Some(held),
            })
        );
        assert_eq!(self.sessions(), ["s1", "s2", "s3"]);
    }
}

fn view(state: AccountStatusState) -> AccountStatusView {
    AccountStatusView {
        state,
        checked_at: None,
        email: None,
        plan: None,
    }
}

#[tokio::test]
async fn only_a_confirmed_sign_in_moves_login_at_and_clears_that_accounts_checkpoints() {
    let desk = desk();
    let (room, a) = (&desk.room, &desk.a);
    assert_eq!(
        settle(
            room,
            a,
            Err("closed".into()),
            &view(SignedIn),
            "Claude Code"
        )
        .await,
        Err("closed".into())
    );
    assert_eq!(
        settle(room, a, Ok(()), &view(SignedOut), "Claude Code").await,
        Err("Claude Code finished, but Account 1 is not signed in.".into())
    );
    assert!(
        settle(room, a, Ok(()), &view(Unavailable), "Claude Code")
            .await
            .is_err()
    );
    assert_eq!(desk.login_at(A), Some(5));
    assert_eq!(desk.sessions(), ["s1", "s2", "s3"]);

    assert_eq!(
        settle(room, a, Ok(()), &view(SignedIn), "Claude Code").await,
        Ok(())
    );
    assert!(desk.login_at(A) > Some(5));
    assert_eq!(desk.login_at(B), Some(5));
    assert_eq!(desk.sessions(), ["s2", "s3"]);
}

#[test]
fn one_running_attempt_per_account() {
    let logins = logins();
    let first = logins.admit(A, CancellationToken::new()).unwrap();
    assert_eq!(
        logins.admit(A, CancellationToken::new()).err(),
        Some("A sign-in is already running for this account.".into())
    );
    assert!(logins.admit(B, CancellationToken::new()).is_ok());
    assert_eq!(logins.view(A).map(|view| view.state), Some(Running));
    assert_eq!(
        logins.input(A, &first.id, "x"),
        Err("This sign-in is not accepting terminal input.".into())
    );
    assert_eq!(logins.cancel(A, "other"), Err(GONE.into()));

    logins.cancel(A, &first.id).unwrap();
    assert!(first.cancel.is_cancelled());
    first.finish(Err(CANCELLED.into()));
    assert_eq!(
        logins.view(A).map(|view| (view.state, view.error)),
        Some((Failed, Some(CANCELLED.into())))
    );

    let second = logins.admit(A, CancellationToken::new()).unwrap();
    assert_eq!(logins.poll(A, &first.id).err(), Some(GONE.into()));
    assert!(logins.poll(A, &second.id).is_ok() && logins.poll(B, &second.id).is_err());
}

#[test]
fn only_a_plain_email_becomes_an_argument() {
    assert_eq!(
        login_args(&CLAUDE, Some("ada@example.com")),
        [
            "--cli",
            "auth",
            "login",
            "--claudeai",
            "--email",
            "ada@example.com"
        ]
    );
    assert_eq!(
        login_args(&CLAUDE, None),
        ["--cli", "auth", "login", "--claudeai"]
    );
    for refused in [
        "",
        "ada",
        "ada@example",
        "a b@example.com",
        "a\"b@example.com",
        "a%PATH%@example.com",
        "a&calc@example.com",
        "-x@example.com",
        "a@-example.com",
        "a@example..com",
    ] {
        assert_eq!(
            login_args(&CLAUDE, Some(refused)),
            ["--cli", "auth", "login", "--claudeai"],
            "{refused}"
        );
    }
}

#[tokio::test]
async fn start_refuses_before_anything_is_spawned() {
    assert_eq!(
        (can_sign_in_here(false), can_sign_in_here(true)),
        (cfg!(unix), true)
    );
    let desk = desk();
    let closed = CancellationToken::new();
    closed.cancel();
    assert_eq!(
        desk.logins
            .start(&desk.room, &desk.a, None, closed, true)
            .await,
        Err("The desktop connection has closed.".into())
    );
    #[cfg(windows)]
    assert_eq!(
        desk.logins
            .start(&desk.room, &desk.a, None, CancellationToken::new(), false)
            .await,
        Err("Sign in from the desk itself.".into())
    );

    std::fs::remove_dir(&desk.a.dir).unwrap();
    let path = paths::agent_account_dir(desk.log.root(), A).unwrap();
    assert_eq!(
        desk.logins
            .start(&desk.room, &desk.a, None, CancellationToken::new(), true)
            .await,
        Err(format!(
            "Can't open Account 1's folder ({}): entity not found. Fix the folder or pick another account.",
            path.display()
        ))
    );
    assert!(desk.logins.view(A).is_none());
}

#[tokio::test]
#[cfg_attr(windows, ignore = "needs a console window")]
async fn a_real_sign_in_succeeds_only_on_exit_zero_with_a_confirming_status() {
    for (mode, signed_before, error) in [
        (
            "fail",
            true,
            Some("Claude Code closed before it was signed in."),
        ),
        (
            "silent",
            false,
            Some("Claude Code finished, but Account 1 is not signed in."),
        ),
        ("ok", false, None),
    ] {
        let desk = desk();
        if signed_before {
            desk.plant_credentials();
        }
        desk.sign(mode, PATIENCE, CancellationToken::new()).await;
        let state = if error.is_some() { Failed } else { Succeeded };
        assert_eq!(
            desk.outcome(),
            Some((state, error.map(String::from))),
            "{mode}"
        );
        assert_eq!(
            (desk.login_at(A) > Some(5), desk.sessions()[0] == "s1"),
            (error.is_none(), error.is_some()),
            "{mode}"
        );
        let status = if mode == "silent" {
            SignedOut
        } else {
            SignedIn
        };
        assert_eq!(desk.logins.statuses.cached(A).state, status, "{mode}");
        if mode == "ok" {
            assert_eq!(
                desk.reported("login-args"),
                ["--claudeai", "--email", "ada@example.com"]
            );
            let seen = desk.reported("login-env");
            assert!(seen.iter().any(|name| name == CLAUDE.env_var));
            for name in CLAUDE.scrub.iter().filter(|name| **name != CLAUDE.env_var) {
                assert!(!seen.iter().any(|seen| seen == name), "{name}");
            }
        }
    }
}

#[tokio::test]
#[cfg_attr(windows, ignore = "needs a console window")]
async fn cancel_disconnect_and_timeout_end_a_sign_in_as_failed() {
    for (end, error) in [
        ("cancel", CANCELLED),
        ("disconnect", DISCONNECTED),
        ("timeout", TIMED_OUT),
    ] {
        let desk = desk();
        desk.plant_credentials();
        let owner = CancellationToken::new();
        let timeout = if end == "timeout" {
            Duration::from_millis(500)
        } else {
            PATIENCE
        };
        let running = tokio::spawn(desk.sign("hang", timeout, owner.clone()));
        let reported = desk.a.dir.join("fixture-login-env.json");
        until("the sign-in started", || {
            reported.exists() || running.is_finished()
        })
        .await;
        match end {
            "cancel" => {
                let id = desk.logins.view(A).unwrap().id;
                desk.logins.cancel(A, &id).unwrap();
            }
            "disconnect" => owner.cancel(),
            _ => {}
        }
        running.await.unwrap();
        assert_eq!(desk.outcome(), Some((Failed, Some(error.into()))), "{end}");
        assert_eq!(
            (desk.login_at(A), desk.sessions()[0] == "s1"),
            (Some(5), true),
            "{end}"
        );
    }
}

#[tokio::test]
#[cfg_attr(windows, ignore = "needs a console window")]
async fn a_sign_in_is_judged_by_its_own_status_not_one_already_running() {
    for (mode, in_flight, state, error) in [
        ("ok", SignedOut, Succeeded, None),
        (
            "silent",
            SignedIn,
            Failed,
            Some("Claude Code finished, but Account 1 is not signed in."),
        ),
    ] {
        let desk = desk();
        let (release, held) = oneshot::channel::<()>();
        let statuses = desk.logins.statuses.clone();
        let holding = tokio::spawn(async move {
            statuses
                .check_with(A, move || async move {
                    held.await.ok();
                    view(in_flight)
                })
                .await
        });
        until("a check holds the gate", || {
            desk.logins.statuses.checks_in_line(A) == 1
        })
        .await;
        let signing = tokio::spawn(desk.sign(mode, PATIENCE, CancellationToken::new()));
        until("the sign-in waits for its own status", || {
            desk.logins.statuses.checks_in_line(A) == 2
        })
        .await;
        release.send(()).unwrap();
        assert_eq!(holding.await.unwrap().state, in_flight, "{mode}");
        signing.await.unwrap();
        assert_eq!(
            desk.outcome(),
            Some((state, error.map(String::from))),
            "{mode}"
        );
    }
}

#[tokio::test]
async fn a_switch_held_for_a_running_sign_in_applies_once_it_is_confirmed() {
    let desk = desk();
    let attempt = desk.logins.admit(A, CancellationToken::new()).unwrap();
    desk.switch_to_a(AccountHold::SigningIn).await;
    desk.plant_credentials();
    desk.logins
        .conclude(
            &desk.room,
            &desk.run("ok", PATIENCE),
            &attempt,
            Outcome::Exited(Ok(())),
        )
        .await;
    until("the switch applies", || desk.sessions().is_empty()).await;
    assert_eq!(desk.outcome(), Some((Succeeded, None)));
    assert_eq!(desk.room.info("ada").account, None);
}

#[tokio::test]
async fn a_hold_that_ends_without_a_new_sign_in_still_applies_the_switch() {
    {
        let desk = desk();
        desk.room.account_statuses().put(A, view(SignedOut));
        desk.switch_to_a(AccountHold::NotSignedIn).await;
        desk.logins
            .statuses
            .check_with(A, || async { view(SignedIn) })
            .await;
        until("a status answer releases the switch", || {
            desk.sessions().is_empty()
        })
        .await;
    }
    {
        let desk = desk();
        let attempt = desk.logins.admit(A, CancellationToken::new()).unwrap();
        desk.switch_to_a(AccountHold::SigningIn).await;
        desk.plant_credentials();
        desk.logins
            .conclude(
                &desk.room,
                &desk.run("ok", PATIENCE),
                &attempt,
                Outcome::Stopped(CANCELLED),
            )
            .await;
        until("a cancelled sign-in releases the switch", || {
            desk.sessions().is_empty()
        })
        .await;
        assert_eq!(desk.login_at(A), Some(5));
    }
}

#[tokio::test]
async fn a_stopped_sign_in_is_failed_while_its_status_is_checked_again() {
    let desk = desk();
    let attempt = desk.logins.admit(A, CancellationToken::new()).unwrap();
    let run = Run {
        launch: acp_fixture_launch(&[("FIXTURE_STATUS_MODE", "hang")]),
        status_bound: Duration::from_millis(500),
        ..desk.run("ok", PATIENCE)
    };
    let concluding = tokio::spawn({
        let (logins, room) = (desk.logins.clone(), desk.room.clone());
        async move {
            logins
                .conclude(&room, &run, &attempt, Outcome::Stopped(CANCELLED))
                .await;
        }
    });
    until("the status runs again", || {
        desk.logins.statuses.checks_in_line(A) == 1
    })
    .await;
    assert_eq!(desk.outcome(), Some((Failed, Some(CANCELLED.into()))));
    assert_eq!(desk.room.account_live(A).0.state, Checking);
    concluding.await.unwrap();
    assert_eq!(desk.logins.statuses.cached(A).state, Unavailable);
}

#[tokio::test]
async fn a_folder_swapped_for_a_link_during_a_sign_in_is_not_signed_in() {
    let desk = desk();
    let attempt = desk.logins.admit(A, CancellationToken::new()).unwrap();
    let moved = desk.a.dir.with_file_name("moved");
    std::fs::rename(&desk.a.dir, &moved).unwrap();
    link(&moved, &desk.a.dir);
    std::fs::write(
        moved.join(".credentials.json"),
        r#"{"email":"ada@example.com"}"#,
    )
    .unwrap();
    desk.logins
        .conclude(
            &desk.room,
            &desk.run("ok", PATIENCE),
            &attempt,
            Outcome::Exited(Ok(())),
        )
        .await;
    assert_eq!(desk.outcome().map(|(state, _)| state), Some(Failed));
    assert_eq!(desk.login_at(A), Some(5));
    assert_eq!(desk.sessions(), ["s1", "s2", "s3"]);
    assert_ne!(desk.logins.statuses.cached(A).state, SignedIn);
}

#[tokio::test]
async fn a_switch_held_signed_out_stays_held_while_that_account_is_checked_again() {
    let desk = desk();
    desk.room.account_statuses().put(A, view(SignedOut));
    let (release, held) = oneshot::channel::<()>();
    let statuses = desk.logins.statuses.clone();
    let checking = tokio::spawn(async move {
        statuses
            .check_with(A, move || async move {
                held.await.ok();
                view(SignedIn)
            })
            .await
    });
    until("a check holds the gate", || {
        desk.logins.statuses.checks_in_line(A) == 1
    })
    .await;
    desk.switch_to_a(AccountHold::NotSignedIn).await;
    release.send(()).unwrap();
    checking.await.unwrap();
    until("the recorded answer releases the switch", || {
        desk.sessions().is_empty()
    })
    .await;
}

#[tokio::test]
#[cfg_attr(windows, ignore = "needs a console window")]
async fn a_real_sign_in_releases_the_switch_it_held() {
    let desk = desk();
    let signing = desk.sign("ok", PATIENCE, CancellationToken::new());
    desk.switch_to_a(AccountHold::SigningIn).await;
    signing.await;
    until("the switch applies", || desk.sessions().is_empty()).await;
    assert_eq!(desk.outcome(), Some((Succeeded, None)));
    assert_eq!(desk.room.info("ada").account, None);
}

#[cfg(unix)]
#[tokio::test]
async fn terminal_input_answers_the_cli_prompt() {
    let desk = desk();
    let running = tokio::spawn(desk.sign("prompt", PATIENCE, CancellationToken::new()));
    let id = desk.logins.view(A).unwrap().id;
    let mut output = String::new();
    until("the prompt", || {
        output.push_str(&desk.logins.poll(A, &id).unwrap().output);
        output.contains("Paste code:")
    })
    .await;
    assert_eq!(desk.logins.input(A, &id, "yes\n"), Ok(()));
    running.await.unwrap();
    assert_eq!(desk.logins.view(A).map(|view| view.state), Some(Succeeded));
}
