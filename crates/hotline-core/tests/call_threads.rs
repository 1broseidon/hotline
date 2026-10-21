mod call_threads {
    mod live_work;
    mod reader;
}
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
    let said = said
        .split("The whole call so far (conversation data):")
        .next()
        .unwrap_or(said);
    if said.contains("Like we said on the call") {
        let key = said
            .rsplit_once("(thread call:")
            .expect("the home agent is handed the call key")
            .1
            .split(')')
            .next()
            .unwrap();
        return vec![(
            "read_thread".into(),
            json!({"thread":format!("call:{key}")}).to_string(),
        )];
    }
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
            chunk(
                json!({"role":"assistant","content": if text_of(&messages[asked_at]).contains("harbour-plan.txt") { "Saving the beacon plan." } else { "" },"tool_calls":[call]}),
                None,
            ),
            chunk(json!({}), Some("tool_calls")),
        ]);
    }
    let said = text_of(&messages[asked_at]);
    let words = if let Some(reply) = said
        .lines()
        .find_map(|line| line.trim().strip_prefix("REPLY_JSON "))
    {
        serde_json::from_str::<String>(reply).expect("REPLY_JSON is a JSON string")
    } else if said.contains("ECHO_HISTORY") {
        messages.iter().map(text_of).collect::<Vec<_>>().join("\n")
    } else if asked.is_empty() {
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
    root: TempDir,
    requests: Arc<std::sync::Mutex<Vec<Value>>>,
    spoken: Arc<std::sync::atomic::AtomicUsize>,
}

async fn open() -> Desk {
    open_with_front(None).await
}

