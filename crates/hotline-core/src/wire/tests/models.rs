use super::*;

#[tokio::test]
async fn teammate_tools_answers_null_as_a_result_not_a_void() {
    let (_root, _log, port) = door("tools-null");
    let mut socket = desk(port).await;
    let created = create(&mut socket, 1, "Ada").await;
    let persona_id = created["id"].as_str().unwrap();

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "teammate.tools", "params": { "personaId": persona_id } }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    let fields = answer.as_object().expect("an answer is an object");
    assert!(
        fields.contains_key("result"),
        "teammate.tools with no ledger omitted result: {answer}"
    );
    assert_eq!(fields.get("result"), Some(&Value::Null));
}

#[tokio::test]
async fn a_key_provider_cannot_start_a_device_login() {
    let (_root, _log, port) = door("login-key");
    let mut socket = desk(port).await;
    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "credential.login", "params": { "providerId": "anthropic" } }),
    )
    .await;
    let refused = answered(&mut socket, 1).await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(
        refused["error"].as_str(),
        Some("Anthropic takes an API key, not a sign-in.")
    );
}

#[tokio::test]
async fn an_unknown_login_id_is_an_error() {
    let (_root, _log, port) = door("login-status");
    let mut socket = desk(port).await;
    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "credential.login_status", "params": { "loginId": "nobody" } }),
    )
    .await;
    let refused = answered(&mut socket, 1).await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"].as_str(), Some("There is no login nobody."));
}

#[tokio::test]
async fn credential_refresh_models_needs_a_login_and_refuses_a_key() {
    let (_root, _log, port) = door("refresh-models");
    let mut socket = desk(port).await;

    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "credential.refresh_models", "params": { "providerId": "anthropic" } }),
    )
    .await;
    let key = answered(&mut socket, 1).await;
    assert_eq!(key["ok"], false, "{key}");
    assert_eq!(
        key["error"].as_str(),
        Some("Anthropic takes an API key, not a sign-in.")
    );

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "credential.refresh_models", "params": { "providerId": "github-copilot" } }),
    )
    .await;
    let unsigned = answered(&mut socket, 2).await;
    assert_eq!(unsigned["ok"], false, "{unsigned}");
    assert_eq!(
        unsigned["error"].as_str(),
        Some("There is no sign-in for GitHub Copilot.")
    );
}

#[tokio::test]
async fn models_catalog_lists_a_wired_provider_and_refuses_an_unwired_one() {
    let (_root, _log, port) = door("models-catalog");
    let mut socket = desk(port).await;

    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "models.catalog", "params": { "providerId": "nope" } }),
    )
    .await;
    let refused = answered(&mut socket, 1).await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(
        refused["error"].as_str(),
        Some("nope is not a provider Hotline Agent can use.")
    );

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "models.catalog", "params": { "providerId": "anthropic" } }),
    )
    .await;
    let listed = answered(&mut socket, 2).await;
    assert_eq!(listed["ok"], true, "{listed}");
    let models = listed["result"].as_array().expect("a catalogue is a list");
    assert_ne!(models.as_slice(), [] as [Value; 0]);
    assert!(
        models.iter().all(|model| model["enabled"] == true),
        "{listed}"
    );
    assert!(
        models
            .iter()
            .all(|model| model["id"].as_str().is_some_and(|id| !id.is_empty())),
        "{listed}"
    );
}

/// Setting a model on an idle Hotline Agent teammate writes the choice on the
/// persona and remembers it as the last model used, without needing a live
/// session to hold it.
#[tokio::test]
async fn session_set_model_on_an_idle_hotline_agent_writes_the_persona_and_last_used() {
    let (_root, log, port) = door("set-model-idle");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap();
    assert!(ada.get("modelId").is_none(), "{ada}");

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "session.set_model",
            "params": { "personaId": id, "modelId": "openai/gpt" },
        }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["result"]["state"], "idle");
    assert_eq!(answer["result"]["personaId"], id);

    let roster = room::roster(&log);
    assert_eq!(roster[0].model_id.as_deref(), Some("openai/gpt"));
    assert_eq!(room::settings(&log)["lastModelId"], "openai/gpt");
}

