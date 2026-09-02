//! Toad Agent: the driver that runs the model in this process, on Rig.
//!
//! A turn is a Rig multi-turn stream — text and reasoning deltas, the tools
//! the agent calls, their results, and a final response — and every one of
//! those becomes an [`Update`]. Nothing asks permission: a teammate's one
//! policy is how far its tools reach, and the session says which with every
//! prompt.
//!
//! Two things this driver cannot learn from the stream it gets, and how it
//! learns them anyway:
//!
//! - **Whether a tool failed.** The tool result Rig streams is the message
//!   the model will see (`{call, name, content}`); it carries no error flag,
//!   and a tool that returned `Err` is presented as its error text. Rig's
//!   canonical result — the one that knows — is offered to a hook, so
//!   [`ToolOutcomes`] is registered as one and the loop reads the outcome
//!   back by the call id the stream item carries.
//! - **That the human pressed Stop.** The turn waits on a [`Stop`] beside the
//!   stream; raising it abandons the stream, which drops the tool future in
//!   flight, which kills that command's process group. It is a raised flag
//!   and not a bare wake, because a turn between two of its own awaits is
//!   waiting nowhere, and a wake nobody is waiting for never happened.

use super::{Driver, DriverInfo, MessageKind, Update, clip};
use crate::contract::{
    AgentKind, Attachment, NoticeLevel, Persona, Reach, TokenUsage, ToolSourceKind,
};
use crate::mcp::server::TeammateTools;
use crate::mcp::{self, McpServer};
use crate::models::{self, Client};
use crate::session::ledger::ToolLedger;
use crate::session::{ProviderAuth, ProviderKeys};
use crate::tools::{
    self, EditFile, FindFiles, ListDirectory, ReadFile, RunCommand, SearchFiles, Workspace,
    WriteFile,
};
use async_trait::async_trait;
use futures_util::StreamExt;
use rig::agent::MultiTurnStreamItem;
use rig::agent::hook::{AgentHook, HookContext, ToolResultAction, ToolResultEvent};
use rig::message::{Message, ReasoningContent, ToolResultContent};
use rig::prelude::*;
use rig::providers::{
    anthropic, chatgpt, copilot, deepseek, gemini, groq, mistral, openai, openrouter, xai,
};
use rig::streaming::{StreamedAssistantContent, StreamedUserContent};
use rig::tool::{DynamicTool, ToolExecutionError, ToolOutput};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::{Mutex as AsyncMutex, Notify, mpsc};

/// How many rounds of tool calls one prompt may take. High enough that no
/// prompt reaches it: the human's Stop is the cap on a long-running turn, and
/// a ceiling that ends a turn on its own is a teammate that quits mid-job.
const MAX_TURNS: usize = 10_000;

/// The name a teammate on this driver answers to.
const AGENT_NAME: &str = "Toad Agent";

/// Only the tool's subject, not the whole command, goes in the title line.
const TITLE_CHARS: usize = 120;

/// How much of a tool's result the model is shown. Big enough for a build log;
/// anything larger is kept in full on disk beside the tape, and the text the
/// model sees ends with that path so the agent can read the rest.
const MODEL_TOOL_OUTPUT_BYTES: usize = 256 * 1024;

/// One answer, with no tools and no conversation: the note that closes a
/// chapter, and anything else that asks a model a single question.
///
/// This is not a session and does not become one. The system prompt is the
/// agent's preamble, which is how a Rig agent is told the rules for an answer
/// it will give exactly once.
pub async fn complete(
    keys: &HashMap<String, ProviderAuth>,
    model_id: &str,
    system: &str,
    prompt: &str,
) -> Result<String, String> {
    let agent = agent_builder(keys, model_id)?.preamble(system).build();
    agent
        .prompt(prompt)
        .await
        .map_err(|error| error.to_string())
}

/// One line of the conversation the agent is being started back into.
///
/// The session reads these off the tape, because the tape is the record and a
/// driver does not know there is one. Only what was said is seeded: tool calls
/// and their output are the agent's own working memory of a turn, not the
/// conversation, and replaying them would put a stale filesystem in the model's
/// head as if it were current.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Said {
    User(String),
    Agent(String),
}

/// Toad Agent for one teammate.
pub struct InProcess {
    keys: Arc<dyn ProviderKeys>,
    preamble: String,
    /// Learned at `start`, from the persona: where relative paths start and
    /// where commands run.
    cwd: Mutex<PathBuf>,
    model: Mutex<String>,
    history: Arc<AsyncMutex<Vec<Message>>>,
    /// The stop for the turn in flight. A fresh one per prompt, so a stop
    /// nobody was running is not still standing over the next turn.
    stop: Mutex<Arc<Stop>>,
    /// Directory oversized tool results are written into, one `{call id}.txt`
    /// each. Created on first use so a teammate that never overflows never
    /// gets a folder.
    output_dir: PathBuf,
    /// Servers this teammate's policy granted. Connected at `start`, live
    /// until this driver is dropped.
    mcp_servers: Vec<McpServer>,
    /// Policy ids that named a server the room no longer has.
    mcp_missing: Vec<String>,
    mcp: Mutex<Option<mcp::Connections>>,
    /// This teammate's tools over its own conversation. In this process they
    /// are the functions themselves, not a server reached over a transport:
    /// Toad Agent and Toad's MCP server are two halves of one program.
    teammate: TeammateTools,
}

