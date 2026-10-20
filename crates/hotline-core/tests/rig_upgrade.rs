//! A 0.42 profile on Rig 0.45, through the same desk and wire as the window.
//! Only provider origins, speech synthesis and the OS secret store are stubs.
mod common;

use axum::{
    Router,
    body::Bytes,
    http::{HeaderMap, Uri},
};
use base64::{Engine, prelude::BASE64_URL_SAFE_NO_PAD};
use common::{ANSWER, Client};
use hotline_core::voice::room::dispatcher as ProviderDispatcher;
use hotline_core::{
    credentials::{CredentialFiles, MemoryStore},
    desk::Desk,
    log::Log,
    provider_fixture::Origin,
    vault::Vault,
    voice::Services,
    wire::Door,
};
use hotline_voice::speech::{Clip, Speech, SpeechError, SpeechId, SpeechSet};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    fmt::Write,
    hash::{DefaultHasher, Hash, Hasher},
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc;

const MODEL: &str = "claude-haiku-4-5";
const GOAL: &str = "Keep the upgrade proof visible in every round.";

struct Request {
    path: String,
    headers: HeaderMap,
    body: Value,
    release: Option<tokio::sync::oneshot::Sender<()>>,
}
struct Stub {
    url: String,
    requests: mpsc::UnboundedReceiver<Request>,
    task: tokio::task::JoinHandle<()>,
}

impl Stub {
    async fn new(answers: Vec<String>) -> Self {
        Self::serve(answers, false).await
    }

