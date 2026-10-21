use super::*;

#[test]
fn goodbye_matches_the_whole_utterance() {
    for text in [
        "Bye.",
        " goodBYE! ",
        "hang up",
        "end the call",
        "That's all.",
        "Okay, bye.",
        "Thanks, bye",
        "Alright, bye bye",
        "Bye, Hotline",
        "ok bye-bye",
        "Thank you, goodbye desk!",
        "All right, okay, thanks, see you later, Hotline.",
        "See ya later",
        "Good bye",
        "Okay, hang up, desk.",
        "Thanks, end call",
        "Thank you, that is all, Hotline!",
        "Okay, that’s all.",
        "  OKAY\t\nbye   desk!  ",
    ] {
        assert!(goodbye(text), "{text}");
    }
    for text in [
        "Ask Mack to say goodbye",
        "Don't hang up",
        "bye, ask Mack first",
        "that's all for the PR",
        "goodbye\nask Mack",
        "say bye to Mack",
        "tell Ada goodbye",
        "Okay, bye, then check the PR",
        "thanks for saying goodbye",
        "bye Hotline please run the tests",
        "okay",
        "thank you",
        "bye 123",
        "bye 再检查",
        "",
    ] {
        assert!(!goodbye(text), "{text}");
    }
}

#[tokio::test]
async fn goodbye_is_spoken_without_a_model_and_kept_on_the_call() {
    let fake = Arc::new(Fake::default());
    *lock(&fake.transcript) = "Okay, bye.".into();
    let (_root, desk, calls) = desk(with_fake(fake.clone()));
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk.clone()).unwrap();
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    utterance(&calls, &id, 0).unwrap();
    event(&mut rx, |e| matches!(e, VoiceEvent::Clip { .. })).await;
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
    assert_eq!(
        crate::room::roster(&desk.log),
        [] as [crate::contract::Persona; 0]
    );
    assert!(
        desk.log
            .load(&StreamId::Call(id))
            .iter()
            .any(|event| event["text"] == "Goodbye.")
    );
}

#[tokio::test]
async fn clips_are_sent_in_sentence_order_and_sequences_cannot_replay() {
    let (_root, desk, calls) = desk(services());
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk).unwrap();
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    utterance(&calls, &id, 1).unwrap();
    assert!(utterance(&calls, &id, 1).is_err());
    // One answer is one line: the second sentence extends the first under
    // its id, and its clips carry on that id's indices.
    let mut line = None;
    for (index, text) in [
        "The first sentence.",
        "The first sentence. The second sentence.",
    ]
    .into_iter()
    .enumerate()
    {
        let said = event(&mut rx, |e| matches!(e, VoiceEvent::Said { .. })).await;
        let VoiceEvent::Said { id, text: actual } = said else {
            unreachable!("event() waited for a Said")
        };
        assert_eq!(actual, text);
        let line = line.get_or_insert(id.clone());
        assert_eq!(&id, line);
        let clip = event(&mut rx, |e| matches!(e, VoiceEvent::Clip { .. })).await;
        assert!(
            matches!(&clip, VoiceEvent::Clip { id, index: at, r#final: false, .. } if id == line && *at == index as u32),
            "{clip:?}"
        );
    }
    let last = event(&mut rx, |e| matches!(e, VoiceEvent::Clip { .. })).await;
    assert!(matches!(
        last,
        VoiceEvent::Clip { id, index: 2, r#final: true, data, .. } if Some(&id) == line.as_ref() && data.is_empty()
    ));
    calls.end(&id).unwrap();
    calls.end(&id).unwrap();
}

#[tokio::test]
async fn holding_uses_normal_push_and_refuses_microphone_audio() {
    let (_root, desk, calls) = desk(services());
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk).unwrap();
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    calls.hold(&id, true).unwrap();
    assert!(utterance(&calls, &id, 1).is_err());
    assert!(!calls.delivery("mack", "reply", "Mack", "The tests passed.", true, None));
    tokio::time::sleep(Duration::from_millis(30)).await;
    while let Ok(event) = rx.try_recv() {
        assert!(!matches!(
            event,
            VoiceEvent::Clip { .. } | VoiceEvent::Said { .. } | VoiceEvent::Delivery { .. }
        ));
    }
    calls.hold(&id, false).unwrap();
    assert!(calls.delivery(
        "mack",
        "new-reply",
        "Mack",
        "The new tests passed.",
        false,
        None
    ));
    event(
        &mut rx,
        |e| matches!(e, VoiceEvent::Delivery { event_id, .. } if event_id == "new-reply"),
    )
    .await;
    calls.end(&id).unwrap();
}

#[tokio::test]
async fn interrupt_cancels_audio_without_ending_the_call() {
    let fake = Arc::new(Fake::default());
    let (_root, desk, calls) = desk(with_fake(fake.clone()));
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk).unwrap();
    *lock(&fake.delay) = Duration::from_secs(1);
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    utterance(&calls, &id, 1).unwrap();
    event(
        &mut rx,
        |e| matches!(e, VoiceEvent::Said { text, .. } if text == "The first sentence."),
    )
    .await;
    calls.interrupt(&id).unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    while let Ok(event) = rx.try_recv() {
        assert!(!matches!(
            event,
            VoiceEvent::Clip { .. }
                | VoiceEvent::State {
                    state: VoiceState::Ended,
                    ..
                }
        ));
    }
    calls.end(&id).unwrap();
}

