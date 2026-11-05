use super::*;

/// Turning the computer on or off restarts the teammate, because it gains
/// or loses the tools. Its limits and mounts do not: they wait for the
/// container to be made again. Nor do the teammate's secrets, which are
/// handed to the computer where it runs. A turn in flight is not cut short.
#[tokio::test]
async fn computer_details_do_not_restart_the_teammate_but_turning_it_on_does() {
    let quiet = Arc::new(Quiet::new());
    let (_root, _log, port) = door_with("computer-details", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let ada_id = ada["id"].as_str().unwrap().to_string();

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "persona.update", "params": { "id": ada_id, "patch": {
            "computer": { "enabled": true },
        } } }),
    )
    .await;
    assert_eq!(answered(&mut socket, 2).await["ok"], true);
    assert_eq!(quiet.reattached(), vec![ada_id.clone()]);

    ask(
        &mut socket,
        json!({ "id": 3, "cmd": "persona.update", "params": { "id": ada_id, "patch": {
            "computer": { "enabled": true, "memory": "8g" },
        } } }),
    )
    .await;
    let answer = answered(&mut socket, 3).await;
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["result"]["computer"]["memory"], "8g");
    ask(
        &mut socket,
        json!({ "id": 5, "cmd": "persona.update", "params": { "id": ada_id, "patch": {
            "secrets": ["GITHUB_TOKEN"],
        } } }),
    )
    .await;
    let answer = answered(&mut socket, 5).await;
    assert_eq!(answer["result"]["secrets"], json!(["GITHUB_TOKEN"]));
    assert_eq!(
        quiet.computer_changes.lock().unwrap().last(),
        Some(&ada_id),
        "the teammate's secrets are handed to whichever computer it has"
    );
    // An older client ticks them on the computer; that is the teammate's grant.
    ask(
        &mut socket,
        json!({ "id": 6, "cmd": "persona.update", "params": { "id": ada_id, "patch": {
            "computer": { "enabled": true, "memory": "8g", "secrets": ["NPM_TOKEN"] },
        } } }),
    )
    .await;
    let answer = answered(&mut socket, 6).await;
    assert_eq!(answer["result"]["secrets"], json!(["NPM_TOKEN"]));
    assert!(answer["result"]["computer"].get("secrets").is_none());
    assert_eq!(
        quiet.reattached(),
        vec![ada_id.clone()],
        "no second restart"
    );
    assert_eq!(*quiet.invalidations.lock().unwrap(), vec![ada_id.clone()]);

    ask(
        &mut socket,
        json!({ "id": 4, "cmd": "persona.update", "params": { "id": ada_id, "patch": {
            "computer": { "enabled": false, "memory": "8g" },
        } } }),
    )
    .await;
    assert_eq!(answered(&mut socket, 4).await["ok"], true);
    assert_eq!(quiet.reattached(), vec![ada_id.clone(), ada_id]);
}

/// Removing a tool source is the decision, so it leaves every teammate's
/// grant as well: no policy is left naming a server that is gone, including
/// one an older build left behind. A setting that does not name servers
/// leaves grants alone.
#[tokio::test]
async fn removing_a_tool_source_takes_it_out_of_every_grant() {
    let quiet = Arc::new(Quiet::new());
    let (_root, log, port) = door_with("forget-servers", quiet.clone());
    log.append(
        &StreamId::Room,
        &json!({ "kind": "setting", "id": "mcpServers", "value": [
            { "id": "prism", "type": "http", "name": "Prism", "url": "http://127.0.0.1:9086/mcp" },
            { "id": "ketch", "type": "stdio", "name": "ketch", "command": "ketch", "args": [] },
        ] }),
    )
    .unwrap();
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let ada_id = ada["id"].as_str().unwrap().to_string();
    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "persona.update", "params": { "id": ada_id, "patch": {
            "mcpPolicy": { "mode": "some", "serverIds": ["prism", "ketch", "ghost"] },
        } } }),
    )
    .await;
    assert_eq!(answered(&mut socket, 2).await["ok"], true);
    let granted = |log: &Log| {
        room::roster(log)
            .into_iter()
            .find(|persona| persona.id == ada_id)
            .unwrap()
            .mcp_policy
            .server_ids
    };

    ask(
        &mut socket,
        json!({ "id": 3, "cmd": "settings.update", "params": { "patch": { "chapterIdleHours": 2 } } }),
    )
    .await;
    assert_eq!(answered(&mut socket, 3).await["ok"], true);
    assert_eq!(granted(&log), ["prism", "ketch", "ghost"]);

    ask(
        &mut socket,
        json!({ "id": 4, "cmd": "settings.update", "params": { "patch": { "mcpServers": [] } } }),
    )
    .await;
    assert_eq!(answered(&mut socket, 4).await["ok"], true);
    assert_eq!(granted(&log), [] as [String; 0]);
}

