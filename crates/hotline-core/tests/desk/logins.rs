use super::*;

#[tokio::test]
async fn rig_vault_deleting_a_login_over_the_wire_removes_its_chunks() {
    use hotline_core::credentials::{CredentialFiles, MemoryStore};
    let root = tempfile::tempdir().unwrap();
    let memory = Arc::new(MemoryStore::default());
    let log = hotline_core::log::Log::open(root.path());
    let vault =
        hotline_core::vault::Vault::open_with_store(root.path(), log, memory.clone()).unwrap();
    let files = CredentialFiles::new(root.path().into(), memory.clone());
    let mut logins = Vec::new();
    for (provider, record) in [
        ("openai-codex", "auth.json"),
        ("github-copilot", "copilot.json"),
    ] {
        let (id, dir) = vault.begin_login().unwrap();
        files
            .file(dir.join(record))
            .write(
                json!({"token": "private-token".repeat(500)})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
        vault.finish_login(&id, provider, provider).unwrap();
        logins.push((id, dir));
    }
    drop(vault);
    let login_keys: Vec<_> = memory.0.lock().unwrap().keys().cloned().collect();
    let desk = hotline_core::desk::Desk::open_with_store(root.path(), memory.clone()).unwrap();
    let parts = desk.parts();
    let door = Door::bind(desk.log.clone(), TOKEN.into(), Arc::new(desk), parts).unwrap();
    let port = door.port();
    let running = tokio::spawn(door.run());
    let mut client = Client::connect(port, TOKEN).await;
    assert!(memory.0.lock().unwrap().len() > 2);
    for (id, dir) in logins {
        let deleted = client.call("credential.delete", json!({"id":id})).await;
        assert_eq!(deleted["ok"], true, "{deleted}");
        assert!(!dir.exists());
    }
    let kept = memory.0.lock().unwrap();
    assert!(login_keys.iter().all(|key| !kept.contains_key(key)));
    running.abort();
}

#[tokio::test]
async fn legacy_mcp_secrets_are_hidden_and_repair_removes_the_plaintext_history() {
    let root = scratch("legacy-mcp-redaction");
    let log = hotline_core::log::Log::open(&root);
    log.append(&hotline_core::log::StreamId::Room, &json!({
        "kind":"setting", "id":"mcpServers", "value":[
            {"id":"remote", "name":"Remote", "type":"http", "url":"https://example.com/mcp?token=legacy-url-secret"},
            {"id":"local", "name":"Local", "type":"stdio", "command":"example-server", "args":["legacy-argument-secret"], "env":{"TOKEN":"legacy-env-secret"}}
        ]
    })).unwrap();
    drop(log);
    let port = open_at(&root);
    let mut client = Client::connect(port, TOKEN).await;
    let subscription = client.subscribe(json!("room")).await;
    let snapshot = client
        .next_where(Duration::from_secs(5), |frame| {
            is_sub(frame, subscription, "snapshot")
        })
        .await;
    for secret in [
        "legacy-url-secret",
        "legacy-argument-secret",
        "legacy-env-secret",
    ] {
        assert!(!snapshot.to_string().contains(secret));
    }
    let setting = snapshot["snapshot"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["id"] == "mcpServers")
        .unwrap();
    let public = &setting["value"];
    assert_eq!(public[0]["url"], "");
    assert_eq!(public[0]["urlNeedsRepair"], true);
    assert_eq!(public[1]["launchValuesPending"], true);
    let refused = client
        .call("settings.update", json!({"patch":{"mcpServers":public}}))
        .await;
    assert_eq!(refused["ok"], false);
    let updated = client.call("settings.update", json!({"patch":{"mcpServers":[
        {"id":"remote", "name":"Remote", "type":"http", "url":"https://example.com/mcp"},
        {"id":"local", "name":"Local", "type":"stdio", "command":"example-server", "args":["replacement-secret"]}
    ]}})).await;
    assert_eq!(updated["ok"], true, "{updated}");
    assert!(!updated.to_string().contains("replacement-secret"));
    let disk = std::fs::read_to_string(root.join("room.jsonl")).unwrap();
    for secret in [
        "legacy-url-secret",
        "legacy-argument-secret",
        "legacy-env-secret",
        "replacement-secret",
    ] {
        assert!(!disk.contains(secret));
    }
    assert!(disk.contains("credentialRef"));
}

#[tokio::test]
async fn grok_signout_removes_tokens_and_zai_plans_have_separate_connections() {
    let root = scratch("grok-zai-connections");
    let log = hotline_core::log::Log::open(&root);
    let vault = common::open_vault(&root, log).unwrap();
    // A completed fake login is written before the core starts, like an
    // imported credential. The wire must never expose this private half.
    let (id, dir) = vault.begin_login().unwrap();
    std::fs::write(dir.join("auth.json"), json!({"access_token":"private-grok-access", "refresh_token":"private-grok-refresh", "refresh_at":u64::MAX}).to_string()).unwrap();
    vault.finish_login(&id, "xai", "SuperGrok").unwrap();
    drop(vault);
    let port = open_at(&root);
    let mut client = Client::connect(port, TOKEN).await;
    let providers = client.call("providers.list", json!({})).await;
    let providers = providers["result"].as_array().unwrap();
    for (id, kinds) in [
        ("xai", json!(["oauth", "api_key"])),
        ("zai", json!(["api_key"])),
        ("zai-coding-plan", json!(["api_key"])),
    ] {
        assert_eq!(
            providers.iter().find(|p| p["id"] == id).unwrap()["credentialKinds"],
            kinds
        );
    }
    let credentials = client.call("credential.list", json!({})).await;
    assert_eq!(credentials["result"][0]["credentialKind"], "oauth");
    assert!(!credentials.to_string().contains("private-grok"));
    let models = client.call("models.list", json!({})).await;
    assert!(
        models["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"].as_str().unwrap().starts_with("xai/"))
    );
    let revoked = client.call("credential.revoke", json!({"id":id})).await;
    assert_eq!(revoked["ok"], true, "{revoked}");
    assert!(!dir.exists());
    assert_eq!(
        client.call("models.list", json!({})).await["result"],
        json!([])
    );

    let standard = client
        .call(
            "credential.create",
            json!({"providerId":"zai", "label":"standard", "secret":"private-standard-key"}),
        )
        .await;
    assert_eq!(standard["ok"], true, "{standard}");
    let models = client.call("models.list", json!({})).await;
    assert_ne!(
        models["result"].as_array().unwrap().as_slice(),
        [] as [Value; 0]
    );
    assert!(
        models["result"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["id"].as_str().unwrap().starts_with("zai/"))
    );
    let coding = client.call("credential.create", json!({"providerId":"zai-coding-plan", "label":"coding", "secret":"private-coding-key"})).await;
    assert_eq!(coding["ok"], true, "{coding}");
    let removed = client
        .call("credential.delete", json!({"id":standard["result"]["id"]}))
        .await;
    assert_eq!(removed["ok"], true, "{removed}");
    let models = client.call("models.list", json!({})).await;
    assert_ne!(
        models["result"].as_array().unwrap().as_slice(),
        [] as [Value; 0]
    );
    assert!(
        models["result"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["id"].as_str().unwrap().starts_with("zai-coding-plan/"))
    );
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let made = client.call("persona.create", json!({"draft":{"name":"Zai tester", "goal":"Check the connection.", "cwd":workspace.to_string_lossy()}})).await;
    let started = client
        .call("session.start", json!({"personaId":made["result"]["id"]}))
        .await;
    assert_eq!(started["ok"], true, "{started}");
    let credentials = client.call("credential.list", json!({})).await;
    assert!(!credentials.to_string().contains("private-"));
}

