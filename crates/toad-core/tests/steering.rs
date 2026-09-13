//! Active input through the real core and real Rig adapters, with disposable
//! HTTP model endpoints. No native steering events or live credentials.
mod common;

use axum::{Router, body::Bytes, routing::post};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{collections::VecDeque, sync::Arc, time::Duration};
use toad_core::wire::Door;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

const TOKEN: &str = "disposable-steering-test";

/// An already executed read survives interruption of the following request.
/// The update must reach another request while the old HTTP call is still
/// parked, with one valid call/result pair and both operator messages.
#[tokio::test(flavor = "multi_thread")]
async fn steering_restarts_inference_and_preserves_tool_history_on_both_rig_routes() {
    for api in ["responses", "chat_completions"] {
        let (seen, mut requests) = tokio::sync::mpsc::channel(8);
        let path = if api == "responses" {
            "/v1/responses"
        } else {
            "/v1/chat/completions"
        };
        let app = Router::new().route(
            path,
            post(move |body: Bytes| {
                let seen = seen.clone();
                async move {
                    let request: Value = serde_json::from_slice(&body).unwrap();
                    let serialized = request.to_string();
                    let redirected = serialized.contains("Actually focus on login");
                    let read = serialized.contains("a confirmed fact");
                    seen.send(request).await.unwrap();
                    if read && !redirected {
                        std::future::pending::<()>().await;
                    }
                    let events = events(api, redirected);
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
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let root = std::env::temp_dir().join(format!("toad-steering-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("workspace")).unwrap();
        std::fs::write(root.join("workspace/note.txt"), "a confirmed fact").unwrap();
        let desk = common::open_desk(&root).unwrap();
        let door = Door::bind(desk.log.clone(), TOKEN.into(), Arc::new(desk)).unwrap();
        let port = door.port();
        let door_task = tokio::spawn(door.run());
        let mut client = Client::connect(port).await;
        let saved = client
            .call(
                "credential.custom_save",
                json!({"draft": {
                    "name":api, "baseUrl":base, "api":api, "models":["vendor/coder"]
                }}),
            )
            .await;
        assert_eq!(saved["ok"], true, "{saved}");
        let made = client.call("persona.create", json!({"draft": {
            "name":"Steering tester", "goal":"Follow the operator", "cwd":root.join("workspace").to_string_lossy()
        }})).await;
        let persona = made["result"]["id"].as_str().unwrap().to_owned();
        let tape = client.subscribe(json!({"tape":persona})).await;
        let started = client
            .call("session.start", json!({"personaId":persona}))
            .await;
        assert_eq!(started["ok"], true, "{started}");
        assert_eq!(
            started["result"]["capabilities"]["activeInput"], true,
            "{started}"
        );
        let sent = client
            .call(
                "session.prompt",
                json!({"personaId":persona,"text":"Read note.txt then work on caching"}),
            )
            .await;
        assert_eq!(sent["ok"], true, "{sent}");
        let first = next_request(&mut requests).await;
        assert_eq!(first["model"], "vendor/coder");
        let waiting = next_request(&mut requests).await;
        assert!(waiting.to_string().contains("a confirmed fact"));

        let sent = client
            .call(
                "session.prompt",
                json!({"personaId":persona,"text":"Actually focus on login"}),
            )
            .await;
        assert_eq!(sent["ok"], true, "{sent}");
        let resumed = next_request(&mut requests).await;
        let serialized = resumed.to_string();
        assert!(serialized.contains("Read note.txt then work on caching"));
        assert!(serialized.contains("Actually focus on login"));
        assert!(serialized.contains("a confirmed fact"));
        let items = resumed[if api == "responses" {
            "input"
        } else {
            "messages"
        }]
        .as_array()
        .unwrap();
        let results: Vec<_> = items
            .iter()
            .filter(|item| item["type"] == "function_call_output" || item["role"] == "tool")
            .collect();
        assert_eq!(
            results.len(),
            1,
            "one completed read is retained: {resumed}"
        );
        assert_eq!(
            results[0][if api == "responses" {
                "call_id"
            } else {
                "tool_call_id"
            }],
            "call_read"
        );
        // The steer is read once the agent produces anything with it in
        // context: the tape says so with a receipt, not a notice.
        let updated = client
            .next_where(Duration::from_secs(15), |frame| {
                frame["sub"] == tape
                    && frame["event"]["kind"] == "user"
                    && frame["event"]["text"] == "Actually focus on login"
                    && frame["event"]["receipt"] == "read"
            })
            .await;
        assert_eq!(updated["event"]["receipt"], "read");
        let completed = client
            .next_where(Duration::from_secs(15), |frame| {
                frame["sub"] == tape && frame["event"]["kind"] == "turn"
            })
            .await;
        assert_eq!(completed["event"]["stopReason"], "end_turn");
        client
            .call("session.stop", json!({"personaId":persona}))
            .await;
        drop(client);
        door_task.abort();
        server.abort();
        let _ = std::fs::remove_dir_all(root);
    }
}

/// Reproduces the operator's screenshot with a real process: shell returns a
/// handle, wait_jobs is interrupted, and cancel_job reaches the process tree.
#[tokio::test(flavor = "multi_thread")]
async fn an_operator_cancels_a_running_shell_before_its_ninety_second_wait_finishes() {
    for api in ["responses", "chat_completions"] {
        let (seen, mut requests) = tokio::sync::mpsc::channel(8);
        let step = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let command = if cfg!(windows) {
            "echo started & echo x > heartbeat & ping -n 91 127.0.0.1 > nul & echo done > done"
        } else {
            "echo started; while true; do echo x >> heartbeat; sleep 0.05; done & sleep 90; echo done > done"
        };
        let app = Router::new().route(
            if api == "responses" { "/v1/responses" } else { "/v1/chat/completions" },
            post(move |body: Bytes| {
                let seen = seen.clone();
                let step = step.clone();
                async move {
                    let request: Value = serde_json::from_slice(&body).unwrap();
                    let index = step.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let receipt = json_values(&request).into_iter().find_map(|value| {
                        value.get("job").and_then(|job| job["job_id"].as_str()).map(str::to_owned)
                    });
                    let events = match index {
                        0 => tool_events(api, "shell", json!({"command":command}), "call_shell"),
                        1 => tool_events(api, "wait_jobs", json!({"job_ids":[receipt.unwrap()]}), "call_wait"),
                        2 => {
                            assert!(request.to_string().contains("nice, cancel that"));
                            assert!(json_values(&request).iter().any(|value| value["status"] == "interrupted_by_message"));
                            tool_events(api, "cancel_job", json!({"job_id":receipt.unwrap(),"reason":"The operator asked to cancel it"}), "call_cancel")
                        }
                        3 => tool_events(api, "wait_jobs", json!({"job_ids":[receipt.unwrap()]}), "call_confirm"),
                        4 => {
                            assert!(json_values(&request).iter().any(|value| value["jobs"].as_array().is_some_and(|jobs| jobs.iter().any(|job| job["state"] == "cancelled"))));
                            events(api, true)
                        }
                        _ => panic!("unexpected continuation after cancellation: {request}"),
                    };
                    seen.send(request).await.unwrap();
                    let body = events.into_iter().map(|event| format!("data: {event}\n\n")).collect::<String>();
                    ([("Content-Type", "text/event-stream")], format!("{body}data: [DONE]\n\n"))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let desk = common::open_desk(root.path()).unwrap();
        let door = Door::bind(desk.log.clone(), TOKEN.into(), Arc::new(desk)).unwrap();
        let port = door.port();
        let door_task = tokio::spawn(door.run());
        let mut client = Client::connect(port).await;
        let saved = client
            .call(
                "credential.custom_save",
                json!({"draft": {
                    "name":api,"baseUrl":base,"api":api,"models":["vendor/coder"]
                }}),
            )
            .await;
        assert_eq!(saved["ok"], true, "{saved}");
        let made = client.call("persona.create", json!({"draft": {
            "name":"Shell steering tester", "goal":"Follow the operator", "cwd":workspace.to_string_lossy(),
            "reach":toad_core::contract::Reach::Machine
        }})).await;
        let persona = made["result"]["id"].as_str().unwrap().to_owned();
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
                    json!({"personaId":persona,"text":"Run a command that takes 90 seconds"})
                )
                .await["ok"],
            true
        );
        next_request(&mut requests).await;
        let launched = next_request(&mut requests).await;
        assert!(
            json_values(&launched)
                .iter()
                .any(|value| value["status"] == "accepted")
        );
        client
            .next_where(Duration::from_secs(15), |frame| {
                frame["sub"] == tape
                    && frame["event"]["kind"] == "tool"
                    && frame["event"]["toolKind"] == "wait_jobs"
                    && frame["event"]["status"] == "in_progress"
            })
            .await;
        tokio::time::timeout(Duration::from_secs(15), async {
            while !std::fs::metadata(workspace.join("heartbeat")).is_ok_and(|file| file.len() > 0) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the command must actually start before cancellation");
        let began = tokio::time::Instant::now();
        assert_eq!(
            client
                .call(
                    "session.prompt",
                    json!({"personaId":persona,"text":"nice, cancel that"})
                )
                .await["ok"],
            true
        );
        next_request(&mut requests).await;
        next_request(&mut requests).await;
        let confirmed = next_request(&mut requests).await;
        assert!(began.elapsed() < Duration::from_secs(15));
        let items = confirmed[if api == "responses" {
            "input"
        } else {
            "messages"
        }]
        .as_array()
        .unwrap();
        assert_eq!(
            items
                .iter()
                .filter(|item| item["call_id"] == "call_shell"
                    && item["type"] == "function_call_output"
                    || item["tool_call_id"] == "call_shell" && item["role"] == "tool")
                .count(),
            1,
            "the launch gets exactly one ordinary tool reply"
        );
        let completion = client
            .next_where(Duration::from_secs(15), |frame| {
                frame["sub"] == tape && frame["event"]["kind"] == "turn"
            })
            .await;
        assert_eq!(completion["event"]["stopReason"], "end_turn");
        let shell = client
            .next_where(Duration::from_secs(15), |frame| {
                frame["sub"] == tape
                    && frame["event"]["kind"] == "tool"
                    && frame["event"]["toolKind"] == "shell"
                    && frame["event"]["status"] == "failed"
            })
            .await;
        assert!(shell.to_string().contains("Cancelled"), "{shell}");
        assert!(
            shell.to_string().contains("started"),
            "partial output must survive cancellation: {shell}"
        );
        let before = std::fs::read(workspace.join("heartbeat")).unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            std::fs::read(workspace.join("heartbeat")).unwrap(),
            before,
            "a descendant kept running after cancellation was reported"
        );
        assert!(!workspace.join("done").exists());
        assert_eq!(
            client
                .call("session.stop", json!({"personaId":persona}))
                .await["ok"],
            true
        );
        drop(client);
        door_task.abort();
        server.abort();
    }
}

fn json_values(value: &Value) -> Vec<Value> {
    match value {
        Value::String(text) => serde_json::from_str::<Value>(text)
            .ok()
            .into_iter()
            .collect(),
        Value::Array(items) => items.iter().flat_map(json_values).collect(),
        Value::Object(fields) => fields.values().flat_map(json_values).collect(),
        _ => Vec::new(),
    }
}

fn tool_events(api: &str, name: &str, arguments: Value, call_id: &str) -> Vec<Value> {
    if api == "responses" {
        let output = json!({"type":"function_call","id":format!("fc_{call_id}"),"call_id":call_id,"name":name,"arguments":arguments.to_string(),"status":"completed"});
        vec![
            json!({"type":"response.output_item.added","output_index":0,"sequence_number":1,"item":{"type":"function_call","id":format!("fc_{call_id}"),"call_id":call_id,"name":name,"arguments":"","status":"in_progress"}}),
            json!({"type":"response.function_call_arguments.delta","item_id":format!("fc_{call_id}"),"output_index":0,"sequence_number":2,"delta":arguments.to_string()}),
            json!({"type":"response.output_item.done","output_index":0,"sequence_number":3,"item":output}),
            json!({"type":"response.completed","sequence_number":4,"response":{"id":"resp_test","object":"response","created_at":1,"status":"completed","model":"vendor/coder","output":[output],"usage":{"input_tokens":3,"output_tokens":4,"total_tokens":7}}}),
        ]
    } else {
        vec![
            json!({"id":"chat_test","object":"chat.completion.chunk","created":1,"model":"vendor/coder","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":call_id,"type":"function","function":{"name":name,"arguments":arguments.to_string()}}]},"finish_reason":null}]}),
            json!({"id":"chat_test","object":"chat.completion.chunk","created":1,"model":"vendor/coder","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
        ]
    }
}

async fn next_request(requests: &mut tokio::sync::mpsc::Receiver<Value>) -> Value {
    tokio::time::timeout(Duration::from_secs(15), requests.recv())
        .await
        .expect("steering must not wait for the old request to finish")
        .unwrap()
}

fn events(api: &str, answer: bool) -> Vec<Value> {
    if api == "responses" {
        let output = if answer {
            json!({"type":"message","id":"msg_answer","status":"completed","role":"assistant","content":[{"type":"output_text","text":"Now working on login","annotations":[]}]})
        } else {
            json!({"type":"function_call","id":"fc_read","call_id":"call_read","name":"read","arguments":"{\"path\":\"note.txt\"}","status":"completed"})
        };
        let mut events = if answer {
            vec![
                json!({"type":"response.output_text.delta","item_id":"msg_answer","output_index":0,"content_index":0,"sequence_number":1,"delta":"Now working on login"}),
            ]
        } else {
            vec![
                json!({"type":"response.output_item.added","output_index":0,"sequence_number":1,"item":{"type":"function_call","id":"fc_read","call_id":"call_read","name":"read","arguments":"","status":"in_progress"}}),
                json!({"type":"response.function_call_arguments.delta","item_id":"fc_read","output_index":0,"sequence_number":2,"delta":"{\"path\":\"note.txt\"}"}),
                json!({"type":"response.output_item.done","output_index":0,"sequence_number":3,"item":output}),
            ]
        };
        events.push(json!({"type":"response.completed","sequence_number":4,"response":{"id":"resp_test","object":"response","created_at":1,"status":"completed","model":"vendor/coder","output":[output],"usage":{"input_tokens":3,"output_tokens":4,"total_tokens":7}}}));
        events
    } else {
        let delta = if answer {
            json!({"role":"assistant","content":"Now working on login"})
        } else {
            json!({"role":"assistant","tool_calls":[{"index":0,"id":"call_read","type":"function","function":{"name":"read","arguments":"{\"path\":\"note.txt\"}"}}]})
        };
        vec![
            json!({"id":"chat_test","object":"chat.completion.chunk","created":1,"model":"vendor/coder","choices":[{"index":0,"delta":delta,"finish_reason":null}]}),
            json!({"id":"chat_test","object":"chat.completion.chunk","created":1,"model":"vendor/coder","choices":[{"index":0,"delta":{},"finish_reason":if answer {"stop"} else {"tool_calls"}}]}),
        ]
    }
}

/// The window's half of the wire, as the harness plays it.
struct Client {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    next_id: i64,
    /// Frames that arrived while waiting for something else: a subscription's
    /// snapshot or event lands whenever the room has one.
    inbox: VecDeque<Value>,
}

impl Client {
    async fn connect(port: u16) -> Client {
        let (socket, _) = connect_async(format!("ws://127.0.0.1:{port}/ws?token={TOKEN}"))
            .await
            .unwrap();
        Client {
            socket,
            next_id: 1,
            inbox: VecDeque::new(),
        }
    }

    async fn read(&mut self) -> Value {
        loop {
            match self.socket.next().await.expect("the door closed") {
                Ok(Message::Text(text)) => return serde_json::from_str(&text).unwrap(),
                Ok(_) => continue,
                Err(error) => panic!("the socket failed: {error}"),
            }
        }
    }

    /// A command, answered. Frames for subscriptions that arrive first are
    /// kept for `next_where`.
    async fn call(&mut self, cmd: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let frame = json!({ "id": id, "cmd": cmd, "params": params });
        self.socket
            .send(Message::text(frame.to_string()))
            .await
            .unwrap();
        loop {
            let frame = self.read().await;
            if frame.get("id").and_then(Value::as_i64) == Some(id) {
                return frame;
            }
            self.inbox.push_back(frame);
        }
    }

    async fn subscribe(&mut self, target: Value) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        self.socket
            .send(Message::text(
                json!({ "id": id, "sub": target }).to_string(),
            ))
            .await
            .unwrap();
        loop {
            let frame = self.read().await;
            if frame.get("id").and_then(Value::as_i64) == Some(id) {
                assert_eq!(frame["ok"], true, "{frame}");
                return id;
            }
            self.inbox.push_back(frame);
        }
    }

    /// The next frame that satisfies `wanted`, from the inbox first and then
    /// the socket, within `patience`.
    async fn next_where(&mut self, patience: Duration, wanted: impl Fn(&Value) -> bool) -> Value {
        if let Some(at) = self.inbox.iter().position(&wanted) {
            return self.inbox.remove(at).unwrap();
        }
        let deadline = tokio::time::Instant::now() + patience;
        loop {
            let frame = tokio::time::timeout_at(deadline, self.read())
                .await
                .unwrap_or_else(|_| panic!("nothing wanted arrived within {patience:?}"));
            if wanted(&frame) {
                return frame;
            }
            self.inbox.push_back(frame);
        }
    }
}