async fn open_with_front(realtime: Option<Arc<dyn hotline_voice::realtime::Realtime>>) -> Desk {
    let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = requests.clone();
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(move |body: axum::body::Bytes| {
            let captured = captured.clone();
            async move {
                let request: Value = serde_json::from_slice(&body).unwrap_or_default();
                captured.lock().unwrap().push(request.clone());
                if request["stream"] != true {
                    let note = if request.to_string().contains("Summarise this finished reply") {
                        if request.to_string().contains("REFUSE_REPLY_SUMMARY") { json!({"note":"too long ".repeat(200)}) } else { json!({"note":"Eddystone lighthouse history: the account covers the successive towers and how each design improved resistance to the sea."}) }
                    } else {
                        json!({"title":"Harbour plan", "note":"Harbour plan: agreed on the beacon repair."})
                    }.to_string();
                    return ([("Content-Type", "application/json")], json!({"id":"note","object":"chat.completion","created":1,"model":"vendor/coder","choices":[{"index":0,"message":{"role":"assistant","content":note},"finish_reason":"stop"}]}).to_string());
                }
                let messages = request["messages"].as_array().unwrap();
                if messages.last().is_some_and(|message| message["role"] == "tool") {
                    let line = messages.iter().rev().find(|message| message["role"] == "user").map(text_of).unwrap_or_default();
                    if let Some(path) = line.lines().find_map(|line| line.trim().strip_prefix("WAIT_FOR ")) {
                        while !std::path::Path::new(path).exists() { tokio::time::sleep(Duration::from_millis(20)).await; }
                    }
                }
                ([("Content-Type", "text/event-stream")], model(&request))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let root = tempfile::tempdir().unwrap();
    acp_fixture::install(root.path());
    let workspace = root.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let spoken = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let speech = Arc::new(FakeSpeech(spoken.clone()));
    let desk = hotline_core::desk::Desk::open_with_voice_services(
        root.path(),
        crate::common::store(),
        Some(hotline_core::voice::Services {
            speech: hotline_voice::speech::SpeechSet {
                stt: speech.clone(),
                tts: speech,
                fallback_tts: None,
            },
            dispatcher: None,
            realtime,
        }),
    )
    .unwrap();
    let log = desk.log.clone();
    let parts = desk.parts();
    let door = Door::bind(desk.log.clone(), TOKEN.to_string(), Arc::new(desk), parts).unwrap();
    let port = door.port();
    tokio::spawn(door.run());
    let mut client = Client::connect(port, TOKEN).await;
    let hello = client
        .call("client.hello", json!({"capabilities":["threads2"]}))
        .await;
    assert_eq!(hello["ok"], true);
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
        root,
        requests,
        spoken,
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
    /// the snapshot the subscription opened with. A turn that ended before
    /// the subscription arrives whole in its snapshot, so the snapshot goes
    /// back to the inbox less the event taken, for the next wait to read.
    async fn event(&mut self, sub: i64, wanted: impl Fn(&Value) -> bool) -> Value {
        let mut frame = self
            .client
            .next_where(PATIENCE, |frame| {
                frame["sub"] == sub
                    && (wanted(&frame["event"])
                        || frame["snapshot"]
                            .as_array()
                            .is_some_and(|events| events.iter().any(&wanted)))
            })
            .await;
        let Some(events) = frame["snapshot"].as_array_mut() else {
            return frame["event"].clone();
        };
        let at = events.iter().position(&wanted).unwrap();
        let event = events.remove(at);
        self.client.inbox.push_front(frame);
        event
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

struct FakeSpeech(Arc<std::sync::atomic::AtomicUsize>);
#[async_trait::async_trait]
impl hotline_voice::speech::Speech for FakeSpeech {
    fn id(&self) -> hotline_voice::speech::SpeechId {
        hotline_voice::speech::SpeechId {
            provider_id: "test".into(),
            model_id: "fake".into(),
            voice: None,
        }
    }
    async fn transcribe(
        &self,
        _: hotline_voice::speech::Clip,
    ) -> Result<String, hotline_voice::speech::SpeechError> {
        Ok("Harbour plan".into())
    }
    async fn speak(
        &self,
        _: &str,
    ) -> Result<hotline_voice::speech::Clip, hotline_voice::speech::SpeechError> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(hotline_voice::speech::Clip {
            mime: "audio/wav".into(),
            bytes: include_bytes!("fixtures/voice/acknowledgement.wav").to_vec(),
        })
    }
}

impl Desk {
    async fn call_on(&mut self, persona: &str, thread: Option<Value>) -> (String, Value, i64) {
        let call = uuid::Uuid::new_v4().to_string();
        let mut params = json!({"callId":call,"personaId":persona,"inputMode":"text"});
        if let Some(thread) = thread {
            params["thread"] = thread;
        }
        let thread = params
            .get("thread")
            .cloned()
            .unwrap_or_else(|| json!({"kind":"call","key":call}));
        let result = self.client.call("voice.call_start", params).await;
        assert_eq!(result["ok"], true, "{result}");
        let sub = self.client.subscribe(json!({"threadId":thread})).await;
        (call, thread, sub)
    }
    async fn say(&mut self, call: &str, seq: u32, text: &str) {
        let result = self
            .client
            .call("voice.text", json!({"callId":call,"seq":seq,"text":text}))
            .await;
        assert_eq!(result["ok"], true, "{result}");
    }
    async fn hang_up(&mut self, call: &str, sub: i64) {
        let result = self
            .client
            .call("voice.call_end", json!({"callId":call}))
            .await;
        assert_eq!(result["ok"], true, "{result}");
        self.event(sub, |event| {
            event["kind"] == "notice"
                && event["text"]
                    .as_str()
                    .is_some_and(|s| s.starts_with("Voice disconnected"))
        })
        .await;
    }
    async fn advance(&self, seconds: u64) {
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(seconds)).await;
        tokio::time::resume();
    }
    async fn reply(&mut self, sub: i64) -> Value {
        self.event(sub, |event| event["kind"] == "agent").await
    }
    async fn finished(&mut self, sub: i64) {
        self.event(sub, |event| event["kind"] == "turn").await;
    }
    async fn closing(&mut self, tape: i64) -> Value {
        self.event(tape, |event| {
            event["kind"] == "agent"
                && event["text"] == "Harbour plan: agreed on the beacon repair."
        })
        .await
    }
    async fn type_in(&mut self, thread: &Value, text: &str) {
        let result = self
            .client
            .call("thread.prompt", json!({"thread":thread,"text":text}))
            .await;
        assert_eq!(result["ok"], true, "{result}");
    }
    fn stream(&self, thread: &Value) -> Vec<Value> {
        let thread: hotline_core::contract::ThreadId =
            serde_json::from_value(thread.clone()).unwrap();
        self.log.load(&StreamId::of_thread(&thread).unwrap())
    }
    async fn gate(&mut self, call: &str, sub: i64) -> std::path::PathBuf {
        let gate = self.root.path().join("release-turn");
        self.say(
            call,
            1,
            &format!(
                "CALL request_human {{\"reason\":\"Beacon approval\"}}\nWAIT_FOR {}",
                gate.display()
            ),
        )
        .await;
        self.event(sub, |event| event["kind"] == "human_action")
            .await;
        gate
    }
}

#[tokio::test]
async fn legacy_utterances_are_on_the_calls_thread_never_the_dm() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, _) = desk.teammate("Ada", driver, false).await;
        let (call, thread, sub) = desk.call_on(&ada, None).await;
        desk.say(&call, 1, "Harbour plan").await;
        desk.reply(sub).await;
        desk.finished(sub).await;
        assert!(
            desk.stream(&thread)
                .iter()
                .any(|event| event["kind"] == "user" && event["text"] == "Harbour plan")
        );
        assert!(
            !desk
                .log
                .load(&StreamId::Tape(ada))
                .iter()
                .any(|event| event["kind"] == "user")
        );
        desk.hang_up(&call, sub).await;
    }
}

