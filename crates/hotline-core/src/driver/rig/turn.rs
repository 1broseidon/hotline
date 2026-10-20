//! A controllable loop over ordinary Rig requests. Neither steering nor tool
//! execution depends on which provider the model handle uses.

use super::history::OpenMessage;
use super::*;
use crate::pricing::Tokens;
use crate::session::jobs::{self, JobArgs, JobKind, JobSnapshot, JobState, Jobs, WaitArgs};
use rig::completion::{CompletionModel, CompletionRequest, ToolDefinition};
use rig::message::{AssistantContent, ToolCall, UserContent};
use rig::streaming::StreamedAssistantContent;
use rig::tool::{ToolContext, ToolResult, ToolSet};
use serde_json::json;

/// Meters a turn's rounds against the Chat budget when its model is billed
/// per token: each round reserves an estimate before its request goes out
/// and is settled to what the provider reports it used. A round that fails,
/// is interrupted, or reports no usage keeps its reservation. A sign-in, a
/// local server or a free model has no meter, so it is never counted or
/// refused. ACP agents bill their own accounts and are not metered here.
pub(crate) struct ChatMeter {
    budget: Arc<crate::voice::metering::Budget>,
    price: crate::contract::ModelCost,
}

impl ChatMeter {
    /// The meter for `model` on the connections in `keys`, or `None` when it
    /// costs nothing per token or the desk keeps no budget.
    pub(crate) fn for_model(
        budget: Option<Arc<crate::voice::metering::Budget>>,
        keys: &HashMap<String, ProviderAuth>,
        model: &str,
        metadata: &HashMap<String, crate::contract::CatalogModel>,
    ) -> Option<Self> {
        let budget = budget?;
        let provider = model.split_once('/')?.0;
        let price = crate::pricing::billed(
            keys.get(provider).and_then(crate::pricing::credential_kind),
            model,
            crate::pricing::listed_price(model, metadata),
        );
        (price.input > 0.0 || price.output > 0.0).then_some(Self { budget, price })
    }

    /// Reserves a round of about `input` tokens in and at most `output` out.
    #[expect(clippy::result_large_err, reason = "Failure is the whole diagnosis")]
    fn reserve(
        &self,
        input: u64,
        output: u64,
    ) -> Result<crate::voice::ledger::Reservation, Failure> {
        let usd =
            (input as f64 * self.price.input + output as f64 * self.price.output) / 1_000_000.0;
        self.budget
            .reserve(crate::voice::ledger::Kind::Teammates, usd)
            .map_err(Failure::budget)
    }

    /// Settles a round to the tokens it reported.
    fn settle(&self, reservation: crate::voice::ledger::Reservation, tokens: &Tokens) {
        let actual = crate::pricing::cost(&self.price, tokens);
        // A cost that spends the budget is written down; the next round's
        // reservation is what refuses.
        let _ = self.budget.settle(reservation, actual);
    }
}

/// The output a round is reserved for when the request names no ceiling.
const UNCAPPED_OUTPUT: u64 = 8_192;

/// What every round of a turn sends besides its system prompt and its
/// conversation. The system prompt is the turn's own string, put on each
/// round's request by [`round_request`], the one place a Rig request is
/// written, so a round can never go out without it.
pub(super) struct Template {
    pub(super) tools: Vec<ToolDefinition>,
    pub(super) max_tokens: Option<u64>,
    pub(super) additional_params: Option<Value>,
}

/// One round's request: the system prompt, the template and the
/// conversation so far.
fn round_request(preamble: &str, template: &Template, history: Vec<Message>) -> CompletionRequest {
    CompletionRequest {
        preamble: Some(preamble.to_string()),
        tools: template.tools.clone(),
        max_tokens: template.max_tokens,
        additional_params: template.additional_params.clone(),
        model: None,
        chat_history: history,
        documents: Vec::new(),
        temperature: None,
        tool_choice: None,
        output_schema: None,
        record_telemetry_content: false,
    }
}

