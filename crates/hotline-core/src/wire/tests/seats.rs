use super::*;
use serde_json::Map;

#[tokio::test]
async fn human_answer_is_a_command_the_wire_can_read() {
    let (_root, _log, port) = door("human-answer");
    let mut socket = desk(port).await;

    ask(
        &mut socket,
        json!({
            "id": 1,
            "cmd": "human.answer",
            "params": { "personaId": "ada", "actionId": "act-1", "status": "done" },
        }),
    )
    .await;
    let answered = heard(&mut socket).await;
    assert_eq!(answered["id"], 1);
    assert_eq!(answered["ok"], true, "{answered}");

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "human.answer",
            "params": { "personaId": "ada", "actionId": "act-1", "status": "declined", "note": "not now" },
        }),
    )
    .await;
    let declined = heard(&mut socket).await;
    assert_eq!(declined["ok"], true, "{declined}");
}

/// An id off the wire is not a teammate this room has, and one with no
/// characters in it names the transcripts directory rather than a tape in it.
/// It is answered like any other stranger, because a command that panics is a
/// command that is never answered at all — and it takes the socket with it.
#[tokio::test]
async fn a_command_naming_a_teammate_with_no_id_is_answered() {
    let (_root, log, port) = door("blank-id");
    let _indexer = crate::store::search::Indexer::open(&log).unwrap();
    let mut socket = desk(port).await;

    for (id, cmd, params) in [
        (1, "chapter.list", json!({ "personaId": "" })),
        (2, "peers.list", json!({ "personaId": "" })),
        (
            3,
            "search.thread",
            json!({ "personaId": "", "query": "crane" }),
        ),
        (4, "teammate.tools", json!({ "personaId": "" })),
    ] {
        ask(
            &mut socket,
            json!({ "id": id, "cmd": cmd, "params": params }),
        )
        .await;
        let answer = answered(&mut socket, id).await;
        assert_eq!(answer["ok"], true, "{cmd}: {answer}");
    }

    // The socket is still the one that answered the first of them.
    let created = create(&mut socket, 5, "Ada").await;
    assert_eq!(created["name"], "Ada");
}

/// A subscription to a tape that cannot exist is still a subscription: it is
/// acknowledged, snapshotted empty, and closed by an unsubscribe.
#[tokio::test]
async fn a_tape_subscription_for_a_teammate_with_no_id_snapshots_nothing() {
    let (_root, _log, port) = door("blank-id-sub");
    let mut socket = desk(port).await;

    ask(&mut socket, json!({ "id": 1, "sub": { "tape": "" } })).await;
    let opened = answered(&mut socket, 1).await;
    assert_eq!(opened["ok"], true, "{opened}");
    let snapshot = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        heard_where(&mut socket, |frame| frame["sub"] == 1),
    )
    .await
    .expect("the snapshot never arrived");
    assert_eq!(snapshot["snapshot"], json!([]));
}

#[test]
fn the_desk_seat_may_do_everything_the_room_can_do() {
    assert!(Seat::Desk.permits(&Command::ModelsList {}));
    assert!(Seat::Desk.permits(&Command::ModelsCatalog {
        provider_id: "anthropic".to_string()
    }));
    assert!(Seat::Desk.permits(&Command::PersonaDelete {
        id: "ada".to_string()
    }));
    assert!(Seat::Desk.permits_sub(&Target::Room));
    assert!(Seat::Desk.permits_sub(&Target::View(ViewName::Roster)));
}

#[test]
fn the_phone_seat_may_watch_and_stop_a_computer_but_not_remove_or_remake_it() {
    let persona_id = "ada".to_string();
    assert!(Seat::Phone.permits(&Command::ComputerStatus {
        persona_id: persona_id.clone()
    }));
    assert!(Seat::Phone.permits(&Command::ComputerStop {
        persona_id: persona_id.clone()
    }));
    assert!(!Seat::Phone.permits(&Command::ComputerRestart {
        persona_id: persona_id.clone()
    }));
    assert!(!Seat::Phone.permits(&Command::ComputerRemove { persona_id }));
    assert!(!Seat::Phone.permits(&Command::ComputerRuntimes {}));
}

/// A process a teammate left running is the desk's and the owner's to read
/// and stop; a companion phone has neither, as it has no `session.stop`.
#[test]
fn process_chip_only_the_desk_and_the_owner_reach_a_process() {
    let commands = [
        Command::ProcessStop {
            persona_id: "ada".to_string(),
            id: "call_serve".to_string(),
        },
        Command::ProcessOutput {
            persona_id: "ada".to_string(),
            id: "call_serve".to_string(),
        },
    ];
    for command in &commands {
        assert!(Seat::Desk.permits(command), "{command:?}");
        assert!(Seat::Owner.permits(command), "{command:?}");
        assert!(!Seat::Phone.permits(command), "{command:?}");
        assert!(!Seat::Link.permits(command), "{command:?}");
    }
}