#[tokio::test]
async fn hanging_up_while_work_runs_finishes_in_the_chat_without_speech() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, tape) = desk.teammate("Ada", driver, false).await;
        let (call, thread, sub) = desk.call_on(&ada, None).await;
        let gate = desk.gate(&call, sub).await;
        desk.hang_up(&call, sub).await;
        let spoken = desk.spoken.load(std::sync::atomic::Ordering::SeqCst);
        std::fs::write(gate, "finish").unwrap();
        let post = desk.reply(tape).await;
        assert_eq!(post["from"]["thread"], thread["key"]);
        assert!(post["text"].as_str().unwrap().contains("request_human"));
        desk.finished(sub).await;
        assert_eq!(
            desk.spoken.load(std::sync::atomic::Ordering::SeqCst),
            spoken
        );
    }
}

#[tokio::test]
async fn picking_up_before_old_work_finishes_still_posts_its_reply() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, tape) = desk.teammate("Ada", driver, false).await;
        let (call, thread, sub) = desk.call_on(&ada, None).await;
        let gate = desk.gate(&call, sub).await;
        desk.hang_up(&call, sub).await;
        let (_next, same, _) = desk.call_on(&ada, Some(thread.clone())).await;
        assert_eq!(same, thread);
        std::fs::write(gate, "finish").unwrap();
        let post = desk.reply(tape).await;
        assert_eq!(post["from"]["thread"], thread["key"]);
        assert!(post["text"].as_str().unwrap().contains("request_human"));
    }
}

#[tokio::test]
async fn closing_note_waits_thirty_seconds_and_for_finished_work_then_parks() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, tape) = desk.teammate("Ada", driver, false).await;
        let roster = desk.client.subscribe(json!({"view":"roster"})).await;
        let (call, thread, sub) = desk.call_on(&ada, None).await;
        let gate = desk.gate(&call, sub).await;
        desk.client
            .next_where(PATIENCE, |frame| {
                frame["sub"] == roster
                    && frame["event"]["persona"]["id"] == ada
                    && frame["event"]["workingCalls"] == json!([thread["key"]])
            })
            .await;
        desk.hang_up(&call, sub).await;
        desk.advance(29).await;
        assert!(
            !desk
                .chat(&ada)
                .iter()
                .any(|event| event["text"] == "Harbour plan: agreed on the beacon repair.")
        );
        desk.advance(2).await;
        assert!(
            !desk
                .requests
                .lock()
                .unwrap()
                .iter()
                .any(|request| request["stream"] != true)
        );
        // Discard earlier idle rows so only the turn's completion can satisfy this.
        desk.client.inbox.retain(|frame| frame["sub"] != roster);
        std::fs::write(gate, "finish").unwrap();
        desk.client
            .next_where(PATIENCE, |frame| {
                frame["sub"] == roster
                    && frame["event"]["persona"]["id"] == ada
                    && frame["event"].get("workingCalls").is_none()
            })
            .await;
        desk.closing(tape).await;
        desk.event(sub, |event| {
            event["kind"] == "link" && event["state"] == "parked"
        })
        .await;
        assert!(
            desk.requests
                .lock()
                .unwrap()
                .iter()
                .filter(|request| request["stream"] != true)
                .any(|request| request.to_string().contains("RESULTS"))
        );
    }
}