/// A server that no longer exists is settled, not an error, even when a
/// policy still names its id: its missing rows leave the ledger the last
/// start published. A server that exists and failed, and the computer, stay.
#[tokio::test]
async fn a_server_that_no_longer_exists_leaves_no_missing_rows() {
    use crate::contract::{AgentKind, ToolSourceKind, ToolState};
    use crate::session::ledger::ToolLedger;
    let root = scratch("stale-tool-rows");
    let log = Log::open(&root);
    // The ledger store is shared by every test; this teammate is only here.
    let id = format!("stale-{}", uuid::Uuid::new_v4());
    let persona = |server_ids: &[&str]| {
        json!({
            "kind": "persona", "id": id, "name": "Ada", "goal": "Tools",
            "backendId": "hotline", "cwd": root.to_string_lossy(),
            "mcpPolicy": { "mode": "some", "serverIds": server_ids },
            "backgroundWork": false, "sessionCheckpoints": [], "lastSessionId": null,
            "createdAt": 1, "updatedAt": 1,
        })
    };
    let servers = |ids: &[&str]| {
        let list: Vec<Value> = ids
            .iter()
            .map(|id| json!({ "id": id, "type": "stdio", "name": id, "command": id, "args": [] }))
            .collect();
        json!({ "kind": "setting", "id": "mcpServers", "value": list })
    };
    log.append(&StreamId::Room, &servers(&["prism", "ketch"]))
        .unwrap();
    log.append(&StreamId::Room, &persona(&["prism", "ketch", "ghost"]))
        .unwrap();
    let room = Room::with_agents(log.clone(), Arc::new(NoKeys), Arc::new(NoAgents));
    let mut ledger = ToolLedger::new(id.clone(), AgentKind::Hotline, "hotline");
    ledger
        .verified(ToolSourceKind::Builtin, "hotline", "send_file", "built in")
        .absent(ToolSourceKind::Mcp, "prism", "prism", "failed to start")
        .declared(ToolSourceKind::Mcp, "ketch", "search", "handed over")
        .absent(ToolSourceKind::Mcp, "ghost", "ghost", "no longer exists")
        .absent(
            ToolSourceKind::Mcp,
            crate::computer::SERVER_ID,
            "computer",
            "stopped",
        );
    ledger.publish();
    let rows = |room: &Room| {
        let mut rows: Vec<String> = room
            .teammate_tools(&id)
            .unwrap()
            .rows
            .into_iter()
            .map(|row| row.origin)
            .collect();
        rows.sort();
        rows
    };
    // Ghost is gone though the policy names it; prism exists and failed.
    assert_eq!(rows(&room), ["computer", "hotline", "ketch", "prism"]);

    // Prism is deleted too: its failure is no longer news.
    log.append(&StreamId::Room, &servers(&["ketch"])).unwrap();
    assert_eq!(rows(&room), ["computer", "hotline", "ketch"]);
    assert!(
        room.teammate_tools(&id)
            .unwrap()
            .rows
            .iter()
            .any(|row| row.origin == "computer" && row.state == ToolState::Absent)
    );
    crate::session::ledger::forget(&id);
}

