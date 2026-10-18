use super::*;

/// A front call hung up on a question nothing answered: neither handed to
/// the teammate nor replied to by the front. The question goes to the
/// teammate as a line from the call, told the call is over so it answers in
/// writing with nothing to say aloud; a goodbye left hanging does not.
#[tokio::test]
async fn a_question_the_call_ended_on_goes_to_the_teammate_to_answer_in_writing() {
    let mut desk = front_desk(true, 64).await;
    let root = desk.root.path().to_path_buf();
    let socket = &mut desk.socket;
    let (provider, requests) = provider_seeing("The release checklist has two items left.").await;
    let saved = answer(socket, 90, json!({"cmd":"credential.custom_save","params":{"draft":{"name":"Front fixture","baseUrl":provider,"api":"chat_completions","models":["test"],"secret":"fixture-key"}}})).await;
    assert_eq!(saved["ok"], true);
    let persona = answer(socket, 91, json!({"cmd":"persona.create","params":{"draft":{"name":"Mack","goal":"Check the PR.","cwd":root.to_str().unwrap()}}})).await["result"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        answer(
            socket,
            92,
            json!({"cmd":"session.start","params":{"personaId":persona}})
        )
        .await["ok"],
        true
    );

    let call = Uuid::new_v4().to_string();
    let started = answer(
        socket,
        1,
        json!({"cmd":"voice.call_start","params":{"callId":call,"personaId":persona,"mode":"front"}}),
    )
    .await;
    assert_eq!(started["ok"], true, "{started}");
    send(socket, json!({"id":2,"sub":{"call":call}})).await;
    frame_where(socket, |f| f["sub"] == 2 && f["snapshot"].is_array()).await;
    common::until("the front is opened", || {
        desk.front.config.lock().unwrap().is_some()
    })
    .await;

    // The person asks; the front starts to answer itself and is cut off by the hang-up.
    let words = "what's left on the release checklist?";
    desk.events
        .send(Ok(RealtimeEvent::Heard(words.into())))
        .await
        .unwrap();
    desk.events
        .send(Ok(RealtimeEvent::Saying("Let me think, the".into())))
        .await
        .unwrap();
    frame_where(socket, |f| f["sub"] == 2 && f["event"]["type"] == "heard").await;
    let ended = answer(
        socket,
        3,
        json!({"cmd":"voice.call_end","params":{"callId":call}}),
    )
    .await;
    assert_eq!(ended["ok"], true);

    let tape = hotline_core::log::StreamId::Tape(persona.clone());
    common::until(
        "the teammate answers the question the call ended on",
        || {
            desk.log
                .load(&tape)
                .iter()
                .any(|v| v["kind"] == "agent" && !is_call_post(v))
        },
    )
    .await;
    let tape = desk.log.load(&tape);
    let users: Vec<&Value> = tape.iter().filter(|v| v["kind"] == "user").collect();
    assert_eq!(users.len(), 1, "{tape:?}");
    assert_eq!(users[0]["text"], words, "the person's words, unrephrased");
    assert!(
        users[0]["id"]
            .as_str()
            .unwrap()
            .starts_with(&format!("voice:{call}:"))
    );
    let agent = tape
        .iter()
        .find(|v| v["kind"] == "agent" && !is_call_post(v))
        .unwrap();
    assert_eq!(agent["text"], "The release checklist has two items left.");
    assert!(agent.get("spoken").is_none(), "written only: {agent}");

    let asked = requests
        .lock()
        .unwrap()
        .iter()
        .flat_map(|request| request["messages"].as_array().cloned().unwrap_or_default())
        .filter(|message| message["role"] == "user")
        .map(|message| message["content"].as_str().unwrap_or_default().to_string())
        .find(|content| content.contains("ended before it was answered"))
        .expect("the teammate was told the call ended on the question");
    assert!(
        asked.contains(&format!(
            "{words}\n\n[This came from a voice call that ended"
        )),
        "{asked}"
    );
    assert!(
        !asked.contains("[Voice call: answer twice"),
        "no spoken version: {asked}"
    );
}

