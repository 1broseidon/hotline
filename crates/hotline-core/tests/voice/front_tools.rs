use super::*;

/// The front answers from the teammate's own tools: a question about what
/// was worked on is answered from `search_thread` with the result handed
/// back for the front to speak, and nothing sent to the teammate or posted
/// to its conversation. A tool the front was not given is refused through
/// the same handler, and changing the teammate's grants refuses the next call.
#[tokio::test]
async fn a_front_call_answers_from_a_desk_side_tool_and_a_revoked_grant_is_refused() {
    let mut desk = front_desk(true, 64).await;
    let root = desk.root.path().to_path_buf();
    let socket = &mut desk.socket;
    let (provider, _requests) = provider_seeing(TAGGED_REPLY).await;
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
    // Earlier work, in the conversation the search reads.
    let typed = answer(socket, 93, json!({"cmd":"session.prompt","params":{"personaId":persona,"text":"we fixed the zebracrossing scheduler bug"}})).await;
    assert_eq!(typed["ok"], true, "{typed}");
    let tape = hotline_core::log::StreamId::Tape(persona.clone());
    common::until("the teammate's turn ends", || {
        desk.log
            .load(&tape)
            .iter()
            .any(|event| event["kind"] == "turn")
    })
    .await;
    let call = Uuid::new_v4().to_string();
    let started = answer(
        socket,
        1,
        json!({"cmd":"voice.call_start","params":{"callId":call,"personaId":persona,"mode":"front"}}),
    )
    .await;
    assert_eq!(started["ok"], true, "{started}");
    common::until("the front is opened", || {
        desk.front.config.lock().unwrap().is_some()
    })
    .await;
    let turns = desk.log.load(&tape).len();

    let tool_result = |call_id: &'static str, name: &'static str, arguments: Value| {
        let events = desk.events.clone();
        async move {
            events
                .send(Ok(RealtimeEvent::ToolCall {
                    call_id: call_id.into(),
                    name: name.into(),
                    arguments: arguments.to_string(),
                }))
                .await
                .unwrap();
        }
    };
    tool_result(
        "s1",
        "search_thread",
        json!({"query":"zebracrossing scheduler"}),
    )
    .await;
    let Asked::ToolResult {
        call_id,
        output,
        reply,
    } = next_asked(&mut desk.asked).await
    else {
        panic!("the search went unanswered")
    };
    assert_eq!(call_id, "s1");
    assert!(output.contains("zebracrossing"), "{output}");
    assert!(reply, "the front speaks once it has the result");
    assert_eq!(
        desk.log.load(&tape).len(),
        turns,
        "a lookup sends nothing to the teammate and posts nothing"
    );

    // Not on the front's list: refused by the handler, not by the model.
    tool_result("d1", "send_file", json!({"path":"x"})).await;
    // Not a tool of the room at all.
    let Asked::ToolResult { output, .. } = next_asked(&mut desk.asked).await else {
        panic!("the refusal went unanswered")
    };
    assert!(
        output.contains("There is no tool called send_file"),
        "{output}"
    );

    // All writes use work_on_it; a front cannot bypass the clone to post.
    tool_result("p1", "post_to_chat", json!({"text":"not written"})).await;
    let Asked::ToolResult { output, .. } = next_asked(&mut desk.asked).await else {
        panic!("no refusal")
    };
    assert!(output.contains("There is no tool"));
    // Revoking a grant on the card refuses the next call.
    let patched = answer(socket, 94, json!({"cmd":"persona.update","params":{"id":persona,"patch":{"mcpPolicy":{"mode":"all","serverIds":[]}}}})).await;
    assert_eq!(patched["ok"], true, "{patched}");
    tool_result("s2", "search_thread", json!({"query":"zebracrossing"})).await;
    let Asked::ToolResult { output, .. } = next_asked(&mut desk.asked).await else {
        panic!("the revoked search went unanswered")
    };
    assert!(output.contains("revoked"), "{output}");
    tool_result("w2", "work_on_it", json!({"words":"Write a plan."})).await;
    let Asked::ToolResult { output, .. } = next_asked(&mut desk.asked).await else {
        panic!("the revoked work went unanswered")
    };
    assert!(output.contains("revoked"), "{output}");
    assert!(
        !desk
            .log
            .load(&hotline_core::log::StreamId::Call(call))
            .iter()
            .any(|line| line["kind"] == "turn")
    );
    assert_eq!(desk.log.load(&tape).len(), turns);
}

