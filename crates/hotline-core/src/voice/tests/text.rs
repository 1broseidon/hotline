use super::*;

#[tokio::test]
async fn finalized_text_skips_stt_and_cannot_replay_or_overtake_a_pending_turn() {
    let fake = Arc::new(Fake::default());
    fake.fail_transcription.store(true, Ordering::SeqCst);
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *lock(&fake.answer_gate) = Some(gate.clone());
    let (root, desk, calls) = desk(with_fake(fake.clone()));
    let id = Uuid::new_v4().to_string();
    let call = calls
        .start_with_input(&id, None, true, VoiceInputMode::Text, desk)
        .unwrap();
    assert_eq!(call.input_mode, VoiceInputMode::Text);
    assert_eq!(call.input, ["text/plain"]);
    assert!(
        calls
            .change(&id, |c| Ok(c.speech_services.stt.is_none()))
            .unwrap()
    );
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    calls.text(&id, 1, "  Check the failing test.  ").unwrap();
    assert!(
        calls
            .text(&id, 1, "Duplicate.")
            .unwrap_err()
            .contains("must rise")
    );
    assert!(
        calls
            .text(&id, 2, "Too early.")
            .unwrap_err()
            .contains("still being handled")
    );
    let heard = event(&mut rx, |e| matches!(e, VoiceEvent::Heard { .. })).await;
    assert!(
        matches!(heard, VoiceEvent::Heard { seq: 1, text } if text == "Check the failing test.")
    );
    gate.add_permits(1);
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
    assert_eq!(fake.transcriptions.load(Ordering::SeqCst), 0);
    assert_eq!(fake.answers.load(Ordering::SeqCst), 1);
    let ledger: Value =
        serde_json::from_slice(&std::fs::read(root.path().join("voice-ledger.json")).unwrap())
            .unwrap();
    assert_eq!(ledger["daySpend"]["stt"], 0.0);
    assert_eq!(ledger["monthSpend"]["stt"], 0.0);
    assert!(ledger["daySpend"]["tts"].as_f64().unwrap() > 0.0);
    assert!(calls.text(&id, 1, "Replay after completion.").is_err());
    calls.end(&id).unwrap();
}

#[tokio::test]
async fn text_mode_needs_only_output_configuration_and_legacy_audio_stays_unavailable() {
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_voice_services(
            root.path(),
            Arc::new(crate::credentials::MemoryStore::default()),
            None,
        )
        .unwrap(),
    );
    let calls = desk.voice().unwrap();
    calls
        .vault
        .create("openai", "Fixture", "unused-test-key")
        .unwrap();
    desk.log
        .append(
            &StreamId::Room,
            &crate::room::room_event(
                "setting",
                json!({
                    "id":"voice", "value": {
                        "stt":{"provider":"not-connected"},
                        "tts":{"provider":"openai"},
                        "dispatcher":{"provider":"openai","model":"gpt-5-mini"}
                    }
                }),
            ),
        )
        .unwrap();
    assert!(!calls.status().available);
    let status = calls.status_for(VoiceInputMode::Text);
    assert!(status.available, "{status:?}");
    assert!(status.stt.is_none());
    let id = Uuid::new_v4().to_string();
    assert!(calls.start(&id, desk.clone()).is_err());
    let call = calls
        .start_with_input(&id, None, true, VoiceInputMode::Text, desk.clone())
        .unwrap();
    assert_eq!(call.input, ["text/plain"]);
    assert!(calls.start_target(&id, None, true, desk).is_err());
    assert!(
        calls
            .audio(&id, 1, 0, &STANDARD.encode([0u8; 2]), true)
            .unwrap_err()
            .contains("does not accept live PCM")
    );
    calls.end(&id).unwrap();
}

