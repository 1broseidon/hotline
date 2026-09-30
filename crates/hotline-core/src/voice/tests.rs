use super::*;
use async_trait::async_trait;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) struct Fake {
    pub transcript: Mutex<String>,
    pub answers: AtomicUsize,
    pub spoken: Mutex<Vec<String>>,
    pub delay: Mutex<Duration>,
    pub fail: bool,
}
impl Default for Fake {
    fn default() -> Self {
        Self {
            transcript: Mutex::new("hello".into()),
            answers: AtomicUsize::new(0),
            spoken: Mutex::new(Vec::new()),
            delay: Mutex::new(Duration::ZERO),
            fail: false,
        }
    }
}
#[async_trait]
impl speech::Speech for Fake {
    fn id(&self) -> SpeechId {
        SpeechId {
            provider_id: "fixture".into(),
            model_id: "fake".into(),
            voice: None,
        }
    }
    async fn transcribe(&self, _: Clip) -> Result<String, speech::SpeechError> {
        Ok(lock(&self.transcript).clone())
    }
    async fn speak(&self, text: &str) -> Result<Clip, speech::SpeechError> {
        lock(&self.spoken).push(text.into());
        let delay = *lock(&self.delay);
        tokio::time::sleep(delay).await;
        if self.fail {
            return Err(speech::SpeechError::Unreachable {
                provider_id: "fixture".into(),
            });
        }
        Ok(Clip {
            mime: "audio/wav".into(),
            bytes: wav(),
        })
    }
}
#[async_trait]
impl Dispatcher for Fake {
    fn id(&self) -> VoiceModel {
        model(speech::Speech::id(self))
    }
    async fn answer(&self, _: Context, _: &str, _: Arc<Budget>) -> Result<String, String> {
        self.answers.fetch_add(1, Ordering::SeqCst);
        Ok("The first sentence. The second sentence.".into())
    }
    async fn narrate(&self, name: &str, text: &str, _: Arc<Budget>) -> Result<String, String> {
        Ok(format!("{name} says: {text}"))
    }
}
pub(crate) fn services() -> Services {
    with_fake(Arc::new(Fake::default()))
}
fn with_fake(fake: Arc<Fake>) -> Services {
    Services {
        speech: SpeechSet {
            stt: fake.clone(),
            tts: fake.clone(),
            fallback_tts: None,
        },
        dispatcher: fake,
    }
}
fn desk(services: Services) -> (tempfile::TempDir, Arc<crate::desk::Desk>, Arc<Calls>) {
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        crate::desk::Desk::open_with_voice_services(
            root.path(),
            Arc::new(crate::credentials::tests::MemoryStore::default()),
            Some(services),
        )
        .unwrap(),
    );
    let calls = desk.voice().unwrap();
    (root, desk, calls)
}
fn wav() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(b"RIFF");
    bytes.extend(32036u32.to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(16000u32.to_le_bytes());
    bytes.extend(32000u32.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(32000u32.to_le_bytes());
    bytes.resize(32044, 0);
    bytes
}
async fn event(
    rx: &mut broadcast::Receiver<VoiceEvent>,
    predicate: impl Fn(&VoiceEvent) -> bool,
) -> VoiceEvent {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let event = rx.recv().await.unwrap();
            if predicate(&event) {
                return event;
            }
        }
    })
    .await
    .unwrap()
}
fn utterance(calls: &Calls, id: &str, seq: u32) -> Result<(), String> {
    calls.utterance(id, seq, "audio/wav", &STANDARD.encode(wav()), 1000)
}

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
async fn goodbye_is_spoken_without_a_model_and_hidden_tape_is_searchable() {
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
    assert!(crate::room::roster(&desk.log).is_empty());
    let room: Arc<dyn RoomHandle> = desk.clone();
    for command in [
        crate::contract::Command::SearchAll {
            query: "goodbye".into(),
            limit: None,
        },
        crate::contract::Command::SearchThread {
            persona_id: TAPE_ID.into(),
            query: "goodbye".into(),
            limit: None,
        },
    ] {
        let found = crate::wire::commands::run(command, &desk.log, &room)
            .await
            .unwrap();
        assert!(
            found.to_string().contains("Goodbye") || found.to_string().contains("goodbye"),
            "{found}"
        );
    }
}