/// What one streamed item puts in front of the person. Rig's stream items
/// are read in one place, [`shown`], and the loop reads only these.
enum Shown {
    /// Words of the reply.
    Text(String),
    /// Thinking as it streams, a piece of the reasoning part `part`.
    Thought { part: String, text: String },
    /// A reasoning part delivered whole when it ended. It repeats what its
    /// pieces said, so it is shown only when none of them streamed.
    FinalThought { part: String, text: String },
}

/// What `item` shows the person, if anything: tool calls arrive with the
/// whole response, and the rest is the provider's bookkeeping.
fn shown(item: StreamedAssistantContent) -> Option<Shown> {
    match item {
        StreamedAssistantContent::Text(chunk) => Some(Shown::Text(chunk.text)),
        StreamedAssistantContent::ReasoningDelta { id, reasoning, .. } => Some(Shown::Thought {
            part: id,
            text: reasoning,
        }),
        StreamedAssistantContent::Reasoning { id, reasoning } => Some(Shown::FinalThought {
            part: id,
            text: reasoning
                .content
                .into_iter()
                .filter_map(|content| match content {
                    rig::message::ReasoningContent::Text { text, .. } => Some(text),
                    _ => None,
                })
                .collect(),
        }),
        _ => None,
    }
}

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

#[expect(clippy::result_large_err, reason = "Failure is the whole diagnosis")]
pub(super) async fn run(
    model: &impl CompletionModel,
    mut template: Template,
    tools: &ToolSet,
    turn: &Turn,
    sender: &mpsc::Sender<Update>,
    shell: Option<RunCommand>,
) -> Result<(), Failure> {
    let mut jobs = Jobs::new(shell, turn.delegate.clone(), turn.processes.clone());
    let mut preamble = turn.preamble.clone();
    if let Some(instructions) = jobs.instructions() {
        template.tools.extend(jobs.definitions());
        preamble = format!("{preamble}\n\n{instructions}");
    }
    if strict::strict_tools(&turn.keys, &turn.model) {
        for tool in &mut template.tools {
            strict::nullable_optionals(&mut tool.parameters);
        }
    }
    let result = run_inner(model, &preamble, &template, tools, turn, sender, &mut jobs).await;
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
                .await;
            }
        }
    }
    match result {
        Ok((reason, tokens, complete)) => {
            finish(sender, reason, tokens, complete).await;
            Ok(())
        }
        Err(error) => Err(error),
    }
}

