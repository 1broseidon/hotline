use super::*;

const ACCOUNTS_PATH: &str = "wire::tests::accounts::";
const MISSING_SETUP_CASE: &str = "HOTLINE_TEST_MISSING_SETUP_CASE";

pub(super) fn account_frames(name: &str) -> Vec<Value> {
    let attempt = json!({"accountId":"acct-1","id":"fixture-attempt"});
    [
        ("accounts.list", json!({})),
        (
            "accounts.create",
            json!({"backendId":"claude-acp","name":name,"startWith":"empty"}),
        ),
        (
            "accounts.rename",
            json!({"accountId":"acct-1","name":"Renamed"}),
        ),
        ("accounts.remove", json!({"accountId":"acct-1"})),
        ("accounts.status", json!({"accountId":"acct-1"})),
        ("accounts.login.start", json!({"accountId":"acct-1"})),
        ("accounts.login.poll", attempt.clone()),
        (
            "accounts.login.input",
            json!({"accountId":"acct-1","id":"fixture-attempt","input":"fixture input"}),
        ),
        ("accounts.login.cancel", attempt),
        ("accounts.setup.list", json!({"accountId":"acct-1"})),
        (
            "accounts.setup.copy",
            json!({"accountId":"acct-1","categories":["instructions"]}),
        ),
        (
            "persona.account",
            json!({"personaId":"ada","accountId":"acct-1"}),
        ),
    ]
    .into_iter()
    .map(|(cmd, params)| json!({"id": 1, "cmd": cmd, "params": params}))
    .collect()
}

pub(super) async fn run(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    cmd: &str,
    params: Value,
) -> Result<Value, String> {
    let command = crate::wire::socket::read_command(&json!({"cmd": cmd, "params": params}))?;
    crate::wire::commands::run(command, log, room, &Parts::default()).await
}

pub(super) async fn created(log: &Log, room: &Arc<dyn RoomHandle>, name: &str) -> Value {
    run(
        log,
        room,
        "accounts.create",
        json!({"backendId":"claude-acp","name":name,"startWith":"empty"}),
    )
    .await
    .unwrap()
}

fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
    dunce::canonicalize(a).unwrap() == dunce::canonicalize(b).unwrap()
}

#[tokio::test]
async fn a_name_still_being_created_is_refused_before_a_second_folder_is_made() {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let room: Arc<dyn RoomHandle> = Arc::new(Quiet::new());
    let create = |name: &str| {
        run(
            &log,
            &room,
            "accounts.create",
            json!({"backendId":"claude-acp","name":name,"startWith":"empty"}),
        )
    };

    let (first, second) = tokio::join!(create("Reviewer"), create("reviewer"));

    let first = first.unwrap();
    assert_eq!(
        second.unwrap_err(),
        "An account named reviewer is still being made for that agent."
    );
    let folders = std::fs::read_dir(paths::agent_accounts_dir(root.path()))
        .unwrap()
        .count();
    assert_eq!(folders, 1);
    let id = first["account"]["account"]["id"].as_str().unwrap();
    run(&log, &room, "accounts.remove", json!({"accountId": id}))
        .await
        .unwrap();
    created(&log, &room, "reviewer").await;
}

#[tokio::test]
async fn renaming_to_a_name_still_being_created_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let room: Arc<dyn RoomHandle> = Arc::new(Quiet::new());
    let personal = created(&log, &room, "Personal").await;
    let id = personal["account"]["account"]["id"].as_str().unwrap();

    let (made, renamed) = tokio::join!(
        run(
            &log,
            &room,
            "accounts.create",
            json!({"backendId":"claude-acp","name":"Work","startWith":"empty"}),
        ),
        run(
            &log,
            &room,
            "accounts.rename",
            json!({"accountId": id, "name": "work"}),
        ),
    );

    assert!(made.is_ok(), "{made:?}");
    assert_eq!(
        renamed.unwrap_err(),
        "An account named work is still being made for that agent."
    );
}

