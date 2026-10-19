//! `post_to_chat` through the real core, on both drivers, read off the wire,
//! and the files `send_file` posts the same way.
//!
//! Each teammate does what its line says: a line `CALL <tool> <json>` is one
//! call to one of Hotline's own tools, made in order, and the turn's reply is
//! `RESULTS` with one line per call saying what came back. On Hotline Agent a
//! disposable model endpoint plays that script through the in-process tools;
//! on ACP the fixture child (`hotline-acp-fixture`) plays it over the MCP
//! server named in its `session/new`. So the same lines prove the same gate
//! through both of the handler's doors: what may post, where it lands, what
//! is refused, and that a refusal posts nothing.

use crate::common::{Client, acp_fixture};
use hotline_core::log::{Log, StreamId};
use hotline_core::wire::Door;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

const TOKEN: &str = "a-token-only-this-harness-knows";
const PATIENCE: Duration = Duration::from_secs(60);

#[derive(Clone, Copy)]
enum Driver {
    HotlineAgent,
    Acp,
}

/// The text of a chat-completions message, whether it came as a string or as
/// parts.
fn text_of(message: &Value) -> String {
    match &message["content"] {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The calls a line asks for, in order: the fixture child's reading.
fn calls(said: &str) -> Vec<(String, String)> {
    said.lines()
        .filter_map(|line| {
            let rest = &line[line.find("CALL ")? + "CALL ".len()..];
            let (tool, arguments) = rest.split_once(' ')?;
            Some((tool.to_string(), arguments.to_string()))
        })
        .collect()
}

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

/// Hotline Agent's model: the next call the person's last line asks for, or
/// once every call has answered, what they said.
fn model(request: &Value) -> String {
    let messages = request["messages"].as_array().cloned().unwrap_or_default();
    let Some(asked_at) = messages
        .iter()
        .rposition(|message| message["role"] == "user")
    else {
        return streamed(vec![chunk(
            json!({"role":"assistant","content":"On it."}),
            Some("stop"),
        )]);
    };
    let asked = calls(&text_of(&messages[asked_at]));
    let answered: Vec<String> = messages[asked_at + 1..]
        .iter()
        .filter(|message| message["role"] == "tool")
        .map(text_of)
        .collect();
    if let Some((tool, arguments)) = asked.get(answered.len()) {
        let call = json!({"index":0,"id":format!("call_{}", answered.len()),"type":"function","function":{
            "name":tool,"arguments":arguments
        }});
        return streamed(vec![
            chunk(json!({"role":"assistant","tool_calls":[call]}), None),
            chunk(json!({}), Some("tool_calls")),
        ]);
    }
    let words = if asked.is_empty() {
        "On it.".to_string()
    } else {
        let mut lines = vec!["RESULTS".to_string()];
        lines.extend(
            asked
                .iter()
                .zip(&answered)
                .map(|((tool, _), result)| format!("{tool}: {result}")),
        );
        lines.join("\n")
    };
    streamed(vec![
        chunk(json!({"role":"assistant","content":words}), None),
        chunk(json!({}), Some("stop")),
    ])
}

/// A desk with the scripted model connected and the fixture in its catalogue.
struct Desk {
    client: Client,
    log: Log,
    workspace: String,
    _root: TempDir,
}

async fn open() -> Desk {
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(|body: axum::body::Bytes| async move {
            let request: Value = serde_json::from_slice(&body).unwrap_or_default();
            ([("Content-Type", "text/event-stream")], model(&request))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let root = tempfile::tempdir().unwrap();
    acp_fixture::install(root.path());
    let workspace = root.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let desk = crate::common::open_desk(root.path()).unwrap();
    let log = desk.log.clone();
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
    Desk {
        client,
        log,
        workspace: workspace.to_string_lossy().into_owned(),
        _root: root,
    }
}

impl Desk {
    /// A running teammate on `driver`, and its tape's subscription.
    async fn teammate(&mut self, name: &str, driver: Driver, machine: bool) -> (String, i64) {
        let mut draft = json!({"name":name,"goal":"Keep the harbour.","cwd":self.workspace});
        if matches!(driver, Driver::Acp) {
            draft["backendId"] = json!(acp_fixture::BACKEND);
        }
        if machine {
            draft["reach"] = json!("machine");
        }
        let created = self
            .client
            .call("persona.create", json!({"draft": draft}))
            .await;
        assert_eq!(created["ok"], true, "{created}");
        let id = created["result"]["id"].as_str().unwrap().to_owned();
        let started = self
            .client
            .call("session.start", json!({"personaId": id}))
            .await;
        assert_eq!(started["ok"], true, "{started}");
        let tape = self.client.subscribe(json!({"tape": id})).await;
        (id, tape)
    }

    /// The next event on subscription `sub` that `wanted` picks, live or in
    /// the snapshot the subscription opened with.
    async fn event(&mut self, sub: i64, wanted: impl Fn(&Value) -> bool) -> Value {
        let frame = self
            .client
            .next_where(PATIENCE, |frame| {
                frame["sub"] == sub
                    && (wanted(&frame["event"])
                        || frame["snapshot"]
                            .as_array()
                            .is_some_and(|events| events.iter().any(&wanted)))
            })
            .await;
        match frame["snapshot"].as_array() {
            Some(events) => events.iter().find(|event| wanted(event)).unwrap().clone(),
            None => frame["event"].clone(),
        }
    }

    /// A turn's `RESULTS`, said or, on a quiet run, thought.
    async fn results(&mut self, sub: i64) -> Vec<String> {
        let said = self
            .event(sub, |event| {
                matches!(event["kind"].as_str(), Some("agent" | "thought"))
                    && event["text"]
                        .as_str()
                        .is_some_and(|text| text.starts_with("RESULTS"))
            })
            .await;
        said["text"]
            .as_str()
            .unwrap()
            .lines()
            .skip(1)
            .map(str::to_owned)
            .collect()
    }

    /// The teammate's messages in its chat, from its tape on disk.
    fn chat(&self, persona_id: &str) -> Vec<Value> {
        self.log
            .load(&StreamId::Tape(persona_id.to_string()))
            .into_iter()
            .filter(|event| event["kind"] == "agent")
            .collect()
    }
}

fn says(lines: &[String], at: usize, sentence: &str) {
    assert!(
        lines.get(at).is_some_and(|line| line.contains(sentence)),
        "call {at} should say {sentence:?}: {lines:#?}"
    );
}

/// The DM's own reply is the chat, so a loud turn may not post; a quiet run,
/// whose replies nobody sees, posts once, to its own chat, stamped with its
/// job, and wakes nobody but the person.
async fn the_dm_posts_only_from_a_quiet_run(driver: Driver) {
    let mut desk = open().await;
    let (ada, tape) = desk.teammate("Ada", driver, false).await;

    let sent = desk
        .client
        .call(
            "session.prompt",
            json!({"personaId": ada, "text": r#"CALL post_to_chat {"text":"From a loud turn."}"#}),
        )
        .await;
    assert_eq!(sent["ok"], true, "{sent}");
    let loud = desk.results(tape).await;
    says(&loud, 0, "already the chat");

    let prompt = [
        r#"CALL post_to_chat {"text":"Into Bob's chat.","to":"bob"}"#,
        r#"CALL post_to_chat {"text":"Wake yourself.","wake":true}"#,
        r#"CALL post_to_chat {"text":"The crane is down: the winch cable snapped."}"#,
        r#"CALL post_to_chat {"text":"And another thing."}"#,
    ]
    .join("\n");
    let job = desk
        .client
        .call(
            "schedule.create",
            json!({
                "personaId": ada,
                "kind": "schedule",
                "when": chrono::Utc::now().timestamp_millis() + 1_500,
                "quiet": true,
                "prompt": prompt,
            }),
        )
        .await;
    assert_eq!(job["ok"], true, "{job}");
    let job_id = job["result"]["id"].as_str().unwrap().to_owned();
    let posted = desk
        .event(tape, |event| {
            event["kind"] == "agent"
                && event["text"] == "The crane is down: the winch cable snapped."
        })
        .await;
    assert_eq!(posted["scheduled"]["jobId"], job_id.as_str());
    assert_eq!(posted["scheduled"]["quiet"], true);
    assert!(posted.get("from").is_none(), "the DM's own turn: {posted}");
    let quiet = desk.results(tape).await;
    says(&quiet, 0, "your own chat only");
    says(&quiet, 1, "nobody to wake");
    says(&quiet, 2, "Posted to the chat.");
    says(&quiet, 3, "one per turn");

    let chat: Vec<Value> = desk
        .chat(&ada)
        .into_iter()
        .map(|event| event["text"].clone())
        .filter(|text| !text.as_str().unwrap_or_default().starts_with("RESULTS"))
        .collect();
    assert_eq!(
        chat,
        [json!("The crane is down: the winch cable snapped.")],
        "only the one post is a message; the refused ones landed nowhere"
    );
}

/// A work thread posts once a turn, stamped as from the thread, and asking to
/// wake hands the post to the main conversation, which answers the person. A
/// second wake in the turn is refused; the next turn may post again, and a
/// post past 2,000 characters is clipped.
async fn a_work_thread_posts_once_a_turn_and_wakes_its_home(driver: Driver) {
    let mut desk = open().await;
    let (ada, tape) = desk.teammate("Ada", driver, false).await;

    let text = [
        r#"CALL post_to_chat {"text":"The build broke under your refactor.","wake":true}"#,
        r#"CALL post_to_chat {"text":"Wake twice.","wake":true}"#,
    ]
    .join("\n");
    let opened = desk
        .client
        .call("thread.open", json!({"personaId": ada, "text": text}))
        .await;
    assert_eq!(opened["ok"], true, "{opened}");
    let thread = opened["result"]["thread"].clone();
    let side = desk.client.subscribe(json!({"threadId": thread})).await;

    let posted = desk
        .event(tape, |event| {
            event["kind"] == "agent" && event["text"] == "The build broke under your refactor."
        })
        .await;
    assert_eq!(posted["from"]["kind"], "side", "{posted}");
    assert!(posted.get("scheduled").is_none(), "{posted}");
    // Woken, the main conversation takes a turn of its own and answers.
    desk.event(tape, |event| {
        event["kind"] == "agent" && event["text"] == "On it."
    })
    .await;
    let first = desk.results(side).await;
    says(&first, 0, "acting on it");
    says(&first, 1, "one per turn");

    let long = "a".repeat(2_500);
    let said = desk
        .client
        .call(
            "thread.prompt",
            json!({"thread": thread, "text": format!(r#"CALL post_to_chat {{"text":"{long}"}}"#)}),
        )
        .await;
    assert_eq!(said["ok"], true, "{said}");
    let clipped = desk
        .event(tape, |event| {
            event["kind"] == "agent"
                && event["text"]
                    .as_str()
                    .is_some_and(|text| text.starts_with("aaaa"))
        })
        .await;
    let text = clipped["text"].as_str().unwrap();
    assert_eq!(text.chars().count(), 2_001, "2,000 and the ellipsis");
    assert!(text.ends_with('…'));
    says(&desk.results(side).await, 0, "Posted to the chat.");
    assert!(
        desk.chat(&ada)
            .iter()
            .all(|event| event["text"] != "Wake twice."),
        "the refused second post landed nowhere"
    );
}

/// A pair posts to its own chat, the target's, never the asker's.
async fn a_pair_posts_to_its_targets_own_chat(driver: Driver) {
    let mut desk = open().await;
    // A Whole machine Hotline Agent asks without a first-contact card.
    let (ada, ada_tape) = desk.teammate("Ada", Driver::HotlineAgent, true).await;
    let (bob, bob_tape) = desk.teammate("Bob", driver, false).await;

    let ask = json!({
        "to": "Bob",
        "intent": "ask",
        "message": r#"CALL post_to_chat {"text":"Ada asked me about the crane."}"#,
    });
    let sent = desk
        .client
        .call(
            "session.prompt",
            json!({"personaId": ada, "text": format!("CALL message_teammate {ask}")}),
        )
        .await;
    assert_eq!(sent["ok"], true, "{sent}");
    let posted = desk
        .event(bob_tape, |event| {
            event["kind"] == "agent" && event["text"] == "Ada asked me about the crane."
        })
        .await;
    assert_eq!(posted["from"]["kind"], "pair", "{posted}");
    says(&desk.results(ada_tape).await, 0, "requestId");
    assert!(
        desk.chat(&ada)
            .iter()
            .all(|event| event["text"] != "Ada asked me about the crane."),
        "the asker's chat is not the pair's to post to"
    );
    assert_eq!(
        desk.chat(&bob)
            .iter()
            .filter(|event| event["text"] == "Ada asked me about the crane.")
            .count(),
        1
    );
}

/// A file sent from a work thread lands in the chat stamped from the
/// thread, which keeps a line naming it. It is not the turn's post: the
/// thread may still post once.
async fn a_work_threads_file_lands_in_the_chat(driver: Driver) {
    let mut desk = open().await;
    std::fs::write(
        std::path::Path::new(&desk.workspace).join("winch.txt"),
        "cable snapped\n",
    )
    .unwrap();
    let (ada, tape) = desk.teammate("Ada", driver, false).await;

    let text = [
        r#"CALL send_file {"source":"workspace","path":"winch.txt","caption":"The winch log."}"#,
        r#"CALL post_to_chat {"text":"The winch log is in the chat."}"#,
    ]
    .join("\n");
    let opened = desk
        .client
        .call("thread.open", json!({"personaId": ada, "text": text}))
        .await;
    assert_eq!(opened["ok"], true, "{opened}");
    let thread = opened["result"]["thread"].clone();
    let side = desk.client.subscribe(json!({"threadId": thread})).await;

    let posted = desk
        .event(tape, |event| {
            event["kind"] == "agent" && event["text"] == "The winch log."
        })
        .await;
    assert_eq!(posted["attachments"][0]["name"], "winch.txt", "{posted}");
    assert_eq!(
        posted["from"],
        json!({"thread": thread["key"], "kind": "side"}),
        "{posted}"
    );
    desk.event(side, |event| {
        event["kind"] == "notice" && event["text"] == "Sent winch.txt to the chat."
    })
    .await;
    let results = desk.results(side).await;
    says(&results, 0, "Sent winch.txt (14 bytes).");
    says(&results, 1, "Posted to the chat.");
}

/// A quiet run's file is posted, stamped with its job, and is the run's one
/// post: a `post_to_chat` after it in the same turn is refused and lands
/// nowhere.
async fn a_quiet_runs_file_is_its_one_post(driver: Driver) {
    let mut desk = open().await;
    std::fs::write(
        std::path::Path::new(&desk.workspace).join("winch.txt"),
        "cable snapped\n",
    )
    .unwrap();
    let (ada, tape) = desk.teammate("Ada", driver, false).await;

    let prompt = [
        r#"CALL send_file {"source":"workspace","path":"winch.txt","caption":"The winch log."}"#,
        r#"CALL post_to_chat {"text":"And a note besides."}"#,
    ]
    .join("\n");
    let job = desk
        .client
        .call(
            "schedule.create",
            json!({
                "personaId": ada,
                "kind": "schedule",
                "when": chrono::Utc::now().timestamp_millis() + 1_500,
                "quiet": true,
                "prompt": prompt,
            }),
        )
        .await;
    assert_eq!(job["ok"], true, "{job}");
    let job_id = job["result"]["id"].as_str().unwrap().to_owned();
    let posted = desk
        .event(tape, |event| {
            event["kind"] == "agent" && event["text"] == "The winch log."
        })
        .await;
    assert_eq!(posted["attachments"][0]["name"], "winch.txt", "{posted}");
    assert_eq!(posted["scheduled"]["jobId"], job_id.as_str());
    assert!(posted.get("from").is_none(), "the DM's own turn: {posted}");
    let quiet = desk.results(tape).await;
    says(&quiet, 0, "Sent winch.txt (14 bytes).");
    says(&quiet, 1, "one per turn");

    let chat: Vec<Value> = desk
        .chat(&ada)
        .into_iter()
        .map(|event| event["text"].clone())
        .collect();
    assert_eq!(
        chat,
        [json!("The winch log.")],
        "the file is the only message; the note after it landed nowhere"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn hotline_agent_work_threads_file_lands_in_the_chat() {
    a_work_threads_file_lands_in_the_chat(Driver::HotlineAgent).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_acp_work_threads_file_lands_in_the_chat() {
    a_work_threads_file_lands_in_the_chat(Driver::Acp).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn hotline_agent_quiet_runs_file_is_its_one_post() {
    a_quiet_runs_file_is_its_one_post(Driver::HotlineAgent).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_acp_quiet_runs_file_is_its_one_post() {
    a_quiet_runs_file_is_its_one_post(Driver::Acp).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn hotline_agent_posts_from_the_dm_only_on_a_quiet_run() {
    the_dm_posts_only_from_a_quiet_run(Driver::HotlineAgent).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_acp_quiet_run_posts_and_a_loud_turn_may_not() {
    the_dm_posts_only_from_a_quiet_run(Driver::Acp).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn hotline_agent_work_thread_posts_once_a_turn_and_wakes_its_home() {
    a_work_thread_posts_once_a_turn_and_wakes_its_home(Driver::HotlineAgent).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_acp_work_thread_posts_once_a_turn_and_wakes_its_home() {
    a_work_thread_posts_once_a_turn_and_wakes_its_home(Driver::Acp).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hotline_agent_pair_posts_to_its_own_chat() {
    a_pair_posts_to_its_targets_own_chat(Driver::HotlineAgent).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_acp_pair_posts_to_its_own_chat() {
    a_pair_posts_to_its_targets_own_chat(Driver::Acp).await;
}
