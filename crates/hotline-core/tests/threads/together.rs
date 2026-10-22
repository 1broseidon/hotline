//! The agent thread layer together (ADR 0008), on one desk with a teammate
//! on each driver: every thread reaches the chat through the one door, each
//! post stamped with where it came from, and what may not post is refused;
//! a job runs on a thread of its own, from its running note; a subagent is a
//! clone that reaches back into its home thread; and what came back to a job
//! just before a restart is handed over after it.
//!
//! Ada is on Hotline Agent, whose disposable model plays the line it is
//! handed: each `CALL <tool> <json>` is one call to one of Hotline's tools,
//! made in order, and the turn's reply is `RESULTS` with what each came back
//! with, or what follows `SAY`. The first line with `STALL` holds its first
//! answer back for [`STALL`]. Bob is on ACP, the fixture child playing the same
//! `CALL` lines over the desk's MCP server.

use crate::common::{self, Client, acp_fixture, until};
use hotline_core::log::{Log, StreamId};
use hotline_core::wire::Door;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;

const TOKEN: &str = "a-token-only-the-together-harness-knows";
/// How long a `STALL` line holds the model: longer than two of a stalling
/// loop's intervals, so the firings due meanwhile are skipped.
const STALL: Duration = Duration::from_secs(3);
/// The stalling loop's interval, which the room stream allows below the
/// tools' least because the harness writes it before the core starts.
const EVERY: i64 = 1_200;

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
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

fn said(words: &str) -> String {
    streamed(vec![
        chunk(json!({"role":"assistant","content":words}), None),
        chunk(json!({}), Some("stop")),
    ])
}

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

/// Ada's model: see the module. A subagent's report or the person's answer
/// coming back is Hotline's data, not a script, so it is never run again.
async fn model(request: Value, heard: Arc<Mutex<Vec<String>>>) -> String {
    let messages = request["messages"].as_array().cloned().unwrap_or_default();
    let Some(asked_at) = messages
        .iter()
        .rposition(|message| message["role"] == "user")
    else {
        return said("On it.");
    };
    let asked = text_of(&messages[asked_at]);
    let answered: Vec<String> = messages[asked_at + 1..]
        .iter()
        .filter(|message| message["role"] == "tool")
        .map(text_of)
        .collect();
    if answered.is_empty() {
        let stalls = {
            let mut heard = heard.lock().unwrap();
            let first = !heard.iter().any(|line| line.contains("STALL"));
            heard.push(asked.clone());
            first && asked.contains("STALL")
        };
        if stalls {
            tokio::time::sleep(STALL).await;
        }
    }
    if asked.contains("Hotline execution data") || asked.contains("answered your request") {
        return said("Noted.");
    }
    let calls: Vec<(String, String)> = asked
        .lines()
        .filter_map(|line| {
            let rest = &line[line.find("CALL ")? + "CALL ".len()..];
            let (tool, arguments) = rest.split_once(' ')?;
            Some((tool.to_string(), arguments.to_string()))
        })
        .collect();
    if let Some((tool, arguments)) = calls.get(answered.len()) {
        let call = json!({"index":0,"id":format!("call_{}", answered.len()),"type":"function","function":{
            "name":tool,"arguments":arguments
        }});
        return streamed(vec![
            chunk(json!({"role":"assistant","tool_calls":[call]}), None),
            chunk(json!({}), Some("tool_calls")),
        ]);
    }
    let say = asked
        .lines()
        .find_map(|line| line.find("SAY ").map(|at| line[at + 4..].trim()));
    let words = match say {
        Some(say) => say.to_string(),
        None if !calls.is_empty() => {
            let mut lines = vec!["RESULTS".to_string()];
            lines.extend(
                calls
                    .iter()
                    .zip(&answered)
                    .map(|((tool, _), result)| format!("{tool}: {result}")),
            );
            lines.join("\n")
        }
        None => "No change.".to_string(),
    };
    said(&words)
}

