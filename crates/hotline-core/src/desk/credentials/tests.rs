use super::*;
use crate::contract::LoginState;
use crate::credentials::MemoryStore;
use crate::providers::rig_login::{COPILOT_RECORD, LoginDirStore};
use axum::{Router, http::HeaderMap, routing::get};
use rig::providers::chatgpt::{self, auth::TokenStore};
use std::future::Future;
use std::task::Poll;

fn desk() -> (tempfile::TempDir, Desk) {
    let root = tempfile::tempdir().unwrap();
    let desk = Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap();
    (root, desk)
}

/// A sign-in that finished in `from`'s folder and signs `into` in again.
fn signed_in_again(desk: &Desk, from: &str, into: &str, client: Client) -> SignIn {
    desk.logins().insert(
        from.to_string(),
        LoginOutcome::Pending(CancellationToken::new()),
    );
    SignIn {
        vault: desk.vault.clone(),
        logins: desk.logins.clone(),
        log: desk.log.clone(),
        id: from.to_string(),
        client,
        landing: Landing::Again {
            into: into.to_string(),
        },
    }
}

fn copilot_record(dir: &Path, token: &str, api: &str) {
    let expires_at = chrono::Utc::now().timestamp() + 3600;
    std::fs::write(
        dir.join(COPILOT_RECORD),
        json!({"token": token, "expires_at": expires_at, "endpoints": {"api": api}}).to_string(),
    )
    .unwrap();
}

#[tokio::test]
async fn a_copilot_account_signed_in_again_lists_the_new_sign_ins_models() {
    let app = Router::new().route(
        "/models",
        get(|headers: HeaderMap| async move {
            let id = if headers["authorization"] == "Bearer new-token" {
                "gpt-new"
            } else {
                "gpt-old"
            };
            json!({"data": [{"id": id}]}).to_string()
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (_root, desk) = desk();
    let (into, into_dir) = desk.vault.begin_login().unwrap();
    copilot_record(&into_dir, "old-token", &api);
    desk.vault
        .finish_login(&into, "github-copilot", "GitHub Copilot")
        .unwrap();
    crate::vault::write_account_models(&into_dir, &["gpt-old".into()]).unwrap();
    let (from, from_dir) = desk.vault.begin_login().unwrap();
    copilot_record(&from_dir, "new-token", &api);

    signed_in_again(&desk, &from, &into, Client::Copilot)
        .land(Ok(()))
        .await;

    let landed = desk.login_status(&from).unwrap().credential.unwrap();
    assert_eq!(landed.id, into);
    assert!(!from_dir.exists());
    assert_eq!(
        desk.vault.account_models().get("github-copilot"),
        Some(&vec!["gpt-new".to_string()])
    );
    server.abort();
}

#[tokio::test]
async fn a_refresh_begun_on_the_old_tokens_saves_before_a_sign_in_again_never_after() {
    let (_root, desk) = desk();
    let (into, into_dir) = desk.vault.begin_login().unwrap();
    desk.vault.login_tokens(&into).write(b"old").unwrap();
    desk.vault
        .finish_login(&into, "openai-codex", "ChatGPT")
        .unwrap();
    let (from, _) = desk.vault.begin_login().unwrap();
    desk.vault.login_tokens(&from).write(b"new").unwrap();

    let store = LoginDirStore::new(&into_dir);
    let refreshing = store.lock(chatgpt::PROVIDER_NAME).await.unwrap();
    let mut landing = Box::pin(signed_in_again(&desk, &from, &into, Client::ChatGpt).land(Ok(())));
    let first = std::future::poll_fn(|cx| Poll::Ready(landing.as_mut().poll(cx))).await;
    assert!(first.is_pending(), "the sign-in waits for the refresh");
    store
        .save(chatgpt::PROVIDER_NAME, "refreshed-old")
        .await
        .unwrap();
    drop(refreshing);
    landing.await;

    assert_eq!(desk.login_status(&from).unwrap().state, LoginState::Done);
    assert_eq!(
        desk.vault.login_tokens(&into).read().unwrap().as_deref(),
        Some(&b"new"[..])
    );
}