/// The room's server list is what every policy of "all" includes, so every
/// live session restarts. A setting that does not name servers leaves them.
#[tokio::test]
async fn settings_update_of_mcp_servers_reattaches_every_live_session() {
    let quiet = Arc::new(Quiet::new());
    let (_root, _log, port) = door_with("reattach-servers", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let bob = create(&mut socket, 2, "Bob").await;
    let ada_id = ada["id"].as_str().unwrap().to_string();
    let bob_id = bob["id"].as_str().unwrap().to_string();

    ask(
        &mut socket,
        json!({ "id": 3, "cmd": "session.start", "params": { "personaId": ada_id } }),
    )
    .await;
    assert_eq!(answered(&mut socket, 3).await["ok"], true);
    ask(
        &mut socket,
        json!({ "id": 4, "cmd": "session.start", "params": { "personaId": bob_id } }),
    )
    .await;
    assert_eq!(answered(&mut socket, 4).await["ok"], true);

    ask(
        &mut socket,
        json!({
            "id": 5,
            "cmd": "settings.update",
            "params": { "patch": { "mcpServers": [] } },
        }),
    )
    .await;
    assert_eq!(answered(&mut socket, 5).await["ok"], true);
    let mut got = quiet.reattached();
    got.sort();
    let mut want = vec![ada_id, bob_id];
    want.sort();
    assert_eq!(got, want);
    assert_eq!(*quiet.invalidations.lock().unwrap(), vec!["*"]);

    ask(
        &mut socket,
        json!({
            "id": 6,
            "cmd": "settings.update",
            "params": { "patch": { "chapterIdleHours": 2 } },
        }),
    )
    .await;
    assert_eq!(answered(&mut socket, 6).await["ok"], true);
    let mut after = quiet.reattached();
    after.sort();
    assert_eq!(after, want, "a chapterIdleHours patch reattached again");
    assert_eq!(*quiet.invalidations.lock().unwrap(), vec!["*"]);
}

#[tokio::test]
async fn computer_commands_and_the_runtime_setting() {
    let quiet = Arc::new(Quiet::new());
    let (_root, _log, port) = door_with("computer-wire", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap().to_string();

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "computer.runtimes", "params": {} }),
    )
    .await;
    let runtimes = answered(&mut socket, 2).await;
    assert_eq!(runtimes["ok"], true, "{runtimes}");
    assert_eq!(runtimes["result"][0]["runtime"], "podman");
    assert_eq!(runtimes["result"][0]["state"], "ready");
    assert_eq!(runtimes["result"][0]["rootless"], true);
    assert_eq!(runtimes["result"][1]["runtime"], "docker");

    ask(
        &mut socket,
        json!({ "id": 3, "cmd": "computer.status", "params": { "personaId": id } }),
    )
    .await;
    let status = answered(&mut socket, 3).await;
    assert_eq!(status["ok"], true, "{status}");
    assert_eq!(status["result"]["state"], "running");
    assert_eq!(status["result"]["url"], "http://127.0.0.1:18787/mcp");
    assert_eq!(status["result"]["viewer"], "http://127.0.0.1:15800");

    ask(
        &mut socket,
        json!({ "id": 4, "cmd": "computer.stop", "params": { "personaId": id } }),
    )
    .await;
    let stopped = answered(&mut socket, 4).await;
    assert_eq!(stopped["ok"], true, "{stopped}");
    assert!(stopped.get("result").is_none(), "{stopped}");

    ask(
        &mut socket,
        json!({ "id": 5, "cmd": "computer.remove", "params": { "personaId": id } }),
    )
    .await;
    let removed = answered(&mut socket, 5).await;
    assert_eq!(removed["ok"], true, "{removed}");
    assert!(removed.get("result").is_none(), "{removed}");

    ask(
        &mut socket,
        json!({
            "id": 6,
            "cmd": "settings.update",
            "params": { "patch": { "computerRuntime": "podman" } },
        }),
    )
    .await;
    let settings = answered(&mut socket, 6).await;
    assert_eq!(settings["ok"], true, "{settings}");
    assert_eq!(settings["result"]["computerRuntime"], "podman");
    assert!(
        quiet.reattached().is_empty(),
        "picking a runtime does not reattach: {:?}",
        quiet.reattached()
    );

    ask(
        &mut socket,
        json!({
            "id": 7,
            "cmd": "computer.status",
            "params": { "personaId": "nobody" },
        }),
    )
    .await;
    let missing = answered(&mut socket, 7).await;
    assert_eq!(missing["ok"], false, "{missing}");
    assert!(
        missing["error"]
            .as_str()
            .unwrap()
            .contains("There is no teammate nobody"),
        "{missing}"
    );
}

