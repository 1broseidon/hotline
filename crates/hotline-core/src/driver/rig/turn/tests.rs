use super::*;
use crate::contract::TokenUsage;
use rig::ProviderError;
use rig::completion::Usage;
use rig::driver::{Exchange, Opened, Opening, Transport};
use rig::test_utils::{MockFrame, MockScript, MockStreamEvent, mock_final};
use std::collections::VecDeque;
use std::time::Duration;

type Chunks = mpsc::Receiver<Result<MockStreamEvent, ProviderError>>;

/// Rig's scripted completion wire, on a runtime each test feeds by hand: a
/// request is recorded, then answered with the next stream the test opened,
/// one event at a time as the test sends them. A `None` stream never answers.
/// As over HTTP, nothing is sent until the reply is first read.
#[derive(Clone)]
struct ScriptModel {
    requests: mpsc::UnboundedSender<CompletionRequest>,
    streams: Arc<Mutex<VecDeque<Option<Chunks>>>>,
}

impl Transport<MockScript> for ScriptModel {
    fn send(&self, request: CompletionRequest, _: Exchange) -> Opening<MockFrame> {
        let this = self.clone();
        Opening::new(async move {
            this.requests.send(request).unwrap();
            let next = lock(&this.streams)
                .pop_front()
                .expect("unexpected model request");
            let Some(chunks) = next else {
                return std::future::pending().await;
            };
            Ok(Opened::new(futures_util::stream::unfold(
                chunks,
                |mut chunks| async move {
                    let item = chunks.recv().await?;
                    Some((item.map(MockFrame::Event), chunks))
                },
            )))
        })
    }
}

/// One part of what was said, with who said it.
#[derive(Debug, PartialEq)]
enum Said {
    User(UserContent),
    Assistant(AssistantContent),
}

/// What was said in a request after its system prompt, one entry per part:
/// a wire joins adjacent user messages, so a turn's messages are compared by
/// what each said and who said it, not by how they were grouped.
fn said(request: &CompletionRequest) -> Vec<Said> {
    let messages: Vec<Message> = request
        .chat_history
        .iter()
        .map(|message| Message::clone(message))
        .collect();
    assert!(
        matches!(messages.first(), Some(Message::System { .. })),
        "every round leads with the system prompt"
    );
    parts(&messages[1..])
}

/// `messages` one part at a time, with who said each.
fn parts(messages: &[Message]) -> Vec<Said> {
    let mut said = Vec::new();
    for message in messages {
        match message {
            Message::System { .. } => panic!("the system prompt leads, and only once"),
            Message::User { content } => said.extend(content.iter().cloned().map(Said::User)),
            Message::Assistant(turn) => {
                said.extend(turn.content.iter().cloned().map(Said::Assistant));
            }
        }
    }
    said
}

/// The model a test's turn runs on: requests go to `requests`, and each is
/// answered by the next of `streams`.
fn scripted(
    requests: mpsc::UnboundedSender<CompletionRequest>,
    streams: impl IntoIterator<Item = Option<Chunks>>,
) -> DynModel<Completion> {
    rig::Model::new(
        MockScript::default(),
        ScriptModel {
            requests,
            streams: Arc::new(Mutex::new(streams.into_iter().collect())),
        },
    )
    .erase()
}

fn fixture() -> (Turn, Template) {
    let turn = Turn {
        keys: HashMap::new(),
        model: "test/model".into(),
        output_limit: None,
        context_limit: None,
        effort: None,
        preamble: "Follow the operator".into(),
        cwd: PathBuf::new(),
        folders: Vec::new(),
        reach: Reach::Machine,
        history: Arc::new(AsyncMutex::new(vec![Message::user("first request")])),
        stop: Arc::new(Stop::default()),
        steering: Arc::new(Steering::default()),
        output_dir: std::env::temp_dir(),
        mcp_tools: Vec::new(),
        capability: None,
        delegate: None,
        meter: None,
        processes: crate::session::jobs::Processes::new(std::env::temp_dir()),
    };
    let template = Template {
        tools: Vec::new(),
        settings: RequestSettings::default(),
    };
    (turn, template)
}

async fn receive<T>(receiver: &mut mpsc::UnboundedReceiver<T>) -> T {
    tokio::time::timeout(Duration::from_secs(3), receiver.recv())
        .await
        .unwrap()
        .unwrap()
}

async fn answer(sender: mpsc::Sender<Result<MockStreamEvent, ProviderError>>) {
    sender
        .send(Ok(MockStreamEvent::Text("the new direction".into())))
        .await
        .unwrap();
    sender
        .send(Ok(MockStreamEvent::FinalResponse(mock_final(
            Usage::default(),
        ))))
        .await
        .unwrap();
}

/// A budget on a scratch desk, with `spending` as the room's setting.
fn chat_budget(
    spending: serde_json::Value,
) -> (tempfile::TempDir, Arc<crate::voice::metering::Budget>) {
    let root = tempfile::tempdir().unwrap();
    let log = crate::log::Log::open(root.path());
    log.append(
        &crate::log::StreamId::Room,
        &crate::room::room_event("setting", json!({"id": "spending", "value": spending})),
    )
    .unwrap();
    let budget = Arc::new(crate::voice::metering::Budget::open(log));
    (root, budget)
}

fn metered(budget: &Arc<crate::voice::metering::Budget>) -> ChatMeter {
    ChatMeter {
        budget: budget.clone(),
        price: crate::contract::ModelCost {
            input: 1.0,
            output: 4.0,
            cache_read: Some(0.1),
            cache_write: None,
        },
    }
}

