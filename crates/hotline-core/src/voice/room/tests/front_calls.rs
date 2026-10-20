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
            .test_change(&id, |c| Ok(c.speech_services.front_only))
            .unwrap()
    );
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
