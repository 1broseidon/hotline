use super::*;

async fn tool(desk: &mut FrontDesk, name: &str, arguments: Value) -> String {
    desk.events
        .send(Ok(RealtimeEvent::ToolCall {
            call_id: name.into(),
            name: name.into(),
            arguments: arguments.to_string(),
        }))
        .await
        .unwrap();
    loop {
        if let Asked::ToolResult {
            call_id, output, ..
        } = next_asked(&mut desk.asked).await
        {
            assert_eq!(call_id, name);
            return output;
        }
    }
}

#[tokio::test]
async fn the_desk_lead_looks_up_the_roster_and_hands_named_words_straight_on() {
    let mut desk = front_desk(true, 64).await;
    let (provider, requests) = provider_seeing("The named work is done.").await;
    let saved = answer(&mut desk.socket, 1, json!({"cmd":"credential.custom_save","params":{"draft":{"name":"Lead fixture","baseUrl":provider,"api":"chat_completions","models":["test"],"secret":"fixture-key"}}})).await;
    assert_eq!(saved["ok"], true);
    let mut team = Vec::new();
    for name in ["Ada", "Mack"] {
        let made = answer(&mut desk.socket, 2, json!({"cmd":"persona.create","params":{"draft":{"name":name,"cwd":desk.root.path().to_str().unwrap()}}})).await;
        assert_eq!(made["ok"], true, "{made}");
        team.push(made["result"]["id"].as_str().unwrap().to_string());
    }
    let updated = answer(&mut desk.socket, 3, json!({"cmd":"persona.update","params":{"id":team[0],"patch":{"voice":{"providerId":"grok","modelId":"front","voice":"ara"}}}})).await;
    assert_eq!(updated["ok"], true, "{updated}");
    let call = Uuid::new_v4().to_string();
    let started = answer(
        &mut desk.socket,
        4,
        json!({"cmd":"voice.call_start","params":{"callId":call,"mode":"front"}}),
    )
    .await;
    assert_eq!(started["ok"], true, "{started}");
    assert_eq!(started["result"]["personaId"], team[0]);
    common::until("the lead's front", || {
        desk.front.config.lock().unwrap().is_some()
    })
    .await;
    let config = desk.front.config.lock().unwrap().clone().unwrap();
    assert!(config.instructions.contains("You are Ada."));
    assert!(config.instructions.contains("team lead"));
    assert_eq!(config.voice, "ara");
    assert!(config.tools.iter().any(|t| t.name == "message_teammate"));
    desk.events
        .send(Ok(RealtimeEvent::Heard("Who is on the team?".into())))
        .await
        .unwrap();
    let roster = tool(&mut desk, "list_teammates", json!({})).await;
    assert!(roster.contains("Mack"), "{roster}");
    desk.events
        .send(Ok(RealtimeEvent::Saying("Mack is on the team.".into())))
        .await
        .unwrap();
    desk.events
        .send(Ok(RealtimeEvent::ResponseDone))
        .await
        .unwrap();
    assert!(
        requests.lock().unwrap().is_empty(),
        "a lookup starts no working model or dispatcher"
    );
    let words = "Mack, check the zebra scheduler and keep the report short.";
    desk.events
        .send(Ok(RealtimeEvent::Heard(words.into())))
        .await
        .unwrap();
    let sent = tool(
        &mut desk,
        "message_teammate",
        json!({"to":"Mack","message":words,"intent":"handoff"}),
    )
    .await;
    assert!(sent.contains("\"sent\":true"), "{sent}");
    assert!(sent.contains("Mack"));
    desk.events
        .send(Ok(RealtimeEvent::Saying("Mack has it.".into())))
        .await
        .unwrap();
    desk.events
        .send(Ok(RealtimeEvent::ResponseDone))
        .await
        .unwrap();
    common::until("the named work", || !requests.lock().unwrap().is_empty()).await;
    assert_eq!(desk.dispatcher_calls.load(Ordering::SeqCst), 0);
    let room = desk.log.load(&hotline_core::log::StreamId::Room);
    let request = room
        .iter()
        .filter(|e| e["kind"] == "exchange_pair")
        .flat_map(|e| e["requests"].as_array().unwrap())
        .find(|r| r["message"] == words)
        .unwrap();
    assert_eq!(request["from"], team[0]);
    assert_eq!(request["to"], team[1]);
    assert_eq!(request["intent"], "handoff");
    assert_eq!(request["lead_call"]["key"], call);
    assert!(
        !desk
            .log
            .load(&hotline_core::log::StreamId::Tape(team[0].clone()))
            .iter()
            .any(|e| e["kind"] == "human_action"),
        "no first-contact card"
    );
    let transcript = desk
        .log
        .load(&hotline_core::log::StreamId::Call(call.clone()));
    assert!(transcript.iter().any(|e| e["title"] == "list_teammates"));
    assert_eq!(
        desk.log.load(&hotline_core::log::StreamId::Tape(
            "voice-dispatcher".into()
        )),
        Vec::<Value>::new()
    );
    let refused = tool(
        &mut desk,
        "message_teammate",
        json!({"to":"Mack","message":"a question","intent":"ask"}),
    )
    .await;
    assert!(refused.contains("only hand named work"), "{refused}");
    let refused = tool(&mut desk, "session.stop", json!({"personaId":team[1]})).await;
    assert!(refused.contains("no tool"), "{refused}");
    let changed = answer(
        &mut desk.socket,
        5,
        json!({"cmd":"persona.lead","params":{"id":team[1]}}),
    )
    .await;
    assert_eq!(changed["ok"], true);
    let refused = tool(
        &mut desk,
        "message_teammate",
        json!({"to":"Mack","message":"do more","intent":"handoff"}),
    )
    .await;
    assert!(refused.contains("no tool"), "{refused}");
    answer(
        &mut desk.socket,
        6,
        json!({"cmd":"voice.call_end","params":{"callId":call}}),
    )
    .await;
}

