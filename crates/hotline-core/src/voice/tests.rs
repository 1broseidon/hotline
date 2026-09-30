use super::*;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub(crate) struct Fake {
    pub transcript: Mutex<String>,
    pub answers: AtomicUsize,
    pub spoken: Mutex<Vec<String>>,
    pub delay: Mutex<Duration>,
    pub fail: bool,
    pub fail_transcription: AtomicBool,
    pub answer_gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
    pub narration: Mutex<Option<Result<String, String>>>,
    pub narrations: AtomicUsize,
}
impl Default for Fake {
    fn default() -> Self {
        Self {
            transcript: Mutex::new("hello".into()),
            answers: AtomicUsize::new(0),
            spoken: Mutex::new(Vec::new()),
            delay: Mutex::new(Duration::ZERO),
            fail: false,
            fail_transcription: AtomicBool::new(false),
            answer_gate: Mutex::new(None),
            narration: Mutex::new(None),
            narrations: AtomicUsize::new(0),
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
    fn accepts(&self) -> &[&str] {
        &["audio/wav", "audio/mp4"]
    }
    async fn transcribe(&self, _: Clip) -> Result<String, speech::SpeechError> {
        if self.fail_transcription.load(Ordering::SeqCst) {
            return Err(speech::SpeechError::Unreachable {
                provider_id: "fixture".into(),
            });
        }
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
        let gate = lock(&self.answer_gate).clone();
        if let Some(gate) = gate {
            gate.acquire().await.unwrap().forget();
        }
        Ok("The first sentence. The second sentence.".into())
    }
    async fn narrate(&self, name: &str, text: &str, _: Arc<Budget>) -> Result<String, String> {
        self.narrations.fetch_add(1, Ordering::SeqCst);
        let gate = lock(&self.answer_gate).clone();
        if let Some(gate) = gate {
            gate.acquire().await.unwrap().forget();
        }
        if let Some(result) = lock(&self.narration).clone() {
            return result;
        }
        Ok(format!("{name} says: {text}"))
    }
}
pub(crate) fn services() -> Services {
    with_fake(Arc::new(Fake::default()))
}
pub(crate) fn with_fake(fake: Arc<Fake>) -> Services {
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
    let mut seen = Vec::new();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let event = rx.recv().await.unwrap();
            if predicate(&event) {
                return event;
            }
            seen.push(event);
        }
    })
    .await
    .unwrap_or_else(|error| panic!("{error}; preceding voice events: {seen:?}"))
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
    for text in [ACK_LINE, "The first sentence.", "The second sentence."] {
        let said = event(&mut rx, |e| matches!(e, VoiceEvent::Said { .. })).await;
        let VoiceEvent::Said {
            id: line,
            text: actual,
        } = said
        else {
            unreachable!()
        };
        assert_eq!(actual, text);
        let clip = event(&mut rx, |e| matches!(e, VoiceEvent::Clip { .. })).await;
        assert!(matches!(clip, VoiceEvent::Clip { id, index: 0, r#final: true, .. } if id == line));
    }
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
    assert!(!calls.delivery("mack", "reply", "Mack", "The tests passed.", true));
    tokio::time::sleep(Duration::from_millis(30)).await;
    while let Ok(event) = rx.try_recv() {
        assert!(!matches!(
            event,
            VoiceEvent::Clip { .. } | VoiceEvent::Said { .. } | VoiceEvent::Delivery { .. }
        ));
    }
    calls.hold(&id, false).unwrap();
    assert!(calls.delivery("mack", "new-reply", "Mack", "The new tests passed.", false));
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
    // Let the worker start before delaying normal speech.
    tokio::time::sleep(Duration::from_millis(30)).await;
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
    let context = Context::new(desk.log.clone(), desk, CancellationToken::new()).for_utterance();
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
    assert!(take_sentences(&mut pending, false).is_empty());
    pending.push_str("rs. Next");
    assert_eq!(take_sentences(&mut pending, false), ["Check main.rs."]);
    assert_eq!(take_sentences(&mut pending, true), ["Next"]);
}

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
async fn acknowledgement_precedes_a_blocked_dispatcher_and_interrupt_preserves_its_text() {
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
        event(
            &mut rx,
            |e| matches!(e, VoiceEvent::Said { text, .. } if text == ACK_LINE),
        )
        .await;
        event(&mut rx, |e| matches!(e, VoiceEvent::Clip { .. })).await;
        assert!(lock(&fake.spoken).is_empty());
        if held {
            calls.hold(&id, true).unwrap();
        } else {
            calls.interrupt(&id).unwrap();
        }
        assert!(utterance(&calls, &id, 2).is_err());
        gate.add_permits(1);
        event(
            &mut rx,
            |e| matches!(e, VoiceEvent::Said { text, .. } if text == "The second sentence."),
        )
        .await;
        tokio::task::yield_now().await;
        assert!(lock(&fake.spoken).is_empty());
        let tape = desk.log.load(&crate::log::StreamId::Tape(TAPE_ID.into()));
        assert!(tape.iter().any(|e| e["text"] == "The second sentence."));
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
            "The tests failed. Check main.rs.",
            false
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
        assert!(!calls.delivery("mack", "empty", "Mack", " \n", true));
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
    let original = calls.change(&id, |c| Ok(c.speech.clone())).unwrap();
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    assert!(calls.delivery("mack", "reply", "Mack", "The tests passed.", false));
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

#[tokio::test]
async fn voice_schedules_require_a_grant_and_cannot_escape_the_call_scope() {
    use crate::contract::Command;
    let (_root, desk, _calls) = desk(services());
    let handle: Arc<dyn RoomHandle> = desk.clone();
    let create: Command = serde_json::from_value(json!({"cmd":"persona.create","params":{"draft":{"name":"Mack","goal":"Test","cwd":desk.log.root().to_str().unwrap()}}})).unwrap();
    let persona = crate::wire::commands::run(create, &desk.log, &handle)
        .await
        .unwrap();
    let id = persona["id"].as_str().unwrap();
    let context = Context::new(desk.log.clone(), handle.clone(), CancellationToken::new());
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
    let other_call =
        Context::new(desk.log.clone(), handle.clone(), CancellationToken::new()).for_utterance();
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
    let operator_job = crate::wire::commands::run(command(params.clone()), &desk.log, &handle)
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
    let persona = crate::wire::commands::run(create, &desk.log, &handle)
        .await
        .unwrap();
    let context = Context::new(desk.log.clone(), handle, CancellationToken::new()).for_utterance();
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
        .change(&id, |c| Ok((c.cancel.clone(), c.deliveries.child_token())))
        .unwrap();
    let context = Context::new(desk.log.clone(), desk, cancel);
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
                    text: "The tests passed.".into(),
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
    assert!(calls.delivery("mack", "reply", "Mack", "The tests passed.", false));
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
        calls.ledger.check().is_ok(),
        "a refused reservation need not have spent the remainder"
    );
}

#[tokio::test]
async fn a_queued_startup_failure_reaches_the_phone_after_hold_or_hangup() {
    for held in [false, true] {
        let fake = Arc::new(Fake::default());
        let (_root, desk, calls) = desk(with_fake(fake.clone()));
        std::fs::write(desk.log.root().join("remote.json"), json!({"desktopId":"desk", "host":"desk.local", "enabled":true, "grants":[{"device":{"id":"phone","name":"Phone","pairedAt":0},"tokenHash":"hash","push":{"token":"ExponentPushToken[phone]","platform":"ios"}}]}).to_string()).unwrap();
        let room = calls.room.upgrade().unwrap();
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