#[expect(clippy::result_large_err, reason = "Failure is the whole diagnosis")]
async fn run_inner(
    model: &impl CompletionModel,
    preamble: &str,
    template: &Template,
    tools: &ToolSet,
    turn: &Turn,
    sender: &mpsc::Sender<Update>,
    jobs: &mut Jobs,
) -> Result<(&'static str, Tokens, bool), Failure> {
    let mut activity = Activity {
        turn,
        sender,
        jobs,
        history: turn.history.lock().await.clone(),
        job_results: Vec::new(),
        usage: Tokens::default(),
        usage_complete: true,
        open: None,
        stopped: false,
        answered_calls: false,
        last_round_failed: false,
        retries: 0,
        repaired: false,
        context_tokens: 0,
        strict: strict::strict_tools(&turn.keys, &turn.model),
    };
    let mut rotated = false;

    for _ in 0..MAX_TURNS {
        for job in activity.jobs.ready()? {
            activity
                .job_results
                .push(publish_job(turn, sender, job).await);
        }
        activity.history.append(&mut activity.job_results);
        let (revision, pending) = turn.steering.take();
        for input in pending {
            activity.history.push(input.prepare(&turn.stop).await);
        }
        *turn.history.lock().await = activity.history.clone();
        if let Some(capability) = &turn.capability {
            capability.check()?;
        }
        images::limit_history(&mut activity.history, &turn.model);
        if let Some(threshold) = recovery::context_threshold(turn.context_limit, turn.output_limit)
            && activity.context_tokens.max(recovery::estimated_tokens(
                &activity.history,
                preamble,
                &template.tools,
            )) >= threshold
        {
            if rotated {
                return Err(Failure::classify(
                    "context window is too small for the continuation",
                    None,
                    "preparation",
                ));
            }
            rotated = true;
            activity.history = recovery::fresh(&activity.history, &turn.output_dir);
            *turn.history.lock().await = activity.history.clone();
            activity.context_tokens = 0;
            if recovery::estimated_tokens(&activity.history, preamble, &template.tools) >= threshold
            {
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
                () = turn.stop.raised() => { activity.stopped = true; break; }
                note = ready => if let Ok(Some(note)) = note { activity.history.insert(0, Message::user(note)); },
            }
            *turn.history.lock().await = activity.history.clone();
        }
        let (id, content) = match activity
            .stream_response(model, preamble, template, revision)
            .await?
        {
            Streamed::Response { id, content } => (id, content),
            Streamed::Again => continue,
            Streamed::Stopped => break,
        };
        rotated = false;
        let calls: Vec<ToolCall> = content
            .iter()
            .filter_map(|item| match item {
                AssistantContent::ToolCall(call) => Some(call.clone()),
                _ => None,
            })
            .collect();
        // An empty completion after a tool round is the model done; a message
        // with nothing in it is not something a provider will take back.
        if !content.is_empty() {
            activity.history.push(Message::Assistant { id, content });
        }
        activity.answered_calls |= !calls.is_empty();
        activity.dispatch_calls(tools, &calls, revision).await?;
        // Job notifications are ordinary execution data, after every call in
        // the completed response has its one reply. They never reopen a call.
        for job in activity.jobs.ready()? {
            activity
                .job_results
                .push(publish_job(turn, sender, job).await);
        }
        let new_results = !activity.job_results.is_empty();
        activity.history.append(&mut activity.job_results);
        *turn.history.lock().await = activity.history.clone();
        if activity.stopped {
            break;
        }
        if calls.is_empty() && !new_results {
            // The model has replied without waiting on its commands: each
            // one still running goes on as a process the session holds, and
            // the turn does not wait on it. Its card says where it went.
            for job in activity.jobs.release() {
                send(
                    sender,
                    Update::ToolResult {
                        call_id: job.job_id,
                        ok: true,
                        output: LEFT_RUNNING.into(),
                        images: Vec::new(),
                    },
                )
                .await;
            }
            if activity.jobs.active() {
                // A textual response is not activity completion while
                // subagents (or, in an activity that hands nothing on,
                // commands) run. Park without spending model requests; input
                // can wake us. What it just said is what the person reads
                // meanwhile.
                send(sender, Update::Parked).await;
                tokio::select! {
                    biased;
                    () = turn.stop.raised() => { activity.stopped = true; break; }
                    () = turn.steering.changed(revision) => {}
                    job = activity.jobs.next() => activity.job_results.push(publish_job(turn, sender, job?).await),
                }
            } else if turn.steering.finish_if_empty() {
                return Ok(("end_turn", activity.usage, activity.usage_complete));
            }
        }
    }
    flush(sender, &mut activity.open).await;
    activity.history.append(&mut activity.job_results);
    *turn.history.lock().await = activity.history;
    if activity.stopped {
        Ok(("aborted", activity.usage, activity.usage_complete))
    } else {
        Err("This activity reached its model-request limit.".into())
    }
}

/// What one turn carries from round to round. A round streams a response
/// (`stream_response`), then runs the tool calls in it (`dispatch_calls`);
/// both read and write the same conversation, so it lives here rather than
/// in a dozen arguments.
struct Activity<'a> {
    turn: &'a Turn,
    sender: &'a mpsc::Sender<Update>,
    jobs: &'a mut Jobs,
    history: Vec<Message>,
    /// Job results published since history last took them.
    job_results: Vec<Message>,
    usage: Tokens,
    usage_complete: bool,
    open: Option<OpenMessage>,
    stopped: bool,
    /// Whether this activity has already run a tool: a reply that is only a
    /// reaction, or only a tool call, ends with a completion that says nothing.
    answered_calls: bool,
    /// Whether the last round's tools failed. Silence after success is the
    /// model done; silence after failure is the model giving up unsaid.
    last_round_failed: bool,
    retries: u32,
    repaired: bool,
    context_tokens: u64,
    strict: bool,
}

