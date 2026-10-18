use super::*;

#[tokio::test]
async fn owner_commands_and_room_subscription_reach_the_real_handler() {
    use crate::credentials::tests::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    let room: Arc<dyn RoomHandle> = desk.clone();
    for seat in [Seat::Owner, Seat::Desk] {
        let answer = remote_control_answer(seat, &room, &desk.log,
            json!({"id":1,"cmd":"settings.update","params":{"patch":{"skillsHome":"/tmp/owner-skills"}}})).await;
        assert_eq!(answer["ok"], true, "{answer}");
        let answer =
            remote_control_answer(seat, &room, &desk.log, json!({"id":2,"sub":"room"})).await;
        assert_eq!(answer["ok"], true, "{answer}");
    }
    let answer = remote_control_answer(Seat::Phone, &room, &desk.log,
        json!({"id":3,"cmd":"settings.update","params":{"patch":{"skillsHome":"/tmp/companion-skills"}}})).await;
    assert_eq!(answer["code"], FORBIDDEN);
    assert_eq!(
        crate::room::settings(&desk.log)["skillsHome"],
        "/tmp/owner-skills"
    );
}

#[tokio::test]
async fn voice_is_owner_only_through_the_real_handler() {
    use crate::credentials::tests::MemoryStore;
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
        ("voice.hold", json!({"callId":call,"hold":true})),
        ("voice.call_end", json!({"callId":call})),
    ] {
        let denied = remote_control_answer(
            Seat::Phone,
            &room,
            &desk.log,
            json!({"id":1,"cmd":command,"params":params}),
        )
        .await;
        assert_eq!(denied["code"], FORBIDDEN, "{denied}");
    }
    let denied = remote_control_answer(
        Seat::Phone,
        &room,
        &desk.log,
        json!({"id":2,"sub":{"call":call}}),
    )
    .await;
    assert_eq!(denied["code"], FORBIDDEN);
    let hidden = remote_control_answer(
        Seat::Phone,
        &room,
        &desk.log,
        json!({"id":20,"sub":{"tape":crate::voice::TAPE_ID}}),
    )
    .await;
    assert_eq!(hidden["code"], FORBIDDEN);
    for seat in [Seat::Desk, Seat::Owner] {
        let allowed = remote_control_answer(
            seat,
            &room,
            &desk.log,
            json!({"id":3,"cmd":"voice.call_start","params":{"callId":call}}),
        )
        .await;
        assert_eq!(allowed["ok"], true, "{allowed}");
    }
    for seat in [Seat::Desk, Seat::Owner] {
        let text_call = uuid::Uuid::new_v4().to_string();
        let started = remote_control_answer(seat, &room, &desk.log,
            json!({"id":4,"cmd":"voice.call_start","params":{"callId":text_call,"inputMode":"text"}})).await;
        assert_eq!(started["ok"], true, "{started}");
        assert_eq!(started["result"]["inputMode"], "text");
        assert_eq!(started["result"]["input"], json!(["text/plain"]));
        let committed = remote_control_answer(seat, &room, &desk.log,
            json!({"id":5,"cmd":"voice.text","params":{"callId":text_call,"seq":1,"text":"Check the PR."}})).await;
        assert_eq!(committed["ok"], true, "{committed}");
        let replay = remote_control_answer(seat, &room, &desk.log,
            json!({"id":6,"cmd":"voice.text","params":{"callId":text_call,"seq":1,"text":"Check the PR."}})).await;
        assert_eq!(replay["ok"], false, "{replay}");
    }
    assert!(
        Seat::Owner
            .capabilities_for(room.as_ref())
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
            .capabilities_for(room.as_ref())
            .contains(&"voiceDirectCalls")
    );
    assert!(
        Seat::Owner
            .capabilities_for(room.as_ref())
            .contains(&"voiceTextInput")
    );
    assert!(
        !Seat::Phone
            .capabilities_for(room.as_ref())
            .contains(&"voiceTextInput")
    );
}

#[tokio::test]
async fn the_desks_speech_models_are_the_owners_through_the_real_handler() {
    use crate::credentials::tests::MemoryStore;
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
            json!({"id":3,"cmd":"voice.model_install","params":{"modelId":"../../vault"}}),
        )
        .await;
        assert_eq!(unknown["ok"], false, "{unknown}");
        let unheard = remote_control_answer(
            seat,
            &room,
            &desk.log,
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
    use crate::credentials::tests::MemoryStore;
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
                    "id":"voice", "value":{"dispatcher":{"provider":"not-connected"}}
                }),
            ),
        )
        .unwrap();
    for params in [json!({}), json!({"inputMode":"text"})] {
        let answer = remote_control_answer(
            Seat::Owner,
            &room,
            &desk.log,
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
        let capabilities = seat.capabilities_for(room.as_ref());
        assert!(capabilities.contains(&"voice"));
        assert!(capabilities.contains(&"voiceDirectCalls"));
        assert!(capabilities.contains(&"voiceTextInput"));
    }
    assert!(
        !Seat::Phone
            .capabilities_for(room.as_ref())
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
    assert!(!desk.voice().unwrap().status().direct_available);
    assert!(
        !desk
            .voice()
            .unwrap()
            .status_for(crate::contract::VoiceInputMode::Text)
            .direct_available
    );
    assert!(
        !Seat::Owner
            .capabilities_for(room.as_ref())
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
                        "stt":{"provider":"not-connected"},
                        "dispatcher":{"provider":"not-connected"}
                    }
                }),
            ),
        )
        .unwrap();
    assert!(!desk.voice().unwrap().status().direct_available);
    let text = remote_control_answer(
        Seat::Owner,
        &room,
        &desk.log,
        json!({"id":2,"cmd":"voice.status","params":{"inputMode":"text"}}),
    )
    .await;
    assert_eq!(text["ok"], true);
    assert_eq!(text["result"]["available"], false);
    assert_eq!(text["result"]["directAvailable"], true);
    assert!(text["result"].get("stt").is_none());
    for seat in [Seat::Owner, Seat::Desk] {
        let capabilities = seat.capabilities_for(room.as_ref());
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
        assert!(!desk.voice().unwrap().status_for(mode).direct_available);
    }
    assert!(
        !Seat::Owner
            .capabilities_for(room.as_ref())
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
        let answer = remote_control_answer(seat, &quiet, &log, request.clone()).await;
        assert_eq!(answer["ok"], seat != Seat::Phone, "{answer}");
        if seat == Seat::Phone {
            assert_eq!(answer["code"], FORBIDDEN);
        }
        assert!(!answer.to_string().contains("private-cookie-value"));
    }
    let mut invalid = request;
    invalid["params"]["transfer"]["domains"] = json!(["other.com"]);
    assert_eq!(
        remote_control_answer(Seat::Owner, &quiet, &log, invalid).await["ok"],
        false
    );
    assert!(log.load(&StreamId::Room).is_empty());
}

#[tokio::test]
async fn images_status_is_operator_only_and_mock_rooms_are_unavailable() {
    let handle: Arc<dyn RoomHandle> = Arc::new(Quiet::new());
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let request = json!({"id": 1, "cmd": "images.status", "params": {}});
    for seat in [Seat::Desk, Seat::Owner, Seat::Phone] {
        let answer = remote_control_answer(seat, &handle, &log, request.clone()).await;
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
    assert!(log.load(&StreamId::Room).is_empty());
}
