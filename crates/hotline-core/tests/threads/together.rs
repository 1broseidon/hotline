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
//! answer back for [`STALL`], and every line with `HOLD` holds its own. Bob is
//! on ACP, the fixture child playing the same `CALL` lines over the desk's MCP
//! server.

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
/// from the thread; a colleague's question is answered by the target's DM,
/// which may not post either; and a subagent reaches back into the DM with
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

    // A colleague's question is answered by the target's main conversation,
    // whose reply is already the chat: it posts to neither chat.
    let ask = json!({
        "to": "Bob", "intent": "ask",
        "message": "About the crane:\nCALL post_to_chat {\"text\":\"Ada asked me about the crane.\"}",
    });
    desk.ok(
        "session.prompt",
        json!({"personaId": ada, "text": format!("CALL message_teammate {ask}")}),
    )
    .await;
    let answered = desk.results(StreamId::Tape(bob.clone()), 1).await;
    assert!(answered.contains("already the chat"), "{answered}");
    for whose in [&ada, &bob] {
        assert!(
            desk.chat(whose)
                .iter()
                .all(|event| event["text"] != "Ada asked me about the crane.")
        );
    }

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

/// Every exchange request on the room stream, as its pair last saved it.
fn requests(log: &Log) -> Vec<Value> {
    let mut pairs: Vec<Value> = Vec::new();
    for pair in log
        .load(&StreamId::Room)
        .into_iter()
        .filter(|event| event["kind"] == "exchange_pair")
    {
        pairs.retain(|kept| kept["id"] != pair["id"]);
        pairs.push(pair);
    }
    pairs
        .iter()
        .flat_map(|pair| pair["requests"].as_array().cloned().unwrap_or_default())
        .collect()
}

/// The id of the exchange request with these words, once it is saved.
fn sent(log: &Log, message: &str) -> Option<String> {
    requests(log)
        .into_iter()
        .find(|request| request["message"] == message)
        .and_then(|request| request["id"].as_str().map(str::to_string))
}

/// The delivery among `events` with this cause for this request.
fn delivered(events: &[Value], cause: &str, id: &str) -> Option<Value> {
    events
        .iter()
        .find(|event| {
            event["kind"] == "delivery"
                && event["cause"]["kind"] == cause
                && event["cause"]["requestId"] == id
        })
        .cloned()
}

/// A follow-up handed into the thread a handoff ran in. Ada hands Bob work;
/// once the result is back she hands him more, naming the thread the first
/// ran in, and it lands in that same work thread of Bob's, with the first
/// brief still in it, and its result comes back to her as the first did.
#[tokio::test(flavor = "multi_thread")]
async fn a_follow_up_handoff_lands_in_the_thread_the_first_ran_in() {
    let mut desk = open().await;
    let ada = desk.teammate("Ada", false).await;
    desk.ok("persona.update", json!({"id": ada, "patch": {"reach": "workspace"}})).await;
    desk.teammate("Bob", false).await;
    let result_for = |desk: &Desk, id: &str| {
        desk.log
            .load(&StreamId::Tape(ada.clone()))
            .into_iter()
            .find(|event| event["kind"] == "delivery" && event["cause"]["requestId"] == id)
    };

    let first =
        json!({"to": "Bob", "intent": "handoff", "message": "SAY The password is hunter2."});
    desk.ok(
        "session.prompt",
        json!({"personaId": ada, "text": format!("CALL message_teammate {first}")}),
    )
    .await;
    let card = || {
        desk.log
            .load(&StreamId::Tape(ada.clone()))
            .into_iter()
            .find(|event| event["kind"] == "permission" && event.get("decision").is_none())
            .and_then(|event| event["requestId"].as_str().map(str::to_string))
    };
    until("the first handoff asks the person", || card().is_some()).await;
    let card = card().unwrap();
    desk.ok(
        "session.answer_permission",
        json!({"personaId": ada, "requestId": card, "optionId": "allow_always"}),
    )
    .await;
    until("the first handoff is queued", || {
        !requests(&desk.log).is_empty()
    })
    .await;
    let first_id = requests(&desk.log)[0]["id"].as_str().unwrap().to_owned();
    until("the first result is back", || {
        result_for(&desk, &first_id).is_some()
    })
    .await;

    let follow_up = json!({
        "to": "Bob", "intent": "handoff", "thread": first_id,
        "message": "SAY Logged in with it.",
    });
    desk.ok(
        "session.prompt",
        json!({"personaId": ada, "text": format!("CALL message_teammate {follow_up}")}),
    )
    .await;
    until("the follow-up is queued", || requests(&desk.log).len() == 2).await;
    let second_id = requests(&desk.log)[1]["id"].as_str().unwrap().to_owned();
    until("the follow-up's result is back", || {
        result_for(&desk, &second_id).is_some()
    })
    .await;

    let saved = requests(&desk.log);
    assert_eq!(saved[1]["continues"], first_id.as_str());
    let thread = saved[0]["thread"]
        .as_str()
        .expect("a work thread")
        .to_owned();
    assert_eq!(saved[1]["thread"], thread.as_str(), "one thread for both");
    let briefs: Vec<String> = desk
        .log
        .load(&StreamId::Side(thread))
        .into_iter()
        .filter(|event| event["kind"] == "delivery" && event["cause"]["kind"] == "handoff")
        .filter_map(|event| event["text"].as_str().map(str::to_string))
        .collect();
    assert_eq!(
        briefs,
        ["SAY The password is hunter2.", "SAY Logged in with it."]
    );
    assert_eq!(
        result_for(&desk, &second_id).unwrap()["cause"]["status"],
        "done"
    );
}

