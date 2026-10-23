use super::*;

#[tokio::test]
async fn a_new_teammate_takes_the_rooms_defaults() {
    let (root, log, port) = door("defaults");
    let mut socket = desk(port).await;

    let created = create(&mut socket, 1, "  Ada  ").await;
    let id = created["id"].as_str().unwrap();
    assert_eq!(created["name"], "Ada");
    assert_eq!(created["goal"], "");
    assert_eq!(created["backendId"], "hotline");
    assert_eq!(
        created["cwd"],
        json!(paths::default_workspace(&root, id).to_string_lossy())
    );
    assert_eq!(
        created["mcpPolicy"],
        json!({ "mode": "none", "serverIds": [] })
    );
    assert_eq!(created["sessionCheckpoints"], json!([]));
    assert_eq!(created["createdAt"], created["updatedAt"]);
    // Absent, not null: the workspace is the wall, and a teammate that never
    // asked for the machine says nothing about reach at all.
    assert!(created.get("reach").is_none(), "{created}");
    assert!(created.get("team").is_none(), "{created}");
    // No room default and no last model used: the driver picks at start.
    assert!(created.get("modelId").is_none(), "{created}");

    // And it is the room's own record, not a value the door made up.
    assert_eq!(json!(room::roster(&log)), json!([created]));
}

#[tokio::test]
async fn a_draft_that_names_things_keeps_them() {
    let (_root, _log, port) = door("draft");
    let mut socket = desk(port).await;

    let draft = PersonaDraft {
        name: "Bob".to_string(),
        goal: Some("  Keep the harbour running.  ".to_string()),
        team: Some("harbour".to_string()),
        backend_id: Some("cursor".to_string()),
        cwd: Some("/tmp/harbour".to_string()),
        reach: Some(crate::contract::Reach::Machine),
        folders: None,
        model_id: Some("gpt-5".to_string()),
        effort_id: None,
        computer: None,
        background_work: Some(true),
        account_id: None,
    };
    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "persona.create", "params": { "draft": draft } }),
    )
    .await;
    let created = heard(&mut socket).await["result"].clone();
    assert_eq!(created["goal"], "Keep the harbour running.");
    assert_eq!(created["team"], "harbour");
    assert_eq!(created["backendId"], "cursor");
    assert_eq!(created["cwd"], "/tmp/harbour");
    assert_eq!(created["reach"], "machine");
    assert_eq!(created["modelId"], "gpt-5");
    assert_eq!(created["backgroundWork"], true);
}

#[tokio::test]
async fn a_patch_is_folded_over_the_teammate_and_the_whole_record_is_written_again() {
    let (_root, log, port) = door("update");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap().to_string();

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "persona.update", "params": { "id": id, "patch": { "name": "Ada Lovelace" } } }),
    )
    .await;
    let updated = heard(&mut socket).await["result"].clone();
    assert_eq!(updated["name"], "Ada Lovelace");
    assert_eq!(updated["goal"], ada["goal"]);
    assert_eq!(updated["createdAt"], ada["createdAt"]);
    assert_eq!(json!(room::roster(&log)), json!([updated]));

    ask(
        &mut socket,
        json!({ "id": 3, "cmd": "persona.update", "params": { "id": "nobody", "patch": {} } }),
    )
    .await;
    let refused = heard(&mut socket).await;
    assert_eq!(refused["ok"], false);
    assert_eq!(refused["error"], "There is no teammate nobody.");
}

#[tokio::test]
async fn a_setting_is_one_event_per_key_and_null_puts_the_default_back() {
    let (_root, log, port) = door("settings");
    let mut socket = desk(port).await;

    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "settings.update", "params": { "patch": { "theme": "dark", "chapterIdleHours": 2 } } }),
    )
    .await;
    let settings = heard(&mut socket).await["result"].clone();
    assert_eq!(settings["theme"], "dark");
    assert_eq!(settings["chapterIdleHours"], 2);

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "settings.update", "params": { "patch": { "chapterIdleHours": null } } }),
    )
    .await;
    let cleared = heard(&mut socket).await["result"].clone();
    assert_eq!(cleared["chapterIdleHours"], 8);
    assert_eq!(json!(room::settings(&log)), cleared);
}

