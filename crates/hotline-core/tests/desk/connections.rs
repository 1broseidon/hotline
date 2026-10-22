use super::*;

/// Local discovery, filtering and a streamed Rig turn use the same wire as
/// Settings and the conversation. No installed Ollama or cloud account needed.
#[tokio::test(flavor = "multi_thread")]
async fn ollama_local_discovers_custom_models_and_runs_through_rig() {
    use axum::{
        Router,
        body::Bytes,
        http::HeaderMap,
        routing::{get, post},
    };
    use std::sync::Mutex;
    let installed = Arc::new(Mutex::new(vec!["custom/coder:latest".to_string()]));
    let tags = installed.clone();
    let (requests_tx, mut requests_rx) = tokio::sync::mpsc::channel(4);
    let app = Router::new()
        .route("/api/tags", get(move |headers: HeaderMap| {
            assert!(!headers.contains_key("authorization"));
            let tags = tags.clone();
            async move {
                json!({"models": tags.lock().unwrap().iter().map(|id| json!({"name": id, "model": id})).collect::<Vec<_>>()}).to_string()
            }
        }))
        .route("/api/chat", post(move |headers: HeaderMap, bytes: Bytes| {
            assert!(!headers.contains_key("authorization"));
            let tx = requests_tx.clone();
            async move {
                let request: Value = serde_json::from_slice(&bytes).unwrap();
                tx.send(request).await.unwrap();
                let chunk = json!({"model":"custom/coder:latest", "created_at":"2026-09-09T00:00:00Z",
                    "message":{"role":"assistant", "content":"Hello from Ollama."}, "done":false});
                let done = json!({"model":"custom/coder:latest", "created_at":"2026-09-09T00:00:00Z",
                    "message":{"role":"assistant", "content":""}, "done":true, "done_reason":"stop",
                    "prompt_eval_count":10, "eval_count":4});
                ([("Content-Type", "application/x-ndjson")], format!("{chunk}\n{done}\n"))
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let (root, port) = open("ollama-local").await;
    let mut client = Client::connect(port, TOKEN).await;
    let providers = client.call("providers.list", json!({})).await;
    let providers = providers["result"].as_array().unwrap();
    assert_eq!(
        providers.iter().find(|p| p["id"] == "openrouter").unwrap()["credentialKinds"],
        json!(["oauth", "api_key"])
    );
    assert_eq!(
        providers.iter().find(|p| p["id"] == "ollama").unwrap()["credentialKinds"],
        json!(["local"])
    );
    assert_eq!(
        providers
            .iter()
            .find(|p| p["id"] == "ollama-cloud")
            .unwrap()["credentialKinds"],
        json!(["api_key"])
    );
    let bad = client
        .call(
            "credential.connect_local",
            json!({"baseUrl":"http://user:password@localhost"}),
        )
        .await;
    assert_eq!(bad["ok"], false, "{bad}");
    assert_eq!(
        client.call("credential.list", json!({})).await["result"],
        json!([])
    );
    let connected = client
        .call(
            "credential.connect_local",
            json!({"baseUrl":format!("{url}/")}),
        )
        .await;
    assert_eq!(connected["ok"], true, "{connected}");
    let credential = connected["result"].clone();
    assert_eq!(credential["credentialKind"], "local");
    assert_eq!(credential["baseUrl"], url);
    assert!(!root.join("vault/secrets.json").exists());
    let models = client.call("models.list", json!({})).await;
    assert_eq!(models["result"][0]["id"], "ollama/custom/coder:latest");
    let cached = root
        .join("vault/logins")
        .join(credential["id"].as_str().unwrap())
        .join("models.json");
    let reopened = common::open_desk(&root).unwrap();
    assert_eq!(
        hotline_core::wire::RoomHandle::models(&reopened)[0].id,
        "ollama/custom/coder:latest"
    );
    drop(reopened);
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let made = client.call("persona.create", json!({"draft":{"name":"Ollama tester", "goal":"Answer plainly.", "cwd":workspace.to_string_lossy()}})).await;
    let persona = made["result"]["id"].as_str().unwrap();
    let tape = client.subscribe(json!({"tape":persona})).await;
    let started = client
        .call("session.start", json!({"personaId":persona}))
        .await;
    assert_eq!(started["ok"], true, "{started}");
    let sent = client
        .call(
            "session.prompt",
            json!({"personaId":persona,"text":"Say hello."}),
        )
        .await;
    assert_eq!(sent["ok"], true, "{sent}");
    let request = tokio::time::timeout(Duration::from_secs(15), requests_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request["model"], "custom/coder:latest");
    assert_eq!(request["stream"], true);
    assert_ne!(
        request["tools"].as_array().unwrap().as_slice(),
        [] as [Value; 0]
    );
    let answer = client
        .next_where(Duration::from_secs(15), |frame| {
            is_sub(frame, tape, "event") && frame["event"]["kind"] == "agent"
        })
        .await;
    assert!(
        answer["event"]["text"]
            .as_str()
            .unwrap()
            .contains("Hello from Ollama."),
        "{answer}"
    );
    installed.lock().unwrap().push("second:cloud".into());
    let refreshed = client
        .call("credential.refresh_models", json!({"providerId":"ollama"}))
        .await;
    assert_eq!(
        refreshed["result"].as_array().unwrap().len(),
        2,
        "{refreshed}"
    );
    client
        .call(
            "settings.update",
            json!({"patch":{"enabledModels":{"ollama":["second:cloud"]}}}),
        )
        .await;
    let filtered = client.call("models.list", json!({})).await;
    assert_eq!(
        filtered["result"].as_array().unwrap().len(),
        1,
        "{filtered}"
    );
    assert_eq!(filtered["result"][0]["id"], "ollama/second:cloud");
    server.abort();
    let _ = server.await;
    let failed = client
        .call("credential.refresh_models", json!({"providerId":"ollama"}))
        .await;
    assert_eq!(failed["ok"], false);
    let preserved = client
        .call("models.catalog", json!({"providerId":"ollama"}))
        .await;
    assert_eq!(preserved["result"].as_array().unwrap().len(), 2);
    client
        .call("credential.delete", json!({"id":credential["id"]}))
        .await;
    assert!(!cached.exists());
    assert_eq!(
        client.call("models.list", json!({})).await["result"],
        json!([])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn custom_connections_keep_models_keys_and_edits_separate() {
    use axum::{Router, http::HeaderMap, routing::get};
    let app = Router::new()
        .route("/local/v1/models", get(|headers: HeaderMap| async move {
            assert!(!headers.contains_key("authorization"));
            json!({"object":"list","data":[{"id":"vendor/coder","object":"model","created":0,"owned_by":"local"}]}).to_string()
        }))
        .route("/cloud/v1/models", get(|headers: HeaderMap| async move {
            assert_eq!(headers["authorization"], "Bearer custom-test-key");
            json!({"object":"list","data":[{"id":"vendor/coder","object":"model","created":0,"owned_by":"cloud"}]}).to_string()
        }))
        .route("/redirect/v1/models", get(|| async {
            axum::response::Redirect::temporary("/cloud/v1/models")
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let (root, port) = open("custom-connections").await;
    let mut client = Client::connect(port, TOKEN).await;
    for url in [
        "ftp://localhost/v1",
        "http://user:password@localhost/v1",
        "http://localhost/v1?api_key=secret",
        "http://localhost/v1#fragment",
    ] {
        let rejected = client
            .call("credential.custom_models", json!({"baseUrl":url}))
            .await;
        assert_eq!(rejected["ok"], false, "{rejected}");
        assert!(!rejected.to_string().contains(url));
    }
    let redirected = client
        .call(
            "credential.custom_models",
            json!({"baseUrl":format!("{base}/redirect/v1"),"secret":"custom-test-key"}),
        )
        .await;
    assert_eq!(
        redirected["ok"], false,
        "a key must not follow an endpoint redirect: {redirected}"
    );
    let mut saved = Vec::new();
    for (name, api, secret) in [
        ("local", "responses", ""),
        ("cloud", "chat_completions", "custom-test-key"),
    ] {
        let url = format!("{base}/{name}/v1");
        let discovered = client
            .call(
                "credential.custom_models",
                json!({"baseUrl":url,"secret":secret}),
            )
            .await;
        assert_eq!(
            discovered["result"],
            json!(["vendor/coder"]),
            "{discovered}"
        );
        let made = client.call("credential.custom_save", json!({"draft":{"name":name,"baseUrl":url,"api":api,"secret":secret,"models":["vendor/coder"]}})).await;
        assert_eq!(made["ok"], true, "{made}");
        saved.push(made["result"].clone());
    }
    assert_ne!(saved[0]["providerId"], saved[1]["providerId"]);
    let choices = client.call("models.list", json!({})).await;
    let choices = choices["result"].as_array().unwrap();
    assert_eq!(choices.len(), 2);
    assert_ne!(choices[0]["id"], choices[1]["id"]);
    assert!(
        !client
            .call("credential.list", json!({}))
            .await
            .to_string()
            .contains("custom-test-key")
    );
    let id = saved[1]["id"].as_str().unwrap();
    let auth = root.join("vault/logins").join(id).join("auth.json");
    assert!(auth.exists());
    let wrong_url = format!("{base}/local/v1");
    let refused = client
        .call(
            "credential.custom_models",
            json!({"id":id,"baseUrl":wrong_url}),
        )
        .await;
    assert_eq!(refused["ok"], false, "{refused}");
    let refused_edit = client.call("credential.custom_save", json!({"id":id,"draft":{"name":"Changed endpoint","baseUrl":wrong_url,"api":"responses","models":["another-model"]}})).await;
    assert_eq!(refused_edit["ok"], false, "{refused_edit}");
    assert_eq!(
        client.call("credential.list", json!({})).await["result"],
        json!(saved)
    );
    let failed = client
        .call(
            "credential.custom_models",
            json!({"baseUrl":format!("{base}/missing"),"secret":""}),
        )
        .await;
    assert_eq!(failed["ok"], false);
    let edited = client.call("credential.custom_save", json!({"id":id,"draft":{"name":"Renamed cloud","baseUrl":saved[1]["baseUrl"],"api":"responses","models":["vendor/manual"]}})).await;
    assert_eq!(edited["result"]["id"], id, "{edited}");
    assert_eq!(edited["result"]["providerId"], saved[1]["providerId"]);
    let discovered = client
        .call(
            "credential.custom_models",
            json!({"id":id,"baseUrl":saved[1]["baseUrl"]}),
        )
        .await;
    assert_eq!(discovered["ok"], true, "{discovered}");
    let cleared = client.call("credential.custom_save", json!({"id":id,"draft":{"name":"Keyless","baseUrl":wrong_url,"api":"responses","secret":"","models":["vendor/manual"]}})).await;
    assert_eq!(cleared["result"]["credentialKind"], "local", "{cleared}");
    assert!(!auth.exists());
    let reopened = common::open_desk(&root).unwrap();
    assert_eq!(hotline_core::wire::RoomHandle::models(&reopened).len(), 2);
    drop(reopened);
    client.call("credential.delete", json!({"id":id})).await;
    assert_eq!(
        client.call("models.list", json!({})).await["result"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let stale = client
        .call(
            "credential.custom_models",
            json!({"id":id,"baseUrl":wrong_url}),
        )
        .await;
    assert_eq!(stale["ok"], false);
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn custom_connections_roundtrip_tools_through_both_native_rig_apis() {
    use axum::{Router, body::Bytes, http::HeaderMap, routing::post};
    for (api, secret) in [
        ("responses", ""),
        ("responses", "custom-inference-key"),
        ("chat_completions", ""),
        ("chat_completions", "custom-inference-key"),
    ] {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let path = if api == "responses" {
            "/prefix/v1/responses"
        } else {
            "/prefix/v1/chat/completions"
        };
        let app = Router::new().route(
            path,
            post(move |headers: HeaderMap, bytes: Bytes| {
                let tx = tx.clone();
                async move {
                    if secret.is_empty() {
                        assert!(!headers.contains_key("authorization"));
                    } else {
                        assert_eq!(headers["authorization"], format!("Bearer {secret}"));
                    }
                    let request = serde_json::from_slice::<Value>(&bytes).unwrap();
                    let has_tool_result = request.to_string().contains("harbour is open");
                    tx.send(request).await.unwrap();
                    let events = custom_stream_events(api, has_tool_result);
                    let body = events
                        .into_iter()
                        .map(|event| format!("data: {event}\n\n"))
                        .collect::<String>();
                    (
                        [("Content-Type", "text/event-stream")],
                        format!("{body}data: [DONE]\n\n"),
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/prefix/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let (root, port) = open(&format!("custom-stream-{api}-{}", !secret.is_empty())).await;
        let mut client = Client::connect(port, TOKEN).await;
        let made = client.call("credential.custom_save", json!({"draft":{"name":api,"baseUrl":url,"api":api,"secret":secret,"models":["vendor/coder"]}})).await;
        assert_eq!(made["ok"], true, "{made}");
        let workspace = root.join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::write(workspace.join("note.txt"), "The harbour is open.\n").unwrap();
        let made = client.call("persona.create", json!({"draft":{"name":"Custom tester","goal":"Answer plainly.","cwd":workspace.to_string_lossy()}})).await;
        let persona = made["result"]["id"].as_str().unwrap();
        let tape = client.subscribe(json!({"tape":persona})).await;
        assert_eq!(
            client
                .call("session.start", json!({"personaId":persona}))
                .await["ok"],
            true
        );
        assert_eq!(
            client
                .call(
                    "session.prompt",
                    json!({"personaId":persona,"text":"Read note.txt and report its contents."})
                )
                .await["ok"],
            true
        );
        let request = tokio::time::timeout(Duration::from_secs(15), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(request["model"], "vendor/coder");
        assert_eq!(request["stream"], true);
        assert!(
            request["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| { tool["name"] == "read" || tool["function"]["name"] == "read" })
        );
        let resumed = tokio::time::timeout(Duration::from_secs(15), rx.recv())
            .await
            .expect("the executed tool must resume the model")
            .unwrap();
        assert_eq!(resumed["model"], "vendor/coder");
        assert_eq!(resumed["stream"], true);
        if api == "responses" {
            let input = resumed["input"].as_array().unwrap();
            let call = input
                .iter()
                .find(|item| item["type"] == "function_call")
                .unwrap();
            assert_eq!(call["call_id"], "call_read");
            assert_eq!(call["name"], "read");
            assert_eq!(
                serde_json::from_str::<Value>(call["arguments"].as_str().unwrap()).unwrap(),
                json!({"path":"note.txt"})
            );
            let result = input
                .iter()
                .find(|item| item["type"] == "function_call_output")
                .unwrap();
            assert_eq!(result["call_id"], "call_read");
            assert!(
                result["output"]
                    .to_string()
                    .contains("The harbour is open.")
            );
        } else {
            let messages = resumed["messages"].as_array().unwrap();
            let call = &messages
                .iter()
                .find(|item| item["tool_calls"].is_array())
                .unwrap()["tool_calls"][0];
            assert_eq!(call["id"], "call_read");
            assert_eq!(call["function"]["name"], "read");
            assert_eq!(
                serde_json::from_str::<Value>(call["function"]["arguments"].as_str().unwrap())
                    .unwrap(),
                json!({"path":"note.txt"})
            );
            let result = messages.iter().find(|item| item["role"] == "tool").unwrap();
            assert_eq!(result["tool_call_id"], "call_read");
            assert!(
                result["content"]
                    .to_string()
                    .contains("The harbour is open.")
            );
        }
        let completed = client
            .next_where(Duration::from_secs(15), |frame| {
                is_sub(frame, tape, "event")
                    && frame["event"]["kind"] == "tool"
                    && frame["event"]["status"] == "completed"
            })
            .await;
        assert_eq!(completed["event"]["toolKind"], "read");
        assert!(
            completed["event"]["output"]
                .to_string()
                .contains("The harbour is open.")
        );
        let answer = client
            .next_where(Duration::from_secs(15), |frame| {
                is_sub(frame, tape, "event") && frame["event"]["kind"] == "agent"
            })
            .await;
        assert!(
            answer["event"]["text"]
                .as_str()
                .unwrap()
                .contains("The harbour is open."),
            "{api}: {answer}"
        );
        let turn = client
            .next_where(Duration::from_secs(15), |frame| {
                is_sub(frame, tape, "event") && frame["event"]["kind"] == "turn"
            })
            .await;
        assert_eq!(turn["event"]["stopReason"], "end_turn");
        client
            .call("session.stop", json!({"personaId":persona}))
            .await;
        server.abort();
    }
}

/// The provider emits one fragmented read call, then answers only once its
/// next request contains the file contents from Hotline's actual workspace tool.
fn custom_stream_events(api: &str, has_tool_result: bool) -> Vec<Value> {
    if api == "responses" {
        let output = if has_tool_result {
            json!({"type":"message","id":"msg_1","status":"completed","role":"assistant","content":[{"type":"output_text","text":"The harbour is open.","annotations":[]}]})
        } else {
            json!({"type":"function_call","id":"fc_read","call_id":"call_read","name":"read","arguments":"{\"path\":\"note.txt\"}","status":"completed"})
        };
        let mut events = if has_tool_result {
            vec![
                json!({"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"sequence_number":1,"delta":"The harbour is open."}),
            ]
        } else {
            vec![
                json!({"type":"response.output_item.added","output_index":0,"sequence_number":1,"item":{"type":"function_call","id":"fc_read","call_id":"call_read","name":"read","arguments":"","status":"in_progress"}}),
                json!({"type":"response.function_call_arguments.delta","item_id":"fc_read","output_index":0,"sequence_number":2,"delta":"{\"path\":"}),
                json!({"type":"response.function_call_arguments.delta","item_id":"fc_read","output_index":0,"sequence_number":3,"delta":"\"note.txt\"}"}),
                json!({"type":"response.output_item.done","output_index":0,"sequence_number":4,"item":output}),
            ]
        };
        events.push(json!({"type":"response.completed","sequence_number":5,"response":{"id":"resp_1","object":"response","created_at":1,"status":"completed","model":"vendor/coder","output":[output],"usage":{"input_tokens":3,"output_tokens":4,"total_tokens":7}}}));
        events
    } else {
        let mut events = if has_tool_result {
            vec![
                json!({"id":"chat_1","object":"chat.completion.chunk","created":1,"model":"vendor/coder","choices":[{"index":0,"delta":{"role":"assistant","content":"The harbour is open."},"finish_reason":null}]}),
            ]
        } else {
            vec![
                json!({"id":"chat_1","object":"chat.completion.chunk","created":1,"model":"vendor/coder","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_read","type":"function","function":{"name":"read","arguments":"{\"path\":"}}]},"finish_reason":null}]}),
                json!({"id":"chat_1","object":"chat.completion.chunk","created":1,"model":"vendor/coder","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"note.txt\"}"}}]},"finish_reason":null}]}),
            ]
        };
        events.push(json!({"id":"chat_1","object":"chat.completion.chunk","created":1,"model":"vendor/coder","choices":[{"index":0,"delta":{},"finish_reason":if has_tool_result { "stop" } else { "tool_calls" }}]}));
        events
    }
}

/// Compatible servers (Meta's Muse among them) stream Responses frames more
/// loosely than OpenAI: no sequence numbers, output items without an id or
/// status, a float `created_at`, `instructions` echoed as input items and an
/// empty `error` object. A turn over such a stream still answers.
#[tokio::test(flavor = "multi_thread")]
async fn custom_responses_connections_read_loosely_shaped_streams() {
    use axum::{Router, routing::post};
    let response = |status: &str, output: Value| {
        json!({"id":"resp_1","object":"response","created_at":1_791_871_040.25,"status":status,"error":{},"incomplete_details":null,
            "instructions":[{"type":"message","role":"developer","content":[{"type":"input_text","text":"Answer plainly."}]}],
            "model":"vendor/coder","output":output,"tools":[{"type":"function","name":"read","description":null,"parameters":{}}],
            "usage":if status == "completed" { json!({"input_tokens":3,"output_tokens":4,"total_tokens":7}) } else { Value::Null }})
    };
    let message = json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"Loose but readable.","annotations":[]}]});
    let events = [
        json!({"type":"response.created","response":response("in_progress", json!([]))}),
        json!({"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","summary":null}}),
        json!({"type":"response.output_item.done","output_index":0,"item":{"type":"reasoning","summary":[]}}),
        json!({"type":"response.output_item.added","output_index":1,"item":{"type":"message","role":"assistant","content":[]}}),
        json!({"type":"response.output_text.delta","item_id":"item_1","output_index":1,"content_index":0,"delta":"Loose but readable."}),
        json!({"type":"response.output_item.done","output_index":1,"item":message}),
        json!({"type":"response.completed","response":response("completed", json!([{"type":"reasoning","summary":null}, message]))}),
    ];
    let body = events
        .iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect::<String>()
        + "data: [DONE]\n\n";
    let app = Router::new().route(
        "/v1/responses",
        post(move || {
            let body = body.clone();
            async move { ([("Content-Type", "text/event-stream")], body) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let (root, port) = open("custom-loose-responses").await;
    let mut client = Client::connect(port, TOKEN).await;
    let made = client.call("credential.custom_save", json!({"draft":{"name":"Loose","baseUrl":url,"api":"responses","secret":"","models":["vendor/coder"]}})).await;
    assert_eq!(made["ok"], true, "{made}");
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let made = client.call("persona.create", json!({"draft":{"name":"Loose tester","goal":"Answer plainly.","cwd":workspace.to_string_lossy()}})).await;
    let persona = made["result"]["id"].as_str().unwrap();
    let tape = client.subscribe(json!({"tape":persona})).await;
    assert_eq!(
        client
            .call("session.start", json!({"personaId":persona}))
            .await["ok"],
        true
    );
    assert_eq!(
        client
            .call(
                "session.prompt",
                json!({"personaId":persona,"text":"hello"})
            )
            .await["ok"],
        true
    );
    let answer = client
        .next_where(Duration::from_secs(15), |frame| {
            is_sub(frame, tape, "event") && frame["event"]["kind"] == "agent"
        })
        .await;
    assert_eq!(answer["event"]["text"], "Loose but readable.", "{answer}");
    let turn = client
        .next_where(Duration::from_secs(15), |frame| {
            is_sub(frame, tape, "event") && frame["event"]["kind"] == "turn"
        })
        .await;
    assert_eq!(turn["event"]["stopReason"], "end_turn", "{turn}");
    client
        .call("session.stop", json!({"personaId":persona}))
        .await;
    server.abort();
}

#[tokio::test]
async fn discovery_and_manual_ids_preserve_filters_selections_and_offline_data_over_the_wire() {
    use axum::{Router, routing::get};
    use hotline_core::wire::RoomHandle;
    use std::sync::Mutex;
    let ids = Arc::new(Mutex::new(vec!["original-coder".to_string()]));
    let served = ids.clone();
    let app = Router::new().route("/api/tags", get(move || {
        let served = served.clone();
        async move { json!({"models":served.lock().unwrap().iter().map(|id| json!({"name":id,"model":id})).collect::<Vec<_>>()}).to_string() }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let (root, port) = open("manual-discovery").await;
    let mut wire = Client::connect(port, TOKEN).await;
    let credential = wire
        .call("credential.connect_local", json!({"baseUrl":url}))
        .await;
    assert_eq!(credential["ok"], true, "{credential}");
    let original_credential = credential["result"].clone();
    let added = wire
        .call(
            "models.manual_set",
            json!({"providerId":"ollama", "modelIds":["manual-coder"]}),
        )
        .await;
    assert_eq!(added["ok"], true, "{added}");
    let manual = added["result"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["id"] == "manual-coder")
        .unwrap();
    assert_eq!(manual["manual"], true);
    assert_eq!(manual["metadataKnown"], false);
    assert!(manual.get("outputLimit").is_none());
    let settings = json!({"defaultModelId":"ollama/manual-coder", "lastModelId":"ollama/original-coder", "enabledModels":{"ollama":["original-coder", "manual-coder", "previously-hidden"]}});
    assert_eq!(
        wire.call("settings.update", json!({"patch":settings}))
            .await["ok"],
        true
    );
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let selected = wire.call("persona.create", json!({"draft":{"name":"Selected", "goal":"Keep my selected model.", "cwd":workspace, "modelId":"ollama/original-coder"}})).await;
    let default = wire
        .call(
            "persona.create",
            json!({"draft":{"name":"Default", "goal":"Keep the room model.", "cwd":workspace}}),
        )
        .await;
    *ids.lock().unwrap() = vec!["new-coder".into()];
    let refreshed = wire
        .call("credential.refresh_models", json!({"providerId":"ollama"}))
        .await;
    assert_eq!(refreshed["ok"], true, "{refreshed}");
    let models = refreshed["result"].as_array().unwrap();
    assert_eq!(
        models
            .iter()
            .map(|model| model["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["manual-coder", "new-coder"]
    );
    assert_eq!(models[1]["enabled"], false);
    assert_eq!(
        wire.call("credential.list", json!({})).await["result"][0],
        original_credential
    );
    let removed = wire
        .call(
            "models.manual_set",
            json!({"providerId":"ollama", "modelIds":[]}),
        )
        .await;
    assert_eq!(removed["ok"], true);
    assert_eq!(removed["result"].as_array().unwrap().len(), 1);
    assert_eq!(
        wire.call("models.list", json!({})).await["result"],
        json!([])
    );
    for (persona, model) in [
        (&selected, "ollama/original-coder"),
        (&default, "ollama/manual-coder"),
    ] {
        let started = wire
            .call(
                "session.start",
                json!({"personaId":persona["result"]["id"]}),
            )
            .await;
        assert_eq!(started["ok"], true, "{started}");
        assert_eq!(started["result"]["currentModelId"], model, "{started}");
        wire.call("session.stop", json!({"personaId":persona["result"]["id"]}))
            .await;
    }
    // Manual additions persist independently from discovery and its failure.
    let saved = wire
        .call(
            "models.manual_set",
            json!({"providerId":"ollama", "modelIds":["manual-coder"]}),
        )
        .await;
    server.abort();
    let _ = server.await;
    let failed = wire
        .call("credential.refresh_models", json!({"providerId":"ollama"}))
        .await;
    assert_eq!(failed["ok"], false);
    assert_eq!(
        wire.call("models.catalog", json!({"providerId":"ollama"}))
            .await["result"],
        saved["result"]
    );
    let reopened = common::open_desk(&root).unwrap();
    assert_eq!(
        serde_json::to_value(reopened.models_catalog("ollama").unwrap()).unwrap(),
        saved["result"]
    );
    let persisted = hotline_core::room::settings(&reopened.log);
    assert_eq!(persisted["defaultModelId"], settings["defaultModelId"]);
    assert_eq!(persisted["enabledModels"], settings["enabledModels"]);
    let dir = root
        .join("vault/logins")
        .join(original_credential["id"].as_str().unwrap());
    assert!(
        dir.join("models.json").exists(),
        "the original legacy data is retained"
    );
    assert!(dir.join("discovery.json").exists());
    assert!(dir.join("manual-models.json").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(dir.join("manual-models.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[tokio::test]
async fn native_connection_metadata_and_account_restrictions_survive_reopening_over_the_wire() {
    use hotline_core::{log::Log, wire::RoomHandle};
    let root = scratch("native-discovery-metadata");
    let log = Log::open(&root);
    let vault = common::open_vault(&root, log).unwrap();
    let anthropic = vault
        .create("anthropic", "Anthropic key", "private-test-key")
        .unwrap();
    let router = vault
        .create("openrouter", "Router key", "private-router-key")
        .unwrap();
    let (copilot_id, copilot_dir) = vault.begin_login().unwrap();
    vault
        .finish_login(&copilot_id, "github-copilot", "Copilot")
        .unwrap();
    let known = hotline_core::models::catalog().providers["anthropic"]
        .models
        .keys()
        .next()
        .unwrap()
        .clone();
    let dir = root.join("vault/logins").join(&anthropic.id);
    std::fs::create_dir_all(&dir).unwrap();
    let legacy = b"[\"legacy-model\"]\n";
    std::fs::write(dir.join("models.json"), legacy).unwrap();
    let discovery = serde_json::to_vec(&json!([
        {"id":known,"name":"Provider label","context_limit":99000,"output_limit":1234},
        {"id":"released-after-hotline","name":"New provider model","output_limit":4096}
    ]))
    .unwrap();
    std::fs::write(dir.join("discovery.json"), &discovery).unwrap();
    let router_dir = root.join("vault/logins").join(router.id);
    std::fs::create_dir_all(&router_dir).unwrap();
    std::fs::write(
        router_dir.join("models.json"),
        serde_json::to_vec(std::slice::from_ref(&known)).unwrap(),
    )
    .unwrap();
    std::fs::write(
        copilot_dir.join("models.json"),
        b"[\"account-only-new-model\"]",
    )
    .unwrap();
    drop(vault);
    let mut wire = Client::connect(open_at(&root), TOKEN).await;
    let providers = wire.call("providers.list", json!({})).await;
    for provider in providers["result"].as_array().unwrap() {
        let supported = [
            "anthropic",
            "openai",
            "openrouter",
            "google",
            "xai",
            "groq",
            "deepseek",
            "mistral",
            "openai-codex",
            "ollama",
            "ollama-cloud",
            "github-copilot",
        ]
        .contains(&provider["id"].as_str().unwrap());
        assert_eq!(provider["modelDiscovery"], supported);
    }
    let models = wire
        .call("models.catalog", json!({"providerId":"anthropic"}))
        .await;
    let exact = models["result"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["id"] == known)
        .unwrap();
    assert_eq!(exact["metadataKnown"], true);
    assert_eq!(exact["name"], "Provider label");
    assert_eq!(exact["contextLimit"], 99000);
    assert_eq!(exact["outputLimit"], 1234);
    assert!(exact["efforts"].is_array());
    let router = wire
        .call("models.catalog", json!({"providerId":"openrouter"}))
        .await;
    assert_eq!(
        router["result"][0]["metadataKnown"], false,
        "metadata must not join across providers"
    );
    let choices = wire.call("models.list", json!({})).await;
    let choice = choices["result"]
        .as_array()
        .unwrap()
        .iter()
        .find(|choice| choice["id"] == format!("anthropic/{known}"))
        .unwrap();
    assert_eq!(choice["name"], "Provider label");
    assert_eq!(choice["group"], "Anthropic");
    let account = wire
        .call("models.catalog", json!({"providerId":"github-copilot"}))
        .await;
    assert_eq!(account["result"].as_array().unwrap().len(), 1);
    assert_eq!(account["result"][0]["id"], "account-only-new-model");
    assert_eq!(account["result"][0]["metadataKnown"], false);
    let rejected = wire
        .call(
            "models.manual_set",
            json!({"providerId":"github-copilot", "modelIds":["outside-account"]}),
        )
        .await;
    assert_eq!(rejected["ok"], false);
    let allowed = wire
        .call(
            "models.manual_set",
            json!({"providerId":"github-copilot", "modelIds":["account-only-new-model"]}),
        )
        .await;
    assert_eq!(allowed["ok"], true, "{allowed}");
    assert_eq!(allowed["result"][0]["manual"], true);
    for (provider, id) in [
        ("anthropic", "bad\nmodel"),
        ("anthropic", "bad\u{202e}model"),
        ("anthropic", "../escape"),
        ("unwired-provider", "coder"),
        ("openai", "coder"),
    ] {
        let rejected = wire
            .call(
                "models.manual_set",
                json!({"providerId":provider, "modelIds":[id]}),
            )
            .await;
        assert_eq!(rejected["ok"], false, "{rejected}");
    }
    let manual = wire
        .call(
            "models.manual_set",
            json!({"providerId":"anthropic", "modelIds":["another-new-model"]}),
        )
        .await;
    assert_eq!(manual["ok"], true);
    let reopened = common::open_desk(&root).unwrap();
    assert_eq!(
        serde_json::to_value(reopened.models_catalog("anthropic").unwrap()).unwrap(),
        manual["result"]
    );
    assert_eq!(
        std::fs::read(dir.join("discovery.json")).unwrap(),
        discovery
    );
    assert_eq!(std::fs::read(dir.join("models.json")).unwrap(), legacy);
    assert!(
        !serde_json::to_string(&manual)
            .unwrap()
            .contains("private-test-key")
    );
}

/// A Hotline Agent teammate on a desk with no provider comes up saying why,
/// on its tape and before anything is sent, and a message is turned away
/// without being written. Connecting a provider is the whole fix: the next
/// message starts the teammate on it and is answered.
#[tokio::test(flavor = "multi_thread")]
async fn hotline_agent_with_no_provider_says_so_before_the_first_turn() {
    use axum::{
        Router,
        routing::{get, post},
    };
    let app = Router::new()
        .route(
            "/api/tags",
            get(|| async { json!({"models": [{"name": "coder", "model": "coder"}]}).to_string() }),
        )
        .route(
            "/api/chat",
            post(|| async {
                let chunk = json!({"model":"coder", "created_at":"2026-10-18T00:00:00Z",
                    "message":{"role":"assistant", "content":"Connected now."}, "done":false});
                let done = json!({"model":"coder", "created_at":"2026-10-18T00:00:00Z",
                    "message":{"role":"assistant", "content":""}, "done":true, "done_reason":"stop",
                    "prompt_eval_count":10, "eval_count":3});
                (
                    [("Content-Type", "application/x-ndjson")],
                    format!("{chunk}\n{done}\n"),
                )
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let (root, port) = open("no-provider").await;
    let mut client = Client::connect(port, TOKEN).await;
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let made = client
        .call(
            "persona.create",
            json!({"draft":{"name":"Unplugged", "goal":"Wait for a provider.", "cwd":workspace.to_string_lossy()}}),
        )
        .await;
    let persona = made["result"]["id"].as_str().unwrap().to_string();
    let tape = client.subscribe(json!({"tape": persona})).await;

    let started = client
        .call("session.start", json!({"personaId": persona}))
        .await;
    assert_eq!(started["ok"], true, "{started}");
    assert_eq!(started["result"]["state"], "error", "{started}");
    let notice = client
        .next_where(Duration::from_secs(5), |frame| {
            is_sub(frame, tape, "event") && frame["event"]["kind"] == "notice"
        })
        .await;
    assert_eq!(notice["event"]["level"], "error");
    let text = notice["event"]["text"].as_str().unwrap();
    assert!(
        text.starts_with("Hotline Agent unavailable: Connect a provider in Settings › Providers"),
        "{text}"
    );
    assert!(text.contains(r#""kind":"no_provider""#), "{text}");

    let refused = client
        .call(
            "session.prompt",
            json!({"personaId": persona, "text": "Are you there?"}),
        )
        .await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(
        refused["error"],
        "Hotline Agent unavailable: connect a provider in Settings › Providers."
    );
    // Not a failed turn: the words were never written, and no turn ran.
    let again = client.subscribe(json!({"tape": persona})).await;
    let snapshot = client
        .next_where(Duration::from_secs(5), |frame| {
            is_sub(frame, again, "snapshot")
        })
        .await;
    let kinds: Vec<&str> = snapshot["snapshot"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|event| event["kind"].as_str())
        .collect();
    assert!(
        !kinds.contains(&"user") && !kinds.contains(&"turn"),
        "{kinds:?}"
    );

    let connected = client
        .call("credential.connect_local", json!({"baseUrl": url}))
        .await;
    assert_eq!(connected["ok"], true, "{connected}");
    let sent = client
        .call(
            "session.prompt",
            json!({"personaId": persona, "text": "Are you there now?"}),
        )
        .await;
    assert_eq!(sent["ok"], true, "{sent}");
    let answer = client
        .next_where(Duration::from_secs(15), |frame| {
            is_sub(frame, tape, "event") && frame["event"]["kind"] == "agent"
        })
        .await;
    assert!(
        answer["event"]["text"]
            .as_str()
            .unwrap()
            .contains("Connected now."),
        "{answer}"
    );
    server.abort();
}

/// The real settings handler waits on the room's policy gate. Discovery on
/// the same socket still answers, and the following write keeps its place.
#[tokio::test]
async fn discovery_answers_while_a_settings_command_waits_on_the_policy_gate() {
    use hotline_core::wire::RoomHandle;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(common::open_desk(root.path()).unwrap());
    let gate = desk.policy_update_lock();
    let held = gate.lock().await;
    let door = Door::bind(desk.log.clone(), TOKEN.into(), desk.clone(), desk.parts()).unwrap();
    let port = door.port();
    let serving = tokio::spawn(door.run());
    let mut client = Client::connect(port, TOKEN).await;
    let first = client
        .send("settings.update", json!({"patch":{"theme":"dark"}}))
        .await;
    let second = client
        .send("settings.update", json!({"patch":{"theme":"light"}}))
        .await;
    let providers = client.call("providers.list", json!({})).await;
    assert_eq!(providers["ok"], true, "{providers}");
    assert_ne!(
        hotline_core::room::settings(&desk.log).get("theme"),
        Some(&json!("light"))
    );
    drop(held);
    assert_eq!(client.reply(first, common::ANSWER).await["ok"], true);
    assert_eq!(client.reply(second, common::ANSWER).await["ok"], true);
    assert_eq!(
        hotline_core::room::settings(&desk.log).get("theme"),
        Some(&json!("light"))
    );
    serving.abort();
}

/// Lag recovery on the real core keeps a client's recent history and
/// resumes live delivery on the same connection.
#[tokio::test]
async fn a_lagging_client_recovers_its_window_and_keeps_receiving_the_core() {
    use hotline_core::log::StreamId;
    let root = tempfile::tempdir().unwrap();
    let desk = Arc::new(common::open_desk(root.path()).unwrap());
    let tape = StreamId::Tape("ada".into());
    for n in 0..300 {
        desk.log
            .append(
                &tape,
                &json!({"kind":"user","id":format!("m{n}"),"ts":n,"text":"line"}),
            )
            .unwrap();
    }
    let door = Door::bind(desk.log.clone(), TOKEN.into(), desk.clone(), desk.parts()).unwrap();
    let port = door.port();
    let serving = tokio::spawn(door.run());
    let mut client = Client::connect(port, TOKEN).await;
    let sub = client.subscribe(json!({"tape":"ada"})).await;
    let seed = client
        .next_where(common::ANSWER, |frame| is_sub(frame, sub, "snapshot"))
        .await;
    assert_eq!(seed["snapshot"].as_array().unwrap().len(), 300);

    // The core remains the only writer. With no await on this runtime, its
    // subscriber necessarily falls behind the 1024-event broadcast ring.
    for n in 300..1500 {
        desk.log
            .append(
                &tape,
                &json!({"kind":"user","id":format!("m{n}"),"ts":n,"text":"line"}),
            )
            .unwrap();
    }
    let recovered = client
        .next_where(common::ANSWER, |frame| is_sub(frame, sub, "snapshot"))
        .await;
    let lines = recovered["snapshot"].as_array().unwrap();
    assert_eq!(lines.len(), 400);
    assert_eq!(lines[0]["id"], "m1100");
    assert_eq!(lines[399]["id"], "m1499");
    desk.log
        .append(
            &tape,
            &json!({"kind":"agent","id":"after","ts":1500,"text":"Still here"}),
        )
        .unwrap();
    let after = client
        .next_where(common::ANSWER, |frame| is_sub(frame, sub, "event"))
        .await;
    assert_eq!(after["event"]["id"], "after");
    assert_eq!(client.call("ping", json!({})).await["ok"], true);
    serving.abort();
}
