//! A controllable loop over ordinary Rig requests. Neither steering nor tool
//! execution depends on which provider the model handle uses.

use super::*;
use crate::session::jobs::{self, JobArgs, JobSnapshot, JobState, Jobs, WaitArgs};
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
    pending: Vec<Input>,
}

impl Steering {
    pub(super) fn admit(&self, message: impl Into<Input>) -> bool {
        let mut state = lock(&self.state);
        if state.closed {
            return false;
        }
        state.pending.push(message.into());
        state.revision += 1;
        self.changed.notify_waiters();
        true
    }

    fn take(&self) -> (u64, Vec<Input>) {
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

    pub(super) fn close_admission(&self) {
        lock(&self.state).closed = true;
    }

    pub(super) fn close(&self) -> Vec<Input> {
        let mut state = lock(&self.state);
        state.closed = true;
        std::mem::take(&mut state.pending)
    }
}

pub(super) async fn run(
    model: &impl CompletionModel,
    mut template: CompletionRequest,
    tools: &ToolSet,
    turn: &Turn,
    sender: &mpsc::Sender<Update>,
    shell: Option<RunCommand>,
) -> Result<(), Failure> {
    let mut jobs = Jobs::new(shell, turn.delegate.clone());
    if let Some(instructions) = jobs.instructions() {
        template.tools.extend(jobs.definitions());
        template.preamble = Some(format!(
            "{}\n\n{instructions}",
            template.preamble.unwrap_or_default(),
        ));
    }
    let result = run_inner(model, template, tools, turn, sender, &mut jobs).await;
    turn.steering.close_admission();
    // Stop, revocation, provider failure and ordinary completion all leave
    // through the owner. A response ending cannot orphan a shell command.
    jobs.cancel_all();
    while jobs.active() {
        match jobs.next().await {
            Ok(job) => {
                publish_job(turn, sender, job).await;
            }
            Err(error) => {
                send(
                    sender,
                    Update::Notice {
                        level: NoticeLevel::Error,
                        text: error,
                    },
                )
                .await
            }
        }
    }
    match result {
        Ok((reason, usage, complete)) => {
            finish(sender, reason, usage, complete).await;
            Ok(())
        }
        Err(error) => Err(error),
    }
}

async fn run_inner(
    model: &impl CompletionModel,
    template: CompletionRequest,
    tools: &ToolSet,
    turn: &Turn,
    sender: &mpsc::Sender<Update>,
    jobs: &mut Jobs,
) -> Result<(&'static str, rig::completion::Usage, bool), Failure> {
    let mut history = turn.history.lock().await.clone();
    let mut usage = rig::completion::Usage::default();
    let mut usage_complete = true;
    let mut open = None;
    let mut stopped = false;
    let mut job_results = Vec::new();
    // Whether this activity has already run a tool: a reply that is only a
    // reaction, or only a tool call, ends with a completion that says nothing.
    let mut answered_calls = false;
    // Whether the last round's tools failed. Silence after success is the
    // model done; silence after failure is the model giving up unsaid.
    let mut last_round_failed = false;
    let mut retries = 0;
    let mut repaired = false;
    let mut context_tokens = 0;
    let mut rotated = false;

    'attempt: for _ in 0..MAX_TURNS {
        for job in jobs.ready()? {
            job_results.push(publish_job(turn, sender, job).await);
        }
        history.append(&mut job_results);
        let (revision, pending) = turn.steering.take();
        for input in pending {
            history.push(input.prepare(&turn.stop).await);
        }
        *turn.history.lock().await = history.clone();
        if let Some(capability) = &turn.capability {
            capability.check()?;
        }
        images::limit_history(&mut history, &turn.model);
        if let Some(threshold) = recovery::context_threshold(turn.context_limit, turn.output_limit)
            && context_tokens.max(recovery::estimated_tokens(&history, &template)) >= threshold
        {
            if rotated {
                return Err(Failure::classify(
                    "context window is too small for the continuation",
                    None,
                    "preparation",
                ));
            }
            rotated = true;
            history = recovery::fresh(&history, &turn.output_dir);
            *turn.history.lock().await = history.clone();
            context_tokens = 0;
            if recovery::estimated_tokens(&history, &template) >= threshold {
                return Err(Failure::classify(
                    "context window is too small for the continuation and tools",
                    None,
                    "preparation",
                ));
            }
            let (boundary, ready) = super::super::ChapterBoundary::new();
            send(sender, Update::Chapter { boundary }).await;
            tokio::select! {
                biased;
                () = turn.stop.raised() => { stopped = true; break; }
                note = ready => if let Ok(Some(note)) = note { history.insert(0, Message::user(note)); },
            }
            *turn.history.lock().await = history.clone();
        }
        let mut request = template.clone();
        request.chat_history = history.clone();
        if let Some(context) = jobs.context() {
            request.chat_history.push(Message::user(context));
        }
        let response = model.stream(request);
        tokio::pin!(response);
        let response = loop {
            tokio::select! {
                biased;
                () = turn.stop.raised() => { stopped = true; usage_complete = false; break 'attempt; }
                () = turn.steering.changed(revision) => { usage_complete = false; continue 'attempt; }
                job = jobs.next(), if jobs.active() => {
                    job_results.push(publish_job(turn, sender, job?).await);
                }
                response = &mut response => break response,
            }
        };
        let mut stream = match response {
            Ok(stream) => stream,
            Err(error) => {
                usage_complete = false;
                let failure = Failure::provider(error, "request").after_tools(answered_calls);
                if recover(
                    &failure,
                    &mut retries,
                    &mut repaired,
                    &mut history,
                    turn,
                    sender,
                    jobs,
                    &mut job_results,
                    revision,
                )
                .await?
                {
                    if matches!(failure.kind, Kind::Context | Kind::Replay) {
                        context_tokens = 0;
                    }
                    continue;
                }
                return Err(failure);
            }
        };
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
                job = jobs.next(), if jobs.active() => {
                    job_results.push(publish_job(turn, sender, job?).await);
                    continue;
                }
                item = stream.next() => item,
            };
            let Some(item) = item else { break };
            let item = match item {
                Ok(item) => item,
                Err(error) => {
                    usage_complete = false;
                    flush(sender, &mut open).await;
                    let failure = Failure::provider(error, "stream").after_tools(answered_calls);
                    stream.cancel();
                    if recover(
                        &failure,
                        &mut retries,
                        &mut repaired,
                        &mut history,
                        turn,
                        sender,
                        jobs,
                        &mut job_results,
                        revision,
                    )
                    .await?
                    {
                        if matches!(failure.kind, Kind::Context | Kind::Replay) {
                            context_tokens = 0;
                        }
                        continue 'attempt;
                    }
                    return Err(failure);
                }
            };
            match item {
                StreamedAssistantContent::Text(chunk) => {
                    chunk_into(sender, &mut open, MessageKind::Agent, &chunk.text).await;
                }
                StreamedAssistantContent::ReasoningDelta { id, reasoning, .. } => {
                    reasoning_deltas.insert(id);
                    chunk_into(sender, &mut open, MessageKind::Thought, &reasoning).await;
                }
                StreamedAssistantContent::Reasoning { id, reasoning }
                    if !reasoning_deltas.contains(&id) =>
                {
                    for part in reasoning.content {
                        if let rig::message::ReasoningContent::Text { text, .. } = part {
                            chunk_into(sender, &mut open, MessageKind::Thought, &text).await;
                        }
                    }
                }
                _ => {}
            }
        }
        let reported = every_input_token(stream.usage(), &turn.model);
        usage_complete &= stream.response.is_some() && reported.has_values();
        context_tokens = reported.input_tokens.saturating_add(reported.output_tokens);
        usage += reported;
        flush(sender, &mut open).await;
        if stopped {
            break;
        }
        if interrupted || turn.steering.superseded(revision) {
            continue;
        }
        if stream.response.is_none() || (stream.choice.is_empty() && !answered_calls) {
            let failure = Failure::classify(
                "The model stream ended without a complete response.",
                Some(502),
                "stream",
            )
            .after_tools(answered_calls);
            if recover(
                &failure,
                &mut retries,
                &mut repaired,
                &mut history,
                turn,
                sender,
                jobs,
                &mut job_results,
                revision,
            )
            .await?
            {
                continue;
            }
            return Err(failure);
        }
        if stream.choice.is_empty() && last_round_failed {
            send(
                sender,
                Update::Notice {
                    level: NoticeLevel::Info,
                    text: "Stopped after a failed step and said nothing.".into(),
                },
            )
            .await;
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
        retries = 0;
        rotated = false;
        let calls: Vec<ToolCall> = stream
            .choice
            .iter()
            .filter_map(|item| match item {
                AssistantContent::ToolCall(call) => Some(call.clone()),
                _ => None,
            })
            .collect();
        // An empty completion after a tool round is the model done; a message
        // with nothing in it is not something a provider will take back.
        if !stream.choice.is_empty() {
            history.push(Message::Assistant {
                id: stream.identity().message_id,
                content: stream.choice,
            });
        }
        answered_calls |= !calls.is_empty();
        let mut round_failed = false;

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
            } else if turn
                .capability
                .as_ref()
                .is_some_and(|capability| !capability.is_current())
            {
                turn.stop.raise();
                stopped = true;
                ToolResult::skipped("Not executed: this activity's tool access was revoked.")
            } else {
                match call.function.name.as_str() {
                    "shell" if jobs.shell_enabled() => {
                        job_result(jobs.launch(call_id.clone(), call.function.arguments.clone()))
                    }
                    jobs::SUBAGENT if jobs.delegates() => {
                        job_result(jobs.delegate(call_id.clone(), call.function.arguments.clone()))
                    }
                    "inspect_job" if jobs.enabled() => job_result(
                        serde_json::from_value::<JobArgs>(call.function.arguments.clone())
                            .map_err(text)
                            .and_then(|args| jobs.inspect(&args.job_id)),
                    ),
                    "cancel_job" if jobs.enabled() => job_result(
                        serde_json::from_value::<JobArgs>(call.function.arguments.clone())
                            .map_err(text)
                            .and_then(|args| jobs.cancel(args)),
                    ),
                    "wait_jobs" if jobs.enabled() => {
                        let result = wait_jobs(
                            jobs,
                            &call.function.arguments,
                            turn,
                            revision,
                            sender,
                            &mut job_results,
                        )
                        .await?;
                        stopped |= turn.stop.raised.load(Ordering::SeqCst);
                        result
                    }
                    _ => {
                        let execution = tools.execute(
                            &call.function.name,
                            call.function.arguments.to_string(),
                            &mut context,
                        );
                        tokio::pin!(execution);
                        loop {
                            tokio::select! {
                                biased;
                                () = turn.stop.raised() => {
                                    stopped = true;
                                    break ToolResult::skipped("Interrupted by the operator. The operation may have partial effects.");
                                }
                                job = jobs.next(), if jobs.active() => {
                                    job_results.push(publish_job(turn, sender, job?).await);
                                }
                                result = &mut execution => break result,
                            }
                        }
                    }
                }
            };
            round_failed |= !result.is_success();
            let (shown, images) = result_of(result.output().as_content());
            // The model gets its one launch receipt now. The transcript's
            // shell card stays running until the owned job produces a result.
            if !jobs.owns(&call_id) {
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
            }
            let output = model_output(turn, &call_id, result.output(), &shown, &images).await;
            history.push(Message::User {
                content: vec![UserContent::ToolResult(rig::message::ToolResult {
                    call: call.id.clone(),
                    provider: call.provider.clone(),
                    name: call.function.name.clone(),
                    content: output.into_content(),
                })],
            });
        }
        if !calls.is_empty() {
            last_round_failed = round_failed;
        }
        // Job notifications are ordinary execution data, after every call in
        // the completed response has its one reply. They never reopen a call.
        for job in jobs.ready()? {
            job_results.push(publish_job(turn, sender, job).await);
        }
        let new_results = !job_results.is_empty();
        history.append(&mut job_results);
        *turn.history.lock().await = history.clone();
        if stopped {
            break;
        }
        if calls.is_empty() && !new_results {
            if jobs.active() {
                // A textual response is not activity completion while jobs
                // run. Park without spending model requests; input can wake us.
                // What it just said is what the person reads meanwhile.
                send(sender, Update::Parked).await;
                tokio::select! {
                    biased;
                    () = turn.stop.raised() => { stopped = true; break; }
                    () = turn.steering.changed(revision) => {}
                    job = jobs.next() => job_results.push(publish_job(turn, sender, job?).await),
                }
            } else if turn.steering.finish_if_empty() {
                return Ok(("end_turn", usage, usage_complete));
            }
        }
    }
    flush(sender, &mut open).await;
    history.append(&mut job_results);
    *turn.history.lock().await = history;
    if stopped {
        Ok(("aborted", usage, usage_complete))
    } else {
        Err("This activity reached its model-request limit.".into())
    }
}

