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

    // Asked to put something in the chat, the front posts it, stamped as from
    // the call, and a client reading the conversation is sent it as such.
    send(socket, json!({"id":80,"sub":{"tape":persona}})).await;
    frame_where(socket, |f| f["id"] == 80 && f["ok"] == true).await;
    tool_result(
        "p1",
        "post_to_chat",
        json!({"text":"the scheduler fix is in"}),
    )
    .await;
    let Asked::ToolResult { output, .. } = next_asked(&mut desk.asked).await else {
        panic!("the post went unanswered")
    };
    assert_eq!(output, "Posted to the chat.");
    let posted: Vec<Value> = desk.log.load(&tape).split_off(turns);
    assert_eq!(posted.len(), 1, "{posted:?}");
    assert_eq!(posted[0]["kind"], "agent", "the teammate says it");
    assert_eq!(posted[0]["text"], "the scheduler fix is in");
    assert_eq!(posted[0]["from"]["kind"], "call", "{posted:?}");
    let sent = frame_where(socket, |f| f["sub"] == 80 && f["event"]["kind"] == "agent").await;
    assert_eq!(sent["event"]["text"], "the scheduler fix is in");
    assert_eq!(
        sent["event"]["from"],
        json!({"thread": call, "kind": "call"}),
        "{sent}"
    );

    // Revoking a grant on the card refuses the next call.
    let patched = answer(socket, 94, json!({"cmd":"persona.update","params":{"id":persona,"patch":{"mcpPolicy":{"mode":"all","serverIds":[]}}}})).await;
    assert_eq!(patched["ok"], true, "{patched}");
    tool_result("s2", "search_thread", json!({"query":"zebracrossing"})).await;
    let Asked::ToolResult { output, .. } = next_asked(&mut desk.asked).await else {
        panic!("the revoked search went unanswered")
    };
    assert!(output.contains("revoked"), "{output}");
    tool_result("p2", "post_to_chat", json!({"text":"too late"})).await;
    let Asked::ToolResult { output, .. } = next_asked(&mut desk.asked).await else {
        panic!("the revoked post went unanswered")
    };
    assert!(output.contains("revoked"), "{output}");
    assert_eq!(
        desk.log.load(&tape).len(),
        turns + 1,
        "a revoked post is not posted"
    );
}