/// After a call that answered a question from `search_thread`, the DM holds
/// one first-person summary with the question and the front's answer and no
/// tool name, posted as the teammate; and the teammate's next typed turn is
/// told what the call said.
#[tokio::test]
async fn a_calls_summary_is_the_teammates_own_words_and_its_next_turn_knows_it() {
    let mut desk = front_desk(true, 64).await;
    let root = desk.root.path().to_path_buf();
    let socket = &mut desk.socket;
    let (provider, requests) = provider_seeing("Noted.").await;
    let saved = answer(socket, 90, json!({"cmd":"credential.custom_save","params":{"draft":{"name":"Front fixture","baseUrl":provider,"api":"chat_completions","models":["test"],"secret":"fixture-key"}}})).await;
    assert_eq!(saved["ok"], true);
    let persona = answer(socket, 91, json!({"cmd":"persona.create","params":{"draft":{"name":"Mack","goal":"Check the PR.","cwd":root.to_str().unwrap()}}})).await["result"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        answer(
            socket,
            92,
            json!({"cmd":"session.start","params":{"personaId":persona}})
        )
        .await["ok"],
        true
    );
    assert_eq!(
        answer(socket, 93, json!({"cmd":"session.prompt","params":{"personaId":persona,"text":"we fixed the zebracrossing scheduler bug"}})).await["ok"],
        true
    );
    let tape = hotline_core::log::StreamId::Tape(persona.clone());
    common::until("the teammate's turn ends", || {
        desk.log
            .load(&tape)
            .iter()
            .any(|event| event["kind"] == "turn")
    })
    .await;
    let call = Uuid::new_v4().to_string();
    assert_eq!(
        answer(socket, 1, json!({"cmd":"voice.call_start","params":{"callId":call,"personaId":persona,"mode":"front"}})).await["ok"],
        true
    );
    send(socket, json!({"id":2,"sub":{"call":call}})).await;
    frame_where(socket, |f| f["sub"] == 2 && f["snapshot"].is_array()).await;
    common::until("the front is opened", || {
        desk.front.config.lock().unwrap().is_some()
    })
    .await;

    desk.events
        .send(Ok(RealtimeEvent::Heard(
            "what did we fix on the scheduler?".into(),
        )))
        .await
        .unwrap();
    desk.events
        .send(Ok(RealtimeEvent::ToolCall {
            call_id: "s1".into(),
            name: "search_thread".into(),
            arguments: json!({"query":"zebracrossing scheduler"}).to_string(),
        }))
        .await
        .unwrap();
    let Asked::ToolResult { .. } = next_asked(&mut desk.asked).await else {
        panic!("the search went unanswered")
    };
    let answered = "We fixed the zebracrossing bug in the scheduler.";
    desk.events
        .send(Ok(RealtimeEvent::Saying(answered.into())))
        .await
        .unwrap();
    desk.events
        .send(Ok(RealtimeEvent::ResponseDone))
        .await
        .unwrap();
    frame_where(socket, |f| {
        f["event"]["type"] == "said" && f["event"]["text"] == answered
    })
    .await;
    assert_eq!(
        answer(
            socket,
            3,
            json!({"cmd":"voice.call_end","params":{"callId":call}})
        )
        .await["ok"],
        true
    );
    common::until("the call's summary is posted", || {
        desk.log.load(&tape).iter().any(is_call_post)
    })
    .await;
    let summaries: Vec<Value> = desk
        .log
        .load(&tape)
        .into_iter()
        .filter(is_call_post)
        .collect();
    assert_eq!(summaries.len(), 1, "{summaries:?}");
    let text = summaries[0]["text"].as_str().unwrap();
    assert!(text.starts_with("From the call:"), "{text}");
    assert!(text.contains("what did we fix on the scheduler?"), "{text}");
    assert!(text.contains(answered), "{text}");
    assert!(
        !text.contains("search_thread") && !text.contains("working session"),
        "{text}"
    );

    // The next typed turn carries on from the call.
    assert_eq!(
        answer(
            socket,
            94,
            json!({"cmd":"session.prompt","params":{"personaId":persona,"text":"and the release?"}})
        )
        .await["ok"],
        true
    );
    common::until("the follow-up turn is told what the call said", || {
        requests.lock().unwrap().iter().any(|request| {
            let body = request.to_string();
            body.contains("and the release?") && body.contains(answered)
        })
    })
    .await;
}
