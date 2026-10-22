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