#[tokio::test]
async fn budget_failure_speaks_the_bundled_line_and_stops_without_a_model() {
    let fake = Arc::new(Fake::default());
    let (_root, desk, calls) = desk(with_fake(fake.clone()));
    spending(&desk, json!({"chat": {"dayUsd": 2}}));
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk).unwrap();
    calls.budget().charge(Kind::Dispatcher, 3.0);
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    utterance(&calls, &id, 1).unwrap();
    event(
        &mut rx,
        |e| matches!(e, VoiceEvent::Said { text, .. } if text == BUDGET_LINE),
    )
    .await;
    event(&mut rx, |e| matches!(e, VoiceEvent::Clip { .. })).await;
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
    assert_eq!(lock(&fake.spoken).len(), 0);
}

#[tokio::test]
async fn a_failed_voice_uses_one_fallback_and_accounts_for_both_attempts() {
    let primary = Arc::new(Fake {
        fail: true,
        output_mime: "audio/mpeg",
        ..Default::default()
    });
    let fallback = Arc::new(Fake::default());
    let mut services = with_fake(primary.clone());
    services.speech.fallback_tts = Some(fallback.clone());
    let (_root, desk, calls) = desk(services.clone());
    let id = Uuid::new_v4().to_string();
    assert_eq!(calls.start(&id, desk).unwrap().output, "audio/mpeg");
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    calls
        .test_say(
            &id,
            "Test.",
            &CancellationToken::new(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    let clip = event(&mut rx, |e| matches!(e, VoiceEvent::Clip { .. })).await;
    assert!(matches!(clip, VoiceEvent::Clip { mime_type, data, .. }
        if mime_type == "audio/wav" && STANDARD.decode(&data).unwrap() == wav()));
    assert_eq!(lock(&primary.spoken).len(), 1);
    assert_eq!(lock(&fallback.spoken).len(), 1);
    assert!(
        (calls.status().budget.spent_day_usd - 2.0 * ledger::tts_usd("fixture", 5)).abs() < 1e-9
    );
    calls.end(&id).unwrap();
}

#[tokio::test]
async fn a_fallback_needs_its_own_reservation_and_is_attempted_only_once() {
    for denied in [true, false] {
        let primary = Arc::new(Fake {
            fail: true,
            ..Default::default()
        });
        let fallback = Arc::new(Fake {
            fail: true,
            ..Default::default()
        });
        let mut services = with_fake(primary.clone());
        services.speech.fallback_tts = Some(fallback.clone());
        let (_root, desk, calls) = desk(services.clone());
        let cost = ledger::tts_usd("fixture", 5);
        if denied {
            desk.log
                .append(
                    &crate::log::StreamId::Room,
                    &json!({"kind":"setting","id":"voice","value":{"dayUsd":cost * 1.5}}),
                )
                .unwrap();
        }
        let error = calls
            .test_synthesize(&CallSpeech::audio(services.speech.clone()), "Test.")
            .await
            .unwrap_err();
        assert_eq!(lock(&primary.spoken).len(), 1);
        assert_eq!(lock(&fallback.spoken).len(), usize::from(!denied));
        assert_eq!(error == BUDGET_ERROR, denied);
        assert!(
            (calls.status().budget.spent_day_usd - cost * if denied { 1.0 } else { 2.0 }).abs()
                < 1e-9
        );
    }
}

#[tokio::test]
async fn idle_replacement_and_disconnect_end_only_the_named_call() {
    let (_root, desk, calls) = desk(services());
    let first = Uuid::new_v4().to_string();
    calls.start(&first, desk.clone()).unwrap();
    let second = Uuid::new_v4().to_string();
    calls.start(&second, desk.clone()).unwrap();
    assert!(matches!(
        calls.subscribe(&first).unwrap().0,
        VoiceEvent::State {
            reason: Some(VoiceEndReason::Replaced),
            ..
        }
    ));
    let retry = calls.start(&first, desk.clone()).unwrap();
    assert_eq!(retry.call_id, first);
    assert!(matches!(
        calls.subscribe(&second).unwrap().0,
        VoiceEvent::State {
            state: VoiceState::Listening,
            ..
        }
    ));
    calls.test_activity(
        &second,
        Instant::now()
            .checked_sub(Duration::from_millis(crate::thread::QUIET_MS as u64))
            .expect("the machine has been up for longer than a call stays quiet"),
    );
    for (call, quiet) in calls.quiet() {
        if quiet >= crate::thread::QUIET_MS {
            calls.end_quiet(&call);
        }
    }
    assert!(matches!(
        calls.subscribe(&second).unwrap().0,
        VoiceEvent::State {
            reason: Some(VoiceEndReason::Idle),
            ..
        }
    ));
    let third = Uuid::new_v4().to_string();
    calls.start(&third, desk).unwrap();
    let (_, mut events) = calls.subscribe(&third).unwrap();
    let revoked = CancellationToken::new();
    calls.bind_connection(third, revoked.clone());
    revoked.cancel();
    event(&mut events, |e| {
        matches!(
            e,
            VoiceEvent::State {
                reason: Some(VoiceEndReason::Client),
                ..
            }
        )
    })
    .await;
}

#[tokio::test]
async fn cards_remain_cards_and_voice_tools_cannot_answer_them() {
    let (_root, desk, calls) = desk(services());
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk.clone()).unwrap();
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    calls.card(
        "mack",
        &json!({"kind":"permission","requestId":"approval","decision":null}),
    );
    assert!(
        matches!(rx.recv().await.unwrap(), VoiceEvent::Card { request_id, .. } if request_id == "approval")
    );
    let context = Context::new(
        desk.log.clone(),
        desk.clone(),
        desk.parts().voice.unwrap(),
        CancellationToken::new(),
    )
    .for_utterance();
    let forbidden: crate::contract::Command = serde_json::from_value(json!({"cmd":"human.answer","params":{"personaId":"mack","actionId":"approval","status":"done","note":null}})).unwrap();
    assert!(
        context
            .execute(forbidden)
            .await
            .unwrap_err()
            .contains("cannot run")
    );
    calls.end(&id).unwrap();
}

#[test]
fn audio_limits_use_actual_wav_samples() {
    assert!(validate_audio("audio/wav", &wav(), 1000).is_ok());
    assert!(validate_audio("audio/wav", &wav(), 100).is_err());
    assert!(validate_audio("audio/wav", b"RIFFstub", 1000).is_err());
    assert!(validate_audio("audio/mp4", b"not audio", 1000).is_err());
}

#[test]
fn mp4_duration_cannot_be_hidden_behind_a_short_client_claim() {
    fn atom(kind: [u8; 4], payload: Vec<u8>) -> Vec<u8> {
        let mut bytes = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        bytes.extend(kind);
        bytes.extend(payload);
        bytes
    }
    fn mp4(milliseconds: u32) -> Vec<u8> {
        let mut mdhd = vec![0u8; 12];
        mdhd.extend(1000u32.to_be_bytes());
        mdhd.extend(milliseconds.to_be_bytes());
        let mut bytes = atom(*b"ftyp", b"M4A ".to_vec());
        bytes.extend(atom(
            *b"moov",
            atom(*b"trak", atom(*b"mdia", atom(*b"mdhd", mdhd))),
        ));
        bytes
    }
    assert_eq!(validate_audio("audio/mp4", &mp4(1000), 1000).unwrap(), 1000);
    assert!(validate_audio("audio/mp4", &mp4(20001), 1000).is_err());
    assert!(validate_audio("audio/mp4", &mp4(10000), 1000).is_err());
    assert!(validate_audio("audio/mp4", &mp4(1000)[..16], 1000).is_err());
}

#[tokio::test]
async fn a_late_provider_completion_cannot_publish_after_interrupt() {
    let (_root, desk, calls) = desk(services());
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk).unwrap();
    let (_, mut events) = calls.subscribe(&id).unwrap();
    let generation = calls
        .test_change(&id, |call| Ok(call.speech.clone()))
        .unwrap();
    calls.interrupt(&id).unwrap();
    // Model the provider returning between the outer cancellation check and
    // publishing its finished clip. The publication boundary must recheck it.
    calls.test_clip(
        &id,
        "old-sentence",
        0,
        true,
        &Clip {
            mime: "audio/wav".into(),
            bytes: wav(),
        },
        Some(&generation),
    );
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event, VoiceEvent::Clip { .. }));
    }
    calls.end(&id).unwrap();
}

