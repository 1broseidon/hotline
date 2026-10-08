//! A teammate's extra folders through the real core: granted and checked over
//! the wire, held to by Hotline Agent's real tools in a real Rig turn against a
//! disposable model endpoint, and taken back by a policy change. No live
//! credentials; the model is a script of tool calls.
mod common;

use axum::{Router, body::Bytes, routing::post};
use futures_util::{SinkExt, StreamExt};
use hotline_core::contract::Reach;
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

const TOKEN: &str = "disposable-folders-test";

/// The tool calls the model makes, one per request, then a closing answer.
type Script = Arc<Mutex<VecDeque<(String, String, Value)>>>;

struct Place {
    _root: tempfile::TempDir,
    data: PathBuf,
    workspace: PathBuf,
    read_only: PathBuf,
    writable: PathBuf,
    outside: PathBuf,
}

impl Place {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().canonicalize().unwrap();
        let [data, workspace, read_only, writable, outside] =
            ["data", "workspace", "read-only", "writable", "outside"].map(|name| base.join(name));
        for directory in [&data, &workspace, &read_only, &writable, &outside] {
            std::fs::create_dir(directory).unwrap();
        }
        std::fs::write(read_only.join("notes.txt"), "granted-canary").unwrap();
        std::fs::write(outside.join("secret.txt"), "outside-canary").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.join("secret.txt"), read_only.join("escape.txt"))
            .unwrap();
        Self {
            _root: root,
            data,
            workspace,
            read_only,
            writable,
            outside,
        }
    }
}

