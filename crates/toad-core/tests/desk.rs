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

#[tokio::test]
async fn grok_signout_removes_tokens_and_zai_plans_have_separate_connections() {
    let root = scratch("grok-zai-connections");
    let log = toad_core::log::Log::open(&root);
    let vault = toad_core::vault::Vault::open(&root, log).unwrap();
    // A completed fake login is written before the core starts, like an
    // imported credential. The wire must never expose this private half.
    let (id, dir) = vault.begin_login("xai").unwrap();
    std::fs::write(dir.join("auth.json"), json!({"access_token":"private-grok-access", "refresh_token":"private-grok-refresh", "refresh_at":u64::MAX}).to_string()).unwrap();
    vault.finish_login(&id, "xai", "SuperGrok").unwrap();
    drop(vault);
    let port = open_at(&root);
    let mut client = Client::connect(port).await;
    let providers = client.call("providers.list", json!({})).await;
    let providers = providers["result"].as_array().unwrap();
    for (id, kinds) in [
        ("xai", json!(["oauth", "api_key"])),
        ("zai", json!(["api_key"])),
        ("zai-coding-plan", json!(["api_key"])),
    ] {
        assert_eq!(
            providers.iter().find(|p| p["id"] == id).unwrap()["credentialKinds"],
            kinds
        );
    }
    let credentials = client.call("credential.list", json!({})).await;
    assert_eq!(credentials["result"][0]["credentialKind"], "oauth");
    assert!(!credentials.to_string().contains("private-grok"));
    let models = client.call("models.list", json!({})).await;
    assert!(
        models["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"].as_str().unwrap().starts_with("xai/"))
    );
    let revoked = client.call("credential.revoke", json!({"id":id})).await;
    assert_eq!(revoked["ok"], true, "{revoked}");
    assert!(!dir.exists());
    assert_eq!(
        client.call("models.list", json!({})).await["result"],
        json!([])
    );

    let standard = client
        .call(
            "credential.create",
            json!({"providerId":"zai", "label":"standard", "secret":"private-standard-key"}),
        )
        .await;
    assert_eq!(standard["ok"], true, "{standard}");
    let models = client.call("models.list", json!({})).await;
    assert!(!models["result"].as_array().unwrap().is_empty());
    assert!(
        models["result"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["id"].as_str().unwrap().starts_with("zai/"))
    );
    let coding = client.call("credential.create", json!({"providerId":"zai-coding-plan", "label":"coding", "secret":"private-coding-key"})).await;
    assert_eq!(coding["ok"], true, "{coding}");
    let removed = client
        .call("credential.delete", json!({"id":standard["result"]["id"]}))
        .await;
    assert_eq!(removed["ok"], true, "{removed}");
    let models = client.call("models.list", json!({})).await;
    assert!(!models["result"].as_array().unwrap().is_empty());
    assert!(
        models["result"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["id"].as_str().unwrap().starts_with("zai-coding-plan/"))
    );
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let made = client.call("persona.create", json!({"draft":{"name":"Zai tester", "goal":"Check the connection.", "cwd":workspace.to_string_lossy()}})).await;
    let started = client
        .call("session.start", json!({"personaId":made["result"]["id"]}))
        .await;
    assert_eq!(started["ok"], true, "{started}");
    let credentials = client.call("credential.list", json!({})).await;
    assert!(!credentials.to_string().contains("private-"));
}

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
        Some("There is no connection for github-copilot.")
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