// Retrying happens before accepting another response, so no tool dispatch is replayed.
#[allow(clippy::too_many_arguments)]
async fn recover(
    failure: &Failure,
    retries: &mut u32,
    repaired: &mut bool,
    history: &mut Vec<Message>,
    turn: &Turn,
    sender: &mpsc::Sender<Update>,
    jobs: &mut Jobs,
    results: &mut Vec<Message>,
    revision: u64,
) -> Result<bool, Failure> {
    if turn.stop.raised.load(Ordering::SeqCst) {
        return Ok(true);
    }
    if let Some(capability) = &turn.capability {
        capability.check()?;
    }
    if matches!(failure.kind, Kind::Replay | Kind::Context) && !*repaired {
        history.append(results);
        *history = recovery::fresh(history, &turn.output_dir);
        *turn.history.lock().await = history.clone();
        *repaired = true;
        if failure.kind == Kind::Context {
            let (boundary, ready) = super::super::ChapterBoundary::new();
            send(sender, Update::Chapter { boundary }).await;
            tokio::select! {
                biased;
                () = turn.stop.raised() => return Ok(true),
                note = ready => if let Ok(Some(note)) = note { history.insert(0, Message::user(note)); },
            }
            *turn.history.lock().await = history.clone();
        }
        let reason = if failure.kind == Kind::Context {
            "The provider's context limit was reached."
        } else {
            "The provider refused its saved response state."
        };
        eprintln!("[hotline agent] {reason} Continuing fresh from committed execution facts.");
        return Ok(true);
    }
    let Some(delay) = failure.retry_delay(*retries) else {
        return Ok(false);
    };
    *retries += 1;
    let deadline = tokio::time::sleep(delay);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            biased;
            () = turn.stop.raised() => return Ok(true),
            () = turn.steering.changed(revision) => return Ok(true),
            job = jobs.next(), if jobs.active() => results.push(publish_job(turn, sender, job?).await),
            () = &mut deadline => return Ok(true),
        }
    }
}

