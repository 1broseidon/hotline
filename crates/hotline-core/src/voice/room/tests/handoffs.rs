use super::*;

#[tokio::test]
async fn voice_schedules_require_a_grant_and_cannot_escape_the_call_scope() {
    use crate::contract::Command;
    let (_root, desk, _calls) = desk(services());
    let handle: Arc<dyn RoomHandle> = desk.clone();
    let create: Command = serde_json::from_value(json!({"cmd":"persona.create","params":{"draft":{"name":"Mack","goal":"Test","cwd":desk.log.root().to_str().unwrap()}}})).unwrap();
    let persona = crate::wire::commands::run(create, &desk.log, &handle, &desk.parts())
        .await
        .unwrap();
    let id = persona["id"].as_str().unwrap();
    let context = Context::new(
        desk.log.clone(),
        handle.clone(),
        desk.parts().voice.unwrap(),
        CancellationToken::new(),
    );
    let heard = context.for_utterance();
    let mut params = json!({"personaId":id,"kind":"schedule","when":chrono::Utc::now().timestamp_millis()+60_000,"prompt":"Check the PR."});
    let command = |params: Value| {
        serde_json::from_value::<Command>(json!({"cmd":"schedule.create","params":params})).unwrap()
    };
    assert!(
        heard
            .execute(command(params.clone()))
            .await
            .unwrap_err()
            .contains("Background work")
    );
    let mut granted = persona;
    granted["kind"] = json!("persona");
    granted["backgroundWork"] = json!(true);
    desk.log
        .append(&crate::log::StreamId::Room, &granted)
        .unwrap();
    for field in [
        ("kind", json!("loop")),
        ("every", json!(60_000)),
        ("quiet", json!(false)),
        ("quiet", json!(true)),
    ] {
        let mut invalid = params.clone();
        invalid[field.0] = field.1;
        assert!(
            heard
                .execute(command(invalid))
                .await
                .unwrap_err()
                .contains("one-shot")
        );
    }
    assert!(
        context
            .execute(command(params.clone()))
            .await
            .unwrap_err()
            .contains("spoken request")
    );
    let job = heard.execute(command(params.clone())).await.unwrap();
    assert_eq!(job["operatorCreated"], false);
    let run = crate::contract::ScheduledRun {
        job_id: job["id"].as_str().unwrap().into(),
        kind: crate::contract::ScheduleKind::Schedule,
        name: "Voice task".into(),
        operator_created: job["operatorCreated"].as_bool().unwrap(),
        quiet: None,
    };
    assert!(crate::session::schedule::scheduled_run_allowed(
        &desk.log,
        params["personaId"].as_str().unwrap(),
        &run
    ));
    let other_call = Context::new(
        desk.log.clone(),
        handle.clone(),
        desk.parts().voice.unwrap(),
        CancellationToken::new(),
    )
    .for_utterance();
    assert!(
        other_call
            .execute(Command::ScheduleCancel {
                id: job["id"].as_str().unwrap().into()
            })
            .await
            .unwrap_err()
            .contains("this call")
    );
    // A later utterance in the same call can cancel its job.
    heard
        .for_utterance()
        .execute(Command::ScheduleCancel {
            id: job["id"].as_str().unwrap().into(),
        })
        .await
        .unwrap();
    let operator_job =
        crate::wire::commands::run(command(params.clone()), &desk.log, &handle, &desk.parts())
            .await
            .unwrap();
    assert_eq!(operator_job["operatorCreated"], true);
    assert!(
        heard
            .execute(Command::ScheduleCancel {
                id: operator_job["id"].as_str().unwrap().into()
            })
            .await
            .is_err()
    );
    granted["backgroundWork"] = json!(false);
    desk.log
        .append(&crate::log::StreamId::Room, &granted)
        .unwrap();
    assert!(!crate::session::schedule::scheduled_run_allowed(
        &desk.log,
        params["personaId"].as_str().unwrap(),
        &run
    ));
    params["when"] = json!(chrono::Utc::now().timestamp_millis() + 120_000);
    assert!(
        heard
            .execute(command(params))
            .await
            .unwrap_err()
            .contains("Background work")
    );
}

#[tokio::test]
async fn a_heard_turn_can_queue_only_three_handoffs_without_waiting_for_startup() {
    use crate::contract::Command;
    let (_root, desk, _calls) = desk(services());
    let handle: Arc<dyn RoomHandle> = desk.clone();
    let create: Command = serde_json::from_value(json!({"cmd":"persona.create","params":{"draft":{"name":"Mack","goal":"Test","cwd":desk.log.root().to_str().unwrap()}}})).unwrap();
    let persona = crate::wire::commands::run(create, &desk.log, &handle, &desk.parts())
        .await
        .unwrap();
    let context = Context::new(
        desk.log.clone(),
        handle,
        desk.parts().voice.unwrap(),
        CancellationToken::new(),
    )
    .for_utterance();
    let command = || Command::SessionPrompt {
        persona_id: persona["id"].as_str().unwrap().into(),
        text: "Check the PR.".into(),
        reply_to: None,
        attachments: None,
    };
    // This desk has no provider: cold startup cannot succeed, but admission is immediate.
    for _ in 0..3 {
        assert_eq!(
            context.execute(command()).await.unwrap()["status"],
            "queued"
        );
    }
    assert!(
        context
            .execute(command())
            .await
            .unwrap_err()
            .contains("three tasks")
    );
    context.cancel.cancel();
    assert!(context.for_utterance().execute(command()).await.is_err());
}