/// A subscription's account list cannot be refreshed without a login. The
/// fake room is allowed to stub this; the desk is the one that knows.
#[tokio::test(flavor = "multi_thread")]
async fn credential_refresh_models_needs_a_login() {
    let (_root, port) = open("refresh-models").await;
    let mut client = Client::connect(port, TOKEN).await;
    let refused = client
        .call(
            "credential.refresh_models",
            json!({ "providerId": "github-copilot" }),
        )
        .await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(
        refused["error"].as_str(),
        Some("There is no connection for github-copilot.")
    );
}

/// The catalogue is listed whether or not a credential is held, and a saved
/// filter flags it. The fake room cannot see settings, so this is the desk.
#[tokio::test(flavor = "multi_thread")]
async fn models_catalog_flags_a_saved_filter_and_refuses_an_unwired_provider() {
    let (_root, port) = open("models-catalog").await;
    let mut client = Client::connect(port, TOKEN).await;

    let nope = client
        .call("models.catalog", json!({ "providerId": "nope" }))
        .await;
    assert_eq!(nope["ok"], false, "{nope}");
    assert_eq!(
        nope["error"].as_str(),
        Some("nope is not a provider Hotline Agent can use.")
    );

    let listed = client
        .call("models.catalog", json!({ "providerId": "anthropic" }))
        .await;
    assert_eq!(listed["ok"], true, "{listed}");
    let models = listed["result"].as_array().expect("a catalogue is a list");
    assert_ne!(models.as_slice(), [] as [Value; 0]);
    assert!(
        models.iter().all(|model| model["enabled"] == true),
        "{listed}"
    );
    let kept = models[0]["id"].as_str().unwrap().to_string();

    let patched = client
        .call(
            "settings.update",
            json!({ "patch": { "enabledModels": { "anthropic": [kept] } } }),
        )
        .await;
    assert_eq!(patched["ok"], true, "{patched}");

    let filtered = client
        .call("models.catalog", json!({ "providerId": "anthropic" }))
        .await;
    assert_eq!(filtered["ok"], true, "{filtered}");
    let flagged = filtered["result"].as_array().unwrap();
    assert_eq!(flagged.len(), models.len());
    let on: Vec<&str> = flagged
        .iter()
        .filter(|model| model["enabled"] == true)
        .map(|model| model["id"].as_str().unwrap())
        .collect();
    assert_eq!(on, [kept.as_str()]);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_openrouter_login_closes_callback_and_discards_pending_credentials() {
    let (root, port) = open("openrouter-cancel").await;
    let mut client = Client::connect(port, TOKEN).await;
    let prompt = client
        .call("credential.login", json!({"providerId":"openrouter"}))
        .await;
    assert_eq!(prompt["ok"], true, "{prompt}");
    assert_eq!(prompt["result"]["userCode"], "");
    let id = prompt["result"]["loginId"].as_str().unwrap();
    let authorize = url::Url::parse(prompt["result"]["verificationUri"].as_str().unwrap()).unwrap();
    assert_eq!(authorize.host_str(), Some("openrouter.ai"));
    let callback = authorize
        .query_pairs()
        .find(|(key, _)| key == "callback_url")
        .unwrap()
        .1
        .into_owned();
    let http = reqwest::Client::builder()
        .no_proxy()
        .pool_max_idle_per_host(0)
        .build()
        .unwrap();
    assert_eq!(http.get(&callback).send().await.unwrap().status(), 400);
    let cancel = client
        .call("credential.login_cancel", json!({"loginId":id}))
        .await;
    assert_eq!(cancel["ok"], true, "{cancel}");
    let status = client
        .call("credential.login_status", json!({"loginId":id}))
        .await;
    assert_eq!(status["result"]["state"], "failed");
    let dir = root.join("vault/logins").join(id);
    tokio::time::timeout(Duration::from_secs(3), async {
        while dir.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    // Login cleanup aborts the callback task; removing the token directory
    // does not wait for that task to drop its listener.
    tokio::time::timeout(Duration::from_secs(3), async {
        while http.get(&callback).send().await.is_ok() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the cancelled OpenRouter callback listener did not close");
    assert_eq!(
        client.call("credential.list", json!({})).await["result"],
        json!([])
    );
}

/// The real wire handler must remove the login even when OpenAI refuses
/// revocation. Each stored token is presented once, and a repeated delete
/// cannot spend it again.
#[tokio::test]
async fn rig_revoke_delete_calls_provider_once_per_token_even_when_it_fails() {
    use axum::{Router, body::Bytes, http::StatusCode, routing::post};
    use hotline_core::{credentials::MemoryStore, provider_fixture::Origin};
    use std::sync::Mutex;
    for status in [StatusCode::OK, StatusCode::INTERNAL_SERVER_ERROR] {
        let root = tempfile::tempdir().unwrap();
        let memory = Arc::new(MemoryStore::default());
        let log = hotline_core::log::Log::open(root.path());
        let vault =
            hotline_core::vault::Vault::open_with_store(root.path(), log, memory.clone()).unwrap();
        let (id, dir) = vault.begin_login().unwrap();
        std::fs::write(
            dir.join("auth.json"),
            json!({
                "refresh_token":"refresh-revoke", "access_token":"access-revoke",
                "expires_at":i64::MAX
            })
            .to_string(),
        )
        .unwrap();
        vault.finish_login(&id, "openai-codex", "ChatGPT").unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = calls.clone();
        let app = Router::new().route(
            "/api/accounts/oauth/revoke",
            post(move |body: Bytes| {
                let form: std::collections::HashMap<String, String> =
                    url::form_urlencoded::parse(&body).into_owned().collect();
                seen.lock()
                    .unwrap()
                    .push((form["token"].clone(), form["token_type_hint"].clone()));
                async move { status }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let _fixture = Origin::new(dir.to_string_lossy(), &origin);
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        drop(vault);
        let desk = hotline_core::desk::Desk::open_with_store(root.path(), memory).unwrap();
        let parts = desk.parts();
        let door = Door::bind(desk.log.clone(), TOKEN.into(), Arc::new(desk), parts).unwrap();
        let port = door.port();
        let running = tokio::spawn(door.run());
        let mut client = Client::connect(port, TOKEN).await;
        let deleted = client.call("credential.delete", json!({"id":id})).await;
        assert_eq!(deleted["ok"], true, "{deleted}");
        assert!(!dir.exists());
        assert_eq!(
            client.call("credential.list", json!({})).await["result"],
            json!([])
        );
        assert_eq!(
            client.call("credential.delete", json!({"id":id})).await["ok"],
            false
        );
        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                ("refresh-revoke".into(), "refresh_token".into()),
                ("access-revoke".into(), "access_token".into()),
            ]
        );
        running.abort();
        server.abort();
    }
}
