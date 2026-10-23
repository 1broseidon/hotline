use super::*;
use crate::accounts::support::CLAUDE;
use crate::contract::{AccountStatusState, Persona};
use crate::log::Log;
use crate::session::tests::{DeskKeys, enrol, persona};
use crate::test_support::{PATIENCE, account_on_disk, acp_fixture_launch, until};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use tempfile::TempDir;
use tokio::sync::oneshot;

fn folder() -> (TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("Conta Teste ção");
    std::fs::create_dir_all(&dir).unwrap();
    let dir = dunce::canonicalize(dir).unwrap();
    (tmp, dir)
}

fn said(answer: &Value) -> Vec<u8> {
    answer.to_string().into_bytes()
}

fn signed_in_at(dir: &Path) -> Value {
    json!({
        "loggedIn": true,
        "authMethod": "claude.ai",
        "apiProvider": "firstParty",
        "email": "ada@example.com",
        "orgId": "6f0a3c1e-2b7d-4e55-9a10-3d1c8b5e7f42",
        "orgName": "Ada's Org",
        "subscriptionType": "max",
        "analyticsDisabled": false,
        "configDirectory": dir.to_str().unwrap(),
    })
}

fn view(state: AccountStatusState, email: Option<&str>, plan: Option<&str>) -> AccountStatusView {
    AccountStatusView {
        state,
        checked_at: None,
        email: email.map(str::to_string),
        plan: plan.map(str::to_string),
    }
}

#[test]
fn signed_in_needs_a_first_party_claude_ai_login_in_this_folder() {
    let (tmp, dir) = folder();
    let signed_in = view(
        AccountStatusState::SignedIn,
        Some("ada@example.com"),
        Some("max"),
    );
    assert_eq!(
        parse_claude_status(&said(&signed_in_at(&dir)), &dir),
        signed_in
    );

    let respelled = dir.join("..").join("Conta Teste ção");
    assert_eq!(
        parse_claude_status(&said(&signed_in_at(&respelled)), &dir),
        signed_in
    );
    let mut null_source = signed_in_at(&dir);
    null_source["apiKeySource"] = Value::Null;
    assert_eq!(parse_claude_status(&said(&null_source), &dir), signed_in);
    let mut with_newline = said(&signed_in_at(&dir));
    with_newline.push(b'\n');
    assert_eq!(parse_claude_status(&with_newline, &dir), signed_in);
    drop(tmp);
}

#[test]
fn logged_out_is_signed_out_whatever_else_it_says() {
    let (_tmp, dir) = folder();
    let signed_out = view(AccountStatusState::SignedOut, None, None);
    let answer = json!({
        "loggedIn": false,
        "authMethod": "none",
        "apiProvider": "firstParty",
        "configDirectory": dir.to_str().unwrap(),
    });
    assert_eq!(parse_claude_status(&said(&answer), &dir), signed_out);
    assert_eq!(
        parse_claude_status(&said(&json!({ "loggedIn": false })), &dir),
        signed_out
    );
}

#[test]
fn any_other_answer_is_unavailable_and_says_nothing_more() {
    let (tmp, dir) = folder();
    let another = tmp.path().join("Outra Conta");
    std::fs::create_dir_all(&another).unwrap();
    let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();

    let mut with = |label, change: &dyn Fn(&mut Value)| {
        let mut answer = signed_in_at(&dir);
        change(&mut answer);
        cases.push((label, said(&answer)));
    };
    with("an API key", &|answer| {
        answer["authMethod"] = json!("api_key");
        answer["apiKeySource"] = json!("ANTHROPIC_API_KEY");
    });
    with("an OAuth token", &|answer| {
        answer["authMethod"] = json!("oauth_token");
    });
    with("a third party", &|answer| {
        answer["authMethod"] = json!("third_party");
        answer["apiProvider"] = json!("bedrock");
    });
    with("a key helper", &|answer| {
        answer["apiKeySource"] = json!("apiKeyHelper");
    });
    with("another folder", &|answer| {
        answer["configDirectory"] = json!(another.to_str().unwrap());
    });
    with("no folder", &|answer| {
        answer.as_object_mut().unwrap().remove("configDirectory");
    });
    with("a logged-in string", &|answer| {
        answer["loggedIn"] = json!("true");
    });
    cases.push(("nothing", Vec::new()));
    cases.push(("not json", b"not json".to_vec()));

    for (label, stdout) in cases {
        assert_eq!(
            parse_claude_status(&stdout, &dir),
            view(AccountStatusState::Unavailable, None, None),
            "{label}"
        );
    }
}

const A: &str = "3f6c2a1e-8b4d-4e7a-9c15-2d8e7f0a6b31";
const B: &str = "a7d94e02-1c5b-4f83-b6e9-0f2c3d4a5b68";
const CATALOGUE: &[(&str, &str)] = &[
    ("ANTHROPIC_API_KEY", "sk-catalogue"),
    ("ANTHROPIC_BASE_URL", "https://proxy.invalid"),
    ("CLAUDE_CODE_USE_BEDROCK", "1"),
];

fn scratch_log() -> (TempDir, Log) {
    let tmp = tempfile::tempdir().unwrap();
    let log = Log::open(tmp.path().join("Conta Teste ção"));
    (tmp, log)
}

async fn status(account: &ResolvedAccount, env: &[(&str, &str)]) -> AccountStatusView {
    run_status(
        &acp_fixture_launch(&[CATALOGUE, env].concat()),
        account,
        PATIENCE,
    )
    .await
}