/// A failed turn is tried again from wherever the person is: the line is
/// already on the tape, so the phone resends nothing it had not sent.
#[test]
fn the_phone_seat_may_try_a_failed_turn_again() {
    assert!(Seat::Phone.permits(&Command::SessionRetry {
        persona_id: "ada".to_string()
    }));
}

/// Who a teammate is — its name and goal — and whether it stays are the
/// person's to change from anywhere; the patch that reaches its grants is
/// not, so the phone gets a narrow edit and never `persona.update`.
#[test]
fn the_phone_seat_renames_and_deletes_a_teammate_but_never_patches_one() {
    assert!(Seat::Phone.permits(&Command::MobilePersonaUpdate {
        id: "ada".to_string(),
        name: Some("Ada Lovelace".to_string()),
        goal: None,
    }));
    assert!(Seat::Phone.permits(&Command::PersonaDelete {
        id: "ada".to_string()
    }));
    assert!(!Seat::Phone.permits(&Command::PersonaUpdate {
        id: "ada".to_string(),
        patch: json!({ "name": "Ada Lovelace" }),
    }));
}

/// A teammate's standing access is the owner phone's to change and never a
/// companion's, and only through the narrow command.
#[test]
fn only_the_owner_phone_changes_a_teammates_access() {
    let access = Command::MobilePersonaAccess {
        id: "ada".to_string(),
        reach: Some(Reach::Machine),
        mode_id: None,
        background_work: Some(true),
    };
    assert!(Seat::Owner.permits(&access));
    assert!(!Seat::Phone.permits(&access));
    {
        let seat = Seat::Phone;
        assert!(!seat.permits(&Command::PersonaUpdate {
            id: "ada".to_string(),
            patch: json!({ "reach": "machine" }),
        }));
        assert!(!seat.permits(&Command::SessionSetMode {
            persona_id: "ada".to_string(),
            mode_id: "bypassPermissions".to_string(),
        }));
    }
    assert!(Seat::Owner.capabilities().contains(&"personaAccess"));
    assert!(!Seat::Phone.capabilities().contains(&"personaAccess"));
}

/// Reach and background work are written for a Hotline Agent teammate, and
/// a mode is refused for one; a harness teammate keeps a mode for its next
/// start, and is refused a reach. Nothing to change is refused too.
#[tokio::test]
async fn mobile_persona_access_sets_reach_mode_and_background_work() {
    let (_root, log, port) = door("mobile-access");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "mobile.persona_access",
            "params": { "id": ada, "reach": "machine", "backgroundWork": true },
        }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["result"]["reach"], "machine");
    assert_eq!(answer["result"]["backgroundWork"], true);

    ask(
        &mut socket,
        json!({
            "id": 3,
            "cmd": "persona.create",
            "params": { "draft": { "name": "Bea", "backendId": "claude-acp" } },
        }),
    )
    .await;
    let bea = answered(&mut socket, 3).await["result"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    ask(
        &mut socket,
        json!({
            "id": 4,
            "cmd": "mobile.persona_access",
            "params": { "id": bea, "modeId": "plan", "backgroundWork": true },
        }),
    )
    .await;
    let answer = answered(&mut socket, 4).await;
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["result"]["modeId"], "plan");
    assert_eq!(answer["result"]["backgroundWork"], true);

    for (n, params, needle) in [
        (5, json!({ "id": ada, "modeId": "plan" }), "no mode"),
        (6, json!({ "id": bea, "reach": "machine" }), "has a reach"),
        (7, json!({ "id": ada }), "Nothing to change"),
        (8, json!({ "id": bea, "modeId": " " }), "needs an id"),
    ] {
        ask(
            &mut socket,
            json!({ "id": n, "cmd": "mobile.persona_access", "params": params }),
        )
        .await;
        let refused = answered(&mut socket, n).await;
        assert_eq!(refused["ok"], false, "{refused}");
        assert!(
            refused["error"].as_str().unwrap().contains(needle),
            "{refused}"
        );
    }
    let kept = room::roster(&log);
    let ada = kept.iter().find(|persona| persona.name == "Ada").unwrap();
    assert_eq!(ada.reach, Some(Reach::Machine));
    assert!(ada.mode_id.is_none());
}

