//! What must survive a Rig upgrade, pinned where Rig cannot move it: the
//! bodies Hotline's requests reach a provider with, and Hotline's own
//! counters. Every test here is named `rig_pin_…`. A Rig bump keeps their
//! assertions and expected values as they are and changes at most how a
//! fixture is built (a client pointed at the stub, a request to refuse); see
//! `docs/plans/rig-0.45-migration.md`.

use super::super::build::{RequestSettings, anthropic_model};
use super::*;
use axum::Router;
use axum::body::Bytes;
use axum::http::{StatusCode, Uri};
use axum::response::IntoResponse;
use rig::DynModel;
use rig::operation::Completion;
use rig::providers::{gemini, openai};
use std::collections::VecDeque;
use std::fmt::Write;

const PREAMBLE: &str = "You are Ada, a teammate in Hotline.";

/// One answer the stub gives, in the order requests reach it.
struct Answer {
    status: StatusCode,
    content_type: &'static str,
    retry_after: Option<&'static str>,
    body: String,
}

/// A provider's server-sent events, `event:` lines included when the
/// provider names its events.
fn events(named: bool, events: &[Value]) -> Answer {
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
    Answer {
        status: StatusCode::OK,
        content_type: "text/event-stream",
        retry_after: None,
        body,
    }
}

fn refusal(status: u16, retry_after: Option<&'static str>, body: &str) -> Answer {
    Answer {
        status: StatusCode::from_u16(status).unwrap(),
        content_type: "application/json",
        retry_after,
        body: body.to_string(),
    }
}

/// A provider that records every request's path and body and answers each
/// with the next of its answers.
struct Stub {
    url: String,
    requests: mpsc::UnboundedReceiver<(String, Value)>,
    server: tokio::task::JoinHandle<()>,
}