#[tokio::test]
async fn one_status_run_per_account_and_waiters_share_its_answer() {
    use AccountStatusState::{Checking, SignedIn, SignedOut};
    let statuses = AccountStatuses::new(StartBounds::default());
    let counter = AtomicUsize::new(0);
    let runs = &counter;
    let (release, held) = oneshot::channel::<()>();
    let first = statuses.check_with("a", move || async move {
        runs.fetch_add(1, SeqCst);
        held.await.ok();
        view(SignedIn, None, None)
    });
    let second = statuses.check_with("a", move || async move {
        runs.fetch_add(1, SeqCst);
        view(SignedOut, None, None)
    });
    let other = async {
        until("a runs", || runs.load(SeqCst) == 1).await;
        assert_eq!(statuses.cached("a").state, Checking);
        let b = statuses
            .check_with("b", move || async move {
                runs.fetch_add(1, SeqCst);
                view(SignedOut, None, None)
            })
            .await;
        assert_eq!(b.state, SignedOut);
        release.send(()).unwrap();
    };
    let (a, b, ()) = tokio::join!(first, second, other);
    assert_eq!(
        (
            runs.load(SeqCst),
            a.state,
            b.state,
            a.checked_at == b.checked_at
        ),
        (2, SignedIn, SignedIn, true)
    );
    assert!(a.checked_at.is_some());

    statuses
        .check_with("a", move || async move {
            runs.fetch_add(1, SeqCst);
            view(SignedOut, None, None)
        })
        .await;
    assert_eq!(
        (runs.load(SeqCst), statuses.cached("a").state),
        (3, SignedOut)
    );
}

#[test]
fn only_an_answer_older_than_ten_minutes_is_stale() {
    let statuses = AccountStatuses::new(StartBounds::default());
    statuses.put(
        "a",
        AccountStatusView {
            checked_at: Some(1_000),
            ..view(AccountStatusState::SignedIn, None, None)
        },
    );
    assert_eq!(
        statuses.cached("never").state,
        AccountStatusState::NotChecked
    );
    assert!(!statuses.stale("never", i64::MAX));
    assert!(!statuses.stale("a", 1_000 + STALE_AFTER_MS));
    assert!(statuses.stale("a", 1_001 + STALE_AFTER_MS));
}

#[tokio::test]
async fn status_runs_in_the_account_folder_without_inherited_credentials() {
    let (_tmp, log) = scratch_log();
    let account = account_on_disk(&log, A, "Account 1", None);
    assert_eq!(
        status(&account, &[]).await.state,
        AccountStatusState::SignedOut
    );

    std::fs::write(
        account.dir.join(".credentials.json"),
        r#"{"email":"ada@example.com"}"#,
    )
    .unwrap();
    assert_eq!(
        status(&account, &[]).await,
        view(
            AccountStatusState::SignedIn,
            Some("ada@example.com"),
            Some("max")
        )
    );
    let seen: Vec<String> = serde_json::from_slice(
        &std::fs::read(account.dir.join("fixture-status-env.json")).unwrap(),
    )
    .unwrap();
    assert!(seen.iter().any(|name| name == CLAUDE.env_var));
    for name in CLAUDE.scrub.iter().filter(|name| **name != CLAUDE.env_var) {
        assert!(!seen.iter().any(|seen| seen == name), "{name}");
    }

    for auth in ["api_key", "bedrock"] {
        assert_eq!(
            status(&account, &[("FIXTURE_AUTH", auth)]).await.state,
            AccountStatusState::Unavailable,
            "{auth}"
        );
    }
}

#[tokio::test]
async fn a_status_that_does_not_answer_in_time_is_unavailable() {
    let (_tmp, log) = scratch_log();
    let account = account_on_disk(&log, A, "Account 1", None);
    let hanging = acp_fixture_launch(&[("FIXTURE_STATUS_MODE", "hang")]);
    assert_eq!(
        run_status(&hanging, &account, Duration::from_millis(300))
            .await
            .state,
        AccountStatusState::Unavailable
    );
}

#[tokio::test]
async fn refresh_rechecks_the_folder_and_covers_only_accounts_in_use() {
    let (_tmp, log) = scratch_log();
    let room = Room::new(log.clone(), Arc::new(DeskKeys));
    let a = account_on_disk(&log, A, "Account 1", None);
    let b = account_on_disk(&log, B, "Account 2", None);
    for (id, backend_id, account) in [
        ("ada", "claude-acp", A),
        ("bob", "codex-acp", B),
        ("cy", "claude-acp", "5d0c6a8e-91b4-4c1d-b3a2-7e4f8a2d6c10"),
    ] {
        enrol(
            &log,
            &Persona {
                backend_id: backend_id.to_string(),
                account_id: Some(account.to_string()),
                ..persona(id)
            },
        );
    }

    let statuses = Arc::new(AccountStatuses::new(StartBounds::default()));
    statuses.refresh_in_use(&room);
    std::fs::remove_dir(&a.dir).unwrap();
    std::fs::remove_dir(&b.dir).unwrap();
    until("A rechecked", || {
        statuses.cached(A).state == AccountStatusState::Unavailable
    })
    .await;
    assert_eq!(statuses.cached(B).state, AccountStatusState::NotChecked);
}

#[tokio::test]
async fn a_recheck_runs_its_own_status_even_behind_one_already_running() {
    use AccountStatusState::{SignedIn, SignedOut};
    let statuses = AccountStatuses::new(StartBounds::default());
    let (release, held) = oneshot::channel::<()>();
    let running = statuses.check_with("a", move || async move {
        held.await.ok();
        view(SignedOut, None, None)
    });
    let recheck = statuses.recheck_with("a", || async { view(SignedIn, None, None) });
    let releaser = async { release.send(()).unwrap() };
    let (earlier, fresh, ()) = tokio::join!(running, recheck, releaser);
    assert_eq!(
        (earlier.state, fresh.state, statuses.cached("a").state),
        (SignedOut, SignedIn, SignedIn)
    );
}