/// The owner's "Use initial" clears a picture, and can set nothing else there.
#[tokio::test]
async fn a_patch_can_clear_a_picture_but_not_invent_one() {
    let (root, _log, port) = door("avatar-clear");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap();
    let kept = paths::avatar_path(&root, id, &"ab".repeat(32)).unwrap();
    std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
    std::fs::write(&kept, b"png").unwrap();

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "persona.update", "params": { "id": id, "patch": {
            "avatar": { "hash": "ab".repeat(32), "by": "person", "updatedAt": "now" } } } }),
    )
    .await;
    let refused = answered(&mut socket, 2).await;
    assert_eq!(refused["ok"], false, "{refused}");

    ask(
        &mut socket,
        json!({ "id": 3, "cmd": "persona.update", "params": { "id": id, "patch": { "avatar": null } } }),
    )
    .await;
    let cleared = answered(&mut socket, 3).await;
    assert_eq!(cleared["ok"], true, "{cleared}");
    assert!(!kept.exists());
}

/// The phone's own create builds exactly the draft it cannot express: a
/// workspace reach, this desk's default workspace, no computer and no
/// background work — proved by reading the fields back off the written
/// persona, not by trusting the params the phone sent. Unknown fields the
/// phone JSON might carry, `reach`, `cwd` and `computer` among them, are
/// ignored on this command the same way they are on every other mobile one:
/// none of the `Command` variants sets `deny_unknown_fields`.
#[tokio::test]
async fn mobile_persona_create_builds_a_confined_draft_the_phone_could_not_express() {
    let (root, log, port) = door("mobile-create-confined");
    let mut socket = desk(port).await;
    let request_id = "b1f0c8b2-1c2b-4e2a-9a3d-6e4b5b6a7c8d";
    ask(
        &mut socket,
        json!({
            "id": 1,
            "cmd": "mobile.persona_create",
            "params": {
                "requestId": request_id,
                "name": "Ada",
                "goal": "Keep the harbour running",
                "reach": "machine",
                "cwd": "/etc",
                "computer": { "enabled": true },
            },
        }),
    )
    .await;
    let answer = answered(&mut socket, 1).await;
    assert_eq!(answer["ok"], true, "{answer}");
    let created = answer["result"].clone();
    assert_eq!(created["id"], request_id);
    assert_eq!(created["name"], "Ada");
    assert_eq!(created["goal"], "Keep the harbour running");
    assert_eq!(created["backendId"], "hotline");
    assert!(created.get("reach").is_none(), "{created}");
    assert!(created.get("computer").is_none(), "{created}");
    assert_eq!(created["backgroundWork"], false);
    assert_eq!(
        created["cwd"],
        json!(paths::default_workspace(&root, request_id).to_string_lossy())
    );
    assert_eq!(room::roster(&log).len(), 1);
}

/// A retried `requestId` after a lost acknowledgement returns the teammate
/// already made rather than a second one.
#[tokio::test]
async fn a_repeated_request_id_does_not_duplicate_the_teammate() {
    let (_root, log, port) = door("mobile-create-idempotent");
    let mut socket = desk(port).await;
    let request_id = "c2a1d9c3-2d3c-4f3b-8b4e-7f5c6c7b8d9e";
    let make = json!({
        "id": 1,
        "cmd": "mobile.persona_create",
        "params": { "requestId": request_id, "name": "Ada" },
    });
    ask(&mut socket, make.clone()).await;
    let first = answered(&mut socket, 1).await["result"].clone();

    let mut retry = make.clone();
    retry["id"] = json!(2);
    ask(&mut socket, retry).await;
    let second = answered(&mut socket, 2).await["result"].clone();

    assert_eq!(first, second);
    assert_eq!(room::roster(&log).len(), 1);
}

