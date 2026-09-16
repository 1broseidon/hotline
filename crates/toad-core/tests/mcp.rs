//! Toad as an MCP client, driven over the wire the window uses.
//!
//! An echo server is spawned as a stdio child the persona is granted. The
//! ledger is what the session was actually built with: a verified row when
//! the tool attached, no MCP rows under a policy of none, and an absent row
//! — with the error as its reason — when the child could not start.

mod common;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use toad_core::log::{Log, StreamId};
use toad_core::mcp::{self, McpServer, McpTransport};
use toad_core::wire::Door;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

const TOKEN: &str = "a-token-only-this-harness-knows";

/// Path to the one-tool stdio server this package builds. Cargo used to
/// inject it at compile time; this toolchain only sets it at run time, and
/// when it does not, the bin is one directory up from this test's own
/// `deps`, wherever the target directory lives.
fn echo_command() -> String {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_toad_mcp_echo") {
        return path;
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| {
            let name = format!("toad-mcp-echo{}", std::env::consts::EXE_SUFFIX);
            Some(exe.parent()?.parent()?.join(name))
        })
        .filter(|path| path.exists())
        .expect("toad-mcp-echo should have been built with this test")
        .to_string_lossy()
        .into_owned()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "toad-harness-mcp-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

async fn open(name: &str) -> (PathBuf, u16) {
    let root = scratch(name);
    let desk = common::open_desk(&root).unwrap();
    let door = Door::bind(desk.log.clone(), TOKEN.to_string(), Arc::new(desk)).unwrap();
    let port = door.port();
    tokio::spawn(door.run());
    (root, port)
}

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
}

fn echo_server() -> Value {
    json!({
        "id": "echo",
        "type": "stdio",
        "name": "Echo",
        "command": echo_command(),
        "args": [],
    })
}

async fn keyed(client: &mut Client) {
    let made = client
        .call(
            "credential.create",
            json!({ "providerId": "anthropic", "label": "test", "secret": "sk-test" }),
        )
        .await;
    assert_eq!(made["ok"], true, "{made}");
}

async fn set_policy(client: &mut Client, persona_id: &str, mode: &str, server_ids: &[&str]) {
    let patched = client
        .call(
            "persona.update",
            json!({
                "id": persona_id,
                "patch": { "mcpPolicy": { "mode": mode, "serverIds": server_ids } },
            }),
        )
        .await;
    assert_eq!(patched["ok"], true, "{patched}");
}

