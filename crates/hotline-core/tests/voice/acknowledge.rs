//! The desk's acknowledgement while a teammate thinks, over the wire with a
//! real teammate session: the turn reaches the teammate's model before the
//! line is written, the turn is told the desk acknowledges it and asked for
//! no opening of its own, and with the setting off the call is as it always
//! was. A desk that never set it acknowledges, and one with no call
//! assistant leaves the turn as it always was. A teammate that then works in
//! silence has one line said on what it is doing, from the step its session
//! took.
use super::*;
use hotline_core::voice::spoken::{ACKNOWLEDGED_CONTRACT, CONTRACT};
use std::time::Instant;

const LINE: &str = "Looking into the failing PR.";

/// A contract as it reads inside the JSON of the model's request.
fn as_sent(contract: &str) -> String {
    let quoted = serde_json::to_string(contract).unwrap();
    quoted[1..quoted.len() - 1].to_string()
}

/// What a progress line was asked with: what the teammate is doing, and
/// what was said aloud before it.
type Progressed = (String, Option<String>);

/// A call assistant that only acknowledges, with `line` after `delay`, and
/// notes when it answered; and says `progress` of a teammate working in
/// silence, noting what it was told the teammate is doing and had said.
struct Acknowledger {
    line: &'static str,
    delay: Duration,
    asked: Arc<AtomicUsize>,
    answered: Arc<std::sync::Mutex<Option<Instant>>>,
    progress: &'static str,
    progressed: Arc<std::sync::Mutex<Vec<Progressed>>>,
}

impl Acknowledger {
    fn new(line: &'static str, delay: Duration, progress: &'static str) -> Self {
        Self {
            line,
            delay,
            asked: Arc::default(),
            answered: Arc::default(),
            progress,
            progressed: Arc::default(),
        }
    }
}

#[async_trait::async_trait]
impl Dispatcher for Acknowledger {
    fn id(&self) -> VoiceModel {
        ScriptDispatcher(Arc::default()).id()
    }
    async fn answer(&self, _: Context, _: &str, _: Arc<Budget>) -> Result<String, String> {
        unreachable!("a call to a teammate is answered by the teammate")
    }
    async fn narrate(&self, _: &str, _: &str, _: &str, _: Arc<Budget>) -> Result<String, String> {
        unreachable!("a call to a teammate narrates nothing")
    }
    async fn rewrite(&self, _: &str, _: &str, _: &str, _: Arc<Budget>) -> Result<String, String> {
        Err("No rewrite here.".into())
    }
    async fn acknowledge(&self, name: &str, words: &str, _: Arc<Budget>) -> Result<String, String> {
        assert_eq!(name, "Mack");
        assert_eq!(words, "ask Mack to check the failing PR");
        self.asked.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        *self.answered.lock().unwrap() = Some(Instant::now());
        Ok(self.line.into())
    }
    async fn progress(
        &self,
        name: &str,
        words: &str,
        doing: &str,
        said: Option<&str>,
        _: Arc<Budget>,
    ) -> Result<String, String> {
        assert_eq!(name, "Mack");
        assert_eq!(words, "ask Mack to check the failing PR");
        self.progressed
            .lock()
            .unwrap()
            .push((doing.into(), said.map(str::to_string)));
        Ok(self.progress.into())
    }
}

/// What a fixture model was sent, and when.
type Requests = Arc<std::sync::Mutex<Vec<(Instant, Value)>>>;

/// The stream chunks of a reply that says `text`.
fn text_chunks(text: &str) -> Vec<Value> {
    vec![
        json!({"id":"slow","object":"chat.completion.chunk","created":1,"model":"test","choices":[{"index":0,"delta":{"role":"assistant","content":text},"finish_reason":null}]}),
        json!({"id":"slow","object":"chat.completion.chunk","created":1,"model":"test","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}),
    ]
}

/// The stream chunks of a reply that reads `path`, with no words before it.
fn read_chunks(path: &str) -> Vec<Value> {
    let arguments = json!({ "path": path }).to_string();
    vec![
        json!({"id":"slow","object":"chat.completion.chunk","created":1,"model":"test","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_read","type":"function","function":{"name":"read","arguments":arguments}}]},"finish_reason":null}]}),
        json!({"id":"slow","object":"chat.completion.chunk","created":1,"model":"test","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}),
    ]
}