#[tokio::test]
async fn a_paid_round_is_charged_exactly_what_its_usage_cost() {
    let (_root, budget) = chat_budget(json!({"chat": {"dayUsd": 5}}));
    let (mut turn, request) = fixture();
    turn.meter = Some(metered(&budget));
    let (seen, mut requests) = mpsc::unbounded_channel();
    let (chunks, stream) = mpsc::channel(8);
    let model = scripted(seen, [Some(stream)]);
    let (updates, _receiver) = mpsc::channel(64);
    let reserved = budget.clone();
    let task = tokio::spawn(async move {
        run(&model, request, &ToolSet::default(), &turn, &updates, None).await
    });
    receive(&mut requests).await;
    // In flight, the round holds its estimate: the whole output ceiling.
    let estimate = reserved.spent().unwrap().day.teammates;
    assert!(
        estimate > (UNCAPPED_OUTPUT as f64 * 4.0) / 1e6,
        "{estimate}"
    );
    chunks
        .send(Ok(MockStreamEvent::Text("done".into())))
        .await
        .unwrap();
    // 1,000 tokens in, 400 of them read from the cache; 200 out.
    let usage = Usage::new()
        .input_tokens(1_000)
        .output_tokens(200)
        .total_tokens(1_200)
        .cached_input_tokens(400);
    chunks
        .send(Ok(MockStreamEvent::FinalResponse(mock_final(usage))))
        .await
        .unwrap();
    drop(chunks);
    task.await.unwrap().unwrap();
    let charged = budget.spent().unwrap().day.teammates;
    let expected = (600.0 * 1.0 + 400.0 * 0.1 + 200.0 * 4.0) / 1e6;
    assert!(
        (charged - expected).abs() < 1e-15,
        "{charged} is not {expected}"
    );
}

#[tokio::test]
async fn a_spent_chat_budget_refuses_the_turn_before_any_request() {
    let (_root, budget) = chat_budget(json!({"chat": {"dayUsd": 1}}));
    budget.charge(crate::voice::ledger::Kind::Dispatcher, 1.0);
    let (mut turn, request) = fixture();
    turn.meter = Some(metered(&budget));
    let (seen, mut requests) = mpsc::unbounded_channel();
    let model = scripted(seen, []);
    let (updates, _receiver) = mpsc::channel(64);
    let failure = run(&model, request, &ToolSet::default(), &turn, &updates, None)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, Kind::Budget);
    assert_eq!(failure.title, "Chat budget spent");
    assert_eq!(
        failure.details,
        "The Chat budget for today is spent. Raise it in Settings › Budgets."
    );
    assert!(failure.retry_delay(0).is_none());
    assert!(failure.notice().contains(r#""kind":"budget""#));
    assert!(requests.try_recv().is_err(), "nothing was sent");
    assert_eq!(budget.spent().unwrap().day.teammates, 0.0);
}

#[test]
fn subscriptions_local_servers_and_desks_without_budgets_are_not_metered() {
    let (_root, budget) = chat_budget(json!({"chat": {"dayUsd": 0}}));
    let metadata = HashMap::new();
    let login = HashMap::from([(
        "github-copilot".to_string(),
        ProviderAuth::Login {
            token_dir: PathBuf::from("/nowhere"),
        },
    )]);
    assert!(
        ChatMeter::for_model(
            Some(budget.clone()),
            &login,
            "github-copilot/claude-opus-5.5",
            &metadata
        )
        .is_none()
    );
    let local = HashMap::from([(
        "ollama".to_string(),
        ProviderAuth::Local {
            base_url: "http://127.0.0.1:11434".into(),
        },
    )]);
    assert!(
        ChatMeter::for_model(Some(budget.clone()), &local, "ollama/llama3", &metadata).is_none()
    );
    let key = HashMap::from([("anthropic".to_string(), ProviderAuth::ApiKey("sk".into()))]);
    assert!(ChatMeter::for_model(None, &key, "anthropic/claude-haiku-5-5", &metadata).is_none());
    let meter =
        ChatMeter::for_model(Some(budget), &key, "anthropic/claude-haiku-5-5", &metadata).unwrap();
    assert_eq!((meter.price.input, meter.price.output), (0.1, 0.5));
}

#[tokio::test]
async fn a_turn_on_a_sign_in_runs_whatever_the_chat_budget_says() {
    let (_root, budget) = chat_budget(json!({"chat": {"dayUsd": 0}}));
    let (mut turn, request) = fixture();
    turn.keys = HashMap::from([(
        "github-copilot".to_string(),
        ProviderAuth::Login {
            token_dir: PathBuf::from("/nowhere"),
        },
    )]);
    turn.model = "github-copilot/claude-opus-5.5".into();
    turn.meter = ChatMeter::for_model(
        Some(budget.clone()),
        &turn.keys,
        &turn.model,
        &HashMap::new(),
    );
    let (seen, mut requests) = mpsc::unbounded_channel();
    let (chunks, stream) = mpsc::channel(8);
    let model = scripted(seen, [Some(stream)]);
    let (updates, _receiver) = mpsc::channel(64);
    let task = tokio::spawn(async move {
        run(&model, request, &ToolSet::default(), &turn, &updates, None).await
    });
    receive(&mut requests).await;
    answer(chunks).await;
    task.await.unwrap().unwrap();
    assert_eq!(budget.spent().unwrap().day.teammates, 0.0);
}

#[tokio::test]
async fn input_interrupts_a_request_before_the_first_stream_exists() {
    let (turn, request) = fixture();
    let steering = turn.steering.clone();
    let (seen, mut requests) = mpsc::unbounded_channel();
    let (chunks, stream) = mpsc::channel(8);
    let model = scripted(seen, [None, Some(stream)]);
    let (updates, mut receiver) = mpsc::channel(64);
    let task = tokio::spawn(async move {
        run(&model, request, &ToolSet::default(), &turn, &updates, None).await
    });
    receive(&mut requests).await;
    assert!(steering.admit(Message::user("actually, do the other thing")));
    assert!(steering.admit(Message::user("keep the scope small")));
    let next = receive(&mut requests).await;
    assert_eq!(
        said(&next),
        parts(&[
            Message::user("first request"),
            Message::user("actually, do the other thing"),
            Message::user("keep the scope small")
        ])
    );
    answer(chunks).await;
    task.await.unwrap().unwrap();
    assert!(!steering.admit(Message::user("after completion")));
    let mut ends = 0;
    while let Some(update) = receiver.recv().await {
        if let Update::Turn { stop_reason, usage } = update {
            assert_eq!(stop_reason, "end_turn");
            assert!(usage.is_none(), "cancelled request usage is unknown");
            ends += 1;
        }
    }
    assert_eq!(ends, 1, "steering is not another logical turn");
}

#[tokio::test]
async fn partial_calls_are_dropped_and_never_replayed_or_executed() {
    let (turn, request) = fixture();
    let steering = turn.steering.clone();
    let (seen, mut requests) = mpsc::unbounded_channel();
    let (old, old_stream) = mpsc::channel(8);
    let (new, new_stream) = mpsc::channel(8);
    let model = scripted(seen, [Some(old_stream), Some(new_stream)]);
    let (updates, mut receiver) = mpsc::channel(64);
    let task = tokio::spawn(async move {
        run(&model, request, &ToolSet::default(), &turn, &updates, None).await
    });
    receive(&mut requests).await;
    old.send(Ok(MockStreamEvent::tool_call_name_delta(
        "partial", "shell",
    )))
    .await
    .unwrap();
    old.send(Ok(MockStreamEvent::Text("unfinished".into())))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), receiver.recv())
        .await
        .unwrap();
    assert!(steering.admit(Message::user("change direction")));
    let next = receive(&mut requests).await;
    assert!(old.is_closed(), "the old provider stream is dropped");
    assert_eq!(
        said(&next),
        parts(&[
            Message::user("first request"),
            Message::assistant("unfinished"),
            Message::user(KEPT),
            Message::user("change direction")
        ])
    );
    answer(new).await;
    task.await.unwrap().unwrap();
    while let Some(update) = receiver.recv().await {
        assert!(!matches!(update, Update::ToolCall { .. }));
    }
}