/// The phone's edit writes a name and a goal and nothing else, whatever
/// else its JSON carries; a blank name, an empty edit and a teammate the
/// room does not hold are each refused with a sentence.
#[tokio::test]
async fn mobile_persona_update_changes_only_the_name_and_goal() {
    let (_root, log, port) = door("mobile-update");
    let mut socket = desk(port).await;
    let created = create(&mut socket, 1, "Ada").await;
    let id = created["id"].as_str().unwrap().to_string();

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "mobile.persona_update",
            "params": {
                "id": id,
                "name": "  Ada Lovelace ",
                "goal": "Keep the harbour running",
                "reach": "machine",
                "cwd": "/etc",
                "backgroundWork": true,
                "computer": { "enabled": true },
            },
        }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    let updated = &answer["result"];
    assert_eq!(updated["name"], "Ada Lovelace");
    assert_eq!(updated["goal"], "Keep the harbour running");
    assert!(updated.get("reach").is_none(), "{updated}");
    assert!(updated.get("computer").is_none(), "{updated}");
    assert_eq!(updated["backgroundWork"], false);
    assert_eq!(updated["cwd"], created["cwd"]);

    // An absent field is left alone; an empty goal clears it.
    ask(
        &mut socket,
        json!({ "id": 3, "cmd": "mobile.persona_update", "params": { "id": id, "goal": " " } }),
    )
    .await;
    let cleared = answered(&mut socket, 3).await["result"].clone();
    assert_eq!(cleared["name"], "Ada Lovelace");
    assert_eq!(cleared["goal"], "");

    for (n, params, needle) in [
        (4, json!({ "id": id, "name": "   " }), "needs a name"),
        (5, json!({ "id": id }), "Nothing to change"),
        (6, json!({ "id": "nobody", "name": "Bob" }), ""),
    ] {
        ask(
            &mut socket,
            json!({ "id": n, "cmd": "mobile.persona_update", "params": params }),
        )
        .await;
        let refused = answered(&mut socket, n).await;
        assert_eq!(refused["ok"], false, "{refused}");
        assert!(
            refused["error"].as_str().unwrap().contains(needle),
            "{refused}"
        );
    }
    let kept = room::roster(&log);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].name, "Ada Lovelace");
}

/// The phone gets a narrow create of its own, and a read of what harnesses
/// this desk can run — but never the full `persona.create`, which can name
/// a reach, a path and a computer the phone posture must not touch.
#[test]
fn the_phone_seat_creates_a_teammate_narrowly_but_not_with_persona_create() {
    assert!(Seat::Phone.permits(&Command::MobilePersonaCreate {
        request_id: "3fbb7d63-0a3e-4c6a-9c0e-8f6f8e8e6b39".to_string(),
        name: "Ada".to_string(),
        goal: None,
        backend_id: None,
        model_id: None,
        effort_id: None,
    }));
    assert!(Seat::Phone.permits(&Command::BackendsList {}));
    assert!(!Seat::Phone.permits(&Command::PersonaCreate {
        draft: PersonaDraft {
            name: "Ada".to_string(),
            goal: None,
            team: None,
            backend_id: None,
            cwd: None,
            reach: None,
            folders: None,
            model_id: None,
            effort_id: None,
            computer: None,
            background_work: None,
            agent_account_id: None,
        }
    }));
}

/// Cookie import reads the person's own machine, so it is the desk's alone:
/// the phone cannot list host browsers, preview, or import, and there is no
/// agent tool for any of it. This is the enforcement point behind the promise
/// that the agent can never pull cookies itself.
#[test]
fn only_the_desk_seat_may_import_host_cookies() {
    let browsers = Command::ComputerBrowsersList {};
    let preview = Command::ComputerCookiesPreview {
        browser_id: "chrome".to_string(),
        profile_id: "Default".to_string(),
    };
    let import = Command::ComputerCookiesImport {
        persona_id: "ada".to_string(),
        browser_id: "chrome".to_string(),
        profile_id: "Default".to_string(),
        domains: vec!["example.com".to_string()],
    };
    let list = Command::ComputerCookiesList {
        persona_id: "ada".to_string(),
    };
    let forget = Command::ComputerCookiesForget {
        persona_id: "ada".to_string(),
        browser_id: "chrome".to_string(),
        profile_id: "Default".to_string(),
        domain: Some("example.com".to_string()),
    };
    for command in [&browsers, &preview, &import, &list, &forget] {
        assert!(Seat::Desk.permits(command), "{command:?}");
        assert!(!Seat::Phone.permits(command), "{command:?}");
    }
}