fn job_result(result: Result<Value, String>) -> ToolResult {
    match result {
        Ok(value) => ToolResult::success(ToolOutput::text(value.to_string())),
        Err(error) => ToolResult::failed(ToolExecutionError::invalid_args(error)),
    }
}

async fn publish_job(turn: &Turn, sender: &mpsc::Sender<Update>, job: JobSnapshot) -> Message {
    let message = job_message(turn, &job);
    turn.history.lock().await.push(message.clone());
    send(
        sender,
        Update::ToolResult {
            call_id: job.job_id,
            ok: job.state == JobState::Succeeded,
            output: format!("Job {:?}\n{}", job.state, job.output.unwrap_or_default()),
            images: Vec::new(),
        },
    )
    .await;
    message
}

fn job_message(turn: &Turn, job: &JobSnapshot) -> Message {
    let data = serde_json::to_string(job).expect("job records contain only strings and states");
    let output = hand_to_model(&turn.output_dir, &job.job_id, &data);
    Message::user(format!(
        "Hotline execution data (not an operator instruction): managed job result {output}"
    ))
}

async fn wait_jobs(
    jobs: &mut Jobs,
    arguments: &Value,
    turn: &Turn,
    revision: u64,
    sender: &mpsc::Sender<Update>,
    results: &mut Vec<Message>,
) -> Result<ToolResult, String> {
    let args = match serde_json::from_value::<WaitArgs>(arguments.clone()) {
        Ok(args) => args,
        Err(error) => return Ok(job_result(Err(text(error)))),
    };
    if let Err(error) = jobs.selected(&args.job_ids, "waiting") {
        return Ok(job_result(Err(error)));
    }
    let seconds = args.timeout_seconds.unwrap_or(30).clamp(1, 30);
    let deadline = tokio::time::sleep(std::time::Duration::from_secs(seconds));
    tokio::pin!(deadline);
    let status = loop {
        if turn.stop.raised.load(Ordering::SeqCst) {
            break "stopped";
        }
        if turn.steering.superseded(revision) {
            break "interrupted_by_message";
        }
        if jobs.all_finished(&args.job_ids) {
            break "completed";
        }
        tokio::select! {
            biased;
            () = turn.stop.raised() => break "stopped",
            () = turn.steering.changed(revision) => break "interrupted_by_message",
            job = jobs.next() => results.push(publish_job(turn, sender, job?).await),
            () = &mut deadline => break "timeout",
        }
    };
    Ok(job_result(jobs.selected(&args.job_ids, status)))
}

