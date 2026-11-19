//! The desk's acknowledgement while a teammate thinks, over the wire with a
//! real teammate session: the line is said only when the teammate's turn
//! was told of it, and with the setting off the call is as it always was.
use super::*;

const LINE: &str = "Looking into the failing PR.";
/// What the teammate's turn is told when the line was said for it.
const TOLD: &str = "you have already said this aloud";

/// A call assistant that only acknowledges, with `line` after `delay`.
struct Acknowledger {
    line: &'static str,
    delay: Duration,
    asked: Arc<AtomicUsize>,
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
        Ok(self.line.into())
    }
}

/// A teammate's model that takes `delay` before it answers `reply`, and
/// keeps what it was sent.
async fn slow_model(
    reply: &'static str,
    delay: Duration,
) -> (String, Arc<std::sync::Mutex<Vec<Value>>>) {
    use axum::{Router, body::Bytes, routing::post};
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let requests = seen.clone();
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |body: Bytes| {
            let requests = requests.clone();
            async move {
                if let Ok(request) = serde_json::from_slice::<Value>(&body) {
                    requests.lock().unwrap().push(request);
                }
                tokio::time::sleep(delay).await;
                let delta = json!({"id":"slow","object":"chat.completion.chunk","created":1,"model":"test","choices":[{"index":0,"delta":{"role":"assistant","content":reply},"finish_reason":null}]});
                let done = json!({"id":"slow","object":"chat.completion.chunk","created":1,"model":"test","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}});
                (
                    [("Content-Type", "text/event-stream")],
                    format!("data: {delta}\n\ndata: {done}\n\ndata: [DONE]\n\n"),
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

/// What one turn of a call to Mack said, in order, and what Mack's model
/// was asked.
struct Turn {
    said: Vec<String>,
    asked: String,
    acknowledgements: usize,
}

/// One device-text turn of a call to Mack, with acknowledging `on`, the
/// call assistant's line after `line_delay`, and Mack's model taking
/// `model_delay` to answer.
async fn one_turn(
    on: bool,
    line: &'static str,
    line_delay: Duration,
    model_delay: Duration,
) -> Turn {
    let root = tempfile::tempdir().unwrap();
    let asked = Arc::new(AtomicUsize::new(0));
    let mut services = services();
    services.dispatcher = Some(Arc::new(Acknowledger {
        line,
        delay: line_delay,
        asked: asked.clone(),
    }));
    let desk = Arc::new(
        hotline_core::desk::Desk::open_with_voice_services(
            root.path(),
            common::store(),
            Some(services),
        )
        .unwrap(),
    );
    let log = desk.log.clone();
    let parts = desk.parts();
    let door = Door::bind(log, "ack-test".into(), desk, parts).unwrap();
    let port = door.port();
    let server = tokio::spawn(door.run());
    let (mut socket, _) = connect_async(format!("ws://127.0.0.1:{port}/ws?token=ack-test"))
        .await
        .unwrap();
    let (model, requests) = slow_model("<spoken>The checks passed.</spoken>", model_delay).await;
    let saved = answer(&mut socket, 90, json!({"cmd":"credential.custom_save","params":{"draft":{"name":"Slow fixture","baseUrl":model,"api":"chat_completions","models":["test"],"secret":"fixture-key"}}})).await;
    assert_eq!(saved["ok"], true, "{saved}");
    if on {
        let set = answer(
            &mut socket,
            89,
            json!({"cmd":"settings.update","params":{"patch":{"voice":{"acknowledge":true}}}}),
        )
        .await;
        assert_eq!(set["ok"], true, "{set}");
    }
    let made = answer(&mut socket, 91, json!({"cmd":"persona.create","params":{"draft":{"name":"Mack","goal":"Check the PR.","cwd":root.path().to_str().unwrap()}}})).await;
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
    send(&mut socket, json!({"id":3,"cmd":"voice.text","params":{"callId":call,"seq":1,"text":"ask Mack to check the failing PR","silenceMs":550}})).await;
    // Everything said until the teammate's reply has been said whole.
    let mut said = Vec::new();
    loop {
        let frame = frame_where(&mut socket, |f| f["event"].is_object()).await;
        let event = &frame["event"];
        if event["type"] == "said" {
            said.push(event["text"].as_str().unwrap().to_string());
        }
        if event["type"] == "clip" && event["final"] == true && event["data"] == "" {
            break;
        }
    }
    let asked_model = requests
        .lock()
        .unwrap()
        .iter()
        .flat_map(|request| request["messages"].as_array().cloned().unwrap_or_default())
        .filter(|message| message["role"] == "user")
        .map(|message| message["content"].to_string())
        .find(|content| content.contains("[Voice call: answer naturally"))
        .expect("the turn's input carries the contract");
    socket.close(None).await.unwrap();
    server.abort();
    Turn {
        said,
        asked: asked_model,
        acknowledgements: asked.load(Ordering::SeqCst),
    }
}

/// A slow teammate: the line is said first, the teammate's turn was told it
/// was said and not to acknowledge again, and the teammate's own words follow.
#[tokio::test]
async fn a_slow_teammate_is_told_its_line_was_said_and_it_is_said_first() {
    let turn = one_turn(true, LINE, Duration::ZERO, Duration::from_millis(1_500)).await;
    assert_eq!(turn.acknowledgements, 1);
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
    assert!(turn.asked.contains(TOLD), "{}", turn.asked);
    assert!(turn.asked.contains(LINE), "{}", turn.asked);
    assert!(
        turn.asked.contains("Do not acknowledge the request again"),
        "{}",
        turn.asked
    );
}

/// An empty line, or one later than the hand-off waits for: nothing is said
/// for it and the teammate's turn is not told of any.
#[tokio::test]
async fn an_empty_or_late_line_is_neither_said_nor_told() {
    for (line, delay) in [("", Duration::ZERO), (LINE, Duration::from_millis(900))] {
        let turn = one_turn(true, line, delay, Duration::from_millis(1_500)).await;
        assert_eq!(turn.acknowledgements, 1);
        assert_eq!(turn.said, ["The checks passed."], "{line:?}");
        assert!(!turn.asked.contains(TOLD), "{}", turn.asked);
        assert!(!turn.asked.contains(LINE), "{}", turn.asked);
    }
}

/// Off, the call assistant is never asked and the turn is today's: the
/// teammate's words alone, and a prompt with no line in it.
#[tokio::test]
async fn with_the_setting_off_the_turn_is_unchanged() {
    let turn = one_turn(false, LINE, Duration::ZERO, Duration::from_millis(1_500)).await;
    assert_eq!(turn.acknowledgements, 0);
    assert_eq!(turn.said, ["The checks passed."]);
    assert!(!turn.asked.contains(TOLD), "{}", turn.asked);
    assert!(!turn.asked.contains(LINE), "{}", turn.asked);
}