/// Stored secrets are the desk's alone: the phone can neither list, store
/// nor delete one, and there is no agent tool for any of it. With the grant
/// itself living on `persona.update`, which the phone may not send either,
/// nothing a model says can put a secret in front of a teammate.
#[test]
fn only_the_desk_seat_may_touch_stored_secrets() {
    let list = Command::SecretsList {};
    let set = Command::SecretsSet {
        name: "GITHUB_TOKEN".to_string(),
        value: "ghp_notarealtoken0001".to_string(),
    };
    let delete = Command::SecretsDelete {
        name: "GITHUB_TOKEN".to_string(),
    };
    let login = Command::SecretsLoginSet {
        name: "GITHUB_LOGIN".to_string(),
        sites: vec!["https://github.com".to_string()],
        username: "george".to_string(),
        password: "correct-horse-battery".to_string(),
        totp: None,
    };
    let register = Command::SecretsPasskeyRegister {
        name: "GITHUB_PASSKEY".to_string(),
        persona_id: "ada".to_string(),
        rp_id: "github.com".to_string(),
    };
    let registration = Command::SecretsPasskeyRegistration {
        persona_id: "ada".to_string(),
    };
    let cancel = Command::SecretsPasskeyCancel {
        persona_id: "ada".to_string(),
    };
    for command in [
        &list,
        &set,
        &delete,
        &login,
        &register,
        &registration,
        &cancel,
    ] {
        assert!(Seat::Desk.permits(command), "{command:?}");
        assert!(!Seat::Phone.permits(command), "{command:?}");
    }
    assert!(!Seat::Phone.permits(&Command::PersonaUpdate {
        id: "ada".to_string(),
        patch: serde_json::json!({"computer": {"enabled": true, "secrets": ["GITHUB_TOKEN"]}}),
    }));
}

#[test]
fn only_the_desk_and_the_owner_reach_agent_accounts() {
    let commands: Vec<Command> = super::accounts::account_frames("Desk")
        .iter()
        .map(|frame| crate::wire::socket::read_command(frame).unwrap())
        .collect();
    assert_eq!(commands.len(), 12);
    for c in &commands {
        assert!(Seat::Desk.permits(c) && Seat::Owner.permits(c), "{c:?}");
        assert!(!Seat::Phone.permits(c) && !Seat::Link.permits(c), "{c:?}");
    }
}

/// Only person seats stop or resume automatic exchanges.
#[test]
fn the_phone_seat_resumes_and_stops_exchanges_for_the_person() {
    let (a, b) = ("ada".to_string(), "bob".to_string());
    for command in [
        Command::TeammatesExchangeStop {
            a: a.clone(),
            b: b.clone(),
        },
        Command::TeammatesExchangeResume { a, b },
    ] {
        assert!(Seat::Desk.permits(&command), "{command:?}");
        assert!(Seat::Phone.permits(&command), "{command:?}");
    }
}

/// A phone answers what a teammate is waiting on and sets how it thinks.
/// What it may reach, and any standing posture, stay at the desk.
#[test]
fn the_phone_seat_answers_for_the_person_but_never_grants_a_standing_one() {
    let persona_id = "ada".to_string();
    assert!(Seat::Phone.permits(&Command::HumanAnswer {
        persona_id: persona_id.clone(),
        action_id: "act".to_string(),
        status: crate::contract::HumanAnswer::Done,
        note: None,
    }));
    // A passkey card is one answer to one request the person armed for at
    // the desk; the arming, and the register that starts it, stay there.
    assert!(Seat::Phone.permits(&Command::SecretsPasskeyAnswer {
        persona_id: persona_id.clone(),
        ask_id: "ask-1".to_string(),
        approved: true,
    }));
    assert!(Seat::Phone.permits(&Command::SessionSetModel {
        persona_id: persona_id.clone(),
        model_id: "anthropic/claude".to_string(),
    }));
    assert!(Seat::Phone.permits(&Command::SessionSetConfig {
        persona_id: persona_id.clone(),
        config_id: "effort".to_string(),
        value: "high".to_string(),
    }));
    // One answer to one request, wherever the person is standing.
    assert!(Seat::Phone.permits(&Command::SessionAnswerPermission {
        persona_id: persona_id.clone(),
        request_id: "req".to_string(),
        option_id: "allow".to_string(),
    }));
    // What a resting teammate could be set to is a list, not a grant.
    assert!(Seat::Phone.permits(&Command::ModelsList {}));
    assert!(Seat::Phone.permits(&Command::ModelsEfforts {
        model_id: "anthropic/claude".to_string(),
    }));
    // A standing grant is a different thing, and a harness mode is one.
    assert!(!Seat::Phone.permits(&Command::SessionSetMode {
        persona_id: persona_id.clone(),
        mode_id: "bypassPermissions".to_string(),
    }));
    assert!(!Seat::Phone.permits(&Command::PersonaUpdate {
        id: persona_id,
        patch: Value::default(),
    }));
}

