//! Threads on the wire, through the real core and a disposable model endpoint:
//! the same conversation, read by a client that declared `threads2` and by one
//! that did not.
mod common;

use axum::{Router, body::Bytes, routing::post};
use futures_util::{SinkExt, StreamExt};
use hotline_core::wire::Door;
use serde_json::{Value, json};
use std::{collections::VecDeque, sync::Arc, time::Duration};
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

const TOKEN: &str = "disposable-threads-test";
const PATIENCE: Duration = Duration::from_secs(20);

/// A model that answers every request with the same words, in one chunk.
fn answer_events() -> Vec<Value> {
    vec![
        json!({"id":"chat_test","object":"chat.completion.chunk","created":1,"model":"vendor/coder","choices":[{"index":0,"delta":{"role":"assistant","content":"On it."},"finish_reason":null}]}),
        json!({"id":"chat_test","object":"chat.completion.chunk","created":1,"model":"vendor/coder","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
    ]
}

/// A new thread opened over the wire is the same conversation to a client of
/// either shape: the older one is sent the markers and per-kind deltas it
/// always had, the one that said `threads2` is sent links and `ThreadDelta`s,
/// and every `thread.*` verb moves the one thread both are reading.
#[tokio::test(flavor = "multi_thread")]
async fn a_work_thread_is_one_conversation_to_old_and_threads2_clients() {
    let app = Router::new().route(
        "/v1/chat/completions",
        post(|_body: Bytes| async {
            let body = answer_events()
                .into_iter()
                .map(|event| format!("data: {event}\n\n"))
                .collect::<String>();
            (
                [("Content-Type", "text/event-stream")],
                format!("{body}data: [DONE]\n\n"),
            )
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

    let mut now = Client::connect(port).await;
    let mut old = Client::connect(port).await;
    let saved = now
        .call(
            "credential.custom_save",
            json!({"draft": {
                "name":"chat","baseUrl":base,"api":"chat_completions","models":["vendor/coder"]
            }}),
        )
        .await;
    assert_eq!(saved["ok"], true, "{saved}");
    let made = now
        .call(
            "persona.create",
            json!({"draft": {
                "name":"Threads tester","goal":"Answer briefly","cwd":workspace.to_string_lossy()
            }}),
        )
        .await;
    let persona = made["result"]["id"].as_str().unwrap().to_owned();

    // The hello names what this core can do, and the socket that declares it
    // is read differently from then on.
    let hello = now
        .call(
            "client.hello",
            json!({"capabilities":["threads2","somethingNew"]}),
        )
        .await;
    assert_eq!(hello["ok"], true, "{hello}");
    assert!(
        hello["result"]["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("threads2")),
        "{hello}"
    );

    let now_tape = now.subscribe(json!({"tape":persona})).await;
    let old_tape = old.subscribe(json!({"tape":persona})).await;

    // Open a work thread, and read it by id on one socket and by side id on
    // the other.
    let opened = now
        .call(
            "thread.open",
            json!({"personaId":persona,"text":"Look into the crane"}),
        )
        .await;
    assert_eq!(opened["ok"], true, "{opened}");
    let summary = &opened["result"];
    assert_eq!(summary["thread"]["kind"], "side");
    assert_eq!(summary["personaId"], persona.as_str());
    assert_eq!(summary["title"], "Look into the crane");
    let key = summary["thread"]["key"].as_str().unwrap().to_owned();
    let thread = json!({"kind":"side","key":key});
    let now_side = now.subscribe(json!({"threadId":thread})).await;
    let old_side = old.subscribe(json!({"side":key})).await;

    // The tape's link, in the shape each client reads.
    let link = now
        .next_where(PATIENCE, |frame| {
            frame["sub"] == now_tape && frame["event"]["kind"] == "link"
        })
        .await;
    assert_eq!(link["event"]["thread"], key.as_str());
    assert_eq!(link["event"]["threadKind"], "side");
    let marker = old
        .next_where(PATIENCE, |frame| {
            frame["sub"] == old_tape && frame["event"]["kind"] == "side"
        })
        .await;
    assert_eq!(marker["event"]["sideId"], key.as_str());
    assert!(
        old.inbox
            .iter()
            .all(|frame| frame["event"]["kind"] != "link"),
        "an old client is never sent a link"
    );

    // The first turn ends, then another is asked for, with both reading.
    now.next_where(PATIENCE, |frame| {
        frame["sub"] == now_side && frame["event"]["kind"] == "turn"
    })
    .await;
    let said = now
        .call(
            "thread.prompt",
            json!({"thread":thread,"text":"And the hoist?"}),
        )
        .await;
    assert_eq!(said["ok"], true, "{said}");
    let delta = now
        .next_where(PATIENCE, |frame| {
            frame["sub"] == now_side && frame["ephemeral"]["type"] == "thread_delta"
        })
        .await;
    assert_eq!(delta["ephemeral"]["thread"], thread);
    assert_eq!(delta["ephemeral"]["kind"], "text");
    assert_eq!(delta["ephemeral"]["text"], "On it.");
    let delta = old
        .next_where(PATIENCE, |frame| {
            frame["sub"] == old_side && frame["ephemeral"]["type"] == "side_agent_delta"
        })
        .await;
    assert_eq!(delta["ephemeral"]["sideId"], key.as_str());
    assert_eq!(delta["ephemeral"]["text"], "On it.");
    assert!(
        old.inbox
            .iter()
            .all(|frame| frame["ephemeral"]["type"] != "thread_delta"),
        "an old client is never sent a ThreadDelta"
    );
    now.next_where(PATIENCE, |frame| {
        frame["sub"] == now_side
            && frame["event"]["kind"] == "agent"
            && frame["event"]["text"] == "On it."
    })
    .await;

    // The same list, by either name; the work thread is live and was asked
    // about by its first line.
    let listed = now.call("thread.list", json!({"personaId":persona})).await["result"].clone();
    let row = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["thread"] == thread)
        .expect("the work thread is listed")
        .clone();
    assert_eq!(row["state"], "live");
    assert_eq!(row["preview"], "On it.");
    assert!(
        listed
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["thread"] == json!({"kind":"dm","key":persona})),
        "the DM is listed with it: {listed}"
    );
    let side_list = old.call("side.list", json!({"personaId":persona})).await["result"].clone();
    assert_eq!(side_list[0]["sideId"], key.as_str());
    assert_eq!(side_list[0]["status"], "live");

    // Park, speak into it (which wakes it), close, continue.
    let parked = now.call("thread.park", json!({"thread":thread})).await;
    assert_eq!(parked["ok"], true, "{parked}");
    assert_eq!(state_of(&mut now, &persona, &thread).await, "parked");
    assert_eq!(
        old.call("side.list", json!({"personaId":persona})).await["result"][0]["status"],
        "parked"
    );
    let woken = old
        .call("side.prompt", json!({"sideId":key,"text":"Still there?"}))
        .await;
    assert_eq!(woken["ok"], true, "{woken}");
    assert_eq!(state_of(&mut now, &persona, &thread).await, "live");
    now.next_where(PATIENCE, |frame| {
        frame["sub"] == now_side
            && frame["event"]["kind"] == "user"
            && frame["event"]["text"] == "Still there?"
    })
    .await;
    let closed = now.call("thread.close", json!({"thread":thread})).await;
    assert_eq!(closed["ok"], true, "{closed}");
    assert_eq!(state_of(&mut now, &persona, &thread).await, "closed");
    let refused = now
        .call("thread.prompt", json!({"thread":thread,"text":"hello?"}))
        .await;
    assert_eq!(
        refused["ok"], false,
        "a closed thread is spoken in only once continued"
    );
    let back = now.call("thread.continue", json!({"thread":thread})).await;
    assert_eq!(back["ok"], true, "{back}");
    assert_eq!(back["result"]["state"], "live");
    let cancelled = now.call("thread.cancel", json!({"thread":thread})).await;
    assert_eq!(cancelled["ok"], true, "{cancelled}");

    // The main conversation is a thread too: the same verbs, the same words.
    assert_eq!(
        now.call("session.start", json!({"personaId":persona}))
            .await["ok"],
        true
    );
    let said = now
        .call(
            "thread.prompt",
            json!({"thread":{"kind":"dm","key":persona},"text":"Morning"}),
        )
        .await;
    assert_eq!(said["ok"], true, "{said}");
    let delta = now
        .next_where(PATIENCE, |frame| {
            frame["sub"] == now_tape && frame["ephemeral"]["type"] == "thread_delta"
        })
        .await;
    assert_eq!(
        delta["ephemeral"]["thread"],
        json!({"kind":"dm","key":persona})
    );
    let delta = old
        .next_where(PATIENCE, |frame| {
            frame["sub"] == old_tape && frame["ephemeral"]["type"] == "agent_delta"
        })
        .await;
    assert_eq!(delta["ephemeral"]["personaId"], persona.as_str());

    now.call("session.stop", json!({"personaId":persona})).await;
    drop(now);
    drop(old);
    door_task.abort();
    server.abort();
}

/// A thread's state in the list, which is where a client reads it.
async fn state_of(client: &mut Client, persona: &str, thread: &Value) -> String {
    let listed = client
        .call("thread.list", json!({"personaId":persona}))
        .await["result"]
        .clone();
    listed
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["thread"] == *thread)
        .expect("the thread is listed")["state"]
        .as_str()
        .unwrap()
        .to_owned()
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
                .unwrap_or_else(|_| {
                    // What did arrive, newest last, so a timeout on a slow
                    // runner says which step stalled.
                    let seen: Vec<String> = self
                        .inbox
                        .iter()
                        .rev()
                        .take(12)
                        .rev()
                        .map(|frame| {
                            let text = frame.to_string();
                            text.chars().take(240).collect()
                        })
                        .collect();
                    panic!(
                        "nothing wanted arrived within {patience:?}; the inbox held:\n{}",
                        seen.join("\n")
                    )
                });
            if wanted(&frame) {
                return frame;
            }
            self.inbox.push_back(frame);
        }
    }
}
