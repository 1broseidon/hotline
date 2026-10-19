use super::*;

#[tokio::test]
async fn a_direct_call_uses_the_existing_agent_conversation_without_dispatching() {
    direct_call_uses_existing_conversation(
        false,
        TAGGED_REPLY,
        "The checks passed.",
        TAGGED_WRITTEN,
        Some("The checks passed."),
        "both",
    )
    .await;
}

#[tokio::test]
async fn device_text_reuses_the_direct_agent_session_and_call_origin() {
    direct_call_uses_existing_conversation(
        true,
        TAGGED_REPLY,
        "The checks passed.",
        TAGGED_WRITTEN,
        Some("The checks passed."),
        "both",
    )
    .await;
}

/// An agent that ignores the tags wrote to be read: the call assistant says
/// it again to be heard, that is said and kept as what was said for it, and
/// the chat shows all of the reply.
#[tokio::test]
async fn a_reply_without_tags_is_rewritten_to_be_heard() {
    direct_call_uses_existing_conversation(
        true,
        UNTAGGED_REPLY,
        REWRITTEN,
        UNTAGGED_REPLY,
        Some(REWRITTEN),
        "rewritten",
    )
    .await;
}

/// One brain, two outputs: what is said on a call to a teammate is a turn of
/// the teammate's own conversation (the person's exact words on its tape,
/// marked as said on the call, in its open chapter), the agent is handed the
/// contract after the words, the call says the reply's spoken version, and
/// the chat shows its written version, with what was said kept beside it.
/// No call assistant is asked anything, and the teammate's turn is metered
/// as Chat.
async fn direct_call_uses_existing_conversation(
    device_text: bool,
    reply: &'static str,
    spoken: &str,
    written: &str,
    kept: Option<&str>,
    counted: &str,
) {
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(
        hotline_core::desk::Desk::open_with_voice_services(
            root.path(),
            common::store(),
            Some(services()),
        )
        .unwrap(),
    );
    let log = desk.log.clone();
    let door = Door::bind(log.clone(), "direct-test".into(), desk).unwrap();
    let port = door.port();
    let server = tokio::spawn(door.run());
    let (mut socket, _) = connect_async(format!("ws://127.0.0.1:{port}/ws?token=direct-test"))
        .await
        .unwrap();
    let (provider, requests) = provider_seeing(reply).await;
    // A key, so the teammate's turns are billed per token.
    send(&mut socket, json!({"id":90,"cmd":"credential.custom_save","params":{"draft":{"name":"Direct fixture","baseUrl":provider,"api":"chat_completions","models":["test"],"secret":"fixture-key"}}})).await;
    assert_eq!(
        frame_where(&mut socket, |f| f["id"] == 90).await["ok"],
        true
    );
    send(&mut socket, json!({"id":91,"cmd":"persona.create","params":{"draft":{"name":"Mack","goal":"Check the PR.","cwd":root.path().to_str().unwrap()}}})).await;
    let persona = frame_where(&mut socket, |f| f["id"] == 91).await["result"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    send(
        &mut socket,
        json!({"id":92,"cmd":"session.start","params":{"personaId":persona}}),
    )
    .await;
    assert_eq!(
        frame_where(&mut socket, |f| f["id"] == 92).await["ok"],
        true
    );
    send(
        &mut socket,
        json!({"id":93,"cmd":"voice.status","params":{}}),
    )
    .await;
    assert!(
        frame_where(&mut socket, |f| f["id"] == 93).await["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "voiceDirectCalls")
    );
    let call = Uuid::new_v4().to_string();
    let mut params = json!({"callId":call,"personaId":persona,"streamAudio":true});
    if device_text {
        params["inputMode"] = json!("text");
    }
    send(
        &mut socket,
        json!({"id":1,"cmd":"voice.call_start","params":params}),
    )
    .await;
    let started = frame_where(&mut socket, |f| f["id"] == 1).await;
    assert_eq!(started["result"]["personaId"], persona);
    assert_eq!(
        started["result"]["inputMode"],
        if device_text { "text" } else { "audio" }
    );
    if device_text {
        assert_eq!(started["result"]["input"], json!(["text/plain"]));
    }
    send(&mut socket, json!({"id":2,"sub":{"call":call}})).await;
    frame_where(&mut socket, |f| f["snapshot"].is_array()).await;
    let input = if device_text {
        json!({"id":3,"cmd":"voice.text","params":{"callId":call,"seq":1,"text":"ask Mack to check the failing PR"}})
    } else {
        json!({"id":3,"cmd":"voice.utterance","params":{"callId":call,"seq":1,"mimeType":"audio/wav","data":STANDARD.encode(wav()),"durationMs":2390}})
    };
    send(&mut socket, input.clone()).await;
    let heard = frame_where(&mut socket, |f| f["event"]["type"] == "heard").await;
    // The reply is one line: its spoken part, said in clips under its id and
    // closed by an empty final clip. The teammate is the voice, so there is no
    // `delivery` naming it.
    let mut said = Vec::new();
    let mut clips = Vec::new();
    loop {
        let frame = frame_where(&mut socket, |f| f["event"].is_object()).await;
        let event = &frame["event"];
        assert_ne!(event["type"], "delivery", "the teammate speaks for itself");
        match event["type"].as_str() {
            Some("said") => said.push(event.clone()),
            Some("clip") => {
                clips.push(event.clone());
                if event["final"] == true {
                    break;
                }
            }
            _ => {}
        }
    }
    let line = said[0]["id"].clone();
    assert!(said.iter().all(|said| said["id"] == line), "{said:?}");
    assert_eq!(said.last().unwrap()["text"], spoken);
    assert!(
        said.iter().all(|said| {
            let text = said["text"].as_str().unwrap();
            !text.contains("spoken>") && !text.contains("written>")
        }),
        "{said:?}"
    );
    assert!(clips.iter().all(|clip| clip["id"] == line));
    let last = clips.last().unwrap();
    assert_eq!(last["data"], "");
    assert_eq!(last["index"], clips.len() - 1);
    // Its turn over, the call listens.
    frame_where(&mut socket, |f| f["event"]["state"] == "listening").await;
    let mut replay = input;
    replay["id"] = json!(7);
    send(&mut socket, replay).await;
    assert_eq!(
        frame_where(&mut socket, |f| f["id"] == 7).await["ok"],
        false
    );

    // The agent heard the words, then the contract.
    let words = heard["event"]["text"].as_str().unwrap().to_string();
    let asked = requests
        .lock()
        .unwrap()
        .iter()
        .flat_map(|request| request["messages"].as_array().cloned().unwrap_or_default())
        .filter(|message| message["role"] == "user")
        .map(|message| message["content"].to_string())
        .find(|content| content.contains("[Voice call: answer twice"))
        .expect("the turn's input carries the contract");
    assert!(asked.contains(&words), "{asked}");
    assert!(
        asked.contains("Do not assume the reader heard the spoken version."),
        "{asked}"
    );

    // What was said is kept beside the reply as soon as the reply is
    // written, which for a rewrite may be just after it is said.
    let tape = hotline_core::log::StreamId::Tape(persona.clone());
    common::until("what was said is kept beside the reply", || {
        log.load(&tape)
            .iter()
            .find(|v| v["kind"] == "agent")
            .is_some_and(|reply| reply["spoken"].as_str() == kept)
    })
    .await;
    let tape = log.load(&tape);
    assert_eq!(
        tape.iter().filter(|v| v["kind"] == "chapter").count(),
        1,
        "{tape:?}"
    );
    let user = tape.iter().find(|v| v["kind"] == "user").unwrap();
    assert_eq!(tape.iter().filter(|v| v["kind"] == "user").count(), 1);
    assert_eq!(user["text"], words, "the person's words, as they said them");
    assert!(
        user["id"]
            .as_str()
            .unwrap()
            .starts_with(&format!("voice:{call}:1:agent:"))
    );
    let agents: Vec<&Value> = tape.iter().filter(|v| v["kind"] == "agent").collect();
    let shown: Vec<&str> = agents.iter().map(|v| v["text"].as_str().unwrap()).collect();
    // Paced into bubbles, so compared word for word.
    let words_of = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert_eq!(
        words_of(&shown.join("\n")),
        words_of(written),
        "the chat shows the written version"
    );
    // What was said is kept on the reply's first bubble, for the transcript
    // line and the model's history.
    assert_eq!(agents[0]["spoken"].as_str(), kept, "{tape:?}");
    assert!(agents[1..].iter().all(|v| v.get("spoken").is_none()));
    assert!(
        !tape.iter().any(|v| {
            let event = v.to_string();
            event.contains("spoken>") || event.contains("written>")
        }),
        "{tape:?}"
    );
    assert!(
        !log.load(&hotline_core::log::StreamId::Tape(
            hotline_core::voice::TAPE_ID.into()
        ))
        .iter()
        .any(|v| matches!(v["kind"].as_str(), Some("user" | "agent")))
    );
    // The turn spent the Chat budget as the teammate's; nothing went on a
    // call assistant.
    let ledger = |file: &str| -> Value {
        std::fs::read(root.path().join(file))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or(Value::Null)
    };
    assert!(
        ledger("chat-ledger.json")["daySpend"]["teammates"]
            .as_f64()
            .is_some_and(|spent| spent > 0.0),
        "{}",
        ledger("chat-ledger.json")
    );
    assert_eq!(
        ledger("voice-ledger.json")["daySpend"]["dispatcher"]
            .as_f64()
            .unwrap_or(0.0),
        0.0
    );
    // The reply is counted by how it was said, under the teammate's agent
    // and model, and the count is on the wire.
    send(
        &mut socket,
        json!({"id":8,"cmd":"voice.status","params":{}}),
    )
    .await;
    let replies = frame_where(&mut socket, |f| f["id"] == 8).await["result"]["replies"].clone();
    assert_eq!(replies.as_array().map(Vec::len), Some(1), "{replies}");
    let model = replies[0]["model"].as_str().unwrap();
    assert!(
        model.starts_with("hotline/") && model.ends_with("/test"),
        "{model}"
    );
    for path in ["both", "spokenOnly", "unclosed", "untagged", "rewritten"] {
        let expected = u64::from(path == counted);
        assert_eq!(replies[0][path], expected, "{path}: {replies}");
    }
    send(
        &mut socket,
        json!({"id":4,"cmd":"voice.call_start","params":{"callId":call}}),
    )
    .await;
    assert_eq!(
        frame_where(&mut socket, |f| f["id"] == 4).await["ok"],
        false,
        "a live target cannot silently change to desk"
    );
    send(&mut socket, json!({"id":5,"cmd":"voice.call_start","params":{"callId":Uuid::new_v4().to_string(),"personaId":"missing"}})).await;
    assert_eq!(
        frame_where(&mut socket, |f| f["id"] == 5).await["ok"],
        false
    );
    send(
        &mut socket,
        json!({"id":6,"cmd":"voice.hold","params":{"callId":call,"hold":true}}),
    )
    .await;
    assert_eq!(
        frame_where(&mut socket, |f| f["id"] == 6).await["ok"],
        true,
        "invalid target did not replace the existing call"
    );
    socket.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn direct_call_readiness_and_start_survive_an_unavailable_desk_dispatcher() {
    let root = tempfile::tempdir().unwrap();
    let desk =
        Arc::new(hotline_core::desk::Desk::open_with_store(root.path(), common::store()).unwrap());
    let door = Door::bind(desk.log.clone(), "readiness-test".into(), desk).unwrap();
    let port = door.port();
    let server = tokio::spawn(door.run());
    let (mut socket, _) = connect_async(format!("ws://127.0.0.1:{port}/ws?token=readiness-test"))
        .await
        .unwrap();
    send(&mut socket, json!({"id":1,"cmd":"credential.create","params":{"providerId":"openai","label":"Speech fixture","secret":"unused-test-key"}})).await;
    assert_eq!(frame_where(&mut socket, |f| f["id"] == 1).await["ok"], true);
    send(&mut socket, json!({"id":2,"cmd":"settings.update","params":{"patch":{"voice":{"dispatcher":{"provider":"not-connected"}}}}})).await;
    assert_eq!(frame_where(&mut socket, |f| f["id"] == 2).await["ok"], true);
    send(&mut socket, json!({"id":3,"cmd":"persona.create","params":{"draft":{"name":"Mack","goal":"Check the PR.","cwd":root.path().to_str().unwrap()}}})).await;
    let persona = frame_where(&mut socket, |f| f["id"] == 3).await["result"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    send(
        &mut socket,
        json!({"id":4,"cmd":"voice.status","params":{}}),
    )
    .await;
    let status = frame_where(&mut socket, |f| f["id"] == 4).await;
    assert_eq!(status["result"]["available"], false);
    assert_eq!(status["result"]["directAvailable"], true);
    send(
        &mut socket,
        json!({"id":5,"cmd":"voice.call_start","params":{"callId":Uuid::new_v4().to_string()}}),
    )
    .await;
    assert_eq!(
        frame_where(&mut socket, |f| f["id"] == 5).await["ok"],
        false
    );
    let call = Uuid::new_v4().to_string();
    send(&mut socket, json!({"id":6,"cmd":"voice.call_start","params":{"callId":call,"personaId":persona,"streamAudio":true}})).await;
    let started = frame_where(&mut socket, |f| f["id"] == 6).await;
    assert_eq!(started["ok"], true);
    assert_eq!(started["result"]["personaId"], persona);
    send(
        &mut socket,
        json!({"id":7,"cmd":"voice.call_end","params":{"callId":call}}),
    )
    .await;
    assert_eq!(frame_where(&mut socket, |f| f["id"] == 7).await["ok"], true);
    send(&mut socket, json!({"id":8,"cmd":"settings.update","params":{"patch":{"voice":{"dispatcher":{"provider":"not-connected"},"stt":{"provider":"not-connected"}}}}})).await;
    assert_eq!(frame_where(&mut socket, |f| f["id"] == 8).await["ok"], true);
    send(
        &mut socket,
        json!({"id":9,"cmd":"voice.status","params":{"inputMode":"text"}}),
    )
    .await;
    let status = frame_where(&mut socket, |f| f["id"] == 9).await;
    assert_eq!(status["result"]["available"], false);
    assert_eq!(status["result"]["directAvailable"], true);
    assert!(status["result"].get("stt").is_none());
    send(
        &mut socket,
        json!({"id":10,"cmd":"voice.call_start","params":{"callId":Uuid::new_v4().to_string(),"inputMode":"text"}}),
    )
    .await;
    assert_eq!(
        frame_where(&mut socket, |f| f["id"] == 10).await["ok"],
        false
    );
    let text_call = Uuid::new_v4().to_string();
    send(&mut socket, json!({"id":11,"cmd":"voice.call_start","params":{"callId":text_call,"personaId":persona,"inputMode":"text","streamAudio":true}})).await;
    let started = frame_where(&mut socket, |f| f["id"] == 11).await;
    assert_eq!(started["ok"], true);
    assert_eq!(started["result"]["personaId"], persona);
    assert_eq!(started["result"]["inputMode"], "text");
    assert_eq!(started["result"]["input"], json!(["text/plain"]));
    send(
        &mut socket,
        json!({"id":12,"cmd":"voice.call_end","params":{"callId":text_call}}),
    )
    .await;
    assert_eq!(
        frame_where(&mut socket, |f| f["id"] == 12).await["ok"],
        true
    );
    socket.close(None).await.unwrap();
    server.abort();
}

/// Live calls Off offers no call at all, to the desk or to a teammate, and
/// `voice.status` says why; turning it back on restores both.
#[tokio::test]
async fn calls_off_refuses_a_call_and_says_why() {
    let root = tempfile::tempdir().unwrap();
    let desk =
        Arc::new(hotline_core::desk::Desk::open_with_store(root.path(), common::store()).unwrap());
    let door = Door::bind(desk.log.clone(), "calls-off-test".into(), desk).unwrap();
    let port = door.port();
    let server = tokio::spawn(door.run());
    let (mut socket, _) = connect_async(format!("ws://127.0.0.1:{port}/ws?token=calls-off-test"))
        .await
        .unwrap();
    send(&mut socket, json!({"id":1,"cmd":"credential.create","params":{"providerId":"openai","label":"Speech fixture","secret":"unused-test-key"}})).await;
    assert_eq!(frame_where(&mut socket, |f| f["id"] == 1).await["ok"], true);
    send(&mut socket, json!({"id":2,"cmd":"persona.create","params":{"draft":{"name":"Mack","goal":"Check the PR.","cwd":root.path().to_str().unwrap()}}})).await;
    let persona = frame_where(&mut socket, |f| f["id"] == 2).await["result"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    send(
        &mut socket,
        json!({"id":3,"cmd":"voice.status","params":{}}),
    )
    .await;
    let status = frame_where(&mut socket, |f| f["id"] == 3).await;
    assert_eq!(status["result"]["directAvailable"], true);

    send(&mut socket, json!({"id":4,"cmd":"settings.update","params":{"patch":{"voice":{"front":{"provider":"off"}}}}})).await;
    assert_eq!(frame_where(&mut socket, |f| f["id"] == 4).await["ok"], true);
    send(
        &mut socket,
        json!({"id":5,"cmd":"voice.status","params":{}}),
    )
    .await;
    let status = frame_where(&mut socket, |f| f["id"] == 5).await;
    assert_eq!(status["result"]["available"], false);
    assert_eq!(status["result"]["directAvailable"], false);
    assert_eq!(status["result"]["unavailable"], "Calls are off in Settings");
    for (id, params) in [
        (6, json!({"callId":Uuid::new_v4().to_string()})),
        (
            7,
            json!({"callId":Uuid::new_v4().to_string(),"personaId":persona}),
        ),
    ] {
        send(
            &mut socket,
            json!({"id":id,"cmd":"voice.call_start","params":params}),
        )
        .await;
        let refused = frame_where(&mut socket, |f| f["id"] == id).await;
        assert_eq!(refused["ok"], false, "{refused}");
        assert!(
            refused.to_string().contains("Calls are off in Settings"),
            "{refused}"
        );
    }

    send(&mut socket, json!({"id":8,"cmd":"settings.update","params":{"patch":{"voice":{"front":{"provider":"legacy"}}}}})).await;
    assert_eq!(frame_where(&mut socket, |f| f["id"] == 8).await["ok"], true);
    send(
        &mut socket,
        json!({"id":9,"cmd":"voice.status","params":{}}),
    )
    .await;
    let status = frame_where(&mut socket, |f| f["id"] == 9).await;
    assert_eq!(
        status["result"]["directAvailable"], true,
        "Legacy voice still calls"
    );
    socket.close(None).await.unwrap();
    server.abort();
}

/// A desk that cannot call yet becomes one that can the moment a voice
/// provider is connected, with no turn between: the room announces the new
/// connection, which is what the window asks `voice.status` again on, and
/// the answer is already ready, the Grok front included.
#[tokio::test]
async fn connecting_a_voice_provider_readies_calls_without_a_turn() {
    let root = tempfile::tempdir().unwrap();
    let desk =
        Arc::new(hotline_core::desk::Desk::open_with_store(root.path(), common::store()).unwrap());
    let door = Door::bind(desk.log.clone(), "fresh-desk-test".into(), desk).unwrap();
    let port = door.port();
    let server = tokio::spawn(door.run());
    let (mut socket, _) = connect_async(format!("ws://127.0.0.1:{port}/ws?token=fresh-desk-test"))
        .await
        .unwrap();
    send(&mut socket, json!({"id":1,"cmd":"settings.update","params":{"patch":{"voice":{"front":{"provider":"grok"}}}}})).await;
    assert_eq!(frame_where(&mut socket, |f| f["id"] == 1).await["ok"], true);
    send(&mut socket, json!({"id":2,"sub":"room"})).await;
    frame_where(&mut socket, |f| f["sub"] == 2 && f["snapshot"].is_array()).await;
    send(
        &mut socket,
        json!({"id":3,"cmd":"voice.status","params":{}}),
    )
    .await;
    let status = frame_where(&mut socket, |f| f["id"] == 3).await;
    assert_eq!(status["result"]["available"], false, "{status}");
    assert_eq!(status["result"]["directAvailable"], false, "{status}");
    assert!(status["result"]["front"].is_null(), "{status}");

    send(&mut socket, json!({"id":4,"cmd":"credential.create","params":{"providerId":"xai","label":"Grok","secret":"unused-test-key"}})).await;
    assert_eq!(frame_where(&mut socket, |f| f["id"] == 4).await["ok"], true);
    frame_where(&mut socket, |f| {
        f["sub"] == 2 && f["event"]["kind"] == "credential" && f["event"]["providerId"] == "xai"
    })
    .await;
    send(
        &mut socket,
        json!({"id":5,"cmd":"voice.status","params":{}}),
    )
    .await;
    let status = frame_where(&mut socket, |f| f["id"] == 5).await;
    assert_eq!(status["result"]["directAvailable"], true, "{status}");
    assert_eq!(status["result"]["front"]["providerId"], "grok", "{status}");
    socket.close(None).await.unwrap();
    server.abort();
}