#[tokio::test]
async fn creating_an_account_makes_its_private_folder_and_ignores_a_client_path() {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let room: Arc<dyn RoomHandle> = Arc::new(Quiet::new());

    let created = run(
        &log,
        &room,
        "accounts.create",
        json!({"backendId":"claude-acp","name":" Account 1 ","startWith":"empty","path":"/elsewhere"}),
    )
    .await
    .unwrap();
    let id = created["account"]["account"]["id"].as_str().unwrap();
    let folder = paths::agent_account_dir(root.path(), id).unwrap();
    assert_eq!(created["account"]["account"]["name"], "Account 1");
    assert_eq!(
        created["account"]["folder"],
        json!(folder.to_string_lossy())
    );
    assert!(folder.is_dir());
    assert!(created["copy"].is_null(), "{created}");
    assert_eq!(created["account"]["account"]["setup"]["from"], "empty");
    assert_eq!(
        created["account"]["account"]["setup"]["categories"],
        json!([])
    );

    for accepted in ["Conta Teste ção".to_string(), "🙂".repeat(64)] {
        run(
            &log,
            &room,
            "accounts.create",
            json!({"backendId":"claude-acp","name":accepted,"startWith":"empty"}),
        )
        .await
        .unwrap();
    }
    let long = "a".repeat(65);
    let emoji = "🙂".repeat(65);
    for (name, backend, refusal) in [
        ("  ", "claude-acp", "An account needs a name."),
        (
            long.as_str(),
            "claude-acp",
            "An account name is at most 64 characters.",
        ),
        (
            emoji.as_str(),
            "claude-acp",
            "An account name is at most 64 characters.",
        ),
        (
            "ACCOUNT 1",
            "claude-acp",
            "There is already an account named ACCOUNT 1 for that agent.",
        ),
        (
            "conta teste ÇÃO",
            "claude-acp",
            "There is already an account named conta teste ÇÃO for that agent.",
        ),
        ("Account 9", "hotline", "That agent does not use accounts."),
    ] {
        let refused = run(
            &log,
            &room,
            "accounts.create",
            json!({"backendId":backend,"name":name,"startWith":"empty"}),
        )
        .await;
        assert_eq!(refused, Err(refusal.to_string()));
    }
    assert_eq!(
        std::fs::read_dir(paths::agent_accounts_dir(root.path()))
            .unwrap()
            .count(),
        3
    );

    let renamed = run(
        &log,
        &room,
        "accounts.rename",
        json!({"accountId":id,"name":"Account 2"}),
    )
    .await
    .unwrap();
    assert_eq!(renamed["account"]["name"], "Account 2");

    let listed = run(&log, &room, "accounts.list", json!({})).await.unwrap();
    assert_eq!(
        listed["agents"],
        json!([{"backendId":"cursor","name":"Cursor","supportsAccounts":false,"takesEmail":false}])
    );
    let account = &listed["accounts"][0];
    assert_eq!(account["account"]["name"], "Account 2");
    assert_eq!(account["usedBy"], json!([]));
    assert_eq!(account["folder"], json!(folder.to_string_lossy()));
    assert_eq!(
        account["canSignInHere"],
        crate::accounts::login::can_sign_in_here(false)
    );
    assert_eq!(account["status"]["state"], "not_checked");
    assert_eq!(room::agent_accounts(&log).len(), 3);
}