#[tokio::test]
async fn holding_while_narration_finishes_does_not_publish_a_delivery() {
    let fake = Arc::new(Fake::default());
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *lock(&fake.answer_gate) = Some(gate.clone());
    let (_root, desk, calls) = desk(with_fake(fake.clone()));
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk.clone()).unwrap();
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    let (cancel, speech) = calls
        .test_change(&id, |c| Ok((c.cancel.clone(), c.deliveries.child_token())))
        .unwrap();
    let context = Context::new(
        desk.log.clone(),
        desk.clone(),
        desk.parts().voice.unwrap(),
        cancel,
    );
    let worker = calls.clone();
    let call_id = id.clone();
    let task = tokio::spawn(async move {
        worker
            .narrate(
                &call_id,
                &context,
                &Delivery {
                    persona: "mack".into(),
                    event: "reply".into(),
                    name: "Mack".into(),
                    text: "The **tests** passed.".into(),
                },
                &speech,
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(15), async {
        while fake.narrations.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    calls.hold(&id, true).unwrap();
    gate.add_permits(1);
    task.await.unwrap().unwrap();
    while let Ok(event) = rx.try_recv() {
        assert!(!matches!(
            event,
            VoiceEvent::Delivery { .. } | VoiceEvent::Clip { .. }
        ));
    }
    assert!(lock(&fake.spoken).is_empty());
    calls.end(&id).unwrap();
}

#[tokio::test]
async fn narration_budget_denial_ends_the_call_without_paid_fallback_speech() {
    let fake = Arc::new(Fake::default());
    *lock(&fake.narration) = Some(Err(BUDGET_ERROR.into()));
    let (_root, desk, calls) = desk(with_fake(fake.clone()));
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk).unwrap();
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    assert!(calls.delivery(
        "mack",
        "reply",
        "Mack",
        "The **tests** passed.",
        false,
        None
    ));
    event(
        &mut rx,
        |e| matches!(e, VoiceEvent::Said { text, .. } if text == BUDGET_LINE),
    )
    .await;
    event(
        &mut rx,
        |e| matches!(e, VoiceEvent::Clip { mime_type, .. } if mime_type == "audio/wav"),
    )
    .await;
    event(&mut rx, |e| {
        matches!(
            e,
            VoiceEvent::State {
                reason: Some(VoiceEndReason::Budget),
                ..
            }
        )
    })
    .await;
    assert!(lock(&fake.spoken).is_empty());
    assert!(
        calls.budget().ready(&[Kind::Tts, Kind::Dispatcher]).is_ok(),
        "a refused reservation need not have spent the remainder"
    );
}

#[tokio::test]
async fn a_queued_startup_failure_reaches_the_phone_after_hold_or_hangup() {
    for held in [false, true] {
        let fake = Arc::new(Fake::default());
        let (_root, desk, calls) = desk(with_fake(fake.clone()));
        std::fs::write(desk.log.root().join("remote.json"), json!({"desktopId":"desk", "host":"desk.local", "enabled":true, "grants":[{"device":{"id":"phone","name":"Phone","pairedAt":0,"publicKey":"AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE"},"push":{"token":"ExponentPushToken[phone]","platform":"ios"}}]}).to_string()).unwrap();
        let room = desk.test_room();
        let id = Uuid::new_v4().to_string();
        calls.start(&id, desk).unwrap();
        let (_, mut rx) = calls.subscribe(&id).unwrap();
        // The worker cannot run until the first await, after this state change.
        calls.handoff_failed("mack", "Mack");
        if held {
            calls.hold(&id, true).unwrap();
        } else {
            calls.end(&id).unwrap();
        }
        tokio::time::timeout(Duration::from_secs(15), async {
            while room.sent_voice_pushes().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let pushes = room.sent_voice_pushes();
        assert_eq!(pushes.len(), 1);
        assert!(
            pushes[0]["body"]
                .as_str()
                .unwrap()
                .contains("could not be delivered")
        );
        assert!(lock(&fake.spoken).is_empty());
        while let Ok(event) = rx.try_recv() {
            assert!(!matches!(
                event,
                VoiceEvent::Delivery { .. } | VoiceEvent::Clip { .. }
            ));
        }
        calls.end(&id).unwrap();
    }
}