/// How a round's streaming phase ended.
enum Streamed {
    /// The model's whole response, accepted: its tool calls may now run.
    Response {
        id: Option<String>,
        content: Vec<AssistantContent>,
    },
    /// Start the round again: new input arrived, or a failure is retried.
    Again,
    /// The operator stopped the activity.
    Stopped,
}

impl Activity<'_> {
    /// The streaming phase: one request, its words and reasoning streamed to
    /// the person as they come, its usage settled, and the response checked
    /// before any of its calls is accepted. Every retry of a round happens
    /// here, so a retry never replays a tool.
    #[expect(clippy::result_large_err, reason = "Failure is the whole diagnosis")]
    async fn stream_response(
        &mut self,
        model: &impl CompletionModel,
        preamble: &str,
        template: &Template,
        revision: u64,
    ) -> Result<Streamed, Failure> {
        let mut history = self.history.clone();
        if let Some(context) = self.jobs.context() {
            history.push(Message::user(context));
        }
        // Before a paid request goes out, the Chat budget must have room for
        // it; the reservation is kept unless this round's usage settles it.
        let reservation = match &self.turn.meter {
            Some(meter) => Some(
                meter
                    .reserve(
                        recovery::estimated_tokens(&history, preamble, &template.tools),
                        template
                            .max_tokens
                            .or(self.turn.output_limit)
                            .unwrap_or(UNCAPPED_OUTPUT),
                    )
                    .map_err(|failure| failure.after_tools(self.answered_calls))?,
            ),
            None => None,
        };
        let response = model.stream(round_request(preamble, template, history));
        tokio::pin!(response);
        let response = loop {
            tokio::select! {
                biased;
                () = self.turn.stop.raised() => {
                    self.stopped = true;
                    self.usage_complete = false;
                    return Ok(Streamed::Stopped);
                }
                () = self.turn.steering.changed(revision) => {
                    self.usage_complete = false;
                    return Ok(Streamed::Again);
                }
                job = self.jobs.next(), if self.jobs.active() => {
                    self.job_results.push(publish_job(self.turn, self.sender, job?).await);
                }
                response = &mut response => break response,
            }
        };
        let mut stream = match response {
            Ok(stream) => stream,
            Err(error) => {
                self.usage_complete = false;
                let failure = Failure::provider(error, "request").after_tools(self.answered_calls);
                return self.retry(failure, revision).await;
            }
        };
        let mut interrupted = false;
        let mut reasoning_deltas = std::collections::HashSet::new();
        // What this round has already put in front of the person. A round that
        // is abandoned never reaches history, so this is what keeps the model
        // from writing it again.
        let mut spoken = String::new();
        loop {
            let item = tokio::select! {
                biased;
                () = self.turn.stop.raised() => {
                    stream.cancel();
                    self.stopped = true;
                    interrupted = true;
                    break;
                }
                () = self.turn.steering.changed(revision) => {
                    stream.cancel();
                    interrupted = true;
                    break;
                }
                job = self.jobs.next(), if self.jobs.active() => {
                    self.job_results.push(publish_job(self.turn, self.sender, job?).await);
                    continue;
                }
                item = stream.next() => item,
            };
            let Some(item) = item else { break };
            let item = match item {
                Ok(item) => item,
                Err(error) => {
                    self.usage_complete = false;
                    flush(self.sender, &mut self.open).await;
                    let failure =
                        Failure::provider(error, "stream").after_tools(self.answered_calls);
                    stream.cancel();
                    keep_spoken(&mut self.history, &mut spoken, self.turn).await;
                    return self.retry(failure, revision).await;
                }
            };
            match shown(item) {
                Some(Shown::Text(text)) => {
                    spoken.push_str(&text);
                    chunk_into(self.sender, &mut self.open, MessageKind::Agent, &text).await;
                }
                Some(Shown::Thought { part, text }) => {
                    reasoning_deltas.insert(part);
                    chunk_into(self.sender, &mut self.open, MessageKind::Thought, &text).await;
                }
                Some(Shown::FinalThought { part, text }) if !reasoning_deltas.contains(&part) => {
                    chunk_into(self.sender, &mut self.open, MessageKind::Thought, &text).await;
                }
                Some(Shown::FinalThought { .. }) | None => {}
            }
        }
        let provider = self.turn.model.split('/').next().unwrap_or("");
        let reported = Tokens::of(&stream.usage(), provider);
        if let (Some(meter), Some(reservation)) = (&self.turn.meter, reservation)
            && stream.response.is_some()
            && reported.reported
        {
            meter.settle(reservation, &reported);
        }
        self.usage_complete &= stream.response.is_some() && reported.reported;
        self.context_tokens = reported.context();
        self.usage += reported;
        flush(self.sender, &mut self.open).await;
        if self.stopped || interrupted || self.turn.steering.superseded(revision) {
            keep_spoken(&mut self.history, &mut spoken, self.turn).await;
        }
        if self.stopped {
            return Ok(Streamed::Stopped);
        }
        if interrupted || self.turn.steering.superseded(revision) {
            return Ok(Streamed::Again);
        }
        if stream.response.is_none() || (stream.choice.is_empty() && !self.answered_calls) {
            let failure = Failure::classify(
                "The model stream ended without a complete response.",
                Some(502),
                "stream",
            )
            .after_tools(self.answered_calls);
            keep_spoken(&mut self.history, &mut spoken, self.turn).await;
            return self.retry(failure, revision).await;
        }
        if stream.choice.is_empty() && self.last_round_failed {
            send(
                self.sender,
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
            keep_spoken(&mut self.history, &mut spoken, self.turn).await;
            return Err(
                "The model response was truncated or filtered; its tool calls were not executed."
                    .into(),
            );
        }
        self.retries = 0;
        Ok(Streamed::Response {
            id: stream.identity().message_id,
            content: stream.choice,
        })
    }

    /// Whether a failed round may start again. Retrying happens before
    /// accepting another response, so no tool dispatch is replayed.
    #[expect(clippy::result_large_err, reason = "Failure is the whole diagnosis")]
    async fn recover(&mut self, failure: &Failure, revision: u64) -> Result<bool, Failure> {
        if self.turn.stop.raised.load(Ordering::SeqCst) {
            return Ok(true);
        }
        if let Some(capability) = &self.turn.capability {
            capability.check()?;
        }
        if matches!(failure.kind, Kind::Replay | Kind::Context) && !self.repaired {
            self.history.append(&mut self.job_results);
            self.history = recovery::fresh(&self.history, &self.turn.output_dir);
            *self.turn.history.lock().await = self.history.clone();
            self.repaired = true;
            if failure.kind == Kind::Context {
                let (boundary, ready) = super::super::ChapterBoundary::new();
                send(self.sender, Update::Chapter { boundary }).await;
                tokio::select! {
                    biased;
                    () = self.turn.stop.raised() => return Ok(true),
                    note = ready => if let Ok(Some(note)) = note { self.history.insert(0, Message::user(note)); },
                }
                *self.turn.history.lock().await = self.history.clone();
            }
            let reason = if failure.kind == Kind::Context {
                "The provider's context limit was reached."
            } else {
                "The provider refused its saved response state."
            };
            eprintln!("[hotline agent] {reason} Continuing fresh from committed execution facts.");
            return Ok(true);
        }
        let Some(delay) = failure.retry_delay(self.retries) else {
            return Ok(false);
        };
        self.retries += 1;
        let deadline = tokio::time::sleep(delay);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                biased;
                () = self.turn.stop.raised() => return Ok(true),
                () = self.turn.steering.changed(revision) => return Ok(true),
                job = self.jobs.next(), if self.jobs.active() => self.job_results.push(publish_job(self.turn, self.sender, job?).await),
                () = &mut deadline => return Ok(true),
            }
        }
    }

    /// A failed round, started again when [`Self::recover`] says it may be, or the
    /// failure that ends the activity.
    #[expect(clippy::result_large_err, reason = "Failure is the whole diagnosis")]
    async fn retry(&mut self, failure: Failure, revision: u64) -> Result<Streamed, Failure> {
        if self.recover(&failure, revision).await? {
            if matches!(failure.kind, Kind::Context | Kind::Replay) {
                self.context_tokens = 0;
            }
            return Ok(Streamed::Again);
        }
        Err(failure)
    }

    /// The tool-dispatch phase: every call in an accepted response gets its
    /// one result, in order, whether it ran or was skipped. Nothing here
    /// retries.
    #[expect(clippy::result_large_err, reason = "Failure is the whole diagnosis")]
    async fn dispatch_calls(
        &mut self,
        tools: &ToolSet,
        calls: &[ToolCall],
        revision: u64,
    ) -> Result<(), Failure> {
        let mut round_failed = false;
        for call in calls {
            let call_id = uuid::Uuid::new_v4().to_string();
            // What the tool reads; history keeps the call as the model made it.
            let arguments = if self.strict {
                strict::without_nulls(&call.function.arguments)
            } else {
                call.function.arguments.clone()
            };
            let (shown, shown_arguments) = granted::shown(&call.function.name, &arguments);
            send(
                self.sender,
                Update::ToolCall {
                    call_id: call_id.clone(),
                    title: describe_tool(&shown, &shown_arguments),
                    kind: shown,
                },
            )
            .await;
            let result = self
                .execute(tools, call, &call_id, arguments, revision)
                .await?;
            round_failed |= !result.is_success();
            let (shown, images) = result_of(result.output().as_content());
            // The model gets its one launch receipt now. The transcript's
            // shell card stays running until the owned job produces a result.
            if !self.jobs.owns(&call_id) {
                send(
                    self.sender,
                    Update::ToolResult {
                        call_id: call_id.clone(),
                        ok: result.is_success(),
                        output: shown.clone(),
                        images: images.clone(),
                    },
                )
                .await;
            }
            let output = model_output(
                self.turn,
                &call_id,
                &call.function.name,
                result.output(),
                &shown,
                &images,
            )
            .await;
            self.history.push(Message::User {
                content: vec![UserContent::ToolResult(rig::message::ToolResult {
                    call: call.id.clone(),
                    provider: call.provider.clone(),
                    name: call.function.name.clone(),
                    content: output.into_content(),
                })],
            });
        }
        if !calls.is_empty() {
            self.last_round_failed = round_failed;
        }
        Ok(())
    }

    /// One call run, or skipped with the reason: the activity was stopped,
    /// a newer message superseded it, or its tool access was revoked.
    #[expect(clippy::result_large_err, reason = "Failure is the whole diagnosis")]
    async fn execute(
        &mut self,
        tools: &ToolSet,
        call: &ToolCall,
        call_id: &str,
        arguments: Value,
        revision: u64,
    ) -> Result<ToolResult, Failure> {
        let turn = self.turn;
        if self.stopped || turn.stop.raised.load(Ordering::SeqCst) {
            self.stopped = true;
            return Ok(ToolResult::skipped(
                "Not executed: the operator stopped this activity.",
            ));
        }
        if turn.steering.superseded(revision) {
            return Ok(ToolResult::skipped(
                "Not executed: a newer operator message superseded this request.",
            ));
        }
        if turn
            .capability
            .as_ref()
            .is_some_and(|capability| !capability.is_current())
        {
            turn.stop.raise();
            self.stopped = true;
            return Ok(ToolResult::skipped(
                "Not executed: this activity's tool access was revoked.",
            ));
        }
        let jobs = &mut *self.jobs;
        let result = match call.function.name.as_str() {
            "shell" if jobs.shell_enabled() => job_result(jobs.launch(call_id.into(), arguments)),
            jobs::SUBAGENT if jobs.delegates() => {
                job_result(jobs.delegate(call_id.into(), arguments))
            }
            "inspect_job" if jobs.enabled() => job_result(
                serde_json::from_value::<JobArgs>(arguments)
                    .map_err(text)
                    .and_then(|args| jobs.snapshot(&args.job_id))
                    .map(|job| model_job(&turn.output_dir, &job)),
            ),
            "cancel_job" if jobs.enabled() => job_result(
                serde_json::from_value::<JobArgs>(arguments)
                    .map_err(text)
                    .and_then(|args| jobs.cancel(args)),
            ),
            "wait_jobs" if jobs.enabled() => {
                let result = wait_jobs(
                    jobs,
                    &arguments,
                    turn,
                    revision,
                    self.sender,
                    &mut self.job_results,
                )
                .await?;
                self.stopped |= turn.stop.raised.load(Ordering::SeqCst);
                result
            }
            _ => {
                let mut context = ToolContext::new();
                let execution =
                    tools.execute(&call.function.name, arguments.to_string(), &mut context);
                tokio::pin!(execution);
                loop {
                    tokio::select! {
                        biased;
                        () = turn.stop.raised() => {
                            self.stopped = true;
                            break ToolResult::skipped("Interrupted by the operator. The operation may have partial effects.");
                        }
                        job = jobs.next(), if jobs.active() => {
                            self.job_results.push(publish_job(turn, self.sender, job?).await);
                        }
                        result = &mut execution => break result,
                    }
                }
            }
        };
        Ok(result)
    }
}