#[tokio::test]
async fn session_set_model_refuses_a_hotline_agent_id_the_desk_cannot_reach() {
    let (_root, _log, port) = door("set-model-unknown");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap();

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "session.set_model",
            "params": { "personaId": id, "modelId": "nope/nope" },
        }),
    )
    .await;
    let refused = answered(&mut socket, 2).await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(
        refused["error"].as_str(),
        Some("nope/nope is not a model this desk can reach.")
    );
}

/// An ACP harness names its own models. The wire stores what it is given
/// and lets the child refuse an id it does not know, once it is live.
#[tokio::test]
async fn session_set_model_accepts_an_arbitrary_id_on_an_acp_teammate() {
    let (_root, log, port) = door("set-model-acp");
    let mut socket = desk(port).await;
    let draft = PersonaDraft {
        name: "Bob".to_string(),
        goal: None,
        team: None,
        backend_id: Some("cursor".to_string()),
        cwd: None,
        reach: None,
        folders: None,
        model_id: None,
        effort_id: None,
        computer: None,
        background_work: None,
    };
    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "persona.create", "params": { "draft": draft } }),
    )
    .await;
    let created = answered(&mut socket, 1).await["result"].clone();
    let id = created["id"].as_str().unwrap();

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "session.set_model",
            "params": { "personaId": id, "modelId": "cursor-special" },
        }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(
        room::roster(&log)[0].model_id.as_deref(),
        Some("cursor-special")
    );
    assert!(
        room::settings(&log).get("lastModelId").is_none(),
        "an ACP choice is not the room's last Hotline Agent model"
    );
}

fn a_model_with_efforts() -> (String, Vec<String>) {
    crate::models::catalog()
        .providers
        .iter()
        .find_map(|(provider, entry)| {
            entry.models.iter().find_map(|(id, model)| {
                (!model.efforts.is_empty())
                    .then(|| (format!("{provider}/{id}"), model.efforts.clone()))
            })
        })
        .expect("the snapshot has a model with an effort list")
}

/// The idle picker's reply names the effort a teammate with none stored
/// runs at, so the strip shows `high` before any session has said so.
#[tokio::test]
async fn models_efforts_names_the_default_a_blank_teammate_runs_at() {
    let (_root, _log, port) = door("efforts-default");
    let mut socket = desk(port).await;
    let (model, offered) = a_model_with_efforts();
    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "models.efforts", "params": { "modelId": model } }),
    )
    .await;
    let answer = answered(&mut socket, 1).await;
    assert_eq!(answer["ok"], true, "{answer}");
    let ids: Vec<&str> = answer["result"]["choices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|one| one["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, offered.iter().map(String::as_str).collect::<Vec<_>>());
    let expected = offered.iter().any(|one| one == "high").then_some("high");
    assert_eq!(answer["result"]["defaultId"].as_str(), expected, "{answer}");
}

/// Setting effort on an idle Hotline Agent teammate writes it on the persona
/// and answers idle info, without needing a live session to hold it.
#[tokio::test]
async fn session_set_config_on_an_idle_hotline_agent_writes_effort_id() {
    let (_root, log, port) = door("set-config-idle");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap();
    assert!(ada.get("effortId").is_none(), "{ada}");

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "session.set_config",
            "params": { "personaId": id, "configId": "effort", "value": "high" },
        }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["result"]["state"], "idle");
    assert_eq!(room::roster(&log)[0].effort_id.as_deref(), Some("high"));

    ask(
        &mut socket,
        json!({
            "id": 3,
            "cmd": "session.set_config",
            "params": { "personaId": id, "configId": "effort", "value": "" },
        }),
    )
    .await;
    let cleared = answered(&mut socket, 3).await;
    assert_eq!(cleared["ok"], true, "{cleared}");
    assert!(
        room::roster(&log)[0].effort_id.is_none(),
        "{:?}",
        room::roster(&log)[0]
    );
}

