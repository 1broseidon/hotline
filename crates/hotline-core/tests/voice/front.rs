use super::*;

#[tokio::test]
async fn a_front_call_carries_pcm_both_ways_and_truncates_on_barge_in() {
    let mut desk = front_desk(true, 64).await;
    let socket = &mut desk.socket;
    let status = answer(socket, 1, json!({"cmd":"voice.status","params":{}})).await;
    assert!(
        status["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == "voiceFront")
    );

    let call = Uuid::new_v4().to_string();
    let started = answer(
        socket,
        2,
        json!({"cmd":"voice.call_start","params":{"callId":call,"mode":"front","inputRate":16000}}),
    )
    .await;
    assert_eq!(
        started["result"]["front"],
        json!({"inputRate":16000,"outputRate":24000})
    );
    // The front is told what the desk sends and takes: 24 kHz both ways.
    send(socket, json!({"id":3,"sub":{"call":call}})).await;
    frame_where(socket, |f| f["sub"] == 3 && f["snapshot"].is_array()).await;

    // Microphone PCM at 16 kHz reaches the front at 24 kHz.
    for _ in 0..50 {
        let sent = answer(socket, 4, json!({"cmd":"voice.front_audio","params":{"callId":call,"seq":0,"data":pcm(&[0, 3000, 6000, 9000])}})).await;
        if sent["ok"] == true {
            break;
        }
        assert_eq!(sent["error"], "The front is still connecting.");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let Asked::Audio(heard) = next_asked(&mut desk.asked).await else {
        panic!("the front was not handed audio")
    };
    assert_eq!(heard.len(), 12, "four samples at 16 kHz are six at 24 kHz");
    let config = desk.front.config.lock().unwrap().clone().unwrap();
    assert_eq!((config.input_rate, config.output_rate), (24_000, 24_000));

    // Sequence numbers only rise.
    let repeated = answer(
        socket,
        5,
        json!({"cmd":"voice.front_audio","params":{"callId":call,"seq":0,"data":pcm(&[1,2])}}),
    )
    .await;
    assert_eq!(repeated["ok"], false);

    // The front's speech comes back in small numbered frames.
    desk.events
        .send(Ok(RealtimeEvent::Audio(vec![1; 2 * 4_800 + 100])))
        .await
        .unwrap();
    // The call is speaking before the first frame arrives.
    let speaking = frame_where(socket, |f| f["event"]["type"] == "state").await;
    assert_eq!(speaking["event"]["state"], "speaking");
    let first = frame_where(socket, |f| f["event"]["type"] == "frontAudio").await;
    assert_eq!(first["event"]["seq"], 0);
    assert_eq!(
        STANDARD
            .decode(first["event"]["data"].as_str().unwrap())
            .unwrap()
            .len(),
        4_800
    );
    let third = frame_where(socket, |f| {
        f["event"]["type"] == "frontAudio" && f["event"]["seq"] == 2
    })
    .await;
    assert_eq!(
        STANDARD
            .decode(third["event"]["data"].as_str().unwrap())
            .unwrap()
            .len(),
        100
    );

    // The provider hears the person over the front: stop playing, then say
    // how much was heard, and the front keeps only that.
    desk.events
        .send(Ok(RealtimeEvent::SpeechStarted))
        .await
        .unwrap();
    frame_where(socket, |f| f["event"]["type"] == "frontStop").await;
    let listening = frame_where(socket, |f| f["event"]["type"] == "state").await;
    assert_eq!(listening["event"]["state"], "listening");
    let barged = answer(
        socket,
        6,
        json!({"cmd":"voice.front_barge","params":{"callId":call,"playedMs":250}}),
    )
    .await;
    assert_eq!(barged["ok"], true, "{barged}");
    assert_eq!(
        next_asked(&mut desk.asked).await,
        Asked::Truncate { played_ms: 250 }
    );

    // What each side said reaches the call card.
    desk.events
        .send(Ok(RealtimeEvent::Heard("check the PR".into())))
        .await
        .unwrap();
    assert_eq!(
        frame_where(socket, |f| f["event"]["type"] == "heard").await["event"]["text"],
        "check the PR"
    );
    desk.events
        .send(Ok(RealtimeEvent::Saying("On it.".into())))
        .await
        .unwrap();
    desk.events
        .send(Ok(RealtimeEvent::ResponseDone))
        .await
        .unwrap();
    assert_eq!(
        frame_where(socket, |f| f["event"]["type"] == "said").await["event"]["text"],
        "On it."
    );

    // A front call takes front audio only: the pipeline's inputs are shut.
    let utterance = answer(
        socket,
        7,
        json!({"cmd":"voice.text","params":{"callId":call,"seq":1,"text":"hi"}}),
    )
    .await;
    assert_eq!(utterance["ok"], false);
    let ended = answer(
        socket,
        8,
        json!({"cmd":"voice.call_end","params":{"callId":call}}),
    )
    .await;
    assert_eq!(ended["ok"], true);
    // Ending the call closes the front's session.
    tokio::time::timeout(Duration::from_secs(5), async {
        while desk.asked.recv().await.is_some() {}
    })
    .await
    .expect("the front outlived its call");
}

#[tokio::test]
async fn a_desk_without_a_front_refuses_live_and_only_starts_legacy_explicitly() {
    let mut desk = front_desk(false, 64).await;
    let socket = &mut desk.socket;
    let setting = answer(
        socket,
        1,
        json!({"cmd":"settings.update","params":{"patch":{"voice":{"front":{"provider":"grok"}}}}}),
    )
    .await;
    assert_eq!(setting["ok"], true, "{setting}");
    let status = answer(socket, 2, json!({"cmd":"voice.status","params":{}})).await;
    assert_eq!(status["result"]["liveSelected"], true);
    assert_eq!(status["result"]["available"], false);
    for mode in [None, Some("front")] {
        let call = Uuid::new_v4().to_string();
        let mut params = json!({"callId":call});
        if let Some(mode) = mode {
            params["mode"] = json!(mode);
        }
        let started = answer(socket, 3, json!({"cmd":"voice.call_start","params":params})).await;
        assert_eq!(started["ok"], false, "{started}");
        assert!(started["error"].as_str().unwrap().contains("xAI"));
    }
    let call = Uuid::new_v4().to_string();
    let started = answer(
        socket,
        4,
        json!({"cmd":"voice.call_start","params":{"callId":call,"mode":"pipeline"}}),
    )
    .await;
    assert_eq!(started["ok"], true, "{started}");
    assert!(started["result"].get("front").is_none());
}

#[tokio::test]
async fn a_front_that_cannot_open_ends_visibly_without_accepting_legacy_input() {
    let mut desk = front_desk(true, 64).await;
    desk.front.session.lock().unwrap().take();
    let call = Uuid::new_v4().to_string();
    let started = answer(
        &mut desk.socket,
        1,
        json!({"cmd":"voice.call_start","params":{"callId":call}}),
    )
    .await;
    assert_eq!(started["result"]["front"]["outputRate"], 24000);
    send(&mut desk.socket, json!({"id":101,"sub":{"call":call}})).await;
    let ended = frame_where(&mut desk.socket, |f| {
        f["event"]["liveError"].is_string() || f["snapshot"][0]["liveError"].is_string()
    })
    .await;
    let state = if ended["event"].is_object() {
        &ended["event"]
    } else {
        &ended["snapshot"][0]
    };
    assert_eq!(state["state"], "ended");
    assert!(
        state["liveError"]
            .as_str()
            .unwrap()
            .contains("could not start")
    );
    let text = answer(
        &mut desk.socket,
        2,
        json!({"cmd":"voice.text","params":{"callId":call,"seq":1,"text":"hi"}}),
    )
    .await;
    assert_eq!(text["ok"], false);
    assert!(text["error"].as_str().unwrap().contains("ended"));
}

#[tokio::test]
async fn a_dropped_live_call_reconnects_with_its_context_and_keeps_its_audio_mode() {
    let mut desk = front_desk(true, 64).await;
    let call = Uuid::new_v4().to_string();
    answer(
        &mut desk.socket,
        1,
        json!({"cmd":"voice.call_start","params":{"callId":call}}),
    )
    .await;
    send(&mut desk.socket, json!({"id":101,"sub":{"call":call}})).await;
    frame_where(&mut desk.socket, |f| f["snapshot"].is_array()).await;
    desk.events
        .send(Ok(RealtimeEvent::Heard("Keep the current design.".into())))
        .await
        .unwrap();
    frame_where(&mut desk.socket, |f| f["event"]["type"] == "heard").await;
    let (replacement, mut asked, events) = RealtimeSession::scripted(64);
    *desk.front.session.lock().unwrap() = Some(replacement);
    desk.events
        .send(Err(SpeechError::Unreachable {
            provider_id: "test".into(),
        }))
        .await
        .unwrap();
    frame_where(&mut desk.socket, |f| f["event"]["reconnecting"] == true).await;
    frame_where(&mut desk.socket, |f| {
        f["event"]["type"] == "state" && f["event"]["reconnecting"] != true
    })
    .await;
    assert_eq!(
        tokio::time::timeout(common::ANSWER, asked.recv())
            .await
            .unwrap(),
        Some(Asked::Say("We're back on the same call.".into()))
    );
    events
        .send(Ok(RealtimeEvent::Heard("Are you back?".into())))
        .await
        .unwrap();
    let heard = frame_where(&mut desk.socket, |f| f["event"]["text"] == "Are you back?").await;
    assert_eq!(heard["event"]["seq"], 1);
    assert!(
        desk.front
            .config
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .instructions
            .contains("Keep the current design.")
    );
    let text = answer(
        &mut desk.socket,
        2,
        json!({"cmd":"voice.text","params":{"callId":call,"seq":1,"text":"hi"}}),
    )
    .await;
    assert_eq!(
        text["error"],
        "This call has a voice front. Send front audio."
    );
    answer(
        &mut desk.socket,
        3,
        json!({"cmd":"voice.call_end","params":{"callId":call}}),
    )
    .await;
}

#[tokio::test]
async fn a_dropped_live_call_that_cannot_reconnect_ends_once_with_a_persistent_reason() {
    let mut desk = front_desk(true, 64).await;
    let call = Uuid::new_v4().to_string();
    answer(
        &mut desk.socket,
        1,
        json!({"cmd":"voice.call_start","params":{"callId":call}}),
    )
    .await;
    send(&mut desk.socket, json!({"id":101,"sub":{"call":call}})).await;
    frame_where(&mut desk.socket, |f| f["snapshot"].is_array()).await;
    desk.events
        .send(Ok(RealtimeEvent::Heard("Keep the current design.".into())))
        .await
        .unwrap();
    frame_where(&mut desk.socket, |f| f["event"]["type"] == "heard").await;
    drop(desk.events);
    let notice = frame_where(&mut desk.socket, |f| f["event"]["type"] == "said").await;
    assert_eq!(
        notice["event"]["text"],
        "Live voice is unavailable. You can try again or choose Legacy voice."
    );
    let clip = frame_where(&mut desk.socket, |f| f["event"]["type"] == "clip").await;
    assert_eq!(clip["event"]["mimeType"], "audio/wav");
    let ended = frame_where(&mut desk.socket, |f| f["event"]["state"] == "ended").await;
    assert!(
        ended["event"]["liveError"]
            .as_str()
            .unwrap()
            .contains("could not reconnect")
    );
    send(&mut desk.socket, json!({"id":102,"sub":{"call":call}})).await;
    let snapshot = frame_where(&mut desk.socket, |f| {
        f["sub"] == 102 && f["snapshot"].is_array()
    })
    .await;
    assert_eq!(snapshot["snapshot"][0], ended["event"]);
    let text = answer(
        &mut desk.socket,
        2,
        json!({"cmd":"voice.text","params":{"callId":call,"seq":1,"text":"hi"}}),
    )
    .await;
    assert_eq!(text["ok"], false);
}

#[tokio::test]
async fn a_front_that_is_behind_refuses_audio_instead_of_stalling_the_wire() {
    let mut desk = front_desk(true, 2).await;
    let socket = &mut desk.socket;
    let call = Uuid::new_v4().to_string();
    answer(
        socket,
        1,
        json!({"cmd":"voice.call_start","params":{"callId":call,"mode":"front"}}),
    )
    .await;
    let mut refused = None;
    for seq in 0..20u32 {
        let sent = answer(socket, 10 + seq, json!({"cmd":"voice.front_audio","params":{"callId":call,"seq":seq,"data":pcm(&[1,2])}})).await;
        if sent["ok"] == false && sent["error"] != "The front is still connecting." {
            refused = Some(sent);
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let refused = refused.expect("a full front queue refuses audio");
    assert_eq!(
        refused["error"],
        "The front is behind. Drop this audio and send the next."
    );
    // The same socket still answers.
    let status = answer(socket, 99, json!({"cmd":"voice.status","params":{}})).await;
    assert_eq!(status["ok"], true);
}

/// The front wired end to end, against a scripted provider: it opens with
/// the teammate's prompt and the bridge's two tools; its `work_on_it`
/// hands the words to the teammate's own session unrephrased, as a typed
/// message would go (its open chapter, the call's origin on the line), told
/// they are a transcript, and is answered; the call
/// thinks with `listening` while the turn works; the teammate's spoken line
/// comes back to the front to say verbatim, and the desk synthesizes none of
/// it; `remain_silent` asks for no reply; and the call's thread keeps what
/// the front heard and said, each once.
#[tokio::test]
async fn a_front_call_works_on_its_thread_and_says_its_spoken_line() {
    let mut desk = front_desk(true, 64).await;
    let root = desk.root.path().to_path_buf();
    let socket = &mut desk.socket;
    let (provider, requests) = provider_seeing(TAGGED_REPLY).await;
    let saved = answer(socket, 90, json!({"cmd":"credential.custom_save","params":{"draft":{"name":"Front fixture","baseUrl":provider,"api":"chat_completions","models":["test"],"secret":"fixture-key"}}})).await;
    assert_eq!(saved["ok"], true);
    let persona = answer(socket, 91, json!({"cmd":"persona.create","params":{"draft":{"name":"Mack","goal":"Check the PR.","cwd":root.to_str().unwrap()}}})).await["result"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let started = answer(
        socket,
        92,
        json!({"cmd":"session.start","params":{"personaId":persona}}),
    )
    .await;
    assert_eq!(started["ok"], true);

    // The desk says which front calls run on and its voices; the teammate
    // picks its own voice from them, and the front speaks in it.
    let status = answer(socket, 93, json!({"cmd":"voice.status","params":{}})).await;
    let front = &status["result"]["front"];
    assert_eq!(front["providerId"], "grok", "{status}");
    assert_eq!(front["voice"], "eve");
    assert!(front["voices"].as_array().unwrap().contains(&json!("rex")));
    let picked = answer(
        socket,
        94,
        json!({"cmd":"persona.update","params":{"id":persona,"patch":{"voice":{"providerId":"grok","modelId":front["modelId"],"voice":"rex"}}}}),
    )
    .await;
    assert_eq!(picked["ok"], true, "{picked}");

    let call = Uuid::new_v4().to_string();
    let started = answer(
        socket,
        1,
        json!({"cmd":"voice.call_start","params":{"callId":call,"personaId":persona}}),
    )
    .await;
    assert_eq!(started["result"]["front"]["outputRate"], 24000, "{started}");
    send(socket, json!({"id":2,"sub":{"call":call}})).await;
    frame_where(socket, |f| f["sub"] == 2 && f["snapshot"].is_array()).await;

    // The front opens as the teammate's voice, with the bridge's tools.
    common::until("the front is opened", || {
        desk.front.config.lock().unwrap().is_some()
    })
    .await;
    let config = desk.front.config.lock().unwrap().clone().unwrap();
    assert!(config.instructions.contains("You are Mack."));
    assert!(config.instructions.contains("first person"));
    let tools: Vec<&str> = config.tools.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(
        tools,
        [
            "work_on_it",
            "remain_silent",
            "search_thread",
            "read_thread",
            "list_chapters",
            "list_schedules",
            "web_search"
        ],
        "the bridge's two, the teammate's desk-side reads and the post to the chat"
    );
    assert_eq!(config.voice, "rex", "the teammate's own voice");

    // The person asks, and the front hands it over in their words.
    let words = "check the failing PR from monday";
    desk.events
        .send(Ok(RealtimeEvent::Heard(words.into())))
        .await
        .unwrap();
    frame_where(socket, |f| f["event"]["type"] == "heard").await;
    desk.events
        .send(Ok(RealtimeEvent::ToolCall {
            call_id: "c1".into(),
            name: "work_on_it".into(),
            arguments: json!({ "words": words }).to_string(),
        }))
        .await
        .unwrap();
    let Asked::ToolResult {
        call_id,
        output,
        reply,
    } = next_asked(&mut desk.asked).await
    else {
        panic!("the front's tool call went unanswered")
    };
    assert_eq!(call_id, "c1");
    assert!(output.contains("Nothing is done yet"), "{output}");
    assert!(
        output.contains("Work accepted on this call") && !output.contains("Handed to"),
        "a direct call hands over as the teammate: {output}"
    );
    assert!(
        !reply,
        "the front acknowledged it as it sent it; no second word"
    );

    // The call thinks with the floor open while the teammate works, then
    // the teammate's spoken line goes to the front to be said, and the desk
    // speaks none of it itself.
    let mut listening = false;
    let said = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            tokio::select! {
                frame = socket.next() => {
                    let frame: Value = serde_json::from_str(
                        frame.unwrap().unwrap().to_text().unwrap(),
                    )
                    .unwrap();
                    assert_ne!(frame["event"]["type"], "clip", "the desk spoke: {frame}");
                    if frame["event"]["type"] == "state"
                        && frame["event"]["state"] == "thinking"
                        && frame["event"]["listening"] == true
                    {
                        listening = true;
                    }
                }
                asked = desk.asked.recv() => {
                    if let Some(Asked::Say(text)) = asked {
                        return text;
                    }
                }
            }
        }
    })
    .await
    .expect("the teammate's line reached the front");
    assert!(listening, "the call showed the teammate working");
    assert_eq!(said, "The checks passed.");

    // A front that relays the line it was given back to the teammate is
    // refused, or the teammate would answer its own reply, round and round.
    desk.events
        .send(Ok(RealtimeEvent::ToolCall {
            call_id: "c2".into(),
            name: "work_on_it".into(),
            arguments: json!({ "words": said }).to_string(),
        }))
        .await
        .unwrap();
    let Asked::ToolResult {
        call_id,
        output,
        reply,
    } = next_asked(&mut desk.asked).await
    else {
        panic!("the echo went unanswered")
    };
    assert_eq!(call_id, "c2");
    assert!(output.starts_with("Work could not start"), "{output}");
    assert!(reply, "a refusal is spoken to");

    // The front says it; that is the call's line, kept once.
    desk.events
        .send(Ok(RealtimeEvent::Saying("The checks passed.".into())))
        .await
        .unwrap();
    desk.events
        .send(Ok(RealtimeEvent::ResponseDone))
        .await
        .unwrap();
    let line = frame_where(socket, |f| {
        f["event"]["type"] == "said" && f["event"]["id"] == "front-0"
    })
    .await;
    assert_eq!(line["event"]["text"], "The checks passed.");

    // Choosing silence is answered, and asks the front for nothing.
    desk.events
        .send(Ok(RealtimeEvent::ToolCall {
            call_id: "c2".into(),
            name: "remain_silent".into(),
            arguments: "{}".into(),
        }))
        .await
        .unwrap();
    assert_eq!(
        next_asked(&mut desk.asked).await,
        Asked::ToolResult {
            call_id: "c2".into(),
            output: String::new(),
            reply: false
        }
    );

    let ended = answer(
        socket,
        3,
        json!({"cmd":"voice.call_end","params":{"callId":call}}),
    )
    .await;
    assert_eq!(ended["ok"], true);
    let log = &desk.log;
    let record = hotline_core::log::StreamId::Call(call.clone());
    common::until("the front's line reaches the call's record", || {
        log.load(&record)
            .iter()
            .any(|v| v["text"] == "The checks passed.")
    })
    .await;
    let record = log.load(&record);
    let texts: Vec<&str> = record.iter().filter_map(|v| v["text"].as_str()).collect();
    assert_eq!(
        texts.iter().filter(|text| **text == words).count(),
        1,
        "{record:?}"
    );
    assert_eq!(
        texts
            .iter()
            .filter(|text| **text == "The checks passed.")
            .count(),
        1,
        "{record:?}"
    );

    let requests = requests.lock().unwrap();
    assert!(
        requests
            .iter()
            .any(|request| request.to_string().contains("The whole call so far"))
    );
    let tape = log.load(&hotline_core::log::StreamId::Tape(persona));
    assert!(
        !tape.iter().any(|v| v["kind"] == "user"),
        "home thread was interrupted: {tape:?}"
    );
    let tools: Vec<_> = record.iter().filter(|v| v["kind"] == "tool").collect();
    assert!(tools.iter().any(|v| v["title"] == "work_on_it"));
    for line in &record {
        if line["kind"] == "tool" {
            serde_json::from_value::<hotline_core::contract::TranscriptEvent>(line.clone())
                .unwrap();
        }
    }
}

/// A front on an API key costs its minutes, on a subscription login nothing,
/// and a call is refused before it starts when the budget has no room.
#[tokio::test]
async fn a_fronts_minutes_are_counted_by_login_kind() {
    use async_trait::async_trait;
    use hotline_voice::realtime::{Realtime, RealtimeConfig, RealtimeSession};
    use hotline_voice::speech::{SpeechError, SpeechId};

    struct Front {
        provider: &'static str,
        subscription: bool,
    }
    #[async_trait]
    impl Realtime for Front {
        fn id(&self) -> SpeechId {
            SpeechId {
                provider_id: self.provider.into(),
                model_id: "fixture".into(),
                voice: None,
            }
        }
        fn is_subscription(&self) -> bool {
            self.subscription
        }
        async fn open(&self, _: RealtimeConfig) -> Result<RealtimeSession, SpeechError> {
            Err(SpeechError::NothingToSay)
        }
    }

    let root = tempfile::tempdir().unwrap();
    let desk = hotline_core::desk::Desk::open_with_voice_services(
        root.path(),
        common::store(),
        Some(services()),
    )
    .unwrap();
    let calls = desk.parts().voice.unwrap();
    let grok = Front {
        provider: "xai",
        subscription: false,
    };
    let gpt = Front {
        provider: "openai",
        subscription: false,
    };
    let login = Front {
        provider: "xai-subscription",
        subscription: true,
    };

    calls.front_ready(&grok).unwrap();
    calls.front_spend(&login, 600.0).unwrap();
    assert_eq!(calls.status().budget.spent_day_usd, 0.0);
    calls.front_spend(&grok, 60.0).unwrap();
    calls.front_spend(&gpt, 60.0).unwrap();
    let spent = calls.status().budget.spent_day_usd;
    assert!((spent - 0.18).abs() < 1e-9, "{spent}");
}

/// The PCM16 samples of a WAV file's `data` chunk.
fn wav_pcm(wav: &[u8]) -> Vec<u8> {
    let mut at = 12;
    while at + 8 <= wav.len() {
        let size = u32::from_le_bytes(wav[at + 4..at + 8].try_into().unwrap()) as usize;
        if &wav[at..at + 4] == b"data" {
            return wav[at + 8..(at + 8 + size).min(wav.len())].to_vec();
        }
        at += 8 + size + size % 2;
    }
    panic!("a WAV with no data chunk")
}

/// Sends `pcm` as a phone would, 100 ms of 16 kHz at a time and in time.
async fn speak_into(socket: &mut Socket, call: &str, seq: &mut u32, pcm: &[u8]) {
    for frame in pcm.chunks(3_200) {
        let id = 1_000 + *seq;
        send(socket, json!({"id":id,"cmd":"voice.front_audio","params":{"callId":call,"seq":*seq,"data":STANDARD.encode(frame)}})).await;
        *seq += 1;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The wired front against Grok Voice itself, with a scripted teammate:
/// the person's 16 kHz speech is heard and transcribed, the front calls
/// `work_on_it`, the teammate's spoken line comes back as 24 kHz
/// speech, and speaking over it is a barge-in. Needs an xAI API key in
/// `HOTLINE_LIVE_XAI_KEY` and is charged by the minute;
/// `cargo test -p hotline-core --test voice live -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "talks to Grok Voice with HOTLINE_LIVE_XAI_KEY"]
async fn live_a_grok_front_relays_to_the_teammate_and_yields_the_floor() {
    use hotline_voice::realtime::RealtimeSocket;
    use hotline_voice::speech::Endpoint;

    let key = std::env::var("HOTLINE_LIVE_XAI_KEY").expect("set HOTLINE_LIVE_XAI_KEY");
    let root = tempfile::tempdir().unwrap();
    let mut services = services();
    services.realtime = Some(Arc::new(RealtimeSocket::grok(Endpoint {
        provider_id: "xai".into(),
        base_url: "https://api.x.ai/v1".into(),
        key: Some(key),
    })));
    let desk = Arc::new(
        hotline_core::desk::Desk::open_with_voice_services(
            root.path(),
            common::store(),
            Some(services),
        )
        .unwrap(),
    );
    let log = desk.log.clone();
    let parts = desk.parts();
    let door = Door::bind(log.clone(), "front-test".into(), desk, parts).unwrap();
    let port = door.port();
    tokio::spawn(door.run());
    let (mut socket, _) = connect_async(format!("ws://127.0.0.1:{port}/ws?token=front-test"))
        .await
        .unwrap();
    let socket = &mut socket;
    let provider = provider(TAGGED_REPLY).await;
    answer(socket, 90, json!({"cmd":"credential.custom_save","params":{"draft":{"name":"Front fixture","baseUrl":provider,"api":"chat_completions","models":["test"],"secret":"fixture-key"}}})).await;
    let persona = answer(socket, 91, json!({"cmd":"persona.create","params":{"draft":{"name":"Mack","goal":"Check the PR.","cwd":root.path().to_str().unwrap()}}})).await["result"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    answer(
        socket,
        92,
        json!({"cmd":"session.start","params":{"personaId":persona}}),
    )
    .await;

    let call = Uuid::new_v4().to_string();
    let started = answer(
        socket,
        1,
        json!({"cmd":"voice.call_start","params":{"callId":call,"personaId":persona,"mode":"front","inputRate":16000}}),
    )
    .await;
    assert_eq!(
        started["result"]["front"],
        json!({"inputRate":16000,"outputRate":24000})
    );
    send(socket, json!({"id":2,"sub":{"call":call}})).await;
    frame_where(socket, |f| f["sub"] == 2 && f["snapshot"].is_array()).await;
    let mut seq = 0;
    for attempt in 0..100 {
        let sent = answer(socket, 3, json!({"cmd":"voice.front_audio","params":{"callId":call,"seq":seq,"data":STANDARD.encode([0u8; 320])}})).await;
        seq += 1;
        if sent["ok"] == true {
            break;
        }
        if attempt < 3 {
            eprintln!("front not ready: {sent}");
        }
        assert!(attempt < 99, "the front never connected: {sent}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let speech = wav_pcm(&wav());
    let silence = vec![0u8; 32_000 * 2];
    speak_into(socket, &call, &mut seq, &speech).await;
    speak_into(socket, &call, &mut seq, &silence).await;

    // Heard, relayed, answered and said, within a minute.
    let mut heard = None;
    let mut frames = 0;
    let mut longest = 0;
    let mut said = Vec::new();
    let relayed = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let frame = frame_where(socket, |f| f["event"].is_object()).await;
            let event = &frame["event"];
            match event["type"].as_str().unwrap() {
                "heard" => heard = Some(event["text"].as_str().unwrap().to_string()),
                "frontAudio" => {
                    frames += 1;
                    let bytes = STANDARD.decode(event["data"].as_str().unwrap()).unwrap();
                    longest = longest.max(bytes.len());
                }
                "said" => {
                    let text = event["text"].as_str().unwrap().to_string();
                    eprintln!("said {}: {text}", event["id"]);
                    let done = event["id"].as_str().unwrap().starts_with("front-")
                        && text.contains("checks passed");
                    said.push(text);
                    if done {
                        return;
                    }
                }
                _ => {}
            }
        }
    })
    .await;
    let tape = log.load(&hotline_core::log::StreamId::Tape(persona.clone()));
    let handed: Vec<&str> = tape
        .iter()
        .filter(|v| v["kind"] == "user")
        .filter_map(|v| v["text"].as_str())
        .collect();
    eprintln!(
        "heard: {heard:?}\nhanded to the teammate: {handed:?}\nsaid: {said:?}\nfrontAudio frames: {frames}"
    );
    let heard = heard.expect("the person's speech was transcribed");
    assert!(heard.to_lowercase().contains("mac"), "{heard}");
    assert!(!handed.is_empty(), "the front called work_on_it");
    assert!(frames > 0, "the front spoke");
    assert_eq!(longest, 4_800, "100 ms frames of 24 kHz PCM16");
    relayed.expect("the teammate's spoken line was said by the front");

    // The person talks over the front: it stops and yields the floor.
    speak_into(socket, &call, &mut seq, &speech).await;
    frame_where(socket, |f| f["event"]["type"] == "frontStop").await;
    let ended = answer(
        socket,
        5,
        json!({"cmd":"voice.call_end","params":{"callId":call}}),
    )
    .await;
    assert_eq!(ended["ok"], true);
}

#[tokio::test]
async fn a_selected_paid_front_with_no_budget_never_starts_legacy() {
    struct PaidFront;
    #[async_trait::async_trait]
    impl Realtime for PaidFront {
        fn is_subscription(&self) -> bool {
            false
        }
        fn id(&self) -> SpeechId {
            SpeechId {
                provider_id: "xai".into(),
                model_id: "fixture".into(),
                voice: None,
            }
        }
        async fn open(&self, _: RealtimeConfig) -> Result<RealtimeSession, SpeechError> {
            Err(SpeechError::Unreachable {
                provider_id: "xai".into(),
            })
        }
    }
    let root = tempfile::tempdir().unwrap();
    let mut services = services();
    services.realtime = Some(Arc::new(PaidFront));
    let desk = Arc::new(
        hotline_core::desk::Desk::open_with_voice_services(
            root.path(),
            common::store(),
            Some(services),
        )
        .unwrap(),
    );
    let parts = desk.parts();
    let door = Door::bind(desk.log.clone(), "live-budget-test".into(), desk, parts).unwrap();
    let port = door.port();
    let server = tokio::spawn(door.run());
    let (mut socket, _) = connect_async(format!("ws://127.0.0.1:{port}/ws?token=live-budget-test"))
        .await
        .unwrap();
    let settings = answer(&mut socket, 2, json!({"cmd":"settings.update","params":{"patch":{"voice":{"front":{"provider":"grok"},"dayUsd":0}}}})).await;
    assert_eq!(settings["ok"], true);
    let status = answer(&mut socket, 3, json!({"cmd":"voice.status","params":{}})).await;
    assert_eq!(status["result"]["liveSelected"], true);
    assert_eq!(status["result"]["directAvailable"], false);
    let started = answer(
        &mut socket,
        4,
        json!({"cmd":"voice.call_start","params":{"callId":Uuid::new_v4().to_string()}}),
    )
    .await;
    assert_eq!(started["ok"], false, "{started}");
    assert!(
        started["error"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("budget")
    );
    socket.close(None).await.unwrap();
    server.abort();
}

/// Interrupting: a tap keeps the cut line on the call thread, rejects the
/// old response's late audio, and lets the reply to the interruption speak.
#[tokio::test]
async fn a_live_cut_is_recorded_and_late_audio_cannot_resume_it() {
    let mut desk = front_desk(true, 64).await;
    let socket = &mut desk.socket;
    let persona = answer(socket, 1, json!({"cmd":"persona.create","params":{"draft":{"name":"Mack","goal":"Check the PR.","cwd":desk.root.path().to_str().unwrap()}}})).await["result"]["id"].as_str().unwrap().to_string();
    let call = Uuid::new_v4().to_string();
    let started = answer(socket, 2, json!({"cmd":"voice.call_start","params":{"callId":call,"personaId":persona,"mode":"front"}})).await;
    assert_eq!(started["ok"], true, "{started}");
    send(socket, json!({"id":3,"sub":{"call":call}})).await;
    frame_where(socket, |f| f["sub"] == 3 && f["snapshot"].is_array()).await;
    common::until("the front opens", || {
        desk.front.config.lock().unwrap().is_some()
    })
    .await;
    desk.events
        .send(Ok(RealtimeEvent::Saying("The old explanation.".into())))
        .await
        .unwrap();
    desk.events
        .send(Ok(RealtimeEvent::Audio(vec![1; 480])))
        .await
        .unwrap();
    frame_where(socket, |f| f["event"]["type"] == "frontAudio").await;
    let cut = answer(
        socket,
        4,
        json!({"cmd":"voice.front_barge","params":{"callId":call,"playedMs":80}}),
    )
    .await;
    assert_eq!(cut["ok"], true);
    assert_eq!(
        next_asked(&mut desk.asked).await,
        Asked::Truncate { played_ms: 80 }
    );
    desk.events
        .send(Ok(RealtimeEvent::Audio(vec![2; 480])))
        .await
        .unwrap();
    desk.events
        .send(Ok(RealtimeEvent::ResponseDone))
        .await
        .unwrap();
    desk.events
        .send(Ok(RealtimeEvent::Heard("Actually, the beacon.".into())))
        .await
        .unwrap();
    loop {
        let frame = frame_where(socket, |f| f["event"].is_object()).await;
        assert_ne!(frame["event"]["type"], "frontAudio", "late audio: {frame}");
        if frame["event"]["type"] == "heard" {
            break;
        }
    }
    desk.events
        .send(Ok(RealtimeEvent::Saying("The beacon is ready.".into())))
        .await
        .unwrap();
    desk.events
        .send(Ok(RealtimeEvent::Audio(vec![3; 480])))
        .await
        .unwrap();
    frame_where(socket, |f| f["event"]["type"] == "frontAudio").await;
    desk.events
        .send(Ok(RealtimeEvent::ResponseDone))
        .await
        .unwrap();
    frame_where(socket, |f| {
        f["event"]["type"] == "said" && f["event"]["text"] == "The beacon is ready."
    })
    .await;
    let stream = hotline_core::log::StreamId::Call(call.clone());
    common::until("the cut notice", || {
        desk.log
            .load(&stream)
            .iter()
            .any(|line| line["interrupted"] == true)
    })
    .await;
    let lines = desk.log.load(&stream);
    let cuts: Vec<_> = lines
        .iter()
        .filter(|line| line["interrupted"] == true)
        .collect();
    assert_eq!(cuts.len(), 1);
    assert_eq!(cuts[0]["playedMs"], 80);
    let old = lines
        .iter()
        .position(|line| line["text"] == "The old explanation.")
        .unwrap();
    let cut = lines
        .iter()
        .position(|line| line["interrupted"] == true)
        .unwrap();
    let correction = lines
        .iter()
        .position(|line| line["text"] == "Actually, the beacon.")
        .unwrap();
    let reply = lines
        .iter()
        .position(|line| line["text"] == "The beacon is ready.")
        .unwrap();
    assert!(
        old < cut && cut < correction && correction < reply,
        "{lines:?}"
    );
    assert_eq!(cuts[0]["speechId"], lines[old]["id"]);

    assert!(
        cuts[0]["text"]
            .as_str()
            .unwrap()
            .contains("The old explanation.")
    );
    desk.events
        .send(Ok(RealtimeEvent::Saying("One last unfinished line".into())))
        .await
        .unwrap();
    desk.events
        .send(Ok(RealtimeEvent::Audio(vec![4; 480])))
        .await
        .unwrap();
    frame_where(socket, |f| f["event"]["type"] == "frontAudio").await;
    answer(
        socket,
        5,
        json!({"cmd":"voice.call_end","params":{"callId":call}}),
    )
    .await;
    let lines = desk.log.load(&stream);
    assert_eq!(
        lines
            .iter()
            .filter(|line| line["text"] == "One last unfinished line")
            .count(),
        1
    );
    assert_eq!(
        lines.iter().filter(|line| line["kind"] == "agent").count(),
        3
    );
}

#[tokio::test]
async fn a_wire_drop_resumes_the_same_live_call_and_announces_it() {
    let mut desk = front_desk(true, 64).await;
    let call = Uuid::new_v4().to_string();
    answer(
        &mut desk.socket,
        1,
        json!({"cmd":"voice.call_start","params":{"callId":call}}),
    )
    .await;
    send(&mut desk.socket, json!({"id":101,"sub":{"call":call}})).await;
    frame_where(&mut desk.socket, |f| f["snapshot"].is_array()).await;
    desk.socket.close(None).await.unwrap();
    // Reopening the same subscription reattaches to the original call.
    let (mut socket, _) =
        connect_async(format!("ws://127.0.0.1:{}/ws?token=front-test", desk.port))
            .await
            .unwrap();
    send(&mut socket, json!({"id":102,"sub":{"call":call}})).await;
    let snapshot = frame_where(&mut socket, |f| f["snapshot"].is_array()).await;
    assert_ne!(snapshot["snapshot"][0]["state"], "ended");
    let said = tokio::time::timeout(common::ANSWER, async {
        loop {
            if let Some(Asked::Say(text)) = desk.asked.recv().await {
                break text;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(said, "We're back on the same call.");
    let ended = answer(
        &mut socket,
        2,
        json!({"cmd":"voice.call_end","params":{"callId":call}}),
    )
    .await;
    assert_eq!(ended["ok"], true);
}