#[tokio::test]
async fn clips_are_sent_in_sentence_order_and_sequences_cannot_replay() {
    let (_root, desk, calls) = desk(services());
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk).unwrap();
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    utterance(&calls, &id, 1).unwrap();
    assert!(utterance(&calls, &id, 1).is_err());
    let first = event(&mut rx, |e| matches!(e, VoiceEvent::Clip { .. })).await;
    let last = event(&mut rx, |e| matches!(e, VoiceEvent::Clip { .. })).await;
    assert!(matches!(
        first,
        VoiceEvent::Clip {
            index: 0,
            r#final: false,
            ..
        }
    ));
    assert!(matches!(
        last,
        VoiceEvent::Clip {
            index: 1,
            r#final: true,
            ..
        }
    ));
    calls.end(&id).unwrap();
    calls.end(&id).unwrap();
}

#[tokio::test]
async fn holding_queues_deliveries_and_refuses_microphone_audio() {
    let (_root, desk, calls) = desk(services());
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk).unwrap();
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    calls.hold(&id, true).unwrap();
    assert!(utterance(&calls, &id, 1).is_err());
    assert!(calls.delivery("mack", "reply", "Mack", "The tests passed."));
    tokio::time::sleep(Duration::from_millis(30)).await;
    while let Ok(event) = rx.try_recv() {
        assert!(!matches!(
            event,
            VoiceEvent::Clip { .. } | VoiceEvent::Said { .. } | VoiceEvent::Delivery { .. }
        ));
    }
    calls.hold(&id, false).unwrap();
    event(
        &mut rx,
        |e| matches!(e, VoiceEvent::Delivery { event_id, .. } if event_id == "reply"),
    )
    .await;
    event(&mut rx, |e| matches!(e, VoiceEvent::Said { .. })).await;
    event(&mut rx, |e| matches!(e, VoiceEvent::Clip { .. })).await;
    calls.end(&id).unwrap();
}

#[tokio::test]
async fn interrupt_cancels_audio_without_ending_the_call() {
    let fake = Arc::new(Fake::default());
    let (_root, desk, calls) = desk(with_fake(fake.clone()));
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk).unwrap();
    // Let the worker start before delaying normal speech.
    tokio::time::sleep(Duration::from_millis(30)).await;
    *lock(&fake.delay) = Duration::from_secs(1);
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    utterance(&calls, &id, 1).unwrap();
    event(&mut rx, |e| matches!(e, VoiceEvent::Said { .. })).await;
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
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk).unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    calls.ledger.charge(Kind::Dispatcher, 3.0);
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
        ..Default::default()
    });
    let fallback = Arc::new(Fake::default());
    let mut services = with_fake(primary.clone());
    services.speech.fallback_tts = Some(fallback.clone());
    let (_root, _desk, calls) = desk(services.clone());
    calls.synthesize(&services.speech, "Test.").await.unwrap();
    assert_eq!(lock(&primary.spoken).len(), 1);
    assert_eq!(lock(&fallback.spoken).len(), 1);
    assert!(
        (calls.status().budget.spent_day_usd - 2.0 * ledger::tts_usd("fixture", 5)).abs() < 1e-9
    );
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
    lock(&calls.calls)
        .iter_mut()
        .find(|c| c.id == second)
        .unwrap()
        .activity = Instant::now() - IDLE;
    calls.expire();
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
    let context = Context {
        log: desk.log.clone(),
        room: desk,
        cancel: CancellationToken::new(),
    };
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
    fn atom(kind: &[u8; 4], payload: Vec<u8>) -> Vec<u8> {
        let mut bytes = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        bytes.extend(kind);
        bytes.extend(payload);
        bytes
    }
    fn mp4(milliseconds: u32) -> Vec<u8> {
        let mut mdhd = vec![0u8; 12];
        mdhd.extend(1000u32.to_be_bytes());
        mdhd.extend(milliseconds.to_be_bytes());
        let mut bytes = atom(b"ftyp", b"M4A ".to_vec());
        bytes.extend(atom(
            b"moov",
            atom(b"trak", atom(b"mdia", atom(b"mdhd", mdhd))),
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
    let generation = lock(&calls.calls).back().unwrap().speech.clone();
    calls.interrupt(&id).unwrap();
    // Model the provider returning between the outer cancellation check and
    // publishing its finished clip. The publication boundary must recheck it.
    calls.clip(
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
