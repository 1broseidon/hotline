//! The scheduler over the wire: a job is created, listed, silenced, and
//! cancelled, and the room stream is what remembers it; a quiet run is heard
//! only by what it posts.

mod common;

use common::Client;
use hotline_core::log::{Log, StreamId};
use hotline_core::wire::Door;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const TOKEN: &str = "a-token-only-this-harness-knows";
const PATIENCE: Duration = Duration::from_secs(20);

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
    let parts = desk.parts();
    let door = Door::bind(desk.log.clone(), TOKEN.to_string(), Arc::new(desk), parts).unwrap();
    let port = door.port();
    tokio::spawn(door.run());
    port
}

#[tokio::test]
async fn a_job_is_created_listed_silenced_and_cancelled_over_the_wire() {
    let port = open("commands").await;
    let mut client = Client::connect(port, TOKEN).await;

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
    let mut client = Client::connect(port, TOKEN).await;

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
    let parts = desk.parts();
    let door = Door::bind(desk.log.clone(), TOKEN.to_string(), Arc::new(desk), parts).unwrap();
    let port = door.port();
    tokio::spawn(door.run());

    let mut client = Client::connect(port, TOKEN).await;
    let listed = client.call("schedule.list", json!({})).await;
    assert_eq!(listed["ok"], true, "{listed}");
    assert_eq!(listed["result"].as_array().unwrap().len(), 1);
    assert_eq!(listed["result"][0]["id"], "legacy-job");
    assert_eq!(listed["result"][0]["operatorCreated"], false);
}

/// One streamed answer from the model endpoint below.
fn streamed(chunks: Vec<Value>) -> String {
    let body: String = chunks
        .into_iter()
        .map(|chunk| format!("data: {chunk}\n\n"))
        .collect();
    format!("{body}data: [DONE]\n\n")
}

fn chunk(delta: Value, finish: Option<&str>) -> Value {
    json!({"id":"chat_test","object":"chat.completion.chunk","created":1,"model":"vendor/coder","choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
}

/// A model that finds nothing on the crane, unless it is asked to watch for
/// trouble, when it hands the person one note with `tell_person`.
async fn crane_model(request: Value) -> String {
    let messages = request["messages"].as_array().cloned().unwrap_or_default();
    let answered = messages
        .last()
        .is_some_and(|message| message["role"] == "tool");
    let asked = messages
        .iter()
        .rev()
        .find(|message| message["role"] == "user")
        .map(|message| message["content"].to_string())
        .unwrap_or_default();
    if !answered && asked.contains("watch for trouble") {
        let call = json!({"index":0,"id":"call_tell","type":"function","function":{
            "name":"tell_person","arguments":json!({"message":"The crane is down: the winch cable snapped."}).to_string()
        }});
        return streamed(vec![
            chunk(json!({"role":"assistant","tool_calls":[call]}), None),
            chunk(json!({}), Some("tool_calls")),
        ]);
    }
    let words = if answered { "Told them." } else { "No change." };
    streamed(vec![
        chunk(json!({"role":"assistant","content":words}), None),
        chunk(json!({}), Some("stop")),
    ])
}

/// A quiet run is heard only by posting, and what it posts is its note as it
/// wrote it: the teammate's message in the chat, stamped with its job, read
/// off the wire. A quiet run that finds nothing puts nothing in the chat.
#[tokio::test(flavor = "multi_thread")]
async fn a_quiet_run_is_heard_only_when_it_posts() {
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(|body: axum::body::Bytes| async move {
            let request: Value = serde_json::from_slice(&body).unwrap_or_default();
            (
                [("Content-Type", "text/event-stream")],
                crane_model(request).await,
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let root = scratch("quiet-posts");
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let desk = common::open_desk(&root).unwrap();
    let parts = desk.parts();
    let door = Door::bind(desk.log.clone(), TOKEN.to_string(), Arc::new(desk), parts).unwrap();
    let port = door.port();
    tokio::spawn(door.run());

    let mut client = Client::connect(port, TOKEN).await;
    let saved = client
        .call(
            "credential.custom_save",
            json!({"draft": {
                "name":"chat","baseUrl":base,"api":"chat_completions","models":["vendor/coder"]
            }}),
        )
        .await;
    assert_eq!(saved["ok"], true, "{saved}");
    let created = client
        .call(
            "persona.create",
            json!({"draft": {
                "name":"Ada","goal":"Keep the harbour.","cwd":workspace.to_string_lossy()
            }}),
        )
        .await;
    let persona_id = created["result"]["id"].as_str().unwrap().to_owned();
    let tape = client.subscribe(json!({"tape": persona_id})).await;
    let quiet_job = |prompt: &str| {
        json!({
            "personaId": persona_id,
            "kind": "schedule",
            "when": chrono::Utc::now().timestamp_millis() + 1_500,
            "prompt": prompt,
            "quiet": true,
        })
    };

    // The first run finds nothing: its words are thinking, and its turn ends.
    let job = client
        .call("schedule.create", quiet_job("check the crane"))
        .await;
    assert_eq!(job["ok"], true, "{job}");
    client
        .next_where(PATIENCE, |frame| {
            frame["sub"] == tape && frame["event"]["kind"] == "turn"
        })
        .await;

    // The second finds trouble and posts it.
    let job = client
        .call(
            "schedule.create",
            quiet_job("check the crane and watch for trouble"),
        )
        .await;
    assert_eq!(job["ok"], true, "{job}");
    let job_id = job["result"]["id"].as_str().unwrap().to_owned();
    let posted = client
        .next_where(PATIENCE, |frame| {
            frame["sub"] == tape && frame["event"]["kind"] == "agent"
        })
        .await;
    let posted = &posted["event"];
    assert_eq!(
        posted["text"], "The crane is down: the winch cable snapped.",
        "the note is posted as the teammate wrote it"
    );
    assert_eq!(posted["scheduled"]["jobId"], job_id.as_str());
    assert_eq!(posted["scheduled"]["quiet"], true);
    assert!(posted.get("from").is_none(), "{posted}");

    // Nothing either run said otherwise reached the chat: the first run's
    // answer and the second's are thinking, and only the note is a message.
    let seen: Vec<&Value> = client
        .inbox
        .iter()
        .filter(|frame| frame["sub"] == tape)
        .map(|frame| &frame["event"])
        .collect();
    assert!(
        seen.iter().all(|event| event["kind"] != "agent"),
        "{seen:?}"
    );
    let thoughts: Vec<&str> = seen
        .iter()
        .filter(|event| event["kind"] == "thought")
        .filter_map(|event| event["text"].as_str())
        .collect();
    assert!(thoughts.contains(&"No change."), "{seen:?}");
}