impl InProcess {
    pub fn new(
        keys: Arc<dyn ProviderKeys>,
        preamble: String,
        said: Vec<Said>,
        output_dir: PathBuf,
        teammate: TeammateTools,
    ) -> Self {
        let history = said
            .into_iter()
            .map(|line| match line {
                Said::User(text) => Message::user(text),
                Said::Agent(text) => Message::assistant(text),
            })
            .collect();
        Self {
            keys,
            preamble,
            cwd: Mutex::new(PathBuf::new()),
            model: Mutex::new(String::new()),
            history: Arc::new(AsyncMutex::new(history)),
            stop: Mutex::new(Arc::new(Stop::default())),
            output_dir,
            mcp_servers: Vec::new(),
            mcp_missing: Vec::new(),
            mcp: Mutex::new(None),
            teammate,
        }
    }

    /// The MCP servers this teammate may use, selected before the driver
    /// is built so `start` can connect without asking the room again.
    pub fn with_mcp(mut self, servers: Vec<McpServer>, missing: Vec<String>) -> Self {
        self.mcp_servers = servers;
        self.mcp_missing = missing;
        self
    }

    fn info(&self, keys: &HashMap<String, ProviderAuth>) -> DriverInfo {
        let model = lock(&self.model).clone();
        DriverInfo {
            agent_name: AGENT_NAME.to_string(),
            models: models::choices(keys),
            model_label: models::label_of(&model),
            current_model_id: model,
            ..DriverInfo::default()
        }
    }
}

#[async_trait]
impl Driver for InProcess {
    async fn start(&self, persona: &Persona) -> Result<DriverInfo, String> {
        let keys = self.keys.provider_auth();
        let model = match &persona.model_id {
            Some(id) if keys.contains_key(id.split('/').next().unwrap_or("")) => id.clone(),
            _ => models::choices(&keys)
                .first()
                .map(|model| model.id.clone())
                .ok_or_else(|| {
                    "No model key is available to Toad Agent. Add a provider key under Settings → Agents."
                        .to_string()
                })?,
        };
        *lock(&self.cwd) = PathBuf::from(&persona.cwd);
        *lock(&self.model) = model;
        let connected = mcp::connect(&persona.id, &self.mcp_servers).await;
        publish_ledger(persona, &self.mcp_missing, &connected);
        *lock(&self.mcp) = Some(connected);
        Ok(self.info(&keys))
    }

    async fn prompt(
        &self,
        text: String,
        attachments: Vec<Attachment>,
        reach: Reach,
    ) -> mpsc::Receiver<Update> {
        let text = with_paths(&text, &attachments);
        let (sender, receiver) = mpsc::channel(UPDATE_DEPTH);
        // The stop this turn answers to, installed before the turn is spawned
        // so a Stop pressed the instant the prompt returns still finds it.
        let stop = Arc::new(Stop::default());
        *lock(&self.stop) = stop.clone();
        let mut mcp_tools: Vec<DynamicTool> = self.teammate.as_dynamic();
        if let Some(connected) = lock(&self.mcp).as_ref() {
            mcp_tools.extend(
                connected
                    .tools
                    .iter()
                    .cloned()
                    .map(|tool| mcp_dynamic(tool, sender.clone())),
            );
        }
        let turn = Turn {
            keys: self.keys.provider_auth(),
            model: lock(&self.model).clone(),
            preamble: self.preamble.clone(),
            cwd: lock(&self.cwd).clone(),
            reach,
            history: self.history.clone(),
            stop,
            output_dir: self.output_dir.clone(),
            mcp_tools,
        };
        tokio::spawn(async move {
            if let Err(error) = turn.run(&sender, text).await {
                let _ = sender
                    .send(Update::Notice {
                        level: NoticeLevel::Error,
                        text: format!("Turn failed: {error}"),
                    })
                    .await;
            }
        });
        receiver
    }

    fn cancel(&self) {
        lock(&self.stop).raise();
    }

    async fn set_model(&self, model_id: &str) -> Result<DriverInfo, String> {
        let keys = self.keys.provider_auth();
        let provider = model_id.split('/').next().unwrap_or("");
        if !keys.contains_key(provider) {
            return Err(format!("No key for {provider} is available on this desk."));
        }
        *lock(&self.model) = model_id.to_string();
        Ok(self.info(&keys))
    }
}