#[tokio::test]
async fn on_a_strict_route_an_unused_argument_is_null_and_the_tool_never_sees_it() {
    let (mut turn, mut request) = fixture();
    let login = tempfile::tempdir().unwrap();
    turn.keys = HashMap::from([(
        "github-copilot".to_string(),
        ProviderAuth::Login {
            token_dir: login.path().to_path_buf(),
        },
    )]);
    turn.model = "github-copilot/gpt-5.3-codex".into();
    let (called, mut calls) = mpsc::unbounded_channel();
    let mut tools = ToolSet::default();
    tools.add_dynamic_tool(DynamicTool::new(
        rig::message::ToolName::new("fill").unwrap(),
        "Fill a field with text or a secret",
        json!({
            "type": "object",
            "properties": {
                "selector": {"type": "string"},
                "text": {"type": "string"},
                "secret": {"type": "string"}
            },
            "required": ["selector"]
        }),
        move |arguments| {
            called.send(arguments).unwrap();
            Box::pin(async { Ok(ToolOutput::text("filled")) })
        },
    ));
    request.tools = tools.tool_definitions();
    let (seen, mut requests) = mpsc::unbounded_channel();
    let (first, first_stream) = mpsc::channel(8);
    let (last, last_stream) = mpsc::channel(8);
    let model = scripted(seen, [Some(first_stream), Some(last_stream)]);
    let (updates, _receiver) = mpsc::channel(64);
    let task =
        tokio::spawn(async move { run(&model, request, &tools, &turn, &updates, None).await });
    let sent = receive(&mut requests).await;
    let schema = &sent.tools[0].parameters["properties"];
    assert_eq!(schema["selector"]["type"], "string");
    assert_eq!(schema["text"]["type"], json!(["string", "null"]));
    assert_eq!(schema["secret"]["type"], json!(["string", "null"]));
    first
        .send(Ok(MockStreamEvent::tool_call(
            "fill",
            "fill",
            json!({"selector": "#password", "text": null, "secret": "github"}),
        )))
        .await
        .unwrap();
    first
        .send(Ok(MockStreamEvent::FinalResponse(mock_final(
            Usage::default(),
        ))))
        .await
        .unwrap();
    drop(first);
    assert_eq!(
        receive(&mut calls).await,
        json!({"selector": "#password", "secret": "github"})
    );
    receive(&mut requests).await;
    answer(last).await;
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn steering_keeps_a_running_tools_result_and_skips_its_unstarted_siblings() {
    let (turn, request) = fixture();
    let steering = turn.steering.clone();
    let kept = turn.history.clone();
    let (seen, mut requests) = mpsc::unbounded_channel();
    let (old, old_stream) = mpsc::channel(8);
    let (new, new_stream) = mpsc::channel(8);
    let model = scripted(seen, [Some(old_stream), Some(new_stream)]);
    let (started, mut starts) = mpsc::unbounded_channel();
    let release = Arc::new(Notify::new());
    let gate = release.clone();
    let mut tools = ToolSet::default();
    tools.add_dynamic_tool(DynamicTool::new(
        rig::message::ToolName::new("slow").unwrap(),
        "A synchronous tool with an observable effect",
        serde_json::json!({"type": "object"}),
        move |_arguments| {
            let started = started.clone();
            let gate = gate.clone();
            Box::pin(async move {
                started.send(()).unwrap();
                gate.notified().await;
                Ok(ToolOutput::text("the first effect completed"))
            })
        },
    ));
    let (updates, _receiver) = mpsc::channel(64);
    let task =
        tokio::spawn(async move { run(&model, request, &tools, &turn, &updates, None).await });
    receive(&mut requests).await;
    for id in ["first", "second"] {
        old.send(Ok(MockStreamEvent::tool_call(
            id,
            "slow",
            serde_json::json!({}),
        )))
        .await
        .unwrap();
    }
    old.send(Ok(MockStreamEvent::FinalResponse(mock_final(
        Usage::default(),
    ))))
    .await
    .unwrap();
    drop(old);
    receive(&mut starts).await;
    assert!(steering.admit(Message::user("change direction")));
    assert!(requests.try_recv().is_err());
    release.notify_one();
    receive(&mut requests).await;
    assert!(starts.try_recv().is_err(), "the second call never started");
    // What the next request was written from: the request, the two calls,
    // their two results, the new direction.
    let history = parts(&kept.lock().await);
    assert_eq!(history.len(), 6, "{history:?}");
    assert_eq!(
        history[5],
        Said::User(UserContent::text("change direction"))
    );
    let mut results = Vec::new();
    for (call, result) in history[1..3].iter().zip(&history[3..5]) {
        let Said::Assistant(AssistantContent::ToolCall(call)) = call else {
            panic!("the completed tool calls are retained: {call:?}");
        };
        let Said::User(UserContent::ToolResult(result)) = result else {
            panic!("expected its result: {result:?}");
        };
        assert_eq!(call.id, result.call);
        results.push(
            result
                .content
                .iter()
                .filter_map(rig::message::ToolResultContent::as_text)
                .collect::<String>(),
        );
    }
    assert!(results[0].contains("the first effect completed"));
    assert!(results[1].contains("Not executed"));
    answer(new).await;
    task.await.unwrap().unwrap();
}

/// A tool that does what it is asked, for turns whose shape is what is
/// under test rather than the tool.
struct Nod;

impl rig::tool::Tool for Nod {
    const NAME: &'static str = "react";
    type Error = crate::tools::ToolError;
    type Args = serde_json::Value;
    type Output = String;

    fn description(&self) -> String {
        "nods".into()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    fn map_error(&self, error: Self::Error) -> rig::tool::ToolExecutionError {
        error.into_execution_error()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        _args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        Ok("Reacted.".into())
    }
}

/// One tool round, then a completion with nothing in it. `tools` decides
/// whether the round succeeded; the updates say how the turn read it.
async fn tool_then_silence(tools: ToolSet) -> Vec<Update> {
    let (turn, request) = fixture();
    let (seen, mut requests) = mpsc::unbounded_channel();
    let (first, stream_one) = mpsc::channel(8);
    let (second, stream_two) = mpsc::channel(8);
    let model = scripted(seen, [Some(stream_one), Some(stream_two)]);
    let (updates, mut receiver) = mpsc::channel(64);
    let task =
        tokio::spawn(async move { run(&model, request, &tools, &turn, &updates, None).await });
    receive(&mut requests).await;
    first
        .send(Ok(MockStreamEvent::tool_call(
            "react-1",
            "react",
            serde_json::json!({ "emoji": "👍" }),
        )))
        .await
        .unwrap();
    first
        .send(Ok(MockStreamEvent::FinalResponse(mock_final(
            Usage::default(),
        ))))
        .await
        .unwrap();
    drop(first);
    receive(&mut requests).await;
    second
        .send(Ok(MockStreamEvent::FinalResponse(mock_final(
            Usage::default(),
        ))))
        .await
        .unwrap();
    drop(second);
    task.await.unwrap().unwrap();
    let mut seen = Vec::new();
    while let Some(update) = receiver.recv().await {
        seen.push(update);
    }
    seen
}

fn ended_with(updates: &[Update]) -> Option<&str> {
    updates.iter().find_map(|update| match update {
        Update::Turn { stop_reason, .. } => Some(stop_reason.as_str()),
        _ => None,
    })
}

fn notices(updates: &[Update]) -> Vec<&str> {
    updates
        .iter()
        .filter_map(|update| match update {
            Update::Notice { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// A reply that is only a reaction: the model calls the tool, hears the
/// result, and has nothing to add. That is a finished turn, not a stream
/// that died, and nothing is said about it.
#[tokio::test]
async fn a_turn_that_ends_after_a_tool_with_nothing_to_say_is_complete() {
    let mut tools = ToolSet::default();
    tools.add_tool(Nod);
    let updates = tool_then_silence(tools).await;
    assert!(updates.iter().any(|u| matches!(u, Update::ToolCall { .. })));
    assert_eq!(notices(&updates), Vec::<&str>::new());
    assert_eq!(ended_with(&updates), Some("end_turn"));
}

/// The tool failed and the model gave up without a word: the turn still
/// ends, and one quiet line says so, pointing at the step.
#[tokio::test]
async fn silence_after_a_failed_step_is_said_out_loud() {
    let updates = tool_then_silence(ToolSet::default()).await;
    assert!(
        updates
            .iter()
            .any(|u| matches!(u, Update::ToolResult { ok: false, .. }))
    );
    assert_eq!(
        notices(&updates),
        ["Stopped after a failed step and said nothing."]
    );
    assert_eq!(ended_with(&updates), Some("end_turn"));
}

#[tokio::test]
async fn a_truncated_response_never_dispatches_its_tools() {
    let (turn, request) = fixture();
    let history = turn.history.clone();
    let (seen, mut requests) = mpsc::unbounded_channel();
    let (chunks, stream) = mpsc::channel(8);
    let model = scripted(seen, [Some(stream)]);
    let (updates, mut receiver) = mpsc::channel(64);
    let task = tokio::spawn(async move {
        run(&model, request, &ToolSet::default(), &turn, &updates, None).await
    });
    receive(&mut requests).await;
    chunks
        .send(Ok(MockStreamEvent::tool_call(
            "truncated",
            "shell",
            serde_json::json!({}),
        )))
        .await
        .unwrap();
    let mut terminal = mock_final(Usage::default());
    terminal.reason = Some(FinishReason::Length);
    chunks
        .send(Ok(MockStreamEvent::FinalResponse(terminal)))
        .await
        .unwrap();
    drop(chunks);
    assert!(
        task.await
            .unwrap()
            .unwrap_err()
            .details
            .contains("truncated")
    );
    assert_eq!(*history.lock().await, [Message::user("first request")]);
    while let Some(update) = receiver.recv().await {
        assert!(!matches!(update, Update::ToolCall { .. }));
    }
}

/// A megabyte of build log reaches the model once, as a few KB with both
/// streams' ends and where the rest is, and not again in wait_jobs.
#[cfg(unix)]
#[tokio::test]
async fn a_long_build_reaches_the_model_once_as_its_ends_and_where_the_rest_is() {
    let root = tempfile::tempdir().unwrap();
    let outputs = root.path().join("outputs");
    let shell = RunCommand::new(
        Workspace::open(root.path().into(), Reach::Machine, outputs.clone()).unwrap(),
    );
    let (mut turn, request) = fixture();
    turn.output_dir = outputs;
    let (seen, mut requests) = mpsc::unbounded_channel();
    let (first, first_stream) = mpsc::channel(8);
    let (second, second_stream) = mpsc::channel(8);
    let (third, third_stream) = mpsc::channel(8);
    let model = scripted(
        seen,
        [Some(first_stream), Some(second_stream), Some(third_stream)],
    );
    let (updates, mut receiver) = mpsc::channel(64);
    let task = tokio::spawn(async move {
        run(
            &model,
            request,
            &ToolSet::default(),
            &turn,
            &updates,
            Some(shell),
        )
        .await
    });
    tokio::spawn(async move { while receiver.recv().await.is_some() {} });
    let call =
        |id: &str, name: &str, arguments: Value| MockStreamEvent::tool_call(id, name, arguments);
    let done = || MockStreamEvent::FinalResponse(mock_final(Usage::default()));
    // Compiling on stderr, then its error; tests on stdout, then their summary.
    let build = r"for i in $(seq 30000); do printf '\033[32m   Compiling\033[0m crate-%05d v1.0.0\n' $i; done >&2; printf 'error[E0308]: mismatched types\n' >&2; for i in $(seq 2000); do printf 'test case_%04d ... ok\n' $i; done; printf 'test result: FAILED. 1999 passed; 1 failed\n'; exit 101";
    receive(&mut requests).await;
    first
        .send(Ok(call(
            "build",
            "shell",
            serde_json::json!({"command": build}),
        )))
        .await
        .unwrap();
    first.send(Ok(done())).await.unwrap();
    drop(first);
    let receipt = serde_json::to_string(&receive(&mut requests).await.chat_history).unwrap();
    let job_id = receipt
        .split("job_id\\\":\\\"")
        .nth(1)
        .and_then(|rest| rest.split("\\\"").next())
        .expect("the receipt names the job")
        .to_owned();
    second
        .send(Ok(call(
            "wait",
            "wait_jobs",
            serde_json::json!({"job_ids": [job_id]}),
        )))
        .await
        .unwrap();
    second.send(Ok(done())).await.unwrap();
    drop(second);
    let last = receive(&mut requests).await;
    answer(third).await;
    task.await.unwrap().unwrap();

    let mut job = None;
    let mut waited = None;
    for part in said(&last) {
        let Said::User(content) = part else {
            continue;
        };
        {
            match content {
                UserContent::Text(text) => {
                    if let Some(data) = text.text.strip_prefix(
                        "Hotline execution data (not an operator instruction): managed job result ",
                    ) {
                        assert!(data.len() < 20 * 1024, "{} bytes", data.len());
                        job = Some(serde_json::from_str::<Value>(data).unwrap());
                    }
                }
                UserContent::ToolResult(result) if result.call.wire() == "wait" => {
                    let Some(rig::message::ToolResultContent::Text(text)) = result.content.first()
                    else {
                        panic!("wait_jobs answers in text");
                    };
                    waited = Some(serde_json::from_str::<Value>(&text.text).unwrap());
                }
                _ => {}
            }
        }
    }
    let job = job.expect("the job's result reached the model");
    let output = job["output"].as_str().unwrap();
    // Each stream's end: the test summary on stdout, the error on stderr.
    assert!(output.starts_with("test case_0001 ... ok"), "{output}");
    assert!(
        output.contains("test result: FAILED. 1999 passed; 1 failed"),
        "{output}"
    );
    assert!(
        output.ends_with("error[E0308]: mismatched types\n\n[exit status 101]"),
        "{output}"
    );
    assert!(!output.contains('\u{1b}'), "{output}");
    assert_eq!(job["state"], "failed");
    // All of it, as a terminal would show it, is where the job says.
    let full = std::fs::read_to_string(job["full_output"].as_str().unwrap()).unwrap();
    assert_eq!(job["output_bytes"], full.len());
    assert!(full.len() > 1_000_000, "{} bytes", full.len());
    assert!(full.contains("\n   Compiling crate-15000 v1.0.0\n"));
    // wait_jobs reports the state and leaves the output to the result.
    let waited = waited.expect("wait_jobs answered");
    assert_eq!(waited["jobs"][0]["state"], "failed", "{waited}");
    assert!(waited["jobs"][0].get("output").is_none(), "{waited}");
}

#[cfg(unix)]
#[tokio::test]
async fn a_side_question_preserves_the_job_and_stop_waits_for_its_exit() {
    check_job_lifetime(false).await;
}

#[cfg(unix)]
#[tokio::test]
async fn an_inference_failure_cleans_up_the_owned_shell_job() {
    check_job_lifetime(true).await;
}

#[cfg(unix)]
async fn check_job_lifetime(fail_inference: bool) {
    let root = tempfile::tempdir().unwrap();
    let shell = RunCommand::new(
        Workspace::open(
            root.path().into(),
            Reach::Machine,
            root.path().join("outputs"),
        )
        .unwrap(),
    );
    let (mut turn, request) = fixture();
    turn.output_dir = root.path().join("outputs");
    let steering = turn.steering.clone();
    let stop = turn.stop.clone();
    let (seen, mut requests) = mpsc::unbounded_channel();
    let (first, first_stream) = mpsc::channel(8);
    let (second, second_stream) = mpsc::channel(8);
    let (third, third_stream) = mpsc::channel(8);
    let model = scripted(
        seen,
        [Some(first_stream), Some(second_stream), Some(third_stream)],
    );
    let (updates, mut receiver) = mpsc::channel(64);
    let task = tokio::spawn(async move {
        run(
            &model,
            request,
            &ToolSet::default(),
            &turn,
            &updates,
            Some(shell),
        )
        .await
    });
    let request = receive(&mut requests).await;
    assert!(request.tools.iter().any(|tool| tool.name == "cancel_job"));
    first.send(Ok(MockStreamEvent::tool_call(
        "owned_shell", "shell", serde_json::json!({"command":"echo started; while true; do echo x >> heartbeat; sleep 0.05; done & wait"})
    ))).await.unwrap();
    first
        .send(Ok(MockStreamEvent::FinalResponse(mock_final(
            Usage::default(),
        ))))
        .await
        .unwrap();
    drop(first);
    receive(&mut requests).await;
    let heartbeat = root.path().join("heartbeat");
    tokio::time::timeout(Duration::from_secs(3), async {
        while !std::fs::metadata(&heartbeat).is_ok_and(|file| file.len() > 0) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let mut observed = Vec::new();
    if fail_inference {
        second
            .send(Err(ProviderError::ProviderResponse(
                rig::ProviderResponseError::new(
                    http::StatusCode::TOO_MANY_REQUESTS,
                    r#"{"error":{"code":"insufficient_quota"}}"#,
                ),
            )))
            .await
            .unwrap();
        drop(second);
    } else {
        answer(second).await;
        loop {
            let update = tokio::time::timeout(Duration::from_secs(3), receiver.recv())
                .await
                .unwrap()
                .unwrap();
            let answered = matches!(
                update,
                Update::Message {
                    kind: MessageKind::Agent,
                    ..
                }
            );
            observed.push(update);
            if answered {
                break;
            }
        }
        assert!(
            !task.is_finished(),
            "a text answer cannot end an activity with a running job"
        );
        assert!(steering.admit(Message::user(
            "what is the status? Keep the command running"
        )));
        let request = receive(&mut requests).await;
        assert!(
            serde_json::to_string(&request.chat_history)
                .unwrap()
                .contains("active managed jobs")
        );
        answer(third).await;
        // The job writes every 50 ms; a loaded runner can stall it for
        // longer than that, so wait for growth rather than sample it.
        let before = std::fs::metadata(&heartbeat).unwrap().len();
        tokio::time::timeout(Duration::from_secs(3), async {
            while std::fs::metadata(&heartbeat).unwrap().len() <= before {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("a side question must not cancel the job");
        assert!(!task.is_finished());
        assert!(
            requests.try_recv().is_err(),
            "waiting for a job must not spin through model requests"
        );
        stop.raise();
    }
    let result = tokio::time::timeout(Duration::from_secs(6), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.is_err(), fail_inference);
    while let Some(update) = receiver.recv().await {
        observed.push(update);
    }
    let ended = observed
        .iter()
        .position(|update| matches!(update, Update::Turn { .. }));
    let cancelled = observed.iter().position(|update| matches!(update, Update::ToolResult { output, .. } if output.contains("Job Cancelled") && output.contains("started"))).expect("owned shell cancellation must be observed");
    if !fail_inference {
        assert!(
            cancelled < ended.unwrap(),
            "cancellation settles before the activity ends"
        );
        assert_eq!(
            observed
                .iter()
                .filter(|update| matches!(update, Update::Turn { .. }))
                .count(),
            1
        );
    }
    let before = std::fs::read(&heartbeat).unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(std::fs::read(&heartbeat).unwrap(), before);
}

#[test]
fn closing_admission_retains_existing_input_and_refuses_new_input() {
    let steering = Steering::default();
    assert!(steering.admit(Message::user("already admitted")));
    steering.close_admission();
    assert!(!steering.admit(Message::user("must queue after shutdown")));
    assert_eq!(
        steering.close(),
        [Input::Prepared(Message::user("already admitted"))]
    );
}

#[test]
fn completion_and_admission_have_one_order() {
    let steering = Steering::default();
    assert!(steering.admit(Message::user("before completion")));
    assert!(!steering.finish_if_empty());
    assert_eq!(
        steering.take().1,
        [Input::Prepared(Message::user("before completion"))]
    );
    assert!(steering.finish_if_empty());
    assert!(!steering.admit(Message::user("must stay in the session queue")));
}
#[tokio::test]
async fn a_transient_failure_after_tools_retries_only_the_inference() {
    let (turn, request) = fixture();
    let history = turn.history.clone();
    let (seen, mut requests) = mpsc::unbounded_channel();
    let (first, a) = mpsc::channel(8);
    let (second, b) = mpsc::channel(8);
    let (third, c) = mpsc::channel(8);
    let model = scripted(seen, [Some(a), Some(b), Some(c)]);
    let mut tools = ToolSet::default();
    tools.add_tool(Nod);
    let (updates, mut receiver) = mpsc::channel(64);
    let task =
        tokio::spawn(async move { run(&model, request, &tools, &turn, &updates, None).await });
    receive(&mut requests).await;
    first
        .send(Ok(MockStreamEvent::tool_call(
            "done-once",
            "react",
            serde_json::json!({"emoji":"👍"}),
        )))
        .await
        .unwrap();
    first
        .send(Ok(MockStreamEvent::FinalResponse(mock_final(
            Usage::default(),
        ))))
        .await
        .unwrap();
    drop(first);
    let before = receive(&mut requests).await;
    second
        .send(Ok(MockStreamEvent::Text("partial response".into())))
        .await
        .unwrap();
    second
        .send(Err(ProviderError::ProviderResponse(
            rig::ProviderResponseError::new(http::StatusCode::SERVICE_UNAVAILABLE, "busy"),
        )))
        .await
        .unwrap();
    drop(second);
    let retry = receive(&mut requests).await;
    let mut expected = said(&before);
    expected.extend(parts(&[
        Message::assistant("partial response"),
        Message::user(KEPT),
    ]));
    assert_eq!(said(&retry), expected);
    answer(third).await;
    task.await.unwrap().unwrap();
    let mut calls = 0;
    while let Some(update) = receiver.recv().await {
        if matches!(update, Update::ToolCall { .. }) {
            calls += 1;
        }
    }
    assert_eq!(calls, 1);
    assert_eq!(history.lock().await.iter().filter(|m| matches!(m, Message::User { content } if content.iter().any(|c| matches!(c, UserContent::ToolResult(_))))).count(), 1);
}

/// The standup that posted three times: a tool round, then the report
/// streams to the person and the stream ends without its terminal record.
/// The retry must know the report was delivered, or it writes it again.
#[tokio::test]
async fn a_report_shown_before_the_stream_dropped_is_remembered_on_retry() {
    let (turn, request) = fixture();
    let history = turn.history.clone();
    let (seen, mut requests) = mpsc::unbounded_channel();
    let (first, a) = mpsc::channel(8);
    let (second, b) = mpsc::channel(8);
    let (third, c) = mpsc::channel(8);
    let model = scripted(seen, [Some(a), Some(b), Some(c)]);
    let mut tools = ToolSet::default();
    tools.add_tool(Nod);
    let (updates, mut receiver) = mpsc::channel(64);
    let task =
        tokio::spawn(async move { run(&model, request, &tools, &turn, &updates, None).await });
    receive(&mut requests).await;
    first
        .send(Ok(MockStreamEvent::tool_call(
            "schedule-1",
            "react",
            serde_json::json!({"emoji":"👍"}),
        )))
        .await
        .unwrap();
    first
        .send(Ok(MockStreamEvent::FinalResponse(mock_final(
            Usage::default(),
        ))))
        .await
        .unwrap();
    drop(first);
    receive(&mut requests).await;
    second
        .send(Ok(MockStreamEvent::Text("Today — Mon 5 Oct".into())))
        .await
        .unwrap();
    drop(second);
    let retry = receive(&mut requests).await;
    let retried = said(&retry);
    assert_eq!(
        retried[retried.len() - 2..],
        parts(&[Message::assistant("Today — Mon 5 Oct"), Message::user(KEPT)])
    );
    answer(third).await;
    task.await.unwrap().unwrap();
    assert!(
        history
            .lock()
            .await
            .contains(&Message::assistant("Today — Mon 5 Oct"))
    );
    let mut said = 0;
    while let Some(update) = receiver.recv().await {
        if matches!(&update, Update::Message { kind: MessageKind::Agent, text, .. } if text == "Today — Mon 5 Oct")
        {
            said += 1;
        }
    }
    assert_eq!(said, 1);
}

#[tokio::test(start_paused = true)]
async fn stop_during_backoff_is_terminal() {
    let (turn, request) = fixture();
    let stop = turn.stop.clone();
    let (seen, mut requests) = mpsc::unbounded_channel();
    let (first, stream) = mpsc::channel(8);
    let model = scripted(seen, [Some(stream)]);
    let (updates, _receiver) = mpsc::channel(64);
    let task = tokio::spawn(async move {
        run(&model, request, &ToolSet::default(), &turn, &updates, None).await
    });
    receive(&mut requests).await;
    first
        .send(Err(ProviderError::ProviderResponse(
            rig::ProviderResponseError::new(http::StatusCode::TOO_MANY_REQUESTS, "rate limit"),
        )))
        .await
        .unwrap();
    drop(first);
    // Backing off says nothing. A paused clock moves only once every task
    // is parked, and the backoff is a second at least, so this returns with
    // the turn waiting it out.
    tokio::time::sleep(Duration::from_millis(100)).await;
    stop.raise();
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(requests.try_recv().is_err());
}

#[tokio::test]
async fn a_failed_repair_stays_repaired_for_the_next_operator_message() {
    let (turn, request) = fixture();
    let history = turn.history.clone();
    history
        .lock()
        .await
        .push(Message::Assistant(rig::message::AssistantMessage::new(
            vec![AssistantContent::reasoning("").with_native(json!({
                "type": "reasoning",
                "id": "bad:reasoning:item",
                "encrypted_content": "opaque-state",
            }))],
        )));
    let (seen, mut requests) = mpsc::unbounded_channel();
    let (first, a) = mpsc::channel(8);
    let (second, b) = mpsc::channel(8);
    let model = scripted(seen, [Some(a), Some(b)]);
    let (updates, mut receiver) = mpsc::channel(64);
    let task = tokio::spawn(async move {
        run(&model, request, &ToolSet::default(), &turn, &updates, None).await
    });
    receive(&mut requests).await;
    first
        .send(Err(ProviderError::ProviderResponse(
            rig::ProviderResponseError::new(
                http::StatusCode::BAD_REQUEST,
                "Invalid encrypted reasoning item",
            ),
        )))
        .await
        .unwrap();
    drop(first);
    let repaired = receive(&mut requests).await;
    let text = serde_json::to_string(&repaired.chat_history).unwrap();
    assert!(!text.contains("opaque-state"));
    assert!(!text.contains("bad:reasoning:item"));
    assert_eq!(text.matches("first request").count(), 1);
    second
        .send(Err(ProviderError::ProviderResponse(
            rig::ProviderResponseError::new(
                http::StatusCode::BAD_REQUEST,
                "invalid effort setting",
            ),
        )))
        .await
        .unwrap();
    drop(second);
    assert_eq!(task.await.unwrap().unwrap_err().kind, Kind::Configuration);
    assert_eq!(parts(&history.lock().await), said(&repaired));
    let mut notices = 0;
    while let Some(update) = receiver.recv().await {
        if matches!(update, Update::Notice { .. }) {
            notices += 1;
        }
    }
    assert_eq!(notices, 0, "the recovery is not narrated to the person");
}

#[tokio::test(start_paused = true)]
async fn steering_during_backoff_is_consumed_once_without_waiting_for_the_delay() {
    let (turn, request) = fixture();
    let steering = turn.steering.clone();
    let (seen, mut requests) = mpsc::unbounded_channel();
    let (first, a) = mpsc::channel(8);
    let (second, b) = mpsc::channel(8);
    let model = scripted(seen, [Some(a), Some(b)]);
    let (updates, _receiver) = mpsc::channel(64);
    let task = tokio::spawn(async move {
        run(&model, request, &ToolSet::default(), &turn, &updates, None).await
    });
    receive(&mut requests).await;
    first
        .send(Err(ProviderError::ProviderResponse(
            rig::ProviderResponseError::new(
                http::StatusCode::TOO_MANY_REQUESTS,
                r#"{"retry_after_seconds":30}"#,
            ),
        )))
        .await
        .unwrap();
    drop(first);
    // Backing off says nothing. A paused clock moves only once every task
    // is parked, and the backoff is a second at least, so this returns with
    // the turn waiting it out.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(steering.admit(Message::user("new direction")));
    let request = receive(&mut requests).await;
    let text = serde_json::to_string(&request.chat_history).unwrap();
    assert_eq!(text.matches("new direction").count(), 1);
    assert_eq!(text.matches("first request").count(), 1);
    answer(second).await;
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn context_rotation_waits_for_the_session_handoff_before_requesting_again() {
    let recorded = hand_off_a_full_context(
        "test/model",
        Usage::new()
            .input_tokens(17_000)
            .output_tokens(100)
            .total_tokens(17_100),
    )
    .await;
    assert_eq!(recorded.input_tokens, Some(17_010));
    assert_eq!(
        (recorded.cache_read_tokens, recorded.cache_write_tokens),
        (None, None)
    );
}

/// Rig counts what Claude read from its prompt cache, and what it wrote
/// there, inside its input. A context that came mostly from the cache is
/// just as full, and the turn's record says how much of it was cached.
#[tokio::test]
async fn a_context_claude_read_from_its_cache_is_just_as_full() {
    let recorded = hand_off_a_full_context(
        "anthropic/claude-test",
        Usage::new()
            .input_tokens(17_000)
            .cached_input_tokens(16_000)
            .cache_creation_input_tokens(960)
            .output_tokens(100)
            .total_tokens(17_100),
    )
    .await;
    assert_eq!(recorded.input_tokens, Some(17_010));
    assert_eq!(
        (recorded.cache_read_tokens, recorded.cache_write_tokens),
        (Some(16_000), Some(960))
    );
}

/// A turn whose first round reports `usage` against a 20k context: the
/// chapter hands off before the next request goes out, and the turn's
/// record is returned.
async fn hand_off_a_full_context(model_id: &str, usage: rig::completion::Usage) -> TokenUsage {
    let (mut turn, request) = fixture();
    turn.model = model_id.into();
    turn.context_limit = Some(20_000);
    let (seen, mut requests) = mpsc::unbounded_channel();
    let (first, a) = mpsc::channel(8);
    let (second, b) = mpsc::channel(8);
    let model = scripted(seen, [Some(a), Some(b)]);
    let (updates, mut receiver) = mpsc::channel(64);
    let mut tools = ToolSet::default();
    tools.add_tool(Nod);
    let task =
        tokio::spawn(async move { run(&model, request, &tools, &turn, &updates, None).await });
    receive(&mut requests).await;
    first
        .send(Ok(MockStreamEvent::tool_call(
            "done-once",
            "react",
            serde_json::json!({"emoji":"👍"}),
        )))
        .await
        .unwrap();
    first
        .send(Ok(MockStreamEvent::FinalResponse(mock_final(usage))))
        .await
        .unwrap();
    drop(first);
    let boundary = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Update::Chapter { boundary } = receiver.recv().await.unwrap() {
                return boundary;
            }
        }
    })
    .await
    .expect("a full context hands off before the next request");
    assert!(requests.try_recv().is_err());
    boundary.finish(Some("Chapter handoff: reaction completed.".into()));
    let next = receive(&mut requests).await;
    let rendered = serde_json::to_string(&next.chat_history).unwrap();
    assert!(rendered.contains("reaction completed"));
    assert!(rendered.contains("done-once"));
    assert!(rendered.contains("Fresh continuation"));
    second
        .send(Ok(MockStreamEvent::Text("the new direction".into())))
        .await
        .unwrap();
    second
        .send(Ok(MockStreamEvent::FinalResponse(mock_final(
            Usage::new()
                .input_tokens(10)
                .output_tokens(5)
                .total_tokens(15),
        ))))
        .await
        .unwrap();
    drop(second);
    task.await.unwrap().unwrap();
    loop {
        if let Update::Turn { usage, .. } = receiver.recv().await.unwrap() {
            return usage.expect("every round reported its usage");
        }
    }
}