impl Stub {
    async fn serve(answers: Vec<Answer>) -> Self {
        let (seen, requests) = mpsc::unbounded_channel();
        let answers = Arc::new(Mutex::new(VecDeque::from(answers)));
        let app = Router::new().fallback(move |uri: Uri, body: Bytes| {
            let seen = seen.clone();
            let answers = answers.clone();
            async move {
                let body = serde_json::from_slice(&body).unwrap_or(Value::Null);
                seen.send((uri.path().to_string(), body)).unwrap();
                let answer = lock(&answers)
                    .pop_front()
                    .unwrap_or_else(|| panic!("an unscripted request to {uri}"));
                let mut response = (
                    answer.status,
                    [("content-type", answer.content_type)],
                    answer.body,
                )
                    .into_response();
                if let Some(seconds) = answer.retry_after {
                    response
                        .headers_mut()
                        .insert("retry-after", seconds.parse().unwrap());
                }
                response
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            url,
            requests,
            server,
        }
    }

    /// The bodies of every request that reached the stub, in order.
    fn bodies(&mut self) -> Vec<(String, Value)> {
        let mut bodies = Vec::new();
        while let Ok(request) = self.requests.try_recv() {
            bodies.push(request);
        }
        bodies
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.server.abort();
    }
}

/// A turn of `model` on `keys`, in a scratch workspace, after `history`.
fn turn_on(
    keys: HashMap<String, ProviderAuth>,
    model: &str,
    history: Vec<Message>,
    scratch: &Path,
) -> Turn {
    Turn {
        keys,
        model: model.into(),
        meter: None,
        output_limit: None,
        context_limit: None,
        effort: None,
        preamble: PREAMBLE.into(),
        cwd: scratch.to_path_buf(),
        folders: Vec::new(),
        reach: Reach::Machine,
        history: Arc::new(AsyncMutex::new(history)),
        stop: Arc::new(Stop::default()),
        steering: Arc::new(turn::Steering::default()),
        output_dir: scratch.join("outputs"),
        mcp_tools: Vec::new(),
        capability: None,
        delegate: None,
        processes: crate::session::jobs::Processes::new(scratch.join("processes")),
    }
}

/// The one tool the scripted turns call. It answers at once.
fn note_tool() -> DynamicTool {
    DynamicTool::new(
        rig::message::ToolName::new("note").unwrap(),
        "Write a note down.",
        json!({"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}),
        |_arguments| Box::pin(async { Ok(ToolOutput::text("noted")) }),
    )
}

/// `turn` run on `model` with only [`note_tool`], as [`Turn::run`] runs one
/// on a model it built: what the person was shown, and how it ended.
async fn run_on(model: &DynModel<Completion>, turn: &Turn) -> (Result<(), Failure>, Vec<Update>) {
    let mut tools = rig::tool::ToolSet::default();
    tools.add_dynamic_tool(note_tool());
    let template = turn::Template {
        tools: tools.tool_definitions(),
        settings: request_settings(
            &turn.keys,
            &turn.model,
            turn.effort.as_deref(),
            None,
            Reuse::Rounds,
        )
        .unwrap(),
    };
    let (sender, mut receiver) = mpsc::channel(1024);
    let result = turn::run(model, template, &tools, turn, &sender, None).await;
    drop(sender);
    let mut updates = Vec::new();
    while let Some(update) = receiver.recv().await {
        updates.push(update);
    }
    (result, updates)
}

/// The usage the turn recorded when it ended.
fn recorded(updates: &[Update]) -> Option<crate::contract::TokenUsage> {
    updates.iter().find_map(|update| match update {
        Update::Turn { usage, .. } => Some(*usage),
        _ => None,
    })?
}

fn api_key(provider: &str) -> HashMap<String, ProviderAuth> {
    HashMap::from([(
        provider.into(),
        ProviderAuth::ApiKey("test-only-key".into()),
    )])
}

// ---- Claude over the API --------------------------------------------------

fn claude_at(url: &str, reuse: Reuse) -> DynModel<Completion> {
    let client = anthropic::AnthropicConfig::new("test-only-key")
        .with_base_url(url)
        .client();
    anthropic_model(&client, "claude-test", reuse).erase()
}

/// Claude's stream of one response: `blocks` between the message's start
/// and its end, with Anthropic's usage on both, as the API sends it.
fn claude_stream(blocks: &[Value], stop: &str, usage: Value) -> Answer {
    let mut start_usage = usage.clone();
    start_usage["output_tokens"] = json!(1);
    let mut all = vec![json!({"type": "message_start", "message": {
        "id": "msg_pin", "type": "message", "role": "assistant", "model": "claude-test",
        "content": [], "stop_reason": null, "stop_sequence": null, "usage": start_usage
    }})];
    all.extend(blocks.iter().cloned());
    all.push(json!({"type": "message_delta",
        "delta": {"stop_reason": stop, "stop_sequence": null}, "usage": usage}));
    all.push(json!({"type": "message_stop"}));
    events(true, &all)
}

fn claude_note_call() -> Vec<Value> {
    vec![
        json!({"type": "content_block_start", "index": 0, "content_block":
            {"type": "tool_use", "id": "toolu_pin", "name": "note", "input": {}}}),
        json!({"type": "content_block_delta", "index": 0, "delta":
            {"type": "input_json_delta", "partial_json": "{\"text\":\"hi\"}"}}),
        json!({"type": "content_block_stop", "index": 0}),
    ]
}

fn claude_text(text: &str) -> Vec<Value> {
    vec![
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": text}}),
        json!({"type": "content_block_stop", "index": 0}),
    ]
}

/// Claude's two rounds of one turn: a call to `note` that wrote 9,702 tokens
/// to the prompt cache, then a reply that read them back and wrote 40 more.
fn claude_two_rounds() -> Vec<Answer> {
    vec![
        claude_stream(
            &claude_note_call(),
            "tool_use",
            json!({"input_tokens": 3, "cache_creation_input_tokens": 9_702,
                "cache_read_input_tokens": 0, "output_tokens": 7}),
        ),
        claude_stream(
            &claude_text("Noted."),
            "end_turn",
            json!({"input_tokens": 12, "cache_creation_input_tokens": 40,
                "cache_read_input_tokens": 9_702, "output_tokens": 5}),
        ),
    ]
}

/// A budget on a scratch desk with five dollars a day for Chat.
fn chat_budget(scratch: &Path) -> Arc<crate::voice::metering::Budget> {
    let log = crate::log::Log::open(scratch);
    log.append(
        &crate::log::StreamId::Room,
        &crate::room::room_event(
            "setting",
            json!({"id": "spending", "value": {"chat": {"dayUsd": 5}}}),
        ),
    )
    .unwrap();
    Arc::new(crate::voice::metering::Budget::open(log))
}

/// Claude's cache reads and writes are input the request carried: a turn's
/// record counts them in its input and beside it, and the Chat budget
/// prices each cached token once, at its cache rate.
#[tokio::test]
async fn rig_pin_claude_cache_tokens_are_counted_once_in_the_record_and_the_budget() {
    let scratch = tempfile::tempdir().unwrap();
    let stub = Stub::serve(claude_two_rounds()).await;
    let keys = api_key("anthropic");
    let mut turn = turn_on(
        keys.clone(),
        "anthropic/claude-test",
        vec![Message::user("Note hi.")],
        scratch.path(),
    );
    let budget = chat_budget(&scratch.path().join("desk"));
    let price = crate::contract::ModelCost {
        input: 1.0,
        output: 5.0,
        cache_read: Some(0.1),
        cache_write: Some(1.25),
    };
    let listed = HashMap::from([(
        "anthropic/claude-test".to_string(),
        crate::contract::CatalogModel {
            id: "claude-test".into(),
            name: "Claude Test".into(),
            release_date: String::new(),
            enabled: true,
            manual: false,
            metadata_known: true,
            context_limit: None,
            output_limit: None,
            reasoning: None,
            attachment: None,
            efforts: None,
            cost: Some(price),
        },
    )]);
    turn.meter = turn::ChatMeter::for_model(
        Some(budget.clone()),
        &keys,
        "anthropic/claude-test",
        &listed,
    );
    assert!(turn.meter.is_some(), "a key on a priced model is metered");

    let (result, updates) = run_on(&claude_at(&stub.url, Reuse::Rounds), &turn).await;
    result.unwrap();
    assert_eq!(
        recorded(&updates),
        Some(crate::contract::TokenUsage {
            input_tokens: Some(3 + 9_702 + 12 + 9_702 + 40),
            output_tokens: Some(7 + 5),
            total_tokens: Some(3 + 9_702 + 12 + 9_702 + 40 + 7 + 5),
            cache_read_tokens: Some(9_702),
            cache_write_tokens: Some(9_702 + 40),
        })
    );
    let charged = budget.spent().unwrap().day.teammates;
    let expected = (3.0 * 1.0 + 9_702.0 * 1.25 + 7.0 * 5.0 // the call
        + 12.0 * 1.0 + 9_702.0 * 0.1 + 40.0 * 1.25 + 5.0 * 5.0) // the reply
        / 1e6;
    assert!(
        (charged - expected).abs() < 1e-12,
        "{charged} is not {expected}"
    );
}

/// A context Claude read from its cache is just as full: the turn hands off
/// to a new chapter before its next request, as it would had the same
/// tokens come uncached.
#[tokio::test]
async fn rig_pin_claude_cache_reads_fill_the_context_threshold() {
    let scratch = tempfile::tempdir().unwrap();
    let mut stub = Stub::serve(claude_two_rounds()).await;
    let mut turn = turn_on(
        api_key("anthropic"),
        "anthropic/claude-test",
        vec![Message::user("Note hi.")],
        scratch.path(),
    );
    // 12,000 less a 4,096-token reply leaves 7,904. The first round reports
    // 3 + 9,702 in and 7 out, so only its cache puts it over.
    turn.context_limit = Some(12_000);
    let model = claude_at(&stub.url, Reuse::Rounds);
    let mut tools = rig::tool::ToolSet::default();
    tools.add_dynamic_tool(note_tool());
    let template = turn::Template {
        tools: tools.tool_definitions(),
        settings: RequestSettings {
            max_tokens: Some(64),
            ..request_settings(&turn.keys, &turn.model, None, None, Reuse::Rounds).unwrap()
        },
    };
    let (sender, mut receiver) = mpsc::channel(1024);
    let handed_off = async {
        loop {
            if let Update::Chapter { boundary } = receiver.recv().await.unwrap() {
                break boundary;
            }
        }
    };
    let (result, requests_before_handoff) = tokio::join!(
        async { turn::run(&model, template, &tools, &turn, &sender, None).await },
        async {
            let boundary = tokio::time::timeout(std::time::Duration::from_secs(10), handed_off)
                .await
                .expect("a context full of cached tokens hands off");
            let requests = stub.bodies().len();
            boundary.finish(Some("Chapter handoff: noted hi.".into()));
            requests
        }
    );
    result.unwrap();
    assert_eq!(requests_before_handoff, 1);
    assert_eq!(
        stub.bodies().len(),
        1,
        "the next request follows the handoff"
    );
}

/// A turn asks Claude to cache: a breakpoint that moves with the
/// conversation, and one on the system prompt. A single answer asks for
/// neither, because a cache write costs more than a read nothing follows.
#[tokio::test]
async fn rig_pin_claude_cache_markers_on_a_turn_and_none_on_a_single_answer() {
    let scratch = tempfile::tempdir().unwrap();
    let mut stub = Stub::serve(claude_two_rounds()).await;
    let turn = turn_on(
        api_key("anthropic"),
        "anthropic/claude-test",
        vec![Message::user("Note hi.")],
        scratch.path(),
    );
    let (result, _) = run_on(&claude_at(&stub.url, Reuse::Rounds), &turn).await;
    result.unwrap();
    let bodies = stub.bodies();
    assert_eq!(bodies.len(), 2);
    for (path, body) in &bodies {
        assert_eq!(path, "/v1/messages");
        assert_eq!(
            body["cache_control"],
            json!({"type": "ephemeral"}),
            "{body}"
        );
        assert_eq!(
            body["system"],
            json!([{"type": "text", "text": PREAMBLE, "cache_control": {"type": "ephemeral"}}]),
            "{body}"
        );
    }

    let mut once = Stub::serve(vec![Answer {
        status: StatusCode::OK,
        content_type: "application/json",
        retry_after: None,
        body: json!({"id": "msg_pin", "type": "message", "role": "assistant",
            "model": "claude-test", "content": [{"type": "text", "text": "ok"}],
            "stop_reason": "end_turn", "stop_sequence": null,
            "usage": {"input_tokens": 2, "output_tokens": 1}})
        .to_string(),
    }])
    .await;
    let agent = rig::agent::AgentBuilder::new(claude_at(&once.url, Reuse::Once))
        .preamble(PREAMBLE)
        .max_tokens(64)
        .build();
    assert_eq!(agent.prompt("hello").await.unwrap().output(), "ok");
    let (_, body) = once.bodies().pop().unwrap();
    assert!(
        !body.to_string().contains("cache_control"),
        "a single answer asks for no cache: {body}"
    );
}

// ---- Native state on the same provider ----------------------------------

/// The parts of the request a turn's second round sent that say what the
/// model did in its first: its reasoning, its call and the call's result.
async fn second_round(
    model: &DynModel<Completion>,
    provider: &str,
    model_id: &str,
    stub: &mut Stub,
) -> Value {
    let scratch = tempfile::tempdir().unwrap();
    let mut turn = turn_on(
        api_key(provider),
        model_id,
        vec![Message::user("Note hi.")],
        scratch.path(),
    );
    turn.effort = Some("high".into());
    let (result, _) = run_on(model, &turn).await;
    result.unwrap();
    let bodies = stub.bodies();
    assert_eq!(bodies.len(), 2);
    bodies[1].1.clone()
}

/// Claude's thinking goes back to Claude with its signature, in front of
/// the call it made, and the call's id is the one its result answers.
#[tokio::test]
async fn rig_pin_claude_thinking_goes_back_signed_with_its_call() {
    let thinking = vec![
        json!({"type": "content_block_start", "index": 0, "content_block":
            {"type": "thinking", "thinking": "", "signature": ""}}),
        json!({"type": "content_block_delta", "index": 0, "delta":
            {"type": "thinking_delta", "thinking": "I will note it."}}),
        json!({"type": "content_block_delta", "index": 0, "delta":
            {"type": "signature_delta", "signature": "signed-thinking"}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "content_block_start", "index": 1, "content_block":
            {"type": "tool_use", "id": "toolu_pin", "name": "note", "input": {}}}),
        json!({"type": "content_block_delta", "index": 1, "delta":
            {"type": "input_json_delta", "partial_json": "{\"text\":\"hi\"}"}}),
        json!({"type": "content_block_stop", "index": 1}),
    ];
    let usage = json!({"input_tokens": 3, "output_tokens": 7});
    let mut stub = Stub::serve(vec![
        claude_stream(&thinking, "tool_use", usage.clone()),
        claude_stream(&claude_text("Noted."), "end_turn", usage),
    ])
    .await;
    let model = claude_at(&stub.url, Reuse::Rounds);
    let body = second_round(&model, "anthropic", "anthropic/claude-test", &mut stub).await;
    let messages = body["messages"].as_array().unwrap();
    let said = &messages[1]["content"];
    assert_eq!(said[0]["type"], "thinking", "{body}");
    assert_eq!(said[0]["thinking"], "I will note it.", "{body}");
    assert_eq!(said[0]["signature"], "signed-thinking", "{body}");
    assert_eq!(said[1]["type"], "tool_use", "{body}");
    assert_eq!(said[1]["id"], "toolu_pin", "{body}");
    assert_eq!(
        messages[2]["content"][0]["tool_use_id"], "toolu_pin",
        "{body}"
    );
}

/// OpenAI's reasoning goes back to OpenAI as the item it came as, its id and
/// encrypted state whole, and the call's id is the one its result answers.
#[tokio::test]
async fn rig_pin_openai_reasoning_goes_back_whole_with_its_call() {
    let reasoning = json!({"type": "reasoning", "id": "rs_pin",
        "summary": [{"type": "summary_text", "text": "Noting it."}],
        "encrypted_content": "encrypted-state"});
    let call = json!({"type": "function_call", "id": "fc_pin", "call_id": "call_pin",
        "name": "note", "arguments": "{\"text\":\"hi\"}", "status": "completed"});
    let message = json!({"type": "message", "id": "msg_pin", "role": "assistant",
        "status": "completed",
        "content": [{"type": "output_text", "text": "Noted.", "annotations": []}]});
    let response = |status: &str, output: Value| {
        json!({"id": "resp_pin", "object": "response", "created_at": 1, "status": status,
            "model": "gpt-test", "output": output,
            "usage": {"input_tokens": 10, "output_tokens": 5, "total_tokens": 15}})
    };
    let round = |items: Vec<Value>| {
        let mut all = vec![json!({"type": "response.created", "sequence_number": 0,
            "response": response("in_progress", json!([]))})];
        for (index, item) in items.iter().enumerate() {
            all.push(json!({"type": "response.output_item.done",
                "sequence_number": index + 1, "output_index": index, "item": item}));
        }
        all.push(
            json!({"type": "response.completed", "sequence_number": items.len() + 1,
            "response": response("completed", json!(items))}),
        );
        events(true, &all)
    };
    let mut stub = Stub::serve(vec![round(vec![reasoning, call]), round(vec![message])]).await;
    let model = openai::OpenAIConfig::new("test-only-key")
        .with_base_url(&stub.url)
        .client()
        .completion("gpt-test")
        .erase();
    let body = second_round(&model, "openai", "openai/gpt-test", &mut stub).await;
    let input = body["input"].as_array().unwrap();
    let item = |kind: &str| {
        input
            .iter()
            .find(|item| item["type"] == kind)
            .unwrap_or_else(|| panic!("no {kind} in {body}"))
    };
    assert_eq!(item("reasoning")["id"], "rs_pin", "{body}");
    assert_eq!(
        item("reasoning")["encrypted_content"],
        "encrypted-state",
        "{body}"
    );
    assert_eq!(item("function_call")["call_id"], "call_pin", "{body}");
    assert_eq!(
        item("function_call_output")["call_id"],
        "call_pin",
        "{body}"
    );
}

/// Gemini's signature on the call it made goes back with that call, beside
/// the call's result. Signatures are base64, as Gemini sends them.
#[tokio::test]
async fn rig_pin_gemini_signatures_go_back_with_the_call() {
    let chunk = |parts: Value| {
        json!({"candidates": [{"content": {"role": "model", "parts": parts},
            "finishReason": "STOP", "index": 0}],
            "usageMetadata": {"promptTokenCount": 10, "candidatesTokenCount": 5,
                "totalTokenCount": 15}})
    };
    let sse = |chunk: Value| Answer {
        status: StatusCode::OK,
        content_type: "text/event-stream",
        retry_after: None,
        body: format!("data: {chunk}\n\n"),
    };
    let mut stub = Stub::serve(vec![
        sse(chunk(json!([
            {"text": "Noting it.", "thought": true, "thoughtSignature": "c2lnbmVkLXRob3VnaHQ="},
            {"functionCall": {"name": "note", "args": {"text": "hi"}},
                "thoughtSignature": "c2lnbmVkLWNhbGw="}
        ]))),
        sse(chunk(json!([{"text": "Noted."}]))),
    ])
    .await;
    let model = gemini::GeminiConfig::new("test-only-key")
        .with_base_url(&stub.url)
        .client()
        .completion("gemini-test")
        .erase();
    let body = second_round(&model, "google", "google/gemini-test", &mut stub).await;
    let parts: Vec<&Value> = body["contents"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|content| content["parts"].as_array().unwrap())
        .collect();
    let call = parts
        .iter()
        .find(|part| part.get("functionCall").is_some())
        .unwrap_or_else(|| panic!("no call in {body}"));
    assert_eq!(call["functionCall"]["name"], "note", "{body}");
    assert_eq!(call["thoughtSignature"], "c2lnbmVkLWNhbGw=", "{body}");
    assert!(
        parts
            .iter()
            .any(|part| part["functionResponse"]["name"] == "note"),
        "{body}"
    );
}

// ---- Claude on OpenRouter -------------------------------------------------

/// An OpenAI chat-completions stream of one text reply.
fn chat_reply(text: &str) -> Answer {
    events(
        false,
        &[
            json!({"id": "gen-pin", "object": "chat.completion.chunk", "created": 1, "model": "routed",
                "choices": [{"index": 0, "delta": {"role": "assistant", "content": text}, "finish_reason": null}]}),
            json!({"id": "gen-pin", "object": "chat.completion.chunk", "created": 1, "model": "routed",
                "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 2, "total_tokens": 12}}),
        ],
    )
}

/// OpenRouter passes a cache marker on the system prompt through to Claude,
/// which caches nothing without one; a model that caches on its own is sent
/// none.
#[tokio::test]
async fn rig_pin_openrouter_marks_claudes_system_prompt_and_no_other_models() {
    let scratch = tempfile::tempdir().unwrap();
    for (model, marked) in [("anthropic/claude-test", true), ("openai/gpt-test", false)] {
        let mut stub = Stub::serve(vec![chat_reply("ok")]).await;
        let config = openai::OpenAIConfig::with_key(&openai::wire::OPENROUTER, "test-only-key")
            .with_base_url(&stub.url);
        let turn = turn_on(
            api_key("openrouter"),
            &format!("openrouter/{model}"),
            vec![Message::user("hello")],
            scratch.path(),
        );
        let routed = openrouter_model(config, model, Reuse::Rounds).unwrap();
        let (result, _) = run_on(&routed, &turn).await;
        result.unwrap();
        let (path, body) = stub.bodies().pop().unwrap();
        assert_eq!(path, "/chat/completions");
        let system = &body["messages"][0];
        assert_eq!(system["role"], "system", "{body}");
        assert_eq!(text_of(&system["content"]), PREAMBLE, "{body}");
        if marked {
            assert_eq!(
                system["content"],
                json!([{"type": "text", "text": PREAMBLE, "cache_control": {"type": "ephemeral"}}]),
                "{body}"
            );
        } else {
            assert!(
                !body.to_string().contains("cache_control"),
                "{model}: {body}"
            );
        }
    }
}

// ---- The system prompt ----------------------------------------------------

/// Every round of a turn carries the teammate's system prompt, not only the
/// first: a round's conversation is rebuilt each time, and the prompt goes
/// with it.
#[tokio::test]
async fn rig_pin_every_round_of_a_turn_carries_the_system_prompt() {
    let scratch = tempfile::tempdir().unwrap();
    let mut stub = Stub::serve(claude_two_rounds()).await;
    let turn = turn_on(
        api_key("anthropic"),
        "anthropic/claude-test",
        vec![Message::user("Note hi.")],
        scratch.path(),
    );
    let (result, _) = run_on(&claude_at(&stub.url, Reuse::Rounds), &turn).await;
    result.unwrap();
    let bodies = stub.bodies();
    assert_eq!(bodies.len(), 2);
    for (_, body) in &bodies {
        assert_eq!(body["system"][0]["text"], PREAMBLE, "{body}");
        let said = body["messages"].to_string();
        assert!(!said.contains(PREAMBLE), "the prompt is said once: {body}");
    }

    // The same over a chat-completions route, where it leads the messages.
    let (bodies, _) = copilot_two_rounds(scratch.path()).await;
    assert_eq!(bodies.len(), 2);
    for body in &bodies {
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "system", "{body}");
        assert!(
            text_of(&messages[0]["content"]).starts_with(PREAMBLE),
            "{body}"
        );
        assert_eq!(
            messages.iter().filter(|m| m["role"] == "system").count(),
            1,
            "{body}"
        );
    }
    assert_eq!(
        bodies[0]["messages"][0], bodies[1]["messages"][0],
        "the same prompt on both rounds"
    );
}

/// A chat message's text, whether it went as a string or as text parts.
fn text_of(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect(),
        _ => String::new(),
    }
}

// ---- Copilot's chat route -------------------------------------------------

/// A Copilot chat-route turn over [`Turn::run`], on a history that already
/// holds a reply: a round that says something and calls `ls`,
/// then one that answers. The bodies of both requests, in order.
async fn copilot_two_rounds(scratch: &Path) -> (Vec<Value>, Vec<Update>) {
    let chunk = |delta: Value, finish: Value| {
        json!({"id": "chatcmpl-pin", "object": "chat.completion.chunk", "created": 1,
            "model": "claude-sonnet-5",
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
    };
    let mut usage = chunk(json!({}), json!("tool_calls"));
    usage["usage"] = json!({"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15});
    let mut stub = Stub::serve(vec![
        events(
            false,
            &[
                chunk(
                    json!({"role": "assistant", "content": "Looking."}),
                    Value::Null,
                ),
                chunk(
                    json!({"tool_calls": [{"index": 0, "id": "call_pin", "type": "function",
                        "function": {"name": "ls", "arguments": "{\"path\":\".\"}"}}]}),
                    Value::Null,
                ),
                usage,
            ],
        ),
        chat_reply("Nothing there."),
    ])
    .await;
    let dir = crate::providers::copilot::tests::seeded_login("pin", &stub.url);
    crate::providers::copilot::tests::record_endpoints(
        &dir,
        json!({"claude-sonnet-5": ["/chat/completions"]}),
    );
    let keys = HashMap::from([(
        "github-copilot".to_string(),
        ProviderAuth::Login {
            token_dir: dir.clone(),
        },
    )]);
    let workspace = scratch.join(format!("copilot-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&workspace).unwrap();
    let turn = turn_on(
        keys,
        "github-copilot/claude-sonnet-5",
        vec![
            Message::user("Is anything there?"),
            Message::assistant("I will look."),
        ],
        &workspace,
    );
    let (sender, mut receiver) = mpsc::channel(1024);
    turn.run(&sender, Some(Message::user("Look now.")))
        .await
        .unwrap();
    drop(sender);
    let mut updates = Vec::new();
    while let Some(update) = receiver.recv().await {
        updates.push(update);
    }
    let _ = fs::remove_dir_all(&dir);
    let bodies = stub.bodies();
    assert!(bodies.iter().all(|(path, _)| path == "/chat/completions"));
    (bodies.into_iter().map(|(_, body)| body).collect(), updates)
}

/// Copilot's chat route hands Claude only an assistant message's string
/// form, so every assistant message goes as a string: a reply from an
/// earlier turn, and one beside the tool call it made this turn.
#[tokio::test]
async fn rig_pin_copilot_chat_sends_assistant_content_as_a_string() {
    let scratch = tempfile::tempdir().unwrap();
    let (bodies, _) = copilot_two_rounds(scratch.path()).await;
    let assistant = |body: &Value| -> Vec<Value> {
        body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["role"] == "assistant")
            .cloned()
            .collect()
    };
    let first = assistant(&bodies[0]);
    assert_eq!(first.len(), 1, "{}", bodies[0]);
    assert_eq!(first[0]["content"], "I will look.");
    let second = assistant(&bodies[1]);
    assert_eq!(second.len(), 2, "{}", bodies[1]);
    assert_eq!(second[0]["content"], "I will look.");
    assert_eq!(second[1]["content"], "Looking.");
    assert_eq!(
        second[1]["tool_calls"][0]["function"]["name"], "ls",
        "{}",
        bodies[1]
    );
}

// ---- Ollama ---------------------------------------------------------------

/// Ollama takes a thinking level only on its own `/api/chat`, as `think`.
/// A turn at an effort sends it there.
#[tokio::test]
async fn rig_pin_ollama_effort_reaches_api_chat_as_think() {
    let scratch = tempfile::tempdir().unwrap();
    let line = |content: &str, done: bool| {
        let mut line = json!({"model": "qwen-test", "created_at": "2026-10-20T00:00:00Z",
            "message": {"role": "assistant", "content": content}, "done": done});
        if done {
            line["done_reason"] = json!("stop");
            line["prompt_eval_count"] = json!(3);
            line["eval_count"] = json!(1);
        }
        format!("{line}\n")
    };
    let mut stub = Stub::serve(vec![Answer {
        status: StatusCode::OK,
        content_type: "application/x-ndjson",
        retry_after: None,
        body: line("ok", false) + &line("", true),
    }])
    .await;
    let keys = HashMap::from([(
        "ollama".to_string(),
        ProviderAuth::Local {
            base_url: stub.url.clone(),
        },
    )]);
    let mut turn = turn_on(
        keys,
        "ollama/qwen-test",
        vec![Message::user("hello")],
        scratch.path(),
    );
    turn.effort = Some("high".into());
    let (sender, _receiver) = mpsc::channel(1024);
    turn.run(&sender, None).await.unwrap();
    let (path, body) = stub.bodies().pop().unwrap();
    assert_eq!(path, "/api/chat");
    assert_eq!(body["think"], "high", "{body}");
    assert_eq!(body["model"], "qwen-test");
}

// ---- Custom servers -------------------------------------------------------

/// A compatible server writes the Responses API more loosely than OpenAI:
/// output items without an id or a status, a reasoning item's summary null,
/// `created_at` a float, `instructions` echoed back as input items, an empty
/// `error`, a tool's description null, frames with no sequence number. Rig
/// reads each frame as the JSON it is, so a turn on one still answers, calls
/// its tool and sends the call back, and Hotline repairs nothing.
#[tokio::test]
async fn rig_pin_a_custom_servers_loose_responses_frames_make_a_turn() {
    let scratch = tempfile::tempdir().unwrap();
    let response = |output: Value| {
        json!({"id": "resp_pin", "created_at": 1_791_871_040.5, "status": "completed",
            "model": "coder", "error": {}, "incomplete_details": {},
            "instructions": [{"type": "message", "role": "developer", "content": []}],
            "tools": [{"type": "function", "name": "note", "description": null,
                "parameters": {}}],
            "output": output,
            "usage": {"input_tokens": 10, "output_tokens": 5, "total_tokens": 15}})
    };
    let round = |items: Vec<Value>| {
        let mut all = vec![json!({"type": "response.created", "response": response(json!([]))})];
        for (index, item) in items.iter().enumerate() {
            all.push(
                json!({"type": "response.output_item.done", "output_index": index,
                "item": item}),
            );
        }
        all.push(json!({"type": "response.completed", "response": response(json!(items))}));
        events(false, &all)
    };
    let reasoning = json!({"type": "reasoning", "summary": null});
    let call = json!({"type": "function_call", "call_id": "call_pin", "name": "note",
        "arguments": "{\"text\":\"hi\"}"});
    let message = json!({"type": "message", "role": "assistant",
        "content": [{"type": "output_text", "text": "Noted.", "annotations": []}]});
    let mut stub = Stub::serve(vec![round(vec![reasoning, call]), round(vec![message])]).await;
    let provider = format!("custom-{}", uuid::Uuid::new_v4());
    let keys = HashMap::from([(
        provider.clone(),
        ProviderAuth::Custom {
            name: "Local adapter".into(),
            base_url: stub.url.clone(),
            api_key: None,
            config: crate::contract::CustomProvider {
                api: crate::contract::OpenAiApi::Responses,
                models: vec!["coder".into()],
            },
        },
    )]);
    let turn = turn_on(
        keys,
        &format!("{provider}/coder"),
        vec![Message::user("Note hi.")],
        scratch.path(),
    );
    let model = crate::providers::custom::client(&stub.url, None, openai::Route::Responses)
        .unwrap()
        .completion("coder")
        .erase();
    let (result, updates) = run_on(&model, &turn).await;
    result.unwrap();
    assert!(
        updates.iter().any(|update| matches!(update,
            Update::Message { text, .. } if text == "Noted.")),
        "{updates:?}"
    );
    let bodies = stub.bodies();
    assert_eq!(bodies.len(), 2);
    let input = bodies[1].1["input"].as_array().unwrap();
    assert!(
        input
            .iter()
            .any(|item| item["type"] == "function_call_output" && item["call_id"] == "call_pin"),
        "{}",
        bodies[1].1
    );
}

// ---- Refusals -------------------------------------------------------------

/// How Hotline reads a provider's refusal of a turn's request, as the
/// provider sends it. The shapes are the providers' own error bodies.
#[tokio::test]
async fn rig_pin_provider_refusals_classify_as_before() {
    let anthropic = |url: &str| -> DynModel<Completion> { claude_at(url, Reuse::Rounds) };
    let openai = |url: &str| -> DynModel<Completion> {
        openai::OpenAIConfig::new("test-only-key")
            .with_base_url(url)
            .client()
            .completion("gpt-test")
            .erase()
    };
    let openrouter = |url: &str| -> DynModel<Completion> {
        openrouter_model(
            openai::OpenAIConfig::with_key(&openai::wire::OPENROUTER, "test-only-key")
                .with_base_url(url),
            "anthropic/claude-test",
            Reuse::Rounds,
        )
        .unwrap()
    };
    let gemini = |url: &str| -> DynModel<Completion> {
        gemini::GeminiConfig::new("test-only-key")
            .with_base_url(url)
            .client()
            .completion("gemini-test")
            .erase()
    };
    type Route = fn(&str) -> DynModel<Completion>;
    /// What a route answered and how Hotline reads it: the case, the route, the
    /// status, `retry-after`, the body; the kind and the retry it reads.
    type Case = (
        &'static str,
        Route,
        u16,
        Option<&'static str>,
        &'static str,
        Kind,
        Option<u64>,
    );
    #[rustfmt::skip]
    let table: Vec<Case> = vec![
        ("claude overloaded", anthropic, 529, None,
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"},"request_id":"req_011CTpin"}"#,
            Kind::Provider, None),
        ("claude rate limit", anthropic, 429, Some("17"),
            r#"{"type":"error","error":{"type":"rate_limit_error","message":"This request would exceed the rate limit for your organization of 50,000 input tokens per minute. For details, refer to: https://docs.claude.com/en/api/rate-limits."}}"#,
            Kind::RateLimit, Some(17)),
        ("claude prompt too long", anthropic, 400, None,
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 208310 tokens > 200000 maximum"}}"#,
            Kind::Context, None),
        ("claude credit", anthropic, 400, None,
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"Your credit balance is too low to access the Anthropic API. Please go to Plans & Billing to upgrade or purchase credits."}}"#,
            Kind::Quota, None),
        ("claude key", anthropic, 401, None,
            r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#,
            Kind::Auth, None),
        ("openai quota", openai, 429, None,
            r#"{"error":{"message":"You exceeded your current quota, please check your plan and billing details.","type":"insufficient_quota","param":null,"code":"insufficient_quota"}}"#,
            Kind::Quota, None),
        ("openai context", openai, 400, None,
            r#"{"error":{"message":"Your input exceeds the context window of this model. Please adjust your input and try again.","type":"invalid_request_error","param":"input","code":"context_length_exceeded"}}"#,
            Kind::Context, None),
        ("openai unsaved item", openai, 400, None,
            r#"{"error":{"message":"Item with id 'rs_pin' not found. Items are not persisted when `store` is set to false. Try again with `store` set to true, or remove this item from your input.","type":"invalid_request_error","param":"input","code":null}}"#,
            Kind::Replay, None),
        ("openai key", openai, 401, None,
            r#"{"error":{"message":"Incorrect API key provided: test-onl*-key.","type":"invalid_request_error","param":null,"code":"invalid_api_key"}}"#,
            Kind::Auth, None),
        ("openai overloaded", openai, 503, None,
            r#"{"error":{"message":"The server is overloaded or not ready yet.","type":"server_error","param":null,"code":null}}"#,
            Kind::Transport, None),
        ("openrouter credits", openrouter, 402, None,
            r#"{"error":{"message":"This request requires more credits, or fewer max_tokens. You requested up to 4096 tokens, but can only afford 1000.","code":402},"user_id":"user_pin"}"#,
            Kind::Quota, None),
        ("openrouter upstream limit", openrouter, 429, None,
            r#"{"error":{"message":"Provider returned error","code":429,"metadata":{"raw":"anthropic/claude-test is temporarily rate-limited upstream. Please retry shortly.","provider_name":"Anthropic"}},"user_id":"user_pin"}"#,
            Kind::RateLimit, None),
        ("gemini exhausted", gemini, 429, None,
            r#"{"error":{"code":429,"message":"Resource has been exhausted (e.g. check quota).","status":"RESOURCE_EXHAUSTED"}}"#,
            Kind::RateLimit, None),
        ("gemini overloaded", gemini, 503, None,
            r#"{"error":{"code":503,"message":"The model is overloaded. Please try again later.","status":"UNAVAILABLE"}}"#,
            Kind::Transport, None),
    ];
    for (case, route, status, retry_after, body, kind, retry_after_seconds) in table {
        let stub = Stub::serve(vec![refusal(status, retry_after, body)]).await;
        let failure = refused(&route(&stub.url)).await;
        assert_eq!(
            (failure.kind, failure.status, failure.retry_after_seconds),
            (kind, Some(status), retry_after_seconds),
            "{case}: {}",
            failure.details
        );
    }
}

/// The failure a turn's streamed request on `model` ends in, whether the
/// request is refused or its stream is.
async fn refused(model: &DynModel<Completion>) -> Failure {
    let request = rig::completion::CompletionRequest::new("hello")
        .preamble(PREAMBLE)
        .max_tokens(64);
    match model.stream(request) {
        Err(error) => Failure::provider(error, "request"),
        Ok(mut stream) => loop {
            match stream.next().await {
                Some(Err(error)) => break Failure::provider(error, "stream"),
                Some(Ok(_)) => {}
                None => panic!("the stream ended without a refusal"),
            }
        },
    }
}
