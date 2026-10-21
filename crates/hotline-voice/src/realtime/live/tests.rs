use super::*;

/// The login in `token_dir/auth.json` as it stands, for the calls here: the
/// refresh is the core's ChatGPT provider's, so a real call wants a login
/// signed in within the hour.
fn signed_in(token_dir: &Path) -> BoxFuture<'_, Result<Login, String>> {
    Box::pin(async move {
        let bytes =
            std::fs::read(token_dir.join("auth.json")).map_err(|error| error.to_string())?;
        let auth: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        Ok(Login {
            access_token: auth["access_token"].as_str().map(String::from),
            account_id: auth["account_id"].as_str().map(String::from),
        })
    })
}

fn pcm(samples: &[i16]) -> String {
    let bytes: Vec<u8> = samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect();
    STANDARD.encode(bytes)
}

#[test]
fn the_call_id_comes_from_the_location_it_was_created_at() {
    assert_eq!(
        call_id("/v1/realtime/calls/rtc_u0_EZKQTp3oNDvVE80TJkXjkmjc3czDNn9A").as_deref(),
        Some("rtc_u0_EZKQTp3oNDvVE80TJkXjkmjc3czDNn9A")
    );
    assert_eq!(
        call_id("/v1/realtime/calls/calls/rtc_backend?x=1").as_deref(),
        Some("rtc_backend")
    );
    assert_eq!(
        call_id("/v1/live/2f1c4a3e-9b7d-4c2a-8e1f-0a1b2c3d4e5f").as_deref(),
        Some("2f1c4a3e-9b7d-4c2a-8e1f-0a1b2c3d4e5f")
    );
    assert_eq!(call_id("/v1/realtime/calls/"), None);
    assert_eq!(call_id("/v1/realtime/calls/rtc_"), None);
    // Nothing that could leave the path segment it is joined into.
    assert_eq!(call_id("/calls/rtc_a%2F..%2Fb"), None);
}

#[test]
fn a_line_to_say_goes_in_pieces_the_sideband_takes() {
    let long = "é".repeat(400);
    let pieces = context_pieces(&long);
    assert!(pieces.iter().all(|piece| piece.len() <= CONTEXT_BYTES));
    assert_eq!(pieces.concat(), long);
    assert_eq!(context_pieces("Hi."), ["Hi."]);
}

#[test]
fn a_hand_off_is_the_send_the_bridge_answers_in_the_persons_words() {
    let mut heard = Heard::default();
    let events = heard
        .event(&json!({
            "type": "delegation.created",
            "item": {"id": "item_1", "type": "delegation", "target": "client",
                     "content": [{"type": "input_text", "text": "check whether the build passed"}]},
        }))
        .unwrap();
    assert_eq!(
        events,
        [RealtimeEvent::ToolCall {
            call_id: "item_1".into(),
            name: SEND_TO_TEAMMATE.into(),
            arguments: json!({"words": "check whether the build passed"}).to_string(),
        }]
    );
    // A delegation to the server's own backend is not the desk's to answer.
    let server = json!({"type": "delegation.created", "item": {"id": "x", "target": "responses", "content": []}});
    assert_eq!(heard.event(&server).unwrap(), [] as [RealtimeEvent; 0]);
}

#[test]
fn the_model_speaks_in_turns_and_its_silence_between_them_is_dropped() {
    let mut heard = Heard::default();
    let quiet = json!({"type": "session.output_audio.delta", "delta": pcm(&[0, 3, -2])});
    assert!(
        heard.event(&quiet).unwrap().is_empty(),
        "silence between turns"
    );
    let loud = json!({"type": "session.output_audio.delta", "delta": pcm(&[4000, -4000])});
    assert!(matches!(
        heard.event(&loud).unwrap()[..],
        [RealtimeEvent::Audio(_)]
    ));
    assert_eq!(
        heard
            .event(&json!({"type": "output_transcript.added", "item": {"text": " Hi"}}))
            .unwrap(),
        [RealtimeEvent::Saying(" Hi".into())]
    );
    // Inside a reply a pause is part of it.
    assert!(matches!(
        heard.event(&quiet).unwrap()[..],
        [RealtimeEvent::Audio(_)]
    ));
    assert_eq!(
        heard
            .event(
                &json!({"type": "turn.done", "turn": {"role": "assistant", "transcript": " Hi."}})
            )
            .unwrap(),
        [RealtimeEvent::ResponseDone]
    );
    assert_eq!(heard.event(&quiet).unwrap(), [] as [RealtimeEvent; 0]);
}