/// Why a harness is unavailable can name the machine's accounts and folders:
/// the owner reads it, a companion reads only that it is unavailable.
#[tokio::test]
async fn a_companion_reads_that_a_harness_is_unavailable_not_why() {
    let handle: Arc<dyn RoomHandle> = Arc::new(Quiet::new());
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let request = json!({"id": 1, "cmd": "backends.list", "params": {}});
    let reason = |answer: &Value| answer["result"][1]["unavailable"].clone();
    let owner = remote_control_answer(
        Seat::Owner,
        &handle,
        &log,
        &Parts::default(),
        request.clone(),
    )
    .await;
    assert_eq!(reason(&owner), "Not signed in.");
    let phone = remote_control_answer(Seat::Phone, &handle, &log, &Parts::default(), request).await;
    assert_eq!(phone["ok"], true, "{phone}");
    assert_eq!(reason(&phone), super::PRIVATE_REASON);
    assert!(phone["result"][0].get("unavailable").is_none());
}

/// A bad uuid, a blank name, an unknown backend and one `backends.list`
/// reports unavailable are each refused with a sentence a phone can show.
#[tokio::test]
async fn mobile_persona_create_refuses_a_bad_uuid_a_blank_name_and_a_backend_that_is_not_ready() {
    let (_root, _log, port) = door("mobile-create-refusals");
    let mut socket = desk(port).await;
    for (n, params, needle) in [
        (
            1,
            json!({ "requestId": "not-a-uuid", "name": "Ada" }),
            "requestId must be a uuid",
        ),
        (
            2,
            json!({ "requestId": "d3b2e0d4-3e4d-405c-9c5f-8a6d7d8c9e0f", "name": "   " }),
            "needs a name",
        ),
        (
            3,
            json!({
                "requestId": "e4c3f1e5-4f5e-416d-ad6a-9b7e8e9dafa0",
                "name": "Ada",
                "backendId": "nope",
            }),
            "no harness nope",
        ),
        (
            4,
            json!({
                "requestId": "f5d4a2f6-5a6f-427e-be7b-ac8f9fabab11",
                "name": "Ada",
                "backendId": "cursor",
            }),
            "not ready",
        ),
    ] {
        ask(
            &mut socket,
            json!({ "id": n, "cmd": "mobile.persona_create", "params": params }),
        )
        .await;
        let refused = answered(&mut socket, n).await;
        assert_eq!(refused["ok"], false, "{refused}");
        let error = refused["error"].as_str().unwrap();
        assert!(error.contains(needle), "{error}");
    }
}

#[tokio::test]
async fn session_set_config_on_an_acp_teammate_goes_to_the_room() {
    let (_root, _log, port) = door("set-config-acp");
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
        account_id: None,
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
            "params": { "personaId": id, "configId": "effort", "value": "high" },
        }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["result"]["configs"][0]["id"], "effort");
    assert_eq!(answer["result"]["configs"][0]["currentId"], "high");
}

#[tokio::test]
async fn persona_create_fills_model_id_from_the_room_default() {
    let (_root, _log, port) = door("create-default-model");
    let mut socket = desk(port).await;
    ask(
        &mut socket,
        json!({
            "id": 1,
            "cmd": "settings.update",
            "params": { "patch": { "defaultModelId": "anthropic/claude" } },
        }),
    )
    .await;
    assert_eq!(answered(&mut socket, 1).await["ok"], true);

    let created = create(&mut socket, 2, "Ada").await;
    assert_eq!(created["modelId"], "anthropic/claude");
}

#[tokio::test]
async fn persona_create_fills_model_id_from_the_last_used_when_no_default() {
    let (_root, _log, port) = door("create-last-model");
    let mut socket = desk(port).await;
    ask(
        &mut socket,
        json!({
            "id": 1,
            "cmd": "settings.update",
            "params": { "patch": { "lastModelId": "openai/gpt" } },
        }),
    )
    .await;
    assert_eq!(answered(&mut socket, 1).await["ok"], true);

    let created = create(&mut socket, 2, "Ada").await;
    assert_eq!(created["modelId"], "openai/gpt");
}

#[tokio::test]
async fn persona_create_leaves_model_id_absent_when_the_room_has_no_preference() {
    let (_root, _log, port) = door("create-no-model");
    let mut socket = desk(port).await;
    let created = create(&mut socket, 1, "Ada").await;
    assert!(created.get("modelId").is_none(), "{created}");
}