fn path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Granted at create, read where granted, changed only where it may be, and
/// nothing outside or through a link; then narrowed and removed over the wire,
/// after which the next tool call is refused.
#[tokio::test(flavor = "multi_thread")]
async fn granted_folders_hold_the_real_tools_and_a_change_takes_them_back() {
    let place = Place::new();
    let script: Script = Arc::default();
    let (seen, mut requests) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let served = script.clone();
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |body: Bytes| {
            let seen = seen.clone();
            let served = served.clone();
            async move {
                let request: Value = serde_json::from_slice(&body).unwrap();
                seen.send(request.clone()).unwrap();
                let mut next = served.lock().unwrap().pop_front();
                // A shell call answers with a job receipt; the wait names it.
                if let Some((_, name, arguments)) = next.as_mut()
                    && name == "wait_jobs"
                {
                    let receipt = request["messages"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .rev()
                        .find(|message| message["role"] == "tool")
                        .map(|message| message["content"].to_string())
                        .unwrap_or_default();
                    let job = receipt
                        .split("job_id")
                        .nth(1)
                        .map(|rest| {
                            rest.chars()
                                .skip_while(|c| !c.is_ascii_hexdigit())
                                .take_while(|c| c.is_ascii_hexdigit() || *c == '-')
                                .collect::<String>()
                        })
                        .unwrap_or_default();
                    *arguments = json!({"job_ids": [job]});
                }
                let delta = match next {
                    Some((id, name, arguments)) => json!({"role":"assistant","tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":name,"arguments":arguments.to_string()}}]}),
                    None => json!({"role":"assistant","content":"Done."}),
                };
                let finish = if delta["tool_calls"].is_array() { "tool_calls" } else { "stop" };
                let events = [
                    json!({"id":"chat_test","object":"chat.completion.chunk","created":1,"model":"vendor/coder","choices":[{"index":0,"delta":delta,"finish_reason":null}]}),
                    json!({"id":"chat_test","object":"chat.completion.chunk","created":1,"model":"vendor/coder","choices":[{"index":0,"delta":{},"finish_reason":finish}]}),
                ];
                let body = events.iter().map(|event| format!("data: {event}\n\n")).collect::<String>();
                ([("Content-Type", "text/event-stream")], format!("{body}data: [DONE]\n\n"))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let desk = common::open_desk(&place.data).unwrap();
    let door = Door::bind(desk.log.clone(), TOKEN.into(), Arc::new(desk)).unwrap();
    let port = door.port();
    let door_task = tokio::spawn(door.run());
    let mut client = Client::connect(port).await;
    let saved = client
        .call(
            "credential.custom_save",
            json!({"draft": {
                "name":"folders","baseUrl":base,"api":"chat_completions","models":["vendor/coder"]
            }}),
        )
        .await;
    assert_eq!(saved["ok"], true, "{saved}");

    // A teammate made without the field has no folders, and its record
    // says nothing about them.
    let plain = client
        .call(
            "persona.create",
            json!({"draft": {"name":"Plain","cwd":path(&place.workspace)}}),
        )
        .await;
    assert_eq!(plain["ok"], true, "{plain}");
    assert!(plain["result"].get("folders").is_none(), "{plain}");

    let made = client
        .call(
            "persona.create",
            json!({"draft": {
                "name":"Folders tester", "goal":"Follow the script", "cwd":path(&place.workspace),
                "folders":[
                    {"path": path(&place.read_only)},
                    {"path": path(&place.writable), "writable": true},
                ],
            }}),
        )
        .await;
    assert_eq!(made["ok"], true, "{made}");
    assert_eq!(
        made["result"]["folders"],
        json!([
            {"path": path(&place.read_only), "writable": false},
            {"path": path(&place.writable), "writable": true},
        ])
    );
    let persona = made["result"]["id"].as_str().unwrap().to_owned();

    let shell = hotline_core::tools::shell_available(Reach::Workspace).is_ok();
    {
        let mut calls = script.lock().unwrap();
        for (id, name, arguments) in [
            (
                "read_granted",
                "read",
                json!({"path": path(&place.read_only.join("notes.txt"))}),
            ),
            (
                "write_read_only",
                "write",
                json!({"path": path(&place.read_only.join("new.txt")), "content":"no"}),
            ),
            (
                "write_writable",
                "write",
                json!({"path": path(&place.writable.join("new.txt")), "content":"yes"}),
            ),
            (
                "read_outside",
                "read",
                json!({"path": path(&place.outside.join("secret.txt"))}),
            ),
            (
                "read_escape",
                "read",
                json!({"path": path(&place.read_only.join("escape.txt"))}),
            ),
        ] {
            calls.push_back((id.into(), name.into(), arguments));
        }
        if shell {
            calls.push_back((
                "shell".into(),
                "shell".into(),
                json!({"command": format!(
                    "cat '{}' > '{}'; echo y > '{}'; cat '{}' > '{}'",
                    place.read_only.join("notes.txt").display(),
                    place.writable.join("copied.txt").display(),
                    place.read_only.join("shell.txt").display(),
                    place.outside.join("secret.txt").display(),
                    place.writable.join("leaked.txt").display(),
                )}),
            ));
            calls.push_back(("wait".into(), "wait_jobs".into(), Value::Null));
        }
    }
    let results = turn(
        &mut client,
        &mut requests,
        &persona,
        "Work through the folders",
    )
    .await;
    let result = |id: &str| {
        results
            .iter()
            .find(|(call, _)| call == id)
            .map(|(_, text)| text.clone())
            .unwrap_or_else(|| panic!("no result for {id}: {results:?}"))
    };
    assert!(result("read_granted").contains("granted-canary"));
    assert!(
        result("write_read_only").contains("read but not change"),
        "{results:?}"
    );
    assert!(!place.read_only.join("new.txt").exists());
    assert_eq!(
        std::fs::read_to_string(place.writable.join("new.txt")).unwrap(),
        "yes"
    );
    for refused in ["read_outside", "read_escape"] {
        assert!(
            !result(refused).contains("outside-canary"),
            "{refused}: {results:?}"
        );
    }
    if shell {
        // The job ran to its end (it fails on the refused lines) before
        // the files are read.
        assert!(
            result("wait").contains("\\\"state\\\":\\\"failed\\\""),
            "{results:?}"
        );
        assert_eq!(
            std::fs::read_to_string(place.writable.join("copied.txt")).unwrap(),
            "granted-canary"
        );
        assert!(!place.read_only.join("shell.txt").exists());
        assert!(
            !std::fs::read_to_string(place.writable.join("leaked.txt"))
                .unwrap_or_default()
                .contains("outside-canary")
        );
    }

    // Narrowed over the wire: the writable folder becomes read-only and the
    // other is taken away. The next tool call is held to the new grant.
    let narrowed = client
        .call(
            "persona.update",
            json!({"id": persona, "patch": {"folders": [{"path": path(&place.writable)}]}}),
        )
        .await;
    assert_eq!(narrowed["ok"], true, "{narrowed}");
    script.lock().unwrap().extend([
        (
            "write_after".to_string(),
            "write".to_string(),
            json!({"path": path(&place.writable.join("after.txt")), "content":"late"}),
        ),
        (
            "read_removed".to_string(),
            "read".to_string(),
            json!({"path": path(&place.read_only.join("notes.txt"))}),
        ),
    ]);
    let results = turn(&mut client, &mut requests, &persona, "Again").await;
    let result = |id: &str| {
        results
            .iter()
            .find(|(call, _)| call == id)
            .map(|(_, text)| text.clone())
            .unwrap()
    };
    assert!(
        result("write_after").contains("read but not change"),
        "{results:?}"
    );
    assert!(!place.writable.join("after.txt").exists());
    assert!(
        !result("read_removed").contains("granted-canary"),
        "{results:?}"
    );

    let cleared = client
        .call(
            "persona.update",
            json!({"id": persona, "patch": {"folders": []}}),
        )
        .await;
    assert_eq!(cleared["ok"], true, "{cleared}");
    assert!(cleared["result"].get("folders").is_none(), "{cleared}");

    drop(client);
    door_task.abort();
    server.abort();
}