#[tokio::test]
async fn in_the_chat_afterwards_is_one_summary_not_a_transcript_or_periodic_digest() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, tape) = desk.teammate("Ada", driver, false).await;
        let (call, thread, sub) = desk.call_on(&ada, None).await;
        desk.say(&call, 1, "Harbour plan").await;
        desk.finished(sub).await;
        desk.advance(301).await;
        assert_eq!(desk.chat(&ada), [] as [Value; 0]);
        desk.hang_up(&call, sub).await;
        desk.advance(31).await;
        let note = desk.closing(tape).await;
        assert_eq!(note["from"]["thread"], thread["key"]);
        assert_eq!(note["text"].as_str().unwrap().lines().count(), 1);
        assert!(!note["text"].as_str().unwrap().contains("You asked"));
        let requests = desk.requests.lock().unwrap();
        let summary = requests
            .iter()
            .find(|request| request["stream"] != true)
            .unwrap()
            .to_string();
        assert!(summary.contains("Never write a transcript"));
        assert!(summary.contains("only conversation, write one line"));
    }
}

#[tokio::test]
async fn picking_it_back_up_keeps_context_and_the_next_note_covers_only_new_work() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, tape) = desk.teammate("Ada", driver, false).await;
        let (call, thread, sub) = desk.call_on(&ada, None).await;
        desk.say(&call, 1, "BEACON_OLD_TOPIC").await;
        desk.finished(sub).await;
        desk.hang_up(&call, sub).await;
        desk.advance(31).await;
        desk.closing(tape).await;
        desk.event(sub, |event| {
            event["kind"] == "link" && event["state"] == "parked"
        })
        .await;
        desk.type_in(&thread, "ECHO_HISTORY").await;
        desk.finished(sub).await;
        assert!(desk.stream(&thread).iter().any(|event| {
            event["kind"] == "agent"
                && event["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("BEACON_OLD_TOPIC"))
        }));
        // The context check's reply itself quotes old history, so establish a
        // new closing boundary before testing an interval with only new words.
        let (next, _, next_sub) = desk.call_on(&ada, Some(thread.clone())).await;
        desk.hang_up(&next, next_sub).await;
        desk.advance(31).await;
        desk.closing(tape).await;
        desk.event(next_sub, |event| {
            event["kind"] == "link" && event["state"] == "parked"
        })
        .await;
        let (last, same, last_sub) = desk.call_on(&ada, Some(thread.clone())).await;
        assert_eq!(same, thread);
        desk.say(&last, 1, "BEACON_NEW_TOPIC").await;
        desk.finished(last_sub).await;
        desk.hang_up(&last, last_sub).await;
        desk.advance(31).await;
        desk.closing(tape).await;
        let requests = desk.requests.lock().unwrap();
        let summary = requests
            .iter()
            .rev()
            .find(|request| request["stream"] != true)
            .unwrap()
            .to_string();
        assert!(summary.contains("BEACON_NEW_TOPIC"));
        assert!(!summary.contains("BEACON_OLD_TOPIC"));
    }
}

