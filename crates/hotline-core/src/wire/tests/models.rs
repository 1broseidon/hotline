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

/// What Claude Code offered when a session of it last opened here: two
/// models, Opus current; an effort picker, Medium current; two modes.
fn claude_offer() -> crate::contract::BackendOffer {
    let choice = |id: &str, name: &str| ConfigChoice {
        id: id.to_string(),
        name: name.to_string(),
        description: None,
        group: None,
        account: None,
        efforts: None,
        default_effort: None,
        fast: None,
    };
    crate::contract::BackendOffer {
        models: vec![choice("opus", "Opus 5.5"), choice("sonnet", "Sonnet 5.5")],
        current_model_id: Some("opus".to_string()),
        model_label: Some("Model".to_string()),
        modes: vec![choice("default", "Default"), choice("plan", "Plan")],
        current_mode_id: Some("default".to_string()),
        mode_label: Some("Mode".to_string()),
        configs: vec![crate::contract::SessionConfig {
            id: "reasoning".to_string(),
            name: "Effort".to_string(),
            category: Some(crate::contract::SessionConfigCategory::Effort),
            current_id: Some("medium".to_string()),
            options: vec![
                choice("low", "Low"),
                choice("medium", "Medium"),
                choice("high", "High"),
            ],
            note: None,
        }],
    }
}

async fn create_on(socket: &mut Socket, ask_id: i64, name: &str, backend_id: &str) -> Value {
    let draft = PersonaDraft {
        name: name.to_string(),
        goal: None,
        team: None,
        backend_id: Some(backend_id.to_string()),
        cwd: None,
        reach: None,
        folders: None,
        model_id: None,
        effort_id: None,
        computer: None,
        background_work: None,
    };
    ask(
        socket,
        json!({ "id": ask_id, "cmd": "persona.create", "params": { "draft": draft } }),
    )
    .await;
    answered(socket, ask_id).await["result"].clone()
}

async fn command(socket: &mut Socket, ask_id: i64, cmd: &str, params: Value) -> Value {
    ask(
        socket,
        json!({ "id": ask_id, "cmd": cmd, "params": params }),
    )
    .await;
    answered(socket, ask_id).await
}

/// A harness teammate that is not running keeps every choice made for it —
/// effort, model, mode, any other setting — on itself for its next session,
/// and each answer is what it offers with the choice current: the agent's
/// last offer, its own choices over the agent's defaults. A level the agent
/// does not offer is refused; an empty one goes back to the agent's own.
#[tokio::test]
async fn a_resting_harness_teammate_keeps_what_is_chosen_and_answers_with_its_offer() {
    let (_root, log, port) = door_with(
        "resting-choices",
        Arc::new(Quiet::new().offering("claude-acp", claude_offer())),
    );
    let mut socket = desk(port).await;
    let bob = create_on(&mut socket, 1, "Bob", "claude-acp").await;
    let id = bob["id"].as_str().unwrap();
    let persona = || room::roster(&log).into_iter().next().unwrap();

    let effort = command(
        &mut socket,
        2,
        "session.set_config",
        json!({ "personaId": id, "configId": "reasoning", "value": "high" }),
    )
    .await;
    assert_eq!(effort["ok"], true, "{effort}");
    let info = &effort["result"];
    assert_eq!(info["state"], "idle");
    assert_eq!(info["configs"][0]["currentId"], "high", "{info}");
    assert_eq!(info["models"].as_array().unwrap().len(), 2, "{info}");
    assert_eq!(info["currentModelId"], "opus", "the agent's own model");
    assert_eq!(persona().effort_id.as_deref(), Some("high"));

    let refused = command(
        &mut socket,
        3,
        "session.set_config",
        json!({ "personaId": id, "configId": "reasoning", "value": "extreme" }),
    )
    .await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(persona().effort_id.as_deref(), Some("high"));

    let model = command(
        &mut socket,
        4,
        "session.set_model",
        json!({ "personaId": id, "modelId": "sonnet" }),
    )
    .await;
    assert_eq!(model["result"]["currentModelId"], "sonnet", "{model}");
    assert_eq!(model["result"]["configs"][0]["currentId"], "high");
    assert_eq!(persona().model_id.as_deref(), Some("sonnet"));

    let mode = command(
        &mut socket,
        5,
        "session.set_mode",
        json!({ "personaId": id, "modeId": "plan" }),
    )
    .await;
    assert_eq!(mode["result"]["currentModeId"], "plan", "{mode}");
    assert_eq!(persona().mode_id.as_deref(), Some("plan"));

    let other = command(
        &mut socket,
        6,
        "session.set_config",
        json!({ "personaId": id, "configId": "verbosity", "value": "terse" }),
    )
    .await;
    assert_eq!(other["ok"], true, "{other}");
    assert_eq!(
        persona()
            .config_choices
            .get("verbosity")
            .map(String::as_str),
        Some("terse")
    );
    assert_eq!(persona().effort_id.as_deref(), Some("high"));

    let cleared = command(
        &mut socket,
        7,
        "session.set_config",
        json!({ "personaId": id, "configId": "reasoning", "value": "" }),
    )
    .await;
    assert_eq!(cleared["result"]["configs"][0]["currentId"], "medium");
    assert!(persona().effort_id.is_none());

    // A move to another agent leaves the old one's effort and settings behind.
    let moved = command(
        &mut socket,
        8,
        "session.set_config",
        json!({ "personaId": id, "configId": "reasoning", "value": "low" }),
    )
    .await;
    assert_eq!(moved["ok"], true, "{moved}");
    let to_hotline = command(
        &mut socket,
        9,
        "session.set_backend",
        json!({ "personaId": id, "backendId": "hotline" }),
    )
    .await;
    assert_eq!(to_hotline["ok"], true, "{to_hotline}");
    assert!(persona().effort_id.is_none());
    assert!(persona().config_choices.is_empty());
}

