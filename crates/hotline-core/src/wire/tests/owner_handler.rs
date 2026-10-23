use super::*;

#[tokio::test]
async fn owner_commands_and_room_subscription_reach_the_real_handler() {
    use crate::credentials::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    let room: Arc<dyn RoomHandle> = desk.clone();
    for seat in [Seat::Owner, Seat::Desk] {
        let answer = remote_control_answer(seat, &room, &desk.log, &desk.parts(),
            json!({"id":1,"cmd":"settings.update","params":{"patch":{"skillsHome":"/tmp/owner-skills"}}})).await;
        assert_eq!(answer["ok"], true, "{answer}");
        let answer = remote_control_answer(
            seat,
            &room,
            &desk.log,
            &desk.parts(),
            json!({"id":2,"sub":"room"}),
        )
        .await;
        assert_eq!(answer["ok"], true, "{answer}");
    }
    let answer = remote_control_answer(Seat::Phone, &room, &desk.log, &desk.parts(),
        json!({"id":3,"cmd":"settings.update","params":{"patch":{"skillsHome":"/tmp/companion-skills"}}})).await;
    assert_eq!(answer["code"], FORBIDDEN);
    assert_eq!(
        crate::room::settings(&desk.log)["skillsHome"],
        "/tmp/owner-skills"
    );
}

#[tokio::test]
async fn an_unknown_account_refusal_is_private_for_the_desk_and_the_owner() {
    use crate::credentials::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    let room: Arc<dyn RoomHandle> = desk.clone();
    for (persona_id, backend_id, agent, phase) in [
        ("ada", "claude-acp", "Claude Code", "acp_start"),
        ("hal", "hotline", "Hotline Agent", "start"),
    ] {
        let mut teammate = crate::session::tests::persona(persona_id);
        teammate.backend_id = backend_id.to_string();
        teammate.account_id = Some("33333333-3333-4333-8333-333333333333".to_string());
        teammate.cwd = root.path().join(persona_id).to_string_lossy().into_owned();
        crate::session::tests::enrol(&desk.log, &teammate);
        for seat in [Seat::Desk, Seat::Owner] {
            let answer = remote_control_answer(
                seat,
                &room,
                &desk.log,
                &desk.parts(),
                json!({"id":1,"cmd":"session.start","params":{"personaId":persona_id}}),
            )
            .await;
            let error = answer["error"].as_str().unwrap_or_default();
            let private = format!(
                "Pick a {agent} account for Ada: the one it was set to is not on this desk."
            );
            let phase = format!(r#""phase":"{phase}""#);
            for part in [
                private.as_str(),
                r#""kind":"account_unavailable""#,
                r#""code":"unknown_account""#,
                phase.as_str(),
            ] {
                assert!(error.contains(part), "{answer}");
            }
        }
    }
    for command in ["session.start", "session.prompt"] {
        let denied = remote_control_answer(
            Seat::Phone,
            &room,
            &desk.log,
            &desk.parts(),
            json!({"id":2,"cmd":command,"params":{"personaId":"ada","text":"hi"}}),
        )
        .await;
        assert_eq!(denied["code"], FORBIDDEN, "{denied}");
    }
}

#[tokio::test]
async fn voice_is_owner_only_through_the_real_handler() {
    use crate::credentials::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_voice_services(
            root.path(),
            Arc::new(MemoryStore::default()),
            Some(crate::voice::tests::services()),
        )
        .unwrap(),
    );
    let room: Arc<dyn RoomHandle> = desk.clone();
    let call = uuid::Uuid::new_v4().to_string();
    for (command, params) in [
        ("voice.status", json!({})),
        ("voice.status", json!({"inputMode":"text"})),
        ("voice.call_start", json!({"callId":call})),
        (
            "voice.call_start",
            json!({"callId":call,"personaId":"ada","streamAudio":true}),
        ),
        (
            "voice.call_start",
            json!({"callId":call,"inputMode":"text"}),
        ),
        (
            "voice.text",
            json!({"callId":call,"seq":1,"text":"Check the PR."}),
        ),
        (
            "voice.audio",
            json!({"callId":call,"seq":1,"index":0,"data":"AAA=","final":true}),
        ),
        (
            "voice.utterance",
            json!({"callId":call,"seq":1,"mimeType":"audio/wav","data":"","durationMs":100}),
        ),
        ("voice.interrupt", json!({"callId":call})),
        ("voice.input_state", json!({"callId":call,"speaking":true})),
        ("voice.hold", json!({"callId":call,"hold":true})),
        ("voice.call_end", json!({"callId":call})),
    ] {
        let denied = remote_control_answer(
            Seat::Phone,
            &room,
            &desk.log,
            &desk.parts(),
            json!({"id":1,"cmd":command,"params":params}),
        )
        .await;
        assert_eq!(denied["code"], FORBIDDEN, "{denied}");
    }
    let denied = remote_control_answer(
        Seat::Phone,
        &room,
        &desk.log,
        &desk.parts(),
        json!({"id":2,"sub":{"call":call}}),
    )
    .await;
    assert_eq!(denied["code"], FORBIDDEN);
    let hidden = remote_control_answer(
        Seat::Phone,
        &room,
        &desk.log,
        &desk.parts(),
        json!({"id":20,"sub":{"tape":"voice-dispatcher"}}),
    )
    .await;
    assert_eq!(hidden["code"], FORBIDDEN);
    for seat in [Seat::Desk, Seat::Owner] {
        let allowed = remote_control_answer(
            seat,
            &room,
            &desk.log,
            &desk.parts(),
            json!({"id":3,"cmd":"voice.call_start","params":{"callId":call}}),
        )
        .await;
        assert_eq!(allowed["ok"], true, "{allowed}");
    }
    for seat in [Seat::Desk, Seat::Owner] {
        let text_call = uuid::Uuid::new_v4().to_string();
        let started = remote_control_answer(seat, &room, &desk.log, &desk.parts(),
            json!({"id":4,"cmd":"voice.call_start","params":{"callId":text_call,"inputMode":"text"}})).await;
        assert_eq!(started["ok"], true, "{started}");
        assert_eq!(started["result"]["inputMode"], "text");
        assert_eq!(started["result"]["input"], json!(["text/plain"]));
        let committed = remote_control_answer(seat, &room, &desk.log, &desk.parts(),
            json!({"id":5,"cmd":"voice.text","params":{"callId":text_call,"seq":1,"text":"Check the PR."}})).await;
        assert_eq!(committed["ok"], true, "{committed}");
        let replay = remote_control_answer(seat, &room, &desk.log, &desk.parts(),
            json!({"id":6,"cmd":"voice.text","params":{"callId":text_call,"seq":1,"text":"Check the PR."}})).await;
        assert_eq!(replay["ok"], false, "{replay}");
    }
    assert!(
        Seat::Owner
            .capabilities_for(&desk.parts())
            .contains(&"voiceDirectCalls")
    );
    // A phone is no seat for a front's sound either.
    for command in [
        Command::VoiceFrontAudio {
            call_id: "c".into(),
            seq: 0,
            data: String::new(),
        },
        Command::VoiceFrontBarge {
            call_id: "c".into(),
            played_ms: 0,
        },
    ] {
        assert!(Seat::Owner.permits(&command));
        assert!(!Seat::Phone.permits(&command));
    }
    assert!(
        !Seat::Phone
            .capabilities_for(&desk.parts())
            .contains(&"voiceDirectCalls")
    );
    assert!(
        Seat::Owner
            .capabilities_for(&desk.parts())
            .contains(&"voiceTextInput")
    );
    assert!(
        !Seat::Phone
            .capabilities_for(&desk.parts())
            .contains(&"voiceTextInput")
    );
}

