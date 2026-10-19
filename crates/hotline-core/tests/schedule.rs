//! Jobs over the wire (ADR 0008 §7): a job is created, listed, silenced and
//! cancelled, and the room stream is what remembers it; each firing is a turn
//! on the job's own thread, never the teammate's DM, and what the person is
//! told reaches the chat as a post stamped with the job.
//!
//! Most of these need a loop faster than the fifteen seconds a teammate or the
//! desk may ask for, so they write the room stream before the core starts
//! ([`seeded`]): a first desk on a throwaway root makes the teammate and its
//! model connection over the wire, and its room stream, with the jobs added,
//! is the second desk's before that one opens. The teammate is Hotline Agent
//! on a disposable model endpoint whose answers its line spells out (`SAY`,
//! `CALL`, `STALL`, below), and the endpoint keeps every line it was handed.

mod common;

use common::{Client, until};
use hotline_core::log::{Log, StreamId};
use hotline_core::wire::Door;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;

const TOKEN: &str = "a-token-only-this-harness-knows";
const PATIENCE: Duration = Duration::from_secs(30);
/// How long a `STALL` line holds the model: longer than two of a stalling
/// loop's intervals.
const STALL: Duration = Duration::from_secs(4);

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

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
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

    let when = now() + 3_600_000;
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
    let next_at = now() + 3_600_000;
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

/// What follows `word ` on the first line that has it.
fn after<'a>(said: &'a str, word: &str) -> Option<&'a str> {
    said.lines()
        .find_map(|line| line.find(word).map(|at| line[at + word.len()..].trim()))
}

