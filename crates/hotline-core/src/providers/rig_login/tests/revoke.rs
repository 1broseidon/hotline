use super::*;
use crate::{credentials::MemoryStore, desk::Desk, wire::RoomHandle};

/// Keep the very same signing transport across revocation. Rebuilding the
/// client would only test that the picker stops selecting the dead login.
#[tokio::test]
async fn rig_revoke_copilot_retained_transport_stops_on_its_next_request() {
    for delete in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let memory = Arc::new(MemoryStore::default());
        let vault = crate::vault::Vault::open_with_store(
            root.path(),
            crate::log::Log::open(root.path()),
            memory.clone(),
        )
        .unwrap();
        let (id, dir) = vault.begin_login().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new().route(
            "/model",
            post(move || {
                counted.fetch_add(1, Ordering::SeqCst);
                async { "allowed" }
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        std::fs::write(
            dir.join(COPILOT_RECORD),
            json!({
                "token": "copilot-token", "expires_at": chrono::Utc::now().timestamp() + 3600,
                "endpoints": {"api": base}
            })
            .to_string(),
        )
        .unwrap();
        vault
            .finish_login(&id, "github-copilot", "Copilot")
            .unwrap();
        let desk = Desk::open_with_store(root.path(), memory).unwrap();
        let login = copilot(&dir);
        let transport =
            crate::providers::outgoing::Outgoing::signed(login.clone(), std::convert::identity)
                .unwrap();
        let request = || {
            http::Request::post(format!("{base}/model"))
                .body(Bytes::new())
                .unwrap()
        };
        assert!(transport.send::<_, Bytes>(request()).await.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        if delete {
            desk.credential_delete(&id).await.unwrap();
        } else {
            desk.credential_revoke(&id).unwrap();
        }
        assert!(transport.send::<_, Bytes>(request()).await.is_err());
        assert!(transport.send_streaming(request()).await.is_err());
        assert!(
            login.authenticator().is_err(),
            "the retained cache was dropped"
        );
        assert!(
            !SHARED.lock().unwrap().contains_key(&dir),
            "registry evicted"
        );
        assert!(
            copilot(&dir).context().await.is_err(),
            "a fresh lookup cannot revive it"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "no request left after revocation"
        );
        server.abort();
    }
}

#[tokio::test]
async fn rig_revoke_chatgpt_forgets_retained_tokens_and_refuses_a_new_lookup() {
    let root = tempfile::tempdir().unwrap();
    let memory = Arc::new(MemoryStore::default());
    let vault = crate::vault::Vault::open_with_store(
        root.path(),
        crate::log::Log::open(root.path()),
        memory.clone(),
    )
    .unwrap();
    let (id, dir) = vault.begin_login().unwrap();
    chatgpt_0_42(
        &dir,
        &access_token("access-1", 3600),
        chrono::Utc::now().timestamp() + 3600,
    );
    vault.finish_login(&id, "openai-codex", "ChatGPT").unwrap();
    let desk = Desk::open_with_store(root.path(), memory).unwrap();
    let login = chatgpt(&dir);
    assert!(login.context().await.is_ok());
    desk.credential_revoke(&id).unwrap();
    assert!(login.context().await.is_err());
    assert!(login.authenticator().is_err());
    assert!(!SHARED.lock().unwrap().contains_key(&dir));
    assert!(chatgpt(&dir).context().await.is_err());
}

#[tokio::test]
async fn rig_revoke_a_refresh_in_flight_cannot_restore_the_login() {
    let root = tempfile::tempdir().unwrap();
    let memory = Arc::new(MemoryStore::default());
    let vault = crate::vault::Vault::open_with_store(
        root.path(),
        crate::log::Log::open(root.path()),
        memory.clone(),
    )
    .unwrap();
    let (id, dir) = vault.begin_login().unwrap();
    chatgpt_0_42(
        &dir,
        &access_token("expired", -60),
        chrono::Utc::now().timestamp() - 60,
    );
    vault.finish_login(&id, "openai-codex", "ChatGPT").unwrap();
    let desk = Desk::open_with_store(root.path(), memory).unwrap();
    let (entered, mut entering) = tokio::sync::mpsc::unbounded_channel();
    let release = Arc::new(tokio::sync::Notify::new());
    let released = release.clone();
    let app = Router::new().route("/oauth/token", post(move || {
        let release = released.clone();
        entered.send(()).unwrap();
        async move {
            release.notified().await;
            json!({"access_token": access_token("fresh", 3600), "refresh_token": "fresh-refresh"}).to_string()
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http = ToStub {
        base: format!("http://{}", listener.local_addr().unwrap()),
        http: ReqwestClient::default(),
    };
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let login = chatgpt(&dir);
    let pending = tokio::spawn(async move { login.auth_context(&http).await });
    tokio::time::timeout(Duration::from_secs(10), entering.recv())
        .await
        .unwrap()
        .unwrap();
    desk.credential_revoke(&id).unwrap();
    release.notify_one();
    assert!(pending.await.unwrap().is_err());
    let record = LoginDirStore::new(&dir)
        .load(chatgpt::PROVIDER_NAME)
        .await
        .unwrap()
        .unwrap();
    assert!(
        !record.contains("fresh-refresh"),
        "the revoked login cannot save a refresh"
    );
    assert!(chatgpt(&dir).context().await.is_err());
    server.abort();
}