#[tokio::test]
async fn a_direct_call_to_the_lead_does_not_get_the_desks_handoff_grant() {
    let mut desk = front_desk(true, 64).await;
    let made = answer(&mut desk.socket, 1, json!({"cmd":"persona.create","params":{"draft":{"name":"Ada","cwd":desk.root.path().to_str().unwrap()}}})).await;
    let id = made["result"]["id"].as_str().unwrap();
    let call = Uuid::new_v4().to_string();
    let started = answer(
        &mut desk.socket,
        2,
        json!({"cmd":"voice.call_start","params":{"callId":call,"personaId":id,"mode":"front"}}),
    )
    .await;
    assert_eq!(started["ok"], true);
    common::until("the direct front", || {
        desk.front.config.lock().unwrap().is_some()
    })
    .await;
    assert!(
        !desk
            .front
            .config
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .tools
            .iter()
            .any(|t| t.name == "message_teammate")
    );
    let refused = tool(
        &mut desk,
        "message_teammate",
        json!({"to":"Mack","message":"work","intent":"handoff"}),
    )
    .await;
    assert!(refused.contains("no tool"), "{refused}");
    answer(
        &mut desk.socket,
        3,
        json!({"cmd":"voice.call_end","params":{"callId":call}}),
    )
    .await;
}

#[tokio::test]
async fn a_no_lead_front_knows_the_person_and_roster() {
    let mut desk = front_desk(true, 64).await;
    for name in ["Ada", "Mack"] {
        answer(&mut desk.socket, 1, json!({"cmd":"persona.create","params":{"draft":{"name":name,"cwd":desk.root.path().to_str().unwrap()}}})).await;
    }
    assert_eq!(
        answer(
            &mut desk.socket,
            2,
            json!({"cmd":"persona.lead","params":{}})
        )
        .await["ok"],
        true
    );
    assert_eq!(
        answer(
            &mut desk.socket,
            3,
            json!({"cmd":"settings.update","params":{"patch":{"personName":"George"}}})
        )
        .await["ok"],
        true
    );
    let call = Uuid::new_v4().to_string();
    assert_eq!(
        answer(
            &mut desk.socket,
            4,
            json!({"cmd":"voice.call_start","params":{"callId":call,"mode":"front"}})
        )
        .await["ok"],
        true
    );
    common::until("the fallback front", || {
        desk.front.config.lock().unwrap().is_some()
    })
    .await;
    let config = desk.front.config.lock().unwrap().clone().unwrap();
    for text in ["George", "Ada", "Mack"] {
        assert!(config.instructions.contains(text), "{text}");
    }
    assert!(!config.tools.iter().any(|t| t.name == "message_teammate"));
    answer(
        &mut desk.socket,
        5,
        json!({"cmd":"voice.call_end","params":{"callId":call}}),
    )
    .await;
}