#[test]
fn sentence_boundaries_keep_paths_versions_and_late_warnings() {
    let text = "Check main.rs in v1.2. Then run tests! Approval is still required. Do not deploy.";
    assert_eq!(
        sentences(text),
        [
            "Check main.rs in v1.2.",
            "Then run tests!",
            "Approval is still required.",
            "Do not deploy."
        ]
    );
    let long = format!("{} Approval is required.", "Keep this word. ".repeat(50));
    assert_eq!(sentences(&long).last().unwrap(), "Approval is required.");
    let mut pending = "Check main.".to_string();
    assert_eq!(take_sentences(&mut pending, false), [] as [String; 0]);
    pending.push_str("rs. Next");
    assert_eq!(take_sentences(&mut pending, false), ["Check main.rs."]);
    assert_eq!(take_sentences(&mut pending, true), ["Next"]);
    // A list is said item by item, and a run-on is cut at a pause.
    assert_eq!(
        sentences("Here are the files:\n- `a.png` (2 MB)\n- `b.png` (1 MB)"),
        [
            "Here are the files:",
            "- `a.png` (2 MB)",
            "- `b.png` (1 MB)"
        ]
    );
    let run_on = "word ".repeat(70) + "and then, " + &"more ".repeat(70);
    let pieces = sentences(&run_on);
    assert!(pieces.len() > 1 && pieces.iter().all(|p| p.len() <= 300));
    assert_eq!(pieces.join(" "), run_on.trim());
    let unbroken = "é".repeat(400);
    assert_eq!(sentences(&unbroken).concat(), unbroken);
}