/// A front that runs no tools itself (GPT-Live) delegates everything as
/// `send_to_teammate`: the call assistant answers a lookup with the
/// teammate's tools, said back with no hand-off, and what it cannot answer
/// still reaches the working session.
#[tokio::test]
async fn a_delegating_fronts_lookup_is_answered_desk_side_and_work_is_handed_on() {
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
    let turns = desk.log.load(&tape).len();

    let delegate = |call_id: &'static str, words: &'static str| {
        let events = desk.events.clone();
        async move {
            events
                .send(Ok(RealtimeEvent::ToolCall {
                    call_id: call_id.into(),
                    name: "send_to_teammate".into(),
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
    assert_eq!(
        (call_id.as_str(), output.as_str(), reply),
        ("d1", "We fixed the zebracrossing bug.", true)
    );
    assert_eq!(
        desk.log.load(&tape).len(),
        turns,
        "a lookup is not a hand-off"
    );

    // What the assistant cannot answer is handed on, as it was before.
    delegate("d2", "rename the module").await;
    let Asked::ToolResult { call_id, .. } = next_asked(&mut desk.asked).await else {
        panic!("the hand-off went unanswered")
    };
    assert_eq!(call_id, "d2");
    assert!(
        desk.log.load(&tape).len() > turns,
        "a hand-off reaches the teammate"
    );
}

/// A call with three exchanges and one hand-off leaves the hand-off's turn
/// and one summary in the DM, and none of the front's own turns.
#[tokio::test]
async fn a_front_call_posts_one_summary_to_the_dm_and_not_its_turns() {
    let mut desk = front_desk(true, 64).await;
    let root = desk.root.path().to_path_buf();
    let socket = &mut desk.socket;
    let (provider, _requests) = provider_seeing("The checks passed.").await;
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
                    name: "send_to_teammate".into(),
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
            .load(&tape)
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
    common::until("the call posts to the conversation", || {
        desk.log.load(&tape).iter().any(is_call_post)
    })
    .await;
    let dm = desk.log.load(&tape);
    let users: Vec<&Value> = dm.iter().filter(|v| v["kind"] == "user").collect();
    assert_eq!(users.len(), 1, "only the hand-off is a turn: {dm:?}");
    assert_eq!(users[0]["text"], handoff);
    let summaries: Vec<&Value> = dm.iter().filter(|v| is_call_post(v)).collect();
    assert_eq!(summaries.len(), 1, "{dm:?}");
    let text = summaries[0]["text"].as_str().unwrap();
    assert!(text.starts_with("You asked"), "{text}");
    assert!(text.contains("hello there, can you hear me"), "{text}");
    assert!(
        text.contains(&format!("I'm still working on this: \"{handoff}\"")),
        "{text}"
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

/// One call, start to hang-up: the front speaks in the card's voice, answers
/// a lookup from a desk-side tool, refuses a tool outside the teammate's
/// reach, hands one change on, posts to the chat when asked, and leaves the
/// DM holding exactly the hand-off's turn, the post and one first-person
/// summary that the teammate's history carries.
#[tokio::test]
async fn one_call_proves_the_whole_front() {
    let mut desk = front_desk(true, 64).await;
    let root = desk.root.path().to_path_buf();
    let socket = &mut desk.socket;
    let (provider, requests) = provider_seeing("On it.").await;
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
    answer(socket, 93, json!({"cmd":"session.prompt","params":{"personaId":persona,"text":"we fixed the zebracrossing scheduler bug"}})).await;
    let tape = hotline_core::log::StreamId::Tape(persona.clone());
    common::until("the teammate's turn ends", || {
        desk.log.load(&tape).iter().any(|e| e["kind"] == "turn")
    })
    .await;
    let front =
        answer(socket, 94, json!({"cmd":"voice.status","params":{}})).await["result"]["front"]
            .clone();
    let patched = answer(
        socket,
        95,
        json!({"cmd":"persona.update","params":{"id":persona,"patch":{"voice":{"providerId":"grok","modelId":front["modelId"],"voice":"rex"}}}}),
    )
    .await;
    assert_eq!(patched["ok"], true, "{patched}");

    let call = Uuid::new_v4().to_string();
    let started = answer(socket, 1, json!({"cmd":"voice.call_start","params":{"callId":call,"personaId":persona,"mode":"front"}})).await;
    assert_eq!(started["ok"], true, "{started}");
    send(socket, json!({"id":2,"sub":{"call":call}})).await;
    frame_where(socket, |f| f["sub"] == 2 && f["snapshot"].is_array()).await;
    common::until("the front is opened", || {
        desk.front.config.lock().unwrap().is_some()
    })
    .await;
    let config = desk.front.config.lock().unwrap().clone().unwrap();
    assert_eq!(config.voice, "rex", "the front follows the card's voice");
    let declared: Vec<&str> = config.tools.iter().map(|t| t.name.as_str()).collect();
    for tool in ["send_to_teammate", "search_thread", "post_to_chat"] {
        assert!(declared.contains(&tool), "{declared:?}");
    }
    let turns = desk.log.load(&tape).len();

    let call_tool = |id: &'static str, name: &'static str, args: Value| {
        let events = desk.events.clone();
        async move {
            events
                .send(Ok(RealtimeEvent::ToolCall {
                    call_id: id.into(),
                    name: name.into(),
                    arguments: args.to_string(),
                }))
                .await
                .unwrap();
        }
    };
    let speak = |heard: &'static str, said: &'static str| {
        let events = desk.events.clone();
        async move {
            events
                .send(Ok(RealtimeEvent::Heard(heard.into())))
                .await
                .unwrap();
            events
                .send(Ok(RealtimeEvent::Saying(said.into())))
                .await
                .unwrap();
            events.send(Ok(RealtimeEvent::ResponseDone)).await.unwrap();
        }
    };

    // A lookup: answered desk-side, nothing handed on.
    let answered = "We fixed the zebracrossing bug.";
    speak("what did we fix?", answered).await;
    call_tool("s1", "search_thread", json!({"query":"zebracrossing"})).await;
    let Asked::ToolResult { output, .. } = next_asked(&mut desk.asked).await else {
        panic!("the lookup went unanswered")
    };
    assert!(output.contains("zebracrossing"), "{output}");
    assert_eq!(desk.log.load(&tape).len(), turns, "a lookup is no hand-off");

    // A tool outside the teammate's reach is refused.
    call_tool("d1", "send_file", json!({"path":"x"})).await;
    let Asked::ToolResult { output, .. } = next_asked(&mut desk.asked).await else {
        panic!("the refusal went unanswered")
    };
    assert!(
        output.contains("There is no tool called send_file"),
        "{output}"
    );

    // One code-changing request is handed on.
    let handoff = "rename the scheduler module";
    speak(handoff, "On it.").await;
    call_tool("h1", "send_to_teammate", json!({"words": handoff})).await;
    let Asked::ToolResult { .. } = next_asked(&mut desk.asked).await else {
        panic!("the hand-off went unanswered")
    };

    // "Put that in the chat."
    call_tool(
        "p1",
        "post_to_chat",
        json!({"text":"the scheduler fix is in"}),
    )
    .await;
    let Asked::ToolResult { output, .. } = next_asked(&mut desk.asked).await else {
        panic!("the post went unanswered")
    };
    assert_eq!(output, "Posted to the chat.");

    frame_where(socket, |f| {
        f["event"]["type"] == "said" && f["event"]["text"] == "On it."
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
    common::until("the post and the call's summary", || {
        let added = desk.log.load(&tape).split_off(turns);
        added.iter().filter(|v| is_call_post(v)).count() >= 2
    })
    .await;
    let added = desk.log.load(&tape).split_off(turns);
    let users: Vec<&Value> = added.iter().filter(|v| v["kind"] == "user").collect();
    assert_eq!(users.len(), 1, "only the hand-off is a turn: {added:?}");
    assert_eq!(users[0]["text"], handoff);
    let posts: Vec<&str> = added
        .iter()
        .filter(|v| is_call_post(v))
        .map(|v| v["text"].as_str().unwrap())
        .collect();
    assert_eq!(posts.len(), 2, "the post and one summary: {added:?}");
    assert_eq!(posts[0], "the scheduler fix is in");
    let summary = posts[1];
    assert!(summary.starts_with("You asked"), "{summary}");
    assert!(
        summary.contains("what did we fix?") && summary.contains(answered),
        "{summary}"
    );
    for tool in ["search_thread", "send_to_teammate", "post_to_chat"] {
        assert!(!summary.contains(tool), "{summary}");
    }

    // The teammate's next typed turn carries on from the call.
    answer(
        socket,
        96,
        json!({"cmd":"session.prompt","params":{"personaId":persona,"text":"and the release?"}}),
    )
    .await;
    common::until("the follow-up turn is told what the call said", || {
        requests.lock().unwrap().iter().any(|request| {
            let body = request.to_string();
            body.contains("and the release?") && body.contains(answered)
        })
    })
    .await;
}