/// A phone reads one teammate's schedules and nothing it could change them
/// or the room with: no job made, cancelled or quieted, and no room stream,
/// which is where a setting would reach it. A thread between two teammates
/// and a subagent's run are read like a tape: they hold what was said.
#[test]
fn the_phone_seat_reads_a_teammates_schedules_but_changes_none_of_them() {
    use crate::contract::ScheduleKind;
    assert!(Seat::Phone.permits_sub(&Target::Schedules("ada".to_string())));
    assert!(Seat::Phone.permits_sub(&Target::Tape("ada".to_string())));
    assert!(Seat::Phone.permits_sub(&Target::Thread("ada~bob".to_string())));
    assert!(Seat::Phone.permits_sub(&Target::View(ViewName::Roster)));
    assert!(Seat::Phone.permits_sub(&Target::Run("run-1".to_string())));
    assert!(!Seat::Phone.permits_sub(&Target::Room));
    let create = Command::ScheduleCreate {
        persona_id: "ada".to_string(),
        kind: ScheduleKind::Loop,
        when: None,
        every: Some(60_000),
        prompt: "sweep the inbox".to_string(),
        quiet: None,
        reminder: None,
    };
    let cancel = Command::ScheduleCancel {
        id: "job-1".to_string(),
    };
    let quiet = Command::ScheduleSetQuiet {
        id: "job-1".to_string(),
        quiet: true,
    };
    // The whole room's list is every teammate's; a phone asks for one.
    let list = Command::ScheduleList {};
    let settings = Command::SettingsUpdate {
        patch: Map::default(),
    };
    for command in [&create, &cancel, &quiet, &list, &settings] {
        assert!(Seat::Desk.permits(command), "{command:?}");
        assert!(!Seat::Phone.permits(command), "{command:?}");
    }
    assert!(Seat::Desk.permits_sub(&Target::Schedules("ada".to_string())));
}

/// The seat lets `session.set_config` through, so the narrowing happens where
/// the categories are known: effort is the one a phone may set, whatever a
/// harness happens to call it, and anything else a harness serves is not.
#[test]
fn only_a_config_the_session_calls_effort_is_a_phones_to_set() {
    use crate::contract::{SessionConfig, SessionConfigCategory};
    let room = Arc::new(Quiet::new());
    let mut info = idle("ada");
    info.configs = vec![
        SessionConfig {
            id: "reasoning".to_string(),
            name: "Reasoning".to_string(),
            category: Some(SessionConfigCategory::Effort),
            current_id: Some("high".to_string()),
            options: Vec::new(),
        },
        SessionConfig {
            id: "approval".to_string(),
            name: "Approvals".to_string(),
            category: None,
            current_id: Some("ask".to_string()),
            options: Vec::new(),
        },
    ];
    room.set_info(info);
    let handle: Arc<dyn RoomHandle> = room;
    assert!(super::effort_config(&handle, "ada", "reasoning"));
    assert!(!super::effort_config(&handle, "ada", "approval"));
    assert!(!super::effort_config(
        &handle,
        "ada",
        "nothing-by-that-name"
    ));
    assert!(!super::effort_config(&handle, "bob", "reasoning"));
}

/// A frame the phone can afford travels as a small JPEG; one it cannot
/// decode is stripped as before, and text is untouched either way.
#[test]
fn a_computer_frame_reaches_the_phone_as_a_small_jpeg_or_not_at_all() {
    use base64::{Engine, prelude::BASE64_STANDARD};
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbImage::from_fn(1600, 900, |x, y| {
        image::Rgb([(x % 256) as u8, (y % 256) as u8, 90])
    })
    .write_to(&mut png, image::ImageFormat::Png)
    .unwrap();
    let frame = json!({
        "kind": "computer_frame", "id": "f1", "ts": 1,
        "dataUrl": format!("data:image/png;base64,{}", BASE64_STANDARD.encode(png.into_inner())),
    });
    let small = phone_event(frame);
    let data_url = small["dataUrl"].as_str().unwrap();
    assert!(data_url.starts_with("data:image/jpeg;base64,"));
    let jpeg = BASE64_STANDARD
        .decode(&data_url["data:image/jpeg;base64,".len()..])
        .unwrap();
    assert!(jpeg.len() <= 96 * 1024);
    let decoded = image::load_from_memory(&jpeg).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (720, 405));
    assert!(small.get("mobileTruncated").is_none());

    let junk = json!({"kind": "computer_frame", "id": "f2", "ts": 2, "dataUrl": "data:image/png;base64,AAAA"});
    let stripped = phone_event(junk);
    assert!(stripped.get("dataUrl").is_none());
    assert_eq!(stripped["mobileTruncated"], true);
}