#[tokio::test]
async fn the_desks_speech_models_are_the_owners_through_the_real_handler() {
    use crate::credentials::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    let room: Arc<dyn RoomHandle> = desk.clone();
    let model = "parakeet-tdt-110m-en";
    // A companion can neither see, fetch, remove nor hear with them. None of
    // these is answered, so nothing here reaches the network.
    for (command, params) in [
        ("voice.models", json!({})),
        ("voice.model_install", json!({"modelId":model})),
        ("voice.model_cancel", json!({"modelId":model})),
        ("voice.model_remove", json!({"modelId":model})),
        (
            "voice.transcribe",
            json!({"mimeType":"audio/wav","data":"UklGRg=="}),
        ),
    ] {
        let denied = remote_control_answer(
            Seat::Phone,
            &room,
            &desk.log,
            &desk.parts(),
            json!({"id":1,"cmd":command,"params":params}),
        )
        .await;
        assert_eq!(denied["code"], FORBIDDEN, "{command}: {denied}");
    }
    for seat in [Seat::Desk, Seat::Owner] {
        let listed = remote_control_answer(
            seat,
            &room,
            &desk.log,
            &desk.parts(),
            json!({"id":2,"cmd":"voice.models","params":{}}),
        )
        .await;
        assert_eq!(listed["ok"], true, "{listed}");
        // Nothing is installed until the owner asks.
        let states: Vec<&str> = listed["result"]
            .as_array()
            .unwrap()
            .iter()
            .map(|model| model["state"].as_str().unwrap())
            .collect();
        assert!(states.len() > 1 && states.iter().all(|state| *state == "available"));
        let unknown = remote_control_answer(
            seat,
            &room,
            &desk.log,
            &desk.parts(),
            json!({"id":3,"cmd":"voice.model_install","params":{"modelId":"../../vault"}}),
        )
        .await;
        assert_eq!(unknown["ok"], false, "{unknown}");
        let unheard = remote_control_answer(
            seat,
            &room,
            &desk.log,
            &desk.parts(),
            json!({"id":4,"cmd":"voice.transcribe","params":{"mimeType":"audio/wav","data":"UklGRg=="}}),
        )
        .await;
        assert_eq!(unheard["ok"], false, "{unheard}");
        assert!(
            unheard["error"]
                .as_str()
                .is_some_and(|error| error.contains("Download a speech model")),
            "{unheard}"
        );
    }
}