#[test]
fn a_teammate_voice_applies_only_to_the_model_it_was_picked_from() {
    let desk = VoiceSettings::default();
    let ara = crate::contract::PersonaVoice {
        provider_id: "xai-subscription".into(),
        model_id: "grok-voice-tts-1.0".into(),
        voice: "ara".into(),
    };
    let speaking = |provider: &str, model: &str, voice: &str| speech::SpeechId {
        provider_id: provider.into(),
        model_id: model.into(),
        voice: Some(voice.into()),
    };
    let own = own_voice(
        &desk,
        &ara,
        &speaking("xai-subscription", "grok-voice-tts-1.0", "eve"),
    )
    .unwrap();
    let tts = own.tts.unwrap();
    assert_eq!(
        (
            tts.provider_id.as_str(),
            tts.model_id.as_deref(),
            tts.voice.as_deref()
        ),
        ("xai-subscription", Some("grok-voice-tts-1.0"), Some("ara"))
    );
    assert_eq!((own.chat, own.voice), (desk.chat, desk.voice));
    // The desk moved to another provider or model: its own voice stands.
    assert!(own_voice(&desk, &ara, &speaking("xai", "grok-voice-tts-1.0", "eve")).is_none());
    assert!(own_voice(&desk, &ara, &speaking("xai-subscription", "other", "eve")).is_none());
    // Already that voice: nothing to change.
    assert!(
        own_voice(
            &desk,
            &ara,
            &speaking("xai-subscription", "grok-voice-tts-1.0", "ara")
        )
        .is_none()
    );
}