/// A front that runs no tools itself (GPT-Live) delegates everything as
/// `work_on_it`: its own call-thread agent answers, without an intermediate
/// call-assistant loop or a turn on the home conversation.
#[tokio::test]
async fn gpt_live_delegation_uses_the_same_call_thread_work_tool() {
    let mut desk = front_desk_with(true, 64, true).await;
    let root = desk.root.path().to_path_buf();
    let socket = &mut desk.socket;
    let (provider, _requests) = provider_seeing(TAGGED_REPLY).await;
    let saved = answer(socket, 90, json!({"cmd":"credential.custom_save","params":{"draft":{"name":"Front fixture","baseUrl":provider,"api":"chat_completions","models":["test"],"secret":"fixture-key"}}})).await;
    assert_eq!(saved["ok"], true);
    let persona = answer(socket, 91, json!({"cmd":"persona.create","params":{"draft":{"name":"Mack","goal":"Check the PR.","cwd":root.to_str().unwrap()}}})).await["result"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    answer(
        socket,
        92,
        json!({"cmd":"session.start","params":{"personaId":persona}}),
    )
    .await;
    let typed = answer(socket, 93, json!({"cmd":"session.prompt","params":{"personaId":persona,"text":"we fixed the zebracrossing scheduler bug"}})).await;
    assert_eq!(typed["ok"], true, "{typed}");
    let tape = hotline_core::log::StreamId::Tape(persona.clone());
    common::until("the teammate's turn ends", || {
        desk.log
            .load(&tape)
            .iter()
            .any(|event| event["kind"] == "turn")
    })
    .await;
    let call = Uuid::new_v4().to_string();
    let started = answer(
        socket,
        1,
        json!({"cmd":"voice.call_start","params":{"callId":call,"personaId":persona,"mode":"front"}}),
    )
    .await;
    assert_eq!(started["ok"], true, "{started}");
    common::until("the front is opened", || {
        desk.front.config.lock().unwrap().is_some()
    })
    .await;
    let declared: Vec<String> = desk
        .front
        .config
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect();
    assert!(
        declared.contains(&"search_thread".to_string()),
        "{declared:?}"
    );
    let turns = desk
        .log
        .load(&tape)
        .iter()
        .filter(|event| event["kind"] == "user" || event["kind"] == "turn")
        .count();

    let delegate = |call_id: &'static str, words: &'static str| {
        let events = desk.events.clone();
        async move {
            events
                .send(Ok(RealtimeEvent::ToolCall {
                    call_id: call_id.into(),
                    name: "work_on_it".into(),
                    arguments: json!({ "words": words }).to_string(),
                }))
                .await
                .unwrap();
        }
    };
    delegate("d1", "what did we fix?").await;
    let Asked::ToolResult {
        call_id,
        output,
        reply,
    } = next_asked(&mut desk.asked).await
    else {
        panic!("the lookup went unanswered")
    };
    assert_eq!(call_id, "d1");
    assert!(output.contains("Work accepted"));
    assert!(!reply);
    common::until("the clone finishes the lookup on the call thread", || {
        desk.log
            .load(&hotline_core::log::StreamId::Call(call.clone()))
            .iter()
            .any(|event| event["kind"] == "turn")
    })
    .await;
    assert_eq!(
        desk.log
            .load(&tape)
            .iter()
            .filter(|event| event["kind"] == "user" || event["kind"] == "turn")
            .count(),
        turns,
        "GPT-Live never prompts the home agent"
    );
    assert!(
        desk.log
            .load(&tape)
            .iter()
            .any(|event| is_call_post(event) && event["text"] == TAGGED_WRITTEN),
        "the written table reaches chat"
    );
}

/// A call with three exchanges and one hand-off leaves the hand-off's turn
/// and one summary in the DM, and none of the front's own turns.
#[tokio::test]
async fn a_front_call_posts_one_summary_to_the_dm_and_not_its_turns() {
    let mut desk = front_desk(true, 64).await;
    let root = desk.root.path().to_path_buf();
    let socket = &mut desk.socket;
    let (provider, requests) = provider_seeing("The checks passed.").await;
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

    let handoff = "check the failing PR from monday";
    for (heard, said) in [
        ("hello there, can you hear me", "Loud and clear."),
        (handoff, "On it."),
        ("thanks, that is all for now", "Anytime."),
    ] {
        desk.events
            .send(Ok(RealtimeEvent::Heard(heard.into())))
            .await
            .unwrap();
        if heard == handoff {
            desk.events
                .send(Ok(RealtimeEvent::ToolCall {
                    call_id: "h1".into(),
                    name: "work_on_it".into(),
                    arguments: json!({ "words": handoff }).to_string(),
                }))
                .await
                .unwrap();
            let Asked::ToolResult { .. } = next_asked(&mut desk.asked).await else {
                panic!("the hand-off went unanswered")
            };
        }
        desk.events
            .send(Ok(RealtimeEvent::Saying(said.into())))
            .await
            .unwrap();
        desk.events
            .send(Ok(RealtimeEvent::ResponseDone))
            .await
            .unwrap();
        frame_where(socket, |f| {
            f["event"]["type"] == "said" && f["event"]["text"] == said
        })
        .await;
    }
    let tape = hotline_core::log::StreamId::Tape(persona.clone());
    common::until("the teammate's turn ends", || {
        desk.log
            .load(&hotline_core::log::StreamId::Call(call.clone()))
            .iter()
            .any(|event| event["kind"] == "turn")
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
    closing_note_due(&desk.log, &call).await;
    common::until("the call posts to the conversation", || {
        desk.log.load(&tape).iter().any(is_call_post)
    })
    .await;
    let dm = desk.log.load(&tape);
    let users: Vec<&Value> = dm.iter().filter(|v| v["kind"] == "user").collect();
    assert!(
        users.is_empty(),
        "call work never enters the home thread: {dm:?}"
    );
    let summaries: Vec<&Value> = dm.iter().filter(|v| is_call_post(v)).collect();
    assert_eq!(summaries.len(), 1, "{dm:?}");
    let text = summaries[0]["text"].as_str().unwrap();
    assert_eq!(text, CALL_NOTE);
    assert!(
        requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request["stream"] != true
                && request.to_string().contains(handoff)
                && request.to_string().contains("Loud and clear."))
    );
    for said in ["Loud and clear.", "On it.", "Anytime."] {
        assert!(
            !dm.iter().any(|v| v["text"]
                .as_str()
                .is_some_and(|t| t.contains(said) && v["kind"] != "agent")),
            "the front's {said:?} stays on the call: {dm:?}"
        );
    }
}