#[test]
fn the_person_talking_cuts_the_model_once_and_their_turn_is_heard() {
    let mut heard = Heard::default();
    let started = json!({"type": "turn.created", "turn": {"role": "user", "transcript": " Hey"}});
    assert_eq!(
        heard.event(&started).unwrap(),
        [RealtimeEvent::SpeechStarted]
    );
    let more = json!({"type": "input_transcript.added", "item": {"text": " can you"}});
    assert!(heard.event(&more).unwrap().is_empty(), "one cut per turn");
    assert_eq!(
        heard
            .event(&json!({"type": "turn.done", "turn": {"role": "user", "transcript": " Hey, can you check"}}))
            .unwrap(),
        [RealtimeEvent::Heard("Hey, can you check".into())]
    );
    assert_eq!(heard.event(&more).unwrap(), [RealtimeEvent::SpeechStarted]);
}

#[test]
fn errors_reach_the_caller_as_a_code_never_a_message() {
    let mut heard = Heard::default();
    let error = json!({"type": "error", "error": {"code": "rate_limited", "message": "the person said secret"}});
    assert_eq!(
        heard.event(&error).unwrap(),
        [RealtimeEvent::Refused {
            code: Some("rate_limited".into())
        }]
    );
    let odd = json!({"type": "error", "error": {"code": "has spaces and words"}});
    assert_eq!(
        heard.event(&odd).unwrap(),
        [RealtimeEvent::Refused { code: None }]
    );
    assert!(
        heard
            .event(&json!({"type": "session.output_audio.delta", "delta": "AAA"}))
            .is_err()
    );
    assert!(heard.event(&json!({"no": "type"})).is_err());
}

#[test]
fn the_microphone_is_doubled_to_48_khz_paced_in_20_ms_and_never_lags() {
    let mut mic = Mic::default();
    assert_eq!(mic.packet(), [0; PACKET_SAMPLES], "silence when quiet");
    let frame: Vec<u8> = (0..480i16)
        .flat_map(|sample| (sample * 10).to_le_bytes())
        .collect();
    mic.push(&frame);
    let packet = mic.packet();
    assert_eq!(packet[1], 0);
    assert_eq!(packet[3], 10);
    assert_eq!(packet[2], 5, "interpolated between samples");
    assert!(mic.samples.is_empty());
    for _ in 0..200 {
        mic.push(&frame);
    }
    assert_eq!(mic.samples.len(), MIC_BACKLOG);
}

#[tokio::test]
async fn rates_other_than_24_khz_fail_before_any_call() {
    let live = GptLive::new(std::env::temp_dir().join("hotline-no-login"), signed_in);
    let config = RealtimeConfig {
        instructions: String::new(),
        voice: "cove".into(),
        tools: Vec::new(),
        input_rate: 16_000,
        output_rate: 24_000,
    };
    assert!(matches!(
        live.open(config).await.err().unwrap(),
        SpeechError::UnsupportedFormat(_)
    ));
    assert!(live.is_subscription());
    assert_eq!(live.id().provider_id, PROVIDER_ID);
}

