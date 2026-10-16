//! Commands a teammate leaves running, through the real core and a real Rig
//! route, with a disposable model endpoint that answers from a script.
//!
//! A command still running when the teammate replies is a process its session
//! holds: the turn ends at once, a later turn can stop it, its own end wakes
//! the teammate once, and stopping or revoking the session kills it.
#![cfg(unix)]
mod common;

use axum::{Router, body::Bytes, routing::post};
use futures_util::{SinkExt, StreamExt};
use hotline_core::wire::Door;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

const TOKEN: &str = "disposable-processes-test";

#[tokio::test(flavor = "multi_thread")]
async fn running_process_a_server_left_running_ends_the_turn_and_a_later_turn_stops_it() {
    let mut desk = Desk::open().await;
    let server = "echo $$ > server.pid; exec python3 -m http.server 0 --bind 127.0.0.1";
    desk.script([
        tool_call("shell", json!({"command": server}), "call_serve"),
        text("It is up."),
    ]);
    desk.prompt("serve the chart").await;
    let ended = desk.turn_end().await;
    assert_eq!(ended["event"]["stopReason"], "end_turn");
    let card = desk
        .client
        .next_where(Duration::from_secs(15), |frame| {
            frame["event"]["kind"] == "tool"
                && frame["event"]["toolKind"] == "shell"
                && frame["event"]["status"] == "completed"
        })
        .await;
    assert!(
        card.to_string().contains("Still running on its own"),
        "{card}"
    );

    let pid = desk.pid("server.pid").await;
    assert!(alive(pid), "the server outlives the turn that started it");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(alive(pid));
    desk.request().await;
    let job_id = receipt(&desk.request().await);

    // A later turn is told the server is up, and stops it.
    desk.script([
        tool_call(
            "cancel_job",
            json!({"job_id": job_id, "reason": "the person is done with it"}),
            "call_stop",
        ),
        text("Stopped."),
    ]);
    desk.prompt("stop the server").await;
    let asked = desk.request().await;
    assert!(
        asked.to_string().contains("http.server"),
        "the running process is in the request: {asked}"
    );
    desk.request().await;
    assert_eq!(desk.turn_end().await["event"]["stopReason"], "end_turn");
    until_dead(pid).await;
    desk.quiet_for(Duration::from_secs(1)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn running_process_that_ends_on_its_own_wakes_the_teammate_once() {
    let mut desk = Desk::open().await;
    desk.script([
        tool_call(
            "shell",
            json!({"command": "sleep 2; echo done"}),
            "call_build",
        ),
        text("The build is going."),
        text("The build finished: done."),
    ]);
    let asked = tokio::time::Instant::now();
    desk.prompt("build it").await;
    desk.request().await;
    desk.request().await;
    assert_eq!(desk.turn_end().await["event"]["stopReason"], "end_turn");
    assert!(
        asked.elapsed() < Duration::from_secs(2),
        "the turn ended before the command did"
    );

    let woke = desk.request().await;
    let said = woke.to_string();
    assert!(
        said.contains("a command you left running has ended"),
        "{said}"
    );
    assert!(said.contains("done"), "{said}");
    assert!(said.contains("succeeded"), "{said}");
    assert_eq!(desk.turn_end().await["event"]["stopReason"], "end_turn");
    desk.client
        .next_where(Duration::from_secs(15), |frame| {
            frame["event"]["kind"] == "agent"
                && frame["event"]["text"] == "The build finished: done."
        })
        .await;
    desk.quiet_for(Duration::from_secs(1)).await;
    let tape = desk
        .log
        .load(&hotline_core::log::StreamId::Tape(desk.persona.clone()));
    assert!(
        !tape
            .iter()
            .any(|event| event.to_string().contains("a command you left running")),
        "the wake is Hotline's line to the teammate, not the person's: {tape:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn running_process_dies_with_its_stopped_session() {
    let mut desk = Desk::open().await;
    let pid = desk.leave_running().await;
    let stopped = desk
        .call("session.stop", json!({"personaId": desk.persona}))
        .await;
    assert_eq!(stopped["ok"], true, "{stopped}");
    until_dead(pid).await;
    desk.quiet_for(Duration::from_secs(1)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn running_process_dies_when_its_grant_is_revoked() {
    let mut desk = Desk::open().await;
    let pid = desk.leave_running().await;
    // A changed grant rebuilds the session, and the old lease goes with it.
    let updated = desk
        .call(
            "persona.update",
            json!({"id": desk.persona, "patch": {"reach": "workspace"}}),
        )
        .await;
    assert_eq!(updated["ok"], true, "{updated}");
    until_dead(pid).await;
    desk.quiet_for(Duration::from_secs(1)).await;
}

/// The session as the roster says it, the next time `wanted` holds of it.
async fn session_where(desk: &mut Desk, wanted: impl Fn(&Value) -> bool) -> Value {
    let persona = desk.persona.clone();
    let row = desk
        .client
        .next_where(Duration::from_secs(15), |frame| {
            frame["event"]["persona"]["id"] == persona.as_str()
                && frame["event"]["session"].is_object()
                && wanted(&frame["event"]["session"])
        })
        .await;
    row["event"]["session"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn process_chip_a_process_is_on_the_session_until_the_person_stops_it() {
    let mut desk = Desk::open().await;
    desk.client.subscribe(json!({"view": "roster"})).await;
    let pid = desk.leave_running().await;
    let session = session_where(&mut desk, |session| {
        session["processes"]
            .as_array()
            .is_some_and(|listed| !listed.is_empty())
    })
    .await;
    let process = &session["processes"][0];
    assert_eq!(process["title"], "echo $$ > sleeper.pid; exec sleep 300");
    assert_eq!(process["state"], "running");
    assert!(process["startedAt"].as_i64().unwrap() > 0, "{process}");
    let id = process["id"].as_str().unwrap().to_owned();
    // The turn is over: a process running beside it is not work in flight.
    session_where(&mut desk, |session| {
        session["state"] != "thinking" && session["processes"][0]["id"] == id.as_str()
    })
    .await;

    // Rows from before the process was held say it has none too.
    desk.client.inbox.clear();
    let stopped = desk
        .call("process.stop", json!({"personaId": desk.persona, "id": id}))
        .await;
    assert_eq!(stopped["ok"], true, "{stopped}");
    until_dead(pid).await;
    let gone = session_where(&mut desk, |session| session.get("processes").is_none()).await;
    assert_ne!(gone["state"], "thinking");
    // The person stopped it, so its end wakes nobody.
    desk.quiet_for(Duration::from_secs(1)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn process_chip_stop_is_refused_for_a_teammate_that_does_not_hold_it() {
    let mut desk = Desk::open().await;
    let (pid, job_id) = desk
        .leave_running_as("echo $$ > sleeper.pid; exec sleep 300")
        .await;
    let made = desk
        .call(
            "persona.create",
            json!({"draft": {
                "name": "Other", "goal": "Hold nothing",
                "cwd": desk.workspace.to_string_lossy(), "reach": "machine"
            }}),
        )
        .await;
    let other = made["result"]["id"].as_str().unwrap().to_owned();
    for persona in [other.as_str(), "nobody"] {
        let refused = desk
            .call("process.stop", json!({"personaId": persona, "id": job_id}))
            .await;
        assert_eq!(refused["ok"], false, "{refused}");
        let unread = desk
            .call(
                "process.output",
                json!({"personaId": persona, "id": job_id}),
            )
            .await;
        assert_eq!(unread["ok"], false, "{unread}");
    }
    let started = desk
        .call("session.start", json!({"personaId": other}))
        .await;
    assert_eq!(started["ok"], true, "{started}");
    let refused = desk
        .call("process.stop", json!({"personaId": other, "id": job_id}))
        .await;
    assert_eq!(
        refused["ok"], false,
        "another session's id is not this one's: {refused}"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(alive(pid), "a refused stop stops nothing");

    let stopped = desk
        .call(
            "process.stop",
            json!({"personaId": desk.persona, "id": job_id}),
        )
        .await;
    assert_eq!(stopped["ok"], true, "{stopped}");
    until_dead(pid).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn process_chip_output_is_the_latest_of_what_it_wrote() {
    let mut desk = Desk::open().await;
    let command = "echo $$ > sleeper.pid; for i in $(seq 1 3000); do echo line $i; done; echo oops >&2; exec sleep 300";
    let (pid, job_id) = desk.leave_running_as(command).await;
    let output = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let read = desk
                .call(
                    "process.output",
                    json!({"personaId": desk.persona, "id": job_id}),
                )
                .await;
            assert_eq!(read["ok"], true, "{read}");
            if read["result"]["tail"].as_str().unwrap().contains("oops") {
                return read["result"].clone();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the output arrived while the command runs");
    assert_eq!(output["command"], command);
    let tail = output["tail"].as_str().unwrap();
    assert!(tail.contains("line 3000\n"), "{tail}");
    assert!(
        !tail.contains("line 1\n"),
        "the tail is bounded: {}",
        tail.len()
    );
    assert!(tail.len() <= 16 * 1024);
    assert!(alive(pid));
}

/// A desk with one teammate on a scripted model, its tape subscribed.
struct Desk {
    client: Client,
    log: hotline_core::log::Log,
    persona: String,
    workspace: PathBuf,
    script: Arc<Mutex<VecDeque<Vec<Value>>>>,
    requests: tokio::sync::mpsc::UnboundedReceiver<Value>,
    _root: tempfile::TempDir,
}

impl Desk {
    async fn open() -> Desk {
        let script: Arc<Mutex<VecDeque<Vec<Value>>>> = Arc::default();
        let (seen, requests) = tokio::sync::mpsc::unbounded_channel();
        let answers = script.clone();
        let app = Router::new().route(
            "/v1/chat/completions",
            post(move |body: Bytes| {
                let request: Value = serde_json::from_slice(&body).unwrap();
                let events = answers
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(|| panic!("an unscripted request: {request}"));
                seen.send(request).unwrap();
                let body: String = events
                    .into_iter()
                    .map(|event| format!("data: {event}\n\n"))
                    .collect();
                async move {
                    (
                        [("Content-Type", "text/event-stream")],
                        format!("{body}data: [DONE]\n\n"),
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let desk = common::open_desk(root.path()).unwrap();
        let log = desk.log.clone();
        let door = Door::bind(desk.log.clone(), TOKEN.into(), Arc::new(desk)).unwrap();
        let port = door.port();
        tokio::spawn(door.run());
        let mut client = Client::connect(port).await;
        let saved = client
            .call(
                "credential.custom_save",
                json!({"draft": {
                    "name": "processes", "baseUrl": base, "api": "chat_completions",
                    "models": ["vendor/coder"]
                }}),
            )
            .await;
        assert_eq!(saved["ok"], true, "{saved}");
        let made = client
            .call(
                "persona.create",
                json!({"draft": {
                    "name": "Goldie", "goal": "Serve what the person asks for",
                    "cwd": workspace.to_string_lossy(), "reach": "machine"
                }}),
            )
            .await;
        let persona = made["result"]["id"].as_str().unwrap().to_owned();
        client.subscribe(json!({"tape": persona})).await;
        let started = client
            .call("session.start", json!({"personaId": persona}))
            .await;
        assert_eq!(started["ok"], true, "{started}");
        Desk {
            client,
            log,
            persona,
            workspace,
            script,
            requests,
            _root: root,
        }
    }

    fn script<const N: usize>(&self, answers: [Vec<Value>; N]) {
        self.script.lock().unwrap().extend(answers);
    }

    async fn call(&mut self, cmd: &str, params: Value) -> Value {
        self.client.call(cmd, params).await
    }

    async fn prompt(&mut self, text: &str) {
        let sent = self
            .call(
                "session.prompt",
                json!({"personaId": self.persona, "text": text}),
            )
            .await;
        assert_eq!(sent["ok"], true, "{sent}");
    }

    async fn request(&mut self) -> Value {
        tokio::time::timeout(Duration::from_secs(15), self.requests.recv())
            .await
            .expect("the model was asked nothing")
            .unwrap()
    }

    async fn turn_end(&mut self) -> Value {
        self.client
            .next_where(Duration::from_secs(15), |frame| {
                frame["event"]["kind"] == "turn"
            })
            .await
    }

    /// Nothing more is asked of the model: no process woke it again.
    async fn quiet_for(&mut self, wait: Duration) {
        tokio::time::sleep(wait).await;
        if let Ok(request) = self.requests.try_recv() {
            panic!("the model was asked again: {request}");
        }
    }

    /// The pid a command wrote to `file` in the workspace, once it has.
    async fn pid(&self, file: &str) -> i32 {
        let path = self.workspace.join(file);
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if let Some(pid) = read_pid(&path) {
                    return pid;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the command wrote its pid")
    }

    /// A turn that leaves a long sleep running and replies; its pid.
    async fn leave_running(&mut self) -> i32 {
        self.leave_running_as("echo $$ > sleeper.pid; exec sleep 300")
            .await
            .0
    }

    /// A turn that leaves `command` running and replies; the pid it wrote
    /// to `sleeper.pid`, and its job id.
    async fn leave_running_as(&mut self, command: &str) -> (i32, String) {
        self.script([
            tool_call("shell", json!({"command": command}), "call_sleep"),
            text("Left it running."),
        ]);
        self.prompt("start the watcher").await;
        self.request().await;
        let job_id = receipt(&self.request().await);
        assert_eq!(self.turn_end().await["event"]["stopReason"], "end_turn");
        let pid = self.pid("sleeper.pid").await;
        assert!(alive(pid));
        (pid, job_id)
    }
}

fn read_pid(path: &Path) -> Option<i32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Whether the process we started is still there. Signal 0 only asks.
fn alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

async fn until_dead(pid: i32) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while alive(pid) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the process was killed");
}

/// The job id a launch receipt in this request names.
fn receipt(request: &Value) -> String {
    request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "tool")
        .find_map(|message| {
            let content: Value = serde_json::from_str(message["content"].as_str()?).ok()?;
            content["job"]["job_id"].as_str().map(str::to_owned)
        })
        .expect("the request carries the launch receipt")
}

fn text(words: &str) -> Vec<Value> {
    vec![
        json!({"id":"chat_test","object":"chat.completion.chunk","created":1,"model":"vendor/coder","choices":[{"index":0,"delta":{"role":"assistant","content":words},"finish_reason":null}]}),
        json!({"id":"chat_test","object":"chat.completion.chunk","created":1,"model":"vendor/coder","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
    ]
}

fn tool_call(name: &str, arguments: Value, call_id: &str) -> Vec<Value> {
    vec![
        json!({"id":"chat_test","object":"chat.completion.chunk","created":1,"model":"vendor/coder","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":call_id,"type":"function","function":{"name":name,"arguments":arguments.to_string()}}]},"finish_reason":null}]}),
        json!({"id":"chat_test","object":"chat.completion.chunk","created":1,"model":"vendor/coder","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
    ]
}

/// The window's half of the wire, as the harness plays it.
struct Client {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    next_id: i64,
    /// Frames that arrived while waiting for something else.
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

    async fn subscribe(&mut self, target: Value) {
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
                return;
            }
            self.inbox.push_back(frame);
        }
    }

    /// The next frame that satisfies `wanted`, from the inbox first.
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