/// How many updates may be in flight before the turn waits for the session to
/// catch up. Deltas arrive faster than anything else on this channel, and a
/// turn that outran its reader would either grow without bound or lose text;
/// waiting is the only answer that keeps the message whole.
const UPDATE_DEPTH: usize = 256;

/// The human's Stop, for one turn.
///
/// A latch and not a wake. `Notify::notify_waiters` reaches only what is
/// registered at that instant, and a turn spends most of itself elsewhere —
/// waiting on the first model round trip, or parked pushing an update into a
/// full channel. Stop pressed in any of those moments used to be discarded
/// and the turn ran on, tools and all. The flag is raised first and the wake
/// second, so a waiter that reads the flag as clear is a waiter the wake has
/// not yet passed.
#[derive(Default)]
struct Stop {
    raised: AtomicBool,
    woken: Notify,
}

impl Stop {
    fn raise(&self) {
        self.raised.store(true, Ordering::SeqCst);
        self.woken.notify_waiters();
    }

    /// Resolves once this turn has been stopped, whether that happened while
    /// something was waiting here or before anything was.
    async fn raised(&self) {
        loop {
            let woken = self.woken.notified();
            if self.raised.load(Ordering::SeqCst) {
                return;
            }
            woken.await;
        }
    }
}

/// Everything one turn needs, taken from the session at the moment it starts
/// so the turn owns it and the driver stays free to answer other calls.
struct Turn {
    keys: HashMap<String, ProviderAuth>,
    model: String,
    preamble: String,
    cwd: PathBuf,
    reach: Reach,
    history: Arc<AsyncMutex<Vec<Message>>>,
    stop: Arc<Stop>,
    output_dir: PathBuf,
    mcp_tools: Vec<DynamicTool>,
}