#[tokio::test]
async fn calling_on_a_thread_under_way_queues_speech_on_that_same_thread() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, _) = desk.teammate("Ada", driver, false).await;
        let gate = desk.root.path().join("release-work");
        let opened = desk.client.call("thread.open", json!({"personaId":ada,"text":format!("CALL request_human {{\"reason\":\"Approve harbour plan\"}}\nWAIT_FOR {}",gate.display())})).await;
        assert_eq!(opened["ok"], true, "{opened}");
        let thread = opened["result"]["thread"].clone();
        let (call, same, sub) = desk.call_on(&ada, Some(thread.clone())).await;
        assert_eq!(thread, same);
        desk.event(sub, |event| event["kind"] == "human_action")
            .await;
        desk.say(
            &call,
            1,
            "CALL post_to_chat {\"text\":\"Harbour beacon plan is ready.\"}",
        )
        .await;
        desk.event(sub, |event| {
            event["kind"] == "user"
                && event["text"]
                    .as_str()
                    .is_some_and(|s| s.contains("post_to_chat"))
        })
        .await;
        assert!(
            !desk
                .chat(&ada)
                .iter()
                .any(|event| event["text"] == "Harbour beacon plan is ready.")
        );
        std::fs::write(gate, "finish").unwrap();
        desk.event(sub, |event| {
            event["kind"] == "agent"
                && event["text"]
                    .as_str()
                    .is_some_and(|s| s.contains("post_to_chat"))
        })
        .await;
        let post = desk
            .chat(&ada)
            .into_iter()
            .find(|event| event["text"] == "Harbour beacon plan is ready.")
            .unwrap();
        assert_eq!(post["from"]["kind"], "side");
        assert!(post["from"]["title"].is_string());
        assert!(
            desk.log
                .load(&StreamId::Tape(ada))
                .iter()
                .all(|event| event["kind"] != "link" || event["thread"]["kind"] != "call")
        );
        desk.hang_up(&call, sub).await;
    }
}

#[tokio::test]
async fn posts_are_quiet_on_the_call_and_notify_like_replies_after_hang_up() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, _) = desk.teammate("Ada", driver, false).await;
        let (call, thread, sub) = desk.call_on(&ada, None).await;
        desk.say(
            &call,
            1,
            "CALL post_to_chat {\"text\":\"Harbour beacon plan is ready.\"}",
        )
        .await;
        desk.finished(sub).await;
        let posts = desk.chat(&ada);
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0]["silent"], true);
        assert!(hotline_store::previews::preview(desk.log.root(), &ada).is_none());
        desk.hang_up(&call, sub).await;
        desk.type_in(
            &thread,
            "CALL post_to_chat {\"text\":\"Harbour beacon is repaired.\"}",
        )
        .await;
        desk.finished(sub).await;
        assert!(desk.chat(&ada).iter().any(
            |event| event["text"] == "Harbour beacon is repaired." && event["silent"].is_null()
        ));
        let preview = hotline_store::previews::preview(desk.log.root(), &ada).unwrap();
        assert_eq!(preview["posted"], true);
    }
}

#[tokio::test]
async fn replying_to_a_calls_post_goes_back_to_the_calls_thread() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, _) = desk.teammate("Ada", driver, false).await;
        let (call, thread, sub) = desk.call_on(&ada, None).await;
        desk.say(
            &call,
            1,
            "CALL post_to_chat {\"text\":\"Harbour beacon plan is ready.\"}",
        )
        .await;
        desk.finished(sub).await;
        let post = desk.chat(&ada).remove(0);
        desk.hang_up(&call, sub).await;
        let sent = desk
            .client
            .call(
                "session.prompt",
                json!({"personaId":ada,"replyTo":post["id"],"text":"Apply that beacon plan."}),
            )
            .await;
        assert_eq!(sent["ok"], true, "{sent}");
        desk.finished(sub).await;
        assert!(
            desk.stream(&thread)
                .iter()
                .any(|event| event["kind"] == "user" && event["text"] == "Apply that beacon plan.")
        );
        assert!(
            !desk
                .log
                .load(&StreamId::Tape(ada))
                .iter()
                .any(|event| event["kind"] == "user")
        );
    }
}