/// A real call on a ChatGPT login: `HOTLINE_TEST_CHATGPT_DIR` is a login
/// directory holding `auth.json`, and `HOTLINE_TEST_ASK_WAV` a 24 kHz mono
/// PCM16 WAV of someone asking for something. It says a line, then hears the
/// ask and hands it over, and says what the desk answers it with.
#[tokio::test]
#[ignore = "places a real GPT-Live call on a ChatGPT login"]
async fn a_live_call_says_a_line_hands_over_what_it_hears_and_says_the_answer() {
    let dir = std::env::var("HOTLINE_TEST_CHATGPT_DIR").expect("a ChatGPT login directory");
    let wav = std::fs::read(std::env::var("HOTLINE_TEST_ASK_WAV").expect("a 24 kHz clip")).unwrap();
    let live = GptLive::new(dir.into(), signed_in);
    let mut session = live
        .open(RealtimeConfig {
            instructions: "You are a teammate on a call. Acknowledge a request in a few words and delegate it.".into(),
            voice: "cove".into(),
            tools: Vec::new(),
            input_rate: RATE,
            output_rate: RATE,
        })
        .await
        .unwrap();
    session.input.say("Hi George, I'm here.").await.unwrap();
    let mut saying = String::new();
    let mut audio = 0;
    let mut handed = None;
    let silence = vec![0u8; 1920];
    let ask = wav[44..].to_vec();
    let started = tokio::time::Instant::now();
    let mut sent = 0;
    while started.elapsed() < Duration::from_secs(30) && handed.is_none() {
        // 40 ms frames in real time: silence, the ask after 3 s, then silence.
        let at = started.elapsed().as_millis() as usize;
        while sent * 40 < at {
            let offset = (sent * 1920).saturating_sub(75 * 1920);
            let frame = if sent >= 75 && offset + 1920 <= ask.len() {
                ask[offset..offset + 1920].to_vec()
            } else {
                silence.clone()
            };
            session.input.push_audio(frame).await.unwrap();
            sent += 1;
        }
        if let Ok(Some(event)) = timeout(Duration::from_millis(20), session.events.recv()).await {
            match event.unwrap() {
                RealtimeEvent::Saying(piece) => saying.push_str(&piece),
                RealtimeEvent::Audio(pcm) => audio += pcm.len(),
                RealtimeEvent::ToolCall {
                    call_id,
                    name,
                    arguments,
                } => handed = Some((call_id, name, arguments)),
                _ => {}
            }
        }
    }
    // The speakable channel is said in the model's own words, not verbatim.
    assert!(!saying.trim().is_empty(), "the line was said");
    assert!(
        audio > 24_000,
        "a second of speech came back: {audio} bytes"
    );
    let (call_id, name, arguments) = handed.expect("the ask was handed over");
    assert_eq!(name, SEND_TO_TEAMMATE);
    assert!(arguments.contains("laptop"), "{arguments}");

    // What the desk found for the delegation is said.
    session
        .input
        .tool_result(&call_id, "The laptop ships on Thursday.", true)
        .await
        .unwrap();
    let mut answered = String::new();
    let asked = tokio::time::Instant::now();
    while asked.elapsed() < Duration::from_secs(20) && !answered.contains("Thursday") {
        let at = started.elapsed().as_millis() as usize;
        while sent * 40 < at {
            session.input.push_audio(silence.clone()).await.unwrap();
            sent += 1;
        }
        if let Ok(Some(event)) = timeout(Duration::from_millis(20), session.events.recv()).await
            && let RealtimeEvent::Saying(piece) = event.unwrap()
        {
            answered.push_str(&piece);
        }
    }
    assert!(answered.contains("Thursday"), "{answered}");
    session.close().await;
}

/// The same real call with its session renewed after 15 s: the replacement
/// call takes over between replies, and speaks.
#[tokio::test]
#[ignore = "places two real GPT-Live calls on a ChatGPT login"]
async fn a_live_call_moves_onto_a_replacement_before_the_session_ends() {
    let dir = std::env::var("HOTLINE_TEST_CHATGPT_DIR").expect("a ChatGPT login directory");
    let mut live = GptLive::new(dir.into(), signed_in);
    live.renew_after = Duration::from_secs(15);
    let mut session = live
        .open(RealtimeConfig {
            instructions: "You are a teammate on a call. Keep replies short.".into(),
            voice: "cove".into(),
            tools: Vec::new(),
            input_rate: RATE,
            output_rate: RATE,
        })
        .await
        .unwrap();
    session.input.say("Hi George, I'm here.").await.unwrap();
    let silence = vec![0u8; 1920];
    let started = tokio::time::Instant::now();
    let mut sent = 0;
    let mut renewed = None;
    let mut said_after = String::new();
    while started.elapsed() < Duration::from_secs(45) && said_after.is_empty() {
        while sent * 40 < started.elapsed().as_millis() as usize {
            session.input.push_audio(silence.clone()).await.unwrap();
            sent += 1;
        }
        if let Ok(Some(event)) = timeout(Duration::from_millis(20), session.events.recv()).await {
            match event.unwrap() {
                RealtimeEvent::Renewed if renewed.is_none() => {
                    renewed = Some(started.elapsed());
                    session
                        .input
                        .say("Still here on the new line.")
                        .await
                        .unwrap();
                }
                RealtimeEvent::Saying(piece) if renewed.is_some() => said_after.push_str(&piece),
                _ => {}
            }
        }
    }
    let renewed = renewed.expect("the call moved onto a replacement");
    assert!(renewed >= Duration::from_secs(15), "{renewed:?}");
    assert!(!said_after.trim().is_empty(), "the replacement spoke");
    session.close().await;
}

#[test]
fn the_chatgpt_login_never_requests_the_rejected_responses_backend() {
    let session = session_settings("Use your tools to check the build.", "cove");
    assert_eq!(session["delegation"], json!({"type":"client"}));
    assert!(session.get("tools").is_none());
    assert_eq!(session["model"], MODEL);
}