/// The model: what its line spells out. Each `CALL <tool> <json>` is one call
/// to one of Hotline's tools, made in order; then it says what follows `SAY`,
/// or `RESULTS` with what each call came back with, or "No change." A line
/// with `STALL` holds its first answer back for [`STALL`]. A line of Hotline's
/// own execution data (a subagent's report coming back) is not a script, so
/// a report that quotes one is never run again. Every line it is handed is
/// kept in `heard`.
async fn model(request: Value, heard: Arc<Mutex<Vec<String>>>) -> String {
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
    let asked = text_of(&messages[asked_at]);
    let answered: Vec<String> = messages[asked_at + 1..]
        .iter()
        .filter(|message| message["role"] == "tool")
        .map(text_of)
        .collect();
    if answered.is_empty() {
        heard.lock().unwrap().push(asked.clone());
        if asked.contains("STALL") {
            tokio::time::sleep(STALL).await;
        }
    }
    if asked.contains("Hotline execution data") {
        return streamed(vec![chunk(
            json!({"role":"assistant","content":"Noted."}),
            Some("stop"),
        )]);
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
    let words = match after(&asked, "SAY ") {
        Some(said) => said.to_string(),
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
    streamed(vec![
        chunk(json!({"role":"assistant","content":words}), None),
        chunk(json!({}), Some("stop")),
    ])
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

/// A desk with a teammate on the model endpoint, and what a test reads off it.
struct Desk {
    client: Client,
    log: Log,
    persona_id: String,
    heard: Arc<Mutex<Vec<String>>>,
    _roots: Vec<TempDir>,
}

/// Opens a desk on `root` behind a door, and connects to it.
async fn door(root: &Path) -> (Client, Log) {
    let desk = common::open_desk(root).unwrap();
    let log = desk.log.clone();
    let parts = desk.parts();
    let door = Door::bind(desk.log.clone(), TOKEN.to_string(), Arc::new(desk), parts).unwrap();
    let port = door.port();
    tokio::spawn(door.run());
    (Client::connect(port, TOKEN).await, log)
}

/// Makes the model connection and a teammate on it, Ada, with Background
/// work granted, over the wire of the desk `client` is connected to.
async fn teammate(client: &mut Client, base: &str, workspace: &Path) -> String {
    teammate_with(client, base, workspace, json!({})).await
}

/// [`teammate`], with more of the draft: `extra`'s fields are added to it.
async fn teammate_with(client: &mut Client, base: &str, workspace: &Path, extra: Value) -> String {
    let saved = client
        .call(
            "credential.custom_save",
            json!({"draft": {
                "name":"chat","baseUrl":base,"api":"chat_completions","models":["vendor/coder"]
            }}),
        )
        .await;
    assert_eq!(saved["ok"], true, "{saved}");
    let mut draft = json!({
        "name":"Ada","goal":"Keep the harbour.","cwd":workspace.to_string_lossy(),
        "backgroundWork": true,
    });
    for (key, value) in extra.as_object().cloned().unwrap_or_default() {
        draft[key] = value;
    }
    let created = client.call("persona.create", json!({"draft": draft})).await;
    assert_eq!(created["ok"], true, "{created}");
    created["result"]["id"].as_str().unwrap().to_owned()
}

/// A desk whose teammate's jobs were on its room stream before it started:
/// `jobs` writes them for the teammate's id. See the module.
async fn seeded(jobs: impl FnOnce(&str) -> Vec<Value>) -> Desk {
    let (base, heard) = endpoint().await;
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let workspace = second.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let (mut making, made) = door(first.path()).await;
    let persona_id = teammate(&mut making, &base, &workspace).await;
    let mut room = std::fs::read_to_string(made.root().join("room.jsonl")).unwrap();
    for job in jobs(&persona_id) {
        room.push_str(&job.to_string());
        room.push('\n');
    }
    std::fs::write(second.path().join("room.jsonl"), room).unwrap();
    let (client, log) = door(second.path()).await;
    Desk {
        client,
        log,
        persona_id,
        heard,
        _roots: vec![first, second],
    }
}

/// A desk opened empty, with the teammate made over its wire.
async fn fresh() -> Desk {
    let (base, heard) = endpoint().await;
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let (mut client, log) = door(root.path()).await;
    let persona_id = teammate(&mut client, &base, &workspace).await;
    Desk {
        client,
        log,
        persona_id,
        heard,
        _roots: vec![root],
    }
}

/// A job as the room stream holds one: a loop of `every` that first fires in
/// a moment, entered at the desk.
fn looping(id: &str, persona_id: &str, every: i64, prompt: &str) -> Value {
    json!({
        "kind": "schedule",
        "id": id,
        "personaId": persona_id,
        "every": every,
        "prompt": prompt,
        "operatorCreated": true,
        "nextAt": now() + 500,
        "createdAt": now(),
    })
}

impl Desk {
    /// What the teammate's DM holds, on disk.
    fn tape(&self) -> Vec<Value> {
        self.log.load(&StreamId::Tape(self.persona_id.clone()))
    }

    /// The teammate's messages in its chat.
    fn chat(&self) -> Vec<Value> {
        self.tape()
            .into_iter()
            .filter(|event| event["kind"] == "agent")
            .collect()
    }

    /// What a job's thread holds, on disk, its link left out.
    fn thread(&self, job_id: &str) -> Vec<Value> {
        self.log
            .load(&StreamId::Job(job_id.to_string()))
            .into_iter()
            .filter(|event| event["kind"] != "link")
            .collect()
    }

    /// The kinds of what a job's thread holds.
    fn kinds(&self, job_id: &str) -> Vec<String> {
        self.thread(job_id)
            .iter()
            .map(|event| event["kind"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    /// How a job's thread stands, as its link says.
    fn state(&self, job_id: &str) -> Option<String> {
        self.log
            .load(&StreamId::Job(job_id.to_string()))
            .iter()
            .find(|event| event["kind"] == "link")
            .and_then(|link| link["state"].as_str().map(str::to_string))
    }

    /// The job as the room stream holds it now.
    fn job(&self, job_id: &str) -> Value {
        self.log
            .load(&StreamId::Room)
            .into_iter()
            .rfind(|event| event["kind"] == "schedule" && event["id"] == job_id)
            .unwrap_or_default()
    }

    /// The lines the model was handed that came from this job's thread.
    fn heard(&self, wanted: &str) -> Vec<String> {
        self.heard
            .lock()
            .unwrap()
            .iter()
            .filter(|line| line.contains(wanted))
            .cloned()
            .collect()
    }

    /// The teammate's job listing, as a client subscribed to it is sent it.
    async fn listing(&mut self) -> Vec<Value> {
        let sub = self
            .client
            .subscribe(json!({"schedules": self.persona_id}))
            .await;
        let frame = self
            .client
            .next_where(PATIENCE, |frame| frame["sub"] == sub)
            .await;
        frame["snapshot"].as_array().cloned().unwrap_or_default()
    }

    /// A job made over the wire, firing in a moment.
    async fn once(&mut self, prompt: &str, extra: Value) -> String {
        let mut params = json!({
            "personaId": self.persona_id,
            "kind": "schedule",
            "when": now() + 1_200,
            "prompt": prompt,
        });
        for (key, value) in extra.as_object().cloned().unwrap_or_default() {
            params[key] = value;
        }
        let job = self.client.call("schedule.create", params).await;
        assert_eq!(job["ok"], true, "{job}");
        job["result"]["id"].as_str().unwrap().to_owned()
    }
}

/// Each firing of a loop is a turn on the job's own thread, never the home
/// thread: the DM holds only what it reported, posted as the teammate's
/// message from the job. The job is parked between firings, its agent let go
/// of, and each firing starts from the note its last turn left, not from the
/// thread. A loop with a count ends when it runs out.
#[tokio::test(flavor = "multi_thread")]
async fn each_firing_is_a_turn_on_the_jobs_thread_parked_between_and_started_from_its_note() {
    let mut desk = seeded(|ada| {
        let mut job = looping("crane-loop", ada, 2_500, "SAY The crane is steady.");
        job["times"] = json!(2);
        vec![job]
    })
    .await;
    // Between the firings, the thread says it is parked.
    until("the first firing is over and the job parked", || {
        desk.kinds("crane-loop") == ["user", "agent", "turn"]
            && desk.state("crane-loop").as_deref() == Some("parked")
    })
    .await;
    until("the loop has fired twice", || {
        desk.kinds("crane-loop") == ["user", "agent", "turn", "user", "agent", "turn"]
    })
    .await;
    let thread = desk.thread("crane-loop");
    assert_eq!(thread[0]["text"], "SAY The crane is steady.");
    assert_eq!(thread[0]["scheduled"]["jobId"], "crane-loop");
    assert_eq!(thread[1]["text"], "The crane is steady.");

    // The firings were heard on the job's thread, the second from the first's
    // note.
    let heard = desk.heard("The crane is steady.");
    assert_eq!(heard.len(), 2, "{heard:#?}");
    assert!(!heard[0].contains("Where this job stood"), "{}", heard[0]);
    assert!(
        heard[1].contains(
            "Where this job stood after its last turn, in your own words:\nThe crane is steady."
        ),
        "{}",
        heard[1]
    );

    // The DM holds the two reports and nothing else: no line of the job's
    // turns, and no turn of the home agent's.
    until("both reports are in the chat", || desk.chat().len() == 2).await;
    let tape = desk.tape();
    assert!(
        tape.iter()
            .all(|event| !matches!(event["kind"].as_str(), Some("user" | "turn" | "thought"))),
        "{tape:#?}"
    );
    for posted in desk.chat() {
        assert_eq!(posted["text"], "The crane is steady.");
        assert_eq!(
            posted["from"],
            json!({"thread": "crane-loop", "kind": "job"})
        );
        assert_eq!(posted["scheduled"]["jobId"], "crane-loop");
    }

    // Its count ran out: it is ended, still listed, and its thread is kept.
    until("the loop has ended", || {
        desk.state("crane-loop").as_deref() == Some("closed")
    })
    .await;
    let listed = desk.listing().await;
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0]["name"], "SAY The crane is steady");
    assert!(listed[0]["endedAt"].is_i64(), "{listed:?}");
    assert_eq!(listed[0]["lastPost"]["text"], "The crane is steady.");
    assert!(listed[0]["nextAt"].is_i64());
}

/// A firing still running when the next is due is skipped, the skip is said on
/// the job's thread with an interval that fits, and a job that keeps
/// overrunning says so to the person once.
#[tokio::test(flavor = "multi_thread")]
async fn an_overlapping_firing_is_skipped_and_said_on_the_jobs_thread() {
    let desk = seeded(|ada| {
        let mut job = looping("slow-loop", ada, 1_200, "STALL SAY Checked, slowly.");
        job["quiet"] = json!(true);
        vec![job]
    })
    .await;

    until("two firings were skipped", || {
        desk.thread("slow-loop")
            .iter()
            .filter(|event| {
                event["kind"] == "notice"
                    && event["text"]
                        .as_str()
                        .is_some_and(|text| text.starts_with("Skipped the"))
            })
            .count()
            >= 2
    })
    .await;
    let skip = desk
        .thread("slow-loop")
        .into_iter()
        .find(|event| event["kind"] == "notice")
        .unwrap();
    let said = skip["text"].as_str().unwrap();
    assert!(
        said.contains("the one before it was still going after") && said.contains("would fit"),
        "{said}"
    );
    until("the person was told it keeps overrunning", || {
        !desk.chat().is_empty()
    })
    .await;
    let told = desk.chat();
    assert_eq!(told.len(), 1, "told once: {told:#?}");
    let told = told[0]["text"].as_str().unwrap();
    assert!(told.contains("keeps running past its"), "{told}");
    assert!(told.contains("would fit"), "{told}");
    assert_eq!(
        desk.heard("Checked, slowly.").len(),
        1,
        "the skipped firings never reached the agent"
    );
}

/// The person cancels a job: it ends with a final post of where it stood,
/// is listed as ended, and its thread is closed and kept.
#[tokio::test(flavor = "multi_thread")]
async fn a_job_the_person_cancels_ends_with_a_final_post() {
    let mut desk = seeded(|ada| {
        let mut job = looping("watch", ada, 60_000, "SAY Watching the crane: steady.");
        job["quiet"] = json!(true);
        vec![job]
    })
    .await;
    until("the job has fired and parked", || {
        desk.state("watch").as_deref() == Some("parked") && desk.kinds("watch").len() == 3
    })
    .await;
    assert!(desk.chat().is_empty(), "a quiet job said nothing yet");

    let cancelled = desk
        .client
        .call("schedule.cancel", json!({"id": "watch"}))
        .await;
    assert_eq!(cancelled["ok"], true, "{cancelled}");
    let chat = desk.chat();
    assert_eq!(chat.len(), 1, "{chat:#?}");
    assert_eq!(
        chat[0]["text"],
        "Cancelled. Where it stood: Watching the crane: steady."
    );
    assert_eq!(chat[0]["from"], json!({"thread": "watch", "kind": "job"}));
    assert_eq!(desk.state("watch").as_deref(), Some("closed"));
    let listed = desk.listing().await;
    assert!(listed[0]["endedAt"].is_i64(), "{listed:?}");

    let threads = desk
        .client
        .call("thread.list", json!({"personaId": desk.persona_id}))
        .await;
    let job = threads["result"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["thread"] == json!({"kind": "job", "key": "watch"}))
        .cloned()
        .unwrap_or_else(|| panic!("the job's thread is listed: {threads}"));
    assert_eq!(job["state"], "closed", "{job}");
    assert_eq!(job["end"], "person", "{job}");
}

/// A job that meets its goal ends itself: its report is its final post, and
/// it is ended.
#[tokio::test(flavor = "multi_thread")]
async fn a_job_that_meets_its_goal_ends_itself_with_its_report() {
    let desk = seeded(|ada| {
        let mut job = looping(
            "order",
            ada,
            60_000,
            r#"CALL cancel_schedule {"report":"The order shipped on Tuesday."}"#,
        );
        job["goal"] = json!("the order has shipped");
        vec![job]
    })
    .await;
    until("the job has ended itself", || {
        desk.state("order").as_deref() == Some("closed")
    })
    .await;
    let heard = desk.heard("cancel_schedule");
    assert!(
        heard[0].contains("The job's goal: the order has shipped."),
        "{heard:#?}"
    );
    let chat = desk.chat();
    assert_eq!(chat.len(), 1, "only the report: {chat:#?}");
    assert_eq!(chat[0]["text"], "The order shipped on Tuesday.");
    assert_eq!(chat[0]["scheduled"]["jobId"], "order");
    assert!(desk.job("order")["endedAt"].is_i64());
    until("the turn that ended it is written", || {
        desk.kinds("order").last().map(String::as_str) == Some("turn")
    })
    .await;
}

/// A plain reminder does no work: it fires as one post to the home thread,
/// stamped with its job, with no arm and no thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_plain_reminder_is_one_post_and_no_arm() {
    let mut desk = fresh().await;
    let id = desk
        .once("Call Sam about the crane.", json!({"reminder": true}))
        .await;
    until("the reminder is posted", || !desk.chat().is_empty()).await;
    let chat = desk.chat();
    assert_eq!(chat.len(), 1);
    assert_eq!(chat[0]["text"], "Call Sam about the crane.");
    assert_eq!(chat[0]["scheduled"]["jobId"], id.as_str());
    assert!(chat[0].get("from").is_none(), "{:?}", chat[0]);
    assert!(desk.heard.lock().unwrap().is_empty(), "no agent was asked");
    assert!(desk.thread(&id).is_empty(), "a reminder has no thread");
    let listed = desk.listing().await;
    assert_eq!(listed[0]["reminder"], true);
    assert!(listed[0]["endedAt"].is_i64());
}

/// The person can talk to a job's thread: their line is a turn on it, the
/// job's agent built afresh and told where it stood, and nothing reaches the
/// DM.
#[tokio::test(flavor = "multi_thread")]
async fn the_person_can_talk_to_a_jobs_thread() {
    let mut desk = seeded(|ada| {
        let mut job = looping("inbox", ada, 60_000, "SAY Two new orders.");
        job["quiet"] = json!(true);
        vec![job]
    })
    .await;
    until("the job has fired and parked", || {
        desk.state("inbox").as_deref() == Some("parked") && desk.kinds("inbox").len() == 3
    })
    .await;

    let said = desk
        .client
        .call(
            "thread.prompt",
            json!({
                "thread": {"kind": "job", "key": "inbox"},
                "text": "SAY Skipping the next check, as you asked.",
            }),
        )
        .await;
    assert_eq!(said["ok"], true, "{said}");
    until("the job answered in its thread", || {
        desk.kinds("inbox") == ["user", "agent", "turn", "user", "agent", "turn"]
    })
    .await;
    let thread = desk.thread("inbox");
    assert_eq!(
        thread[3]["text"],
        "SAY Skipping the next check, as you asked."
    );
    assert!(thread[3].get("scheduled").is_none(), "{:?}", thread[3]);
    assert_eq!(thread[4]["text"], "Skipping the next check, as you asked.");
    let heard = desk.heard("Skipping the next check");
    assert!(
        heard[0].contains("Two new orders."),
        "the fresh agent is told where the job stood: {heard:#?}"
    );
    assert!(desk.tape().is_empty(), "{:?}", desk.tape());
}

/// A schedule the previous build wrote, in its shape, keeps firing across the
/// upgrade: on its own job thread, not the DM, and heard only by what it
/// posts.
#[tokio::test(flavor = "multi_thread")]
async fn a_schedule_written_before_the_upgrade_fires_on_its_job_thread() {
    let desk = seeded(|ada| {
        vec![json!({
            "kind": "schedule",
            "id": "before-the-upgrade",
            "personaId": ada,
            "every": 3_600_000,
            "prompt": r#"CALL post_to_chat {"text":"The crane is down: the winch cable snapped."}"#,
            "quiet": true,
            "operatorCreated": true,
            "nextAt": now() - 60_000,
            "createdAt": now() - 7_200_000,
        })]
    })
    .await;
    until("the old job posted", || !desk.chat().is_empty()).await;
    let posted = &desk.chat()[0];
    assert_eq!(
        posted["text"],
        "The crane is down: the winch cable snapped."
    );
    assert_eq!(
        posted["from"],
        json!({"thread": "before-the-upgrade", "kind": "job"})
    );
    assert_eq!(posted["scheduled"]["quiet"], true);
    until("its turn is on its own thread", || {
        let kinds = desk.kinds("before-the-upgrade");
        kinds.first().map(String::as_str) == Some("user")
            && kinds.iter().any(|kind| kind == "tool")
            && kinds.last().map(String::as_str) == Some("turn")
    })
    .await;
    assert!(
        desk.tape().iter().all(|event| event["kind"] == "agent"),
        "nothing of its turn is on the DM: {:?}",
        desk.tape()
    );
    assert!(desk.job("before-the-upgrade")["nextAt"].as_i64().unwrap() > now());
}

/// A quiet job is heard only by posting, and what it posts is its note as it
/// wrote it: the teammate's message in the chat, stamped with its job, read
/// off the wire. A quiet job that finds nothing puts nothing in the chat; its
/// words stay on its thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_quiet_job_is_heard_only_when_it_posts() {
    let mut desk = fresh().await;
    let tape = desk
        .client
        .subscribe(json!({"tape": desk.persona_id}))
        .await;

    let first = desk.once("check the crane", json!({"quiet": true})).await;
    until("the first job's turn ended", || {
        desk.kinds(&first) == ["user", "agent", "turn"]
    })
    .await;
    assert_eq!(desk.thread(&first)[1]["text"], "No change.");

    let second = desk
        .once(
            r#"check the crane and CALL post_to_chat {"text":"The crane is down: the winch cable snapped."}"#,
            json!({"quiet": true}),
        )
        .await;
    let posted = desk
        .client
        .next_where(PATIENCE, |frame| {
            frame["sub"] == tape && frame["event"]["kind"] == "agent"
        })
        .await;
    let posted = &posted["event"];
    assert_eq!(
        posted["text"], "The crane is down: the winch cable snapped.",
        "the note is posted as the teammate wrote it"
    );
    assert_eq!(posted["scheduled"]["jobId"], second.as_str());
    assert_eq!(posted["scheduled"]["quiet"], true);
    assert_eq!(posted["from"], json!({"thread": second, "kind": "job"}));
    let chat = desk.chat();
    assert_eq!(chat.len(), 1, "nothing else reached the chat: {chat:#?}");
}

/// The `loop` tool writes the one schema: the task, the interval, the report
/// policy and the end it asked for, all on the job the person can list.
#[tokio::test(flavor = "multi_thread")]
async fn the_loop_tool_writes_a_job_with_its_report_policy_and_end() {
    let mut desk = fresh().await;
    let started = desk
        .client
        .call("session.start", json!({"personaId": desk.persona_id}))
        .await;
    assert_eq!(started["ok"], true, "{started}");
    let asked = desk
        .client
        .call(
            "session.prompt",
            json!({
                "personaId": desk.persona_id,
                "text": r#"CALL loop {"every":"30m","prompt":"Check the order page.","quiet":true,"goal":"the order has shipped","times":10}"#,
            }),
        )
        .await;
    assert_eq!(asked["ok"], true, "{asked}");
    until("the job is written", || {
        desk.log
            .load(&StreamId::Room)
            .iter()
            .any(|event| event["kind"] == "schedule")
    })
    .await;
    let listed = desk.client.call("schedule.list", json!({})).await;
    let job = &listed["result"][0];
    assert_eq!(job["kind"], "loop", "{listed}");
    assert_eq!(job["every"], 30 * 60_000);
    assert_eq!(job["prompt"], "Check the order page.");
    assert_eq!(job["quiet"], true);
    assert_eq!(job["goal"], "the order has shipped");
    assert_eq!(job["times"], 10);
    assert_eq!(job["operatorCreated"], false);
}

#[path = "schedule/clones.rs"]
mod clones;