#[tokio::test]
async fn text_and_audio_calls_enforce_the_negotiated_mode_and_transcript_bounds() {
    let (_root, desk, calls) = desk(services());
    let old = Uuid::new_v4().to_string();
    let old_call = calls.start(&old, desk.clone()).unwrap();
    assert_eq!(old_call.input_mode, VoiceInputMode::Audio);
    assert!(old_call.input.iter().any(|v| v == "audio/wav"));
    assert!(calls.text(&old, 1, "Text on a legacy call.").is_err());
    let id = Uuid::new_v4().to_string();
    calls
        .start_with_input(&id, None, true, VoiceInputMode::Text, desk)
        .unwrap();
    assert!(utterance(&calls, &id, 1).is_err());
    for text in [" ".to_string(), "a".repeat(8_001), "🙂".repeat(8_001)] {
        assert!(calls.text(&id, 1, &text).is_err());
    }
    calls.hold(&id, true).unwrap();
    assert!(calls.text(&id, 1, "Held input.").is_err());
    assert!(calls.change(&id, |c| Ok(c.seq.is_none())).unwrap());
    calls.end(&id).unwrap();
}

#[tokio::test]
async fn canceled_text_is_not_dispatched_and_disconnect_ends_its_bound_call() {
    for hold in [false, true] {
        let fake = Arc::new(Fake::default());
        let (_root, desk, calls) = desk(with_fake(fake.clone()));
        let id = Uuid::new_v4().to_string();
        calls
            .start_with_input(&id, None, false, VoiceInputMode::Text, desk)
            .unwrap();
        let (_, mut rx) = calls.subscribe(&id).unwrap();
        // No yield between submission and cancellation: the instruction has not
        // yet been accepted by a dispatcher or agent.
        calls.text(&id, 1, "Canceled before acceptance.").unwrap();
        if hold {
            calls.hold(&id, true).unwrap();
        } else {
            calls.interrupt(&id).unwrap();
        }
        tokio::time::timeout(Duration::from_secs(1), async {
            while calls.change(&id, |c| Ok(c.utterance_pending)).unwrap() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(fake.transcriptions.load(Ordering::SeqCst), 0);
        assert_eq!(fake.answers.load(Ordering::SeqCst), 0);
        while let Ok(e) = rx.try_recv() {
            assert!(!matches!(
                e,
                VoiceEvent::Heard { .. } | VoiceEvent::Said { .. } | VoiceEvent::Clip { .. }
            ));
        }
        if hold {
            calls.hold(&id, false).unwrap();
        }
        assert!(
            calls
                .text(&id, 1, "A canceled sequence still cannot replay.")
                .is_err()
        );
        let revoked = CancellationToken::new();
        calls.bind_connection(id.clone(), revoked.clone());
        revoked.cancel();
        event(&mut rx, |e| {
            matches!(
                e,
                VoiceEvent::State {
                    reason: Some(VoiceEndReason::Client),
                    ..
                }
            )
        })
        .await;
        assert!(calls.text(&id, 2, "After disconnect.").is_err());
    }
}

#[tokio::test]
async fn device_goodbye_needs_no_fabricated_audio_or_dispatcher() {
    for direct in [false, true] {
        let fake = Arc::new(Fake::default());
        fake.fail_transcription.store(true, Ordering::SeqCst);
        let (_root, desk, calls) = desk(with_fake(fake.clone()));
        let id = Uuid::new_v4().to_string();
        calls
            .start_with_input(&id, None, true, VoiceInputMode::Text, desk)
            .unwrap();
        if direct {
            calls
                .change(&id, |c| {
                    c.target = Some("fixture-agent".into());
                    Ok(())
                })
                .unwrap();
        }
        let (_, mut rx) = calls.subscribe(&id).unwrap();
        calls.text(&id, 1, "Okay, bye.").unwrap();
        event(&mut rx, |e| {
            matches!(
                e,
                VoiceEvent::State {
                    reason: Some(VoiceEndReason::Goodbye),
                    ..
                }
            )
        })
        .await;
        assert_eq!(fake.answers.load(Ordering::SeqCst), 0);
        assert_eq!(fake.transcriptions.load(Ordering::SeqCst), 0);
        assert_eq!(*lock(&fake.spoken), ["Goodbye."]);
    }
}
