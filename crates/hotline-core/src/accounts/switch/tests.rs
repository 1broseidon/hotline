use super::*;
use AccountStatusState::{Checking, NotChecked, SignedIn, SignedOut, Unavailable};
use LoginAttemptState::{Failed, Running};

fn status(state: AccountStatusState) -> AccountStatusView {
    AccountStatusView {
        state,
        checked_at: None,
        email: None,
        plan: None,
    }
}

fn attempt(state: LoginAttemptState) -> LoginAttemptView {
    LoginAttemptView {
        id: "a1".into(),
        state,
        error: None,
    }
}

#[test]
fn a_running_attempt_holds_as_signing_in_and_signed_out_as_not_signed_in() {
    let held = |state, running: Option<LoginAttemptState>| {
        hold(&status(state), running.map(attempt).as_ref())
    };
    assert_eq!(held(SignedOut, None), Some(AccountHold::NotSignedIn));
    assert_eq!(held(SignedIn, Some(Running)), Some(AccountHold::SigningIn));
    for state in [NotChecked, Checking, Unavailable, SignedIn] {
        for running in [None, Some(Failed)] {
            assert_eq!(held(state, running), None, "{state:?} {running:?}");
        }
    }
}

#[test]
fn the_session_account_shows_a_switch_until_it_applies() {
    let (a, b) = (Some("A".to_string()), Some("B".to_string()));
    let switching = PendingAccount {
        backend_id: "claude-acp".to_string(),
        from: a.clone(),
        wanted: b.clone(),
        relogin: false,
    };
    assert_eq!(session_account(None, None, None), None);
    assert_eq!(
        session_account(a.clone(), None, None),
        Some(SessionAccount {
            running: a.clone(),
            wanted: a.clone(),
            held: None
        })
    );
    assert_eq!(
        session_account(a.clone(), Some(&switching), Some(AccountHold::NotSignedIn)),
        Some(SessionAccount {
            running: a,
            wanted: b,
            held: Some(AccountHold::NotSignedIn)
        })
    );
}