#[tokio::test]
async fn following_up_in_the_chat_hands_the_home_agent_the_calls_posts_and_note() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, tape) = desk.teammate("Ada", driver, false).await;
        let (call, thread, sub) = desk.call_on(&ada, None).await;
        desk.say(
            &call,
            1,
            "CALL post_to_chat {\"text\":\"Harbour beacon plan is ready.\"}",
        )
        .await;
        desk.finished(sub).await;
        desk.hang_up(&call, sub).await;
        desk.advance(31).await;
        desk.closing(tape).await;
        let sent = desk
            .client
            .call(
                "session.prompt",
                json!({"personaId":ada,"text":"ECHO_HISTORY"}),
            )
            .await;
        assert_eq!(sent["ok"], true, "{sent}");
        let reply = desk
            .event(tape, |event| {
                event["kind"] == "agent"
                    && event["from"].is_null()
                    && event["text"]
                        .as_str()
                        .is_some_and(|text| text.contains("ECHO_HISTORY"))
            })
            .await;
        let text = reply["text"].as_str().unwrap();
        assert!(text.contains("Harbour beacon plan is ready."));
        assert!(text.contains("Harbour plan: agreed on the beacon repair."));
        assert!(text.contains(thread["key"].as_str().unwrap()));
        assert!(
            desk.log
                .load(&StreamId::Tape(ada))
                .iter()
                .any(|event| event["kind"] == "user" && event["text"] == "ECHO_HISTORY")
        );
    }
}

#[tokio::test]
async fn posts_and_closing_notes_name_the_subject_for_someone_not_on_the_call() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, tape) = desk.teammate("Ada", driver, false).await;
        let (call, _, sub) = desk.call_on(&ada, None).await;
        desk.say(
            &call,
            1,
            "CALL post_to_chat {\"text\":\"Harbour beacon plan is ready.\"}",
        )
        .await;
        desk.finished(sub).await;
        desk.hang_up(&call, sub).await;
        desk.advance(31).await;
        desk.closing(tape).await;
        let requests = desk.requests.lock().unwrap();
        assert!(
            requests
                .iter()
                .filter(|request| request["stream"] != true)
                .any(|request| request.to_string().contains("someone who was not on it"))
        );
        if matches!(driver, Driver::HotlineAgent) {
            assert!(requests.iter().any(|request| {
                request
                    .to_string()
                    .contains("someone who was not on the call")
            }));
        }
    }
}

#[tokio::test]
async fn a_call_has_only_its_teammates_grants_and_cannot_attach_another_teammates_thread() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, _) = desk.teammate("Ada", driver, false).await;
        let (bea, _) = desk.teammate("Bea", driver, false).await;
        let opened = desk
            .client
            .call("thread.open", json!({"personaId":bea,"text":""}))
            .await;
        let refused = desk.client.call("voice.call_start", json!({"callId":uuid::Uuid::new_v4().to_string(), "personaId":ada, "thread":opened["result"]["thread"],"inputMode":"text"})).await;
        assert_eq!(refused["ok"], false, "{refused}");
        assert!(refused.to_string().contains("another teammate"));
        let (call, thread, sub) = desk.call_on(&ada, None).await;
        desk.say(&call, 1, "CALL schedule {\"when\":\"20m\",\"prompt\":\"Check the harbour beacon\"}\nCALL post_to_chat {\"text\":\"Harbour beacon needs a check.\"}").await;
        desk.finished(sub).await;
        let replies = desk
            .stream(&thread)
            .iter()
            .filter(|event| event["kind"] == "agent")
            .map(|event| event["text"].as_str().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(replies.contains("Background work"), "{replies}");
        assert!(
            desk.chat(&ada)
                .iter()
                .any(|event| event["text"] == "Harbour beacon needs a check.")
        );
        desk.hang_up(&call, sub).await;
    }
}

#[tokio::test]
async fn answering_a_card_after_hang_up_wakes_the_same_call_thread() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, tape) = desk.teammate("Ada", driver, false).await;
        let (call, thread, sub) = desk.call_on(&ada, None).await;
        desk.say(
            &call,
            1,
            "CALL request_human {\"reason\":\"Approve the beacon repair\"}",
        )
        .await;
        let card = desk
            .event(sub, |event| event["kind"] == "human_action")
            .await;
        desk.finished(sub).await;
        desk.hang_up(&call, sub).await;
        desk.advance(31).await;
        desk.closing(tape).await;
        desk.event(sub, |event| {
            event["kind"] == "link" && event["state"] == "parked"
        })
        .await;
        let result = desk.client.call("thread.answer", json!({"thread":thread, "answer":{"kind":"human", "actionId":card["actionId"], "status":"done", "note":"BEACON_APPROVED"}})).await;
        assert_eq!(result["ok"], true, "{result}");
        desk.finished(sub).await;
        assert!(desk.stream(&thread).iter().any(|event| {
            event["kind"] == "delivery"
                && event["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("BEACON_APPROVED"))
        }));
    }
}

