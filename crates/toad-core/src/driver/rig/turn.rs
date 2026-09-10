//! A controllable loop over ordinary Rig requests. Neither steering nor tool
//! execution depends on which provider the model handle uses.

use super::*;
use rig::completion::{CompletionModel, CompletionRequest};
use rig::message::{AssistantContent, ToolCall, UserContent};
use rig::streaming::StreamedAssistantContent;
use rig::tool::{ToolContext, ToolResult, ToolSet};

#[derive(Default)]
pub(super) struct Steering {
    state: Mutex<InputState>,
    changed: Notify,
}

#[derive(Default)]
struct InputState {
    closed: bool,
    revision: u64,
    pending: Vec<String>,
}

impl Steering {
    pub(super) fn admit(&self, text: String) -> bool {
        let mut state = lock(&self.state);
        if state.closed {
            return false;
        }
        state.pending.push(text);
        state.revision += 1;
        self.changed.notify_waiters();
        true
    }

    fn take(&self) -> (u64, Vec<String>) {
        let mut state = lock(&self.state);
        (state.revision, std::mem::take(&mut state.pending))
    }

    fn superseded(&self, revision: u64) -> bool {
        lock(&self.state).revision != revision
    }

    async fn changed(&self, revision: u64) {
        loop {
            let notified = self.changed.notified();
            if self.superseded(revision) {
                return;
            }
            notified.await;
        }
    }

    /// Closing admission and observing an empty inbox are one decision, so
    /// a message racing the final response is either handled here or queued
    /// by the session for a new activity.
    fn finish_if_empty(&self) -> bool {
        let mut state = lock(&self.state);
        if !state.pending.is_empty() {
            return false;
        }
        state.closed = true;
        true
    }

    pub(super) fn close(&self) -> Vec<String> {
        let mut state = lock(&self.state);
        state.closed = true;
        std::mem::take(&mut state.pending)
    }
}

