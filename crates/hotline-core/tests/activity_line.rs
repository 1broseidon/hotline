//! The words beside a teammate's mark, as the roster row says them over the
//! real wire, from the real core running Hotline Agent against a disposable
//! model endpoint that answers one scripted step at a time.
//!
//! The row is never without words while the teammate works or waits on its
//! helpers: the teammate's own from `activity` first, then what the desk
//! knows of the turn. A stopped turn, or one over with nothing still
//! running, says nothing.
use crate::common;

use axum::{Router, body::Bytes, response::IntoResponse, routing::post};
use common::Client;
use hotline_core::wire::Door;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::{Notify, mpsc};

const TOKEN: &str = "disposable-activity-line-test";

/// One scripted reply of the teammate's model, sent once the test lets it
/// go, so the row can be read while the model has not answered yet.
enum Step {
    Call(&'static str, Value),
    Say(&'static str),
}

/// A desk with one Hotline Agent teammate whose model plays `script`, step
/// by step, each held until `go` is notified. A helper's model holds until
/// `report` is notified and then says what it found.
struct Desk {
    client: Client,
    persona: String,
    roster: i64,
    tape: i64,
    go: Arc<Notify>,
    report: Arc<Notify>,
    asked: mpsc::Receiver<usize>,
    _root: tempfile::TempDir,
    tasks: [tokio::task::JoinHandle<()>; 2],
}

impl Drop for Desk {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

async fn desk(script: Vec<Step>) -> Desk {
    let script = Arc::new(script);
    let go = Arc::new(Notify::new());
    let report = Arc::new(Notify::new());
    let (seen, asked) = mpsc::channel(16);
    let step = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (model_go, model_report) = (go.clone(), report.clone());
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |body: Bytes| {
            let (script, go, report, seen, step) = (
                script.clone(),
                model_go.clone(),
                model_report.clone(),
                seen.clone(),
                step.clone(),
            );
            async move {
                let request: Value = serde_json::from_slice(&body).unwrap();
                if request["stream"] != true {
                    return (
                        axum::http::StatusCode::SERVICE_UNAVAILABLE,
                        "not in this test",
                    )
                        .into_response();
                }
                // The helper is offered no helpers of its own; the teammate is.
                let main = request["tools"].as_array().is_some_and(|tools| {
                    tools
                        .iter()
                        .any(|tool| tool["function"]["name"] == "subagent")
                });
                let chunks = if main {
                    let index = step.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    seen.send(index).await.unwrap();
                    go.notified().await;
                    match &script[index] {
                        Step::Call(name, arguments) => call(name, arguments, index),
                        Step::Say(text) => say(text),
                    }
                } else {
                    report.notified().await;
                    say("Two edge cases fail.")
                };
                let body: String = chunks
                    .into_iter()
                    .map(|chunk| format!("data: {chunk}\n\n"))
                    .collect();
                (
                    [("Content-Type", "text/event-stream")],
                    format!("{body}data: [DONE]\n\n"),
                )
                    .into_response()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let model = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(workspace.join("crane.rs"), "fn lift() {}").unwrap();
    let opened = common::open_desk(root.path()).unwrap();
    let parts = opened.parts();
    let door = Door::bind(opened.log.clone(), TOKEN.into(), Arc::new(opened), parts).unwrap();
    let port = door.port();
    let door_task = tokio::spawn(async move {
        let _ = door.run().await;
    });
    let mut client = Client::connect(port, TOKEN).await;
    let saved = client
        .call(
            "credential.custom_save",
            json!({"draft": {"name":"chat", "baseUrl":base, "api":"chat_completions", "models":["vendor/coder"]}}),
        )
        .await;
    assert_eq!(saved["ok"], true, "{saved}");
    let made = client
        .call(
            "persona.create",
            json!({"draft": {"name":"Lift tester", "goal":"Find the lift", "cwd":workspace.to_string_lossy()}}),
        )
        .await;
    let persona = made["result"]["id"].as_str().unwrap().to_owned();
    let tape = client.subscribe(json!({"tape":persona})).await;
    let roster = client.subscribe(json!({"view":"roster"})).await;
    let started = client
        .call("session.start", json!({"personaId":persona}))
        .await;
    assert_eq!(started["ok"], true, "{started}");
    Desk {
        client,
        persona,
        roster,
        tape,
        go,
        report,
        asked,
        _root: root,
        tasks: [model, door_task],
    }
}

impl Desk {
    async fn prompt(&mut self, text: &str) {
        let sent = self
            .client
            .call(
                "session.prompt",
                json!({"personaId":self.persona, "text":text}),
            )
            .await;
        assert_eq!(sent["ok"], true, "{sent}");
    }

    /// Waits until the model has been asked for step `index`, which it holds.
    async fn asked(&mut self, index: usize) {
        let got = tokio::time::timeout(common::ANSWER, self.asked.recv())
            .await
            .expect("the model was never asked")
            .unwrap();
        assert_eq!(got, index);
    }

    /// The next row of this teammate's that `wanted` picks.
    async fn row(&mut self, what: &str, wanted: impl Fn(&Value) -> bool) -> Value {
        let (roster, persona) = (self.roster, self.persona.clone());
        let row = self
            .client
            .next_where(common::ANSWER, move |frame| {
                frame["sub"] == roster
                    && frame["event"]["persona"]["id"] == persona.as_str()
                    && wanted(&frame["event"])
            })
            .await;
        eprintln!("{what}: {}", row["event"]["activity"]);
        row["event"].clone()
    }

    /// The row that says `words` while the session works.
    async fn working(&mut self, words: &str) -> Value {
        self.row(words, |row| {
            row["activity"] == words
                && row["session"]["state"] == "thinking"
                && row["session"]["awaitingSubagents"] != true
        })
        .await
    }

    /// The row that says `words` while the turn is held for its helpers.
    async fn waiting(&mut self, words: &str) -> Value {
        self.row(words, |row| {
            row["activity"] == words && row["session"]["awaitingSubagents"] == true
        })
        .await
    }

    async fn turn_ends(&mut self) -> Value {
        let tape = self.tape;
        self.client
            .next_where(common::ANSWER, move |frame| {
                frame["sub"] == tape && frame["event"]["kind"] == "turn"
            })
            .await["event"]
            .clone()
    }

    /// The row once the turn is over: ready, with no words.
    async fn at_rest(&mut self) -> Value {
        self.row("at rest", |row| {
            row["session"]["state"] == "ready" && row.get("activity").is_none()
        })
        .await
    }
}

/// Each state of a turn on Hotline Agent, read off the roster row: before
/// the model answers, between steps, with a step in flight and nothing
/// declared, the teammate's own words, and the wait on a helper with and
/// without words of its own; the helper's end lets the words go.
#[tokio::test(flavor = "multi_thread")]
async fn the_row_says_what_the_turn_is_on_until_the_last_helper_ends() {
    let mut desk = desk(vec![
        Step::Call("read", json!({"path":"crane.rs"})),
        Step::Call(
            "subagent",
            json!({"task":"Check the edge cases of the lift", "title":"Edge cases"}),
        ),
        Step::Say("The lift is in crane.rs; a helper is checking the edge cases."),
        Step::Call(
            "activity",
            json!({"type":"searching", "message":"Waiting on the edge cases"}),
        ),
        Step::Say("Noted, the brakes are next."),
        Step::Say("Two edge cases fail; I will fix them."),
    ])
    .await;
    desk.prompt("Where is the lift?").await;

    // Nothing back from the model yet.
    desk.asked(0).await;
    desk.working("Getting started").await;
    desk.go.notify_one();
    // The read is done and the model is choosing what next.
    desk.asked(1).await;
    desk.working("Thinking").await;
    desk.go.notify_one();
    // The helper is a step in flight, and nothing is declared.
    desk.asked(2).await;
    desk.working("Working on it").await;
    desk.go.notify_one();

    // The reply is said and the turn is held for the helper alone: it
    // left no words of its own, so the row counts the helper.
    let row = desk.waiting("1 helper working").await;
    assert_eq!(row["subagents"].as_array().unwrap().len(), 1, "{row}");

    // A line said meanwhile goes into the held turn, which declares what it
    // waits on; its own words win while it works and stand through the wait.
    desk.prompt("also check the brakes").await;
    desk.asked(3).await;
    desk.go.notify_one();
    desk.asked(4).await;
    desk.working("Waiting on the edge cases").await;
    desk.go.notify_one();
    desk.waiting("Waiting on the edge cases").await;

    // The helper reports, the turn answers it and ends: no words are left.
    desk.report.notify_one();
    desk.asked(5).await;
    desk.go.notify_one();
    assert_eq!(desk.turn_ends().await["stopReason"], "end_turn");
    let row = desk.at_rest().await;
    assert!(
        row["subagents"].as_array().is_none_or(Vec::is_empty),
        "{row}"
    );
    desk.client
        .call("session.stop", json!({"personaId":desk.persona}))
        .await;
}

/// The person's Stop ends the turn and the helper with it: the row says
/// nothing, not a helper count.
#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_turn_says_nothing() {
    let mut desk = desk(vec![
        Step::Call(
            "subagent",
            json!({"task":"Check the edge cases of the lift", "title":"Edge cases"}),
        ),
        Step::Say("A helper is checking the edge cases."),
    ])
    .await;
    desk.prompt("Where is the lift?").await;
    desk.asked(0).await;
    desk.go.notify_one();
    desk.asked(1).await;
    desk.working("Working on it").await;
    let cancelled = desk
        .client
        .call("session.cancel", json!({"personaId":desk.persona}))
        .await;
    assert_eq!(cancelled["ok"], true, "{cancelled}");
    desk.turn_ends().await;
    let row = desk.at_rest().await;
    assert!(
        row["subagents"].as_array().is_none_or(Vec::is_empty),
        "{row}"
    );
    desk.client
        .call("session.stop", json!({"personaId":desk.persona}))
        .await;
}

fn say(text: &str) -> Vec<Value> {
    vec![
        json!({"id":"chat_test","object":"chat.completion.chunk","created":1,"model":"vendor/coder","choices":[{"index":0,"delta":{"role":"assistant","content":text},"finish_reason":null}]}),
        json!({"id":"chat_test","object":"chat.completion.chunk","created":1,"model":"vendor/coder","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
    ]
}

fn call(name: &str, arguments: &Value, index: usize) -> Vec<Value> {
    let id = format!("call_{index}");
    vec![
        json!({"id":"chat_test","object":"chat.completion.chunk","created":1,"model":"vendor/coder","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":name,"arguments":arguments.to_string()}}]},"finish_reason":null}]}),
        json!({"id":"chat_test","object":"chat.completion.chunk","created":1,"model":"vendor/coder","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
    ]
}