#[tokio::test]
async fn setup_is_read_and_copied_only_in_a_checked_folder() {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let room: Arc<dyn RoomHandle> = Arc::new(Quiet::new());
    let created = created(&log, &room, "Account 1").await;
    let id = created["account"]["account"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let folder = paths::agent_account_dir(root.path(), &id).unwrap();

    let listed = run(&log, &room, "accounts.setup.list", json!({"accountId":id}))
        .await
        .unwrap();
    let categories = listed["categories"].as_array().unwrap();
    let ids: Vec<&str> = categories
        .iter()
        .map(|category| category["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        ["instructions", "skills", "plugins", "mcp", "settings"]
    );
    for category in categories {
        assert_eq!(category["count"], 0, "{category}");
        let path = std::path::Path::new(category["path"].as_str().unwrap());
        assert!(same_dir(path.parent().unwrap(), &folder), "{category}");
    }
    assert_eq!(listed["setup"]["from"], "empty");
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);

    assert_eq!(
        run(
            &log,
            &room,
            "accounts.setup.copy",
            json!({"accountId":id,"categories":[]}),
        )
        .await,
        Err("Choose at least one item to copy.".to_string())
    );

    std::fs::remove_dir_all(&folder).unwrap();
    for (cmd, params) in [
        ("accounts.setup.list", json!({"accountId":id})),
        (
            "accounts.setup.copy",
            json!({"accountId":id,"categories":["instructions"]}),
        ),
    ] {
        let error = run(&log, &room, cmd, params).await.unwrap_err();
        assert!(
            error.starts_with("Can't open Account 1's folder ("),
            "{error}"
        );
    }
    assert!(!folder.exists());
}

#[tokio::test]
async fn creating_from_a_missing_setup_writes_no_account_and_no_folder() {
    let Ok(_case) = std::env::var(MISSING_SETUP_CASE) else {
        let missing = tempfile::tempdir().unwrap();
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                &format!(
                    "{ACCOUNTS_PATH}creating_from_a_missing_setup_writes_no_account_and_no_folder"
                ),
                "--nocapture",
            ])
            .env(MISSING_SETUP_CASE, "child")
            .env("CLAUDE_CONFIG_DIR", missing.path().join("missing"))
            .kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_secs(60), command.output())
            .await
            .expect("the isolated create finished")
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    };

    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(
            root.path(),
            Arc::new(crate::credentials::MemoryStore::default()),
        )
        .unwrap(),
    );
    let room: Arc<dyn RoomHandle> = desk.clone();
    let create = |start_with: &str| {
        json!({"id":1,"cmd":"accounts.create","params":{
            "backendId":"claude-acp","name":"Account 1","startWith":start_with}})
    };
    let refused = remote_control_answer(
        Seat::Desk,
        &room,
        &desk.log,
        &desk.parts(),
        create("system_default"),
    )
    .await;
    assert_eq!(
        refused["error"], "There is no Claude Code setup on this computer to copy.",
        "{refused}"
    );
    let listed = remote_control_answer(
        Seat::Desk,
        &room,
        &desk.log,
        &desk.parts(),
        json!({"id":2,"cmd":"accounts.list","params":{}}),
    )
    .await;
    assert_eq!(listed["result"]["accounts"], json!([]), "{listed}");
    assert!(room::agent_accounts(&desk.log).is_empty());
    let accounts_dir = paths::agent_accounts_dir(root.path());
    assert!(
        std::fs::read_dir(&accounts_dir)
            .ok()
            .is_none_or(|mut entries| entries.next().is_none()),
        "{accounts_dir:?}"
    );

    let made =
        remote_control_answer(Seat::Desk, &room, &desk.log, &desk.parts(), create("empty")).await;
    assert_eq!(made["ok"], true, "{made}");
    assert_eq!(room::agent_accounts(&desk.log).len(), 1);
}

#[tokio::test]
async fn an_account_whose_sign_in_is_running_cannot_be_removed() {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let quiet = Arc::new(Quiet::new());
    let room: Arc<dyn RoomHandle> = quiet.clone();
    let created = created(&log, &room, "Account 1").await;
    let id = created["account"]["account"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let running = crate::contract::LoginAttemptView {
        id: "fixture-attempt".to_string(),
        state: crate::contract::LoginAttemptState::Running,
        error: None,
    };
    *quiet.account_attempt.lock().unwrap() = Some((id.clone(), running));

    assert_eq!(
        run(&log, &room, "accounts.remove", json!({"accountId":id})).await,
        Err("Finish or cancel Account 1's sign-in first.".to_string())
    );
    assert_eq!(room::agent_accounts(&log)[0].id, id);
}

fn desk() -> (
    tempfile::TempDir,
    Arc<crate::desk::Desk>,
    Arc<dyn RoomHandle>,
) {
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(
            root.path(),
            Arc::new(crate::credentials::MemoryStore::default()),
        )
        .unwrap(),
    );
    let room: Arc<dyn RoomHandle> = desk.clone();
    (root, desk, room)
}

