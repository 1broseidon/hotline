use super::*;

/// A desk with only a ChatGPT login has no pipeline to hear or speak with,
/// and GPT-Live does both itself: with it picked, a call to a teammate is
/// offered and starts on the front alone, and losing the front, with
/// nothing to fall back on, ends the call rather than carrying it on mute.
#[tokio::test]
async fn a_front_takes_calls_on_a_desk_with_no_pipeline_speech() {
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_voice_services(
            root.path(),
            Arc::new(crate::credentials::MemoryStore::default()),
            None,
        )
        .unwrap(),
    );
    let calls = desk.parts().voice.unwrap();
    let persona = mack(&desk).await;
    assert!(!calls.status().direct_available, "nothing is connected");
    let (login, _dir) = desk.test_vault().begin_login().unwrap();
    desk.test_vault()
        .finish_login(&login, realtime::live::PROVIDER_ID, "ChatGPT")
        .unwrap();
    assert!(
        !calls.status().direct_available,
        "a ChatGPT login is no speech connection, and no front is picked"
    );
    desk.log
        .append(
            &StreamId::Room,
            &crate::room::room_event(
                "setting",
                json!({"id": "voice", "value": {"front": {"provider": "gpt-live"}}}),
            ),
        )
        .unwrap();
    let status = calls.status();
    assert!(status.direct_available, "{status:?}");
    assert!(
        status
            .unavailable
            .as_deref()
            .is_none_or(|why| !why.contains("speech")),
        "{status:?}"
    );
    let id = Uuid::new_v4().to_string();
    let call = calls
        .start_call(
            &id,
            CallRequest {
                thread: None,
                target: Some(persona.clone()),
                stream_audio: true,
                input_mode: VoiceInputMode::Audio,
                mode: VoiceCallMode::Front,
                mic_rate: Some(48_000),
            },
            desk.clone(),
        )
        .unwrap();
    assert!(call.front.is_some(), "{call:?}");
    assert!(
        calls
            .start_call(
                &id,
                CallRequest {
                    thread: None,
                    target: Some(persona),
                    stream_audio: true,
                    input_mode: VoiceInputMode::Audio,
                    mode: VoiceCallMode::Pipeline,
                    mic_rate: None,
                },
                desk.clone(),
            )
            .unwrap_err()
            .contains("cannot change")
    );
    calls.test_lose_front(&id);
    assert!(
        calls
            .test_change(&id, |c| Ok(c.state))
            .unwrap_err()
            .contains("ended"),
        "no pipeline to fall back on"
    );
}

#[tokio::test(start_paused = true)]
async fn a_spent_live_budget_says_one_local_notice_and_never_selects_legacy() {
    struct PaidFront(Mutex<Option<realtime::RealtimeSession>>);
    #[async_trait]
    impl realtime::Realtime for PaidFront {
        fn id(&self) -> SpeechId {
            SpeechId {
                provider_id: "xai".into(),
                model_id: "fixture".into(),
                voice: None,
            }
        }
        fn is_subscription(&self) -> bool {
            false
        }
        async fn open(
            &self,
            _: realtime::RealtimeConfig,
        ) -> Result<realtime::RealtimeSession, speech::SpeechError> {
            Ok(lock(&self.0).take().unwrap())
        }
    }
    let (session, _input, _output) = realtime::RealtimeSession::scripted(16);
    let mut services = with_fake(Arc::new(Fake::default()));
    services.realtime = Some(Arc::new(PaidFront(Mutex::new(Some(session)))));
    let (_root, desk, calls) = desk(services);
    spending(&desk, json!({"voice": {"dayUsd": 1}}));
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk).unwrap();
    let (_, mut events) = calls.subscribe(&id).unwrap();
    event(&mut events, |e| matches!(e, VoiceEvent::State { .. })).await;
    calls.budget().charge(Kind::Tts, 1.0);
    tokio::time::advance(Duration::from_secs(60)).await;
    let mut notices = Vec::new();
    let mut clips = 0;
    loop {
        match events.recv().await.unwrap() {
            VoiceEvent::Said { text, .. } => notices.push(text),
            VoiceEvent::Clip { mime_type, .. } => {
                assert_eq!(mime_type, "audio/wav");
                clips += 1;
            }
            VoiceEvent::State {
                state: VoiceState::Ended,
                live_error,
                ..
            } => {
                assert!(live_error.unwrap().contains("Raise it in Settings"));
                break;
            }
            _ => {}
        }
    }
    assert_eq!(
        notices,
        ["The Voice budget is spent. Raise it in Settings or choose Legacy voice."]
    );
    assert_eq!(clips, 1);
    assert!(calls.text(&id, 1, "Must not become Legacy.").is_err());
    assert!(matches!(
        calls.subscribe(&id).unwrap().0,
        VoiceEvent::State {
            state: VoiceState::Ended,
            ..
        }
    ));
}
