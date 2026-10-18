use super::*;

#[tokio::test]
async fn oauth_proxy_requires_its_own_bearer_and_a_live_grant() {
    use rmcp::transport::auth::{
        AuthorizationManager, CredentialStore, InMemoryCredentialStore, StoredCredentials,
    };

    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let upstream = Router::new().fallback(any(move |request: Request<Body>| {
        let seen = seen.clone();
        async move {
            assert_eq!(
                request.headers()[AUTHORIZATION],
                "Bearer upstream-oauth-token"
            );
            seen.fetch_add(1, Ordering::SeqCst);
            "tool result"
        }
    }));
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let serving = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let credentials: StoredCredentials = serde_json::from_value(serde_json::json!({
        "client_id": "test-client",
        "token_response": { "access_token": "upstream-oauth-token", "token_type": "Bearer" }
    }))
    .unwrap();
    let store = InMemoryCredentialStore::new();
    store.save(credentials).await.unwrap();
    let mut manager = AuthorizationManager::new(&url).await.unwrap();
    manager.set_credential_store(store);
    let epoch = crate::driver::CapabilityEpoch::default();
    let state = Arc::new(OAuthProxyState {
        token: "session-capability".to_string(),
        upstream: url.parse().unwrap(),
        upstream_path: "/mcp".to_string(),
        client: reqwest::Client::new(),
        credential: ProxyCredential::Oauth {
            manager: Box::new(tokio::sync::Mutex::new(manager)),
            token_lock: Arc::new(tokio::sync::Mutex::new(())),
        },
        capability: Some(epoch.lease()),
    });
    let request = |token: Option<&str>| {
        let mut builder = Request::builder().method("POST").uri("/mcp");
        if let Some(token) = token {
            builder = builder.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        builder.body(Body::empty()).unwrap()
    };
    for token in [
        None,
        Some("another-session-capability"),
        Some("upstream-oauth-token"),
    ] {
        let response = oauth_proxy_request(State(state.clone()), request(token)).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    let response =
        oauth_proxy_request(State(state.clone()), request(Some("session-capability"))).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap(),
        "tool result"
    );
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    epoch.invalidate();
    let response = oauth_proxy_request(State(state), request(Some("session-capability"))).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    serving.abort();
}