impl Turn {
    async fn run(&self, sender: &mpsc::Sender<Update>, text: String) -> Result<(), String> {
        // Taken first so a failure after we have the prompt still keeps the
        // line: cancel already did, and a retry without the question reaches
        // the model as a stranger.
        let mut history = self.history.lock().await;
        let workspace = match Workspace::open(self.cwd.clone(), self.reach) {
            Ok(workspace) => workspace,
            Err(error) => {
                remember_prompt(&mut history, &text);
                return Err(error.to_string());
            }
        };
        let outcomes = ToolOutcomes::new(self.output_dir.clone());
        let agent = match agent_builder(&self.keys, &self.model) {
            Ok(builder) => builder
                .preamble(&self.preamble)
                .tool(ListDirectory::new(workspace.clone()))
                .tool(ReadFile::new(workspace.clone()))
                .tool(SearchFiles::new(workspace.clone()))
                .tool(FindFiles::new(workspace.clone()))
                .tool(WriteFile::new(workspace.clone()))
                .tool(EditFile::new(workspace.clone()))
                .tool(RunCommand::new(workspace))
                .dynamic_tools(self.mcp_tools.clone())
                .add_hook(outcomes.clone())
                .default_max_turns(MAX_TURNS)
                .build(),
            Err(error) => {
                remember_prompt(&mut history, &text);
                return Err(error);
            }
        };
        let mut stream = agent
            .stream_chat(text.as_str(), history.clone())
            .max_turns(MAX_TURNS)
            .await;

        let mut open: Option<OpenMessage> = None;
        let mut usage = (0i64, 0i64, 0i64);
        let mut ended = false;

        loop {
            let item = tokio::select! {
                () = self.stop.raised() => {
                    flush(sender, &mut open).await;
                    send(sender, Update::Turn { stop_reason: "aborted".to_string(), usage: None }).await;
                    history.push(Message::user(text));
                    return Ok(());
                }
                item = stream.next() => item,
            };
            let Some(item) = item else { break };
            let item = match item {
                Ok(item) => item,
                Err(error) => {
                    remember_prompt(&mut history, &text);
                    return Err(error.to_string());
                }
            };
            match item {
                MultiTurnStreamItem::StreamAssistantItem(content) => match content {
                    StreamedAssistantContent::Text(chunk) => {
                        chunk_into(sender, &mut open, MessageKind::Agent, &chunk.text).await;
                    }
                    StreamedAssistantContent::ReasoningDelta { reasoning, .. } => {
                        chunk_into(sender, &mut open, MessageKind::Thought, &reasoning).await;
                    }
                    StreamedAssistantContent::Reasoning { reasoning, .. } => {
                        for part in reasoning.content {
                            if let ReasoningContent::Text { text, .. } = part {
                                chunk_into(sender, &mut open, MessageKind::Thought, &text).await;
                            }
                        }
                    }
                    StreamedAssistantContent::ToolCall {
                        tool_call,
                        internal_call_id,
                    } => {
                        flush(sender, &mut open).await;
                        send(
                            sender,
                            Update::ToolCall {
                                call_id: internal_call_id,
                                title: describe_tool(
                                    &tool_call.function.name,
                                    &tool_call.function.arguments,
                                ),
                                kind: tool_call.function.name,
                            },
                        )
                        .await;
                    }
                    _ => {}
                },
                MultiTurnStreamItem::StreamUserItem(StreamedUserContent::ToolResult {
                    tool_result,
                    internal_call_id,
                }) => {
                    let output = tool_result
                        .content
                        .iter()
                        .map(|item| match item {
                            ToolResultContent::Text(text) => text.text.clone(),
                            ToolResultContent::Json { .. } => "[structured result]".to_string(),
                            _ => "[non-text result]".to_string(),
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    let ok = outcomes.take(&internal_call_id);
                    send(
                        sender,
                        Update::ToolResult {
                            call_id: internal_call_id,
                            ok,
                            output,
                        },
                    )
                    .await;
                }
                MultiTurnStreamItem::CompletionCall(call) => {
                    usage.0 += call.usage.input_tokens as i64;
                    usage.1 += call.usage.output_tokens as i64;
                    usage.2 += call.usage.total_tokens as i64;
                }
                MultiTurnStreamItem::FinalResponse(response) => {
                    flush(sender, &mut open).await;
                    send(
                        sender,
                        Update::Turn {
                            stop_reason: "end_turn".to_string(),
                            usage: Some(TokenUsage {
                                input_tokens: Some(usage.0),
                                output_tokens: Some(usage.1),
                                total_tokens: Some(usage.2),
                            }),
                        },
                    )
                    .await;
                    match response.messages {
                        Some(messages) => *history = messages,
                        None => {
                            history.push(Message::user(text.clone()));
                            history.push(Message::assistant(response.output));
                        }
                    }
                    ended = true;
                }
                _ => {}
            }
        }
        flush(sender, &mut open).await;
        if !ended {
            remember_prompt(&mut history, &text);
            return Err("the model ended the turn without a response".to_string());
        }
        Ok(())
    }
}

/// A failed turn still owes the model the question that started it, so a
/// retry is a continuation rather than a stranger asking something new.
fn remember_prompt(history: &mut Vec<Message>, text: &str) {
    history.push(Message::user(text));
}

/// Which tool calls failed, by the id the stream will name them with, and the
/// rewrite that keeps an oversized result off the model's context.
///
/// Rig hands the canonical result — the one carrying the disposition — only to
/// a hook, so this is registered as one and the turn loop reads the outcome
/// back. The same hook is the only place that can change what the model is
/// shown, so a result that does not fit is written to disk here and rewritten
/// to the elided text plus that path.
#[derive(Clone)]
struct ToolOutcomes {
    outcomes: Arc<Mutex<HashMap<String, bool>>>,
    output_dir: PathBuf,
}

impl ToolOutcomes {
    fn new(output_dir: PathBuf) -> Self {
        Self {
            outcomes: Arc::new(Mutex::new(HashMap::new())),
            output_dir,
        }
    }

    /// Whether that call succeeded. A call the hook never saw reads as
    /// succeeded: the transcript's job is to mark the failures it knows about,
    /// not to accuse a tool of failing because Rig went quiet.
    fn take(&self, internal_call_id: &str) -> bool {
        lock(&self.outcomes)
            .remove(internal_call_id)
            .unwrap_or(true)
    }
}

impl AgentHook for ToolOutcomes {
    async fn on_tool_result(
        &self,
        _context: &HookContext,
        event: ToolResultEvent<'_>,
    ) -> ToolResultAction {
        lock(&self.outcomes).insert(
            event.internal_call_id.to_string(),
            event.raw_result.is_success(),
        );
        let text = event.presentation.render();
        if text.len() <= MODEL_TOOL_OUTPUT_BYTES {
            return ToolResultAction::Keep;
        }
        ToolResultAction::rewrite(hand_to_model(
            &self.output_dir,
            event.internal_call_id,
            &text,
        ))
    }
}

/// The text the model is given for a tool result: the whole thing when it
/// fits, otherwise the head and tail with the rest written to
/// `{output_dir}/{call_id}.txt` and that path on the last line.
fn hand_to_model(output_dir: &Path, call_id: &str, output: &str) -> String {
    if output.len() <= MODEL_TOOL_OUTPUT_BYTES {
        return output.to_string();
    }
    let elided = elide(output, MODEL_TOOL_OUTPUT_BYTES);
    let path = output_dir.join(format!("{call_id}.txt"));
    match std::fs::create_dir_all(output_dir).and_then(|_| std::fs::write(&path, output)) {
        Ok(()) => {
            let named = path.canonicalize().unwrap_or(path);
            format!("{elided}\nFull output: {}", named.display())
        }
        Err(_) => elided,
    }
}

/// The head and the tail of the output, with one line where the middle was.
///
/// A cut in the middle is the honest one: the head holds what the command
/// said it was doing and the tail holds how it ended, and a build log that
/// only kept its first quarter would hide the error the agent ran it for.
fn elide(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let half = limit / 2;
    let head = floor_boundary(text, half);
    let tail = ceil_boundary(text, text.len() - half);
    let cut = tail - head;
    format!(
        "{}\n[… {cut} bytes elided …]\n{}",
        &text[..head],
        &text[tail..]
    )
}

fn floor_boundary(text: &str, at: usize) -> usize {
    (0..=at)
        .rev()
        .find(|at| text.is_char_boundary(*at))
        .unwrap_or(0)
}

fn ceil_boundary(text: &str, at: usize) -> usize {
    (at..=text.len())
        .find(|at| text.is_char_boundary(*at))
        .unwrap_or(text.len())
}

/// A message being streamed: its id, whether it is speech or thought, and
/// what has arrived so far.
struct OpenMessage {
    id: String,
    kind: MessageKind,
    text: String,
}

async fn send(sender: &mpsc::Sender<Update>, update: Update) {
    let _ = sender.send(update).await;
}

/// Adds text to the message being streamed, opening one — and closing any
/// message of the other kind — when the agent changes voice.
async fn chunk_into(
    sender: &mpsc::Sender<Update>,
    open: &mut Option<OpenMessage>,
    kind: MessageKind,
    text: &str,
) {
    if text.is_empty() {
        return;
    }
    if open.as_ref().is_none_or(|message| message.kind != kind) {
        flush(sender, open).await;
        *open = Some(OpenMessage {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            text: String::new(),
        });
    }
    let Some(message) = open.as_mut() else { return };
    message.text.push_str(text);
    send(
        sender,
        Update::Delta {
            kind,
            message_id: message.id.clone(),
            text: text.to_string(),
        },
    )
    .await;
}

/// Closes the streamed message: one durable [`Update::Message`] holding
/// everything its deltas carried.
async fn flush(sender: &mpsc::Sender<Update>, open: &mut Option<OpenMessage>) {
    let Some(message) = open.take() else { return };
    if message.text.is_empty() {
        return;
    }
    send(
        sender,
        Update::Message {
            kind: message.kind,
            id: message.id,
            text: message.text,
        },
    )
    .await;
}

/// The builder for one model on the provider whose credential the desk holds.
fn agent_builder(
    keys: &HashMap<String, ProviderAuth>,
    model_id: &str,
) -> Result<rig::agent::AgentBuilder, String> {
    let (provider, model) = model_id
        .split_once('/')
        .ok_or_else(|| format!("{model_id} is not a provider/model id"))?;
    let held = keys
        .get(provider)
        .ok_or_else(|| format!("No key for {provider} is available on this desk."))?;
    let wiring = models::wiring(provider)
        .ok_or_else(|| format!("{provider} is not a provider Toad Agent can use"))?;
    let builder = match (wiring.client, held) {
        (Client::ChatGpt, ProviderAuth::Login { token_dir }) => chatgpt::Client::builder()
            .oauth()
            .auth_file(token_dir.join("auth.json"))
            .allow_device_flow(false)
            .build()
            .map_err(text)?
            .agent(model),
        (Client::Copilot, ProviderAuth::Login { token_dir }) => copilot::Client::builder()
            .oauth()
            .token_dir(token_dir)
            .allow_device_flow(false)
            .build()
            .map_err(text)?
            .agent(model),
        (Client::ChatGpt | Client::Copilot, ProviderAuth::ApiKey(_)) => {
            return Err(format!("{provider} needs a sign-in, not a key."));
        }
        (_, ProviderAuth::Login { .. }) => {
            return Err(format!("{provider} needs a key, not a sign-in."));
        }
        (Client::Anthropic, ProviderAuth::ApiKey(key)) => {
            anthropic::Client::new(key).map_err(text)?.agent(model)
        }
        (Client::OpenAi, ProviderAuth::ApiKey(key)) => {
            openai::Client::new(key).map_err(text)?.agent(model)
        }
        (Client::OpenRouter, ProviderAuth::ApiKey(key)) => {
            openrouter::Client::new(key).map_err(text)?.agent(model)
        }
        (Client::Gemini, ProviderAuth::ApiKey(key)) => {
            gemini::Client::new(key).map_err(text)?.agent(model)
        }
        (Client::XAi, ProviderAuth::ApiKey(key)) => {
            xai::Client::new(key).map_err(text)?.agent(model)
        }
        (Client::Groq, ProviderAuth::ApiKey(key)) => {
            groq::Client::new(key).map_err(text)?.agent(model)
        }
        (Client::DeepSeek, ProviderAuth::ApiKey(key)) => {
            deepseek::Client::new(key).map_err(text)?.agent(model)
        }
        (Client::Mistral, ProviderAuth::ApiKey(key)) => {
            mistral::Client::new(key).map_err(text)?.agent(model)
        }
    };
    // Anthropic refuses a request that names no ceiling, and Rig only knows
    // one for the models it shipped with. The catalogue knows every model's,
    // so every request carries it rather than only the ones Rig remembers.
    Ok(match models::output_limit(model_id) {
        Some(ceiling) => builder.max_tokens(ceiling),
        None => builder,
    })
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn lock<T>(held: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    held.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The message with the attached paths under it, because this agent opens a
/// file with its read tool rather than being handed its bytes.
fn with_paths(text: &str, attachments: &[Attachment]) -> String {
    if attachments.is_empty() {
        return text.to_string();
    }
    let paths: Vec<&str> = attachments
        .iter()
        .map(|attachment| attachment.path.as_str())
        .collect();
    format!("{text}\n\nAttached files:\n{}", paths.join("\n"))
}

/// The Rig adapter for one granted MCP tool. A transport death is already
/// on the ledger; the notice rides this turn's update channel so the
/// session writes it on the tape, once, without the tool knowing what a
/// tape is.
fn mcp_dynamic(tool: mcp::McpTool, notices: mpsc::Sender<Update>) -> DynamicTool {
    DynamicTool::new(
        tool.name.clone(),
        tool.description.clone(),
        tool.parameters.clone(),
        move |_context, arguments| {
            let tool = tool.clone();
            let notices = notices.clone();
            Box::pin(async move {
                match tool.call(arguments).await {
                    Ok(text) => Ok(ToolOutput::text(text)),
                    Err(mcp::CallError::Transport { message, notice }) => {
                        if let Some(text) = notice {
                            let _ = notices
                                .send(Update::Notice {
                                    level: NoticeLevel::Warn,
                                    text,
                                })
                                .await;
                        }
                        Err(ToolExecutionError::other(message))
                    }
                    Err(mcp::CallError::Tool(message)) => Err(ToolExecutionError::other(message)),
                }
            })
        },
    )
}

/// The ledger reads what this session was actually built with, not what
/// the configuration promised. Built-ins are verified because Toad handed
/// them to the agent; MCP tools are verified when the server listed them,
/// and absent — with the error as the reason — when it did not.
fn publish_ledger(persona: &Persona, missing: &[String], connected: &mcp::Connections) {
    let mut ledger = ToolLedger::new(
        persona.id.clone(),
        AgentKind::Pi,
        persona.backend_id.clone(),
    );
    ledger.all(
        crate::contract::ToolState::Verified,
        ToolSourceKind::Builtin,
        "pi",
        tools::BUILTIN,
        "Toad handed them to the agent",
    );
    // Toad's own tools are built here, not connected to: the agent and the
    // server are the same process, so there is nothing to observe and nothing
    // that can have gone wrong between them.
    ledger.all(
        crate::contract::ToolState::Verified,
        ToolSourceKind::Builtin,
        mcp::server::SERVER_NAME,
        &mcp::server::TOOL_NAMES,
        "Toad's own tools, called in this process",
    );
    for tool in &connected.tools {
        ledger.verified(
            ToolSourceKind::Mcp,
            &tool.origin,
            &tool.name,
            format!("attached from the {} MCP server", tool.origin),
        );
    }
    for failed in &connected.failed {
        ledger.absent(ToolSourceKind::Mcp, &failed.id, &failed.id, &failed.reason);
    }
    for id in missing {
        ledger.absent(ToolSourceKind::Mcp, id, id, mcp::missing_reason(id));
    }
    ledger.publish();
}

/// A tool call as a line in the transcript: the name and the one argument
/// that says what it touched.
fn describe_tool(name: &str, arguments: &Value) -> String {
    let subject = ["path", "command", "pattern", "query", "expression"]
        .iter()
        .find_map(|key| arguments.get(*key).and_then(Value::as_str))
        .map(|value| clip(value, TITLE_CHARS));
    match subject {
        Some(subject) => format!("{name} {subject}"),
        None => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::Reach;
    use rig::tool::{Tool, ToolContext};
    use serde_json::json;

    /// Toad Agent is handed paths rather than bytes, because it opens a file
    /// with its read tool.
    #[test]
    fn attachments_reach_this_agent_as_paths_under_the_message() {
        use crate::contract::AttachmentKind;
        let attachment = Attachment {
            kind: AttachmentKind::File,
            name: "note.txt".to_string(),
            path: "/tmp/note.txt".to_string(),
            mime_type: None,
            size: None,
        };
        assert_eq!(with_paths("look", &[]), "look");
        assert_eq!(
            with_paths("look", std::slice::from_ref(&attachment)),
            "look\n\nAttached files:\n/tmp/note.txt"
        );
    }

    #[test]
    fn a_tool_call_is_described_by_what_it_touched() {
        assert_eq!(
            describe_tool("read", &json!({"path": "src/x.rs"})),
            "read src/x.rs"
        );
        assert_eq!(
            describe_tool("shell", &json!({"command": "ls"})),
            "shell ls"
        );
        assert_eq!(describe_tool("glob", &json!({})), "glob");
    }

    /// A tool nobody reported on succeeded; one the hook saw fail did not, and
    /// the answer is taken only once because a tape event is written once.
    #[test]
    fn a_tool_outcome_is_read_back_by_the_call_id_the_stream_names() {
        let outcomes = ToolOutcomes::new(PathBuf::new());
        assert!(outcomes.take("call-1"));
        lock(&outcomes.outcomes).insert("call-1".to_string(), false);
        assert!(!outcomes.take("call-1"));
        assert!(outcomes.take("call-1"));
    }

    #[test]
    fn output_under_the_limit_is_untouched() {
        assert_eq!(elide("hello", 16), "hello");
    }

    /// The head and the tail both survive, and the line between them says how
    /// much did not.
    #[test]
    fn a_long_output_keeps_both_ends_and_says_what_it_cut() {
        let text = format!("start{}end", "x".repeat(1_000));
        let elided = elide(&text, 100);
        assert!(elided.starts_with("start"));
        assert!(elided.ends_with("end"));
        assert!(elided.contains("[… 908 bytes elided …]"), "{elided}");
    }

    /// The cut lands on a character boundary, never inside one.
    #[test]
    fn a_multibyte_output_is_cut_between_characters() {
        let text = "é".repeat(1_000);
        let elided = elide(&text, 101);
        assert!(elided.contains("[…"));
        assert!(elided.starts_with('é'));
        assert!(elided.ends_with('é'));
    }

    /// A command whose output exceeds what the model is handed lands its full
    /// output on disk, and the text the model sees names that path.
    #[tokio::test]
    async fn a_shell_command_whose_output_exceeds_the_elision_lands_on_disk() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "toad-core-tool-output-{}-{nonce}",
            std::process::id()
        ));
        let workspace_dir = root.join("workspace");
        let output_dir = root.join("tool-output");
        std::fs::create_dir_all(&workspace_dir).unwrap();
        let body = "x".repeat(MODEL_TOOL_OUTPUT_BYTES + 64);
        std::fs::write(workspace_dir.join("big.txt"), &body).unwrap();
        let workspace = Workspace::open(workspace_dir, Reach::Workspace).unwrap();
        let args: <RunCommand as Tool>::Args =
            serde_json::from_value(json!({"command": "cat big.txt"})).unwrap();
        let output = RunCommand::new(workspace)
            .call(&mut ToolContext::new(), args)
            .await
            .unwrap();
        assert_eq!(output, body);

        let handed = hand_to_model(&output_dir, "call-1", &output);
        let saved = output_dir.join("call-1.txt");
        assert_eq!(std::fs::read_to_string(&saved).unwrap(), body);
        let named = saved.canonicalize().unwrap();
        assert!(handed.contains(&named.display().to_string()), "{handed}");
        assert!(handed.contains("elided"), "{handed}");
        assert!(handed.starts_with('x') && handed.contains("Full output:"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The bug this replaced: a turn is only registered on the stop while it
    /// is polling for the next stream item, and it spends the first model
    /// round trip and every full-channel push registered nowhere. A wake sent
    /// in one of those moments reached nobody and the turn ran on.
    #[tokio::test]
    async fn a_stop_pressed_before_the_turn_waits_on_it_is_still_heard() {
        let stop = Stop::default();
        stop.raise();
        tokio::time::timeout(std::time::Duration::from_secs(5), stop.raised())
            .await
            .expect("the turn was never told to stop");
    }

    /// And one pressed while the turn is waiting wakes it there.
    #[tokio::test]
    async fn a_stop_pressed_while_the_turn_waits_wakes_it() {
        let stop = Arc::new(Stop::default());
        let waiting = tokio::spawn({
            let stop = stop.clone();
            async move { stop.raised().await }
        });
        tokio::task::yield_now().await;
        stop.raise();
        tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
            .await
            .expect("the turn was never told to stop")
            .unwrap();
    }

    /// A scripted failure (a provider this agent does not speak) used to
    /// drop the user's line, so the next turn reached the model with no
    /// record of the question. Cancel already kept it.
    #[tokio::test]
    async fn a_failed_turn_keeps_the_users_line_so_a_retry_still_has_the_question() {
        let root = std::env::temp_dir().join(format!(
            "toad-core-rig-fail-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let history = Arc::new(AsyncMutex::new(Vec::new()));
        let turn = Turn {
            keys: HashMap::new(),
            model: "nope/none".to_string(),
            preamble: "you are Ada".to_string(),
            cwd: root.clone(),
            reach: Reach::Workspace,
            history: history.clone(),
            stop: Arc::new(Stop::default()),
            output_dir: root.join("out"),
            mcp_tools: Vec::new(),
        };
        let (sender, _receiver) = mpsc::channel(8);
        let result = turn.run(&sender, "did the crane jam?".to_string()).await;
        assert!(result.is_err(), "{result:?}");
        let held = history.lock().await;
        assert_eq!(*held, vec![Message::user("did the crane jam?")]);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn login_scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "toad-core-rig-login-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// A turn never starts a login: empty ChatGPT auth with the device flow
    /// disallowed fails locally, before any network call.
    #[tokio::test]
    async fn a_chatgpt_turn_without_a_login_refuses_instead_of_starting_one() {
        let dir = login_scratch("chatgpt");
        std::fs::write(dir.join("auth.json"), "{}").unwrap();
        let keys = HashMap::from([(
            "openai-codex".to_string(),
            ProviderAuth::Login {
                token_dir: dir.clone(),
            },
        )]);
        let err = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            complete(&keys, "openai-codex/gpt-5.6", "you are Ada", "hello"),
        )
        .await
        .expect("ChatGPT auth without a token must not wait on the network")
        .expect_err("empty auth.json must not complete");
        assert!(
            err.to_ascii_lowercase().contains("sign-in"),
            "a turn must name the missing sign-in, not start one: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Same proof for Copilot: an empty access-token and a `{}` api-key
    /// record, with the device flow disallowed, fails locally.
    #[tokio::test]
    async fn a_copilot_turn_without_a_login_refuses_instead_of_starting_one() {
        let dir = login_scratch("copilot");
        std::fs::write(dir.join("access-token"), "").unwrap();
        std::fs::write(dir.join("api-key.json"), "{}").unwrap();
        let keys = HashMap::from([(
            "github-copilot".to_string(),
            ProviderAuth::Login {
                token_dir: dir.clone(),
            },
        )]);
        let err = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            complete(&keys, "github-copilot/gpt-5-mini", "you are Ada", "hello"),
        )
        .await
        .expect("Copilot auth without a token must not wait on the network")
        .expect_err("empty Copilot files must not complete");
        assert!(
            err.to_ascii_lowercase().contains("sign-in"),
            "a turn must name the missing sign-in, not start one: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn echo_command() -> String {
        if let Ok(path) = std::env::var("CARGO_BIN_EXE_toad_mcp_echo") {
            return path;
        }
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/debug/toad-mcp-echo")
            .canonicalize()
            .expect("toad-mcp-echo should have been built with this test")
            .to_string_lossy()
            .into_owned()
    }

    /// The failing call returns the error to the model as before, and the
    /// notice that the origin is gone rides the same channel the session
    /// writes to the tape — once.
    #[tokio::test]
    async fn a_dead_mcp_transport_sends_the_went_away_notice_once() {
        use crate::mcp::{McpServer, McpTransport};
        use rig::tool::ToolSet;

        let connected = mcp::connect(
            "mcp-notice-turn",
            &[McpServer {
                id: "echo".into(),
                name: "Echo".into(),
                transport: McpTransport::Stdio {
                    command: echo_command(),
                    args: Vec::new(),
                    env: HashMap::new(),
                },
                refuse: None,
            }],
        )
        .await;
        assert!(connected.failed.is_empty(), "{:?}", connected.failed);
        let (tx, mut rx) = mpsc::channel(8);
        let tool = connected.tools[0].clone();
        let name = tool.name.clone();
        let set = ToolSet::from_dynamic_tools(vec![mcp_dynamic(tool, tx)]);
        let first = set
            .execute(
                &name,
                json!({"text": "harbour"}).to_string(),
                &mut ToolContext::new(),
            )
            .await;
        assert!(first.is_success(), "{first:?}");

        drop(connected);
        let second = set
            .execute(
                &name,
                json!({"text": "harbour"}).to_string(),
                &mut ToolContext::new(),
            )
            .await;
        assert!(!second.is_success(), "{second:?}");
        match rx.try_recv() {
            Ok(Update::Notice { level, text }) => {
                assert_eq!(level, NoticeLevel::Warn);
                assert!(text.starts_with("The Echo MCP server went away:"), "{text}");
                assert!(
                    text.contains("Its tools are gone until the teammate restarts."),
                    "{text}"
                );
            }
            other => panic!("the failing call sends the notice, not {other:?}"),
        }

        let third = set
            .execute(
                &name,
                json!({"text": "harbour"}).to_string(),
                &mut ToolContext::new(),
            )
            .await;
        assert!(!third.is_success(), "{third:?}");
        assert!(rx.try_recv().is_err(), "the notice lands once");
    }
}