/// The fake room's start reports a model, which is enough for the wire to
/// remember it as the last one used.
#[tokio::test]
async fn session_start_writes_last_model_id_from_the_sessions_info() {
    let (_root, log, port) = door("start-last-model");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap();

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "session.start", "params": { "personaId": id } }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(room::settings(&log)["lastModelId"], "anthropic/claude");
}

/// A harness picks its own model and only says so once running, so what a
/// start reports is written to the teammate: the band can name it before
/// the child is started again. The room's last model is Hotline Agent's alone.
#[tokio::test]
async fn session_start_writes_the_reported_model_on_an_acp_teammate() {
    let (_root, log, port) = door("start-acp-model");
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
        account_id: None,
    };
    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "persona.create", "params": { "draft": draft } }),
    )
    .await;
    let id = answered(&mut socket, 1).await["result"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "session.start", "params": { "personaId": id } }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    let bob = room::roster(&log)
        .into_iter()
        .find(|persona| persona.id == id)
        .unwrap();
    assert_eq!(bob.model_id.as_deref(), Some("anthropic/claude"));
    assert!(
        room::settings(&log).get("lastModelId").is_none(),
        "a harness's model is the teammate's, not the room's last"
    );
}

/// A patch that changes what a teammate can use restarts it; a patch of
/// only the name does not, because the driver is not built from the name.
#[tokio::test]
async fn persona_update_of_reach_reattaches_and_a_name_patch_does_not() {
    let quiet = Arc::new(Quiet::new());
    let (_root, _log, port) = door_with("reattach-persona", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap().to_string();

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "persona.update",
            "params": { "id": id, "patch": { "reach": "machine" } },
        }),
    )
    .await;
    assert_eq!(answered(&mut socket, 2).await["ok"], true);
    assert_eq!(quiet.reattached(), vec![id.clone()]);
    assert_eq!(*quiet.invalidations.lock().unwrap(), vec![id.clone()]);

    ask(
        &mut socket,
        json!({
            "id": 3,
            "cmd": "persona.update",
            "params": { "id": id, "patch": { "name": "Ada Lovelace" } },
        }),
    )
    .await;
    assert_eq!(answered(&mut socket, 3).await["ok"], true);
    assert_eq!(
        quiet.reattached(),
        vec![id.clone()],
        "a name patch reattached a second time"
    );
    assert_eq!(*quiet.invalidations.lock().unwrap(), vec![id.clone()]);

    ask(
        &mut socket,
        json!({
            "id": 4,
            "cmd": "persona.update",
            "params": { "id": id, "patch": { "reach": "invalid" } },
        }),
    )
    .await;
    assert_eq!(answered(&mut socket, 4).await["ok"], false);
    assert_eq!(
        *quiet.invalidations.lock().unwrap(),
        vec![id],
        "an invalid patch must not revoke a working session"
    );
}