/// The reach selected over the real wire governs the real tools. No model is
/// needed to choose an attack command, and a provider cannot skip the probe.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn workspace_reach_keeps_another_projects_env_out_of_the_tools() {
    use rig::tool::{Tool, ToolContext};
    use toad_core::contract::Persona;
    use toad_core::tools::{ReadFile, RunCommand, Workspace};

    let probe = std::process::Command::new("bwrap")
        .args(["--ro-bind", "/", "/", "--unshare-pid", "/bin/true"])
        .output();
    if !probe.is_ok_and(|output| output.status.success()) {
        eprintln!("skipped: this machine cannot run bubblewrap");
        return;
    }
    let (root, port) = open("workspace-reach").await;
    let workspace = root.join("project");
    let other = root.join("other-project");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    let secret = other.join(".env");
    std::fs::write(&secret, "OTHER_PROJECT_TOKEN=outside-canary").unwrap();
    std::os::unix::fs::symlink("../other-project/.env", workspace.join("escape")).unwrap();
    let mut client = Client::connect(port).await;
    let created = client
        .call(
            "persona.create",
            json!({"draft": {
                "name": "Confined", "goal": "Prove workspace reach.", "cwd": workspace,
            }}),
        )
        .await;
    assert_eq!(created["ok"], true, "{created}");
    let mut persona: Persona = serde_json::from_value(created["result"].clone()).unwrap();

    for (reach, allowed) in [
        ("workspace", false),
        ("machine", true),
        ("workspace", false),
    ] {
        let updated = client
            .call(
                "persona.update",
                json!({
                    "id": persona.id, "patch": {"reach": reach},
                }),
            )
            .await;
        assert_eq!(updated["ok"], true, "{updated}");
        persona = serde_json::from_value(updated["result"].clone()).unwrap();
        let tools = Workspace::open(
            PathBuf::from(&persona.cwd),
            persona.reach.unwrap_or_default(),
            root.join("tool-output").join(&persona.id),
        )
        .unwrap();
        for path in ["../other-project/.env", "escape"] {
            let read = ReadFile::new(tools.clone())
                .call(
                    &mut ToolContext::new(),
                    serde_json::from_value(json!({"path": path})).unwrap(),
                )
                .await;
            assert_eq!(read.is_ok(), allowed, "{reach}: {path}: {read:?}");
            let output = RunCommand::new(tools.clone())
                .call(
                    &mut ToolContext::new(),
                    serde_json::from_value(json!({"command": format!("cat {path}")})).unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                output.contains("outside-canary"),
                allowed,
                "{reach}: {output}"
            );
            assert_eq!(
                output.contains("[exit status"),
                !allowed,
                "{reach}: {output}"
            );
        }
    }
}

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
    let mut client = Client::connect(port).await;
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
    let reopened = Desk::open(&root).unwrap();
    assert_eq!(
        toad_core::wire::RoomHandle::models(&reopened)[0].id,
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
    assert!(!request["tools"].as_array().unwrap().is_empty());
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
async fn cancelling_openrouter_login_closes_callback_and_discards_pending_credentials() {
    let (root, port) = open("openrouter-cancel").await;
    let mut client = Client::connect(port).await;
    let prompt = client
        .call("credential.login", json!({"providerId":"openrouter"}))
        .await;
    assert_eq!(prompt["ok"], true, "{prompt}");
    assert_eq!(prompt["result"]["userCode"], "");
    let id = prompt["result"]["loginId"].as_str().unwrap();
    let authorize = url::Url::parse(prompt["result"]["verificationUri"].as_str().unwrap()).unwrap();
    assert_eq!(authorize.host_str(), Some("openrouter.ai"));
    let callback = authorize
        .query_pairs()
        .find(|(key, _)| key == "callback_url")
        .unwrap()
        .1
        .into_owned();
    let cancel = client
        .call("credential.login_cancel", json!({"loginId":id}))
        .await;
    assert_eq!(cancel["ok"], true, "{cancel}");
    let status = client
        .call("credential.login_status", json!({"loginId":id}))
        .await;
    assert_eq!(status["result"]["state"], "failed");
    let dir = root.join("vault/logins").join(id);
    tokio::time::timeout(Duration::from_secs(3), async {
        while dir.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(reqwest::get(callback).await.is_err());
    assert_eq!(
        client.call("credential.list", json!({})).await["result"],
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
    let mut client = Client::connect(port).await;
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
    let reopened = Desk::open(&root).unwrap();
    assert_eq!(toad_core::wire::RoomHandle::models(&reopened).len(), 2);
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
        let mut client = Client::connect(port).await;
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
/// next request contains the file contents from Toad's actual workspace tool.
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

#[tokio::test]
async fn discovery_and_manual_ids_preserve_filters_selections_and_offline_data_over_the_wire() {
    use axum::{Router, routing::get};
    use std::sync::Mutex;
    use toad_core::wire::RoomHandle;
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
    let mut wire = Client::connect(port).await;
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
    let reopened = Desk::open(&root).unwrap();
    assert_eq!(
        serde_json::to_value(reopened.models_catalog("ollama").unwrap()).unwrap(),
        saved["result"]
    );
    let persisted = toad_core::room::settings(&reopened.log);
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
    use toad_core::{log::Log, vault::Vault, wire::RoomHandle};
    let root = scratch("native-discovery-metadata");
    let log = Log::open(&root);
    let vault = Vault::open(&root, log).unwrap();
    let anthropic = vault
        .create("anthropic", "Anthropic key", "private-test-key")
        .unwrap();
    let router = vault
        .create("openrouter", "Router key", "private-router-key")
        .unwrap();
    let (copilot_id, copilot_dir) = vault.begin_login("github-copilot").unwrap();
    vault
        .finish_login(&copilot_id, "github-copilot", "Copilot")
        .unwrap();
    let known = toad_core::models::catalog().providers["anthropic"]
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
        {"id":"released-after-toad","name":"New provider model","output_limit":4096}
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
    let mut wire = Client::connect(open_at(&root)).await;
    let providers = wire.call("providers.list", json!({})).await;
    for provider in providers["result"].as_array().unwrap() {
        let supported = [
            "anthropic",
            "openai",
            "openrouter",
            "google",
            "groq",
            "deepseek",
            "mistral",
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
    let reopened = Desk::open(&root).unwrap();
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