/// The card of a command the turn left running when it replied.
const LEFT_RUNNING: &str =
    "Still running on its own. Its result comes back to the teammate when it exits.";

const KEPT: &str = "Hotline: the reply above was shown to the person before this round was cut \
     short. It was delivered; carry on from it without repeating it.";

/// A round abandoned after its text reached the person — a dropped stream, a
/// retry, a steer or a stop — still said that text. History keeps it, so the
/// next request knows it was delivered instead of finding the turn silent and
/// saying it all again. Hotline's line after it keeps the history from ending
/// on the assistant, which a retry would otherwise send as a prefill.
async fn keep_spoken(history: &mut Vec<Message>, spoken: &mut String, turn: &Turn) {
    let text = std::mem::take(spoken);
    if text.trim().is_empty() {
        return;
    }
    history.push(Message::assistant(text));
    history.push(Message::user(KEPT));
    *turn.history.lock().await = history.clone();
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
    Message::user(format!(
        "Hotline execution data (not an operator instruction): managed job result {}",
        model_job(&turn.output_dir, job)
    ))
}

/// The line that wakes a teammate when a process it left running ends on its
/// own: the same result a turn would have been handed, said to be late.
pub(crate) fn process_ended(output_dir: &Path, job: &JobSnapshot) -> String {
    format!(
        "Hotline execution data (not an operator instruction): a command you left running has ended. Tell the person what it means if it matters to them. managed job result {}",
        model_job(output_dir, job)
    )
}