#[tokio::test]
async fn direct_voice_readiness_does_not_require_a_dispatcher_but_requires_speech_and_budget() {
    use crate::credentials::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    let room: Arc<dyn RoomHandle> = desk.clone();
    desk.credential_create("openai", "Speech fixture", "unused-test-key")
        .unwrap();
    desk.log
        .append(
            &StreamId::Room,
            &crate::room::room_event(
                "setting",
                json!({
                    "id":"voice", "value":{"front":{"provider":"legacy"},"dispatcher":{"provider":"not-connected"}}
                }),
            ),
        )
        .unwrap();
    for params in [json!({}), json!({"inputMode":"text"})] {
        let answer = remote_control_answer(
            Seat::Owner,
            &room,
            &desk.log,
            &desk.parts(),
            json!({"id":1,"cmd":"voice.status","params":params}),
        )
        .await;
        assert_eq!(answer["ok"], true);
        assert_eq!(answer["result"]["available"], false);
        assert_eq!(answer["result"]["directAvailable"], true);
        if params.get("inputMode").is_some() {
            assert!(answer["result"].get("stt").is_none());
        } else {
            assert_eq!(answer["result"]["stt"]["providerId"], "openai");
        }
        assert_eq!(answer["result"]["tts"]["providerId"], "openai");
        assert!(answer["result"].get("dispatcher").is_none());
    }
    for seat in [Seat::Owner, Seat::Desk] {
        let capabilities = seat.capabilities_for(&desk.parts());
        assert!(capabilities.contains(&"voice"));
        assert!(capabilities.contains(&"voiceDirectCalls"));
        assert!(capabilities.contains(&"voiceTextInput"));
    }
    assert!(
        !Seat::Phone
            .capabilities_for(&desk.parts())
            .contains(&"voiceDirectCalls")
    );
    desk.log
        .append(
            &StreamId::Room,
            &crate::room::room_event(
                "setting",
                json!({
                    "id":"spending", "value":{"dayUsd":0,"monthUsd":0}
                }),
            ),
        )
        .unwrap();
    assert!(!desk.parts().voice.unwrap().status().direct_available);
    assert!(
        !desk
            .parts()
            .voice
            .unwrap()
            .status_for(crate::contract::VoiceInputMode::Text)
            .direct_available
    );
    assert!(
        !Seat::Owner
            .capabilities_for(&desk.parts())
            .contains(&"voice")
    );
    desk.log
        .append(
            &StreamId::Room,
            &crate::room::room_event(
                "setting",
                json!({
                    "id":"spending", "value":{"dayUsd":2,"monthUsd":20}
                }),
            ),
        )
        .unwrap();
    desk.log
        .append(
            &StreamId::Room,
            &crate::room::room_event(
                "setting",
                json!({
                    "id":"voice", "value":{
                        "front":{"provider":"legacy"},
                        "stt":{"provider":"not-connected"},
                        "dispatcher":{"provider":"not-connected"}
                    }
                }),
            ),
        )
        .unwrap();
    assert!(!desk.parts().voice.unwrap().status().direct_available);
    let text = remote_control_answer(
        Seat::Owner,
        &room,
        &desk.log,
        &desk.parts(),
        json!({"id":2,"cmd":"voice.status","params":{"inputMode":"text"}}),
    )
    .await;
    assert_eq!(text["ok"], true);
    assert_eq!(text["result"]["available"], false);
    assert_eq!(text["result"]["directAvailable"], true);
    assert!(text["result"].get("stt").is_none());
    for seat in [Seat::Owner, Seat::Desk] {
        let capabilities = seat.capabilities_for(&desk.parts());
        assert!(capabilities.contains(&"voice"));
        assert!(capabilities.contains(&"voiceDirectCalls"));
        assert!(capabilities.contains(&"voiceTextInput"));
    }
    desk.log
        .append(
            &StreamId::Room,
            &crate::room::room_event(
                "setting",
                json!({"id":"voice", "value":{"tts":{"provider":"not-connected"}}}),
            ),
        )
        .unwrap();
    for mode in [
        crate::contract::VoiceInputMode::Audio,
        crate::contract::VoiceInputMode::Text,
    ] {
        assert!(
            !desk
                .parts()
                .voice
                .unwrap()
                .status_for(mode)
                .direct_available
        );
    }
    assert!(
        !Seat::Owner
            .capabilities_for(&desk.parts())
            .contains(&"voiceDirectCalls")
    );
}

