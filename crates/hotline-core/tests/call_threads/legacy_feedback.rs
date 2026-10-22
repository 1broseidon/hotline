use super::super::*;

/// Both session drivers can ignore the requested opening. The shared Legacy
/// runtime still sends audio, then quiet progress, while real tool work waits.
#[tokio::test]
async fn tool_first_legacy_turns_get_a_cue_and_progress_on_both_drivers() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let gate = desk.root.path().join("finish-tool-turn");
        let grok = if matches!(driver, Driver::HotlineAgent) {
            Some(grok_tool_first(&mut desk, gate.clone()).await)
        } else {
            None
        };
        let (ada, _) = desk.teammate("Ada", driver, false).await;
        if grok.is_some() {
            let updated = desk
                .client
                .call(
                    "persona.update",
                    json!({
                        "id":ada,"patch":{"modelId":"xai/grok-4.7"}
                    }),
                )
                .await;
            assert_eq!(updated["ok"], true, "{updated}");
        }
        let (call, _, thread_sub) = desk.call_on(&ada, None).await;
        let voice_sub = desk.client.subscribe(json!({"call":call})).await;
        desk.say(
            &call,
            1,
            &format!(
                "REPORT_TOOLS\nCALL list_teammates {{}}\nWAIT_FOR {}\nREPLY_JSON \"The lookup is finished.\"",
                gate.display()
            ),
        )
        .await;
        desk.event(thread_sub, |event| event["kind"] == "tool")
            .await;
        // Kernel IO and the ACP child use real time; advance only the core's
        // feedback timer once the actual tool has reached the call thread.
        desk.advance(2).await;
        let cue = desk.event(voice_sub, |event| event["type"] == "clip").await;
        assert_eq!(cue["id"], "waiting-1");
        assert_eq!(cue["mimeType"], "audio/wav");
        let mut progress = Vec::new();
        for _ in 0..3 {
            for _ in 0..20 {
                desk.advance(1).await;
                tokio::task::yield_now().await;
            }
            let said = desk.event(voice_sub, |event| event["type"] == "said").await;
            let text = said["text"].as_str().unwrap();
            assert_ne!(text, "");
            progress.push(text.to_owned());
            let clip = desk.event(voice_sub, |event| event["type"] == "clip").await;
            assert_eq!(clip["id"], said["id"]);
            assert_ne!(clip["data"].as_str().unwrap(), "");
        }
        assert!(progress.windows(2).all(|pair| pair[0] != pair[1]));
        assert!(progress.iter().any(|line| line != &progress[0]));
        std::fs::write(gate, "ready").unwrap();
        let said = desk.event(voice_sub, |event| event["type"] == "said").await;
        assert_eq!(said["text"], "The lookup is finished.");
        desk.finished(thread_sub).await;
        desk.hang_up(&call, thread_sub).await;
    }
}

/// Exercise the native xAI Responses adapter with a credential-scoped local
/// origin. The model deliberately ignores its spoken-opening instruction.
async fn grok_tool_first(
    desk: &mut Desk,
    gate: std::path::PathBuf,
) -> hotline_core::provider_fixture::Origin {
    let app = axum::Router::new().route("/v1/responses", axum::routing::post(move |body: axum::body::Bytes| {
        let gate = gate.clone();
        async move {
            let request: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(request["model"], "grok-4.7");
            let answered = request["input"].as_array().unwrap().iter().any(|item| item["type"] == "function_call_output");
            let item = if answered {
                while !gate.exists() { tokio::time::sleep(Duration::from_millis(20)).await; }
                json!({"type":"message","id":"answer","status":"completed","role":"assistant","content":[{"type":"output_text","text":"The lookup is finished.","annotations":[]}]})
            } else {
                json!({"type":"function_call","id":"lookup","call_id":"lookup","name":"list_teammates","arguments":"{}","status":"completed"})
            };
            let events = [
                json!({"type":"response.output_item.done","output_index":0,"item":item}),
                json!({"type":"response.completed","response":{"id":"response","status":"completed","output":[item],"usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}}}),
            ];
            let body = events.iter().map(|event| format!("data: {event}\n\n")).collect::<String>();
            ([("Content-Type", "text/event-stream")], format!("{body}data: [DONE]\n\n"))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let key = uuid::Uuid::new_v4().to_string();
    let origin = hotline_core::provider_fixture::Origin::new(&key, &url);
    let saved = desk
        .client
        .call(
            "credential.create",
            json!({"providerId":"xai","label":"Grok fixture","secret":key}),
        )
        .await;
    assert_eq!(saved["ok"], true, "{saved}");
    origin
}