#[tokio::test]
async fn session_set_config_refuses_an_effort_the_model_does_not_list() {
    let (model, listed) = a_model_with_efforts();
    let (_root, _log, port) = door("set-config-unknown");
    let mut socket = desk(port).await;
    let draft = PersonaDraft {
        name: "Ada".to_string(),
        goal: None,
        team: None,
        backend_id: None,
        cwd: None,
        reach: None,
        folders: None,
        model_id: Some(model.clone()),
        effort_id: None,
        computer: None,
        background_work: None,
    };
    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "persona.create", "params": { "draft": draft } }),
    )
    .await;
    let created = answered(&mut socket, 1).await["result"].clone();
    let id = created["id"].as_str().unwrap();

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "session.set_config",
            "params": { "personaId": id, "configId": "effort", "value": "nope" },
        }),
    )
    .await;
    let refused = answered(&mut socket, 2).await;
    assert_eq!(refused["ok"], false, "{refused}");
    let error = refused["error"].as_str().unwrap();
    assert!(error.contains("nope is not an effort"), "{error}");
    assert!(
        listed.iter().all(|id| id != "nope"),
        "the fixture must pick a model that does not list nope"
    );
}

async fn set_backend(socket: &mut Socket, ask_id: i64, persona: &str, backend: &str) -> Value {
    ask(
        socket,
        json!({
            "id": ask_id,
            "cmd": "session.set_backend",
            "params": { "personaId": persona, "backendId": backend },
        }),
    )
    .await;
    answered(socket, ask_id).await
}

/// An idle teammate moves to another agent at once: the record names the
/// agent and leaves the model to it, the agent's old session is withdrawn so
/// it starts fresh (never A→B→A back into a stale one), and the live
/// session restarts on it.
#[tokio::test]
async fn session_set_backend_moves_an_idle_teammate_and_withdraws_the_agents_old_session() {
    let quiet = Arc::new(Quiet::new());
    let (_root, log, port) = door_with("set-backend-idle", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap().to_string();
    room::checkpoint_session(&log, &id, "codex", "an-old-codex-session").unwrap();

    let moved = set_backend(&mut socket, 2, &id, "codex").await;
    assert_eq!(moved["ok"], true, "{moved}");
    let roster = room::roster(&log);
    assert_eq!(roster[0].backend_id, "codex");
    assert_eq!(
        roster[0].model_id, None,
        "the agent's own model until one is named"
    );
    assert!(
        roster[0]
            .session_checkpoints
            .iter()
            .all(|checkpoint| checkpoint.backend_id != "codex"),
        "the agent's old session is not reopened"
    );
    assert_eq!(quiet.reattached(), vec![id.clone()]);
    assert!(
        quiet.invalidations.lock().unwrap().is_empty(),
        "nothing was cut short"
    );
}

/// A teammate mid-turn keeps its turn: the record changes now, and the
/// session restarts on the new agent only once the teammate is idle.
#[tokio::test]
async fn session_set_backend_waits_for_a_busy_teammate_to_finish_its_turn() {
    let quiet = Arc::new(Quiet::new());
    let (_root, log, port) = door_with("set-backend-busy", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap().to_string();
    let mut thinking = idle(&id);
    thinking.state = SessionState::Thinking;
    quiet.set_info(thinking);

    let moved = set_backend(&mut socket, 2, &id, "codex").await;
    assert_eq!(moved["ok"], true, "{moved}");
    assert_eq!(room::roster(&log)[0].backend_id, "codex");
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    assert!(
        quiet.reattached().is_empty(),
        "a turn in progress is not restarted"
    );

    quiet.set_info(idle(&id));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while quiet.reattached().is_empty() && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(quiet.reattached(), vec![id]);
}

#[tokio::test]
async fn session_set_backend_refuses_an_agent_that_is_not_ready_or_not_there() {
    let quiet = Arc::new(Quiet::new());
    let (_root, log, port) = door_with("set-backend-refused", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap().to_string();
    let not_ready = set_backend(&mut socket, 2, &id, "cursor").await;
    assert_eq!(not_ready["ok"], false);
    assert_eq!(not_ready["error"], "Cursor is not ready: Not signed in.");
    let missing = set_backend(&mut socket, 3, &id, "no-such-agent").await;
    assert_eq!(missing["ok"], false);
    assert_eq!(room::roster(&log)[0].backend_id, "hotline");
    assert_eq!(quiet.reattached(), Vec::<String>::new());
}