/// Every folder that cannot be granted is refused over the wire with a
/// sentence, and a refusal leaves the teammate as it was.
#[tokio::test(flavor = "multi_thread")]
async fn a_folder_that_cannot_be_granted_is_refused_and_changes_nothing() {
    let place = Place::new();
    let desk = common::open_desk(&place.data).unwrap();
    let door = Door::bind(desk.log.clone(), TOKEN.into(), Arc::new(desk)).unwrap();
    let port = door.port();
    let door_task = tokio::spawn(door.run());
    let mut client = Client::connect(port).await;
    let made = client
        .call(
            "persona.create",
            json!({"draft": {"name":"Checked","cwd":path(&place.workspace),
                "folders":[{"path": path(&place.read_only)}]}}),
        )
        .await;
    assert_eq!(made["ok"], true, "{made}");
    let persona = made["result"]["id"].as_str().unwrap().to_owned();
    std::fs::create_dir(place.workspace.join("inner")).unwrap();
    std::fs::create_dir(place.read_only.join("inner")).unwrap();
    let file = place.outside.join("secret.txt");
    let other_workspace = place.data.join("workspaces").join("someone");
    std::fs::create_dir_all(&other_workspace).unwrap();
    let home = std::env::var("HOME").unwrap();
    let too_many: Vec<Value> = (0..17)
        .map(|n| {
            let folder = place.outside.join(format!("many-{n}"));
            std::fs::create_dir(&folder).unwrap();
            json!({"path": path(&folder)})
        })
        .collect();
    for (folders, why) in [
        (json!([{"path": "relative/folder"}]), "absolute"),
        (
            json!([{"path": path(&place.outside.join("missing"))}]),
            "not a folder",
        ),
        (json!([{"path": path(&file)}]), "not a folder"),
        (json!([{"path": "/"}]), "whole disk"),
        (json!([{"path": home}]), "home folder"),
        (json!([{"path": "~"}]), "home folder"),
        (json!([{"path": path(&place.data)}]), "Hotline's own data"),
        (
            json!([{"path": path(&place.data.join("workspaces"))}]),
            "inside Hotline's own data",
        ),
        (json!([{"path": path(&place.workspace)}]), "own workspace"),
        (
            json!([{"path": path(&place.workspace.join("inner"))}]),
            "own workspace",
        ),
        (
            json!([{"path": path(&place.read_only)}, {"path": path(&place.read_only.join("inner"))}]),
            "overlap",
        ),
        (json!(too_many), "up to 16"),
    ] {
        let refused = client
            .call(
                "persona.update",
                json!({"id": persona, "patch": {"folders": folders}}),
            )
            .await;
        assert_eq!(refused["ok"], false, "{folders}: {refused}");
        assert!(
            refused["error"].to_string().contains(why),
            "{why}: {refused}"
        );
    }
    // A name patch answers the whole record as it now stands.
    let kept = client
        .call(
            "persona.update",
            json!({"id": persona, "patch": {"name": "Checked"}}),
        )
        .await;
    assert_eq!(
        kept["result"]["folders"],
        json!([{"path": path(&place.read_only), "writable": false}])
    );

    // Another teammate's workspace in the data directory may be granted, and
    // the same folder twice is one grant, editable if either said so.
    let granted = client
        .call(
            "persona.update",
            json!({"id": persona, "patch": {"folders": [
                {"path": path(&other_workspace)},
                {"path": path(&place.outside)},
                {"path": path(&place.outside), "writable": true},
            ]}}),
        )
        .await;
    assert_eq!(granted["ok"], true, "{granted}");
    assert_eq!(
        granted["result"]["folders"],
        json!([
            {"path": path(&other_workspace.canonicalize().unwrap()), "writable": false},
            {"path": path(&place.outside), "writable": true},
        ])
    );
    drop(client);
    door_task.abort();
}

/// Runs one turn and answers each tool call's id with the text its result
/// carried to the model.
async fn turn(
    client: &mut Client,
    requests: &mut tokio::sync::mpsc::UnboundedReceiver<Value>,
    persona: &str,
    text: &str,
) -> Vec<(String, String)> {
    let tape = client.subscribe(json!({"tape": persona})).await;
    let started = client
        .call("session.start", json!({"personaId": persona}))
        .await;
    assert_eq!(started["ok"], true, "{started}");
    let prompted = client
        .call(
            "session.prompt",
            json!({"personaId": persona, "text": text}),
        )
        .await;
    assert_eq!(prompted["ok"], true, "{prompted}");
    client
        .next_where(Duration::from_secs(60), |frame| {
            frame["sub"] == tape && frame["event"]["kind"] == "turn"
        })
        .await;
    let mut last = None;
    while let Ok(request) = requests.try_recv() {
        last = Some(request);
    }
    let last = last.expect("the turn reached the model");
    last["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "tool")
        .map(|message| {
            (
                message["tool_call_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                message["content"].to_string(),
            )
        })
        .collect()
}

/// The window's half of the wire, as the harness plays it.
struct Client {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    next_id: i64,
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
