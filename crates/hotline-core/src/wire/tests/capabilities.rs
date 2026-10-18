use super::*;

#[tokio::test]
async fn capabilities_options_are_for_the_desk_and_owner_and_never_a_companion() {
    use crate::credentials::tests::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    let handle: Arc<dyn RoomHandle> = desk.clone();
    let request = json!({"id": 1, "cmd": "capabilities.options", "params": {}});
    for seat in [Seat::Desk, Seat::Owner, Seat::Phone] {
        let answer = remote_control_answer(seat, &handle, &desk.log, request.clone()).await;
        if seat == Seat::Phone {
            assert_eq!(answer["code"], FORBIDDEN, "{answer}");
            assert!(answer.get("result").is_none());
        } else {
            assert_eq!(answer["ok"], true, "{answer}");
        }
    }
    let quiet: Arc<dyn RoomHandle> = Arc::new(Quiet::new());
    let answer = remote_control_answer(Seat::Owner, &quiet, &desk.log, request).await;
    assert_eq!(answer["ok"], false, "{answer}");
}

#[tokio::test]
async fn capabilities_options_list_only_connected_providers_and_what_automatic_picks() {
    use crate::credentials::tests::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    let handle: Arc<dyn RoomHandle> = desk.clone();
    let request = json!({"id": 1, "cmd": "capabilities.options", "params": {}});
    let empty = remote_control_answer(Seat::Owner, &handle, &desk.log, request.clone()).await;
    assert_eq!(empty["ok"], true, "{empty}");
    let result = &empty["result"];
    for (job, says) in [
        ("images", "Connect"),
        ("stt", "Download a speech model for the desk, or connect"),
        ("tts", "Connect"),
    ] {
        assert_eq!(result[job]["options"], json!([]), "{job}");
        assert!(
            result[job]["unavailable"].as_str().unwrap().contains(says),
            "{job}"
        );
        assert!(result[job].get("automatic").is_none());
        assert!(result[job].get("selected").is_none());
    }
    assert_eq!(
        result["spending"],
        json!({"budgets": [
            {
                "kind": "chat", "dayUsd": null, "monthUsd": null,
                "spentDayUsd": 0.0, "spentMonthUsd": 0.0,
                "lines": [
                    {"kind": "teammates", "dayUsd": 0.0, "monthUsd": 0.0},
                    {"kind": "callAssistant", "dayUsd": 0.0, "monthUsd": 0.0},
                ]
            },
            {
                "kind": "voice", "dayUsd": null, "monthUsd": null,
                "spentDayUsd": 0.0, "spentMonthUsd": 0.0,
                "lines": [
                    {"kind": "transcription", "dayUsd": 0.0, "monthUsd": 0.0},
                    {"kind": "speech", "dayUsd": 0.0, "monthUsd": 0.0},
                ]
            },
            {
                "kind": "images", "dayUsd": null, "monthUsd": null,
                "spentDayUsd": 0.0, "spentMonthUsd": 0.0, "lines": []
            },
        ]})
    );

    let secret = "capabilities-private-credential";
    desk.credential_create("anthropic", "Chat only", secret)
        .unwrap();
    desk.credential_create("groq", "Speech", secret).unwrap();
    desk.credential_create("openai", "Everything", secret)
        .unwrap();
    let answer = remote_control_answer(Seat::Owner, &handle, &desk.log, request.clone()).await;
    assert!(!answer.to_string().contains(secret));
    let result = &answer["result"];
    let names = |job: &str| -> Vec<String> {
        result[job]["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|provider| provider["providerName"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(names("images"), ["OpenAI"]);
    assert_eq!(names("stt"), ["Groq", "OpenAI"]);
    assert_eq!(names("tts"), ["Groq", "OpenAI"]);
    assert!(names("dispatcher").contains(&"OpenAI".to_string()));
    assert!(
        result["images"]["options"][0]["models"]
            .as_array()
            .unwrap()
            .iter()
            .all(|model| model["id"].as_str().unwrap().contains("image"))
    );
    assert_eq!(
        result["images"]["automatic"],
        json!({"providerId": "openai", "providerName": "OpenAI", "modelId": "gpt-image-2.5-flare"})
    );
    assert_eq!(
        result["stt"]["automatic"]["modelId"],
        "whisper-large-v3-turbo"
    );
    assert_eq!(result["tts"]["automatic"]["providerId"], "groq");
    assert_eq!(result["tts"]["automatic"]["voice"], "hannah");
    assert_eq!(
        result["tts"]["options"][1]["models"][0]["voices"][0],
        "marin"
    );

    // A choice is reported as made, and automatic still says what it would be.
    let selected = remote_control_answer(
        Seat::Owner,
        &handle,
        &desk.log,
        json!({"id": 2, "cmd": "settings.update", "params": {"patch": {
            "images": {"provider": "openai", "model": "gpt-image-1-mini"},
            "voice": {"tts": {"provider": "openai", "voice": "cedar"}},
            "spending": {"chat": {"dayUsd": 3.0}, "voice": {"dayUsd": 0.5, "monthUsd": 5.0}}
        }}}),
    )
    .await;
    assert_eq!(selected["ok"], true, "{selected}");
    let answer = remote_control_answer(Seat::Desk, &handle, &desk.log, request).await;
    let result = &answer["result"];
    assert_eq!(result["images"]["selected"]["modelId"], "gpt-image-1-mini");
    assert_eq!(result["tts"]["selected"]["providerName"], "OpenAI");
    assert_eq!(result["tts"]["selected"]["voice"], "cedar");
    assert_eq!(result["tts"]["automatic"]["providerId"], "groq");
    assert!(result["stt"].get("selected").is_none());
    let budgets = &result["spending"]["budgets"];
    assert_eq!(budgets[0]["kind"], "chat");
    assert_eq!(budgets[0]["dayUsd"], 3.0);
    assert_eq!(budgets[0]["monthUsd"], Value::Null);
    assert_eq!(budgets[1]["kind"], "voice");
    assert_eq!(budgets[1]["dayUsd"], 0.5);
    assert_eq!(budgets[1]["monthUsd"], 5.0);
    assert_eq!(budgets[2]["dayUsd"], Value::Null);
}

#[tokio::test]
async fn capabilities_options_report_each_budget_from_its_own_tally() {
    use crate::credentials::tests::MemoryStore;
    use crate::spending::SpendLedger;
    let root = tempfile::tempdir().unwrap();
    SpendLedger::new(root.path().to_path_buf())
        .reserve(&crate::contract::BudgetLimits::default(), 0.25)
        .unwrap()
        .charge(0.125)
        .unwrap();
    std::fs::write(
        root.path().join("voice-ledger.json"),
        json!({
            "day": chrono::Local::now().format("%Y-%m-%d").to_string(),
            "month": chrono::Local::now().format("%Y-%m").to_string(),
            "daySpend": {"stt": 0.0, "tts": 0.25, "dispatcher": 0.0625},
            "monthSpend": {"stt": 0.125, "tts": 0.5, "dispatcher": 0.0625}
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        root.path().join("chat-ledger.json"),
        json!({
            "day": chrono::Local::now().format("%Y-%m-%d").to_string(),
            "month": chrono::Local::now().format("%Y-%m").to_string(),
            "daySpend": {"stt": 0.0, "tts": 0.0, "dispatcher": 0.0, "teammates": 1.0},
            "monthSpend": {"stt": 0.0, "tts": 0.0, "dispatcher": 0.0, "teammates": 3.0}
        })
        .to_string(),
    )
    .unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    let handle: Arc<dyn RoomHandle> = desk.clone();
    let answer = remote_control_answer(
        Seat::Owner,
        &handle,
        &desk.log,
        json!({"id": 1, "cmd": "capabilities.options", "params": {}}),
    )
    .await;
    let budgets = &answer["result"]["spending"]["budgets"];
    assert_eq!(budgets[0]["spentDayUsd"], 1.0625, "{answer}");
    assert_eq!(budgets[0]["spentMonthUsd"], 3.0625);
    assert_eq!(
        budgets[0]["lines"][0],
        json!({"kind": "teammates", "dayUsd": 1.0, "monthUsd": 3.0})
    );
    assert_eq!(budgets[0]["lines"][1]["kind"], "callAssistant");
    assert_eq!(budgets[0]["lines"][1]["monthUsd"], 0.0625);
    assert_eq!(budgets[1]["spentDayUsd"], 0.25);
    assert_eq!(budgets[1]["spentMonthUsd"], 0.625);
    assert_eq!(budgets[1]["lines"][0]["monthUsd"], 0.125);
    assert_eq!(budgets[2]["spentDayUsd"], 0.125);
    assert_eq!(budgets[2]["spentMonthUsd"], 0.125);
    assert!(answer["result"]["spending"].get("unavailable").is_none());
}

#[tokio::test]
async fn images_status_resolves_the_desks_vault_without_exposing_credentials() {
    use crate::credentials::tests::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    let handle: Arc<dyn RoomHandle> = desk.clone();
    let request = json!({"id": 1, "cmd": "images.status", "params": {}});
    let missing = remote_control_answer(Seat::Desk, &handle, &desk.log, request.clone()).await;
    assert_eq!(missing["ok"], true, "{missing}");
    assert_eq!(missing["result"]["available"], false);
    assert!(
        missing["result"]["unavailable"]
            .as_str()
            .unwrap()
            .contains("Connect")
    );
    assert!(missing["result"].get("provider").is_none());
    assert!(missing["result"].get("model").is_none());

    let secret = "image-status-private-credential";
    desk.credential_create("anthropic", "No image provider", secret)
        .unwrap();
    let unsupported = remote_control_answer(Seat::Owner, &handle, &desk.log, request.clone()).await;
    assert_eq!(unsupported["result"]["available"], false, "{unsupported}");
    let openai = desk
        .credential_create("openai", "Image provider", secret)
        .unwrap();
    desk.credential_create("google", "Second image provider", secret)
        .unwrap();
    let before = desk.log.load(&StreamId::Room);
    for seat in [Seat::Desk, Seat::Owner] {
        let available = remote_control_answer(seat, &handle, &desk.log, request.clone()).await;
        assert_eq!(
            available["result"],
            json!({
                "available": true, "provider": "openai", "model": "gpt-image-2.5-flare",
                "spending": {"dayUsd": 0.0, "monthUsd": 0.0}
            })
        );
        assert!(!available.to_string().contains(secret));
    }
    assert_eq!(desk.log.load(&StreamId::Room), before);

    let selected = remote_control_answer(
        Seat::Owner,
        &handle,
        &desk.log,
        json!({
            "id": 2, "cmd": "settings.update", "params": {"patch": {
                "images": {"provider": "google", "model": "gemini-3.1-flash-lite-image"}
            }}
        }),
    )
    .await;
    assert_eq!(selected["ok"], true, "{selected}");
    let available = remote_control_answer(Seat::Owner, &handle, &desk.log, request.clone()).await;
    assert_eq!(
        available["result"],
        json!({
            "available": true, "provider": "google", "model": "gemini-3.1-flash-lite-image",
            "spending": {"dayUsd": 0.0, "monthUsd": 0.0}
        })
    );

    let selected = remote_control_answer(Seat::Desk, &handle, &desk.log, json!({
        "id": 3, "cmd": "settings.update", "params": {"patch": {"images": {"provider": "openai"}}}
    })).await;
    assert_eq!(selected["ok"], true, "{selected}");
    desk.credential_revoke(&openai.id).unwrap();
    let before = desk.log.load(&StreamId::Room);
    let revoked = remote_control_answer(Seat::Owner, &handle, &desk.log, request.clone()).await;
    assert_eq!(revoked["result"]["available"], false, "{revoked}");
    assert!(
        revoked["result"]["unavailable"]
            .as_str()
            .unwrap()
            .contains("isn't connected")
    );
    assert!(revoked["result"].get("provider").is_none());
    assert!(revoked["result"].get("model").is_none());
    assert_eq!(desk.log.load(&StreamId::Room), before);

    let reset = remote_control_answer(
        Seat::Desk,
        &handle,
        &desk.log,
        json!({
            "id": 4, "cmd": "settings.update", "params": {"patch": {"images": null}}
        }),
    )
    .await;
    assert_eq!(reset["ok"], true, "{reset}");
    let available = remote_control_answer(Seat::Owner, &handle, &desk.log, request).await;
    assert_eq!(available["result"]["available"], true, "{available}");
    assert_eq!(available["result"]["provider"], "google");
    let denied = remote_control_answer(
        Seat::Phone,
        &handle,
        &desk.log,
        json!({"id": 5, "cmd": "images.status", "params": {}}),
    )
    .await;
    assert_eq!(denied["code"], FORBIDDEN);
    assert!(denied.get("result").is_none());
    for answer in [missing, unsupported, selected, available, revoked, reset] {
        assert!(!answer.to_string().contains(secret), "{answer}");
    }
    assert!(
        !serde_json::to_string(&desk.log.load(&StreamId::Room))
            .unwrap()
            .contains(secret)
    );
}

#[tokio::test]
async fn chatgpt_images_status_is_automatic_without_checking_entitlement() {
    use crate::credentials::tests::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::default());
    let log = crate::log::Log::open(root.path());
    let vault = crate::vault::Vault::open_with_store(root.path(), log, store.clone()).unwrap();
    let (id, token_dir) = vault.begin_login("openai-codex").unwrap();
    vault.finish_login(&id, "openai-codex", "ChatGPT").unwrap();
    drop(vault);
    let desk = Arc::new(crate::desk::Desk::open_with_store(root.path(), store).unwrap());
    let handle: Arc<dyn RoomHandle> = desk.clone();
    let request = json!({"id": 1, "cmd": "images.status", "params": {}});
    let automatic = remote_control_answer(Seat::Desk, &handle, &desk.log, request.clone()).await;
    assert_eq!(automatic["result"]["available"], true);
    assert_eq!(automatic["result"]["provider"], "openai-codex");
    let selected = remote_control_answer(
        Seat::Owner,
        &handle,
        &desk.log,
        json!({
            "id": 2, "cmd": "settings.update", "params": {"patch": {
                "images": {"provider": "openai-codex"}
            }}
        }),
    )
    .await;
    assert_eq!(selected["ok"], true);
    let denied = remote_control_answer(Seat::Phone, &handle, &desk.log, request.clone()).await;
    assert_eq!(denied["code"], FORBIDDEN);
    for seat in [Seat::Owner, Seat::Desk] {
        let status = remote_control_answer(seat, &handle, &desk.log, request.clone()).await;
        assert_eq!(status["result"]["available"], true);
        assert_eq!(status["result"]["provider"], "openai-codex");
        assert_eq!(status["result"]["model"], "gpt-image-2");
        assert!(
            !status
                .to_string()
                .contains(&token_dir.to_string_lossy().to_string())
        );
    }
    assert_eq!(
        std::fs::read_to_string(token_dir.join("auth.json")).unwrap(),
        "{}"
    );
}

#[tokio::test]
async fn image_and_spending_settings_are_typed_validated_and_resettable() {
    let handle: Arc<dyn RoomHandle> = Arc::new(Quiet::new());
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let defaults = remote_control_answer(
        Seat::Desk,
        &handle,
        &log,
        json!({
            "id": 1, "cmd": "settings.update", "params": {"patch": {}}
        }),
    )
    .await;
    assert_eq!(defaults["result"]["images"], json!({}));
    assert_eq!(
        defaults["result"]["spending"],
        json!({"chat": {}, "voice": {}, "images": {}})
    );

    let updated = remote_control_answer(Seat::Owner, &handle, &log, json!({
        "id": 2, "cmd": "settings.update", "params": {"patch": {
            "images": {"provider": "openai", "model": "gpt-image-1-mini", "secret": "not-a-setting"},
            "spending": {"dayUsd": 0, "monthUsd": 0},
            "futureSetting": {"enabled": true}
        }}
    })).await;
    assert_eq!(updated["ok"], true, "{updated}");
    assert_eq!(
        updated["result"]["images"],
        json!({"provider": "openai", "model": "gpt-image-1-mini"})
    );
    // The shared limits of earlier versions are written as voice's and images'.
    assert_eq!(
        updated["result"]["spending"],
        json!({
            "chat": {},
            "voice": {"dayUsd": 0.0, "monthUsd": 0.0},
            "images": {"dayUsd": 0.0, "monthUsd": 0.0}
        })
    );
    assert_eq!(updated["result"]["futureSetting"], json!({"enabled": true}));
    assert!(
        !serde_json::to_string(&log.load(&StreamId::Room))
            .unwrap()
            .contains("not-a-setting")
    );

    let partial = remote_control_answer(
        Seat::Desk,
        &handle,
        &log,
        json!({
            "id": 3, "cmd": "settings.update", "params": {"patch": {
                "images": {"model": "gpt-image-1-mini"},
                "spending": {"chat": {"monthUsd": 7.5}, "voice": {"dayUsd": null}}
            }}
        }),
    )
    .await;
    assert_eq!(partial["ok"], true, "{partial}");
    assert_eq!(
        partial["result"]["images"],
        json!({"model": "gpt-image-1-mini"})
    );
    assert_eq!(
        partial["result"]["spending"],
        json!({"chat": {"monthUsd": 7.5}, "voice": {}, "images": {}})
    );

    let reset = remote_control_answer(Seat::Owner, &handle, &log, json!({
        "id": 4, "cmd": "settings.update", "params": {"patch": {"images": null, "spending": null}}
    })).await;
    assert_eq!(reset["ok"], true, "{reset}");
    assert_eq!(reset["result"]["images"], json!({}));
    assert_eq!(
        reset["result"]["spending"],
        json!({"chat": {}, "voice": {}, "images": {}})
    );
    assert_eq!(reset["result"]["futureSetting"], json!({"enabled": true}));
    let before = log.load(&StreamId::Room);
    let denied = remote_control_answer(
        Seat::Phone,
        &handle,
        &log,
        json!({
            "id": 5, "cmd": "settings.update", "params": {"patch": {"spending": {"dayUsd": 0}}}
        }),
    )
    .await;
    assert_eq!(denied["code"], FORBIDDEN);
    assert_eq!(log.load(&StreamId::Room), before);
}

#[tokio::test]
async fn images_status_reports_recorded_spending_without_creating_or_writing_a_ledger() {
    use crate::credentials::tests::MemoryStore;
    use crate::spending::SpendLedger;
    let root = tempfile::tempdir().unwrap();
    {
        let ledger = SpendLedger::new(root.path().to_path_buf());
        ledger
            .reserve(&crate::contract::BudgetLimits::default(), 0.25)
            .unwrap()
            .charge(0.125)
            .unwrap();
    }
    let ledger_path = root.path().join("spending.json");
    let before = std::fs::read(&ledger_path).unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    let handle: Arc<dyn RoomHandle> = desk.clone();
    let answer = remote_control_answer(
        Seat::Owner,
        &handle,
        &desk.log,
        json!({"id": 1, "cmd": "images.status", "params": {}}),
    )
    .await;
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(
        answer["result"]["spending"],
        json!({"dayUsd": 0.125, "monthUsd": 0.125})
    );
    assert!(answer["result"].get("spendingUnavailable").is_none());
    assert_eq!(std::fs::read(&ledger_path).unwrap(), before);

    let fresh_root = tempfile::tempdir().unwrap();
    let fresh_desk = Arc::new(
        crate::desk::Desk::open_with_store(fresh_root.path(), Arc::new(MemoryStore::default()))
            .unwrap(),
    );
    let fresh_handle: Arc<dyn RoomHandle> = fresh_desk.clone();
    let answer = remote_control_answer(
        Seat::Desk,
        &fresh_handle,
        &fresh_desk.log,
        json!({"id": 2, "cmd": "images.status", "params": {}}),
    )
    .await;
    assert_eq!(
        answer["result"]["spending"],
        json!({"dayUsd": 0.0, "monthUsd": 0.0})
    );
    assert!(!fresh_root.path().join("spending.json").exists());
}

#[tokio::test]
async fn images_status_preserves_the_shared_ledgers_failure_state_without_leaking_contents() {
    use crate::credentials::tests::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let ledger_path = root.path().join("spending.json");
    std::fs::write(&ledger_path, b"private-corrupt-ledger-content").unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    desk.credential_create("openai", "Image provider", "private-image-key")
        .unwrap();
    let handle: Arc<dyn RoomHandle> = desk.clone();
    for seat in [Seat::Desk, Seat::Owner] {
        let answer = remote_control_answer(
            seat,
            &handle,
            &desk.log,
            json!({"id": 1, "cmd": "images.status", "params": {}}),
        )
        .await;
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(answer["result"]["available"], true);
        assert!(answer["result"].get("spending").is_none());
        assert!(
            answer["result"]["spendingUnavailable"]
                .as_str()
                .unwrap()
                .contains("Spending is blocked")
        );
        assert!(
            !answer
                .to_string()
                .contains("private-corrupt-ledger-content")
        );
        assert!(!answer.to_string().contains("private-image-key"));
        if ledger_path.exists() {
            std::fs::remove_file(&ledger_path).unwrap();
        }
    }
    assert!(!ledger_path.exists());
}

#[tokio::test]
async fn images_status_refuses_malformed_saved_selections_and_marks_invalid_caps_unavailable() {
    use crate::credentials::tests::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    desk.credential_create("openai", "Image provider", "private-image-key")
        .unwrap();
    let handle: Arc<dyn RoomHandle> = desk.clone();
    for images in [json!({"provider": 17}), json!([]), json!({"model": " "})] {
        desk.log
            .append(
                &StreamId::Room,
                &json!({
                    "kind": "setting", "id": "images", "value": images
                }),
            )
            .unwrap();
        let answer = remote_control_answer(
            Seat::Desk,
            &handle,
            &desk.log,
            json!({"id": 1, "cmd": "images.status", "params": {}}),
        )
        .await;
        assert_eq!(answer["result"]["available"], false, "{answer}");
        assert!(answer["result"]["unavailable"].is_string());
        assert!(answer["result"].get("provider").is_none());
        assert_eq!(crate::room::settings(&desk.log)["images"], images);
    }
    desk.log
        .append(
            &StreamId::Room,
            &json!({
                "kind": "setting", "id": "images", "deleted": true
            }),
        )
        .unwrap();
    let spending = json!({"dayUsd": "disabled", "monthUsd": 0});
    desk.log
        .append(
            &StreamId::Room,
            &json!({
                "kind": "setting", "id": "spending", "value": spending
            }),
        )
        .unwrap();
    let answer = remote_control_answer(
        Seat::Owner,
        &handle,
        &desk.log,
        json!({"id": 2, "cmd": "images.status", "params": {}}),
    )
    .await;
    assert_eq!(answer["result"]["available"], true, "{answer}");
    assert!(answer["result"].get("spending").is_none());
    assert!(answer["result"]["spendingUnavailable"].is_string());
    assert_eq!(crate::room::settings(&desk.log)["spending"], spending);
    assert!(serde_json::from_value::<crate::contract::SpendingSettings>(spending).is_err());
}

#[tokio::test]
async fn invalid_image_or_spending_settings_do_not_write_any_patch_key() {
    let quiet = Arc::new(Quiet::new());
    let handle: Arc<dyn RoomHandle> = quiet.clone();
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    for (key, value) in [
        ("images", json!("not-an-object")),
        ("images", json!([])),
        ("images", json!({"provider": 17})),
        ("images", json!({"model": false})),
        ("images", json!({"provider": " "})),
        ("images", json!({"model": ""})),
        ("spending", json!([])),
        ("spending", json!({"dayUsd": -1})),
        ("spending", json!({"monthUsd": -1})),
        ("spending", json!({"dayUsd": "2"})),
        ("spending", json!({"voice": {"dayUsd": -1}})),
        ("spending", json!({"chat": {"monthUsd": "2"}})),
        ("spending", json!({"images": "off"})),
        ("spending", json!({"dayUsd": 1, "voice": {"dayUsd": 2}})),
    ] {
        let mut patch = serde_json::Map::new();
        patch.insert("theme".into(), json!("dark"));
        patch.insert("mcpServers".into(), json!([]));
        patch.insert(key.into(), value);
        let refused = remote_control_answer(
            Seat::Owner,
            &handle,
            &log,
            json!({
                "id": 1, "cmd": "settings.update", "params": {"patch": patch}
            }),
        )
        .await;
        assert_eq!(refused["ok"], false, "{refused}");
        assert!(refused["error"].is_string());
        assert!(log.load(&StreamId::Room).is_empty());
        assert!(quiet.invalidations.lock().unwrap().is_empty());
        assert!(quiet.reattached().is_empty());
    }
}

#[tokio::test]
async fn images_status_refuses_corrupt_saved_zero_caps_without_exposing_room_contents() {
    use crate::credentials::tests::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_store(root.path(), Arc::new(MemoryStore::default())).unwrap(),
    );
    desk.credential_create("openai", "Image provider", "private-image-key")
        .unwrap();
    let handle: Arc<dyn RoomHandle> = desk.clone();
    let disabled = remote_control_answer(
        Seat::Owner,
        &handle,
        &desk.log,
        json!({
            "id": 1, "cmd": "settings.update", "params": {"patch": {
                "spending": {"dayUsd": 0, "monthUsd": 0}
            }}
        }),
    )
    .await;
    assert_eq!(disabled["ok"], true, "{disabled}");
    std::fs::write(
        crate::paths::room_path(root.path()),
        "{\"kind\":\"setting\",\"id\":\"spending\",\"value\":{\"dayUsd\":0,\"monthUsd\":0},\"private\":\"corrupt-room-canary\"\n",
    ).unwrap();
    for seat in [Seat::Desk, Seat::Owner, Seat::Phone] {
        let answer = remote_control_answer(
            seat,
            &handle,
            &desk.log,
            json!({"id": 2, "cmd": "images.status", "params": {}}),
        )
        .await;
        if seat == Seat::Phone {
            assert_eq!(answer["code"], FORBIDDEN);
            assert!(answer.get("result").is_none());
        } else {
            assert_eq!(answer["ok"], true, "{answer}");
            assert_eq!(answer["result"]["available"], false);
            assert!(
                answer["result"]["unavailable"]
                    .as_str()
                    .unwrap()
                    .contains("could not be safely read")
            );
            assert_eq!(
                answer["result"]["spendingUnavailable"],
                answer["result"]["unavailable"]
            );
            for field in ["provider", "model", "spending"] {
                assert!(answer["result"].get(field).is_none());
            }
        }
        assert!(!answer.to_string().contains("corrupt-room-canary"));
        assert!(!answer.to_string().contains("private-image-key"));
    }
    assert!(!root.path().join("spending.json").exists());
}

#[tokio::test]
async fn capabilities_options_offer_chatgpt_images_as_the_automatic_subscription() {
    use crate::credentials::tests::MemoryStore;
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::default());
    let log = crate::log::Log::open(root.path());
    let vault = crate::vault::Vault::open_with_store(root.path(), log, store.clone()).unwrap();
    let (id, token_dir) = vault.begin_login("openai-codex").unwrap();
    vault.finish_login(&id, "openai-codex", "ChatGPT").unwrap();
    drop(vault);
    let desk = Arc::new(crate::desk::Desk::open_with_store(root.path(), store).unwrap());
    let handle: Arc<dyn RoomHandle> = desk.clone();
    let request = json!({"id": 1, "cmd": "capabilities.options", "params": {}});
    for seat in [Seat::Desk, Seat::Owner] {
        let offered = remote_control_answer(seat, &handle, &desk.log, request.clone()).await;
        assert_eq!(offered["ok"], true, "{offered}");
        assert_eq!(
            offered["result"]["images"]["options"],
            json!([{
                "providerId": "openai-codex",
                "providerName": "Codex (ChatGPT subscription)",
                "models": [{"id": "gpt-image-2"}]
            }])
        );
        assert_eq!(
            offered["result"]["images"]["automatic"]["providerId"],
            "openai-codex"
        );
        assert!(offered["result"]["images"].get("selected").is_none());
        assert!(
            !offered
                .to_string()
                .contains(&token_dir.to_string_lossy().to_string())
        );
    }
    desk.credential_create("openai", "API", "options-test-key")
        .unwrap();
    let selected = remote_control_answer(
        Seat::Owner,
        &handle,
        &desk.log,
        json!({"id": 2, "cmd": "settings.update", "params": {"patch": {
            "images": {"provider": "openai-codex", "model": "gpt-image-2"}
        }}}),
    )
    .await;
    assert_eq!(selected["ok"], true, "{selected}");
    let offered = remote_control_answer(Seat::Desk, &handle, &desk.log, request.clone()).await;
    assert_eq!(
        offered["result"]["images"]["selected"],
        json!({
            "providerId": "openai-codex",
            "providerName": "Codex (ChatGPT subscription)",
            "modelId": "gpt-image-2"
        })
    );
    // A subscription stays the automatic pick when a paid key joins it.
    assert_eq!(
        offered["result"]["images"]["automatic"]["providerId"],
        "openai-codex"
    );
    let denied = remote_control_answer(Seat::Phone, &handle, &desk.log, request).await;
    assert_eq!(denied["code"], FORBIDDEN);
    assert_eq!(
        std::fs::read_to_string(token_dir.join("auth.json")).unwrap(),
        "{}"
    );
}