/// The Tools pane's web search commands are the desk's: a keys-and-switches
/// surface a companion phone must not reach, and a key is never answered.
#[test]
fn only_the_desk_and_owner_seats_may_touch_web_search_settings() {
    use crate::contract::WebSearchProvider;
    let status = Command::WebsearchStatus {};
    let enable = Command::WebsearchSetEnabled {
        provider: WebSearchProvider::Exa,
        enabled: false,
    };
    let key = Command::WebsearchSetKey {
        provider: WebSearchProvider::Exa,
        key: Some("exa-key-0123456789".to_string()),
    };
    let url = Command::WebsearchSetUrl {
        provider: WebSearchProvider::Searxng,
        url: Some("http://localhost:8888".to_string()),
    };
    for command in [&status, &enable, &key, &url] {
        assert!(Seat::Desk.permits(command), "{command:?}");
        assert!(Seat::Owner.permits(command), "{command:?}");
        assert!(!Seat::Phone.permits(command), "{command:?}");
    }
}

#[tokio::test]
async fn web_search_switches_and_keys_round_trip_without_the_key_ever_leaving() {
    use crate::credentials::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    let handle: Arc<dyn RoomHandle> = desk.clone();
    let log = &desk.log;
    let ask = |cmd: &str, params: Value| json!({"id": 1, "cmd": cmd, "params": params});
    let key = "exa-private-key-0123456789";

    let fresh = remote_control_answer(
        Seat::Desk,
        &handle,
        log,
        &desk.parts(),
        ask("websearch.status", json!({})),
    )
    .await;
    assert_eq!(fresh["ok"], true, "{fresh}");
    assert_eq!(
        fresh["result"]["providers"],
        json!([
            {"provider": "parallel", "name": "Parallel", "enabled": true, "hasKey": false},
            {"provider": "exa", "name": "Exa", "enabled": true, "hasKey": false},
            {"provider": "keenable", "name": "Keenable", "enabled": true, "hasKey": false},
            {"provider": "firecrawl", "name": "Firecrawl", "enabled": true, "hasKey": false},
            {"provider": "searxng", "name": "SearXNG", "enabled": false, "hasKey": false, "takesUrl": true},
        ])
    );

    let off = remote_control_answer(
        Seat::Owner,
        &handle,
        log,
        &desk.parts(),
        ask(
            "websearch.set_enabled",
            json!({"provider": "keenable", "enabled": false}),
        ),
    )
    .await;
    assert_eq!(off["result"]["providers"][2]["enabled"], false, "{off}");
    assert_eq!(
        crate::room::settings(log)["webSearch"],
        json!({"disabled": ["keenable"]})
    );

    let saved = remote_control_answer(
        Seat::Desk,
        &handle,
        log,
        &desk.parts(),
        ask("websearch.set_key", json!({"provider": "exa", "key": key})),
    )
    .await;
    assert_eq!(saved["result"]["providers"][1]["hasKey"], true, "{saved}");
    assert!(!saved.to_string().contains(key));
    assert_eq!(
        desk.saved_web_search_key(crate::contract::WebSearchProvider::Exa),
        Some(key.to_string())
    );
    assert!(!format!("{:?}", log.load(&StreamId::Room)).contains(key));

    let on = remote_control_answer(
        Seat::Desk,
        &handle,
        log,
        &desk.parts(),
        ask(
            "websearch.set_enabled",
            json!({"provider": "keenable", "enabled": true}),
        ),
    )
    .await;
    assert_eq!(on["result"]["providers"][2]["enabled"], true);
    assert_eq!(
        crate::room::settings(log)["webSearch"],
        json!({"disabled": []})
    );

    let cleared = remote_control_answer(
        Seat::Desk,
        &handle,
        log,
        &desk.parts(),
        ask("websearch.set_key", json!({"provider": "exa", "key": null})),
    )
    .await;
    assert_eq!(
        cleared["result"]["providers"][1]["hasKey"], false,
        "{cleared}"
    );

    let refused = remote_control_answer(
        Seat::Phone,
        &handle,
        log,
        &desk.parts(),
        ask("websearch.status", json!({})),
    )
    .await;
    assert_eq!(refused["code"], FORBIDDEN, "{refused}");
    let bad = remote_control_answer(
        Seat::Desk,
        &handle,
        log,
        &desk.parts(),
        ask(
            "websearch.set_key",
            json!({"provider": "brave", "key": key}),
        ),
    )
    .await;
    assert_eq!(bad["ok"], false, "{bad}");
}