    async fn serve(answers: Vec<String>, paused: bool) -> Self {
        let answers = Arc::new(Mutex::new(VecDeque::from(answers)));
        let (tx, requests) = mpsc::unbounded_channel();
        let app = Router::new().fallback(move |uri: Uri, headers: HeaderMap, bytes: Bytes| {
            let answers = answers.clone();
            let tx = tx.clone();
            async move {
                let (release, ready) = tokio::sync::oneshot::channel();
                tx.send(Request {
                    path: uri.path().into(),
                    headers,
                    body: serde_json::from_slice(&bytes).unwrap(),
                    release: paused.then_some(release),
                })
                .unwrap();
                if paused {
                    ready.await.unwrap();
                }
                let body = answers
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("an unscripted provider request");
                ([("content-type", "text/event-stream")], body)
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            url,
            requests,
            task,
        }
    }
    async fn request(&mut self) -> Request {
        tokio::time::timeout(ANSWER, self.requests.recv())
            .await
            .expect("provider was never called")
            .unwrap()
    }
}
impl Drop for Stub {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Running {
    client: Client,
    task: tokio::task::JoinHandle<()>,
}
impl Running {
    async fn new(root: &Path, memory: Arc<MemoryStore>, services: Option<Services>) -> Self {
        let desk = Arc::new(Desk::open_with_voice_services(root, memory, services).unwrap());
        let parts = desk.parts();
        let door = Door::bind(desk.log.clone(), "upgrade".into(), desk, parts).unwrap();
        let port = door.port();
        let task = tokio::spawn(async move { door.run().await.unwrap() });
        Self {
            client: Client::connect(port, "upgrade").await,
            task,
        }
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn call(client: &mut Client, command: &str, params: Value) -> Value {
    let reply = client.call(command, params).await;
    assert_eq!(reply["ok"], true, "{command}: {reply}");
    reply["result"].clone()
}

async fn teammate(client: &mut Client, root: &Path, model: &str) -> (String, i64) {
    let workspace = root.join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(
        workspace.join("note.txt"),
        "The upgrade kept this tool result.",
    )
    .unwrap();
    let made = call(
        client,
        "persona.create",
        json!({"draft":{
            "name":"Upgrade tester", "goal":GOAL, "cwd":workspace, "modelId":model
        }}),
    )
    .await;
    let id = made["id"].as_str().unwrap().to_string();
    let tape = client.subscribe(json!({"tape":id})).await;
    call(client, "session.start", json!({"personaId":id})).await;
    call(
        client,
        "session.prompt",
        json!({"personaId":id,"text":"Read note.txt and report it."}),
    )
    .await;
    (id, tape)
}

async fn turned(client: &mut Client, tape: i64) -> Value {
    loop {
        let frame = client
            .next_where(ANSWER, |f| {
                f["sub"] == tape && matches!(f["event"]["kind"].as_str(), Some("turn" | "notice"))
            })
            .await;
        assert_ne!(frame["event"]["level"], "error", "{frame}");
        if frame["event"]["kind"] == "turn" {
            assert_eq!(frame["event"]["stopReason"], "end_turn", "{frame}");
            return frame["event"].clone();
        }
    }
}

fn sse(events: Vec<Value>, named: bool) -> String {
    let mut body = String::new();
    for event in events {
        if named {
            writeln!(body, "event: {}", event["type"].as_str().unwrap()).unwrap();
        }
        write!(body, "data: {event}\n\n").unwrap();
    }
    if !named {
        body.push_str("data: [DONE]\n\n");
    }
    body
}

fn responses() -> String {
    let item = json!({"type":"message","id":"msg_upgrade","status":"completed","role":"assistant", "content":[{"type":"output_text","text":"Still signed in.","annotations":[]}]});
    sse(
        vec![
            json!({"type":"response.output_item.done","output_index":0,"item":item}),
            json!({"type":"response.completed","response":{"id":"resp_upgrade","status":"completed","output":[item],"usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}}}),
        ],
        false,
    )
}
fn chat() -> String {
    sse(
        vec![
            json!({"id":"chat_upgrade","object":"chat.completion.chunk","created":1,"model":"gpt-4.1","choices":[{"index":0,"delta":{"role":"assistant","content":"Still signed in."},"finish_reason":null}]}),
            json!({"id":"chat_upgrade","object":"chat.completion.chunk","created":1,"model":"gpt-4.1","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}),
        ],
        false,
    )
}

/// Both historical Copilot files must traverse the login store's migration,
/// not a fixture preconverted to copilot.json. Its original API token is used.
#[tokio::test]
async fn rig_upgrade_a_042_profile_keeps_both_signins_and_leaves_no_plaintext_tokens() {
    let root = tempfile::tempdir().unwrap();
    let memory = Arc::new(MemoryStore::default());
    let mut gpt = Stub::new(vec![responses()]).await;
    let mut copilot = Stub::new(vec![chat(), responses()]).await;
    let vault =
        Vault::open_with_store(root.path(), Log::open(root.path()), memory.clone()).unwrap();
    let (gpt_id, gpt_dir) = vault.begin_login().unwrap();
    let expires = chrono::Utc::now().timestamp() + 3600;
    let claims = BASE64_URL_SAFE_NO_PAD.encode(json!({"exp":expires,"https://api.openai.com/auth":{"chatgpt_account_id":"upgrade-account"}}).to_string());
    let access = format!("e30.{claims}.upgrade-access");
    let auth = json!({"access_token":access,"refresh_token":"upgrade-refresh","id_token":null,"expires_at":expires,"account_id":"upgrade-account"});
    std::fs::write(gpt_dir.join("auth.json"), auth.to_string()).unwrap();
    std::fs::write(gpt_dir.join("models.json"), json!(["gpt-5.4"]).to_string()).unwrap();
    vault
        .finish_login(&gpt_id, "openai-codex", "Existing ChatGPT")
        .unwrap();
    let _origin = Origin::new(gpt_dir.to_string_lossy(), &gpt.url);
    let (copilot_id, copilot_dir) = vault.begin_login().unwrap();
    let github = "upgrade-github-token";
    let mut hash = DefaultHasher::new();
    github.hash(&mut hash);
    let api = json!({"token":"upgrade-copilot-token","expires_at":expires,"endpoints":{"api":copilot.url},"bootstrap_token_fingerprint":format!("{:016x}", hash.finish())});
    std::fs::write(copilot_dir.join("access-token"), format!("{github}\n")).unwrap();
    std::fs::write(copilot_dir.join("api-key.json"), api.to_string()).unwrap();
    std::fs::write(
        copilot_dir.join("models.json"),
        json!(["gpt-4.1", "gpt-5.4"]).to_string(),
    )
    .unwrap();
    std::fs::write(
        copilot_dir.join("endpoints.json"),
        json!({"gpt-4.1":["/chat/completions"],"gpt-5.4":["/responses"]}).to_string(),
    )
    .unwrap();
    vault
        .finish_login(&copilot_id, "github-copilot", "Existing Copilot")
        .unwrap();
    drop(vault);

    let mut running = Running::new(root.path(), memory.clone(), None).await;
    for model in [
        "openai-codex/gpt-5.4",
        "github-copilot/gpt-4.1",
        "github-copilot/gpt-5.4",
    ] {
        let (id, tape) = teammate(&mut running.client, root.path(), model).await;
        let turn = turned(&mut running.client, tape).await;
        assert_eq!(turn["usage"]["inputTokens"], 10);
        let said = running
            .client
            .next_where(ANSWER, |f| {
                f["sub"] == tape && f["event"]["kind"] == "agent"
            })
            .await;
        assert_eq!(said["event"]["text"], "Still signed in.");
        call(&mut running.client, "session.stop", json!({"personaId":id})).await;
    }
    let request = gpt.request().await;
    assert!(request.path.ends_with("/responses"), "{}", request.path);
    assert_eq!(request.headers["authorization"], format!("Bearer {access}"));
    assert_eq!(request.headers["chatgpt-account-id"], "upgrade-account");
    for path in ["/chat/completions", "/responses"] {
        let request = copilot.request().await;
        assert_eq!(request.path, path);
        assert_eq!(
            request.headers["authorization"],
            "Bearer upgrade-copilot-token"
        );
    }
    assert!(gpt.requests.try_recv().is_err());
    assert!(copilot.requests.try_recv().is_err());
    assert!(!copilot_dir.join("access-token").exists());
    assert!(!copilot_dir.join("api-key.json").exists());
    let files = CredentialFiles::new(root.path().into(), memory.clone());
    for (dir, record, expected) in [
        (&gpt_dir, "auth.json", auth),
        (&copilot_dir, "copilot.json", {
            let mut value = api;
            value["github_token"] = json!(github);
            value
        }),
    ] {
        let path = dir.join(record);
        let reference: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(reference["hotlineCredential"], 1);
        let kept: Value =
            serde_json::from_slice(&files.file(path).read().unwrap().unwrap()).unwrap();
        assert_eq!(kept, expected);
    }
    assert!(!memory.0.lock().unwrap().is_empty());
    no_secrets(
        root.path(),
        &[&access, "upgrade-refresh", github, "upgrade-copilot-token"],
    );
}

fn no_secrets(dir: &Path, secrets: &[&str]) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            no_secrets(&path, secrets);
        } else {
            let bytes = std::fs::read(&path).unwrap();
            for secret in secrets {
                assert!(
                    !bytes
                        .windows(secret.len())
                        .any(|part| part == secret.as_bytes()),
                    "plaintext in {}",
                    path.display()
                );
            }
        }
    }
}

fn claude(tool: bool, usage: Value, complete: bool) -> String {
    let mut start_usage = usage.clone();
    start_usage["output_tokens"] = json!(1);
    let mut events = vec![
        json!({"type":"message_start","message":{"id":"msg_upgrade","type":"message","role":"assistant","model":MODEL,"content":[],"stop_reason":null,"stop_sequence":null,"usage":start_usage}}),
    ];
    if tool {
        events.extend([
            json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_upgrade","name":"read","input":{}}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"note.txt\"}"}}),
        ]);
    } else {
        events.extend([
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"The upgrade kept this tool result."}}),
        ]);
    }
    if complete {
        events.extend([
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":if tool {"tool_use"} else {"end_turn"},"stop_sequence":null},"usage":usage}),
            json!({"type":"message_stop"}),
        ]);
    }
    sse(events, true)
}
fn cache_write() -> Value {
    json!({"input_tokens":3,"cache_creation_input_tokens":9702,"cache_read_input_tokens":0,"output_tokens":7})
}
fn cache_read() -> Value {
    json!({"input_tokens":12,"cache_creation_input_tokens":40,"cache_read_input_tokens":9702,"output_tokens":5})
}
fn tenfold(mut usage: Value) -> Value {
    for value in usage.as_object_mut().unwrap().values_mut() {
        *value = json!(value.as_u64().unwrap() * 10);
    }
    usage
}

fn baseline_cost() -> f64 {
    (3.0 + 9702.0 * 1.25 + 7.0 * 5.0 + 12.0 + 9702.0 * 0.1 + 40.0 * 1.25 + 5.0 * 5.0) / 1e6
}
fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-12, "{actual} != {expected}");
}

