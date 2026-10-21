use super::*;

#[tokio::test]
async fn errors_are_spoken_and_only_three_consecutive_failures_end_the_call() {
    let fake = Arc::new(Fake::default());
    fake.fail_transcription.store(true, Ordering::SeqCst);
    let (_root, desk, calls) = desk(with_fake(fake.clone()));
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk).unwrap();
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    // A successful utterance resets the failure count.
    for (seq, fail, last) in [
        (1, true, false),
        (2, true, false),
        (3, false, false),
        (4, true, false),
        (5, true, false),
        (6, true, true),
    ] {
        fake.fail_transcription.store(fail, Ordering::SeqCst);
        utterance(&calls, &id, seq).unwrap();
        if fail {
            let line = if last { ERROR_LINE } else { RETRY_LINE };
            event(
                &mut rx,
                |e| matches!(e, VoiceEvent::Said { text, .. } if text == line),
            )
            .await;
            let clip = event(&mut rx, |e| matches!(e, VoiceEvent::Clip { .. })).await;
            assert!(
                matches!(clip, VoiceEvent::Clip { mime_type, data, .. } if mime_type == "audio/wav" && STANDARD.decode(&data).unwrap().starts_with(b"RIFF"))
            );
        }
        let state = event(&mut rx, |e| {
            matches!(
                e,
                VoiceEvent::State {
                    state: VoiceState::Listening | VoiceState::Ended,
                    ..
                }
            )
        })
        .await;
        assert!(if last {
            matches!(
                state,
                VoiceEvent::State {
                    reason: Some(VoiceEndReason::Error),
                    ..
                }
            )
        } else {
            matches!(
                state,
                VoiceEvent::State {
                    state: VoiceState::Listening,
                    ..
                }
            )
        });
    }
}

#[tokio::test]
async fn a_blocked_dispatcher_says_nothing_and_interrupt_preserves_its_text() {
    for held in [false, true] {
        let fake = Arc::new(Fake::default());
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *lock(&fake.answer_gate) = Some(gate.clone());
        let (_root, desk, calls) = desk(with_fake(fake.clone()));
        let id = Uuid::new_v4().to_string();
        calls.start(&id, desk.clone()).unwrap();
        let (_, mut rx) = calls.subscribe(&id).unwrap();
        utterance(&calls, &id, 1).unwrap();
        // The model cannot finish until this test releases it. No wall-clock SLA in CI.
        // While it works the desk says nothing: the window's blip-blip covers the wait.
        event(&mut rx, |e| matches!(e, VoiceEvent::Heard { .. })).await;
        tokio::task::yield_now().await;
        while let Ok(e) = rx.try_recv() {
            assert!(
                !matches!(e, VoiceEvent::Said { .. } | VoiceEvent::Clip { .. }),
                "{e:?}"
            );
        }
        assert!(
            calls
                .test_change(&id, |c| Ok(c.first_clip_started.is_some()))
                .unwrap()
        );
        assert!(lock(&fake.spoken).is_empty());
        if held {
            calls.hold(&id, true).unwrap();
        } else {
            calls.interrupt(&id).unwrap();
        }
        assert!(utterance(&calls, &id, 2).is_err());
        gate.add_permits(1);
        let whole = "The first sentence. The second sentence.";
        event(
            &mut rx,
            |e| matches!(e, VoiceEvent::Said { text, .. } if text == whole),
        )
        .await;
        tokio::task::yield_now().await;
        assert!(lock(&fake.spoken).is_empty());
        until("the answer", || {
            desk.log
                .load(&crate::log::StreamId::Call(id.clone()))
                .iter()
                .any(|e| e["text"] == whole)
        })
        .await;
        while let Ok(e) = rx.try_recv() {
            assert!(!matches!(e, VoiceEvent::Clip { .. }));
        }
        calls.end(&id).unwrap();
    }
}

#[tokio::test]
async fn failed_or_empty_narration_falls_back_without_empty_deliveries() {
    for result in [Err("provider down".into()), Ok(" \n".into())] {
        let fake = Arc::new(Fake::default());
        *lock(&fake.narration) = Some(result);
        let (_root, desk, calls) = desk(with_fake(fake));
        let id = Uuid::new_v4().to_string();
        calls.start(&id, desk).unwrap();
        let (_, mut rx) = calls.subscribe(&id).unwrap();
        assert!(calls.delivery(
            "mack",
            "result",
            "Mack",
            "The tests failed. Check `main.rs`.",
            false,
            None
        ));
        event(
            &mut rx,
            |e| matches!(e, VoiceEvent::Delivery { text, .. } if text == "The tests failed."),
        )
        .await;
        event(&mut rx, |e| matches!(e, VoiceEvent::Clip { .. })).await;
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
        assert!(!calls.delivery("mack", "empty", "Mack", " \n", true, None));
        calls.end(&id).unwrap();
    }
}

#[tokio::test]
async fn goodbye_still_speaks_and_ends_as_goodbye_when_tts_fails() {
    let fake = Arc::new(Fake {
        fail: true,
        ..Default::default()
    });
    *lock(&fake.transcript) = "Bye.".into();
    let (_root, desk, calls) = desk(with_fake(fake));
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk).unwrap();
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    utterance(&calls, &id, 1).unwrap();
    let clip = event(&mut rx, |e| matches!(e, VoiceEvent::Clip { .. })).await;
    assert!(matches!(clip, VoiceEvent::Clip { mime_type, .. } if mime_type == "audio/wav"));
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
}

#[tokio::test]
async fn a_delivery_never_replaces_the_utterance_cancel_token() {
    let fake = Arc::new(Fake::default());
    *lock(&fake.delay) = Duration::from_millis(100);
    let (_root, desk, calls) = desk(with_fake(fake));
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk).unwrap();
    let original = calls.test_change(&id, |c| Ok(c.speech.clone())).unwrap();
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    assert!(calls.delivery("mack", "reply", "Mack", "The tests passed.", false, None));
    event(&mut rx, |e| matches!(e, VoiceEvent::Said { .. })).await;
    calls.interrupt(&id).unwrap();
    assert!(original.is_cancelled());
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
    tokio::time::sleep(Duration::from_millis(120)).await;
    while let Ok(e) = rx.try_recv() {
        assert!(!matches!(e, VoiceEvent::Clip { .. }));
    }
    calls.end(&id).unwrap();
}