/// The model endpoint, and every line it has been handed.
async fn endpoint() -> (String, Arc<Mutex<Vec<String>>>) {
    let heard: Arc<Mutex<Vec<String>>> = Arc::default();
    let kept = heard.clone();
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(move |body: axum::body::Bytes| {
            let heard = kept.clone();
            async move {
                let request: Value = serde_json::from_slice(&body).unwrap_or_default();
                (
                    [("Content-Type", "text/event-stream")],
                    model(request, heard).await,
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, heard)
}

struct Desk {
    client: Client,
    log: Log,
    workspace: String,
    heard: Arc<Mutex<Vec<String>>>,
    _roots: Vec<TempDir>,
}

/// A desk on `root` behind a door, with the fixture in its catalogue.
async fn door(root: &Path) -> (Client, Log) {
    acp_fixture::install(root);
    let desk = common::open_desk(root).unwrap();
    let log = desk.log.clone();
    let parts = desk.parts();
    let door = Door::bind(desk.log.clone(), TOKEN.to_string(), Arc::new(desk), parts).unwrap();
    let port = door.port();
    tokio::spawn(door.run());
    (Client::connect(port, TOKEN).await, log)
}

async fn open() -> Desk {
    let (base, heard) = endpoint().await;
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let (mut client, log) = door(root.path()).await;
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
        heard,
        _roots: vec![root],
    }
}

impl Desk {
    /// A running teammate: Ada on Hotline Agent with Whole machine reach, so
    /// she asks a colleague without a first-contact card, or Bob on ACP.
    async fn teammate(&mut self, name: &str, acp: bool) -> String {
        let mut draft = json!({"name":name,"goal":"Keep the harbour.","cwd":self.workspace});
        if acp {
            draft["backendId"] = json!(acp_fixture::BACKEND);
        } else {
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
        id
    }

    async fn ok(&mut self, cmd: &str, params: Value) -> Value {
        let answered = self.client.call(cmd, params).await;
        assert_eq!(answered["ok"], true, "{cmd}: {answered}");
        answered["result"].clone()
    }

    /// The first `RESULTS` a stream holds after its first `skip`, once it does.
    async fn results(&self, stream: StreamId, skip: usize) -> String {
        let found = || {
            self.log
                .load(&stream)
                .into_iter()
                .filter(|event| matches!(event["kind"].as_str(), Some("agent" | "thought")))
                .filter_map(|event| event["text"].as_str().map(str::to_string))
                .filter(|text| text.starts_with("RESULTS"))
                .nth(skip)
        };
        until(&format!("{stream:?} reports"), || found().is_some()).await;
        found().unwrap()
    }

    /// The teammate's message in its chat with these words, once it is there.
    async fn posted(&self, persona_id: &str, text: &str) -> Value {
        let found = || {
            self.chat(persona_id)
                .into_iter()
                .find(|event| event["text"] == text)
        };
        until(&format!("{text:?} is in the chat"), || found().is_some()).await;
        found().unwrap()
    }

    fn chat(&self, persona_id: &str) -> Vec<Value> {
        self.log
            .load(&StreamId::Tape(persona_id.to_string()))
            .into_iter()
            .filter(|event| event["kind"] == "agent")
            .collect()
    }

    fn heard(&self, wanted: &str) -> Vec<String> {
        self.heard
            .lock()
            .unwrap()
            .iter()
            .filter(|line| line.contains(wanted))
            .cloned()
            .collect()
    }

    fn job_thread(&self, job_id: &str) -> Vec<Value> {
        self.log.load(&StreamId::Job(job_id.to_string()))
    }
}

/// One teammate on each driver, and every thread they post from: the DM may
/// not, being the chat already, on either driver; a quiet job on ACP posts
/// once, stamped with its job; a work thread's file and post land stamped
/// from the thread; a pair posts to its target's chat, stamped from the
/// pair; and a subagent reaches back into the DM with `search_thread` and is
/// refused the chat.
#[tokio::test(flavor = "multi_thread")]
async fn every_thread_reaches_the_chat_through_one_door_on_both_drivers() {
    let mut desk = open().await;
    std::fs::write(
        Path::new(&desk.workspace).join("winch.txt"),
        "cable snapped\n",
    )
    .unwrap();
    let ada = desk.teammate("Ada", false).await;
    let bob = desk.teammate("Bob", true).await;

    // A loud turn on either driver is refused, and what Ada said there is
    // what her subagent will find.
    for persona_id in [&ada, &bob] {
        desk.ok(
            "session.prompt",
            json!({"personaId": persona_id, "text": r#"CALL post_to_chat {"text":"Bertha is the blue crane."}"#}),
        )
        .await;
        let loud = desk.results(StreamId::Tape(persona_id.clone()), 0).await;
        assert!(loud.contains("already the chat"), "{loud}");
    }

    // A quiet job on ACP posts once, from its own thread.
    let job = desk
        .ok(
            "schedule.create",
            json!({
                "personaId": bob, "kind": "schedule", "when": now() + 1_500, "quiet": true,
                "prompt": r#"CALL post_to_chat {"text":"The winch is fixed."}"#,
            }),
        )
        .await;
    let job_id = job["id"].as_str().unwrap();
    let from_job = desk.posted(&bob, "The winch is fixed.").await;
    assert_eq!(from_job["from"]["thread"], job_id);
    assert_eq!(from_job["from"]["kind"], "job");
    assert_eq!(from_job["scheduled"]["jobId"], job_id);

    // A work thread's file and its one post land in Ada's chat from it.
    let text = [
        r#"CALL send_file {"source":"workspace","path":"winch.txt","caption":"The winch log."}"#,
        r#"CALL post_to_chat {"text":"The winch log is in the chat."}"#,
    ]
    .join("\n");
    let opened = desk
        .ok("thread.open", json!({"personaId": ada, "text": text}))
        .await;
    let side = opened["thread"]["key"].as_str().unwrap().to_owned();
    let stamp = json!({"thread": side, "kind": "side", "title":opened["title"]});
    let file = desk.posted(&ada, "The winch log.").await;
    assert_eq!(file["attachments"][0]["name"], "winch.txt", "{file}");
    assert_eq!(file["from"], stamp);
    let post = desk.posted(&ada, "The winch log is in the chat.").await;
    assert_eq!(post["from"], stamp);

    // A pair posts to its target's chat, never the asker's.
    let ask = json!({
        "to": "Bob", "intent": "ask",
        "message": r#"CALL post_to_chat {"text":"Ada asked me about the crane."}"#,
    });
    desk.ok(
        "session.prompt",
        json!({"personaId": ada, "text": format!("CALL message_teammate {ask}")}),
    )
    .await;
    let from_pair = desk.posted(&bob, "Ada asked me about the crane.").await;
    assert_eq!(from_pair["from"]["kind"], "pair", "{from_pair}");
    assert!(
        desk.chat(&ada)
            .iter()
            .all(|event| event["text"] != "Ada asked me about the crane.")
    );

    // A subagent reaches back into the DM under its lease, and may not post.
    let task = [
        r#"CALL search_thread {"query":"Bertha"}"#,
        r#"CALL post_to_chat {"text":"From a clone."}"#,
    ]
    .join("\n");
    let subagent = json!({"task": task, "title": "Reach back"});
    desk.ok(
        "session.prompt",
        json!({"personaId": ada, "text": format!("CALL subagent {subagent}")}),
    )
    .await;
    let run = || {
        desk.log
            .load(&StreamId::Tape(ada.clone()))
            .into_iter()
            .find(|event| event["kind"] == "link" && event["title"] == "Reach back")
            .and_then(|link| link["thread"].as_str().map(str::to_string))
    };
    until("the subagent started", || run().is_some()).await;
    let reached = desk.results(StreamId::Run(run().unwrap()), 0).await;
    assert!(reached.contains("Bertha is the blue crane."), "{reached}");
    assert!(!reached.contains("Posted to the chat."), "{reached}");
    assert!(
        desk.chat(&ada)
            .iter()
            .all(|event| event["text"] != "From a clone."),
        "a run posts nothing"
    );
}

/// A recurring job and what came back to jobs, across a restart. The first
/// desk makes Ada; the second starts on its data with three jobs on the room
/// stream: a stalling loop, a job waiting for later whose thread holds the
/// person's answer to its card, written and not yet heard, and an ended job
/// whose thread holds the same.
///
/// After the start, the living job hears its answer and the ended job's goes
/// to the DM. The loop fires on its own thread, skips the firings due while
/// one runs and says what interval would fit, starts its next firing from its
/// note, and ends on the person's cancel with a final post.
#[tokio::test(flavor = "multi_thread")]
async fn a_recurring_job_and_what_came_back_to_jobs_carry_across_a_restart() {
    let mut first = open().await;
    let ada = first.teammate("Ada", false).await;

    let second = tempfile::tempdir().unwrap();
    let mut room = std::fs::read_to_string(first.log.root().join("room.jsonl")).unwrap();
    let job = |id: &str, extra: Value| {
        let mut job = json!({
            "kind": "schedule", "id": id, "personaId": ada, "prompt": "SAY Watching.",
            "operatorCreated": true, "createdAt": now(),
        });
        for (key, value) in extra.as_object().cloned().unwrap() {
            job[key] = value;
        }
        job
    };
    let later = now() + 3_600_000;
    for written in [
        job(
            "crane-watch",
            json!({"every": EVERY, "nextAt": now() + 500, "quiet": true,
                   "prompt": "STALL SAY The crane is steady."}),
        ),
        job("waiting", json!({"when": later, "nextAt": later})),
        job(
            "ended",
            json!({"when": now() - 60_000, "nextAt": now() - 60_000, "endedAt": now() - 30_000}),
        ),
    ] {
        room.push_str(&written.to_string());
        room.push('\n');
    }
    std::fs::write(second.path().join("room.jsonl"), room).unwrap();
    let jobs = second.path().join("jobs");
    std::fs::create_dir_all(&jobs).unwrap();
    for (job_id, note) in [
        ("waiting", "Use the blue crane."),
        ("ended", "Too late now."),
    ] {
        let delivery = json!({
            "kind": "delivery", "id": format!("human-answer:{job_id}-card"), "ts": now(),
            "from": {"thread": job_id, "kind": "job", "request": format!("{job_id}-card")},
            "cause": {"kind": "answer", "actionId": format!("{job_id}-card"),
                      "status": "done", "about": "Which crane?"},
            "text": note, "receipt": "sent",
        });
        std::fs::write(
            jobs.join(format!("{job_id}.jsonl")),
            format!("{delivery}\n"),
        )
        .unwrap();
    }
    let (client, log) = door(second.path()).await;
    let mut desk = Desk {
        client,
        log,
        workspace: first.workspace.clone(),
        heard: first.heard.clone(),
        _roots: vec![second],
    };

    // The living job hears its answer, on its own thread; the ended one's is
    // the DM's.
    until("the waiting job heard its answer", || {
        !desk.heard("Use the blue crane.").is_empty()
            && desk.job_thread("waiting").iter().any(|event| {
                event["id"] == "human-answer:waiting-card" && event["receipt"] == "read"
            })
    })
    .await;
    until("the ended job's answer is the DM's", || {
        desk.log
            .load(&StreamId::Tape(ada.clone()))
            .iter()
            .any(|event| event["id"] == "human-answer:ended-card" && event["kind"] == "delivery")
    })
    .await;
    assert_eq!(desk.heard("Use the blue crane.").len(), 1, "heard once");

    // The loop's firings run on its thread; those due while one runs are
    // skipped, and the next starts from where the last one stood.
    let skips = |desk: &Desk| -> Vec<String> {
        desk.job_thread("crane-watch")
            .iter()
            .filter(|event| event["kind"] == "notice")
            .filter_map(|event| event["text"].as_str().map(str::to_string))
            .filter(|text| text.starts_with("Skipped the"))
            .collect()
    };
    until("a firing was skipped", || !skips(&desk).is_empty()).await;
    assert!(skips(&desk)[0].contains("would fit"), "{:?}", skips(&desk));
    until("the loop fired again, from its note", || {
        desk.heard("The crane is steady.")
            .iter()
            .any(|line| line.contains("Where this job stood after its last turn"))
    })
    .await;
    let tape = desk.log.load(&StreamId::Tape(ada.clone()));
    assert!(
        tape.iter().all(|event| !(event["kind"] == "user"
            && event["text"]
                .as_str()
                .unwrap_or_default()
                .contains("crane is steady"))),
        "no firing is a turn of the DM"
    );

    desk.ok("schedule.cancel", json!({"id": "crane-watch"}))
        .await;
    let last = desk
        .posted(&ada, "Cancelled. Where it stood: The crane is steady.")
        .await;
    assert_eq!(last["from"]["thread"], "crane-watch");
    assert_eq!(last["from"]["kind"], "job");
    drop(first);
}