#[tokio::test]
async fn picking_it_back_up_keeps_typed_turns_in_the_parked_calls_thread() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, tape) = desk.teammate("Ada", driver, false).await;
        let (call, thread, sub) = desk.call_on(&ada, None).await;
        desk.say(&call, 1, "Harbour plan").await;
        desk.finished(sub).await;
        assert_eq!(desk.chat(&ada), [] as [Value; 0]);
        desk.hang_up(&call, sub).await;
        desk.advance(31).await;
        desk.closing(tape).await;
        desk.event(sub, |event| {
            event["kind"] == "link" && event["state"] == "parked"
        })
        .await;
        let before = desk.chat(&ada);
        desk.type_in(&thread, "Make the lighthouse poem rhyme. Include https://example.com and a | table. ECHO_HISTORY").await;
        desk.finished(sub).await;
        assert_eq!(desk.chat(&ada), before);
        assert!(desk.stream(&thread).iter().any(|event| {
            event["kind"] == "agent"
                && event["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("Make the lighthouse poem rhyme"))
        }));
        let (next, _, next_sub) = desk.call_on(&ada, Some(thread.clone())).await;
        desk.say(&next, 1, "ECHO_HISTORY https://example.com | beacon")
            .await;
        desk.finished(next_sub).await;
        assert_eq!(desk.chat(&ada), before);
        desk.hang_up(&next, next_sub).await;
    }
}

#[tokio::test]
async fn hanging_up_during_a_long_reply_posts_one_subject_summary_and_keeps_the_full_text() {
    long_reply_after_hang_up(false).await;
}

#[tokio::test]
async fn hanging_up_with_an_unusable_summary_never_pastes_the_long_reply() {
    long_reply_after_hang_up(true).await;
}

async fn long_reply_after_hang_up(refuse_summary: bool) {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, tape) = desk.teammate("Ada", driver, false).await;
        let (call, thread, sub) = desk.call_on(&ada, None).await;
        let gate = desk.root.path().join("release-history");
        let marker = if refuse_summary {
            "REFUSE_REPLY_SUMMARY"
        } else {
            ""
        };
        let history = (1..=5)
            .map(|part| {
                format!(
                    "Eddystone history paragraph {part}: {}",
                    "The lighthouse protected ships from the dangerous rocks off the coast. "
                        .repeat(5)
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        desk.say(&call, 1, &format!("CALL request_human {{\"reason\":\"History approval\"}}\nWAIT_FOR {}\nREPLY_JSON {}", gate.display(), serde_json::to_string(&format!("<spoken>The account is ready.</spoken><written>{history} {marker}</written>")).unwrap())).await;
        desk.event(sub, |event| event["kind"] == "human_action")
            .await;
        desk.hang_up(&call, sub).await;
        std::fs::write(gate, "finish").unwrap();
        let post = desk.reply(tape).await;
        desk.finished(sub).await;
        assert_eq!(desk.chat(&ada).len(), 1);
        assert_eq!(post["from"]["kind"], "call");
        assert_eq!(post["from"]["thread"], thread["key"]);
        let text = post["text"].as_str().unwrap();
        if refuse_summary {
            assert!(text.contains("could not prepare its short summary"));
        } else {
            assert!(text.starts_with("Eddystone lighthouse history:"));
        }
        assert!(text.len() < 600);
        assert!(!text.contains("Eddystone history paragraph"));
        let full = desk
            .stream(&thread)
            .iter()
            .filter(|event| event["kind"] == "agent")
            .filter_map(|event| event["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        for part in 1..=5 {
            assert!(
                full.contains(&format!("Eddystone history paragraph {part}:")),
                "{full}"
            );
        }
        let requests = desk.requests.lock().unwrap();
        let summary = requests
            .iter()
            .find(|request| {
                request
                    .to_string()
                    .contains("Summarise this finished reply")
            })
            .unwrap()
            .to_string();
        assert!(summary.contains("paragraph 5:"));
    }
}

#[tokio::test]
async fn an_untitled_calls_post_uses_the_call_label_instead_of_a_placeholder_title() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, _) = desk.teammate("Ada", driver, false).await;
        let (call, thread, sub) = desk.call_on(&ada, None).await;
        desk.say(
            &call,
            1,
            "CALL post_to_chat {\"text\":\"The beacon plan is ready.\"}",
        )
        .await;
        desk.finished(sub).await;
        let posts = desk.chat(&ada);
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0]["from"]["kind"], "call");
        assert_eq!(posts[0]["from"]["thread"], thread["key"]);
        assert!(posts[0]["from"]["title"].is_null());
        desk.hang_up(&call, sub).await;
    }
}