/// A teammate's model that answers its requests in turn from `answers`,
/// each after its delay (the last answers any later request), and keeps
/// what it was sent and when.
async fn model_answering(answers: Vec<(Duration, Vec<Value>)>) -> (String, Requests) {
    use axum::{Router, body::Bytes, routing::post};
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let requests = seen.clone();
    let answers = Arc::new(answers);
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |body: Bytes| {
            let requests = requests.clone();
            let answers = answers.clone();
            async move {
                let asked = {
                    let mut requests = requests.lock().unwrap();
                    if let Ok(request) = serde_json::from_slice::<Value>(&body) {
                        requests.push((Instant::now(), request));
                    }
                    requests.len()
                };
                let (delay, chunks) = &answers[asked.min(answers.len()) - 1];
                tokio::time::sleep(*delay).await;
                let body: String = chunks
                    .iter()
                    .map(|chunk| format!("data: {chunk}\n\n"))
                    .collect();
                (
                    [("Content-Type", "text/event-stream")],
                    format!("{body}data: [DONE]\n\n"),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, seen)
}

/// What one turn of a call to Mack said, in order, what Mack's model was
/// asked and when, and when the call assistant answered.
struct Turn {
    said: Vec<String>,
    asked: String,
    asked_at: Instant,
    acknowledgements: usize,
    answered_at: Option<Instant>,
}

/// A desk with `assistant` for its call assistant (none without one) and
/// acknowledging as `acknowledge` sets it (never set for `None`), Mack on
/// `model` working in `root`, and a device-text call to Mack: the socket,
/// what stops the desk's server, and the call.
async fn call_to_mack(
    acknowledge: Option<bool>,
    assistant: Option<Acknowledger>,
    model: &str,
    root: &std::path::Path,
) -> (Socket, tokio::task::AbortHandle, String) {
    let mut services = services();
    services.dispatcher = assistant.map(|assistant| Arc::new(assistant) as Arc<dyn Dispatcher>);
    let desk = Arc::new(
        hotline_core::desk::Desk::open_with_voice_services(root, common::store(), Some(services))
            .unwrap(),
    );
    let log = desk.log.clone();
    let parts = desk.parts();
    let door = Door::bind(log, "ack-test".into(), desk, parts).unwrap();
    let port = door.port();
    let server = tokio::spawn(door.run()).abort_handle();
    let (mut socket, _) = connect_async(format!("ws://127.0.0.1:{port}/ws?token=ack-test"))
        .await
        .unwrap();
    let saved = answer(&mut socket, 90, json!({"cmd":"credential.custom_save","params":{"draft":{"name":"Slow fixture","baseUrl":model,"api":"chat_completions","models":["test"],"secret":"fixture-key"}}})).await;
    assert_eq!(saved["ok"], true, "{saved}");
    if let Some(on) = acknowledge {
        let set = answer(
            &mut socket,
            89,
            json!({"cmd":"settings.update","params":{"patch":{"voice":{"acknowledge":on}}}}),
        )
        .await;
        assert_eq!(set["ok"], true, "{set}");
    }
    let made = answer(&mut socket, 91, json!({"cmd":"persona.create","params":{"draft":{"name":"Mack","goal":"Check the PR.","cwd":root.to_str().unwrap()}}})).await;
    let persona = made["result"]["id"].as_str().unwrap().to_string();
    let started = answer(
        &mut socket,
        92,
        json!({"cmd":"session.start","params":{"personaId":persona}}),
    )
    .await;
    assert_eq!(started["ok"], true, "{started}");
    let call = Uuid::new_v4().to_string();
    let started = answer(&mut socket, 1, json!({"cmd":"voice.call_start","params":{"callId":call,"personaId":persona,"inputMode":"text","streamAudio":true}})).await;
    assert_eq!(started["ok"], true, "{started}");
    send(&mut socket, json!({"id":2,"sub":{"call":call}})).await;
    frame_where(&mut socket, |f| f["snapshot"].is_array()).await;
    (socket, server, call)
}

/// What the waiting tone is heard as among the lines said.
const TONE: &str = "(the waiting tone)";

/// The person asks Mack to check the failing PR, their speech ending
/// `silence_ms` before the desk takes it; then everything said, and when,
/// and the waiting tone as [`TONE`], until the teammate's reply has been
/// said whole.
async fn ask_and_hear(socket: &mut Socket, call: &str, silence_ms: u32) -> Vec<(Instant, String)> {
    send(socket, json!({"id":3,"cmd":"voice.text","params":{"callId":call,"seq":1,"text":"ask Mack to check the failing PR","silenceMs":silence_ms}})).await;
    let mut said = Vec::new();
    loop {
        let frame = frame_where(socket, |f| f["event"].is_object()).await;
        let event = &frame["event"];
        if event["type"] == "said" {
            said.push((Instant::now(), event["text"].as_str().unwrap().to_string()));
        }
        if event["type"] == "clip"
            && event["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("waiting-"))
        {
            said.push((Instant::now(), TONE.to_string()));
        }
        if event["type"] == "clip" && event["final"] == true && event["data"] == "" {
            return said;
        }
    }
}

/// One device-text turn of a call to Mack, with acknowledging as
/// `acknowledge` sets it, the person's speech ending `silence_ms` before the
/// desk takes it, the call assistant's line after `line_delay` (no call
/// assistant without `assistant`), and Mack's model taking `model_delay` to
/// answer.
async fn one_turn(
    acknowledge: Option<bool>,
    assistant: bool,
    silence_ms: u32,
    line: &'static str,
    line_delay: Duration,
    model_delay: Duration,
) -> Turn {
    let root = tempfile::tempdir().unwrap();
    let acknowledger = Acknowledger::new(line, line_delay, "");
    let asked = acknowledger.asked.clone();
    let answered = acknowledger.answered.clone();
    let (model, requests) = model_answering(vec![(
        model_delay,
        text_chunks("<spoken>The checks passed.</spoken>"),
    )])
    .await;
    let (mut socket, server, call) = call_to_mack(
        acknowledge,
        assistant.then_some(acknowledger),
        &model,
        root.path(),
    )
    .await;
    let said = ask_and_hear(&mut socket, &call, silence_ms).await;
    let (asked_at, asked_model) = requests
        .lock()
        .unwrap()
        .iter()
        .find_map(|(at, request)| {
            request["messages"]
                .as_array()?
                .iter()
                .filter(|message| message["role"] == "user")
                .map(|message| message["content"].to_string())
                .find(|content| content.contains("[Voice call: answer naturally"))
                .map(|content| (*at, content))
        })
        .expect("the turn's input carries the contract");
    socket.close(None).await.unwrap();
    server.abort();
    Turn {
        said: said.into_iter().map(|(_, line)| line).collect(),
        asked: asked_model,
        asked_at,
        acknowledgements: asked.load(Ordering::SeqCst),
        answered_at: *answered.lock().unwrap(),
    }
}

/// A slow teammate: its model is asked before the call assistant answers,
/// told the desk acknowledges the turn and asked for no opening of its own;
/// the line is said first and the teammate's own words follow.
#[tokio::test]
async fn a_slow_teammate_is_asked_at_once_and_its_line_is_said_first() {
    let turn = one_turn(
        Some(true),
        true,
        550,
        LINE,
        Duration::from_millis(800),
        Duration::from_millis(2_500),
    )
    .await;
    assert_eq!(turn.acknowledgements, 1);
    assert!(
        turn.answered_at
            .is_some_and(|answered| turn.asked_at < answered),
        "the teammate's model is asked before the line is written"
    );
    assert_eq!(
        turn.said.first().map(String::as_str),
        Some(LINE),
        "{:?}",
        turn.said
    );
    assert_eq!(
        turn.said.last().map(String::as_str),
        Some("The checks passed.")
    );
    assert!(
        turn.asked.contains(&as_sent(ACKNOWLEDGED_CONTRACT)),
        "{}",
        turn.asked
    );
    assert!(!turn.asked.contains("Before a tool"), "{}", turn.asked);
    assert!(!turn.asked.contains(LINE), "{}", turn.asked);
}

/// An empty line, or one past the cap from the end of the person's speech:
/// nothing is said for it, and the teammate, told the desk would
/// acknowledge, simply answers.
#[tokio::test]
async fn an_empty_or_late_line_is_not_said() {
    for (silence, line, delay) in [
        (550, "", Duration::ZERO),
        (3_800, LINE, Duration::from_millis(900)),
    ] {
        let turn = one_turn(
            Some(true),
            true,
            silence,
            line,
            delay,
            Duration::from_millis(1_500),
        )
        .await;
        assert_eq!(turn.acknowledgements, 1);
        assert_eq!(turn.said, ["The checks passed."], "{line:?}");
        assert!(
            turn.asked.contains(&as_sent(ACKNOWLEDGED_CONTRACT)),
            "{}",
            turn.asked
        );
    }
}

/// Switched off, the call assistant is never asked and the turn is today's:
/// the teammate's words alone, and the contract that asks for its own
/// opening.
#[tokio::test]
async fn with_the_setting_off_the_turn_is_unchanged() {
    let turn = one_turn(
        Some(false),
        true,
        550,
        LINE,
        Duration::ZERO,
        Duration::from_millis(1_500),
    )
    .await;
    assert_eq!(turn.acknowledgements, 0);
    assert_eq!(turn.said, ["The checks passed."]);
    assert!(turn.asked.contains(&as_sent(CONTRACT)), "{}", turn.asked);
    assert!(
        !turn.asked.contains("brief acknowledgement"),
        "{}",
        turn.asked
    );
}

/// A desk that never set it acknowledges: the line is asked for and said
/// first, and the teammate is told the desk acknowledges the turn.
#[tokio::test]
async fn never_set_the_turn_is_acknowledged() {
    let turn = one_turn(
        None,
        true,
        550,
        LINE,
        Duration::from_millis(300),
        Duration::from_millis(2_500),
    )
    .await;
    assert_eq!(turn.acknowledgements, 1);
    assert_eq!(turn.said, [LINE, "The checks passed."], "{:?}", turn.said);
    assert!(
        turn.asked.contains(&as_sent(ACKNOWLEDGED_CONTRACT)),
        "{}",
        turn.asked
    );
}

/// Never set, on a desk with no call assistant: no line can be written, so
/// the teammate is not told of one and keeps its own opening, as the
/// contract it always had asks.
#[tokio::test]
async fn never_set_without_a_call_assistant_the_turn_is_unchanged() {
    let turn = one_turn(
        None,
        false,
        550,
        LINE,
        Duration::ZERO,
        Duration::from_millis(1_500),
    )
    .await;
    assert_eq!(turn.acknowledgements, 0);
    assert_eq!(turn.said, ["The checks passed."]);
    assert!(turn.asked.contains(&as_sent(CONTRACT)), "{}", turn.asked);
    assert!(turn.asked.contains("Before a tool"), "{}", turn.asked);
    assert!(
        !turn.asked.contains("brief acknowledgement"),
        "{}",
        turn.asked
    );
}

/// A teammate that goes straight to work and stays silent: about six seconds
/// after the person stopped speaking, with nothing said yet, the desk says
/// one line on what its session is doing, worded by the call assistant from
/// the step it took and told nothing was said before it; the waiting tone
/// does not follow, and the teammate's own answer comes after.
#[tokio::test]
async fn a_teammate_working_in_silence_is_heard_saying_what_it_is_doing() {
    const PROGRESS: &str = "Reading the PR notes now.";
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("notes.md"), "The PR fails on lint.").unwrap();
    let assistant = Acknowledger::new("", Duration::ZERO, PROGRESS);
    let progressed = assistant.progressed.clone();
    let (model, _requests) = model_answering(vec![
        (Duration::ZERO, read_chunks("notes.md")),
        (
            Duration::from_secs(9),
            text_chunks("<spoken>The PR fails on lint.</spoken>"),
        ),
    ])
    .await;
    let (mut socket, server, call) =
        call_to_mack(Some(true), Some(assistant), &model, root.path()).await;
    let asked = Instant::now();
    let said = ask_and_hear(&mut socket, &call, 550).await;
    socket.close(None).await.unwrap();
    server.abort();
    let lines: Vec<&str> = said.iter().map(|(_, line)| line.as_str()).collect();
    assert_eq!(lines, [PROGRESS, "The PR fails on lint."]);
    let after = said[0].0.duration_since(asked);
    assert!(
        after >= Duration::from_millis(5_400) && after < Duration::from_millis(7_500),
        "six seconds after the person's last word, 550 ms of it before the desk took the turn: {after:?}"
    );
    assert_eq!(
        *progressed.lock().unwrap(),
        [("reading files".to_string(), None)]
    );
}