/// SearXNG over the wire: off until its URL is set and it is switched on;
/// the URL is checked and kept in the room's settings, its key in the vault
/// and never answered; clearing the URL switches it off; no other provider
/// takes a URL.
#[tokio::test]
async fn searxng_needs_its_url_before_it_is_on_and_its_key_never_leaves_the_vault() {
    use crate::credentials::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    let handle: Arc<dyn RoomHandle> = desk.clone();
    let log = &desk.log;
    let parts = desk.parts();
    let key = "searxng-private-key-0123456789";
    let ask = |cmd: &str, params: Value| {
        let frame = json!({"id": 1, "cmd": cmd, "params": params});
        let handle = handle.clone();
        let parts = &parts;
        async move { remote_control_answer(Seat::Desk, &handle, log, parts, frame).await }
    };
    let searxng = |answer: &Value| answer["result"]["providers"][4].clone();

    let early = ask(
        "websearch.set_enabled",
        json!({"provider": "searxng", "enabled": true}),
    )
    .await;
    assert_eq!(early["ok"], false, "{early}");
    assert!(early["error"].as_str().unwrap().contains("URL"), "{early}");

    for bad in [
        "localhost:8888",
        "https://me:secret@search.example.org",
        "https://search.example.org/?token=x",
        "file:///etc/passwd",
    ] {
        let refused = ask(
            "websearch.set_url",
            json!({"provider": "searxng", "url": bad}),
        )
        .await;
        assert_eq!(refused["ok"], false, "{bad}: {refused}");
    }
    let hosted = ask(
        "websearch.set_url",
        json!({"provider": "exa", "url": "https://exa.example"}),
    )
    .await;
    assert_eq!(hosted["ok"], false, "{hosted}");
    assert!(room::settings(log).get("webSearch").is_none());

    let set = ask(
        "websearch.set_url",
        json!({"provider": "searxng", "url": " http://localhost:8888/ "}),
    )
    .await;
    assert_eq!(
        searxng(&set),
        json!({"provider": "searxng", "name": "SearXNG", "enabled": false, "hasKey": false, "takesUrl": true, "url": "http://localhost:8888"}),
        "{set}"
    );

    let on = ask(
        "websearch.set_enabled",
        json!({"provider": "searxng", "enabled": true}),
    )
    .await;
    assert_eq!(searxng(&on)["enabled"], true, "{on}");
    assert_eq!(
        room::settings(log)["webSearch"],
        json!({"disabled": [], "enabled": ["searxng"], "urls": {"searxng": "http://localhost:8888"}})
    );
    // Switching a hosted provider keeps SearXNG as it was.
    ask(
        "websearch.set_enabled",
        json!({"provider": "exa", "enabled": false}),
    )
    .await;
    assert_eq!(
        room::settings(log)["webSearch"],
        json!({"disabled": ["exa"], "enabled": ["searxng"], "urls": {"searxng": "http://localhost:8888"}})
    );

    let keyed = ask(
        "websearch.set_key",
        json!({"provider": "searxng", "key": key}),
    )
    .await;
    assert_eq!(searxng(&keyed)["hasKey"], true, "{keyed}");
    assert!(!keyed.to_string().contains(key));
    assert_eq!(
        desk.saved_web_search_key(crate::contract::WebSearchProvider::Searxng),
        Some(key.to_string())
    );
    assert!(!format!("{:?}", log.load(&StreamId::Room)).contains(key));
    let status = ask("websearch.status", json!({})).await;
    assert!(!status.to_string().contains(key));

    let cleared = ask(
        "websearch.set_url",
        json!({"provider": "searxng", "url": null}),
    )
    .await;
    assert_eq!(searxng(&cleared)["enabled"], false, "{cleared}");
    assert!(searxng(&cleared).get("url").is_none(), "{cleared}");
    assert_eq!(
        room::settings(log)["webSearch"],
        json!({"disabled": ["exa"]})
    );
    // The key outlives the URL, as a key outlives its switch.
    assert_eq!(searxng(&cleared)["hasKey"], true, "{cleared}");

    // A hand-written setting is held to the same rules.
    let planted = ask(
        "settings.update",
        json!({"patch": {"webSearch": {"enabled": ["searxng"], "urls": {"searxng": "https://u:p@x.example"}}}}),
    )
    .await;
    assert_eq!(planted["ok"], false, "{planted}");
}