/// A delegated thread, the whole of it. Ada hands Bob work and its result
/// comes back; the person speaks in Bob's thread, and Ada is shown his report
/// without their words and hears it once with her next line, never woken for
/// it. Ada's ask naming the thread is answered there, the person has Bob ask
/// Ada back, which reaches her conversation without a card, and her plain
/// ask is answered by Bob's main conversation.
///
/// Each message opens with a line of its own, since a delivery names the
/// message and the work it is about by their first lines, and the model plays
/// the first `SAY` it reads.
#[tokio::test(flavor = "multi_thread")]
async fn a_delegated_thread_reports_back_takes_the_persons_words_and_answers_questions_where_the_work_is()
 {
    let mut desk = open().await;
    let ada = desk.teammate("Ada", false).await;
    desk.ok("persona.update", json!({"id": ada, "patch": {"reach": "workspace"}})).await;
    // Bob keeps to his workspace, so only the delegation lets him ask Ada
    // back without a card.
    let bob_draft = json!({"name":"Bob","goal":"Keep the harbour.","cwd":desk.workspace});
    let bob = desk.ok("persona.create", json!({"draft": bob_draft})).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    desk.ok("session.start", json!({"personaId": bob})).await;
    let tape =
        |desk: &Desk, persona_id: &str| desk.log.load(&StreamId::Tape(persona_id.to_string()));
    let answered = |desk: &Desk| -> Vec<String> {
        tape(desk, &ada)
            .iter()
            .filter(|event| event["kind"] == "delivery")
            .filter_map(|event| event["cause"]["requestId"].as_str().map(str::to_string))
            .collect()
    };

    let handoff =
        json!({"to": "Bob", "intent": "handoff", "message": "Mind the crane.\nSAY Report one."});
    desk.ok(
        "session.prompt",
        json!({"personaId": ada, "text": format!("CALL message_teammate {handoff}")}),
    )
    .await;
    let card = || {
        tape(&desk, &ada)
            .into_iter()
            .find(|event| event["kind"] == "permission" && event.get("decision").is_none())
            .and_then(|event| event["requestId"].as_str().map(str::to_string))
    };
    until("the handoff asks the person", || card().is_some()).await;
    let card = card().unwrap();
    desk.ok(
        "session.answer_permission",
        json!({"personaId": ada, "requestId": card, "optionId": "allow_always"}),
    )
    .await;
    until("the handoff is queued", || !requests(&desk.log).is_empty()).await;
    let first_id = requests(&desk.log)[0]["id"].as_str().unwrap().to_owned();
    until("the result is back and Ada has taken it", || {
        delivered(&tape(&desk, &ada), "peer", &first_id)
            .is_some_and(|result| result["receipt"] == "read")
    })
    .await;
    let result = delivered(&tape(&desk, &ada), "peer", &first_id).unwrap();
    let pair = StreamId::Pair(result["cause"]["threadKey"].as_str().unwrap().to_owned());
    let thread = requests(&desk.log)[0]["thread"]
        .as_str()
        .expect("a work thread")
        .to_owned();
    let side = json!({"kind": "side", "key": thread});
    let work = StreamId::Side(thread.clone());

    // The person's turn in Bob's thread reaches Ada as his report alone.
    desk.ok("thread.continue", json!({"thread": side})).await;
    desk.ok(
        "thread.prompt",
        json!({"thread": side, "text": "my secret is swordfish\nSAY Report two."}),
    )
    .await;
    let report = || {
        tape(&desk, &ada).into_iter().find(|event| {
            event["kind"] == "notice"
                && event["id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("progress:"))
                && event["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("Report two."))
        })
    };
    until("Ada is shown Bob's report", || report().is_some()).await;
    let report = report().unwrap();
    assert!(
        !report["text"].as_str().unwrap().contains("swordfish"),
        "{report}"
    );
    let results = answered(&desk);
    assert_eq!(
        results.iter().filter(|id| **id == first_id).count(),
        1,
        "the person's turn is no result: {results:?}"
    );

    // Ada was not woken for it, and hears it once with her next line.
    desk.ok(
        "session.prompt",
        json!({"personaId": ada, "text": "SAY Heard."}),
    )
    .await;
    until("Ada hears the person's next line", || {
        !desk.heard("SAY Heard.").is_empty()
    })
    .await;
    let carrying = desk.heard("Report two.");
    assert_eq!(carrying.len(), 2, "Bob's turn and Ada's line: {carrying:?}");
    assert!(carrying[0].contains("swordfish"), "{carrying:?}");
    assert!(carrying[1].contains("SAY Heard."), "{carrying:?}");
    assert_eq!(
        carrying[1].matches("Report two.").count(),
        1,
        "{carrying:?}"
    );
    assert!(!carrying[1].contains("swordfish"), "{carrying:?}");

    // An ask naming the thread is answered in it, and it stays open.
    let three = "About the crane:\nSAY Answer three.";
    let in_thread = json!({"to": "Bob", "intent": "ask", "thread": first_id, "message": three});
    desk.ok(
        "session.prompt",
        json!({"personaId": ada, "text": format!("CALL message_teammate {in_thread}")}),
    )
    .await;
    until("Ada asks in the thread", || {
        sent(&desk.log, three).is_some()
    })
    .await;
    let three = sent(&desk.log, three).unwrap();
    until("Ada has the thread's answer", || {
        delivered(&tape(&desk, &ada), "peer", &three).is_some()
    })
    .await;
    assert!(
        delivered(&desk.log.load(&work), "question", &three).is_some(),
        "the question is asked in Bob's thread"
    );
    let answer = delivered(&tape(&desk, &ada), "peer", &three).unwrap();
    assert_eq!(answer["cause"]["status"], "done", "{answer}");
    assert_eq!(answer["text"], "Answer three.");
    let link = tape(&desk, &bob)
        .into_iter()
        .find(|event| event["kind"] == "link" && event["thread"] == thread.as_str())
        .expect("Bob's tape links his thread");
    assert_ne!(link["state"], "closed", "{link}");

    // Bob's question back reaches the conversation that handed the work
    // over, and its answer returns to his thread.
    let four = "About the crane:\nSAY Back four.";
    let back = json!({"to": "Ada", "intent": "ask", "message": four});
    desk.ok(
        "thread.prompt",
        json!({"thread": side, "text": format!("CALL message_teammate {back}")}),
    )
    .await;
    until("Bob asks Ada back", || sent(&desk.log, four).is_some()).await;
    let four = sent(&desk.log, four).unwrap();
    until("Bob's thread has Ada's answer", || {
        delivered(&desk.log.load(&work), "peer", &four).is_some()
    })
    .await;
    let ada_tape = tape(&desk, &ada);
    let question = ada_tape
        .iter()
        .position(|event| {
            event["kind"] == "delivery"
                && event["cause"]["kind"] == "question"
                && event["cause"]["requestId"] == four.as_str()
        })
        .expect("the question reaches Ada's conversation");
    assert!(
        ada_tape[question..]
            .iter()
            .any(|event| event["kind"] == "agent" && event["text"] == "Back four."),
        "Ada answers it there"
    );
    assert_eq!(
        delivered(&desk.log.load(&work), "peer", &four).unwrap()["text"],
        "Back four."
    );

    // A plain ask is answered by Bob's main conversation, and the pair's
    // thread keeps both lines.
    let five = "About the crane:\nSAY Main five.";
    let plain = json!({"to": "Bob", "intent": "ask", "message": five});
    desk.ok(
        "session.prompt",
        json!({"personaId": ada, "text": format!("CALL message_teammate {plain}")}),
    )
    .await;
    until("Ada asks Bob", || sent(&desk.log, five).is_some()).await;
    let five_id = sent(&desk.log, five).unwrap();
    let kept = |desk: &Desk, id: &str, text: &str| {
        desk.log
            .load(&pair)
            .iter()
            .any(|event| event["id"] == id && event["text"] == text)
    };
    until("Ada has Bob's answer and the pair keeps it", || {
        delivered(&tape(&desk, &ada), "peer", &five_id).is_some()
            && kept(&desk, &format!("result:{five_id}"), "Main five.")
    })
    .await;
    let bob_tape = tape(&desk, &bob);
    let question = bob_tape
        .iter()
        .position(|event| {
            event["kind"] == "delivery"
                && event["cause"]["kind"] == "question"
                && event["cause"]["requestId"] == five_id.as_str()
        })
        .expect("the ask reaches Bob's main conversation");
    assert!(
        bob_tape[question..]
            .iter()
            .any(|event| event["kind"] == "agent" && event["text"] == "Main five."),
        "Bob answers it there"
    );
    let answer = delivered(&tape(&desk, &ada), "peer", &five_id).unwrap();
    assert_eq!(answer["cause"]["status"], "done", "{answer}");
    assert_eq!(answer["text"], "Main five.");
    assert!(kept(&desk, &five_id, five), "the pair keeps the question");

    // The first handoff's card is the only one, and each request came back
    // to Ada once.
    let cards: usize = [
        StreamId::Room,
        StreamId::Tape(ada.clone()),
        StreamId::Tape(bob.clone()),
        work,
        pair,
    ]
    .iter()
    .map(|stream| {
        desk.log
            .load(stream)
            .iter()
            .filter(|event| event["kind"] == "permission")
            .count()
    })
    .sum();
    assert_eq!(cards, 1);
    let results = answered(&desk);
    let mut once = results.clone();
    once.sort();
    once.dedup();
    assert_eq!(once.len(), results.len(), "{results:?}");
}

/// The main conversation's Interrupt, by either command the window sends it
/// as, reaches the room as the person redirecting the turn: the colleague
/// whose question that turn was answering hears that the person interrupted
/// the answer and may ask again, never that the person stopped it.
#[tokio::test(flavor = "multi_thread")]
async fn the_main_conversations_interrupt_over_the_wire_ends_a_colleagues_answer_as_interrupted() {
    let mut desk = open().await;
    let ada = desk.teammate("Ada", false).await;
    let cy = desk.teammate("Cy", false).await;
    let interrupted = "The person interrupted the answer to this question.";
    let told = |desk: &Desk| -> Vec<String> {
        desk.log
            .load(&StreamId::Tape(ada.clone()))
            .into_iter()
            .filter(|event| event["kind"] == "notice")
            .filter_map(|event| event["text"].as_str().map(str::to_string))
            .filter(|text| text.contains("answering"))
            .collect()
    };
    for (n, (cmd, params)) in [
        (
            "session.cancel",
            json!({"personaId": cy, "interrupt": true}),
        ),
        (
            "thread.cancel",
            json!({"thread": {"kind": "dm", "key": cy}, "interrupt": true}),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        // Escaped, so only Cy's turn answering it is held, not Ada's asking.
        let question = format!(r"Which port, {n}? \u0048OLD");
        let ask = format!(r#"{{"to":"Cy","intent":"ask","message":"{question}"}}"#);
        desk.ok(
            "session.prompt",
            json!({"personaId": ada, "text": format!("CALL message_teammate {ask}")}),
        )
        .await;
        let held = format!("Which port, {n}? HOLD");
        until("Cy's main conversation is answering", || {
            !desk.heard(&held).is_empty()
        })
        .await;
        desk.ok(cmd, params).await;
        until(&format!("{cmd}: Ada is told"), || {
            told(&desk).len() == n + 1
        })
        .await;
        let said = &told(&desk)[n];
        assert!(said.contains(interrupted), "{cmd}: {said}");
    }
    let tape = desk.log.load(&StreamId::Tape(ada.clone()));
    assert!(
        tape.iter()
            .all(|event| !event.to_string().contains("Do not ask it again")),
        "never as the person's stop"
    );
}