/// The roster draws a resting harness teammate with its agent's offer and
/// its own choices; one on an agent that has never reported here has only
/// what it chose, and a Hotline Agent teammate is left to the desk's models.
#[tokio::test]
async fn a_resting_teammates_roster_row_carries_its_agents_offer() {
    let (_root, _log, port) = door_with(
        "resting-roster",
        Arc::new(Quiet::new().offering("claude-acp", claude_offer())),
    );
    let mut socket = desk(port).await;
    let bob = create_on(&mut socket, 1, "Bob", "claude-acp").await;
    let cy = create_on(&mut socket, 2, "Cy", "cursor").await;
    let ada = create(&mut socket, 3, "Ada").await;
    command(
        &mut socket,
        4,
        "session.set_config",
        json!({ "personaId": bob["id"], "configId": "reasoning", "value": "high" }),
    )
    .await;

    ask(&mut socket, json!({ "id": 5, "sub": { "view": "roster" } })).await;
    let snapshot = heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    let rows = snapshot["snapshot"].as_array().unwrap();
    let row = |who: &Value| {
        rows.iter()
            .find(|row| row["persona"]["id"] == who["id"])
            .unwrap()["session"]
            .clone()
    };
    let resting = row(&bob);
    assert_eq!(resting["state"], "idle");
    assert_eq!(
        resting["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|model| model["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["Opus 5.5", "Sonnet 5.5"]
    );
    assert_eq!(resting["currentModelId"], "opus");
    assert_eq!(resting["configs"][0]["category"], "effort");
    assert_eq!(resting["configs"][0]["currentId"], "high");
    assert_eq!(resting["modes"].as_array().unwrap().len(), 2);
    assert_eq!(resting["currentModeId"], "default");

    let never = row(&cy);
    assert_eq!(never["models"], json!([]), "{never}");
    assert_eq!(never["configs"], json!([]));
    let hotline = row(&ada);
    assert_eq!(hotline["configs"], json!([]), "{hotline}");
    assert!(hotline.get("currentModelId").is_none(), "{hotline}");
}

/// Codex's models as `model/list` lists them out of band, before any
/// session of Codex has opened here: each with its own levels.
fn codex_catalog() -> crate::driver::acp::catalog::Catalog {
    let level = |id: &str| ConfigChoice {
        id: id.to_string(),
        name: id.to_string(),
        description: None,
        group: None,
        account: None,
        efforts: None,
        default_effort: None,
        fast: None,
    };
    let model = |id: &str, levels: &[&str], default: &str| ConfigChoice {
        efforts: Some(levels.iter().map(|id| level(id)).collect()),
        default_effort: Some(default.to_string()),
        fast: None,
        ..level(id)
    };
    crate::driver::acp::catalog::Catalog {
        models: vec![
            model("gpt-6.1-sol", &["low", "medium", "xhigh"], "low"),
            model("gpt-6-luna", &["medium", "max"], "medium"),
        ],
        default_model: Some("gpt-6.1-sol".to_string()),
        effort_id: "reasoning_effort".to_string(),
        effort_name: "Reasoning effort".to_string(),
        ..Default::default()
    }
}

/// What the desk offers for an agent known only by its catalogue.
fn catalogued_offer() -> crate::contract::BackendOffer {
    let mut offer = crate::contract::BackendOffer::default();
    crate::driver::acp::catalog::fill(&mut offer, Some(&codex_catalog()));
    offer
}

fn levels(model: &Value) -> Vec<&str> {
    model["efforts"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|level| level["id"].as_str().unwrap())
        .collect()
}

/// A teammate on an agent that has never run here offers the agent's
/// models, each with its levels, once its catalogue has been read, and its
/// effort is shown and set at rest: a model not in use is chosen with one of
/// its own levels as the menu does it, the model and then the effort, and
/// both are kept for the next session; a level the model does not have is
/// refused.
#[tokio::test]
async fn a_resting_teammate_on_a_never_run_agent_chooses_any_models_effort() {
    let (_root, log, port) = door_with(
        "catalogued",
        Arc::new(Quiet::new().offering("codex-acp", catalogued_offer())),
    );
    let mut socket = desk(port).await;
    let cy = create_on(&mut socket, 1, "Cy", "codex-acp").await;
    let id = cy["id"].as_str().unwrap();
    let persona = || room::roster(&log).into_iter().next().unwrap();

    ask(&mut socket, json!({ "id": 2, "sub": { "view": "roster" } })).await;
    let snapshot = heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    let row = &snapshot["snapshot"][0]["session"];
    assert_eq!(row["currentModelId"], "gpt-6.1-sol", "{row}");
    assert_eq!(levels(&row["models"][0]), ["low", "medium", "xhigh"]);
    assert_eq!(levels(&row["models"][1]), ["medium", "max"]);
    assert_eq!(row["models"][1]["defaultEffort"], "medium");
    assert_eq!(row["configs"][0]["id"], "reasoning_effort");
    assert_eq!(row["configs"][0]["currentId"], "low", "the pill's effort");

    let model = command(
        &mut socket,
        3,
        "session.set_model",
        json!({ "personaId": id, "modelId": "gpt-6-luna" }),
    )
    .await;
    let effort = &model["result"]["configs"][0];
    assert_eq!(effort["currentId"], "medium", "{model}");
    let refused = command(
        &mut socket,
        4,
        "session.set_config",
        json!({ "personaId": id, "configId": effort["id"], "value": "xhigh" }),
    )
    .await;
    assert_eq!(refused["ok"], false, "luna has no xhigh: {refused}");
    let set = command(
        &mut socket,
        5,
        "session.set_config",
        json!({ "personaId": id, "configId": effort["id"], "value": "max" }),
    )
    .await;
    assert_eq!(set["result"]["currentModelId"], "gpt-6-luna", "{set}");
    assert_eq!(set["result"]["configs"][0]["currentId"], "max");
    assert_eq!(persona().model_id.as_deref(), Some("gpt-6-luna"));
    assert_eq!(persona().effort_id.as_deref(), Some("max"));

    // Back on a model without that level, the highest it has below it.
    let back = command(
        &mut socket,
        6,
        "session.set_model",
        json!({ "personaId": id, "modelId": "gpt-6.1-sol" }),
    )
    .await;
    assert_eq!(back["result"]["configs"][0]["currentId"], "xhigh", "{back}");
}

/// A teammate whose session is up shows the levels its session reports for
/// the model in use, over the catalogue's, and the catalogue's for every
/// other model.
#[tokio::test]
async fn a_live_sessions_report_wins_for_its_model_and_the_catalogue_fills_the_rest() {
    let quiet = Arc::new(Quiet::new().offering("codex-acp", catalogued_offer()));
    let (_root, _log, port) = door_with("catalogued-live", quiet.clone());
    let mut socket = desk(port).await;
    let cy = create_on(&mut socket, 1, "Cy", "codex-acp").await;
    let id = cy["id"].as_str().unwrap();
    let mut live = idle(id);
    live.session_id = Some("s-1".to_string());
    live.models = catalogued_offer()
        .models
        .into_iter()
        .map(|model| ConfigChoice {
            efforts: None,
            default_effort: None,
            fast: None,
            ..model
        })
        .collect();
    live.current_model_id = Some("gpt-6.1-sol".to_string());
    let mut reported = catalogued_offer().configs.remove(0);
    reported.options.truncate(2);
    reported.current_id = Some("medium".to_string());
    live.configs = vec![reported];
    quiet.set_info(live);

    ask(&mut socket, json!({ "id": 2, "sub": { "view": "roster" } })).await;
    let snapshot = heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    let row = &snapshot["snapshot"][0]["session"];
    assert_eq!(levels(&row["models"][0]), ["low", "medium"], "{row}");
    assert_eq!(row["models"][0]["defaultEffort"], "medium");
    assert_eq!(levels(&row["models"][1]), ["medium", "max"]);
    assert_eq!(row["configs"][0]["currentId"], "medium");
}

/// Claude Code's models as its `initialize` lists them out of band: Opus
/// with six levels, Auto first; Sonnet without Xhigh; Haiku with none.
fn claude_catalog() -> crate::driver::acp::catalog::Catalog {
    let level = |id: &str| ConfigChoice {
        id: id.to_string(),
        name: id.to_string(),
        description: None,
        group: None,
        account: None,
        efforts: None,
        default_effort: None,
        fast: None,
    };
    let model = |id: &str, levels: &[&str]| ConfigChoice {
        efforts: Some(levels.iter().map(|id| level(id)).collect()),
        default_effort: levels.first().map(|id| (*id).to_string()),
        fast: None,
        ..level(id)
    };
    crate::driver::acp::catalog::Catalog {
        models: vec![
            model(
                "opus",
                &["default", "low", "medium", "high", "xhigh", "max"],
            ),
            model("sonnet", &["default", "low", "medium", "high", "max"]),
            model("haiku", &[]),
        ],
        default_model: Some("opus".to_string()),
        effort_id: "effort".to_string(),
        effort_name: "Effort".to_string(),
        ..Default::default()
    }
}

fn claude_catalogued() -> crate::contract::BackendOffer {
    let mut offer = crate::contract::BackendOffer::default();
    crate::driver::acp::catalog::fill(&mut offer, Some(&claude_catalog()));
    offer
}

/// The effort a teammate's answer says it runs at.
fn effort_of(answer: &Value) -> Value {
    answer["result"]["configs"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|config| config["category"] == "effort")
        .map_or(Value::Null, |config| config["currentId"].clone())
}

fn desk_effort(log: &Log, key: &str) -> Option<String> {
    room::settings(log)
        .get("effortByModel")
        .and_then(|memory| memory.get(key))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// A teammate at rest switching models lands on an effort by the rules:
/// its own last for the model, else the desk's from any teammate, else the
/// one it runs at clamped down to what the model has, never up. Every level
/// chosen, on a tab or by a switch, is remembered on the teammate and the
/// desk, and both are still there when the desk opens again.
#[tokio::test]
async fn a_model_switch_lands_on_the_remembered_effort_and_remembers_it() {
    let (root, log, port) = door_with(
        "effort-memory",
        Arc::new(Quiet::new().offering("claude-acp", claude_catalogued())),
    );
    let mut socket = desk(port).await;
    let bob = create_on(&mut socket, 1, "Bob", "claude-acp").await;
    let cy = create_on(&mut socket, 2, "Cy", "claude-acp").await;
    let (bob, cy) = (bob["id"].as_str().unwrap(), cy["id"].as_str().unwrap());
    let persona = |log: &Log, id: &str| {
        room::roster(log)
            .into_iter()
            .find(|persona| persona.id == id)
            .unwrap()
    };
    let set_model = |id: &str, model: &str| json!({ "personaId": id, "modelId": model });
    let set_effort =
        |id: &str, level: &str| json!({ "personaId": id, "configId": "effort", "value": level });

    // A tab: Xhigh on Opus, remembered for Opus on Bob and on the desk.
    let tab = command(
        &mut socket,
        3,
        "session.set_config",
        set_effort(bob, "xhigh"),
    )
    .await;
    assert_eq!(effort_of(&tab), "xhigh", "{tab}");
    assert_eq!(
        persona(&log, bob)
            .effort_by_model
            .get("claude-acp/opus")
            .map(String::as_str),
        Some("xhigh")
    );
    assert_eq!(
        desk_effort(&log, "claude-acp/opus").as_deref(),
        Some("xhigh")
    );

    // Sonnet has no Xhigh: the switch steps down to High, never up to Max,
    // and remembers it.
    let sonnet = command(
        &mut socket,
        4,
        "session.set_model",
        set_model(bob, "sonnet"),
    )
    .await;
    assert_eq!(effort_of(&sonnet), "high", "{sonnet}");
    assert_eq!(persona(&log, bob).effort_id.as_deref(), Some("high"));
    assert_eq!(
        desk_effort(&log, "claude-acp/sonnet").as_deref(),
        Some("high")
    );
    command(&mut socket, 5, "session.set_config", set_effort(bob, "low")).await;
    assert_eq!(
        desk_effort(&log, "claude-acp/sonnet").as_deref(),
        Some("low")
    );

    // Back on Opus: Bob's own Xhigh for it.
    let opus = command(&mut socket, 6, "session.set_model", set_model(bob, "opus")).await;
    assert_eq!(effort_of(&opus), "xhigh", "{opus}");

    // Cy has never run Sonnet: the desk's last for it, Bob's Low.
    let cy_sonnet = command(&mut socket, 7, "session.set_model", set_model(cy, "sonnet")).await;
    assert_eq!(effort_of(&cy_sonnet), "low", "{cy_sonnet}");
    assert_eq!(
        persona(&log, cy)
            .effort_by_model
            .get("claude-acp/sonnet")
            .map(String::as_str),
        Some("low")
    );

    // Haiku has no levels: nothing to land on, nothing remembered, and the
    // teammate's effort is left for the next model.
    let haiku = command(&mut socket, 8, "session.set_model", set_model(cy, "haiku")).await;
    assert_eq!(effort_of(&haiku), Value::Null, "{haiku}");
    assert!(
        !persona(&log, cy)
            .effort_by_model
            .contains_key("claude-acp/haiku")
    );
    assert_eq!(persona(&log, cy).effort_id.as_deref(), Some("low"));
    drop(socket);

    // The desk opens again: both memories are where they were.
    let log = Log::open(&root);
    let door = Door::bind(
        log.clone(),
        "desk-token".to_string(),
        Arc::new(Quiet::new().offering("claude-acp", claude_catalogued())),
        Parts::default(),
    )
    .unwrap();
    let port = door.port();
    tokio::spawn(door.run());
    let mut socket = desk(port).await;
    assert_eq!(
        persona(&log, bob)
            .effort_by_model
            .get("claude-acp/opus")
            .map(String::as_str),
        Some("xhigh")
    );
    let dee = create_on(&mut socket, 9, "Dee", "claude-acp").await;
    let dee = dee["id"].as_str().unwrap();
    let carried = command(
        &mut socket,
        10,
        "session.set_model",
        set_model(dee, "sonnet"),
    )
    .await;
    assert_eq!(
        effort_of(&carried),
        "low",
        "the desk's, after reopening: {carried}"
    );
}

/// A teammate whose session is up is switched model first, then effort, on
/// the session: the effort the rules land on, set only when the session is
/// not already on it. A model whose levels only its session says is placed
/// on what the session reports once the model is set.
#[tokio::test]
async fn a_live_switch_sets_the_model_then_the_landed_effort() {
    let mut offer = claude_catalogued();
    offer.models.push(ConfigChoice {
        id: "mystery".to_string(),
        name: "Mystery".to_string(),
        description: None,
        group: None,
        account: None,
        efforts: None,
        default_effort: None,
        fast: None,
    });
    let level = |id: &str| ConfigChoice {
        id: id.to_string(),
        name: id.to_string(),
        description: None,
        group: None,
        account: None,
        efforts: None,
        default_effort: None,
        fast: None,
    };
    let picker = |current: &str, levels: &[&str]| crate::contract::SessionConfig {
        id: "effort".to_string(),
        name: "Effort".to_string(),
        category: Some(crate::contract::SessionConfigCategory::Effort),
        current_id: Some(current.to_string()),
        options: levels.iter().map(|id| level(id)).collect(),
        note: None,
    };
    let quiet = Arc::new(
        Quiet::new()
            .offering("claude-acp", offer.clone())
            .reporting(
                "sonnet",
                picker("default", &["default", "low", "medium", "high", "max"]),
            )
            .reporting("mystery", picker("low", &["low", "medium"])),
    );
    let (_root, log, port) = door_with("effort-live", quiet.clone());
    let mut socket = desk(port).await;
    let bob = create_on(&mut socket, 1, "Bob", "claude-acp").await;
    let bob = bob["id"].as_str().unwrap();
    let mut live = idle(bob);
    live.state = SessionState::Ready;
    live.session_id = Some("s-1".to_string());
    live.models = offer.models.clone();
    live.current_model_id = Some("opus".to_string());
    live.configs = vec![picker(
        "xhigh",
        &["default", "low", "medium", "high", "xhigh", "max"],
    )];
    quiet.set_info(live);

    let sonnet = command(
        &mut socket,
        2,
        "session.set_model",
        json!({ "personaId": bob, "modelId": "sonnet" }),
    )
    .await;
    assert_eq!(effort_of(&sonnet), "high", "{sonnet}");
    assert_eq!(
        quiet.calls(),
        ["set_model sonnet", "set_config effort high"]
    );
    let bob_now = room::roster(&log).into_iter().next().unwrap();
    assert_eq!(bob_now.model_id.as_deref(), Some("sonnet"));
    assert_eq!(bob_now.effort_id.as_deref(), Some("high"));
    assert_eq!(
        desk_effort(&log, "claude-acp/sonnet").as_deref(),
        Some("high")
    );

    // Mystery's levels come from its session: High clamps to Medium.
    let mystery = command(
        &mut socket,
        3,
        "session.set_model",
        json!({ "personaId": bob, "modelId": "mystery" }),
    )
    .await;
    assert_eq!(effort_of(&mystery), "medium", "{mystery}");
    assert_eq!(
        quiet.calls()[2..],
        ["set_model mystery", "set_config effort medium"]
    );
    let bob_now = room::roster(&log).into_iter().next().unwrap();
    assert_eq!(bob_now.effort_id.as_deref(), Some("medium"));
    assert_eq!(
        bob_now
            .effort_by_model
            .get("claude-acp/mystery")
            .map(String::as_str),
        Some("medium")
    );

    // A tab on a live session goes to it and is remembered.
    let tab = command(
        &mut socket,
        4,
        "session.set_config",
        json!({ "personaId": bob, "configId": "effort", "value": "low" }),
    )
    .await;
    assert_eq!(effort_of(&tab), "low", "{tab}");
    assert_eq!(quiet.calls()[4..], ["set_config effort low"]);
    assert_eq!(
        desk_effort(&log, "claude-acp/mystery").as_deref(),
        Some("low")
    );
}

/// Claude Code's fast mode toggle, as its adapter offers it to a client
/// without boolean settings.
fn fast_toggle(current: &str) -> crate::contract::SessionConfig {
    let value = |id: &str| ConfigChoice {
        id: id.to_string(),
        name: id.to_string(),
        description: None,
        group: None,
        account: None,
        efforts: None,
        default_effort: None,
        fast: None,
    };
    crate::contract::SessionConfig {
        id: "fast".to_string(),
        name: "Fast mode".to_string(),
        category: Some(crate::contract::SessionConfigCategory::Fast),
        current_id: Some(current.to_string()),
        options: vec![value("on"), value("off")],
        note: None,
    }
}

/// The fast toggle the answer draws, if any.
fn fast_of(answer: &Value) -> Option<Value> {
    answer["result"]["configs"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|config| config["category"] == "fast")
        .map(|config| config["currentId"].clone())
}

/// Fast mode is the teammate's choice: kept on the teammate, drawn at rest
/// only on a model that can run fast, and put back on a live session when
/// its model moves to one that can, the adapter having brought it back off.
#[tokio::test]
async fn fast_mode_is_the_teammates_choice_on_the_models_that_have_it() {
    let mut catalog = claude_catalog();
    catalog.models[0].fast = Some(true);
    catalog.models[1].fast = Some(false);
    let mut offer = crate::contract::BackendOffer::default();
    crate::driver::acp::catalog::fill(&mut offer, Some(&catalog));
    offer.configs.push(fast_toggle("off"));
    let quiet = Arc::new(
        Quiet::new()
            .offering("claude-acp", offer.clone())
            .reporting("opus", fast_toggle("off")),
    );
    let (_root, log, port) = door_with("fast-mode", quiet.clone());
    let mut socket = desk(port).await;
    let bob = create_on(&mut socket, 1, "Bob", "claude-acp").await;
    let bob = bob["id"].as_str().unwrap();
    let set_model = |model: &str| json!({ "personaId": bob, "modelId": model });

    let on = command(
        &mut socket,
        2,
        "session.set_config",
        json!({ "personaId": bob, "configId": "fast", "value": "on" }),
    )
    .await;
    assert_eq!(fast_of(&on), Some(json!("on")), "{on}");
    let bob_now = || room::roster(&log).into_iter().next().unwrap();
    assert_eq!(
        bob_now().config_choices.get("fast").map(String::as_str),
        Some("on")
    );

    // Sonnet cannot run fast: no toggle at rest, and the choice stays.
    let sonnet = command(&mut socket, 3, "session.set_model", set_model("sonnet")).await;
    assert_eq!(fast_of(&sonnet), None, "{sonnet}");
    let opus = command(&mut socket, 4, "session.set_model", set_model("opus")).await;
    assert_eq!(fast_of(&opus), Some(json!("on")), "{opus}");

    // Live on Sonnet, moved to Opus: the adapter offers it off, the
    // teammate's On is put back.
    let mut live = idle(bob);
    live.state = SessionState::Ready;
    live.session_id = Some("s-1".to_string());
    live.models = offer.models.clone();
    live.current_model_id = Some("sonnet".to_string());
    quiet.set_info(live);
    let moved = command(&mut socket, 5, "session.set_model", set_model("opus")).await;
    assert_eq!(
        quiet.calls(),
        ["set_model opus", "set_config fast on"],
        "{moved}"
    );
}

/// Cursor's Run Everything is a launch, not a session mode: a live Cursor
/// teammate moving into it is kept in it and started again, never asked to
/// set it on its session; moving back out starts it again too.
#[tokio::test]
async fn a_live_cursor_teammate_moving_into_run_everything_is_started_again() {
    use crate::driver::acp::modes::RUN_EVERYTHING;
    let mode = |id: &str| ConfigChoice {
        id: id.to_string(),
        name: id.to_string(),
        description: None,
        group: None,
        account: None,
        efforts: None,
        default_effort: None,
        fast: None,
    };
    let quiet = Arc::new(Quiet::new());
    let (_root, log, port) = door_with("cursor-run-everything", quiet.clone());
    let mut socket = desk(port).await;
    let bob = create_on(&mut socket, 1, "Bob", "cursor").await;
    let bob = bob["id"].as_str().unwrap();
    let mut live = idle(bob);
    live.state = SessionState::Ready;
    live.session_id = Some("s-1".to_string());
    live.modes = vec![
        mode("agent"),
        mode("plan"),
        mode("ask"),
        mode(RUN_EVERYTHING),
    ];
    live.current_mode_id = Some("agent".to_string());
    quiet.set_info(live.clone());

    command(
        &mut socket,
        2,
        "session.set_mode",
        json!({ "personaId": bob, "modeId": RUN_EVERYTHING }),
    )
    .await;
    let kept = room::roster(&log).into_iter().next().unwrap();
    assert_eq!(kept.mode_id.as_deref(), Some(RUN_EVERYTHING));
    assert_eq!(quiet.reattached(), [bob.to_string()]);

    live.current_mode_id = Some(RUN_EVERYTHING.to_string());
    quiet.set_info(live);
    command(
        &mut socket,
        3,
        "session.set_mode",
        json!({ "personaId": bob, "modeId": "plan" }),
    )
    .await;
    assert_eq!(quiet.reattached(), [bob.to_string(), bob.to_string()]);
    let kept = room::roster(&log).into_iter().next().unwrap();
    assert_eq!(kept.mode_id.as_deref(), Some("plan"));
}
