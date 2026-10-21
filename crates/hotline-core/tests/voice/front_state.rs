use super::*;

async fn refreshed(asked: &mut tokio::sync::mpsc::Receiver<Asked>, contains: &str) -> String {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Some(Asked::Instructions(text)) = asked.recv().await
                && text.contains(contains)
            {
                return text;
            }
        }
    })
    .await
    .expect("the live state was not refreshed")
}

#[tokio::test]
async fn live_state_tracks_running_work_and_only_posts_that_reached_the_chat() {
    use axum::{Router, body::Bytes, routing::post};
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let wait = gate.clone();
    let app = Router::new().route("/v1/chat/completions", post(move |body: Bytes| {
        let wait = wait.clone();
        async move {
            let request: Value = serde_json::from_slice(&body).unwrap();
            let posted = request.to_string().contains("Posted to the chat.");
            let delta = if posted {
                json!({"role":"assistant", "content":"<spoken>The parser report is ready.</spoken><written>The parser report is ready.</written>"})
            } else {
                wait.acquire().await.unwrap().forget();
                json!({"role":"assistant", "tool_calls":[{"index":0,"id":"post-report","type":"function","function":{"name":"post_to_chat","arguments":json!({"text":"Parser report: done. sk-private204 ```hidden code```"}).to_string()}}]})
            };
            let chunk = json!({"id":"state-fixture","object":"chat.completion.chunk","created":1,"model":"test","choices":[{"index":0,"delta":delta,"finish_reason":null}]});
            let done = json!({"id":"state-fixture","object":"chat.completion.chunk","created":1,"model":"test","choices":[{"index":0,"delta":{},"finish_reason":if posted {"stop"} else {"tool_calls"}}]});
            ([("Content-Type", "text/event-stream")], format!("data: {chunk}\n\ndata: {done}\n\ndata: [DONE]\n\n"))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut desk = front_desk(true, 64).await;
    let saved = answer(&mut desk.socket, 1, json!({"cmd":"credential.custom_save","params":{"draft":{"name":"State fixture","baseUrl":provider,"api":"chat_completions","models":["test"],"secret":"fixture-key"}}})).await;
    assert_eq!(saved["ok"], true, "{saved}");
    let persona = answer(&mut desk.socket, 2, json!({"cmd":"persona.create","params":{"draft":{"name":"Ada","goal":"Brisk and precise","cwd":desk.root.path().to_str().unwrap()}}})).await["result"]["id"].as_str().unwrap().to_string();
    let call = Uuid::new_v4().to_string();
    let started = answer(&mut desk.socket, 3, json!({"cmd":"voice.call_start","params":{"callId":call,"personaId":persona,"mode":"front"}})).await;
    assert_eq!(started["ok"], true, "{started}");
    desk.events
        .send(Ok(RealtimeEvent::Heard("Write the parser report.".into())))
        .await
        .unwrap();
    desk.events
        .send(Ok(RealtimeEvent::Saying(
            "Writing the parser report.".into(),
        )))
        .await
        .unwrap();
    desk.events
        .send(Ok(RealtimeEvent::ResponseDone))
        .await
        .unwrap();
    desk.events
        .send(Ok(RealtimeEvent::ToolCall {
            call_id: "work".into(),
            name: "work_on_it".into(),
            arguments: json!({"words":"Write the parser report."}).to_string(),
        }))
        .await
        .unwrap();
    let running = refreshed(&mut desk.asked, "Work running: true").await;
    assert!(running.contains("Nothing posted."));
    assert!(running.contains(
        "Already said (including acknowledgements; never reuse an acknowledgement):\nWriting the parser report."
    ));
    gate.add_permits(1);
    let posted = refreshed(&mut desk.asked, "Actually posted to chat from this call (do not repeat a pointer already said):\nParser report: done.").await;
    assert!(!posted.contains("sk-private204"));
    assert!(!posted.contains("hidden code"));
    assert!(
        desk.log
            .load(&hotline_core::log::StreamId::Tape(persona))
            .iter()
            .any(|line| is_call_post(line)
                && line["text"]
                    .as_str()
                    .is_some_and(|text| text.starts_with("Parser report:")))
    );
    if !posted.contains("Work running: false") {
        refreshed(&mut desk.asked, "Work running: false").await;
    }
    answer(
        &mut desk.socket,
        4,
        json!({"cmd":"voice.call_end","params":{"callId":call}}),
    )
    .await;
    server.abort();
}