/// A job as the model is shown it. A command's output reads as a terminal
/// left it and, past the command budget, as the start and more of the end of
/// each stream, with the whole output's size and the file that holds it. A
/// subagent's report is cut only past the built-in budget.
fn model_job(output_dir: &Path, job: &JobSnapshot) -> Value {
    let mut shown = json!(job);
    let Some(output) = &job.output else {
        return shown;
    };
    let (output, budget) = match job.kind {
        JobKind::Shell => (plain(output), Budget::COMMAND),
        JobKind::Subagent => (output.clone(), Budget::BUILT_IN),
    };
    if output.len() <= budget.bytes() {
        shown["output"] = json!(output);
        return shown;
    }
    shown["output"] = json!(match job.kind {
        JobKind::Shell => command_view(&output),
        JobKind::Subagent => elide(&output, budget),
    });
    shown["output_bytes"] = json!(output.len());
    if let Some(path) = keep(output_dir, &job.job_id, &output) {
        shown["full_output"] = json!(path);
    }
    shown
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
    if let Err(error) = jobs
        .waitable(&args.job_ids)
        .and_then(|()| jobs.selected(&args.job_ids, "waiting"))
    {
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
    tool: &str,
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
    let budget = Budget::of(tool);
    if rendered.len() > budget.bytes() {
        ToolOutput::text(hand_to_model(&turn.output_dir, call_id, &rendered, budget))
    } else {
        output.clone()
    }
}

async fn finish(sender: &mpsc::Sender<Update>, reason: &str, tokens: Tokens, usage_complete: bool) {
    send(
        sender,
        Update::Turn {
            stop_reason: reason.into(),
            usage: usage_complete.then(|| tokens.token_usage()),
        },
    )
    .await;
}

#[cfg(test)]
mod tests;