#[tokio::test]
async fn auth_wire_allows_only_desktop_and_disconnect_revokes_its_owner() {
    let quiet = Arc::new(Quiet::new());
    let (_root, log, port) = door_with("auth-wire", quiet.clone());
    let room = crate::wire::Agents {
        log: log.clone(),
        room: quiet.clone(),
        parts: Parts::default(),
    };
    let (tx, mut frames) = mpsc::unbounded_channel();
    let outbox = Outbox {
        call_owner: "desk".into(),
        pairing_cancel: tokio_util::sync::CancellationToken::new(),
        auth_attempts: Arc::default(),
        account_logins: Arc::default(),
        pairing: Arc::default(),
        uploads: Arc::default(),
        threads2: Arc::default(),
        roster_options: Arc::default(),
        lean: Arc::default(),
        sender: Outgoing::Desk(tx),
        cancel: tokio_util::sync::CancellationToken::new(),
        max: usize::MAX,
    };
    let commands = [
        (
            "agent.auth.start",
            json!({"personaId":"ada","methodId":"fixture", "command":"ignored-ui-command", "args":["ignored"]}),
        ),
        (
            "agent.auth.poll",
            json!({"personaId":"ada","id":"fixture-attempt"}),
        ),
        (
            "agent.auth.input",
            json!({"personaId":"ada","id":"fixture-attempt","input":"fixture input"}),
        ),
        (
            "agent.auth.cancel",
            json!({"personaId":"ada","id":"fixture-attempt"}),
        ),
    ];
    for (cmd, params) in &commands {
        let frame = json!({"id":1,"cmd":cmd,"params":params});
        answer(
            &frame,
            Seat::Phone,
            &room,
            &outbox,
            &mut HashMap::new(),
            None,
        )
        .await;
        let response: Value = serde_json::from_str(&frames.recv().await.unwrap()).unwrap();
        assert_eq!(response["ok"], false, "{response}");
        assert_eq!(response["code"], FORBIDDEN, "{response}");
    }
    assert!(
        quiet.auth_owner.lock().unwrap().is_none(),
        "phone never reached auth"
    );
    let mut socket = desk(port).await;
    for (index, (cmd, params)) in commands.iter().enumerate() {
        let id = index as i64 + 1;
        ask(&mut socket, json!({"id":id,"cmd":cmd,"params":params})).await;
        let response = heard_where(&mut socket, |frame| frame["id"] == id).await;
        assert_eq!(response["ok"], true, "{response}");
        if index == 0 {
            assert_eq!(response["result"]["id"], "fixture-attempt");
        }
        if index == 1 {
            assert_eq!(response["result"]["output"], "ephemeral fixture output");
        }
    }
    // A second authenticated desk knows the UUID but cannot use it as authority.
    let mut other = desk(port).await;
    for (index, (cmd, params)) in commands.iter().enumerate().skip(1) {
        let id = index as i64 + 10;
        ask(&mut other, json!({"id":id,"cmd":cmd,"params":params})).await;
        let response = answered(&mut other, id).await;
        assert_eq!(response["ok"], false, "{response}");
        assert!(
            response["error"]
                .as_str()
                .unwrap()
                .contains("another desktop")
        );
    }
    ask(&mut socket, json!({"id":20,"cmd":"agent.auth.poll","params":{"personaId":"ada","id":"fixture-attempt"}})).await;
    let response = answered(&mut socket, 20).await;
    assert_eq!(response["result"]["output"], "ephemeral fixture output");
    let owner = quiet.auth_owner.lock().unwrap().clone().unwrap();
    assert!(!owner.is_cancelled());
    socket.close(None).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), owner.cancelled())
        .await
        .unwrap();
    assert_eq!(log.load(&StreamId::Tape("ada".into())), [] as [Value; 0]);
}