#[tokio::test]
async fn cookie_push_is_operator_only_through_the_real_handler() {
    let quiet: Arc<dyn RoomHandle> = Arc::new(Quiet::new());
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let request = json!({"id":1,"cmd":"computer.cookies.push","params":{"personaId":"ada","transfer":{
        "sourceId":"laptop", "browserId":"firefox", "profileId":"default", "domains":["example.com"],
        "cookies":[{"domain":".example.com","name":"session","value":"private-cookie-value","path":"/"}]
    }}});
    for seat in [Seat::Desk, Seat::Owner, Seat::Phone] {
        let answer =
            remote_control_answer(seat, &quiet, &log, &Parts::default(), request.clone()).await;
        assert_eq!(answer["ok"], seat != Seat::Phone, "{answer}");
        if seat == Seat::Phone {
            assert_eq!(answer["code"], FORBIDDEN);
        }
        assert!(!answer.to_string().contains("private-cookie-value"));
    }
    let mut invalid = request;
    invalid["params"]["transfer"]["domains"] = json!(["other.com"]);
    assert_eq!(
        remote_control_answer(Seat::Owner, &quiet, &log, &Parts::default(), invalid).await["ok"],
        false
    );
    assert_eq!(log.load(&StreamId::Room), [] as [Value; 0]);
}

#[tokio::test]
async fn agent_accounts_are_operator_only_through_the_real_handler() {
    let quiet = Arc::new(Quiet::new());
    let room: Arc<dyn RoomHandle> = quiet.clone();
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    for seat in [Seat::Desk, Seat::Owner, Seat::Phone] {
        for frame in super::accounts::account_frames(&format!("{seat:?}")) {
            let answer =
                remote_control_answer(seat, &room, &log, &Parts::default(), frame.clone()).await;
            assert_eq!(
                answer["code"] == FORBIDDEN,
                seat == Seat::Phone,
                "{frame} {answer}"
            );
        }
    }
    let names: Vec<String> = room::agent_accounts(&log)
        .into_iter()
        .map(|account| account.name)
        .collect();
    assert_eq!(names, ["Desk", "Owner"]);
    let starts: Vec<bool> = quiet
        .account_sign_ins
        .lock()
        .unwrap()
        .iter()
        .map(|(_, seat_is_desk)| *seat_is_desk)
        .collect();
    assert_eq!(starts, [true, false]);
}

