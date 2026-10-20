use super::*;

#[tokio::test]
async fn zero_limits_leave_a_free_voice_ready_and_turn_a_paid_one_off() {
    for (subscription, ready) in [(true, true), (false, false)] {
        let fake = Arc::new(Fake {
            subscription,
            ..Fake::default()
        });
        let (_root, desk, calls) = desk(with_fake(fake));
        desk.log
            .append(
                &StreamId::Room,
                &crate::room::room_event(
                    "setting",
                    json!({"id":"spending", "value":{"dayUsd":0,"monthUsd":0}}),
                ),
            )
            .unwrap();
        let status = calls.status_for(VoiceInputMode::Text);
        assert_eq!(
            status.direct_available, ready,
            "subscription {subscription}"
        );
        if !ready {
            assert_eq!(
                status.unavailable.unwrap(),
                "The Voice budget is set to zero, so its paid use is off. Raise it in Settings › Budgets."
            );
        }
    }
}

/// Heard, spoken and answered for free, a call runs whatever the budgets
/// say, and even when their tally cannot be read.
#[tokio::test]
async fn a_free_call_is_never_ended_by_a_spent_budget() {
    for corrupt in [false, true] {
        let fake = Arc::new(Fake {
            subscription: true,
            free_assistant: true,
            ..Fake::default()
        });
        let (_root, desk, calls) = desk(with_fake(fake.clone()));
        spending(
            &desk,
            json!({"chat": {"dayUsd": 0}, "voice": {"dayUsd": 1, "monthUsd": 1}}),
        );
        if corrupt {
            std::fs::write(desk.log.root().join("voice-ledger.json"), "not json").unwrap();
        } else {
            calls.ledger.charge(Kind::Tts, 5.0);
            calls.ledger.charge(Kind::Dispatcher, 5.0);
        }
        assert!(calls.status().available, "corrupt {corrupt}");
        let id = Uuid::new_v4().to_string();
        calls.start(&id, desk.clone()).unwrap();
        let (_, mut rx) = calls.subscribe(&id).unwrap();
        utterance(&calls, &id, 1).unwrap();
        event(&mut rx, |e| matches!(e, VoiceEvent::Clip { .. })).await;
        assert_eq!(fake.answers.load(Ordering::SeqCst), 1);
        assert!(!lock(&fake.spoken).is_empty());
        assert!(
            calls
                .change(&id, |call| Ok(call.state != VoiceState::Ended))
                .unwrap(),
            "corrupt {corrupt}"
        );
        calls.end(&id).unwrap();
    }
}

/// A call that would pay into a spent budget is refused at the start, with
/// the budget named.
#[tokio::test]
async fn a_paid_call_is_refused_naming_the_budget_that_is_spent() {
    let fake = Arc::new(Fake::default());
    let (_root, desk, calls) = desk(with_fake(fake));
    spending(
        &desk,
        json!({"voice": {"dayUsd": 1}, "chat": {"monthUsd": 1}}),
    );
    calls.ledger.charge(Kind::Tts, 1.0);
    let error = calls
        .start(&Uuid::new_v4().to_string(), desk.clone())
        .unwrap_err();
    assert_eq!(
        error,
        "The Voice budget for today is spent. Raise it in Settings › Budgets."
    );
    spending(&desk, json!({"chat": {"monthUsd": 1}}));
    calls.ledger.charge(Kind::Dispatcher, 1.0);
    let error = calls.start(&Uuid::new_v4().to_string(), desk).unwrap_err();
    assert_eq!(
        error,
        "The Chat budget for this month is spent. Raise it in Settings › Budgets."
    );
}

/// Free speech still pays a billed call assistant, so Chat still refuses it.
#[tokio::test]
async fn free_speech_with_a_paid_assistant_is_refused_by_chat() {
    let free_speech = Arc::new(Fake {
        subscription: true,
        ..Fake::default()
    });
    let (_root, desk, calls) = desk(with_fake(free_speech));
    spending(&desk, json!({"chat": {"dayUsd": 0}}));
    assert!(
        calls
            .start(&Uuid::new_v4().to_string(), desk)
            .unwrap_err()
            .starts_with("The Chat budget is set to zero")
    );
}

#[tokio::test]
async fn text_input_keeps_budget_gating_before_dispatch() {
    let fake = Arc::new(Fake::default());
    let (_root, desk, calls) = desk(with_fake(fake.clone()));
    let id = Uuid::new_v4().to_string();
    calls
        .start_with_input(&id, None, false, VoiceInputMode::Text, desk.clone())
        .unwrap();
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    // A cent spent, then the caps lowered to nothing: today's budget is gone.
    calls.ledger.charge(Kind::Dispatcher, 0.01);
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
    calls.text(&id, 1, "Check the PR.").unwrap();
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
    assert_eq!(fake.answers.load(Ordering::SeqCst), 0);
    assert_eq!(fake.transcriptions.load(Ordering::SeqCst), 0);
    assert!(lock(&fake.spoken).is_empty());
}

#[tokio::test]
async fn a_failed_subscription_never_uses_a_paid_speech_fallback() {
    for streaming in [false, true] {
        let subscription = Arc::new(Fake {
            fail: true,
            subscription: true,
            ..Fake::default()
        });
        let paid = Arc::new(Fake::default());
        let mut services = with_fake(subscription.clone());
        services.speech.fallback_tts = Some(paid.clone());
        let (_root, desk, calls) = desk(services);
        let id = Uuid::new_v4().to_string();
        calls
            .start_with_input(&id, None, streaming, VoiceInputMode::Text, desk)
            .unwrap();
        let (_, mut rx) = calls.subscribe(&id).unwrap();
        calls.text(&id, 1, "Check the PR.").unwrap();
        event(&mut rx, |e| {
            matches!(
                e,
                VoiceEvent::State {
                    state: VoiceState::Listening,
                    ..
                }
            )
        })
        .await;
        assert!(lock(&paid.spoken).is_empty());
        assert_eq!(calls.ledger.spent().unwrap().day.total(), 0.0);
        calls.end(&id).unwrap();
    }
}

#[tokio::test]
async fn whole_clip_fallback_cannot_use_a_disconnected_subscription_login() {
    let root = tempfile::tempdir().unwrap();
    let desk = crate::desk::Desk::open_with_voice_services(
        root.path(),
        Arc::new(crate::credentials::MemoryStore::default()),
        None,
    )
    .unwrap();
    let calls = desk.parts().voice.unwrap();
    let primary = Arc::new(Fake {
        fail: true,
        ..Fake::default()
    });
    let subscription = Arc::new(Fake {
        subscription: true,
        ..Fake::default()
    });
    let speech = CallSpeech::output(SpeechOutput {
        tts: primary,
        fallback_tts: Some(subscription.clone()),
    });
    assert!(
        calls
            .synthesize(&speech, "Test.")
            .await
            .unwrap_err()
            .contains("no longer connected")
    );
    assert!(lock(&subscription.spoken).is_empty());
}
