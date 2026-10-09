use super::*;
use crate::log::StreamId;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub(crate) struct Fake {
    pub transcript: Mutex<String>,
    pub answers: AtomicUsize,
    pub transcriptions: AtomicUsize,
    pub live_chunks: AtomicUsize,
    pub spoken: Mutex<Vec<String>>,
    pub delay: Mutex<Duration>,
    pub fail: bool,
    pub subscription: bool,
    pub output_mime: &'static str,
    pub fail_transcription: AtomicBool,
    pub answer_gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
    pub narration: Mutex<Option<Result<String, String>>>,
    pub narrations: AtomicUsize,
    /// Whether the fake call assistant's model costs nothing.
    pub free_assistant: bool,
}
impl Default for Fake {
    fn default() -> Self {
        Self {
            transcript: Mutex::new("hello".into()),
            answers: AtomicUsize::new(0),
            transcriptions: AtomicUsize::new(0),
            live_chunks: AtomicUsize::new(0),
            spoken: Mutex::new(Vec::new()),
            delay: Mutex::new(Duration::ZERO),
            fail: false,
            subscription: false,
            output_mime: "audio/wav",
            fail_transcription: AtomicBool::new(false),
            answer_gate: Mutex::new(None),
            narration: Mutex::new(None),
            narrations: AtomicUsize::new(0),
            free_assistant: false,
        }
    }
}
#[async_trait]
impl speech::Speech for Fake {
    fn output_mime(&self) -> &str {
        self.output_mime
    }
    fn id(&self) -> SpeechId {
        SpeechId {
            provider_id: if self.subscription {
                "xai-subscription"
            } else {
                "fixture"
            }
            .into(),
            model_id: "fake".into(),
            voice: None,
        }
    }
    fn is_subscription(&self) -> bool {
        self.subscription
    }
    async fn transcribe(&self, _: Clip) -> Result<String, speech::SpeechError> {
        self.transcriptions.fetch_add(1, Ordering::SeqCst);
        if self.fail_transcription.load(Ordering::SeqCst) {
            return Err(speech::SpeechError::Unreachable {
                provider_id: "fixture".into(),
            });
        }
        Ok(lock(&self.transcript).clone())
    }
    fn supports_live_input(&self) -> bool {
        true
    }
    async fn transcribe_live(
        &self,
        mut input: mpsc::Receiver<Vec<u8>>,
        sample_rate: u32,
    ) -> Result<String, speech::SpeechError> {
        assert_eq!(sample_rate, 16_000);
        self.transcriptions.fetch_add(1, Ordering::SeqCst);
        while let Some(bytes) = input.recv().await {
            assert!(!bytes.is_empty());
            self.live_chunks.fetch_add(1, Ordering::SeqCst);
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
    fn free(&self) -> bool {
        self.free_assistant
    }
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
            Arc::new(crate::credentials::tests::MemoryStore::default()),
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

fn spending(desk: &crate::desk::Desk, value: Value) {
    desk.log
        .append(
            &StreamId::Room,
            &crate::room::room_event("setting", json!({"id": "spending", "value": value})),
        )
        .unwrap();
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
    let (_root, desk, calls) = desk(with_fake(fake.clone()));
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
    let error = calls
        .start(&Uuid::new_v4().to_string(), desk.clone())
        .unwrap_err();
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
            .start(&Uuid::new_v4().to_string(), desk.clone())
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
        Arc::new(crate::credentials::tests::MemoryStore::default()),
        None,
    )
    .unwrap();
    let calls = desk.voice().unwrap();
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
            unreachable!()
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
    spending(&desk, json!({"chat": {"dayUsd": 2}}));
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
        .say(
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
            .synthesize(&CallSpeech::audio(services.speech.clone()), "Test.")
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
    lock(&calls.calls)
        .iter_mut()
        .find(|c| c.id == second)
        .unwrap()
        .activity = Instant::now() - Duration::from_millis(crate::thread::QUIET_MS as u64);
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
                .change(&id, |c| Ok(c.first_clip_started.is_some()))
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
        until_written("the answer", || {
            desk.log
                .load(&crate::log::StreamId::Tape(TAPE_ID.into()))
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
    let original = calls.change(&id, |c| Ok(c.speech.clone())).unwrap();
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
        calls.ledger.ready(&[Kind::Tts, Kind::Dispatcher]).is_ok(),
        "a refused reservation need not have spent the remainder"
    );
}

#[tokio::test]
async fn a_queued_startup_failure_reaches_the_phone_after_hold_or_hangup() {
    for held in [false, true] {
        let fake = Arc::new(Fake::default());
        let (_root, desk, calls) = desk(with_fake(fake.clone()));
        std::fs::write(desk.log.root().join("remote.json"), json!({"desktopId":"desk", "host":"desk.local", "enabled":true, "grants":[{"device":{"id":"phone","name":"Phone","pairedAt":0,"publicKey":"AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE"},"push":{"token":"ExponentPushToken[phone]","platform":"ios"}}]}).to_string()).unwrap();
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

#[tokio::test]
async fn live_audio_cannot_dispatch_before_commit_or_after_cancel() {
    let fake = Arc::new(Fake::default());
    let (_root, desk, calls) = desk(with_fake(fake.clone()));
    let id = Uuid::new_v4().to_string();
    let call = calls.start_target(&id, None, true, desk).unwrap();
    assert!(call.input.contains(&"audio/pcm".into()));
    assert!(call.input.contains(&"audio/mp4".into()));
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    let data = STANDARD.encode(vec![0; 6_400]);
    calls.audio(&id, 1, 0, &data, false).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while fake.live_chunks.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fake.answers.load(Ordering::SeqCst), 0);
    assert!(
        rx.try_recv().is_err(),
        "partial audio is not a committed transcript"
    );
    assert!(calls.audio(&id, 1, 2, &data, false).is_err());
    assert!(
        calls
            .audio(&id, 1, 1, &STANDARD.encode([0]), false)
            .is_err()
    );
    assert!(
        calls
            .audio(&id, 1, 1, &STANDARD.encode(vec![0; 32_770]), false)
            .is_err()
    );
    calls.interrupt(&id).unwrap();
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
    assert!(calls.audio(&id, 1, 1, "", true).is_err());
    assert_eq!(fake.answers.load(Ordering::SeqCst), 0);
    assert!(calls.audio(&id, 2, 0, "", true).is_err());
    calls.audio(&id, 2, 0, &data, false).unwrap();
    calls.audio(&id, 2, 1, "", true).unwrap();
    event(&mut rx, |e| matches!(e, VoiceEvent::Heard { seq: 2, .. })).await;
    event(&mut rx, |e| matches!(e, VoiceEvent::Clip { .. })).await;
    assert_eq!(fake.answers.load(Ordering::SeqCst), 1);
    calls.end(&id).unwrap();
}

#[tokio::test]
async fn direct_call_filters_unrelated_future_and_duplicate_replies() {
    let fake = Arc::new(Fake::default());
    let (_root, desk, calls) = desk(with_fake(fake.clone()));
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk).unwrap();
    calls
        .change(&id, |call| {
            call.target = Some("ada".into());
            call.seq = Some(4);
            Ok(())
        })
        .unwrap();
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    let origin = Origin {
        call_id: id.clone(),
        seq: 4,
        direct: true,
    };
    assert!(!calls.delivery("ada", "unrelated", "Ada", "Background work.", false, None));
    assert!(!calls.delivery(
        "mack",
        "other",
        "Mack",
        "Another reply.",
        true,
        Some(&origin)
    ));
    let unsaid = Origin {
        seq: 5,
        ..origin.clone()
    };
    assert!(!calls.delivery("ada", "ahead", "Ada", "Not asked yet.", true, Some(&unsaid)));
    let old_call = Origin {
        call_id: Uuid::new_v4().to_string(),
        ..origin.clone()
    };
    assert!(!calls.delivery("ada", "old-call", "Ada", "Old call.", true, Some(&old_call)));
    // A reply to an earlier turn of this call is the call's too, and is said once.
    let earlier = Origin {
        seq: 3,
        ..origin.clone()
    };
    assert!(calls.delivery("ada", "ack", "Ada", "On it.", true, Some(&earlier)));
    assert!(calls.delivery("ada", "ack", "Ada", "On it.", true, Some(&origin)));
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
    assert_eq!(*lock(&fake.spoken), ["On it."]);
    // The teammate answered itself: no call assistant was asked anything.
    assert_eq!(fake.narrations.load(Ordering::SeqCst), 0);
    assert_eq!(fake.answers.load(Ordering::SeqCst), 0);
    calls.card(
        "mack",
        &json!({"kind":"human_action","status":"pending","actionId":"other"}),
    );
    assert!(rx.try_recv().is_err());
    calls.card(
        "ada",
        &json!({"kind":"human_action","status":"pending","actionId":"mine"}),
    );
    assert!(
        matches!(rx.try_recv().unwrap(), VoiceEvent::Card { persona_id, .. } if persona_id == "ada")
    );
    calls.end(&id).unwrap();
}

#[tokio::test]
async fn short_plain_replies_skip_the_narration_model() {
    let fake = Arc::new(Fake::default());
    let (_root, desk, calls) = desk(with_fake(fake.clone()));
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk).unwrap();
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    assert!(calls.delivery("ada", "reply", "Ada", "The checks passed.", false, None));
    event(
        &mut rx,
        |e| matches!(e, VoiceEvent::Delivery { text, .. } if text == "The checks passed."),
    )
    .await;
    event(&mut rx, |e| matches!(e, VoiceEvent::Clip { .. })).await;
    assert_eq!(fake.narrations.load(Ordering::SeqCst), 0);
    calls.end(&id).unwrap();
}

struct BrokenChunks {
    invalid_first: bool,
}
#[async_trait]
impl speech::Speech for BrokenChunks {
    fn id(&self) -> SpeechId {
        speech::Speech::id(&Fake::default())
    }
    async fn transcribe(&self, _: Clip) -> Result<String, speech::SpeechError> {
        Ok("hello".into())
    }
    async fn speak(&self, _: &str) -> Result<Clip, speech::SpeechError> {
        unreachable!("streaming path")
    }
    async fn speak_chunks(
        &self,
        _: &str,
        output: mpsc::Sender<speech::SpeechChunk>,
    ) -> Result<(), speech::SpeechError> {
        for _ in 0..12 {
            let chunk = speech::SpeechChunk {
                clip: Clip {
                    mime: if self.invalid_first {
                        "audio/garbage"
                    } else {
                        "audio/wav"
                    }
                    .into(),
                    bytes: wav(),
                },
                final_chunk: false,
            };
            output
                .send(chunk)
                .await
                .map_err(|_| speech::SpeechError::Cancelled)?;
            if !self.invalid_first {
                return Err(speech::SpeechError::Malformed {
                    provider_id: "fixture".into(),
                });
            }
        }
        Ok(())
    }
}

#[tokio::test]
async fn streaming_failure_falls_back_only_before_audio_and_drops_rejected_producers() {
    for invalid_first in [true, false] {
        let fallback = Arc::new(Fake::default());
        let mut services = with_fake(fallback.clone());
        services.speech.tts = Arc::new(BrokenChunks { invalid_first });
        services.speech.fallback_tts = Some(fallback.clone());
        let (_root, desk, calls) = desk(services);
        let id = Uuid::new_v4().to_string();
        calls.start_target(&id, None, true, desk).unwrap();
        let (_, mut rx) = calls.subscribe(&id).unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            calls.say(
                &id,
                "A short reply.",
                &CancellationToken::new(),
                &CancellationToken::new(),
            ),
        )
        .await
        .expect("a rejected consumer must drop its receiver promptly");
        assert_eq!(result.is_ok(), invalid_first);
        assert_eq!(lock(&fallback.spoken).len(), usize::from(invalid_first));
        let clip = event(&mut rx, |e| matches!(e, VoiceEvent::Clip { .. })).await;
        assert!(
            matches!(clip, VoiceEvent::Clip { index: 0, r#final, .. } if r#final == invalid_first)
        );
        calls.end(&id).unwrap();
    }
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

/// The teammate Mack, made through the room's own command.
async fn mack(desk: &Arc<crate::desk::Desk>) -> String {
    use crate::contract::Command;
    let handle: Arc<dyn RoomHandle> = desk.clone();
    let create: Command = serde_json::from_value(json!({"cmd":"persona.create","params":{"draft":{"name":"Mack","goal":"Keep the build green","cwd":desk.log.root().to_str().unwrap()}}})).unwrap();
    crate::wire::commands::run(create, &desk.log, &handle)
        .await
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// A direct call that is a thread from the start: the teammate Mack, and a
/// call to it that names Mack as its target when it begins.
async fn direct_call() -> (
    tempfile::TempDir,
    Arc<crate::desk::Desk>,
    Arc<Calls>,
    String,
    String,
    broadcast::Receiver<VoiceEvent>,
    Arc<Fake>,
) {
    let fake = Arc::new(Fake::default());
    *lock(&fake.transcript) = "Can you check the build?".into();
    let (root, desk, calls) = desk(with_fake(fake.clone()));
    let persona = mack(&desk).await;
    let id = Uuid::new_v4().to_string();
    calls
        .start_target(&id, Some(persona.clone()), false, desk.clone())
        .unwrap();
    let (_, rx) = calls.subscribe(&id).unwrap();
    (root, desk, calls, id, persona, rx, fake)
}

/// The call has heard turn `seq`, as if the person had said it: its replies
/// are this call's.
fn on_turn(calls: &Calls, id: &str, seq: u32) -> Origin {
    calls
        .change(id, |call| {
            call.seq = Some(seq);
            Ok(())
        })
        .unwrap();
    Origin {
        call_id: id.into(),
        seq,
        direct: true,
    }
}

/// Every `said` and `clip` of a reply until its final clip.
async fn one_reply(
    rx: &mut broadcast::Receiver<VoiceEvent>,
) -> (Vec<(String, String)>, Vec<(String, u32, bool)>) {
    let mut said = Vec::new();
    let mut clips = Vec::new();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match rx.recv().await.unwrap() {
                VoiceEvent::Said { id, text } => said.push((id, text)),
                VoiceEvent::Clip {
                    id,
                    index,
                    r#final,
                    data,
                    ..
                } => {
                    clips.push((id, index, data.is_empty()));
                    if r#final {
                        return;
                    }
                }
                VoiceEvent::Delivery { .. } => panic!("a teammate's own reply is no delivery"),
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    (said, clips)
}

async fn until_written(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..1000 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("{what} was never written");
}

fn said_on(desk: &crate::desk::Desk, id: &str) -> Vec<(String, String)> {
    desk.log
        .load(&StreamId::Call(id.into()))
        .into_iter()
        .filter(|event| matches!(event["kind"].as_str(), Some("user" | "agent")))
        .map(|event| {
            (
                event["kind"].as_str().unwrap().to_string(),
                event["text"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn call_link(desk: &crate::desk::Desk, persona: &str) -> Option<Value> {
    desk.log
        .load(&StreamId::Tape(persona.into()))
        .into_iter()
        .find(|event| event["kind"] == "link" && event["threadKind"] == "call")
}

#[tokio::test]
async fn a_direct_calls_lines_are_kept_on_its_thread_and_found_by_search() {
    let (_root, desk, calls, id, persona, mut rx, _fake) = direct_call().await;
    utterance(&calls, &id, 1).unwrap();
    // This desk has no model for Mack, so the words never reach a session.
    event(
        &mut rx,
        |e| matches!(e, VoiceEvent::Said { text, .. } if text == RETRY_LINE),
    )
    .await;
    let origin = on_turn(&calls, &id, 1);
    assert!(calls.delivery(
        &persona,
        "reply-1",
        "Mack",
        "What's on your mind? <<<ENDSPEAK>>> Shown only.",
        true,
        Some(&origin)
    ));
    event(
        &mut rx,
        |e| matches!(e, VoiceEvent::Said { text, .. } if text == "What's on your mind?"),
    )
    .await;
    until_written("the call's lines", || said_on(&desk, &id).len() == 3).await;

    assert_eq!(
        said_on(&desk, &id),
        [
            ("user".to_string(), "Can you check the build?".to_string()),
            ("agent".to_string(), RETRY_LINE.to_string()),
            ("agent".to_string(), "What's on your mind?".to_string()),
        ]
    );
    // The DM shows the call, once, and the call stream heads with the same line.
    until_written("the link", || call_link(&desk, &persona).is_some()).await;
    let link = call_link(&desk, &persona).unwrap();
    assert_eq!(link["thread"], id.as_str());
    assert_eq!(link["state"], "live");
    assert_eq!(link["personaId"], persona.as_str());

    // The teammate finds what was said, naming the call.
    let found =
        crate::store::search::search_teammate(desk.log.root(), &persona, "build", None).unwrap();
    assert_eq!(found["hits"][0]["thread"], format!("call:{id}"));
    let heard =
        crate::store::search::search_teammate(desk.log.root(), &persona, "mind", None).unwrap();
    assert_eq!(heard["hits"][0]["thread"], format!("call:{id}"));

    calls.end(&id).unwrap();
    until_written("the closing link", || {
        call_link(&desk, &persona).is_some_and(|link| link["state"] == "closed")
    })
    .await;
    let link = call_link(&desk, &persona).unwrap();
    assert_eq!(link["end"], "person");
    assert_eq!(link["outcome"], "Hung up");
    assert_eq!(
        desk.log
            .load(&StreamId::Tape(persona.clone()))
            .iter()
            .filter(|event| event["kind"] == "link" && event["threadKind"] == "call")
            .count(),
        1,
        "one line, rewritten as the call goes"
    );

    // A closed call reads back as a thread.
    let thread = crate::thread::ThreadStore::new(&desk.log)
        .load(&crate::thread::ThreadId::call(&id))
        .unwrap();
    assert_eq!(
        thread.state,
        crate::thread::ThreadState::Closed(crate::thread::End::Person)
    );
    assert_eq!(
        thread.participants,
        [
            crate::thread::Participant::Person,
            crate::thread::Participant::Voice(persona.clone())
        ]
    );
    assert!(
        crate::thread::ThreadStore::new(&desk.log)
            .list(&persona)
            .iter()
            .any(|listed| listed.id == thread.id)
    );
}

/// A teammate's reply on a call to it is said as it streams, up to the
/// marker, as one reply: one line on screen growing under one id, its audio
/// under that id ending in one empty final clip, and one line on the call's
/// thread. Nothing after the marker, and never the marker, reaches speech.
#[tokio::test]
async fn a_streamed_reply_is_said_up_to_its_marker_as_one_line() {
    let (_root, desk, calls, id, persona, mut rx, fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    let chunks = [
        "The build is still red. It's the fla",
        "ky config test again. <<<END",
        "SPEAK>>>\n| test | result |\n| config | flaky |\n",
    ];
    for chunk in chunks {
        calls.reply_delta(&persona, "reply-1", chunk, &origin);
    }
    let whole = chunks.concat();
    assert!(calls.delivery(&persona, "reply-1", "Mack", &whole, true, Some(&origin)));
    let (said, clips) = one_reply(&mut rx).await;
    let line = &said[0].0;
    assert!(said.iter().all(|(id, _)| id == line), "{said:?}");
    let spoken = "The build is still red. It's the flaky config test again.";
    assert_eq!(
        said.iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>(),
        ["The build is still red.", spoken]
    );
    assert!(clips.iter().all(|(id, _, _)| id == line));
    assert_eq!(
        clips
            .iter()
            .map(|(_, index, empty)| (*index, *empty))
            .collect::<Vec<_>>(),
        [(0, false), (1, false), (2, true)]
    );
    assert_eq!(
        *lock(&fake.spoken),
        [
            "The build is still red.",
            "It's the flaky config test again."
        ]
    );
    until_written("the reply", || said_on(&desk, &id).len() == 1).await;
    assert_eq!(
        said_on(&desk, &id),
        [("agent".to_string(), spoken.to_string())]
    );
    let stored = desk.log.load(&StreamId::Call(id.clone()));
    assert!(stored.iter().any(|event| event["id"] == line.as_str()));
    // The turn's end hands the same reply over again: it is not said twice.
    assert!(calls.delivery(&persona, "reply-1", "Mack", &whole, true, Some(&origin)));
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(lock(&fake.spoken).len(), 2);
    calls.end(&id).unwrap();
}

/// An agent that writes no marker wrote its reply to be read: the call says
/// its opening, up to its first code block, and the chat has the rest.
#[tokio::test]
async fn a_reply_without_the_marker_is_said_up_to_its_first_code_block() {
    let (_root, _desk, calls, id, persona, mut rx, fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    let reply = "Here's the fix for the flaky test:\n```rust\nassert!(ready);\n```\nIt passes ten runs in a row now.";
    assert!(calls.delivery(&persona, "reply-1", "Mack", reply, true, Some(&origin)));
    let (said, _) = one_reply(&mut rx).await;
    assert_eq!(said.last().unwrap().1, "Here's the fix for the flaky test:");
    assert_eq!(*lock(&fake.spoken), ["Here's the fix for the flaky test:"]);
    calls.end(&id).unwrap();
}

/// Speaking over a teammate stops what it is saying and gives the person the
/// floor, without stopping its turn: what it goes on writing of that reply is
/// not said, and the call takes the person's next words at once.
#[tokio::test]
async fn speaking_over_a_reply_stops_it_and_gives_the_person_the_floor() {
    let (_root, _desk, calls, id, persona, mut rx, fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    calls
        .change(&id, |call| {
            call.answering = Some(1);
            Ok(())
        })
        .unwrap();
    *lock(&fake.delay) = Duration::from_secs(5);
    calls.reply_delta(&persona, "reply-1", "I looked at the logs. ", &origin);
    event(
        &mut rx,
        |e| matches!(e, VoiceEvent::Said { text, .. } if text == "I looked at the logs."),
    )
    .await;
    calls.interrupt(&id).unwrap();
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
    calls.reply_delta(
        &persona,
        "reply-1",
        "The cache is stale. <<<ENDSPEAK>>>",
        &origin,
    );
    assert!(calls.delivery(
        &persona,
        "reply-1",
        "Mack",
        "I looked at the logs. The cache is stale. <<<ENDSPEAK>>>",
        true,
        Some(&origin)
    ));
    tokio::time::sleep(Duration::from_millis(50)).await;
    while let Ok(event) = rx.try_recv() {
        assert!(
            !matches!(&event, VoiceEvent::Said { text, .. } if text.contains("stale")),
            "{event:?}"
        );
        assert!(!matches!(event, VoiceEvent::Clip { .. }), "{event:?}");
    }
    *lock(&fake.delay) = Duration::ZERO;
    utterance(&calls, &id, 2).unwrap();
    calls.end(&id).unwrap();
}

/// The call thinks while the teammate's turn is open, between what it says,
/// and listens once the session has finished the turn. A turn that ends
/// before a later one the person said does not end the wait for that one.
#[tokio::test]
async fn the_call_thinks_while_the_teammate_works_and_listens_when_its_turn_ends() {
    let (_root, _desk, calls, id, persona, mut rx, _fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    calls
        .change(&id, |call| {
            call.answering = Some(1);
            call.state(VoiceState::Thinking, None);
            Ok(())
        })
        .unwrap();
    assert!(calls.delivery(
        &persona,
        "ack",
        "Mack",
        "On it. <<<ENDSPEAK>>>",
        true,
        Some(&origin)
    ));
    one_reply(&mut rx).await;
    event(&mut rx, |e| matches!(e, VoiceEvent::State { .. })).await;
    assert_eq!(
        calls.subscribe(&id).unwrap().0,
        VoiceEvent::State {
            state: VoiceState::Thinking,
            reason: None
        },
        "still working"
    );
    calls
        .change(&id, |call| {
            call.seq = Some(2);
            call.answering = Some(2);
            Ok(())
        })
        .unwrap();
    calls.turn_ended(&persona, &origin);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(
        calls.change(&id, |call| Ok(call.answering)).unwrap(),
        Some(2)
    );
    let later = Origin { seq: 2, ..origin };
    calls.turn_ended(&persona, &later);
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
    calls.end(&id).unwrap();
}

/// A reply its turn never finished, because the agent stopped mid-message,
/// is said as far as it got, and the call listens.
#[tokio::test]
async fn a_reply_the_turn_never_finished_is_said_as_far_as_it_got() {
    let (_root, _desk, calls, id, persona, mut rx, _fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    calls.reply_delta(
        &persona,
        "reply-1",
        "The deploy is half done. The rest",
        &origin,
    );
    event(
        &mut rx,
        |e| matches!(e, VoiceEvent::Said { text, .. } if text == "The deploy is half done."),
    )
    .await;
    calls.turn_ended(&persona, &origin);
    let (said, _) = one_reply(&mut rx).await;
    assert_eq!(said.last().unwrap().1, "The deploy is half done. The rest");
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
    calls.end(&id).unwrap();
}

/// A call that thinks says so again while it waits, so a phone does not take
/// a teammate's long work for a desk that went quiet.
#[tokio::test]
async fn a_call_that_thinks_says_so_again() {
    let (_root, _desk, calls, id, _persona, mut rx, _fake) = direct_call().await;
    calls
        .change(&id, |call| {
            call.answering = Some(1);
            call.state(VoiceState::Thinking, None);
            Ok(())
        })
        .unwrap();
    event(&mut rx, |e| matches!(e, VoiceEvent::State { .. })).await;
    let again = tokio::time::Instant::now();
    assert_eq!(
        event(&mut rx, |e| matches!(e, VoiceEvent::State { .. })).await,
        VoiceEvent::State {
            state: VoiceState::Thinking,
            reason: None
        }
    );
    assert!(again.elapsed() <= THINKING_AGAIN * 2);
    calls.end(&id).unwrap();
}

/// A hold cuts off a reply being said, and the phone hears of it as of any
/// reply that comes while the call is held.
#[tokio::test]
async fn a_hold_cuts_off_a_reply_and_leaves_it_to_the_phone() {
    let (_root, _desk, calls, id, persona, mut rx, _fake) = direct_call().await;
    let origin = on_turn(&calls, &id, 1);
    calls.reply_delta(&persona, "reply-1", "The deploy went out. ", &origin);
    event(&mut rx, |e| matches!(e, VoiceEvent::Said { .. })).await;
    calls.hold(&id, true).unwrap();
    calls.reply_delta(
        &persona,
        "reply-1",
        "Nothing broke. <<<ENDSPEAK>>>",
        &origin,
    );
    assert!(!calls.delivery(
        &persona,
        "reply-1",
        "Mack",
        "The deploy went out. Nothing broke. <<<ENDSPEAK>>>",
        true,
        Some(&origin)
    ));
    calls.end(&id).unwrap();
}

/// A call to a teammate has no call assistant, so a Chat budget that is off
/// cannot refuse it; a desk call, which has one, is refused.
#[tokio::test]
async fn a_direct_call_pays_for_no_call_assistant() {
    let free_speech = Arc::new(Fake {
        subscription: true,
        ..Fake::default()
    });
    let (_root, desk, calls) = desk(with_fake(free_speech));
    let persona = mack(&desk).await;
    spending(&desk, json!({"chat": {"dayUsd": 0}}));
    let status = calls.status();
    assert!(status.direct_available);
    assert!(!status.available);
    let id = Uuid::new_v4().to_string();
    calls
        .start_target(&id, Some(persona), false, desk.clone())
        .unwrap();
    assert!(calls.ready_for(&id).is_ok());
    calls.end(&id).unwrap();
}

#[test]
fn a_link_is_said_as_its_site_and_shown_in_full() {
    assert_eq!(
        speakable(
            "Here it is: https://ketch.run (GitHub repo: https://github.com/1broseidon/ketch)"
        ),
        "Here it is: ketch.run (GitHub repo: github.com)"
    );
    assert_eq!(
        speakable("See www.example.com/docs?x=1."),
        "See example.com."
    );
    assert_eq!(
        speakable("Check main.rs in v1.2."),
        "Check main.rs in v1.2."
    );
    assert!(!speech_ready("Here it is: https://ketch.run"));
}

/// A desk call's answer, streamed in sentences, is one line on the
/// dispatcher's tape too.
#[tokio::test]
async fn a_desk_calls_streamed_answer_is_one_line_on_its_tape() {
    let (_root, desk, calls) = desk(services());
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk.clone()).unwrap();
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    utterance(&calls, &id, 1).unwrap();
    event(&mut rx, |e| {
        matches!(e, VoiceEvent::Clip { r#final: true, .. })
    })
    .await;
    let agent = || -> Vec<String> {
        desk.log
            .load(&StreamId::Tape(TAPE_ID.into()))
            .into_iter()
            .filter(|event| event["kind"] == "agent")
            .map(|event| event["text"].as_str().unwrap().to_string())
            .collect()
    };
    until_written("the answer", || !agent().is_empty()).await;
    assert_eq!(agent(), ["The first sentence. The second sentence."]);
    calls.end(&id).unwrap();
}

#[tokio::test]
async fn the_room_sweep_ends_a_call_that_has_gone_quiet_and_leaves_one_that_has_not() {
    let (_root, desk, calls, id, persona, _rx, _fake) = direct_call().await;
    let room = calls.room.upgrade().unwrap();
    let mut looked_again = std::collections::HashMap::new();
    until_written("the link", || call_link(&desk, &persona).is_some()).await;

    room.sweep(
        crate::session::now_ms() + crate::thread::QUIET_MS - 60_000,
        &mut looked_again,
    )
    .await;
    assert_eq!(
        calls.subscribe(&id).unwrap().0,
        VoiceEvent::State {
            state: VoiceState::Listening,
            reason: None
        },
        "not quiet for long enough"
    );

    room.sweep(
        crate::session::now_ms() + crate::thread::QUIET_MS + 60_000,
        &mut looked_again,
    )
    .await;
    assert_eq!(
        calls.subscribe(&id).unwrap().0,
        VoiceEvent::State {
            state: VoiceState::Ended,
            reason: Some(VoiceEndReason::Idle)
        }
    );
    until_written("the closing link", || {
        call_link(&desk, &persona).is_some_and(|link| link["state"] == "closed")
    })
    .await;
    let link = call_link(&desk, &persona).unwrap();
    assert_eq!(link["end"], "idle");
    assert_eq!(link["outcome"], "Went quiet");
}

#[tokio::test]
async fn a_desk_call_is_kept_on_the_dispatchers_tape_and_is_no_thread() {
    let (_root, desk, calls) = desk(services());
    let id = Uuid::new_v4().to_string();
    calls.start(&id, desk.clone()).unwrap();
    utterance(&calls, &id, 1).unwrap();
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    event(&mut rx, |e| matches!(e, VoiceEvent::Said { .. })).await;
    assert!(
        desk.log.load(&StreamId::Call(id.clone())).is_empty(),
        "no thread, and so no call stream"
    );
    assert!(!desk.log.load(&StreamId::Tape(TAPE_ID.into())).is_empty());
    calls.end(&id).unwrap();
}

#[test]
fn a_direct_calls_turn_names_its_call_thread_and_a_desk_calls_does_not() {
    let direct = Origin {
        call_id: Uuid::new_v4().to_string(),
        seq: 3,
        direct: true,
    };
    let from = direct.from().unwrap();
    assert_eq!(from.kind, crate::thread::ThreadKind::Call);
    assert_eq!(from.thread, direct.call_id);
    assert_eq!(from.request.as_deref(), Some("3"));
    assert_eq!(Origin::from_delivery(&from), Some(direct));
    let desk = Origin {
        direct: false,
        ..Origin {
            call_id: Uuid::new_v4().to_string(),
            seq: 1,
            direct: false,
        }
    };
    assert_eq!(desk.from(), None, "a desk call is no thread");
    let pair = crate::contract::DeliveryFrom::new(&crate::thread::ThreadId::pair("a\u{1f}b"), None);
    assert_eq!(Origin::from_delivery(&pair), None);
}