#[tokio::test]
async fn account_sign_in_belongs_to_its_desktop_socket_and_disconnect_revokes_it() {
    let quiet = Arc::new(Quiet::new());
    let (_root, log, port) = door_with("account-sign-in-wire", quiet.clone());
    let room = crate::wire::Agents {
        log: log.clone(),
        room: quiet.clone(),
        parts: Parts::default(),
    };
    let (tx, mut frames) = mpsc::unbounded_channel();
    let outbox = Outbox {
        call_owner: "desk".into(),
        pairing_cancel: tokio_util::sync::CancellationToken::new(),
        auth_attempts: Arc::default(),
        account_logins: Arc::default(),
        pairing: Arc::default(),
        uploads: Arc::default(),
        threads2: Arc::default(),
        roster_options: Arc::default(),
        lean: Arc::default(),
        sender: Outgoing::Desk(tx),
        cancel: tokio_util::sync::CancellationToken::new(),
        max: usize::MAX,
    };
    let commands = [
        (
            "accounts.login.start",
            json!({"accountId":"acct-1","email":"ada@example.com"}),
        ),
        (
            "accounts.login.poll",
            json!({"accountId":"acct-1","id":"fixture-attempt"}),
        ),
        (
            "accounts.login.input",
            json!({"accountId":"acct-1","id":"fixture-attempt","input":"fixture input"}),
        ),
        (
            "accounts.login.cancel",
            json!({"accountId":"acct-1","id":"fixture-attempt"}),
        ),
    ];
    for (cmd, params) in &commands {
        let frame = json!({"id":1,"cmd":cmd,"params":params});
        answer(
            &frame,
            Seat::Phone,
            &room,
            &outbox,
            &mut HashMap::new(),
            None,
        )
        .await;
        let response: Value = serde_json::from_str(&frames.recv().await.unwrap()).unwrap();
        assert_eq!(response["ok"], false, "{response}");
        assert_eq!(response["code"], FORBIDDEN, "{response}");
    }
    assert!(quiet.account_sign_ins.lock().unwrap().is_empty());
    let mut socket = desk(port).await;
    for (index, (cmd, params)) in commands.iter().enumerate() {
        let id = index as i64 + 1;
        ask(&mut socket, json!({"id":id,"cmd":cmd,"params":params})).await;
        let response = answered(&mut socket, id).await;
        assert_eq!(response["ok"], true, "{response}");
        if index == 0 {
            assert_eq!(response["result"]["id"], "fixture-attempt");
        }
        if index == 1 {
            assert_eq!(response["result"]["output"], "ephemeral fixture output");
        }
    }
    let mut other = desk(port).await;
    for (index, (cmd, params)) in commands.iter().enumerate().skip(1) {
        let id = index as i64 + 10;
        ask(&mut other, json!({"id":id,"cmd":cmd,"params":params})).await;
        let response = answered(&mut other, id).await;
        assert_eq!(response["ok"], false, "{response}");
        assert!(
            response["error"]
                .as_str()
                .unwrap()
                .contains("another desktop"),
            "{response}"
        );
    }
    let (owner, seat_is_desk) = quiet.account_sign_ins.lock().unwrap()[0].clone();
    assert!(seat_is_desk);
    assert_eq!(quiet.account_sign_ins.lock().unwrap().len(), 1);
    assert!(!owner.is_cancelled());
    socket.close(None).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), owner.cancelled())
        .await
        .unwrap();
    for (cmd, params) in &commands {
        let command =
            crate::wire::socket::read_command(&json!({"cmd":cmd,"params":params})).unwrap();
        assert_eq!(
            crate::wire::commands::run(command, &log, &room.room, &Parts::default()).await,
            Err("Sign-in requires its owning desktop connection.".to_string())
        );
    }
}

#[tokio::test]
async fn remote_controls_are_denied_to_companions() {
    use crate::credentials::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::default());
    let desk = Arc::new(crate::desk::Desk::open_with_store(root.path(), store.clone()).unwrap());
    let room: Arc<dyn RoomHandle> = desk.clone();
    let agents = crate::wire::Agents {
        log: desk.log.clone(),
        room: room.clone(),
        parts: desk.parts(),
    };
    let remote = crate::remote::Remote::open_with_store(root.path(), agents, store).unwrap();
    let parts = Parts {
        remote: Arc::downgrade(&remote),
        ..desk.parts()
    };

    {
        let seat = Seat::Phone;
        for (cmd, params) in [
            ("remote.status", json!({})),
            ("remote.configure", json!({"enabled":true,"host":"all"})),
            ("remote.devices", json!({})),
            ("remote.revoke", json!({"deviceId":"any"})),
            ("remote.relay", json!({"deskId":"any"})),
            ("remote.relays", json!({})),
            ("remote.become_relay", json!({})),
            (
                "remote.public_url",
                json!({"url":"https://desk.example.com"}),
            ),
            ("remote.pairing", json!({})),
            ("remote.pairing", json!({"role":"companion"})),
            ("remote.pairing", json!({"id":"any"})),
            ("remote.pairing", json!({"id":"any","cancel":true})),
            (
                "persona.update",
                json!({"id":"any","patch":{"reach":"machine"}}),
            ),
            ("settings.update", json!({"patch":{"remote":true}})),
        ] {
            let response = remote_control_answer(
                seat,
                &room,
                &desk.log,
                &parts,
                json!({"id":1,"cmd":cmd,"params":params}),
            )
            .await;
            assert_eq!(response["code"], FORBIDDEN, "{seat:?} {cmd}: {response}");
        }
        let response =
            remote_control_answer(seat, &room, &desk.log, &parts, json!({"id":1,"sub":"room"}))
                .await;
        assert_eq!(response["code"], FORBIDDEN, "{seat:?}: {response}");
        // Both roles retain the same useful, read-only phone command.
        let response = remote_control_answer(
            seat,
            &room,
            &desk.log,
            &parts,
            json!({"id":1,"cmd":"models.list"}),
        )
        .await;
        assert_eq!(response["ok"], true, "{seat:?}: {response}");
    }
    assert!(
        !remote.status().enabled,
        "denied configure must not reach Remote"
    );
    assert!(remote.devices().is_empty());

    for (cmd, params) in [
        ("remote.status", json!({})),
        ("remote.configure", json!({"enabled":false,"host":"all"})),
        ("remote.devices", json!({})),
        ("remote.revoke", json!({"deviceId":"already-absent"})),
    ] {
        let response = remote_control_answer(
            Seat::Desk,
            &room,
            &desk.log,
            &parts,
            json!({"id":1,"cmd":cmd,"params":params}),
        )
        .await;
        assert_eq!(response["ok"], true, "{cmd}: {response}");
    }
    let weak = Arc::downgrade(&remote);
    drop(remote);
    assert!(
        weak.upgrade().is_none(),
        "desk controls must not form a strong cycle"
    );
    assert!(parts.remote.upgrade().is_none());
    let response = remote_control_answer(
        Seat::Desk,
        &room,
        &desk.log,
        &parts,
        json!({"id":1,"cmd":"remote.status"}),
    )
    .await;
    assert_eq!(response["ok"], false);
    assert!(
        response["error"]
            .as_str()
            .unwrap()
            .contains("not available")
    );
}