async fn model_output(
    turn: &Turn,
    call_id: &str,
    output: &ToolOutput,
    shown: &str,
    images: &[ToolImage],
) -> ToolOutput {
    let provider = turn.model.split('/').next().unwrap_or("");
    if !images.is_empty() {
        return if images_to_model(provider) {
            images::tool_output(output, &turn.stop).await
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
                cache_read_tokens: Some(usage.cached_input_tokens as i64).filter(|n| *n > 0),
                cache_write_tokens: Some(usage.cache_creation_input_tokens as i64)
                    .filter(|n| *n > 0),
            }),
        },
    )
    .await;
}

/// A round's usage with `input_tokens` counting every token the request
/// carried. Anthropic reports what it read from its prompt cache, and what it
/// wrote to it, beside `input_tokens`; every other route counts them inside,
/// which is where the context threshold and the turn's record expect them.
pub(super) fn every_input_token(
    mut usage: rig::completion::Usage,
    model_id: &str,
) -> rig::completion::Usage {
    let provider = model_id.split('/').next().unwrap_or("");
    if models::wiring(provider).is_some_and(|wiring| wiring.client == Client::Anthropic) {
        usage.input_tokens = usage
            .input_tokens
            .saturating_add(usage.cached_input_tokens)
            .saturating_add(usage.cache_creation_input_tokens);
    }
    usage
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
            context_limit: None,
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
            delegate: None,
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
            run(&model, request, &ToolSet::default(), &turn, &updates, None).await
        });
        receive(&mut requests).await;
        assert!(steering.admit(Message::user("actually, do the other thing")));
        assert!(steering.admit(Message::user("keep the scope small")));
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
        let model = ScriptModel {
            requests: seen,
            streams: Mutex::new(VecDeque::from([Some(old_stream), Some(new_stream)])),
        };
        let (updates, mut receiver) = mpsc::channel(64);
        let task = tokio::spawn(async move {
            run(&model, request, &ToolSet::default(), &turn, &updates, None).await
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
        assert!(steering.admit(Message::user("change direction")));
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
        let task =
            tokio::spawn(async move { run(&model, request, &tools, &turn, &updates, None).await });
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
        assert!(steering.admit(Message::user("change direction")));
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
        let model = ScriptModel {
            requests: seen,
            streams: Mutex::new(VecDeque::from([Some(stream_one), Some(stream_two)])),
        };
        let (updates, mut receiver) = mpsc::channel(64);
        let task =
            tokio::spawn(async move { run(&model, request, &tools, &turn, &updates, None).await });
        receive(&mut requests).await;
        first
            .send(Ok(RawStreamingChoice::ToolCall(RawStreamingToolCall::new(
                "react-1",
                "react".into(),
                serde_json::json!({ "emoji": "👍" }),
            ))))
            .await
            .unwrap();
        first
            .send(Ok(RawStreamingChoice::FinalResponse(StreamFinal::new(
                "scripted",
                rig::completion::Usage::default(),
            ))))
            .await
            .unwrap();
        drop(first);
        receive(&mut requests).await;
        second
            .send(Ok(RawStreamingChoice::FinalResponse(StreamFinal::new(
                "scripted",
                rig::completion::Usage::default(),
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
        let model = ScriptModel {
            requests: seen,
            streams: Mutex::new(VecDeque::from([Some(stream)])),
        };
        let (updates, mut receiver) = mpsc::channel(64);
        let task = tokio::spawn(async move {
            run(&model, request, &ToolSet::default(), &turn, &updates, None).await
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
        let model = ScriptModel {
            requests: seen,
            streams: Mutex::new(VecDeque::from([
                Some(first_stream),
                Some(second_stream),
                Some(third_stream),
            ])),
        };
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
        first.send(Ok(RawStreamingChoice::ToolCall(RawStreamingToolCall::new(
            "owned_shell", "shell".into(), serde_json::json!({"command":"echo started; while true; do echo x >> heartbeat; sleep 0.05; done & wait"})
        )))).await.unwrap();
        first
            .send(Ok(RawStreamingChoice::FinalResponse(StreamFinal::new(
                "scripted",
                rig::completion::Usage::default(),
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
                .send(Err(CompletionError::ProviderResponse(
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
        let model = ScriptModel {
            requests: seen,
            streams: Mutex::new(VecDeque::from([Some(a), Some(b), Some(c)])),
        };
        let mut tools = ToolSet::default();
        tools.add_tool(Nod);
        let (updates, mut receiver) = mpsc::channel(64);
        let task =
            tokio::spawn(async move { run(&model, request, &tools, &turn, &updates, None).await });
        receive(&mut requests).await;
        first
            .send(Ok(RawStreamingChoice::ToolCall(RawStreamingToolCall::new(
                "done-once",
                "react".into(),
                serde_json::json!({"emoji":"👍"}),
            ))))
            .await
            .unwrap();
        first
            .send(Ok(RawStreamingChoice::FinalResponse(StreamFinal::new(
                "scripted",
                Default::default(),
            ))))
            .await
            .unwrap();
        drop(first);
        let before = receive(&mut requests).await;
        second
            .send(Ok(RawStreamingChoice::Message("partial response".into())))
            .await
            .unwrap();
        second
            .send(Err(CompletionError::ProviderResponse(
                rig::ProviderResponseError::new(http::StatusCode::SERVICE_UNAVAILABLE, "busy"),
            )))
            .await
            .unwrap();
        drop(second);
        let retry = receive(&mut requests).await;
        assert_eq!(before.chat_history, retry.chat_history);
        assert!(
            !serde_json::to_string(&retry.chat_history)
                .unwrap()
                .contains("partial response")
        );
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

    #[tokio::test]
    async fn stop_during_backoff_is_terminal() {
        let (turn, request) = fixture();
        let stop = turn.stop.clone();
        let (seen, mut requests) = mpsc::unbounded_channel();
        let (first, stream) = mpsc::channel(8);
        let model = ScriptModel {
            requests: seen,
            streams: Mutex::new(VecDeque::from([Some(stream)])),
        };
        let (updates, _receiver) = mpsc::channel(64);
        let task = tokio::spawn(async move {
            run(&model, request, &ToolSet::default(), &turn, &updates, None).await
        });
        receive(&mut requests).await;
        first
            .send(Err(CompletionError::ProviderResponse(
                rig::ProviderResponseError::new(http::StatusCode::TOO_MANY_REQUESTS, "rate limit"),
            )))
            .await
            .unwrap();
        drop(first);
        // Backing off says nothing, so give the turn a moment to be in it.
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
        history.lock().await.push(Message::Assistant {
            id: None,
            content: vec![AssistantContent::Reasoning(rig::message::Reasoning {
                id: Some("bad:reasoning:item".into()),
                content: vec![rig::message::ReasoningContent::Encrypted(
                    "opaque-state".into(),
                )],
            })],
        });
        let (seen, mut requests) = mpsc::unbounded_channel();
        let (first, a) = mpsc::channel(8);
        let (second, b) = mpsc::channel(8);
        let model = ScriptModel {
            requests: seen,
            streams: Mutex::new(VecDeque::from([Some(a), Some(b)])),
        };
        let (updates, mut receiver) = mpsc::channel(64);
        let task = tokio::spawn(async move {
            run(&model, request, &ToolSet::default(), &turn, &updates, None).await
        });
        receive(&mut requests).await;
        first
            .send(Err(CompletionError::ProviderResponse(
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
            .send(Err(CompletionError::ProviderResponse(
                rig::ProviderResponseError::new(
                    http::StatusCode::BAD_REQUEST,
                    "invalid effort setting",
                ),
            )))
            .await
            .unwrap();
        drop(second);
        assert_eq!(task.await.unwrap().unwrap_err().kind, Kind::Configuration);
        assert_eq!(*history.lock().await, repaired.chat_history);
        let mut notices = 0;
        while let Some(update) = receiver.recv().await {
            if matches!(update, Update::Notice { .. }) {
                notices += 1;
            }
        }
        assert_eq!(notices, 0, "the recovery is not narrated to the person");
    }

    #[tokio::test]
    async fn steering_during_backoff_is_consumed_once_without_waiting_for_the_delay() {
        let (turn, request) = fixture();
        let steering = turn.steering.clone();
        let (seen, mut requests) = mpsc::unbounded_channel();
        let (first, a) = mpsc::channel(8);
        let (second, b) = mpsc::channel(8);
        let model = ScriptModel {
            requests: seen,
            streams: Mutex::new(VecDeque::from([Some(a), Some(b)])),
        };
        let (updates, _receiver) = mpsc::channel(64);
        let task = tokio::spawn(async move {
            run(&model, request, &ToolSet::default(), &turn, &updates, None).await
        });
        receive(&mut requests).await;
        first
            .send(Err(CompletionError::ProviderResponse(
                rig::ProviderResponseError::new(
                    http::StatusCode::TOO_MANY_REQUESTS,
                    r#"{"retry_after_seconds":30}"#,
                ),
            )))
            .await
            .unwrap();
        drop(first);
        // Backing off says nothing, so give the turn a moment to be in it.
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
            rig::completion::Usage {
                input_tokens: 17_000,
                output_tokens: 100,
                total_tokens: 17_100,
                ..Default::default()
            },
        )
        .await;
        assert_eq!(recorded.input_tokens, Some(17_010));
        assert_eq!(
            (recorded.cache_read_tokens, recorded.cache_write_tokens),
            (None, None)
        );
    }

    /// Claude counts what it read from its prompt cache, and what it wrote
    /// there, beside its input. A context that came mostly from the cache is
    /// just as full, and the turn's record says how much of it was cached.
    #[tokio::test]
    async fn a_context_claude_read_from_its_cache_is_just_as_full() {
        let recorded = hand_off_a_full_context(
            "anthropic/claude-test",
            rig::completion::Usage {
                input_tokens: 40,
                cached_input_tokens: 16_000,
                cache_creation_input_tokens: 960,
                output_tokens: 100,
                total_tokens: 17_100,
                ..Default::default()
            },
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
        let model = ScriptModel {
            requests: seen,
            streams: Mutex::new(VecDeque::from([Some(a), Some(b)])),
        };
        let (updates, mut receiver) = mpsc::channel(64);
        let mut tools = ToolSet::default();
        tools.add_tool(Nod);
        let task =
            tokio::spawn(async move { run(&model, request, &tools, &turn, &updates, None).await });
        receive(&mut requests).await;
        first
            .send(Ok(RawStreamingChoice::ToolCall(RawStreamingToolCall::new(
                "done-once",
                "react".into(),
                serde_json::json!({"emoji":"👍"}),
            ))))
            .await
            .unwrap();
        first
            .send(Ok(RawStreamingChoice::FinalResponse(StreamFinal::new(
                "scripted", usage,
            ))))
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
            .send(Ok(RawStreamingChoice::Message("the new direction".into())))
            .await
            .unwrap();
        second
            .send(Ok(RawStreamingChoice::FinalResponse(StreamFinal::new(
                "scripted",
                rig::completion::Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    total_tokens: 15,
                    ..Default::default()
                },
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
}