#[tokio::test]
async fn removing_an_account_in_use_names_its_teammates_and_keeps_its_folder() {
    let room: Arc<dyn RoomHandle> = Arc::new(Quiet::new());
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let desk = |frame: Value| {
        let (room, log) = (room.clone(), log.clone());
        async move { remote_control_answer(Seat::Desk, &room, &log, &Parts::default(), frame).await }
    };
    let created = desk(json!({"id":1,"cmd":"accounts.create","params":{
        "backendId":"claude-acp","name":"Account 1","startWith":"empty"}}))
    .await;
    let id = created["result"]["account"]["account"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut teammates = Vec::new();
    for name in ["Ada", "Bob"] {
        let made = desk(json!({"id":2,"cmd":"persona.create","params":{"draft":{
            "name":name,"backendId":"claude-acp","accountId":id}}}))
        .await;
        assert_eq!(made["result"]["accountId"], json!(id), "{made}");
        teammates.push(made["result"]["id"].as_str().unwrap().to_string());
    }
    let remove = json!({"id":3,"cmd":"accounts.remove","params":{"accountId":id}});
    let refused = desk(remove.clone()).await;
    assert_eq!(
        refused["error"], "Account 1 can't be removed while Ada and Bob use it.",
        "{refused}"
    );
    for teammate in &teammates {
        let deleted = desk(json!({"id":4,"cmd":"persona.delete","params":{"id":teammate}})).await;
        assert_eq!(deleted["ok"], true, "{deleted}");
    }
    let removed = desk(remove).await;
    assert_eq!(removed, json!({"id":3,"ok":true}));
    assert!(room::agent_accounts(&log).is_empty());
    assert!(paths::agent_account_dir(root.path(), &id).unwrap().is_dir());
}

#[tokio::test]
async fn persona_account_reaches_the_room_through_the_real_handler() {
    use crate::credentials::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    let room: Arc<dyn RoomHandle> = desk.clone();
    let ask = |frame: Value| {
        let (room, desk) = (room.clone(), desk.clone());
        async move { remote_control_answer(Seat::Desk, &room, &desk.log, &desk.parts(), frame).await }
    };
    let created = ask(json!({"id":1,"cmd":"accounts.create","params":{
        "backendId":"claude-acp","name":"Account 1","startWith":"empty"}}))
    .await;
    let id = created["result"]["account"]["account"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let made = ask(json!({"id":2,"cmd":"persona.create","params":{"draft":{
        "name":"Ada","backendId":"claude-acp"}}}))
    .await;
    let ada = made["result"]["id"].as_str().unwrap().to_string();
    let chosen = || {
        room::roster(&desk.log)
            .into_iter()
            .find(|persona| persona.id == ada)
            .and_then(|persona| persona.account_id)
    };
    let set = ask(json!({"id":3,"cmd":"persona.account","params":{
        "personaId":ada,"accountId":id}}))
    .await;
    assert_eq!(
        (&set["ok"], &set["result"]["wanted"]),
        (&json!(true), &json!(id)),
        "{set}"
    );
    assert_eq!(chosen(), Some(id));
    let cleared = ask(json!({"id":4,"cmd":"persona.account","params":{
        "personaId":ada,"accountId":null}}))
    .await;
    assert_eq!(cleared["ok"], true, "{cleared}");
    assert_eq!(chosen(), None);
}

#[tokio::test]
async fn images_status_is_operator_only_and_mock_rooms_are_unavailable() {
    let handle: Arc<dyn RoomHandle> = Arc::new(Quiet::new());
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let request = json!({"id": 1, "cmd": "images.status", "params": {}});
    for seat in [Seat::Desk, Seat::Owner, Seat::Phone] {
        let answer =
            remote_control_answer(seat, &handle, &log, &Parts::default(), request.clone()).await;
        if seat == Seat::Phone {
            assert_eq!(answer["ok"], false, "{answer}");
            assert_eq!(answer["code"], FORBIDDEN);
            assert!(answer.get("result").is_none());
        } else {
            assert_eq!(answer["ok"], true, "{answer}");
            assert_eq!(
                answer["result"],
                json!({
                    "available": false, "unavailable": "This room cannot make images."
                })
            );
        }
    }
    assert_eq!(log.load(&StreamId::Room), [] as [Value; 0]);
}

#[tokio::test]
async fn a_call_subscription_cannot_resume_another_devices_transport() {
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_voice_services(
            root.path(),
            Arc::new(crate::credentials::MemoryStore::default()),
            Some(crate::voice::tests::services()),
        )
        .unwrap(),
    );
    let room: Arc<dyn RoomHandle> = desk.clone();
    let parts = desk.parts();
    let calls = parts.voice.as_ref().unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    calls.start(&id, room.clone()).unwrap();
    let socket = tokio_util::sync::CancellationToken::new();
    calls
        .bind_connection(
            id.clone(),
            "another-device",
            socket.clone(),
            tokio_util::sync::CancellationToken::new(),
        )
        .unwrap();
    socket.cancel();
    let refused = remote_control_answer(
        Seat::Owner,
        &room,
        &desk.log,
        &parts,
        json!({"id":1,"sub":{"call":id}}),
    )
    .await;
    assert_eq!(refused["ok"], false);
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("device that started")
    );
    calls.end(&id).unwrap();

    let id = uuid::Uuid::new_v4().to_string();
    calls.start(&id, room.clone()).unwrap();
    let socket = tokio_util::sync::CancellationToken::new();
    calls
        .bind_connection(
            id.clone(),
            "desk",
            socket.clone(),
            tokio_util::sync::CancellationToken::new(),
        )
        .unwrap();
    socket.cancel();
    let resumed = remote_control_answer(
        Seat::Owner,
        &room,
        &desk.log,
        &parts,
        json!({"id":2,"sub":{"call":id}}),
    )
    .await;
    assert_eq!(resumed["ok"], true);
    calls.end(&id).unwrap();
}
