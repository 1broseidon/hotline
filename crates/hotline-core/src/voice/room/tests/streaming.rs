use super::*;

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
    let persona = mack(&desk).await;
    calls
        .start_target(&id, Some(persona.clone()), false, desk)
        .unwrap();
    calls
        .test_change(&id, |call| {
            call.seq = Some(4);
            Ok(())
        })
        .unwrap();
    let (_, mut rx) = calls.subscribe(&id).unwrap();
    let origin = Origin {
        call_id: id.clone(),
        seq: 4,
        direct: true,
        front: false,
    };
    assert!(!calls.delivery(
        &persona,
        "unrelated",
        "Ada",
        "Background work.",
        false,
        None
    ));
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
    assert!(!calls.delivery(
        &persona,
        "ahead",
        "Ada",
        "Not asked yet.",
        true,
        Some(&unsaid)
    ));
    let old_call = Origin {
        call_id: Uuid::new_v4().to_string(),
        ..origin.clone()
    };
    assert!(!calls.delivery(
        &persona,
        "old-call",
        "Ada",
        "Old call.",
        true,
        Some(&old_call)
    ));
    // A reply to an earlier turn of this call is the call's too, and is said once.
    let earlier = Origin {
        seq: 3,
        ..origin.clone()
    };
    assert!(calls.delivery(&persona, "ack", "Ada", "On it.", true, Some(&earlier)));
    assert!(calls.delivery(&persona, "ack", "Ada", "On it.", true, Some(&origin)));
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
        &persona,
        &json!({"kind":"human_action","status":"pending","actionId":"mine"}),
    );
    assert!(
        matches!(rx.try_recv().unwrap(), VoiceEvent::Card { persona_id, .. } if persona_id == persona)
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
            calls.test_say(
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
