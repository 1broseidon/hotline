//! The scheduler over the wire: a job is created, listed, silenced, and
//! cancelled, and the room stream is what remembers it.

mod common;

use futures_util::{SinkExt, StreamExt};
use hotline_core::log::{Log, StreamId};
use hotline_core::wire::Door;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

const TOKEN: &str = "a-token-only-this-harness-knows";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hotline-harness-schedule-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

async fn open(name: &str) -> u16 {
    let root = scratch(name);
    let desk = common::open_desk(&root).unwrap();
    let door = Door::bind(desk.log.clone(), TOKEN.to_string(), Arc::new(desk)).unwrap();
    let port = door.port();
    tokio::spawn(door.run());
    port
}

struct Client {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    next_id: i64,
}

impl Client {
    async fn connect(port: u16) -> Client {
        let (socket, _) = connect_async(format!("ws://127.0.0.1:{port}/ws?token={TOKEN}"))
            .await
            .unwrap();
        Client { socket, next_id: 1 }
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
            match self.socket.next().await.expect("the door closed") {
                Ok(Message::Text(text)) => {
                    let frame: Value = serde_json::from_str(&text).unwrap();
                    if frame.get("id").and_then(Value::as_i64) == Some(id) {
                        return frame;
                    }
                }
                Ok(_) => continue,
                Err(error) => panic!("the socket failed: {error}"),
            }
        }
    }
}

#[tokio::test]
async fn a_job_is_created_listed_silenced_and_cancelled_over_the_wire() {
    let port = open("commands").await;
    let mut client = Client::connect(port).await;

    let created = client
        .call(
            "persona.create",
            json!({ "draft": { "name": "Ada", "goal": "Keep the harbour." } }),
        )
        .await;
    assert_eq!(created["ok"], true, "{created}");
    assert_eq!(created["result"]["backgroundWork"], false);
    let persona_id = created["result"]["id"].as_str().unwrap();

    let when = chrono::Utc::now().timestamp_millis() + 3_600_000;
    let job = client
        .call(
            "schedule.create",
            json!({
                "personaId": persona_id,
                "kind": "schedule",
                "when": when,
                "prompt": "check the crane",
                "quiet": true,
            }),
        )
        .await;
    assert_eq!(job["ok"], true, "{job}");
    let id = job["result"]["id"].as_str().unwrap().to_string();
    assert_eq!(job["result"]["kind"], "schedule");
    assert_eq!(job["result"]["prompt"], "check the crane");
    assert_eq!(job["result"]["quiet"], true);
    assert_eq!(job["result"]["when"], when);
    assert_eq!(job["result"]["nextAt"], when);
    assert_eq!(job["result"]["operatorCreated"], true);

    let listed = client.call("schedule.list", json!({})).await;
    assert_eq!(listed["ok"], true, "{listed}");
    assert_eq!(listed["result"].as_array().unwrap().len(), 1);
    assert_eq!(listed["result"][0]["id"], id);
    assert_eq!(listed["result"][0]["operatorCreated"], true);

    let loud = client
        .call("schedule.set_quiet", json!({ "id": id, "quiet": false }))
        .await;
    assert_eq!(loud["ok"], true, "{loud}");
    let listed = client.call("schedule.list", json!({})).await;
    assert!(listed["result"][0].get("quiet").is_none(), "{listed}");

    let cancelled = client.call("schedule.cancel", json!({ "id": id })).await;
    assert_eq!(cancelled["ok"], true, "{cancelled}");
    let listed = client.call("schedule.list", json!({})).await;
    assert_eq!(listed["result"], json!([]));
}

#[tokio::test]
async fn a_loop_over_the_wire_carries_every_and_has_no_when() {
    let port = open("loop").await;
    let mut client = Client::connect(port).await;

    let created = client
        .call(
            "persona.create",
            json!({ "draft": { "name": "Ada", "goal": "Keep the harbour." } }),
        )
        .await;
    let persona_id = created["result"]["id"].as_str().unwrap();

    let job = client
        .call(
            "schedule.create",
            json!({
                "personaId": persona_id,
                "kind": "loop",
                "every": 15_000,
                "prompt": "sweep the inbox",
            }),
        )
        .await;
    assert_eq!(job["ok"], true, "{job}");
    assert_eq!(job["result"]["kind"], "loop");
    assert_eq!(job["result"]["every"], 15_000);
    assert_eq!(job["result"]["operatorCreated"], true);
    assert!(job["result"].get("when").is_none(), "{job}");
    assert!(job["result"].get("quiet").is_none(), "{job}");
}

#[tokio::test]
async fn an_old_job_on_the_wire_requires_the_background_grant() {
    let root = scratch("legacy-provenance");
    let log = Log::open(&root);
    let next_at = chrono::Utc::now().timestamp_millis() + 3_600_000;
    // This is the shape written by the previous build: there is no trusted
    // creator bit, so loading it must choose the grant-required path.
    log.append(
        &StreamId::Room,
        &json!({
            "kind": "schedule",
            "id": "legacy-job",
            "personaId": "legacy-teammate",
            "when": next_at,
            "prompt": "check the old crane",
            "nextAt": next_at,
            "createdAt": next_at - 1,
        }),
    )
    .unwrap();
    let desk = common::open_desk(&root).unwrap();
    let door = Door::bind(desk.log.clone(), TOKEN.to_string(), Arc::new(desk)).unwrap();
    let port = door.port();
    tokio::spawn(door.run());

    let mut client = Client::connect(port).await;
    let listed = client.call("schedule.list", json!({})).await;
    assert_eq!(listed["ok"], true, "{listed}");
    assert_eq!(listed["result"].as_array().unwrap().len(), 1);
    assert_eq!(listed["result"][0]["id"], "legacy-job");
    assert_eq!(listed["result"][0]["operatorCreated"], false);
}