#[tokio::test]
async fn an_account_is_set_only_by_persona_account() {
    use super::accounts::run;
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let room: Arc<dyn RoomHandle> = Arc::new(Quiet::new());
    let account = "44444444-4444-4444-8444-444444444444";
    room::write_agent_accounts(
        &log,
        &[crate::contract::AgentAccount {
            id: account.to_string(),
            backend_id: "claude-acp".to_string(),
            name: "Account 1".to_string(),
            created_at: 1,
            login_at: None,
            setup: None,
        }],
    )
    .unwrap();
    let create = |backend: &str, account_id: &str| json!({"draft":{"name":"Ada","backendId":backend,"accountId":account_id}});

    let ada = run(&log, &room, "persona.create", create("claude-acp", account))
        .await
        .unwrap();
    assert_eq!(ada["accountId"], account);
    let id = ada["id"].as_str().unwrap().to_string();
    assert_eq!(
        run(
            &log,
            &room,
            "persona.create",
            create("claude-acp", "55555555-5555-4555-8555-555555555555"),
        )
        .await,
        Err("That account is not on this desk.".to_string())
    );
    assert_eq!(
        run(&log, &room, "persona.create", create("hotline", account)).await,
        Err("That account is not on this desk.".to_string())
    );
    assert_eq!(
        run(&log, &room, "persona.create", create("codex-acp", account)).await,
        Err("That agent does not use accounts.".to_string())
    );
    assert_eq!(room::roster(&log).len(), 1);

    let before = json!(room::roster(&log));
    assert_eq!(
        run(
            &log,
            &room,
            "persona.update",
            json!({"id":id,"patch":{"accountId":null}}),
        )
        .await,
        Err("A teammate's account is set with persona.account.".to_string())
    );
    assert_eq!(json!(room::roster(&log)), before);

    let renamed = run(
        &log,
        &room,
        "mobile.persona_update",
        json!({"id":id,"name":"Ada 2","accountId":null}),
    )
    .await
    .unwrap();
    assert_eq!(renamed["name"], "Ada 2");
    assert!(renamed.get("accountId").is_none(), "{renamed}");
    assert_eq!(room::roster(&log)[0].account_id.as_deref(), Some(account));

    let retried = run(
        &log,
        &room,
        "mobile.persona_create",
        json!({"requestId":id,"name":"Ada"}),
    )
    .await
    .unwrap();
    assert_eq!(retried["id"], json!(id));
    assert!(retried.get("accountId").is_none(), "{retried}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_update_and_a_checkpoint_write_never_undo_each_other() {
    use super::accounts::run;
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let room: Arc<dyn RoomHandle> = Arc::new(Quiet::new());
    let mut ada = crate::session::tests::persona("ada");
    ada.session_checkpoints = vec![crate::contract::SessionCheckpoint {
        backend_id: "claude-acp".to_string(),
        session_id: "s1".to_string(),
        account_id: None,
    }];
    crate::session::tests::enrol(&log, &ada);
    let (go, checkpoint_write) = crate::test_support::clearing_checkpoints(&log, "ada");
    let update = tokio::spawn({
        let (log, room) = (log.clone(), room.clone());
        async move {
            run(
                &log,
                &room,
                "persona.update",
                json!({"id":"ada","patch":{"name":"Ada Lovelace"}}),
            )
            .await
        }
    });
    crate::test_support::until("the update holds the policy gate", || {
        room.policy_update_lock().try_lock().is_err() || update.is_finished()
    })
    .await;
    go.send(()).unwrap();
    checkpoint_write.join().unwrap();
    update.await.unwrap().unwrap();
    let ada = room::roster(&log).pop().unwrap();
    assert_eq!(
        (ada.name.as_str(), ada.session_checkpoints),
        ("Ada Lovelace", Vec::new())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn removing_a_tool_source_and_a_checkpoint_write_never_undo_each_other() {
    use super::accounts::run;
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    log.append(
        &StreamId::Room,
        &json!({ "kind": "setting", "id": "mcpServers", "value": [
            { "id": "prism", "type": "http", "name": "Prism", "url": "http://127.0.0.1:9086/mcp" },
        ] }),
    )
    .unwrap();
    let room: Arc<dyn RoomHandle> = Arc::new(Quiet::new());
    let mut ada = crate::session::tests::persona("ada");
    ada.mcp_policy.server_ids = vec!["prism".to_string()];
    ada.session_checkpoints = vec![crate::contract::SessionCheckpoint {
        backend_id: "claude-acp".to_string(),
        session_id: "s1".to_string(),
        account_id: None,
    }];
    crate::session::tests::enrol(&log, &ada);
    let (go, checkpoint_write) = crate::test_support::clearing_checkpoints(&log, "ada");
    let update = tokio::spawn({
        let (log, room) = (log.clone(), room.clone());
        async move {
            run(
                &log,
                &room,
                "settings.update",
                json!({"patch":{"mcpServers":[]}}),
            )
            .await
        }
    });
    crate::test_support::until("the update holds the policy gate", || {
        room.policy_update_lock().try_lock().is_err() || update.is_finished()
    })
    .await;
    go.send(()).unwrap();
    checkpoint_write.join().unwrap();
    update.await.unwrap().unwrap();
    let ada = room::roster(&log).pop().unwrap();
    assert_eq!(
        (ada.mcp_policy.server_ids, ada.session_checkpoints),
        (Vec::new(), Vec::new())
    );
}
