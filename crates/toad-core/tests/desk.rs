//! The Phase 0 proof: a teammate is created, a session is started and a turn
//! runs, and all of it is watched through the wire the window uses.
//!
//! The first test needs nothing but this machine. The second needs a real
//! provider key in `TOAD_HARNESS_ANTHROPIC_KEY` and is skipped without one,
//! because a turn that reaches a model is the only honest proof that the
//! driver, the tools and the tape agree. The third is the same proof for the
//! other kind of agent: set `TOAD_HARNESS_ACP` to a backend id this machine
//! can run (`cursor`, say) and it drives a real harness as a child.

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use toad_core::desk::Desk;
use toad_core::wire::Door;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

const TOKEN: &str = "a-token-only-this-harness-knows";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("toad-harness-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A desk on a scratch data directory, its door running.
async fn open(name: &str) -> (PathBuf, u16) {
    let root = scratch(name);
    let port = open_at(&root);
    (root, port)
}

/// The same, on a data directory somebody has already put something in.
fn open_at(root: &Path) -> u16 {
    let desk = Desk::open(root).unwrap();
    let door = Door::bind(desk.log.clone(), TOKEN.to_string(), Arc::new(desk)).unwrap();
    let port = door.port();
    tokio::spawn(door.run());
    port
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

fn is_sub(frame: &Value, sub: i64, key: &str) -> bool {
    frame.get("sub").and_then(Value::as_i64) == Some(sub) && frame.get(key).is_some()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_teammate_is_made_watched_keyed_chaptered_and_removed_over_the_wire() {
    let (_root, port) = open("dry").await;
    let mut client = Client::connect(port).await;

    let roster = client.subscribe(json!({ "view": "roster" })).await;
    let empty = client
        .next_where(Duration::from_secs(5), |frame| {
            is_sub(frame, roster, "snapshot")
        })
        .await;
    assert_eq!(empty["snapshot"], json!([]));

    let created = client
        .call(
            "persona.create",
            json!({ "draft": { "name": "Ada", "goal": "Prove the wire." } }),
        )
        .await;
    assert_eq!(created["ok"], true, "{created}");
    let persona_id = created["result"]["id"].as_str().unwrap().to_string();
    assert_eq!(created["result"]["backendId"], "pi");

    let row = client
        .next_where(Duration::from_secs(5), |frame| {
            is_sub(frame, roster, "event")
        })
        .await;
    assert_eq!(row["event"]["persona"]["id"], persona_id);
    assert_eq!(row["event"]["session"]["state"], "idle");
    assert_eq!(row["event"]["preview"], Value::Null);

    let tape = client.subscribe(json!({ "tape": persona_id })).await;
    let blank = client
        .next_where(Duration::from_secs(5), |frame| {
            is_sub(frame, tape, "snapshot")
        })
        .await;
    assert_eq!(blank["snapshot"], json!([]));

    // No key on the desk: the built-in agent says so instead of starting.
    let refused = client
        .call("session.start", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("key"),
        "{refused}"
    );

    let none = client.call("models.list", json!({})).await;
    assert_eq!(none["result"], json!([]));
    let keyed = client
        .call(
            "credential.create",
            json!({ "providerId": "anthropic", "label": "harness", "secret": "sk-ant-harness" }),
        )
        .await;
    assert_eq!(keyed["ok"], true, "{keyed}");
    let some = client.call("models.list", json!({})).await;
    let ids: Vec<&str> = some["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|model| model["id"].as_str().unwrap())
        .collect();
    assert!(
        !ids.is_empty() && ids.iter().all(|id| id.starts_with("anthropic/")),
        "{some}"
    );

    // A session opens a chapter, and closing it is one command. Nothing was
    // said in this one, so it closes untitled without a model being asked.
    let started = client
        .call("session.start", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(started["ok"], true, "{started}");
    let marker = client
        .next_where(Duration::from_secs(5), |frame| {
            is_sub(frame, tape, "event") && frame["event"]["kind"] == "chapter"
        })
        .await;
    assert_eq!(marker["event"]["backendId"], "pi");
    assert_eq!(marker["event"].get("endedAt"), None);

    let closed = client
        .call("chapter.start_fresh", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(closed["ok"], true, "{closed}");
    assert_eq!(closed["result"]["id"], marker["event"]["id"]);
    assert_eq!(closed["result"]["messages"], 0);
    assert_eq!(closed["result"].get("title"), None);

    let gone = client
        .call("persona.delete", json!({ "id": persona_id }))
        .await;
    assert_eq!(gone["ok"], true, "{gone}");
    let removed = client
        .next_where(Duration::from_secs(5), |frame| {
            is_sub(frame, roster, "removed")
        })
        .await;
    assert_eq!(removed["removed"], persona_id);
}

/// A subscription's account list cannot be refreshed without a login. The
/// fake room is allowed to stub this; the desk is the one that knows.
#[tokio::test(flavor = "multi_thread")]
async fn credential_refresh_models_needs_a_login() {
    let (_root, port) = open("refresh-models").await;
    let mut client = Client::connect(port).await;
    let refused = client
        .call(
            "credential.refresh_models",
            json!({ "providerId": "github-copilot" }),
        )
        .await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(
        refused["error"].as_str(),
        Some("There is no sign-in for GitHub Copilot.")
    );
}

/// The catalogue is listed whether or not a credential is held, and a saved
/// filter flags it. The fake room cannot see settings, so this is the desk.
#[tokio::test(flavor = "multi_thread")]
async fn models_catalog_flags_a_saved_filter_and_refuses_an_unwired_provider() {
    let (_root, port) = open("models-catalog").await;
    let mut client = Client::connect(port).await;

    let nope = client
        .call("models.catalog", json!({ "providerId": "nope" }))
        .await;
    assert_eq!(nope["ok"], false, "{nope}");
    assert_eq!(
        nope["error"].as_str(),
        Some("nope is not a provider Toad Agent can use.")
    );

    let listed = client
        .call("models.catalog", json!({ "providerId": "anthropic" }))
        .await;
    assert_eq!(listed["ok"], true, "{listed}");
    let models = listed["result"].as_array().expect("a catalogue is a list");
    assert!(!models.is_empty());
    assert!(
        models.iter().all(|model| model["enabled"] == true),
        "{listed}"
    );
    let kept = models[0]["id"].as_str().unwrap().to_string();

    let patched = client
        .call(
            "settings.update",
            json!({ "patch": { "enabledModels": { "anthropic": [kept] } } }),
        )
        .await;
    assert_eq!(patched["ok"], true, "{patched}");

    let filtered = client
        .call("models.catalog", json!({ "providerId": "anthropic" }))
        .await;
    assert_eq!(filtered["ok"], true, "{filtered}");
    let flagged = filtered["result"].as_array().unwrap();
    assert_eq!(flagged.len(), models.len());
    let on: Vec<&str> = flagged
        .iter()
        .filter(|model| model["enabled"] == true)
        .map(|model| model["id"].as_str().unwrap())
        .collect();
    assert_eq!(on, [kept.as_str()]);
}

/// A peer thread, listed and read over the wire.
///
/// The core is the only writer of a stream, so the two teammates and the
/// conversation between them are written into the data directory before the
/// core opens — which is also exactly what an imported one looks like.
#[tokio::test(flavor = "multi_thread")]
async fn a_peer_thread_is_listed_streamed_and_marked_read_over_the_wire() {
    let root = scratch("peers");
    let teammate = |id: &str, name: &str| {
        json!({
            "kind": "persona", "id": id, "name": name, "goal": "", "backendId": "pi",
            "cwd": root.to_string_lossy(), "mcpPolicy": { "mode": "all", "serverIds": [] },
            "sessionCheckpoints": [], "createdAt": 1, "updatedAt": 1,
        })
    };
    std::fs::write(
        root.join("room.jsonl"),
        format!("{}\n{}\n", teammate("ada", "Ada"), teammate("bob", "Bob")),
    )
    .unwrap();
    let threads = root.join("threads");
    std::fs::create_dir_all(&threads).unwrap();
    std::fs::write(
        threads.join("ada~bob.json"),
        json!({
            "version": 1, "a": "ada", "b": "bob",
            "sides": { "user": "ada", "agent": "bob" },
            "sessions": [], "createdAt": 1, "updatedAt": 1,
        })
        .to_string(),
    )
    .unwrap();
    let said = [
        json!({ "kind": "user", "id": "u1", "ts": 10, "text": "did the crane jam?", "receipt": "read" }),
        json!({ "kind": "agent", "id": "a1", "ts": 11, "text": "on the second lift", "receipt": "sent" }),
        json!({ "kind": "turn", "id": "t1", "ts": 12, "stopReason": "end_turn" }),
    ];
    std::fs::write(
        threads.join("ada~bob.jsonl"),
        said.iter()
            .map(|event| format!("{event}\n"))
            .collect::<String>(),
    )
    .unwrap();

    let mut client = Client::connect(open_at(&root)).await;
    let thread = client.subscribe(json!({ "thread": "ada~bob" })).await;
    let snapshot = client
        .next_where(Duration::from_secs(5), |frame| {
            is_sub(frame, thread, "snapshot")
        })
        .await;
    assert_eq!(snapshot["snapshot"], json!(said));

    // The same conversation from each side, named by the other.
    let listed = client
        .call("peers.list", json!({ "personaId": "ada" }))
        .await;
    assert_eq!(listed["ok"], true, "{listed}");
    let summary = &listed["result"][0];
    assert_eq!(summary["threadKey"], "ada~bob");
    assert_eq!(summary["withPersonaId"], "bob");
    assert_eq!(summary["withName"], "Bob");
    assert_eq!(summary["exchanges"], 1);
    assert_eq!(summary["lastAt"], 12);
    assert_eq!(summary["waiting"], false);
    assert_eq!(summary["preview"]["fromName"], "Bob");
    assert_eq!(summary["preview"]["text"], "on the second lift");
    let theirs = client
        .call("peers.list", json!({ "personaId": "bob" }))
        .await;
    assert_eq!(theirs["result"][0]["withName"], "Ada");

    // The reply is read, and the subscription is told on the same id.
    let read = client
        .call(
            "peers.mark_read",
            json!({ "key": "ada~bob", "eventIds": ["a1"] }),
        )
        .await;
    assert_eq!(read["result"], 1, "{read}");
    let moved = client
        .next_where(Duration::from_secs(5), |frame| {
            is_sub(frame, thread, "event")
        })
        .await;
    assert_eq!(moved["event"]["id"], "a1");
    assert_eq!(moved["event"]["receipt"], "read");

    // A receipt that has already landed moves nothing the second time.
    let again = client
        .call(
            "peers.mark_read",
            json!({ "key": "ada~bob", "eventIds": ["a1", "no-such-id"] }),
        )
        .await;
    assert_eq!(again["result"], 0, "{again}");
}

/// A real turn, with a real key: the agent reads a file in its workspace with
/// a tool and answers from it, and every step reaches the tape and the wire.
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_with_a_real_key_reads_a_file_and_answers_from_it() {
    let Ok(key) = std::env::var("TOAD_HARNESS_ANTHROPIC_KEY") else {
        eprintln!("skipped: set TOAD_HARNESS_ANTHROPIC_KEY to run a real turn");
        return;
    };
    let (root, port) = open("live").await;
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("note.txt"), "the harbour is open\n").unwrap();

    let mut client = Client::connect(port).await;
    let keyed = client
        .call(
            "credential.create",
            json!({ "providerId": "anthropic", "label": "harness", "secret": key }),
        )
        .await;
    assert_eq!(keyed["ok"], true, "{keyed}");

    let created = client
        .call(
            "persona.create",
            json!({ "draft": { "name": "Ada", "goal": "Answer plainly.", "cwd": workspace.to_string_lossy() } }),
        )
        .await;
    let persona_id = created["result"]["id"].as_str().unwrap().to_string();
    let roster = client.subscribe(json!({ "view": "roster" })).await;
    let tape = client.subscribe(json!({ "tape": persona_id })).await;

    let started = client
        .call("session.start", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(started["ok"], true, "{started}");
    assert_eq!(started["result"]["state"], "ready");

    let sent = client
        .call(
            "session.prompt",
            json!({ "personaId": persona_id, "text": "Use your read tool on note.txt and reply with only its contents." }),
        )
        .await;
    assert_eq!(sent["ok"], true, "{sent}");

    let patience = Duration::from_secs(120);
    let user = client
        .next_where(patience, |frame| {
            is_sub(frame, tape, "event") && frame["event"]["kind"] == "user"
        })
        .await;
    assert!(user["event"]["text"].as_str().unwrap().contains("note.txt"));
    let thinking = client
        .next_where(patience, |frame| {
            is_sub(frame, roster, "event") && frame["event"]["session"]["state"] == "thinking"
        })
        .await;
    assert_eq!(thinking["event"]["persona"]["id"], persona_id);
    let tool = client
        .next_where(patience, |frame| {
            is_sub(frame, tape, "event")
                && frame["event"]["kind"] == "tool"
                && frame["event"]["status"] == "completed"
        })
        .await;
    assert_eq!(tool["event"]["toolKind"], "read", "{tool}");
    let answer = client
        .next_where(patience, |frame| {
            is_sub(frame, tape, "event") && frame["event"]["kind"] == "agent"
        })
        .await;
    assert!(
        answer["event"]["text"]
            .as_str()
            .unwrap()
            .contains("harbour is open"),
        "{answer}"
    );
    let turn = client
        .next_where(patience, |frame| {
            is_sub(frame, tape, "event") && frame["event"]["kind"] == "turn"
        })
        .await;
    assert_eq!(turn["event"]["stopReason"], "end_turn");
    client
        .next_where(patience, |frame| {
            is_sub(frame, roster, "event") && frame["event"]["session"]["state"] == "ready"
        })
        .await;

    // The tape on disk folds to what the wire showed.
    let log = toad_core::log::Log::open(&root);
    let kinds: Vec<&str> = log
        .load(&toad_core::log::StreamId::Tape(persona_id.clone()))
        .iter()
        .map(|event| event["kind"].as_str().unwrap().to_string())
        .collect::<Vec<_>>()
        .leak()
        .iter()
        .map(String::as_str)
        .collect();
    assert_eq!(kinds.first(), Some(&"user"));
    assert!(
        kinds.contains(&"tool") && kinds.contains(&"agent") && kinds.last() == Some(&"turn"),
        "{kinds:?}"
    );
}

/// A real ACP harness, as a child: it starts in the teammate's own workspace,
/// answers a prompt, and everything it says reaches the tape and the wire.
///
/// The backend is named rather than assumed, because which harness is
/// installed and logged in is a fact about the machine and not about Toad.
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_on_a_real_acp_harness_reaches_the_tape() {
    let Ok(backend_id) = std::env::var("TOAD_HARNESS_ACP") else {
        eprintln!("skipped: set TOAD_HARNESS_ACP to a backend id (e.g. cursor) to drive a harness");
        return;
    };
    let (root, port) = open("acp").await;
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let mut client = Client::connect(port).await;
    let created = client
        .call(
            "persona.create",
            json!({ "draft": {
                "name": "Ada",
                "goal": "Answer plainly.",
                "backendId": backend_id,
                "cwd": workspace.to_string_lossy(),
            } }),
        )
        .await;
    assert_eq!(created["ok"], true, "{created}");
    let persona_id = created["result"]["id"].as_str().unwrap().to_string();
    let tape = client.subscribe(json!({ "tape": persona_id })).await;

    let started = client
        .call("session.start", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(started["ok"], true, "{started}");
    assert_eq!(started["result"]["state"], "ready");
    assert!(
        started["result"]["sessionId"].is_string(),
        "the harness issued no session id: {started}"
    );

    // Identity reaches an ACP agent as a file, because its session takes no
    // system prompt.
    let identity = std::fs::read_to_string(workspace.join("AGENTS.md")).unwrap();
    assert!(
        identity.starts_with("<!-- managed by Toad -->"),
        "{identity}"
    );
    assert!(identity.contains("Answer plainly."), "{identity}");

    let sent = client
        .call(
            "session.prompt",
            json!({ "personaId": persona_id, "text": "reply with the single word pond" }),
        )
        .await;
    assert_eq!(sent["ok"], true, "{sent}");

    let patience = Duration::from_secs(180);
    let answer = client
        .next_where(patience, |frame| {
            is_sub(frame, tape, "event") && frame["event"]["kind"] == "agent"
        })
        .await;
    assert!(
        answer["event"]["text"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("pond"),
        "{answer}"
    );
    client
        .next_where(patience, |frame| {
            is_sub(frame, tape, "event") && frame["event"]["kind"] == "turn"
        })
        .await;

    // A turn completed on a fresh session, so the agent's own id for this
    // conversation is now on the teammate's record.
    let log = toad_core::log::Log::open(&root);
    let checkpoints: Vec<String> = toad_core::room::roster(&log)
        .into_iter()
        .find(|persona| persona.id == persona_id)
        .expect("the teammate is on the roster")
        .session_checkpoints
        .into_iter()
        .map(|checkpoint| checkpoint.backend_id)
        .collect();
    assert_eq!(checkpoints, [backend_id]);

    let stopped = client
        .call("session.stop", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(stopped["ok"], true, "{stopped}");
}