#[tokio::test]
async fn hanging_up_catches_a_typed_turn_already_running_when_voice_attached() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, tape) = desk.teammate("Ada", driver, false).await;
        let gate = desk.root.path().join("release-typed-reply");
        let reply = "Eddystone history is ready.\n\nThe account covers four successive towers.";
        let opened = desk.client.call("thread.open", json!({"personaId":ada,"text":format!("CALL request_human {{\"reason\":\"History approval\"}}\nWAIT_FOR {}\nREPLY_JSON {}", gate.display(), serde_json::to_string(reply).unwrap())})).await;
        assert_eq!(opened["ok"], true);
        let thread = opened["result"]["thread"].clone();
        let work_sub = desk.client.subscribe(json!({"threadId":thread})).await;
        desk.event(work_sub, |event| event["kind"] == "human_action")
            .await;
        let (call, _, sub) = desk.call_on(&ada, Some(thread.clone())).await;
        desk.hang_up(&call, sub).await;
        std::fs::write(gate, "finish").unwrap();
        let post = desk.reply(tape).await;
        desk.finished(sub).await;
        assert_eq!(post["text"], reply);
        assert_eq!(desk.chat(&ada).len(), 1);
        assert_eq!(post["from"]["thread"], thread["key"]);
        assert!(
            !desk
                .requests
                .lock()
                .unwrap()
                .iter()
                .any(|request| request["stream"] != true)
        );
    }
}

/// Interrupting: the cut survives on the thread and reaches both drivers,
/// while the caller’s saved words remain their own.
#[tokio::test]
async fn interrupted_speech_is_kept_and_the_next_turn_hears_the_cut() {
    for driver in [Driver::HotlineAgent, Driver::Acp] {
        let mut desk = open().await;
        let (ada, _) = desk.teammate("Ada", driver, false).await;
        let (call, thread, sub) = desk.call_on(&ada, None).await;
        desk.say(
            &call,
            1,
            "Explain the harbour.\nREPLY_JSON \"Harbour plan.\"",
        )
        .await;
        desk.reply(sub).await;
        desk.finished(sub).await;
        let result = desk
            .client
            .call("voice.interrupt", json!({"callId":call}))
            .await;
        assert_eq!(result["ok"], true, "{result}");
        let cut = desk.event(sub, |event| event["interrupted"] == true).await;
        assert!(
            cut["text"].as_str().unwrap().contains("Harbour plan"),
            "{cut}"
        );
        desk.say(&call, 2, "Actually, focus on the beacon. ECHO_HISTORY")
            .await;
        desk.finished(sub).await;
        // Echoed history can contain voice tags and becomes several agent
        // events. Inspect the whole new turn, not only its first bubble.
        let replies = desk
            .stream(&thread)
            .into_iter()
            .skip_while(|event| event["id"] != cut["id"])
            .filter(|event| event["kind"] == "agent")
            .filter_map(|event| event["text"].as_str().map(str::to_string))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            replies.contains("Actually, focus on the beacon. ECHO_HISTORY"),
            "{replies}"
        );
        assert!(replies.contains("do not resume"), "{replies}");
        assert!(desk.stream(&thread).iter().any(|event| {
            event["kind"] == "user"
                && event["text"] == "Actually, focus on the beacon. ECHO_HISTORY"
        }));
        desk.hang_up(&call, sub).await;
    }
}