/// Link previews are a room setting, on by default. Off, the desk answers
/// every card and icon with nothing and contacts no site for one; on again,
/// the same asks reach the site. (In tests the desk contacts no site at
/// all; `link_preview::contacted` is what it would have asked.)
#[tokio::test]
async fn with_link_previews_off_the_desk_fetches_nothing() {
    let (_root, log, port) = door("link-previews-off");
    let mut socket = desk(port).await;
    let page = "https://1.2.3.4/link-previews-off";
    let icon_at = "https://1.2.3.5/favicon.ico";
    assert!(room::link_previews(&log), "on until turned off");

    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "settings.update", "params": { "patch": { "linkPreviews": false } } }),
    )
    .await;
    let off = answered(&mut socket, 1).await;
    assert_eq!(off["result"]["linkPreviews"], false, "{off}");
    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "link.preview", "params": { "url": page } }),
    )
    .await;
    ask(
        &mut socket,
        json!({ "id": 3, "cmd": "link.icon", "params": { "site": "1.2.3.5" } }),
    )
    .await;
    let card = answered(&mut socket, 2).await;
    let icon = answered(&mut socket, 3).await;
    assert_eq!(
        (card["ok"].clone(), card["result"].clone()),
        (json!(true), Value::Null),
        "{card}"
    );
    assert_eq!(
        (icon["ok"].clone(), icon["result"].clone()),
        (json!(true), Value::Null),
        "{icon}"
    );
    assert!(!crate::link_preview::contacted(page));
    assert!(!crate::link_preview::contacted(icon_at));

    ask(
        &mut socket,
        json!({ "id": 4, "cmd": "settings.update", "params": { "patch": { "linkPreviews": "no" } } }),
    )
    .await;
    assert_eq!(
        answered(&mut socket, 4).await["ok"],
        false,
        "only true or false"
    );

    ask(
        &mut socket,
        json!({ "id": 5, "cmd": "settings.update", "params": { "patch": { "linkPreviews": null } } }),
    )
    .await;
    assert_eq!(
        answered(&mut socket, 5).await["result"]["linkPreviews"],
        true
    );
    ask(
        &mut socket,
        json!({ "id": 6, "cmd": "link.preview", "params": { "url": page } }),
    )
    .await;
    ask(
        &mut socket,
        json!({ "id": 7, "cmd": "link.icon", "params": { "site": "1.2.3.5" } }),
    )
    .await;
    answered(&mut socket, 6).await;
    answered(&mut socket, 7).await;
    assert!(crate::link_preview::contacted(page));
    assert!(crate::link_preview::contacted(icon_at));
}