async fn mcp_tools(client: &mut Client, persona_id: &str) -> Vec<String> {
    let tools = client
        .call("teammate.tools", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(tools["ok"], true, "{tools}");
    let mut names: Vec<String> = tools["result"]["rows"]
        .as_array()
        .expect("a started session has a ledger")
        .iter()
        .filter(|row| row["source"] == "mcp")
        .map(|row| {
            assert_eq!(row["state"], "verified", "{row}");
            row["name"].as_str().unwrap().to_string()
        })
        .collect();
    names.sort();
    names
}

#[tokio::test(flavor = "multi_thread")]
async fn oauth_client_secrets_are_removed_before_mcp_settings_are_written() {
    let (root, port) = open("oauth-settings-boundary").await;
    let mut client = Client::connect(port).await;
    let updated = client
        .call(
            "settings.update",
            json!({
                "patch": {
                    "mcpServers": [{
                        "id": "oauth",
                        "type": "http",
                        "name": "OAuth",
                        "url": "https://mcp.example.test/mcp",
                        "auth": {
                            "mode": "oauth",
                            "scopes": ["mcp"],
                            "resource": "https://mcp.example.test/mcp",
                            "client": {
                                "clientId": "public-client",
                                "clientSecret": "must-not-persist"
                            }
                        }
                    }]
                }
            }),
        )
        .await;
    assert_eq!(updated["ok"], true, "{updated}");
    let settings = updated["result"].to_string();
    assert!(!settings.contains("must-not-persist"), "{settings}");
    let room = Log::open(&root).load(&StreamId::Room);
    let room = serde_json::to_string(&room).unwrap();
    assert!(!room.contains("must-not-persist"), "{room}");

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_granted_server_lists_its_tool_as_verified_and_a_scripted_call_reaches_it() {
    let (_root, port) = open("granted").await;
    let mut client = Client::connect(port).await;
    keyed(&mut client).await;

    let settings = client
        .call(
            "settings.update",
            json!({ "patch": { "mcpServers": [echo_server()] } }),
        )
        .await;
    assert_eq!(settings["ok"], true, "{settings}");
    assert_eq!(settings["result"]["mcpServers"][0]["id"], "echo");

    let created = client
        .call(
            "persona.create",
            json!({ "draft": { "name": "Ada", "goal": "Shout." } }),
        )
        .await;
    assert_eq!(created["ok"], true, "{created}");
    let persona_id = created["result"]["id"].as_str().unwrap().to_string();

    set_policy(&mut client, &persona_id, "some", &["echo"]).await;

    let started = client
        .call("session.start", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(started["ok"], true, "{started}");

    let tools = client
        .call("teammate.tools", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(tools["ok"], true, "{tools}");
    let rows = tools["result"]["rows"].as_array().unwrap();
    let shout = rows
        .iter()
        .find(|row| row["name"] == "echo__shout")
        .expect("the echo tool is on the ledger");
    assert_eq!(shout["source"], "mcp");
    assert_eq!(shout["origin"], "echo");
    assert_eq!(shout["state"], "verified");
    assert!(!shout["reason"].as_str().unwrap().is_empty());
    assert!(
        rows.iter().any(|row| row["name"] == "read"
            && row["source"] == "builtin"
            && row["state"] == "verified"),
        "{rows:?}"
    );

    let connected = mcp::connect(
        &persona_id,
        &[McpServer {
            id: "echo".into(),
            name: "Echo".into(),
            transport: McpTransport::Stdio {
                command: echo_command(),
                args: Vec::new(),
                env: Default::default(),
            },
            refuse: None,
        }],
    )
    .await;
    assert_eq!(connected.failed.len(), 0, "{:?}", connected.failed);
    let tool = connected
        .tools
        .iter()
        .find(|tool| tool.name == "echo__shout")
        .expect("connect listed shout");
    let shouted = tool
        .call(json!({ "text": "harbour" }))
        .await
        .expect("the call reached the server");
    assert_eq!(shouted.text, "HARBOUR");
}

/// A command with arguments, saved the way the Tools form saves it: the
/// arguments leave the room stream for the vault and come back at start,
/// so a server that is nothing without them still connects. The command
/// here is the shell, which without its `-c …` would sit reading stdin and
/// never answer the handshake.
#[tokio::test(flavor = "multi_thread")]
async fn a_stdio_servers_arguments_come_back_from_the_vault_at_start() {
    let (root, port) = open("args").await;
    let mut client = Client::connect(port).await;
    keyed(&mut client).await;

    let settings = client
        .call(
            "settings.update",
            json!({ "patch": { "mcpServers": [{
                "id": "shelled",
                "type": "stdio",
                "name": "Shelled",
                "command": "sh",
                "args": ["-c", format!("exec '{}'", echo_command())],
            }] } }),
        )
        .await;
    assert_eq!(settings["ok"], true, "{settings}");
    let public = &settings["result"]["mcpServers"][0];
    assert_eq!(public["command"], "sh");
    assert_eq!(
        public["args"],
        json!([]),
        "the arguments are not on the stream"
    );
    assert!(public["credentialRef"].is_string(), "{public}");
    let disk = std::fs::read_to_string(root.join("room.jsonl")).unwrap();
    assert!(
        !disk.contains("exec '"),
        "the arguments are not on disk either"
    );

    // Saved again unchanged, as the form does when another server is added
    // or this one is renamed, the reference stands and nothing is lost.
    let renamed = client
        .call(
            "settings.update",
            json!({ "patch": { "mcpServers": [{
                "id": "shelled",
                "type": "stdio",
                "name": "Shelled again",
                "command": "sh",
                "args": [],
                "credentialRef": public["credentialRef"],
            }] } }),
        )
        .await;
    assert_eq!(renamed["ok"], true, "{renamed}");

    let created = client
        .call(
            "persona.create",
            json!({ "draft": { "name": "Ada", "goal": "Shout through a shell." } }),
        )
        .await;
    let persona_id = created["result"]["id"].as_str().unwrap().to_string();
    set_policy(&mut client, &persona_id, "some", &["shelled"]).await;
    let started = client
        .call("session.start", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(started["ok"], true, "{started}");
    let tools = client
        .call("teammate.tools", json!({ "personaId": persona_id }))
        .await;
    let rows = tools["result"]["rows"].as_array().unwrap();
    let shout = rows
        .iter()
        .find(|row| row["origin"] == "shelled")
        .expect("the shelled server is on the ledger");
    assert_eq!(shout["state"], "verified", "{shout}");
    assert_eq!(shout["name"], "shelled_again__shout");
}

/// Toad's own tools are the teammate's whether or not anything else is:
/// built in this process for Toad Agent, and verified because Toad built them.
#[tokio::test(flavor = "multi_thread")]
async fn toad_agent_gets_toads_own_tools() {
    let (_root, port) = open("toad-tools").await;
    let mut client = Client::connect(port).await;
    keyed(&mut client).await;

    let created = client
        .call(
            "persona.create",
            json!({ "draft": { "name": "Ada", "goal": "Remember." } }),
        )
        .await;
    let persona_id = created["result"]["id"].as_str().unwrap().to_string();
    let started = client
        .call("session.start", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(started["ok"], true, "{started}");

    let tools = client
        .call("teammate.tools", json!({ "personaId": persona_id }))
        .await;
    let rows = tools["result"]["rows"].as_array().unwrap();
    for name in toad_core::mcp::server::TOOL_NAMES {
        let row = rows
            .iter()
            .find(|row| row["name"] == name)
            .unwrap_or_else(|| panic!("{name} is on the ledger: {rows:?}"));
        assert_eq!(row["source"], "builtin");
        assert_eq!(row["origin"], "toad");
        assert_eq!(row["state"], "verified");
        assert!(!row["reason"].as_str().unwrap().is_empty());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_teammate_gets_no_gateway_tools_even_with_machine_reach() {
    let (_root, port) = open("none").await;
    let mut client = Client::connect(port).await;
    keyed(&mut client).await;

    client
        .call(
            "settings.update",
            json!({ "patch": { "mcpServers": [echo_server()] } }),
        )
        .await;
    let created = client
        .call(
            "persona.create",
            json!({ "draft": { "name": "Ada", "goal": "Quiet." } }),
        )
        .await;
    let persona_id = created["result"]["id"].as_str().unwrap().to_string();
    assert_eq!(
        created["result"]["mcpPolicy"],
        json!({ "mode": "none", "serverIds": [] })
    );

    let started = client
        .call("session.start", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(started["ok"], true, "{started}");

    assert!(mcp_tools(&mut client, &persona_id).await.is_empty());

    let patched = client
        .call(
            "persona.update",
            json!({ "id": persona_id, "patch": { "reach": "machine" } }),
        )
        .await;
    assert_eq!(patched["ok"], true, "{patched}");
    assert!(mcp_tools(&mut client, &persona_id).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn gateway_access_is_explicit_and_changes_rebuild_the_live_tools() {
    let (_root, port) = open("gateway").await;
    let mut client = Client::connect(port).await;
    keyed(&mut client).await;
    let created = client
        .call("persona.create", json!({ "draft": { "name": "Ada" } }))
        .await;
    assert_eq!(created["ok"], true, "{created}");
    let persona_id = created["result"]["id"].as_str().unwrap().to_string();
    let started = client
        .call("session.start", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(started["ok"], true, "{started}");

    let mut servers = vec![echo_server()];
    let updated = client
        .call(
            "settings.update",
            json!({ "patch": { "mcpServers": servers } }),
        )
        .await;
    assert_eq!(updated["ok"], true, "{updated}");
    assert!(mcp_tools(&mut client, &persona_id).await.is_empty());

    set_policy(&mut client, &persona_id, "some", &["echo"]).await;
    assert_eq!(mcp_tools(&mut client, &persona_id).await, ["echo__shout"]);

    let mut other = echo_server();
    other["id"] = json!("other");
    other["name"] = json!("Other");
    servers.push(other);
    let updated = client
        .call(
            "settings.update",
            json!({ "patch": { "mcpServers": servers } }),
        )
        .await;
    assert_eq!(updated["ok"], true, "{updated}");
    assert_eq!(mcp_tools(&mut client, &persona_id).await, ["echo__shout"]);

    set_policy(&mut client, &persona_id, "all", &["echo"]).await;
    assert_eq!(
        mcp_tools(&mut client, &persona_id).await,
        ["echo__shout", "other__shout"]
    );
    let mut later = echo_server();
    later["id"] = json!("later");
    later["name"] = json!("Later");
    servers.push(later);
    let updated = client
        .call(
            "settings.update",
            json!({ "patch": { "mcpServers": servers } }),
        )
        .await;
    assert_eq!(updated["ok"], true, "{updated}");
    assert_eq!(
        mcp_tools(&mut client, &persona_id).await,
        ["echo__shout", "later__shout", "other__shout"]
    );

    // Retained selections are dormant while access is off.
    set_policy(&mut client, &persona_id, "none", &["echo"]).await;
    assert!(mcp_tools(&mut client, &persona_id).await.is_empty());
    set_policy(&mut client, &persona_id, "some", &["echo"]).await;
    assert_eq!(mcp_tools(&mut client, &persona_id).await, ["echo__shout"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_fails_to_start_is_absent_with_the_error_as_its_reason() {
    let (_root, port) = open("absent").await;
    let mut client = Client::connect(port).await;
    keyed(&mut client).await;

    let gone = format!(
        "/no-such-toad-mcp-server-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    client
        .call(
            "settings.update",
            json!({ "patch": { "mcpServers": [{
                "id": "gone",
                "type": "stdio",
                "name": "Gone",
                "command": gone,
                "args": [],
            }] } }),
        )
        .await;
    let created = client
        .call(
            "persona.create",
            json!({ "draft": { "name": "Ada", "goal": "Notice the gap." } }),
        )
        .await;
    let persona_id = created["result"]["id"].as_str().unwrap().to_string();
    set_policy(&mut client, &persona_id, "some", &["gone"]).await;
    let started = client
        .call("session.start", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(started["ok"], true, "{started}");

    let tools = client
        .call("teammate.tools", json!({ "personaId": persona_id }))
        .await;
    let rows = tools["result"]["rows"].as_array().unwrap();
    let gone = rows
        .iter()
        .find(|row| row["origin"] == "gone")
        .expect("the failed server is on the ledger");
    assert_eq!(gone["source"], "mcp");
    assert_eq!(gone["state"], "absent");
    assert!(!gone["reason"].as_str().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_malformed_settings_entry_is_skipped() {
    let (_root, port) = open("malformed").await;
    let mut client = Client::connect(port).await;
    let updated = client
        .call(
            "settings.update",
            json!({ "patch": { "mcpServers": [
                { "id": "no-name", "type": "stdio", "command": "echo" },
                echo_server(),
                "not an object",
            ] } }),
        )
        .await;
    assert_eq!(updated["ok"], true, "{updated}");
    let servers = updated["result"]["mcpServers"].as_array().unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0]["id"], "echo");
}

#[tokio::test(flavor = "multi_thread")]
async fn teammate_tools_is_null_before_a_session_has_started() {
    let (_root, port) = open("never").await;
    let mut client = Client::connect(port).await;
    let created = client
        .call("persona.create", json!({ "draft": { "name": "Ada" } }))
        .await;
    let persona_id = created["result"]["id"].as_str().unwrap().to_string();
    let tools = client
        .call("teammate.tools", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(tools["ok"], true, "{tools}");
    let fields = tools.as_object().expect("an answer is an object");
    assert!(
        fields.contains_key("result"),
        "teammate.tools with no ledger omitted result: {tools}"
    );
    assert_eq!(fields.get("result"), Some(&Value::Null));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_non_string_env_value_is_rejected_before_settings_are_written() {
    let (root, port) = open("env-not-string").await;
    let mut client = Client::connect(port).await;
    keyed(&mut client).await;

    let rejected = client
        .call(
            "settings.update",
            json!({ "patch": { "mcpServers": [{
                "id": "needs-token",
                "type": "stdio",
                "name": "Needs token",
                "command": echo_command(),
                "args": [],
                "env": { "API_TOKEN": 1 },
            }] } }),
        )
        .await;
    assert_eq!(rejected["ok"], false, "{rejected}");
    assert!(
        rejected["error"]
            .as_str()
            .unwrap()
            .contains("environment values must be strings")
    );
    assert!(
        !std::fs::read_to_string(root.join("room.jsonl"))
            .unwrap()
            .contains("API_TOKEN")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stdio_child_dropped_after_the_first_call_says_so_once() {
    let connected = mcp::connect(
        "mcp-gone-stdio",
        &[McpServer {
            id: "echo".into(),
            name: "Echo".into(),
            transport: McpTransport::Stdio {
                command: echo_command(),
                args: Vec::new(),
                env: Default::default(),
            },
            refuse: None,
        }],
    )
    .await;
    assert_eq!(connected.failed.len(), 0, "{:?}", connected.failed);
    let tool = connected
        .tools
        .iter()
        .find(|tool| tool.name == "echo__shout")
        .expect("connect listed shout")
        .clone();
    let shouted = tool
        .call(json!({ "text": "harbour" }))
        .await
        .expect("the first call reached the server");
    assert_eq!(shouted.text, "HARBOUR");

    drop(connected);

    let err = tool
        .call(json!({ "text": "harbour" }))
        .await
        .expect_err("the dropped child cannot answer");
    match err {
        mcp::CallError::Transport {
            notice: Some(text), ..
        } => {
            assert!(text.starts_with("The Echo MCP server went away:"), "{text}");
            assert!(
                text.contains("Its tools are gone until the teammate restarts."),
                "{text}"
            );
        }
        other => panic!("a dead stdio child is a transport error, not {other:?}"),
    }

    let err = tool
        .call(json!({ "text": "harbour" }))
        .await
        .expect_err("still gone");
    assert!(
        matches!(err, mcp::CallError::Transport { notice: None, .. }),
        "the notice lands once: {err:?}"
    );
}

/// A stdio server's own children go when the connection does.
///
/// The command in an `mcpServers` entry is usually a launcher — `npx -y
/// some-server`, `uvx …` — and the server everybody means is its child.
/// Killing only the process Toad spawned reparents that child to pid 1, where
/// it keeps whatever it was holding for as long as Toad runs. Here a
/// backgrounded sleep stands in for it.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_stdio_servers_own_children_die_with_the_connection() {
    let root = scratch("orphan");
    let pidfile = root.join("child.pid");
    let wrapper = McpServer {
        id: "wrapper".to_string(),
        name: "Wrapper".to_string(),
        transport: McpTransport::Stdio {
            command: "sh".to_string(),
            args: vec![
                "-c".to_string(),
                format!(
                    "sleep 600 & echo $! > {}; exec {}",
                    pidfile.display(),
                    echo_command()
                ),
            ],
            env: std::collections::HashMap::new(),
        },
        refuse: None,
    };

    let connected = mcp::connect("orphan", std::slice::from_ref(&wrapper)).await;
    assert!(
        connected.failed.is_empty(),
        "{:?}",
        connected
            .failed
            .iter()
            .map(|server| &server.reason)
            .collect::<Vec<_>>()
    );
    assert_eq!(connected.tools.len(), 1);
    let child = std::fs::read_to_string(&pidfile)
        .expect("the wrapper wrote down what it started")
        .trim()
        .to_string();
    assert!(alive(&child), "the wrapper's own child is running");

    drop(connected);
    for _ in 0..300 {
        if !alive(&child) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("the server's own child outlived the connection that started it");
}

/// Whether that process is still there. Signal 0 asks and does nothing else.
#[cfg(unix)]
fn alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid])
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