#[tokio::test]
async fn desk_pairing_commands_start_poll_and_cancel_sealed_pairing() {
    use crate::credentials::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::default());
    let desk = Arc::new(crate::desk::Desk::open_with_store(root.path(), store.clone()).unwrap());
    let room: Arc<dyn RoomHandle> = desk.clone();
    let agents = crate::wire::Agents {
        log: desk.log.clone(),
        room: room.clone(),
        parts: desk.parts(),
    };
    let remote = crate::remote::Remote::open_with_store(root.path(), agents, store).unwrap();
    let parts = Parts {
        remote: Arc::downgrade(&remote),
        ..desk.parts()
    };
    let response = remote_control_answer(
        Seat::Desk,
        &room,
        &desk.log,
        &parts,
        json!({"id":1,"cmd":"remote.configure","params":{"enabled":true,"host":"all"}}),
    )
    .await;
    assert_eq!(response["ok"], true, "{response}");
    for params in [json!({}), json!({"role":"companion"})] {
        let response = remote_control_answer(
            Seat::Desk,
            &room,
            &desk.log,
            &parts,
            json!({"id":1,"cmd":"remote.pairing","params":params}),
        )
        .await;
        assert_eq!(response["ok"], true, "{response}");
        assert!(
            response["result"]["url"]
                .as_str()
                .unwrap()
                .starts_with("hotline://pair?v=2")
        );
        assert!(response["result"].get("manual").is_none());
        let id = response["result"]["id"].as_str().unwrap();
        let pending = remote_control_answer(
            Seat::Desk,
            &room,
            &desk.log,
            &parts,
            json!({"id":1,"cmd":"remote.pairing","params":{"id":id}}),
        )
        .await;
        assert_eq!(pending, json!({"id":1,"ok":true,"result":null}));
        let cancelled = remote_control_answer(
            Seat::Desk,
            &room,
            &desk.log,
            &parts,
            json!({"id":1,"cmd":"remote.pairing","params":{"id":id,"cancel":true}}),
        )
        .await;
        assert_eq!(cancelled, json!({"id":1,"ok":true}));
    }
    for params in [json!({"cancel":true}), json!({"id":"any","role":"owner"})] {
        let refused = remote_control_answer(
            Seat::Desk,
            &room,
            &desk.log,
            &parts,
            json!({"id":1,"cmd":"remote.pairing","params":params}),
        )
        .await;
        assert_eq!(refused["ok"], false, "{refused}");
    }
    remote.configure(false, "all").await.unwrap();
}

#[tokio::test]
async fn a_desktop_disconnect_does_not_drop_an_inflight_mutation() {
    let pending = Arc::new(PendingStop::default());
    let mut room = Quiet::new();
    room.pending_stop = Some(pending.clone());
    let (_root, _log, port) = door_with("desk-pending-stop", Arc::new(room));
    let mut socket = desk(port).await;
    let persona = create(&mut socket, 0, "Ada").await;
    ask(
        &mut socket,
        json!({"id":1,"cmd":"computer.stop","params":{"personaId":persona["id"]}}),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(3), pending.entered.cancelled())
        .await
        .unwrap();
    socket.close(None).await.unwrap();
    drop(socket);
    // Unlike a revoked phone, the local desk's mutation must finish.
    assert!(
        tokio::time::timeout(Duration::from_millis(50), pending.dropped.cancelled())
            .await
            .is_err()
    );
    pending.release.cancel();
    tokio::time::timeout(Duration::from_secs(3), pending.dropped.cancelled())
        .await
        .unwrap();
    assert!(pending.completed.is_cancelled());
}