pub(super) async fn run(
    model: &impl CompletionModel,
    template: CompletionRequest,
    tools: &ToolSet,
    turn: &Turn,
    sender: &mpsc::Sender<Update>,
) -> Result<(), String> {
    let mut history = turn.history.lock().await.clone();
    let mut usage = rig::completion::Usage::default();
    let mut usage_complete = true;
    let mut announce_update = false;
    let mut open = None;
    let mut stopped = false;

    for _ in 0..MAX_TURNS {
        let (revision, pending) = turn.steering.take();
        announce_update |= !pending.is_empty();
        history.extend(pending.into_iter().map(Message::user));
        *turn.history.lock().await = history.clone();
        if let Some(capability) = &turn.capability {
            capability.check()?;
        }
        let mut request = template.clone();
        request.chat_history = history.clone();
        let response = tokio::select! {
            biased;
            () = turn.stop.raised() => { stopped = true; usage_complete = false; break; }
            () = turn.steering.changed(revision) => { usage_complete = false; continue; }
            response = model.stream(request) => response,
        };
        let mut stream = response.map_err(text)?;
        let mut interrupted = false;
        let mut reasoning_deltas = std::collections::HashSet::new();
        loop {
            let item = tokio::select! {
                biased;
                () = turn.stop.raised() => {
                    stream.cancel();
                    stopped = true;
                    interrupted = true;
                    break;
                }
                () = turn.steering.changed(revision) => {
                    stream.cancel();
                    interrupted = true;
                    break;
                }
                item = stream.next() => item,
            };
            let Some(item) = item else { break };
            let item = match item {
                Ok(item) => item,
                Err(error) => {
                    flush(sender, &mut open).await;
                    return Err(text(error));
                }
            };
            if announce_update {
                send(
                    sender,
                    Update::Notice {
                        level: NoticeLevel::Info,
                        text: "Your update is now in the agent's context.".into(),
                    },
                )
                .await;
                announce_update = false;
            }
            match item {
                StreamedAssistantContent::Text(chunk) => {
                    chunk_into(sender, &mut open, MessageKind::Agent, &chunk.text).await;
                }
                StreamedAssistantContent::ReasoningDelta { id, reasoning, .. } => {
                    reasoning_deltas.insert(id);
                    chunk_into(sender, &mut open, MessageKind::Thought, &reasoning).await;
                }
                StreamedAssistantContent::Reasoning { id, reasoning } => {
                    if !reasoning_deltas.contains(&id) {
                        for part in reasoning.content {
                            if let rig::message::ReasoningContent::Text { text, .. } = part {
                                chunk_into(sender, &mut open, MessageKind::Thought, &text).await;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        let reported = stream.usage();
        usage_complete &= stream.response.is_some() && reported.has_values();
        usage += reported;
        flush(sender, &mut open).await;
        if stopped {
            break;
        }
        if interrupted || turn.steering.superseded(revision) {
            continue;
        }
        if stream.response.is_none() || stream.choice.is_empty() {
            return Err("The model stream ended without a complete response.".into());
        }
        if matches!(
            stream
                .response
                .as_ref()
                .and_then(|response| response.finish_reason.as_ref()),
            Some(
                rig::completion::FinishReason::Length
                    | rig::completion::FinishReason::ContentFilter
            )
        ) {
            return Err(
                "The model response was truncated or filtered; its tool calls were not executed."
                    .into(),
            );
        }
        let calls: Vec<ToolCall> = stream
            .choice
            .iter()
            .filter_map(|item| match item {
                AssistantContent::ToolCall(call) => Some(call.clone()),
                _ => None,
            })
            .collect();
        history.push(Message::Assistant {
            id: stream.identity().message_id,
            content: stream.choice,
        });

        for call in &calls {
            let call_id = uuid::Uuid::new_v4().to_string();
            send(
                sender,
                Update::ToolCall {
                    call_id: call_id.clone(),
                    title: describe_tool(&call.function.name, &call.function.arguments),
                    kind: call.function.name.clone(),
                },
            )
            .await;
            let mut context = ToolContext::new();
            let result = if stopped || turn.stop.raised.load(Ordering::SeqCst) {
                stopped = true;
                ToolResult::skipped("Not executed: the operator stopped this activity.")
            } else if turn.steering.superseded(revision) {
                ToolResult::skipped(
                    "Not executed: a newer operator message superseded this request.",
                )
            } else {
                if let Some(capability) = &turn.capability {
                    capability.check()?;
                }
                tokio::select! {
                    biased;
                    () = turn.stop.raised() => {
                        stopped = true;
                        ToolResult::skipped("Interrupted by the operator. The operation may have partial effects.")
                    }
                    result = tools.execute(&call.function.name, call.function.arguments.to_string(), &mut context) => result,
                }
            };
            let (shown, images) = result_of(result.output().as_content());
            send(
                sender,
                Update::ToolResult {
                    call_id: call_id.clone(),
                    ok: result.is_success(),
                    output: shown.clone(),
                    images: images.clone(),
                },
            )
            .await;
            let output = model_output(turn, &call_id, result.output(), &shown, &images);
            history.push(Message::User {
                content: vec![UserContent::ToolResult(rig::message::ToolResult {
                    call: call.id.clone(),
                    provider: call.provider.clone(),
                    name: call.function.name.clone(),
                    content: output.into_content(),
                })],
            });
        }
        // No network call sees an assistant tool call without its reply. The
        // checkpoint contains executed effects even when the next request fails.
        *turn.history.lock().await = history.clone();
        if stopped {
            break;
        }
        if calls.is_empty() && turn.steering.finish_if_empty() {
            finish(sender, "end_turn", usage, usage_complete).await;
            return Ok(());
        }
    }
    flush(sender, &mut open).await;
    if stopped {
        finish(sender, "aborted", usage, usage_complete).await;
        Ok(())
    } else {
        Err("This activity reached its model-request limit.".into())
    }
}

fn model_output(
    turn: &Turn,
    call_id: &str,
    output: &ToolOutput,
    shown: &str,
    images: &[ToolImage],
) -> ToolOutput {
    let provider = turn.model.split('/').next().unwrap_or("");
    if !images.is_empty() {
        return if images_to_model(provider) {
            output.clone()
        } else {
            ToolOutput::text(shown)
        };
    }
    let rendered = output.render();
    if rendered.len() > MODEL_TOOL_OUTPUT_BYTES {
        ToolOutput::text(hand_to_model(&turn.output_dir, call_id, &rendered))
    } else {
        output.clone()
    }
}

async fn finish(
    sender: &mpsc::Sender<Update>,
    reason: &str,
    usage: rig::completion::Usage,
    usage_complete: bool,
) {
    send(
        sender,
        Update::Turn {
            stop_reason: reason.into(),
            usage: usage_complete.then_some(TokenUsage {
                input_tokens: Some(usage.input_tokens as i64),
                output_tokens: Some(usage.output_tokens as i64),
                total_tokens: Some(usage.total_tokens as i64),
            }),
        },
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::completion::{CompletionError, CompletionResponse};
    use rig::streaming::{
        RawStreamingChoice, RawStreamingToolCall, StreamFinal, StreamingCompletionResponse,
    };
    use std::collections::VecDeque;
    use std::time::Duration;

    type Chunks = mpsc::Receiver<Result<RawStreamingChoice, CompletionError>>;

    struct ScriptModel {
        requests: mpsc::UnboundedSender<CompletionRequest>,
        streams: Mutex<VecDeque<Option<Chunks>>>,
    }

    impl CompletionModel for ScriptModel {
        async fn completion(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, CompletionError> {
            unreachable!("the shared loop uses streaming")
        }

        async fn stream(
            &self,
            request: CompletionRequest,
        ) -> Result<StreamingCompletionResponse, CompletionError> {
            self.requests.send(request).unwrap();
            let next = lock(&self.streams)
                .pop_front()
                .expect("unexpected model request");
            let Some(chunks) = next else {
                return std::future::pending().await;
            };
            Ok(StreamingCompletionResponse::stream(
                "scripted",
                Box::pin(futures_util::stream::unfold(
                    chunks,
                    |mut chunks| async move { chunks.recv().await.map(|item| (item, chunks)) },
                )),
            ))
        }
    }

    fn fixture() -> (Turn, CompletionRequest) {
        let turn = Turn {
            keys: HashMap::new(),
            model: "test/model".into(),
            output_limit: None,
            effort: None,
            preamble: "Follow the operator".into(),
            cwd: PathBuf::new(),
            reach: Reach::Machine,
            history: Arc::new(AsyncMutex::new(vec![Message::user("first request")])),
            stop: Arc::new(Stop::default()),
            steering: Arc::new(Steering::default()),
            output_dir: std::env::temp_dir(),
            mcp_tools: Vec::new(),
            capability: None,
        };
        let request = CompletionRequest {
            model: None,
            preamble: Some(turn.preamble.clone()),
            chat_history: Vec::new(),
            documents: Vec::new(),
            tools: Vec::new(),
            temperature: None,
            max_tokens: None,
            tool_choice: None,
            additional_params: None,
            output_schema: None,
            record_telemetry_content: false,
        };
        (turn, request)
    }

    async fn receive<T>(receiver: &mut mpsc::UnboundedReceiver<T>) -> T {
        tokio::time::timeout(Duration::from_secs(3), receiver.recv())
            .await
            .unwrap()
            .unwrap()
    }

    async fn answer(sender: mpsc::Sender<Result<RawStreamingChoice, CompletionError>>) {
        sender
            .send(Ok(RawStreamingChoice::Message("the new direction".into())))
            .await
            .unwrap();
        sender
            .send(Ok(RawStreamingChoice::FinalResponse(StreamFinal::new(
                "scripted",
                rig::completion::Usage::default(),
            ))))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn input_interrupts_a_request_before_the_first_stream_exists() {
        let (turn, request) = fixture();
        let steering = turn.steering.clone();
        let (seen, mut requests) = mpsc::unbounded_channel();
        let (chunks, stream) = mpsc::channel(8);
        let model = ScriptModel {
            requests: seen,
            streams: Mutex::new(VecDeque::from([None, Some(stream)])),
        };
        let (updates, mut receiver) = mpsc::channel(64);
        let task = tokio::spawn(async move {
            run(&model, request, &ToolSet::default(), &turn, &updates).await
        });
        receive(&mut requests).await;
        assert!(steering.admit("actually, do the other thing".into()));
        assert!(steering.admit("keep the scope small".into()));
        let next = receive(&mut requests).await;
        assert_eq!(
            next.chat_history,
            vec![
                Message::user("first request"),
                Message::user("actually, do the other thing"),
                Message::user("keep the scope small")
            ]
        );
        answer(chunks).await;
        task.await.unwrap().unwrap();
        assert!(!steering.admit("after completion".into()));
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
        let model = ScriptModel {
            requests: seen,
            streams: Mutex::new(VecDeque::from([Some(old_stream), Some(new_stream)])),
        };
        let (updates, mut receiver) = mpsc::channel(64);
        let task = tokio::spawn(async move {
            run(&model, request, &ToolSet::default(), &turn, &updates).await
        });
        receive(&mut requests).await;
        old.send(Ok(RawStreamingChoice::ToolCallDelta {
            id: "partial".into(),
            content: rig::streaming::ToolCallDeltaContent::Name("shell".into()),
        }))
        .await
        .unwrap();
        old.send(Ok(RawStreamingChoice::Message("unfinished".into())))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), receiver.recv())
            .await
            .unwrap();
        assert!(steering.admit("change direction".into()));
        let next = receive(&mut requests).await;
        assert!(old.is_closed(), "the old provider stream is dropped");
        assert_eq!(
            next.chat_history,
            vec![
                Message::user("first request"),
                Message::user("change direction")
            ]
        );
        answer(new).await;
        task.await.unwrap().unwrap();
        while let Some(update) = receiver.recv().await {
            assert!(!matches!(update, Update::ToolCall { .. }));
        }
    }

    #[tokio::test]
    async fn steering_keeps_a_running_tools_result_and_skips_its_unstarted_siblings() {
        let (turn, request) = fixture();
        let steering = turn.steering.clone();
        let (seen, mut requests) = mpsc::unbounded_channel();
        let (old, old_stream) = mpsc::channel(8);
        let (new, new_stream) = mpsc::channel(8);
        let model = ScriptModel {
            requests: seen,
            streams: Mutex::new(VecDeque::from([Some(old_stream), Some(new_stream)])),
        };
        let (started, mut starts) = mpsc::unbounded_channel();
        let release = Arc::new(Notify::new());
        let gate = release.clone();
        let mut tools = ToolSet::default();
        tools.add_dynamic_tool(DynamicTool::new(
            "slow",
            "A synchronous tool with an observable effect",
            serde_json::json!({"type": "object"}),
            move |_context, _arguments| {
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
        let task = tokio::spawn(async move { run(&model, request, &tools, &turn, &updates).await });
        receive(&mut requests).await;
        for id in ["first", "second"] {
            old.send(Ok(RawStreamingChoice::ToolCall(RawStreamingToolCall::new(
                id,
                "slow".into(),
                serde_json::json!({}),
            ))))
            .await
            .unwrap();
        }
        old.send(Ok(RawStreamingChoice::FinalResponse(StreamFinal::new(
            "scripted",
            rig::completion::Usage::default(),
        ))))
        .await
        .unwrap();
        drop(old);
        receive(&mut starts).await;
        assert!(steering.admit("change direction".into()));
        assert!(requests.try_recv().is_err());
        release.notify_one();
        let next = receive(&mut requests).await;
        assert!(starts.try_recv().is_err(), "the second call never started");
        let history = &next.chat_history;
        assert_eq!(history.len(), 5);
        assert_eq!(history[4], Message::user("change direction"));
        let Message::Assistant { content, .. } = &history[1] else {
            panic!("the completed tool calls are retained");
        };
        for (call, message) in content.iter().zip(&history[2..4]) {
            let AssistantContent::ToolCall(call) = call else {
                panic!("expected a tool call");
            };
            let Message::User { content } = message else {
                panic!("expected its result");
            };
            let UserContent::ToolResult(result) = &content[0] else {
                panic!("expected its result");
            };
            assert_eq!(call.id, result.call);
            assert_eq!(call.provider, result.provider);
        }
        assert!(
            serde_json::to_string(&history[2])
                .unwrap()
                .contains("the first effect completed")
        );
        assert!(
            serde_json::to_string(&history[3])
                .unwrap()
                .contains("Not executed")
        );
        answer(new).await;
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn a_truncated_response_never_dispatches_its_tools() {
        let (turn, request) = fixture();
        let history = turn.history.clone();
        let (seen, mut requests) = mpsc::unbounded_channel();
        let (chunks, stream) = mpsc::channel(8);
        let model = ScriptModel {
            requests: seen,
            streams: Mutex::new(VecDeque::from([Some(stream)])),
        };
        let (updates, mut receiver) = mpsc::channel(64);
        let task = tokio::spawn(async move {
            run(&model, request, &ToolSet::default(), &turn, &updates).await
        });
        receive(&mut requests).await;
        chunks
            .send(Ok(RawStreamingChoice::ToolCall(RawStreamingToolCall::new(
                "truncated",
                "shell".into(),
                serde_json::json!({}),
            ))))
            .await
            .unwrap();
        let mut terminal = StreamFinal::new("scripted", rig::completion::Usage::default());
        terminal.finish_reason = Some(rig::completion::FinishReason::Length);
        chunks
            .send(Ok(RawStreamingChoice::FinalResponse(terminal)))
            .await
            .unwrap();
        drop(chunks);
        assert!(task.await.unwrap().unwrap_err().contains("truncated"));
        assert_eq!(*history.lock().await, [Message::user("first request")]);
        while let Some(update) = receiver.recv().await {
            assert!(!matches!(update, Update::ToolCall { .. }));
        }
    }

    #[test]
    fn completion_and_admission_have_one_order() {
        let steering = Steering::default();
        assert!(steering.admit("before completion".into()));
        assert!(!steering.finish_if_empty());
        assert_eq!(steering.take().1, ["before completion"]);
        assert!(steering.finish_if_empty());
        assert!(!steering.admit("must stay in the session queue".into()));
    }
}
