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

/// A desk whose voice is OpenRouter's and refuses every sentence with
/// `status`, and a call on it: to the teammate Mack if `direct`, else to the
/// desk's call assistant.
async fn refusing(
    status: u16,
    direct: bool,
) -> (
    tempfile::TempDir,
    Arc<crate::desk::Desk>,
    Arc<Calls>,
    String,
    String,
    broadcast::Receiver<VoiceEvent>,
    Arc<Fake>,
) {
    let fake = Arc::new(Fake {
        provider_id: Some("openrouter"),
        ..Default::default()
    });
    *lock(&fake.refuse_speech) = Some(status);
    let (root, desk, calls) = desk(with_fake(fake.clone()));
    let id = Uuid::new_v4().to_string();
    let persona = if direct {
        let persona = mack(&desk).await;
        calls
            .start_target(&id, Some(persona.clone()), false, desk.clone())
            .unwrap();
        persona
    } else {
        calls.start(&id, desk.clone()).unwrap();
        String::new()
    };
    let (_, rx) = calls.subscribe(&id).unwrap();
    (root, desk, calls, id, persona, rx, fake)
}

/// Every event up to and including the first that matches.
async fn through(
    rx: &mut broadcast::Receiver<VoiceEvent>,
    predicate: impl Fn(&VoiceEvent) -> bool,
) -> Vec<VoiceEvent> {
    let mut seen = Vec::new();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let event = rx.recv().await.unwrap();
            let matched = predicate(&event);
            seen.push(event);
            if matched {
                return;
            }
        }
    })
    .await
    .unwrap_or_else(|error| panic!("{error}; voice events: {seen:?}"));
    seen
}

fn said(events: &[VoiceEvent], line: &str) -> bool {
    events
        .iter()
        .any(|e| matches!(e, VoiceEvent::Said { text, .. } if text == line))
}

fn resting(event: &VoiceEvent) -> bool {
    matches!(
        event,
        VoiceEvent::State {
            state: VoiceState::Listening | VoiceState::Thinking | VoiceState::Ended,
            ..
        }
    )
}

/// Mack's reply on turn `seq` of its call, handed to the call whole.
fn reply(calls: &Arc<Calls>, id: &str, persona: &str, seq: u32, text: &str) {
    let origin = on_turn(calls, id, seq);
    assert!(calls.delivery(
        persona,
        &format!("reply-{seq}"),
        "Mack",
        text,
        true,
        Some(&origin)
    ));
}

const REFUSED: &str =
    "OpenRouter refused to speak (out of credit). The answer is in the transcript.";

/// The person was heard and answered; only the voice refused. Asking them
/// to say it again would only have the same answer refused again, so the
/// call never does, and the answer stays on screen and on the call.
#[tokio::test]
async fn a_voice_that_refuses_never_asks_the_person_to_say_it_again() {
    let (_root, desk, calls, id, _, mut rx, _fake) = refusing(402, false).await;
    utterance(&calls, &id, 1).unwrap();
    let events = through(
        &mut rx,
        |e| matches!(e, VoiceEvent::Said { text, .. } if text == REFUSED),
    )
    .await;
    let whole = "The first sentence. The second sentence.";
    assert!(said(&events, whole), "{events:?}");
    assert!(!said(&events, RETRY_LINE), "{events:?}");
    let clip = event(&mut rx, |e| matches!(e, VoiceEvent::Clip { .. })).await;
    assert!(
        matches!(clip, VoiceEvent::Clip { mime_type, data, .. } if mime_type == "audio/wav" && STANDARD.decode(&data).unwrap().starts_with(b"RIFF"))
    );
    let rest = through(&mut rx, resting).await;
    assert!(!said(&rest, RETRY_LINE), "{rest:?}");
    until("the answer kept on the call", || {
        desk.log
            .load(&crate::log::StreamId::Call(id.clone()))
            .iter()
            .any(|e| e["text"] == whole)
    })
    .await;
    calls.end(&id).unwrap();
}

/// Ears that failed are still asked to hear again, even on a call whose
/// voice is refusing: nothing was heard, so nothing was answered.
#[tokio::test]
async fn a_hearing_failure_still_asks_the_person_to_say_it_again() {
    let (_root, _desk, calls, id, _, mut rx, fake) = refusing(402, false).await;
    fake.fail_transcription.store(true, Ordering::SeqCst);
    utterance(&calls, &id, 1).unwrap();
    let events = through(
        &mut rx,
        |e| matches!(e, VoiceEvent::Said { text, .. } if text == RETRY_LINE),
    )
    .await;
    assert!(!said(&events, REFUSED), "{events:?}");
    let rest = through(&mut rx, resting).await;
    assert!(!said(&rest, REFUSED), "{rest:?}");
    calls.end(&id).unwrap();
}

/// The person is told once a call that the voice refused, which provider
/// and why; each answer after it is only shown, in full.
#[tokio::test]
async fn a_refusing_voice_is_named_once_a_call_with_its_reason() {
    let (_root, _desk, calls, id, persona, mut rx, _fake) = refusing(402, true).await;
    reply(&calls, &id, &persona, 1, "The build is red.");
    let events = through(
        &mut rx,
        |e| matches!(e, VoiceEvent::Said { text, .. } if text == REFUSED),
    )
    .await;
    assert!(
        events.iter().any(
            |e| matches!(e, VoiceEvent::Said { text, .. } if text.starts_with("The build is red."))
        ),
        "{events:?}"
    );
    assert!(!said(&events, RETRY_LINE), "{events:?}");
    through(&mut rx, resting).await;
    reply(&calls, &id, &persona, 2, "The flaky test is fixed.");
    let events = through(&mut rx, |e| matches!(e, VoiceEvent::Said { text, .. } if text.starts_with("The flaky test is fixed."))).await;
    let rest = through(&mut rx, resting).await;
    for events in [&events, &rest] {
        assert!(!said(events, REFUSED), "{events:?}");
        assert!(!said(events, RETRY_LINE), "{events:?}");
    }
    assert!(
        !matches!(
            rest.last(),
            Some(VoiceEvent::State {
                state: VoiceState::Ended,
                ..
            })
        ),
        "{rest:?}"
    );
    calls.end(&id).unwrap();
}

/// A voice that keeps refusing ends the call as failing voice always has,
/// and the last line says which provider refused and why.
#[tokio::test]
async fn a_voice_that_keeps_refusing_ends_the_call_saying_why() {
    let (_root, _desk, calls, id, persona, mut rx, _fake) = refusing(402, true).await;
    for seq in 1..=2 {
        reply(&calls, &id, &persona, seq, "The build is red.");
        through(&mut rx, resting).await;
    }
    reply(&calls, &id, &persona, 3, "The build is red.");
    let events = through(&mut rx, |e| {
        matches!(
            e,
            VoiceEvent::State {
                state: VoiceState::Ended,
                ..
            }
        )
    })
    .await;
    assert!(
        said(
            &events,
            "OpenRouter refused to speak (out of credit). Please continue by text."
        ),
        "{events:?}"
    );
    assert!(!said(&events, ERROR_LINE), "{events:?}");
    assert!(matches!(
        events.last(),
        Some(VoiceEvent::State {
            reason: Some(VoiceEndReason::Error),
            ..
        })
    ));
}
