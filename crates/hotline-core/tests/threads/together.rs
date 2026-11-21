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
//! answer back for [`STALL`], and every line with `HOLD` holds its own. A line
//! that only woke its teammate for its inbox plays what the test set as
//! [`Desk::on_wake`], and one that offered it again what it set aside plays
//! [`Desk::on_pick_up`]. Bob is on ACP, the fixture child playing the same `CALL`
//! lines over the desk's MCP server.

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

/// What a line that woke a teammate for its inbox says, whichever thread it
/// woke.
const WOKEN: &str = "came into your inbox while you were idle";

/// What a line that offered a teammate again the items it set aside says.
const PICKED_UP: &str = "You are idle now. These items in your inbox are still open";

/// The inbox's gather delay, which a pick-up waits as well.
const GATHER: Duration = Duration::from_secs(4);

/// Ada's model: see the module. A subagent's report or the person's answer
/// coming back is Hotline's data, not a script, so it is never run again.
async fn model(request: Value, heard: Arc<Mutex<Vec<String>>>, scripts: Scripts) -> String {
    let messages = request["messages"].as_array().cloned().unwrap_or_default();
    let Some(asked_at) = messages
        .iter()
        .rposition(|message| message["role"] == "user")
    else {
        return said("On it.");
    };
    let mut asked = common::asked(&text_of(&messages[asked_at]));
    if asked.contains(WOKEN) {
        asked = format!("{asked}\n{}", scripts.on_wake.lock().unwrap());
    }
    if asked.contains(PICKED_UP) {
        asked = format!("{asked}\n{}", scripts.on_pick_up.lock().unwrap());
    }
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
            (first && asked.contains("STALL")) || asked.contains("HOLD")
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

/// What the model plays on a line the inbox started: one that woke a
/// teammate for new items, and one that offered it what it set aside.
#[derive(Clone, Default)]
struct Scripts {
    on_wake: Arc<Mutex<String>>,
    on_pick_up: Arc<Mutex<String>>,
}

/// The model endpoint, every line it has been handed, and what it plays on
/// a line the inbox started.
async fn endpoint() -> (String, Arc<Mutex<Vec<String>>>, Scripts) {
    let heard: Arc<Mutex<Vec<String>>> = Arc::default();
    let scripts = Scripts::default();
    let (kept, played) = (heard.clone(), scripts.clone());
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(move |body: axum::body::Bytes| {
            let (heard, scripts) = (kept.clone(), played.clone());
            async move {
                let request: Value = serde_json::from_slice(&body).unwrap_or_default();
                (
                    [("Content-Type", "text/event-stream")],
                    model(request, heard, scripts).await,
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, heard, scripts)
}

struct Desk {
    client: Client,
    log: Log,
    workspace: String,
    heard: Arc<Mutex<Vec<String>>>,
    scripts: Scripts,
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
    let (base, heard, scripts) = endpoint().await;
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
        scripts,
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

    /// `from` sends with its own `send` tool, as its model would: the item's
    /// id, read off the result of the call.
    async fn send(&mut self, from: &str, arguments: Value) -> String {
        self.ok(
            "session.prompt",
            json!({"personaId": from, "text": format!("CALL send {arguments}")}),
        )
        .await;
        let result = self.results(StreamId::Tape(from.to_string()), 0).await;
        let at = result.find("\"id\":\"").expect("the result names the item") + 6;
        result[at..at + 36].to_string()
    }

    /// One item of `persona_id`'s inbox by id, as the room stream keeps it.
    fn item(&self, persona_id: &str, id: &str) -> Option<Value> {
        self.log
            .load(&StreamId::Inbox(persona_id.to_string()))
            .into_iter()
            .find(|event| event["id"] == id)
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

    /// What the model plays from here on when a line woke a teammate for
    /// its inbox: `CALL` and `SAY` lines, as a person's line would carry.
    fn on_wake(&self, script: &str) {
        *self.scripts.on_wake.lock().unwrap() = script.to_string();
    }

    /// What the model plays from here on when a line offered a teammate
    /// again the items it set aside.
    fn on_pick_up(&self, script: &str) {
        *self.scripts.on_pick_up.lock().unwrap() = script.to_string();
    }

    /// Where an item stands, as `inbox.item` reads it.
    async fn state_of(&mut self, id: &str) -> String {
        let detail = self.ok("inbox.item", json!({"id": id})).await;
        detail["item"]["state"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    /// The line on `persona_id`'s tape that says `text`, once it is there.
    async fn line_saying(&self, persona_id: &str, text: &str) -> Value {
        let found = || {
            self.log
                .load(&StreamId::Tape(persona_id.to_string()))
                .into_iter()
                .find(|event| event["text"] == text)
        };
        until(&format!("{text:?} is on the tape"), || found().is_some()).await;
        found().unwrap()
    }

    /// The kinds of the lines on `persona_id`'s tape that say `text`.
    fn kinds_saying(&self, persona_id: &str, text: &str) -> Vec<String> {
        self.log
            .load(&StreamId::Tape(persona_id.to_string()))
            .into_iter()
            .filter(|event| event["text"] == text)
            .filter_map(|event| event["kind"].as_str().map(str::to_string))
            .collect()
    }
}

/// One teammate on each driver, and every thread they post from: the DM may
/// not, being the chat already, on either driver; a quiet job on ACP posts
/// once, stamped with its job; a work thread's file and post land stamped
/// from the thread; and a subagent reaches back into the DM with
/// `search_thread` and is refused the chat.
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
        scripts: first.scripts.clone(),
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

/// Ada's item that wants work, taken by Bob: the work thread's link carries
/// the item's words whole, and so does the copy on Ada's DM, so either
/// conversation says what was asked without opening the thread.
#[tokio::test(flavor = "multi_thread")]
async fn taken_works_link_carries_its_request_into_the_receivers_dm() {
    let mut desk = open().await;
    let ada = desk.teammate("Ada", false).await;
    let bob = desk.teammate("Bob", false).await;
    let message = "Check the winch.\nThe cable snapped at the drum.";
    let item = desk
        .send(
            &ada,
            json!({"to": ["Bob"], "title": "The winch", "message": message, "wants": "work"}),
        )
        .await;
    // Ada and Bob have the same access, so nothing asks the person.
    until("the item lands", || {
        desk.item(&bob, &item)
            .is_some_and(|item| item["held"].is_null())
    })
    .await;
    let take = json!({"items": [{"id": item, "state": "taken"}]});
    desk.ok(
        "session.prompt",
        json!({"personaId": bob, "text": format!("CALL update_inbox {take}")}),
    )
    .await;
    let link_on = |persona_id: &str| {
        desk.log
            .load(&StreamId::Tape(persona_id.to_string()))
            .into_iter()
            .rfind(|event| event["kind"] == "link" && event["openerId"] == ada.as_str())
    };
    // The link is written on Bob's tape and then copied onto Ada's, two
    // appends one after the other, so both are waited for.
    until("the work's link is on both DMs", || {
        link_on(&bob).is_some() && link_on(&ada).is_some()
    })
    .await;

    let received = link_on(&bob).unwrap();
    assert_eq!(received["threadKind"], "side");
    assert_eq!(received["personaId"], bob.as_str());
    assert_eq!(received["openerName"], "Ada");
    let request = received["request"].as_str().unwrap();
    assert!(
        request.starts_with("The winch\n"),
        "named by its title: {request}"
    );
    assert!(
        request.ends_with(message),
        "the words as they were sent: {request}"
    );
    let sent = link_on(&ada).expect("the sender's DM has the link too");
    assert_eq!(sent["thread"], received["thread"]);
    assert_eq!(sent["request"], received["request"]);
}

/// The inbox over the wire, end to end: Ada's `send` puts an item in Bob's
/// inbox and a row on her own conversation; Bob's roster row counts it and
/// `inbox.list` and the `inbox` subscription show it; Bob's `update_inbox`
/// answers it, which writes the sorted line on Bob's conversation, pushes
/// the inbox again, and lands the answer in Ada's inbox naming the item;
/// and `inbox.item` reads the item with its receipts and its reply.
#[tokio::test(flavor = "multi_thread")]
async fn an_item_crosses_the_wire_from_send_to_sort_and_its_answer_comes_back() {
    let mut desk = open().await;
    let ada = desk.teammate("Ada", false).await;
    let bob = desk.teammate("Bob", false).await;
    let inbox = desk.client.subscribe(json!({"inbox": bob})).await;
    desk.client
        .next_where(common::ANSWER, |frame| {
            frame["sub"] == inbox && frame["snapshot"]["open"] == json!([])
        })
        .await;

    let item = desk
        .send(&ada, json!({"to": ["Bob"], "title": "Crane permit", "message": "Is it signed?", "waiting": true}))
        .await;
    let row = desk
        .log
        .load(&StreamId::Tape(ada.clone()))
        .into_iter()
        .find(|event| event["kind"] == "inbox_sent")
        .expect("a row on the sender's conversation");
    assert_eq!(row["item"], item.as_str());
    assert_eq!(row["to"], bob.as_str());
    assert_eq!(row["title"], "Crane permit");
    let listed = desk.ok("inbox.list", json!({"personaId": bob})).await;
    assert_eq!(listed["unread"], 1, "{listed}");
    assert_eq!(listed["open"][0]["id"], item.as_str());
    assert_eq!(listed["open"][0]["waiting"], true);
    let sent = desk.ok("inbox.list", json!({"personaId": ada})).await;
    assert_eq!(sent["sent"][0]["id"], item.as_str(), "{sent}");
    let pushed = desk
        .client
        .next_where(common::ANSWER, |frame| {
            frame["sub"] == inbox && frame["snapshot"]["open"][0]["id"] == item.as_str()
        })
        .await;
    assert_eq!(pushed["snapshot"]["unread"], 1);

    let answer = json!({"items": [{"id": item, "answer": "Signed this morning."}]});
    desk.ok(
        "session.prompt",
        json!({"personaId": bob, "text": format!("CALL update_inbox {answer}")}),
    )
    .await;
    let closed = desk
        .client
        .next_where(common::ANSWER, |frame| {
            frame["sub"] == inbox && frame["snapshot"]["closed"][0]["id"] == item.as_str()
        })
        .await;
    assert_eq!(closed["snapshot"]["closed"][0]["outcome"], "answered");
    until("the sorted line on Bob's conversation", || {
        desk.log
            .load(&StreamId::Tape(bob.clone()))
            .iter()
            .any(|event| {
                event["kind"] == "inbox_sorted" && event["entries"][0]["item"] == item.as_str()
            })
    })
    .await;
    let detail = desk.ok("inbox.item", json!({"id": item})).await;
    let states: Vec<&str> = detail["item"]["receipts"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|receipt| receipt["state"].as_str())
        .collect();
    assert_eq!(states.first(), Some(&"new"), "{detail}");
    assert_eq!(states.last(), Some(&"closed"), "{detail}");
    assert_eq!(detail["replies"][0]["body"], "Signed this morning.");
    assert_eq!(detail["replies"][0]["to"], ada.as_str());
    let ada_inbox = desk.ok("inbox.list", json!({"personaId": ada})).await;
    assert_eq!(
        ada_inbox["open"][0]["replyTo"],
        item.as_str(),
        "{ada_inbox}"
    );
}

/// Bob's main conversation, woken by his inbox while he is idle, is quiet
/// over the wire: nothing he says there is a reply, but thinking on his tape
/// under the sort's line; his one post is an ordinary message in the chat and
/// what his roster row shows, and a second is refused. When the person writes
/// into such a turn, it answers them out loud.
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_the_inbox_woke_reaches_the_person_only_by_its_one_post() {
    let mut desk = open().await;
    let ada = desk.teammate("Ada", false).await;
    let bob = desk.teammate("Bob", false).await;
    desk.ok(
        "session.prompt",
        json!({"personaId": bob, "text": "SAY Morning."}),
    )
    .await;
    desk.posted(&bob, "Morning.").await;

    let item = desk
        .send(
            &ada,
            json!({"to": ["Bob"], "title": "Crane permit", "message": "Is it signed?"}),
        )
        .await;
    let answer = json!({"items": [{"id": item, "answer": "Not yet: it is the person's to sign."}]});
    let post = "Ada needs the crane permit: it is yours to sign.";
    desk.on_wake(&format!(
        "CALL update_inbox {answer}\nCALL post_to_chat {}\nCALL post_to_chat {}",
        json!({"text": post, "kind": "decision"}),
        json!({"text": "Again.", "kind": "decision"}),
    ));
    let closing = desk.results(StreamId::Tape(bob.clone()), 0).await;
    assert!(closing.contains("Posted to the chat."), "{closing}");
    assert!(closing.contains("Already posted"), "{closing}");
    assert_eq!(
        desk.kinds_saying(&bob, &closing),
        ["thought"],
        "the turn's words are its path"
    );
    let tape = desk.log.load(&StreamId::Tape(bob.clone()));
    assert!(
        tape.iter().any(|event| event["kind"] == "inbox_sorted"),
        "the sort is on the conversation"
    );
    let replies: Vec<&str> = tape
        .iter()
        .filter(|event| event["kind"] == "agent")
        .filter_map(|event| event["text"].as_str())
        .collect();
    assert_eq!(replies, ["Morning.", post], "the post and nothing else");
    let posted = desk.posted(&bob, post).await;
    assert!(posted.get("from").is_none(), "{posted}");

    let roster = desk.client.subscribe(json!({"view": "roster"})).await;
    let rows = desk
        .client
        .next_where(common::ANSWER, |frame| {
            frame["sub"] == roster && frame["snapshot"].is_array()
        })
        .await;
    let row = rows["snapshot"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["persona"]["id"] == bob.as_str())
        .cloned()
        .unwrap();
    assert_eq!(row["preview"]["text"], post, "{row}");

    // The person writes while the next quiet turn is still thinking: it
    // answers them.
    desk.send(
        &ada,
        json!({"to": ["Bob"], "title": "Logo colours", "wants": "nothing"}),
    )
    .await;
    desk.on_wake("HOLD SAY Sorted.");
    until("Bob is woken again", || desk.heard(WOKEN).len() == 2).await;
    desk.ok(
        "session.prompt",
        json!({"personaId": bob, "text": "SAY Here now."}),
    )
    .await;
    desk.posted(&bob, "Here now.").await;
    assert_eq!(desk.kinds_saying(&bob, "Here now."), ["agent"]);
    assert!(
        desk.kinds_saying(&bob, "Sorted.").is_empty(),
        "the person's line joined the quiet turn, which answered them instead"
    );
}

/// `read_inbox` reaches past what is open, over the wire: Bob lists what he
/// closed, with how it ended, and Ada lists what she sent, each with whom it
/// went to and where it stands now.
#[tokio::test(flavor = "multi_thread")]
async fn read_inbox_lists_closed_items_and_what_was_sent_with_where_each_stands() {
    let mut desk = open().await;
    let ada = desk.teammate("Ada", false).await;
    let bob = desk.teammate("Bob", false).await;
    let noted = desk
        .send(
            &ada,
            json!({"to": ["Bob"], "title": "Logo colours", "wants": "nothing"}),
        )
        .await;
    desk.send(&ada, json!({"to": ["Bob"], "title": "Crane permit"}))
        .await;
    let close = json!({"items": [{"id": noted, "state": "closed", "outcome": "noted"}]});
    desk.ok(
        "session.prompt",
        json!({"personaId": bob, "text": format!("CALL update_inbox {close}\nCALL read_inbox {}", json!({"list": "closed"}))}),
    )
    .await;
    let closed = desk.results(StreamId::Tape(bob.clone()), 0).await;
    assert!(
        closed.contains(r#""title":"Logo colours""#) && closed.contains(r#""outcome":"noted""#),
        "{closed}"
    );
    assert!(
        !closed.contains("Crane permit"),
        "only what closed: {closed}"
    );

    desk.ok(
        "session.prompt",
        json!({"personaId": ada, "text": format!("CALL read_inbox {}", json!({"list": "sent"}))}),
    )
    .await;
    let sent = desk.results(StreamId::Tape(ada.clone()), 2).await;
    let at = |title: &str| {
        sent.find(title)
            .unwrap_or_else(|| panic!("{title}: {sent}"))
    };
    assert!(
        at("Crane permit") < at("Logo colours"),
        "newest first: {sent}"
    );
    assert!(sent.contains(r#""to":"Bob""#), "{sent}");
    assert!(sent.contains(r#""state":"closed""#), "{sent}");
}

/// One `send` to two colleagues makes an item each, and each says who else
/// got it: on the item kept and read over the wire, and in `read_inbox`.
#[tokio::test(flavor = "multi_thread")]
async fn an_item_sent_to_several_says_who_else_got_it() {
    let mut desk = open().await;
    let ada = desk.teammate("Ada", false).await;
    let bob = desk.teammate("Bob", false).await;
    let cy = desk.teammate("Cy", false).await;
    desk.ok(
        "session.prompt",
        json!({"personaId": ada, "text": format!("CALL send {}", json!({"to": ["Bob", "Cy"], "title": "Release moves to Thursday", "wants": "nothing"}))}),
    )
    .await;
    desk.results(StreamId::Tape(ada.clone()), 0).await;
    let to_bob = desk.ok("inbox.list", json!({"personaId": bob})).await["open"][0].clone();
    assert_eq!(to_bob["alsoTo"], json!([cy]), "{to_bob}");
    let to_cy = desk.ok("inbox.list", json!({"personaId": cy})).await["open"][0].clone();
    assert_eq!(to_cy["alsoTo"], json!([bob]), "{to_cy}");
    let detail = desk.ok("inbox.item", json!({"id": to_bob["id"]})).await;
    assert_eq!(detail["item"]["alsoTo"], json!([cy]), "{detail}");
    desk.ok(
        "session.prompt",
        json!({"personaId": bob, "text": "CALL read_inbox {}"}),
    )
    .await;
    let listed = desk.results(StreamId::Tape(bob.clone()), 0).await;
    assert!(listed.contains(r#""alsoTo":["Cy"]"#), "{listed}");
}

/// Huck's story over the wire: Bob, at the person's work, leaves Ada's item
/// for later with why. Idle once that turn ends, he is offered it once, in a
/// quiet turn whose line gives his reason; he leaves it again, and nothing
/// more comes, not even after the person's next turn: from here it is the
/// item's own clock that offers it.
#[tokio::test(flavor = "multi_thread")]
async fn an_item_left_for_later_is_offered_once_when_idle_and_a_teammate_that_defers_again_settles()
{
    let mut desk = open().await;
    let ada = desk.teammate("Ada", false).await;
    let bob = desk.teammate("Bob", false).await;
    let item = desk
        .send(
            &ada,
            json!({"to": ["Bob"], "title": "Keyboards", "message": "Which keyboards should we buy?"}),
        )
        .await;
    let later = |reason: &str| json!({"items": [{"id": item, "state": "later", "reason": reason}]});
    desk.on_pick_up(&format!(
        "CALL update_inbox {}\nSAY Still later.",
        later("Still on the research.")
    ));
    // The person's line starts a turn before the item's own wake is due.
    desk.ok(
        "session.prompt",
        json!({"personaId": bob, "text": format!(
            "CALL update_inbox {}\nSAY Research done.",
            later("Finishing the deep research first; keyboards are a separate task.")
        )}),
    )
    .await;
    desk.posted(&bob, "Research done.").await;
    assert!(desk.heard(WOKEN).is_empty(), "{:?}", desk.heard(WOKEN));

    until("one pick-up once Bob is idle", || {
        desk.heard(PICKED_UP).len() == 1
    })
    .await;
    let line = &desk.heard(PICKED_UP)[0];
    assert!(line.contains("quiet inbox · "), "{line}");
    assert!(line.contains("\"Keyboards\" · from Ada"), "{line}");
    assert!(
        line.contains("for later")
            && line.contains(
                ": \"Finishing the deep research first; keyboards are a separate task.\""
            ),
        "with the reason he gave: {line}"
    );
    desk.line_saying(&bob, "Still later.").await;
    assert_eq!(
        desk.kinds_saying(&bob, "Still later."),
        ["thought"],
        "the pick-up is quiet"
    );
    tokio::time::sleep(GATHER * 2 + Duration::from_secs(1)).await;
    assert_eq!(desk.heard(PICKED_UP).len(), 1, "settled after one");
    assert_eq!(desk.state_of(&item).await, "later");

    desk.ok(
        "session.prompt",
        json!({"personaId": bob, "text": "SAY Anything else?"}),
    )
    .await;
    desk.posted(&bob, "Anything else?").await;
    tokio::time::sleep(GATHER * 2 + Duration::from_secs(1)).await;
    assert_eq!(
        desk.heard(PICKED_UP).len(),
        1,
        "the person's turn buys no second pick-up"
    );
    assert_eq!(desk.state_of(&item).await, "later");
}

/// The person's line in the gather delay wins over the pick-up the turn
/// before it armed: that one does nothing, and the pick-up that comes is
/// the person's turn's, the whole delay after it.
#[tokio::test(flavor = "multi_thread")]
async fn the_persons_line_in_the_gather_delay_wins_over_the_pick_up() {
    let mut desk = open().await;
    let ada = desk.teammate("Ada", false).await;
    let bob = desk.teammate("Bob", false).await;
    let item = desk
        .send(&ada, json!({"to": ["Bob"], "title": "Keyboards"}))
        .await;
    let later = json!({"items": [{"id": item, "state": "later", "reason": "after this"}]});
    desk.on_pick_up("SAY Picked up.");
    desk.ok(
        "session.prompt",
        json!({"personaId": bob, "text": format!("CALL update_inbox {later}\nSAY Later, then.")}),
    )
    .await;
    desk.posted(&bob, "Later, then.").await;
    // Held for most of the delay, so the first turn's pick-up comes due
    // just after this turn ends.
    desk.ok(
        "session.prompt",
        json!({"personaId": bob, "text": "HOLD SAY Here."}),
    )
    .await;
    let here = desk.posted(&bob, "Here.").await;
    let picked = desk.line_saying(&bob, "Picked up.").await;
    assert_eq!(picked["kind"], "thought");
    let waited = picked["ts"].as_i64().unwrap() - here["ts"].as_i64().unwrap();
    assert!(
        waited >= i64::try_from(GATHER.as_millis()).unwrap(),
        "the pick-up followed the person's turn by the delay, not the turn before: {waited} ms"
    );
    tokio::time::sleep(GATHER * 2 + Duration::from_secs(1)).await;
    assert_eq!(
        desk.heard(PICKED_UP).len(),
        1,
        "{:?}",
        desk.heard(PICKED_UP)
    );
}

/// A turn the person stopped is followed by no pick-up, and neither is the
/// turn before it whose pick-up it took the place of.
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_the_person_stopped_is_followed_by_no_pick_up() {
    let mut desk = open().await;
    let ada = desk.teammate("Ada", false).await;
    let bob = desk.teammate("Bob", false).await;
    let item = desk
        .send(&ada, json!({"to": ["Bob"], "title": "Keyboards"}))
        .await;
    let later = json!({"items": [{"id": item, "state": "later", "reason": "after this"}]});
    desk.on_pick_up("SAY Picked up.");
    desk.ok(
        "session.prompt",
        json!({"personaId": bob, "text": format!("CALL update_inbox {later}\nSAY Later, then.")}),
    )
    .await;
    desk.posted(&bob, "Later, then.").await;
    desk.ok(
        "session.prompt",
        json!({"personaId": bob, "text": "HOLD SAY Never said."}),
    )
    .await;
    until("the held turn reached the model", || {
        !desk.heard("Never said.").is_empty()
    })
    .await;
    desk.ok("session.cancel", json!({"personaId": bob})).await;
    tokio::time::sleep(GATHER * 2 + Duration::from_secs(1)).await;
    assert!(
        desk.heard(PICKED_UP).is_empty(),
        "{:?}",
        desk.heard(PICKED_UP)
    );
    assert_eq!(desk.state_of(&item).await, "later");
}

/// The desk restarts with Bob's item still left for later: once it has
/// opened, Bob, idle and not running, is started and offered it once, in a
/// quiet turn with his reason, without the person writing to him first; he
/// leaves it again, and the open desk offers nothing more.
#[tokio::test(flavor = "multi_thread")]
async fn a_desk_that_opens_offers_what_was_left_for_later_once() {
    let mut first = open().await;
    let ada = first.teammate("Ada", false).await;
    let bob = first.teammate("Bob", false).await;
    let item = first
        .send(&ada, json!({"to": ["Bob"], "title": "Keyboards"}))
        .await;
    let later = |reason: &str| json!({"items": [{"id": item, "state": "later", "reason": reason}]});
    first.on_pick_up(&format!(
        "CALL update_inbox {}\nSAY Still later.",
        later("Still on the research.")
    ));
    first
        .ok(
            "session.prompt",
            json!({"personaId": bob, "text": format!(
                "CALL update_inbox {}\nSAY Research done.",
                later("Finishing the research first.")
            )}),
        )
        .await;
    first.posted(&bob, "Research done.").await;
    first.line_saying(&bob, "Still later.").await;
    assert_eq!(first.heard(PICKED_UP).len(), 1);

    // The desk stops and opens again on what it kept.
    let second = tempfile::tempdir().unwrap();
    std::fs::copy(
        first.log.root().join("room.jsonl"),
        second.path().join("room.jsonl"),
    )
    .unwrap();
    std::fs::create_dir_all(second.path().join("inbox")).unwrap();
    for kept in std::fs::read_dir(first.log.root().join("inbox")).unwrap() {
        let kept = kept.unwrap();
        std::fs::copy(
            kept.path(),
            second.path().join("inbox").join(kept.file_name()),
        )
        .unwrap();
    }
    let (client, log) = door(second.path()).await;
    let desk = Desk {
        client,
        log,
        workspace: first.workspace.clone(),
        heard: first.heard.clone(),
        scripts: first.scripts.clone(),
        _roots: vec![second],
    };
    desk.on_pick_up("SAY Not yet.");
    until("Bob is offered it once the desk has opened", || {
        desk.heard(PICKED_UP).len() == 2
    })
    .await;
    let line = &desk.heard(PICKED_UP)[1];
    assert!(
        line.contains("\"Keyboards\" · from Ada") && line.contains(": \"Still on the research.\""),
        "{line}"
    );
    desk.line_saying(&bob, "Not yet.").await;
    assert_eq!(desk.kinds_saying(&bob, "Not yet."), ["thought"], "quiet");
    tokio::time::sleep(GATHER * 2 + Duration::from_secs(1)).await;
    assert_eq!(
        desk.heard(PICKED_UP).len(),
        2,
        "{:?}",
        desk.heard(PICKED_UP)
    );
    assert!(desk.heard(WOKEN).is_empty(), "{:?}", desk.heard(WOKEN));
}

/// One item Bob left for later, as a stopped desk kept it.
fn left_for_later(
    id: &str,
    from: &str,
    to: &str,
    title: &str,
    ts: i64,
    offer_again: Option<i64>,
) -> Value {
    let mut item = json!({
        "kind": "inbox_item", "id": id, "ts": ts, "from": from,
        "fromThread": {"kind": "dm", "key": from}, "to": to, "title": title,
        "body": format!("About {title}."), "wants": "answer", "state": "later",
        "reason": "after the research",
        "receipts": [
            {"state": "new", "at": ts},
            {"state": "later", "at": ts, "reason": "after the research"},
        ],
        "offered": ts, "senderTold": 0,
    });
    if let Some(at) = offer_again {
        item["offerAgain"] = json!(at);
    }
    item
}

/// A desk that opens with one item of Bob's open three days and one due to
/// be offered again closes the first as expired, once, with its receipt;
/// Ada, its sender, gets Hotline's marked notice and is woken quietly by it;
/// Bob is offered the other once and not the expired one; nothing more
/// follows.
#[tokio::test(flavor = "multi_thread")]
async fn a_desk_that_opens_with_overdue_items_expires_one_once_and_offers_the_other_once() {
    let mut first = open().await;
    let ada = first.teammate("Ada", false).await;
    let bob = first.teammate("Bob", false).await;
    let second = tempfile::tempdir().unwrap();
    std::fs::copy(
        first.log.root().join("room.jsonl"),
        second.path().join("room.jsonl"),
    )
    .unwrap();
    std::fs::create_dir_all(second.path().join("inbox")).unwrap();
    let old = "0ld00000-0000-4000-8000-000000000001";
    let due = "due00000-0000-4000-8000-000000000002";
    let kept = [
        left_for_later(
            old,
            &ada,
            &bob,
            "Old crane",
            now() - 3 * 86_400_000 - 60_000,
            None,
        ),
        left_for_later(
            due,
            &ada,
            &bob,
            "Keyboards",
            now() - 7_200_000,
            Some(now() - 60_000),
        ),
    ];
    std::fs::write(
        second.path().join("inbox").join(format!("{bob}.jsonl")),
        kept.iter()
            .map(|item| format!("{item}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let (client, log) = door(second.path()).await;
    let mut desk = Desk {
        client,
        log,
        workspace: first.workspace.clone(),
        heard: first.heard.clone(),
        scripts: first.scripts.clone(),
        _roots: vec![second],
    };
    desk.on_pick_up("SAY Not yet.");
    let bobs = |desk: &Desk| -> Vec<String> {
        desk.heard(PICKED_UP)
            .into_iter()
            .filter(|line| line.contains("from Ada"))
            .collect()
    };
    until("Bob is offered what is still open", || {
        bobs(&desk).len() == 1
    })
    .await;
    let line = &bobs(&desk)[0];
    assert!(line.contains("\"Keyboards\""), "{line}");
    assert!(!line.contains("Old crane"), "not what expired: {line}");
    let detail = desk.ok("inbox.item", json!({"id": old})).await;
    assert_eq!(detail["item"]["state"], "closed", "{detail}");
    assert_eq!(detail["item"]["outcome"], "expired", "{detail}");
    assert_eq!(
        detail["item"]["receipts"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["outcome"],
        "expired"
    );
    let adas = desk.ok("inbox.list", json!({"personaId": ada})).await;
    let notices: Vec<&Value> = adas["open"]
        .as_array()
        .unwrap()
        .iter()
        .chain(adas["closed"].as_array().unwrap())
        .filter(|item| item["notice"] == "expired")
        .collect();
    assert_eq!(notices.len(), 1, "{adas}");
    assert_eq!(notices[0]["replyTo"], old);
    assert_eq!(notices[0]["wants"], "nothing");
    assert!(
        notices[0]["body"]
            .as_str()
            .unwrap()
            .starts_with("This is Hotline's notice, not Bob's words."),
        "{}",
        notices[0]
    );
    // The line that woke her is the inbox's quiet one, and the notice was
    // in its index, offered to that turn.
    let notice = notices[0]["id"].as_str().unwrap().to_string();
    until("Ada is woken by it, quietly", || {
        !desk.heard(WOKEN).is_empty()
            && desk
                .item(&ada, &notice)
                .is_some_and(|item| item["offered"].is_i64())
    })
    .await;
    assert!(
        desk.heard(WOKEN)[0].contains("quiet inbox · "),
        "{:?}",
        desk.heard(WOKEN)
    );
    tokio::time::sleep(GATHER * 2 + Duration::from_secs(1)).await;
    assert_eq!(bobs(&desk).len(), 1, "{:?}", bobs(&desk));
    assert_eq!(
        desk.ok("inbox.list", json!({"personaId": ada})).await["open"]
            .as_array()
            .unwrap()
            .iter()
            .chain(
                desk.ok("inbox.list", json!({"personaId": ada})).await["closed"]
                    .as_array()
                    .unwrap()
            )
            .filter(|item| item["notice"] == "expired")
            .count(),
        1,
        "one notice"
    );
    assert!(
        desk.chat(&ada).is_empty(),
        "nothing reached the person's chat"
    );
}

/// Over the wire, a teammate cannot close an item as expired, and nothing
/// it sends carries Hotline's mark.
#[tokio::test(flavor = "multi_thread")]
async fn a_teammate_cannot_set_expired_or_forge_hotlines_notice() {
    let mut desk = open().await;
    let ada = desk.teammate("Ada", false).await;
    let bob = desk.teammate("Bob", false).await;
    let item = desk
        .send(&ada, json!({"to": ["Bob"], "title": "Keyboards"}))
        .await;
    let expire = json!({"items": [{"id": item, "state": "closed", "outcome": "expired"}]});
    desk.ok(
        "session.prompt",
        json!({"personaId": bob, "text": format!("CALL update_inbox {expire}")}),
    )
    .await;
    let refused = desk.results(StreamId::Tape(bob.clone()), 0).await;
    assert!(refused.contains("done, noted or dropped"), "{refused}");
    assert_ne!(desk.state_of(&item).await, "closed");
    desk.ok(
        "session.prompt",
        json!({"personaId": ada, "text": format!("CALL send {}", json!({"to": ["Bob"], "title": "Expired: Keyboards", "notice": "expired"}))}),
    )
    .await;
    desk.results(StreamId::Tape(ada.clone()), 1).await;
    let listed = desk.ok("inbox.list", json!({"personaId": bob})).await;
    assert!(
        listed["open"]
            .as_array()
            .unwrap()
            .iter()
            .chain(listed["closed"].as_array().unwrap())
            .all(|item| item.get("notice").is_none()),
        "{listed}"
    );
}