async fn key(log: &Log, room: &Arc<dyn RoomHandle>, provider_id: &str, label: &str) -> String {
    let made = run(
        log,
        room,
        "credential.create",
        json!({"providerId":provider_id,"label":label,"secret":format!("{label}-secret")}),
    )
    .await
    .unwrap();
    made["id"].as_str().unwrap().to_string()
}

fn signed_in(desk: &crate::desk::Desk, provider_id: &str, label: &str) -> String {
    let vault = desk.test_vault();
    let (id, _) = vault.begin_login().unwrap();
    vault
        .login_tokens(&id)
        .write(b"{\"fixture\":true}")
        .unwrap();
    vault.finish_login(&id, provider_id, label).unwrap();
    id
}

async fn teammate(log: &Log, room: &Arc<dyn RoomHandle>, draft: Value) -> String {
    let made = run(log, room, "persona.create", json!({ "draft": draft }))
        .await
        .unwrap();
    made["id"].as_str().unwrap().to_string()
}

fn prefer(log: &Log, model_id: &str) {
    log.append(
        &StreamId::Room,
        &json!({"kind":"setting","id":"defaultModelId","value":model_id}),
    )
    .unwrap();
}

/// An Ollama server that lists `models`.
async fn ollama(models: &[&str]) -> (String, tokio::task::JoinHandle<()>) {
    use axum::{Router, routing::get};
    let tags = json!({"models": models
        .iter()
        .map(|id| json!({"name": id, "model": id}))
        .collect::<Vec<_>>()})
    .to_string();
    let app = Router::new().route("/api/tags", get(move || async move { tags }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, server)
}

async fn listed_credential(log: &Log, room: &Arc<dyn RoomHandle>, id: &str) -> Value {
    let listed = run(log, room, "accounts.list", json!({})).await.unwrap();
    listed["credentials"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["credential"]["id"] == id)
        .cloned()
        .unwrap_or_else(|| panic!("{id} is not listed: {listed}"))
}

#[tokio::test]
async fn accounts_list_names_every_credential_with_its_section_users_and_default() {
    let (_root, desk, room) = desk();
    let log = &desk.log;
    let first = key(log, &room, "anthropic", "Personal").await;
    let second = key(log, &room, "anthropic", "Work").await;
    let login = signed_in(&desk, "openai-codex", "ChatGPT");
    let server = desk
        .test_vault()
        .connect_local("http://127.0.0.1:11434", &["llama".into()], "Ollama Local")
        .unwrap()
        .id;
    let ada = teammate(
        log,
        &room,
        json!({"name":"Ada","backendId":"hotline","modelId":"anthropic/x","accountId":second}),
    )
    .await;
    let mack = teammate(
        log,
        &room,
        json!({"name":"Mack","backendId":"hotline","modelId":"anthropic/x"}),
    )
    .await;
    let eve = teammate(log, &room, json!({"name":"Eve","backendId":"hotline"})).await;
    teammate(
        log,
        &room,
        json!({"name":"Otto","backendId":"hotline","modelId":"openai/gpt"}),
    )
    .await;
    teammate(log, &room, json!({"name":"Cid","backendId":"claude-acp"})).await;
    assert!(room::roster(log)[2].model_id.is_none());
    prefer(log, "anthropic/x");

    let listed = run(log, &room, "accounts.list", json!({})).await.unwrap();
    let rows = listed["credentials"].as_array().unwrap();
    let ids: Vec<&str> = rows
        .iter()
        .map(|row| row["credential"]["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [&first, &second, &login, &server]);
    let row = |id: &str| {
        rows.iter()
            .find(|row| row["credential"]["id"] == id)
            .unwrap()
    };
    assert_eq!(row(&first)["default"], true);
    assert_eq!(row(&first)["usedBy"], json!([mack, eve]));
    assert_eq!(row(&first)["section"], "api_key");
    assert_eq!(row(&second)["default"], false);
    assert_eq!(row(&second)["usedBy"], json!([ada]));
    assert_eq!(row(&second)["section"], "api_key");
    assert_eq!(row(&login)["section"], "subscription");
    assert_eq!(row(&login)["default"], true);
    assert_eq!(row(&login)["usedBy"], json!([]));
    assert_eq!(row(&server)["section"], "server");
    let claude = listed["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| agent["backendId"] == "claude-acp")
        .unwrap();
    assert_eq!(claude["providerId"], "anthropic");
    assert!(
        !listed.to_string().contains("-secret"),
        "a key never reaches the list: {listed}"
    );
}

#[tokio::test]
async fn removing_a_chosen_credential_is_refused_naming_its_teammates_and_the_default_can_go() {
    let (_root, desk, room) = desk();
    let log = &desk.log;
    let first = key(log, &room, "anthropic", "Personal").await;
    let second = key(log, &room, "anthropic", "Work").await;
    let ada = teammate(
        log,
        &room,
        json!({"name":"Ada","backendId":"hotline","modelId":"anthropic/x","accountId":second}),
    )
    .await;
    let mack = teammate(
        log,
        &room,
        json!({"name":"Mack","backendId":"hotline","modelId":"anthropic/x"}),
    )
    .await;

    assert_eq!(
        run(log, &room, "credential.delete", json!({"id":second})).await,
        Err("Work can't be removed while Ada uses it.".to_string())
    );
    assert!(
        room.credentials()
            .iter()
            .any(|row| row.id == second && !row.revoked),
        "a refused removal revokes nothing"
    );

    run(log, &room, "credential.delete", json!({"id":first}))
        .await
        .unwrap();
    let now_default = listed_credential(log, &room, &second).await;
    assert_eq!(now_default["default"], true);
    assert_eq!(now_default["usedBy"], json!([ada, mack]));
}

#[tokio::test]
async fn an_account_is_renamed_and_changed_in_place() {
    let (_root, desk, room) = desk();
    let log = &desk.log;
    let first = key(log, &room, "anthropic", "Personal").await;
    let second = key(log, &room, "anthropic", "Work").await;

    let renamed = run(
        log,
        &room,
        "credential.update",
        json!({"id":first,"label":" Prod "}),
    )
    .await
    .unwrap();
    assert_eq!(renamed["id"], json!(first));
    assert_eq!(renamed["label"], "Prod");
    let rekeyed = run(
        log,
        &room,
        "credential.update",
        json!({"id":first,"secret":" new-secret "}),
    )
    .await
    .unwrap();
    assert_eq!(rekeyed["id"], json!(first));
    assert_eq!(rekeyed["label"], "Prod");
    assert!(!rekeyed.to_string().contains("new-secret"), "{rekeyed}");
    let row = listed_credential(log, &room, &first).await;
    assert_eq!(row["default"], true);
    assert_eq!(row["credential"]["label"], "Prod");
    let (default, auth) = desk.test_vault().connection("anthropic").unwrap();
    assert_eq!(default.id, first);
    assert!(matches!(auth, ProviderAuth::ApiKey(key) if key == "new-secret"));

    for (change, refusal) in [
        (
            json!({"id":second,"label":"prod"}),
            "Anthropic already has an account named prod.",
        ),
        (
            json!({"id":first,"baseUrl":"http://127.0.0.1:9"}),
            "This account cannot be changed that way.",
        ),
        (json!({"id":first,"secret":"  "}), "Enter an API key."),
    ] {
        assert_eq!(
            run(log, &room, "credential.update", change).await,
            Err(refusal.to_string())
        );
    }

    let (old_url, _old) = ollama(&["llama-old"]).await;
    let (new_url, _new) = ollama(&["llama-new"]).await;
    let server = run(
        log,
        &room,
        "credential.connect_local",
        json!({ "baseUrl": old_url }),
    )
    .await
    .unwrap();
    let id = server["id"].as_str().unwrap();
    let moved = run(
        log,
        &room,
        "credential.update",
        json!({"id":id,"baseUrl":new_url}),
    )
    .await
    .unwrap();
    assert_eq!(moved["id"], json!(id));
    assert_eq!(moved["baseUrl"], json!(new_url));
    assert_eq!(
        desk.test_vault().account_models().get("ollama"),
        Some(&vec!["llama-new".to_string()])
    );
}

#[tokio::test]
async fn only_a_live_sign_in_of_the_provider_can_be_signed_in_again() {
    let (_root, desk, room) = desk();
    let log = &desk.log;
    let anthropic = key(log, &room, "anthropic", "Work").await;
    let grok_key = key(log, &room, "xai", "Grok key").await;
    let login = signed_in(&desk, "openai-codex", "ChatGPT");
    let revoked = signed_in(&desk, "openai-codex", "Old ChatGPT");
    run(log, &room, "credential.revoke", json!({ "id": revoked }))
        .await
        .unwrap();
    let logins = desk.test_vault().login_dir(&login);
    let logins = logins.parent().unwrap();
    let folders = || std::fs::read_dir(logins).unwrap().count();
    let before = folders();

    for (provider, into) in [
        ("openai-codex", anthropic.as_str()),
        ("github-copilot", login.as_str()),
        ("openai-codex", revoked.as_str()),
        ("xai", grok_key.as_str()),
        ("openai-codex", "no-such-account"),
    ] {
        assert_eq!(
            run(
                log,
                &room,
                "credential.login",
                json!({"providerId":provider,"id":into}),
            )
            .await,
            Err("This sign-in cannot replace that account's.".to_string()),
            "{provider} {into}"
        );
    }
    assert_eq!(folders(), before, "no sign-in began");
}

#[tokio::test]
async fn a_blank_or_taken_name_is_settled_on_create() {
    let (_root, desk, room) = desk();
    let log = &desk.log;
    key(log, &room, "anthropic", "Work").await;
    assert_eq!(
        run(
            log,
            &room,
            "credential.create",
            json!({"providerId":"anthropic","label":"work","secret":"other"}),
        )
        .await,
        Err("Anthropic already has an account named work.".to_string())
    );
    key(log, &room, "openai", "Work").await;

    let (url, _server) = ollama(&["llama"]).await;
    let connect = |label: Option<&str>| {
        let mut params = json!({ "baseUrl": url });
        if let Some(label) = label {
            params["label"] = json!(label);
        }
        run(log, &room, "credential.connect_local", params)
    };
    assert_eq!(connect(None).await.unwrap()["label"], "Ollama Local");
    assert_eq!(connect(None).await.unwrap()["label"], "Ollama Local 2");
    assert_eq!(connect(Some(" Lab ")).await.unwrap()["label"], "Lab");
    assert_eq!(
        connect(Some("lab")).await,
        Err("Ollama Local already has an account named lab.".to_string())
    );

    signed_in(&desk, "openai-codex", "ChatGPT");
    assert_eq!(
        run(
            log,
            &room,
            "credential.login",
            json!({"providerId":"openai-codex","label":"chatgpt"}),
        )
        .await,
        Err("ChatGPT already has an account named chatgpt.".to_string())
    );
    assert_eq!(room.credentials().len(), 6);
}

#[tokio::test]
async fn persona_create_checks_a_hotline_account_against_its_model() {
    use crate::accounts::{NOT_FOR_THIS_MODEL, NOT_ON_THIS_DESK, SIGNED_OUT};
    let (_root, desk, room) = desk();
    let log = &desk.log;
    let anthropic = key(log, &room, "anthropic", "Work").await;
    let revoked = key(log, &room, "anthropic", "Old").await;
    run(log, &room, "credential.revoke", json!({ "id": revoked }))
        .await
        .unwrap();
    let create = |model_id: Option<&str>, account_id: &str| {
        let mut draft = json!({"name":"Ada","backendId":"hotline","accountId":account_id});
        if let Some(model_id) = model_id {
            draft["modelId"] = json!(model_id);
        }
        run(log, &room, "persona.create", json!({ "draft": draft }))
    };

    for (model_id, account_id, refusal) in [
        (Some("openai/gpt"), anthropic.as_str(), NOT_FOR_THIS_MODEL),
        (None, anthropic.as_str(), NOT_FOR_THIS_MODEL),
        (Some("anthropic/x"), revoked.as_str(), SIGNED_OUT),
        (Some("anthropic/x"), "no-such-account", NOT_ON_THIS_DESK),
    ] {
        assert_eq!(
            create(model_id, account_id).await,
            Err(refusal.to_string()),
            "{model_id:?} {account_id}"
        );
    }
    assert!(room::roster(log).is_empty());

    let made = create(Some("anthropic/x"), &anthropic).await.unwrap();
    assert_eq!(made["accountId"], json!(anthropic));
    prefer(log, "anthropic/x");
    let preferred = create(None, &anthropic).await.unwrap();
    assert_eq!(preferred["accountId"], json!(anthropic));
    assert_eq!(preferred["modelId"], "anthropic/x");
}

#[tokio::test]
async fn a_revoked_credential_a_teammate_is_set_to_cannot_be_removed() {
    let (_root, desk, room) = desk();
    let log = &desk.log;
    let old = key(log, &room, "anthropic", "Old").await;
    teammate(
        log,
        &room,
        json!({"name":"Ada","backendId":"hotline","modelId":"anthropic/x","accountId":old}),
    )
    .await;
    run(log, &room, "credential.revoke", json!({ "id": old }))
        .await
        .unwrap();

    assert_eq!(
        run(log, &room, "credential.delete", json!({ "id": old })).await,
        Err("Old can't be removed while Ada uses it.".to_string())
    );
    assert!(room.credentials().iter().any(|row| row.id == old));
}

/// Written as the room keeps a choice after the model or the agent changes;
/// `persona.account` would refuse each of them.
fn set_account(log: &Log, persona_id: &str, account_id: &str) {
    let mut persona = room::roster(log)
        .into_iter()
        .find(|persona| persona.id == persona_id)
        .unwrap();
    persona.account_id = Some(account_id.to_string());
    room::append_persona(log, &persona).unwrap();
}

#[tokio::test]
async fn a_choice_off_the_provider_follows_its_default_and_an_unknown_one_follows_none() {
    let (_root, desk, room) = desk();
    let log = &desk.log;
    let anthropic = key(log, &room, "anthropic", "Work").await;
    let openai = key(log, &room, "openai", "Work").await;
    let made = created(log, &room, "Personal").await;
    let agent_account = made["account"]["account"]["id"].as_str().unwrap();
    let on_anthropic = |name: &str| {
        teammate(
            log,
            &room,
            json!({"name":name,"backendId":"hotline","modelId":"anthropic/x"}),
        )
    };
    let agentic = on_anthropic("Agentic").await;
    let crossed = on_anthropic("Crossed").await;
    let lost = on_anthropic("Lost").await;
    set_account(log, &agentic, agent_account);
    set_account(log, &crossed, &openai);
    set_account(log, &lost, "no-such-account");

    assert_eq!(
        listed_credential(log, &room, &anthropic).await["usedBy"],
        json!([agentic, crossed])
    );
    assert_eq!(
        listed_credential(log, &room, &openai).await["usedBy"],
        json!([crossed])
    );
}