async fn spend(client: &mut Client, kind: &str) -> f64 {
    let options = call(client, "capabilities.options", json!({})).await;
    options["spending"]["budgets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["kind"] == "chat")
        .unwrap()["lines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|line| line["kind"] == kind)
        .unwrap()["dayUsd"]
        .as_f64()
        .unwrap()
}

#[tokio::test]
async fn rig_upgrade_cache_usage_budget_and_every_rounds_system_prompt_match_the_pins() {
    let root = tempfile::tempdir().unwrap();
    let memory = Arc::new(MemoryStore::default());
    let mut stub = Stub::new(vec![
        claude(true, cache_write(), true),
        claude(false, cache_read(), true),
    ])
    .await;
    let key = uuid::Uuid::new_v4().to_string();
    let _origin = Origin::new(&key, &stub.url);
    let mut running = Running::new(root.path(), memory, None).await;
    call(
        &mut running.client,
        "credential.create",
        json!({"providerId":"anthropic","label":"Stub","secret":key}),
    )
    .await;
    let (id, tape) = teammate(
        &mut running.client,
        root.path(),
        &format!("anthropic/{MODEL}"),
    )
    .await;
    let turn = turned(&mut running.client, tape).await;
    assert_eq!(
        turn["usage"],
        json!({"inputTokens":19459,"outputTokens":12,"totalTokens":19471,"cacheReadTokens":9702,"cacheWriteTokens":9742})
    );
    near(
        spend(&mut running.client, "teammates").await,
        baseline_cost(),
    );
    for round in 0..2 {
        let request = stub.request().await;
        assert_eq!(request.path, "/v1/messages");
        assert!(
            request.body["system"].to_string().contains(GOAL),
            "round {round}: {}",
            request.body
        );
        assert_eq!(request.body["cache_control"]["type"], "ephemeral");
        if round == 1 {
            assert!(
                request.body["messages"]
                    .to_string()
                    .contains("The upgrade kept this tool result.")
            );
        }
    }
    call(&mut running.client, "session.stop", json!({"personaId":id})).await;
    assert!(stub.requests.try_recv().is_err());
}

/// S0's usage scaled tenfold: the full desk's tools alone exceed its tiny
/// 7,904-token threshold. 97,120 tokens cross 120,000 - 40,960 even though
/// only 30 input tokens were uncached. The wire turn carries the committed
/// tool result across recovery instead of executing it again.
#[tokio::test]
async fn rig_upgrade_cached_input_fills_the_same_context_threshold() {
    let root = tempfile::tempdir().unwrap();
    let memory = Arc::new(MemoryStore::default());
    let mut stub = Stub::new(vec![
        claude(true, tenfold(cache_write()), true),
        claude(false, tenfold(cache_read()), true),
    ])
    .await;
    let key = uuid::Uuid::new_v4().to_string();
    let _origin = Origin::new(&key, &stub.url);
    let vault =
        Vault::open_with_store(root.path(), Log::open(root.path()), memory.clone()).unwrap();
    let credential = vault.create("anthropic", "Stub", &key).unwrap();
    let models = root.path().join("vault/logins").join(&credential.id);
    std::fs::create_dir_all(&models).unwrap();
    std::fs::write(
        models.join("discovery.json"),
        json!([{
            "id":MODEL,"context_limit":120_000,"output_limit":40_960
        }])
        .to_string(),
    )
    .unwrap();
    drop(vault);
    let mut running = Running::new(root.path(), memory, None).await;
    let (id, tape) = teammate(
        &mut running.client,
        root.path(),
        &format!("anthropic/{MODEL}"),
    )
    .await;
    turned(&mut running.client, tape).await;
    let first = stub.request().await;
    let second = stub.request().await;
    assert!(
        !first.body["messages"]
            .to_string()
            .contains("Fresh continuation:")
    );
    let continuation = second.body["messages"].to_string();
    assert!(
        continuation.contains("Fresh continuation:"),
        "{continuation}"
    );
    assert!(continuation.contains("The upgrade kept this tool result."));
    for request in [first, second] {
        assert!(request.body["system"].to_string().contains(GOAL));
        assert_eq!(request.body["max_tokens"], 40960);
    }
    near(
        spend(&mut running.client, "teammates").await,
        baseline_cost() * 10.0,
    );
    call(&mut running.client, "session.stop", json!({"personaId":id})).await;
    assert!(stub.requests.try_recv().is_err());
}

struct SilentSpeech;
#[async_trait::async_trait]
impl Speech for SilentSpeech {
    fn id(&self) -> SpeechId {
        SpeechId {
            provider_id: "test".into(),
            model_id: "fake".into(),
            voice: None,
        }
    }
    async fn transcribe(&self, _: Clip) -> Result<String, SpeechError> {
        unreachable!("a text call does not transcribe")
    }
    async fn speak(&self, _: &str) -> Result<Clip, SpeechError> {
        Ok(Clip {
            mime: "audio/wav".into(),
            bytes: include_bytes!("fixtures/voice/acknowledgement.wav").to_vec(),
        })
    }
}

#[tokio::test]
async fn rig_upgrade_a_voice_dispatch_settles_once_and_a_dropped_stream_keeps_its_charge() {
    let root = tempfile::tempdir().unwrap();
    let memory = Arc::new(MemoryStore::default());
    let mut stub = Stub::serve(
        vec![
            claude(false, cache_read(), true),
            claude(false, cache_read(), false),
        ],
        true,
    )
    .await;
    let key = uuid::Uuid::new_v4().to_string();
    let _origin = Origin::new(&key, &stub.url);
    let log = Log::open(root.path());
    let vault = Arc::new(Vault::open_with_store(root.path(), log.clone(), memory.clone()).unwrap());
    vault.create("anthropic", "Stub", &key).unwrap();
    log.append(&hotline_core::log::StreamId::Room, &json!({"kind":"setting","id":"voice","value":{"dispatcher":{"provider":"anthropic","model":MODEL}}})).unwrap();
    let dispatcher = ProviderDispatcher::resolve(vault, &log).unwrap();
    let speech = Arc::new(SilentSpeech);
    let services = Services {
        speech: SpeechSet {
            stt: speech.clone(),
            tts: speech,
            fallback_tts: None,
        },
        dispatcher: Some(dispatcher),
        realtime: None,
    };
    let mut running = Running::new(root.path(), memory, Some(services)).await;
    let call_id = uuid::Uuid::new_v4().to_string();
    call(
        &mut running.client,
        "voice.call_start",
        json!({"callId":call_id,"inputMode":"text"}),
    )
    .await;
    let sub = running.client.subscribe(json!({"call":call_id})).await;
    // Consume the initial listening snapshot before waiting for a turn to finish.
    running
        .client
        .next_where(ANSWER, |f| f["sub"] == sub && f["snapshot"].is_array())
        .await;
    for seq in 1..=2 {
        call(
            &mut running.client,
            "voice.text",
            json!({"callId":call_id,"seq":seq,"text":"Say hello."}),
        )
        .await;
        let mut request = stub.request().await;
        assert_eq!(request.path, "/v1/messages");
        let reserved = spend(&mut running.client, "callAssistant").await;
        request.release.take().unwrap().send(()).unwrap();
        running
            .client
            .next_where(ANSWER, |f| {
                f["sub"] == sub && f["event"]["state"] == "listening"
            })
            .await;
        let charged = spend(&mut running.client, "callAssistant").await;
        let first = (12.0 + 40.0 * 1.25 + 9702.0 * 0.1 + 5.0 * 5.0) / 1e6;
        if seq == 1 {
            near(charged, first);
        } else {
            // The second HTTP stream closes after text, before final usage.
            // Its full 512-token output reservation remains, not a second
            // settlement using the first turn's cache counters.
            assert!(charged >= first + 512.0 * 5.0 / 1e6, "{charged}");
            near(charged, reserved);
            near(spend(&mut running.client, "callAssistant").await, charged);
        }
    }
    call(
        &mut running.client,
        "voice.call_end",
        json!({"callId":call_id}),
    )
    .await;
    assert!(stub.requests.try_recv().is_err());
}
