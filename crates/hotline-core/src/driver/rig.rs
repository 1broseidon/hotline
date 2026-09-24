//! Hotline Agent: the driver that runs the model in this process, on Rig.
//!
//! The shared loop owns each ordinary Rig model request and tool dispatch.
//! Steering replaces an inference attempt while preserving completed context;
//! Stop ends the activity. Provider construction and wire formats remain Rig's.

mod images;
mod recovery;
mod turn;
use super::failure::{Failure, Kind};
use images::Input;
#[cfg(test)]
use images::user_message;

use super::{
    CapabilityLease, Driver, DriverInfo, Escalate, MessageKind, ToolImage, Update, clip,
    with_image_placeholders,
};
use crate::contract::{
    AgentKind, Attachment, AttachmentKind, ConfigChoice, NoticeLevel, Persona, Reach,
    SessionConfig, TokenUsage, ToolSourceKind,
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
use crate::vault::Vault;
use async_trait::async_trait;
use futures_util::StreamExt;
use rig::message::{
    DocumentSourceKind, Image, ImageMediaType, Message, MimeType, ToolResultContent, UserContent,
};
use rig::prelude::*;
use rig::providers::{
    anthropic, chatgpt, copilot, deepseek, gemini, groq, mistral, openai, openrouter, xai, zai,
};
use rig::tool::{DynamicTool, ToolExecutionError, ToolOutput};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::{Mutex as AsyncMutex, Notify, mpsc};

/// How many rounds of tool calls one prompt may take. High enough that no
/// prompt reaches it: the human's Stop is the cap on a long-running turn, and
/// a ceiling that ends a turn on its own is a teammate that quits mid-job.
const MAX_TURNS: usize = 10_000;

/// The name a teammate on this driver answers to.
const AGENT_NAME: &str = "Hotline Agent";

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
    output_limit: Option<u64>,
) -> Result<String, String> {
    let agent = agent_builder(keys, model_id, None, output_limit, Reuse::Once)
        .await?
        .preamble(system)
        .build();
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

type UnconsumedInputs = Vec<(String, Vec<Attachment>)>;

/// Hotline Agent for one teammate.
pub struct InProcess {
    keys: Arc<dyn ProviderKeys>,
    preamble: String,
    /// Learned at `start`, from the persona: where relative paths start and
    /// where commands run.
    cwd: Mutex<PathBuf>,
    model: Mutex<String>,
    /// The effort the next request will send, when the current model lists it.
    effort: Mutex<Option<String>>,
    history: Arc<AsyncMutex<Vec<Message>>>,
    history_origin: Arc<Mutex<Option<recovery::Origin>>>,
    /// The stop for the turn in flight. A fresh one per prompt, so a stop
    /// nobody was running is not still standing over the next turn.
    stop: Mutex<Arc<Stop>>,
    steering: Mutex<Option<Arc<turn::Steering>>>,
    unconsumed: Arc<Mutex<UnconsumedInputs>>,
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
    /// Hotline Agent and Hotline's MCP server are two halves of one program.
    teammate: TeammateTools,
    /// Armed for the next turn only: a quiet scheduled run's `tell_person`.
    escalation: Mutex<Option<Arc<dyn Escalate>>>,
    /// Shared with every tool handle this session created.
    capability: Option<CapabilityLease>,
    /// Protected MCP OAuth registrations and tokens. None is retained for
    /// standalone test drivers, which keep OAuth servers refused.
    mcp_vault: Option<Arc<Vault>>,
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
            effort: Mutex::new(None),
            history: Arc::new(AsyncMutex::new(history)),
            history_origin: Arc::new(Mutex::new(None)),
            stop: Mutex::new(Arc::new(Stop::default())),
            steering: Mutex::new(None),
            unconsumed: Arc::new(Mutex::new(Vec::new())),
            output_dir,
            mcp_servers: Vec::new(),
            mcp_missing: Vec::new(),
            mcp: Mutex::new(None),
            teammate,
            escalation: Mutex::new(None),
            capability: None,
            mcp_vault: None,
        }
    }

    /// The MCP servers this teammate may use, selected before the driver
    /// is built so `start` can connect without asking the room again.
    pub fn with_mcp(mut self, servers: Vec<McpServer>, missing: Vec<String>) -> Self {
        self.mcp_servers = servers;
        self.mcp_missing = missing;
        self
    }

    pub(crate) fn with_capability(mut self, capability: CapabilityLease) -> Self {
        self.capability = Some(capability);
        self
    }

    pub(crate) fn with_mcp_vault(mut self, vault: Arc<Vault>) -> Self {
        self.mcp_vault = Some(vault);
        self
    }

    fn info(&self, keys: &HashMap<String, ProviderAuth>) -> DriverInfo {
        let model = lock(&self.model).clone();
        let effort = lock(&self.effort).clone();
        let metadata = self.keys.model_metadata();
        DriverInfo {
            agent_name: AGENT_NAME.to_string(),
            capabilities: crate::contract::SessionCapabilities {
                active_input: true,
                ..Default::default()
            },
            models: models::choices(
                keys,
                &self.keys.enabled_models(),
                &self.keys.account_models(),
                &metadata,
            ),
            model_label: metadata
                .get(&model)
                .map(|model| model.name.clone())
                .or_else(|| models::label_of(&model)),
            current_model_id: model.clone(),
            configs: effort_config(&model, effort.as_deref()),
            ..DriverInfo::default()
        }
    }
}

#[async_trait]
impl Driver for InProcess {
    fn current_info(&self) -> Option<DriverInfo> {
        Some(self.info(&self.keys.provider_auth()))
    }

    async fn start(&self, persona: &Persona) -> Result<DriverInfo, String> {
        if let Some(capability) = &self.capability {
            capability.check()?;
        }
        let keys = self.keys.provider_auth();
        let choices = models::choices(
            &keys,
            &self.keys.enabled_models(),
            &self.keys.account_models(),
            &self.keys.model_metadata(),
        );
        let preferred = self.keys.preferred_model();
        let model = model_for(
            persona.model_id.as_deref(),
            preferred.as_deref(),
            &choices,
        )
        .ok_or_else(|| {
            "No model key is available to Hotline Agent. Add a provider key under Settings → Agents."
                .to_string()
        })?;
        *lock(&self.cwd) = PathBuf::from(&persona.cwd);
        *lock(&self.model) = model.clone();
        // The stored effort when the model lists it, else the blank's
        // default, so a fresh teammate is never sent without a level the
        // model offers.
        *lock(&self.effort) = persona
            .effort_id
            .clone()
            .filter(|id| models::efforts(&model).iter().any(|offered| offered == id))
            .or_else(|| models::default_effort(&model));
        let connected = mcp::connect_with_capability_and_vault(
            &persona.id,
            &self.mcp_servers,
            self.capability.clone(),
            self.mcp_vault.clone(),
        )
        .await;
        if let Some(capability) = &self.capability {
            capability.check()?;
        }
        // A subagent's start is not the teammate's: the ledger says what the
        // teammate's own session was built with, and a run built without its
        // computer must not rewrite that.
        if !self.teammate.in_run() {
            publish_ledger(
                persona,
                &self.mcp_missing,
                &connected,
                self.teammate.offers_subagents(),
            );
        }
        *lock(&self.mcp) = Some(connected);
        Ok(self.info(&keys))
    }

    fn escalate_next(&self, escalation: Option<Arc<dyn Escalate>>) {
        *lock(&self.escalation) = escalation;
    }

    async fn prompt(
        &self,
        text: String,
        attachments: Vec<Attachment>,
        reach: Reach,
    ) -> mpsc::Receiver<Update> {
        let message = Input::Attachments(text, attachments);
        let (sender, receiver) = mpsc::channel(UPDATE_DEPTH);
        if let Some(capability) = &self.capability
            && let Err(error) = capability.check()
        {
            let _ = sender.try_send(Update::Notice {
                level: NoticeLevel::Error,
                text: error,
            });
            let _ = sender.try_send(Update::Turn {
                stop_reason: "revoked".to_string(),
                usage: None,
            });
            return receiver;
        }
        // The stop this turn answers to, installed before the turn is spawned
        // so a Stop pressed the instant the prompt returns still finds it.
        let stop = Arc::new(Stop::default());
        *lock(&self.stop) = stop.clone();
        let steering = Arc::new(turn::Steering::default());
        *lock(&self.steering) = Some(steering.clone());
        let mut mcp_tools: Vec<DynamicTool> = self.teammate.as_dynamic();
        if let Some(connected) = lock(&self.mcp).as_ref() {
            mcp_tools.extend(connected.tools.iter().cloned().map(mcp_dynamic));
        }
        if let Some(escalation) = lock(&self.escalation).take() {
            mcp_tools.push(tell_person(escalation));
        }
        let model = lock(&self.model).clone();
        let output_limit = self
            .keys
            .model_metadata()
            .get(&model)
            .and_then(|model| model.output_limit);
        let turn = Turn {
            keys: self.keys.provider_auth(),
            model: model.clone(),
            output_limit,
            context_limit: self
                .keys
                .model_metadata()
                .get(&model)
                .and_then(|m| m.context_limit),
            effort: lock(&self.effort).clone(),
            preamble: self.preamble.clone(),
            cwd: lock(&self.cwd).clone(),
            reach,
            history: self.history.clone(),
            stop,
            steering,
            output_dir: self.output_dir.clone(),
            mcp_tools,
            capability: self.capability.clone(),
            delegate: self.teammate.delegate(),
        };
        let history_origin = self.history_origin.clone();
        let unconsumed = self.unconsumed.clone();
        tokio::spawn(async move {
            let origin = recovery::Origin::of(&turn.model, &turn.keys);
            let changed = lock(&history_origin)
                .as_ref()
                .is_some_and(|previous| previous != &origin);
            if changed {
                let mut history = turn.history.lock().await;
                *history = recovery::fresh(&history, &turn.output_dir);
            }
            *lock(&history_origin) = Some(origin);
            let message = message.prepare(&turn.stop).await;
            let result = turn.run(&sender, message).await;
            let pending = turn.steering.close();
            if !turn.stop.raised.load(Ordering::SeqCst) {
                for input in pending {
                    if let Input::Attachments(text, attachments) = input {
                        lock(&unconsumed).push((text, attachments));
                    }
                }
            }
            if let Err(mut error) = result {
                for auth in turn.keys.values() {
                    match auth {
                        ProviderAuth::ApiKey(key)
                        | ProviderAuth::Custom {
                            api_key: Some(key), ..
                        } => error.redact_value(key),
                        _ => {}
                    }
                }
                let _ = sender
                    .send(Update::Notice {
                        level: NoticeLevel::Error,
                        text: error.notice(),
                    })
                    .await;
                send(
                    &sender,
                    Update::Turn {
                        stop_reason: "failed".into(),
                        usage: None,
                    },
                )
                .await;
            }
        });
        receiver
    }

    fn steer(&self, text: String, attachments: Vec<Attachment>) -> bool {
        lock(&self.steering)
            .as_ref()
            .is_some_and(|steering| steering.admit(Input::Attachments(text, attachments)))
    }

    fn take_unconsumed(&self) -> Vec<(String, Vec<Attachment>)> {
        std::mem::take(&mut *lock(&self.unconsumed))
    }

    fn cancel(&self) {
        if let Some(steering) = lock(&self.steering).as_ref() {
            steering.close_admission();
        }
        lock(&self.stop).raise();
    }

    fn invalidate(&self) {
        if let Some(capability) = &self.capability {
            capability.revoke();
        }
        self.cancel();
        // Dropping the live connections closes transports and kills granted
        // stdio server groups immediately. Cloned McpTool handles still carry
        // the lease and refuse calls after the room advances it.
        lock(&self.mcp).take();
    }

    async fn set_model(&self, model_id: &str) -> Result<DriverInfo, String> {
        let keys = self.keys.provider_auth();
        let provider = model_id.split('/').next().unwrap_or("");
        if !keys.contains_key(provider) {
            return Err(format!(
                "No connection for {provider} is available on this desk."
            ));
        }
        *lock(&self.model) = model_id.to_string();
        // A model switch keeps the effort only when the new model lists it;
        // otherwise the new model's blank default stands.
        {
            let mut effort = lock(&self.effort);
            let kept = effort
                .as_ref()
                .is_some_and(|current| models::efforts(model_id).iter().any(|id| id == current));
            if !kept {
                *effort = models::default_effort(model_id);
            }
        }
        Ok(self.info(&keys))
    }

    async fn set_config(&self, config_id: &str, value: &str) -> Result<DriverInfo, String> {
        if config_id != "effort" {
            return Err("This agent does not offer that setting.".to_string());
        }
        let keys = self.keys.provider_auth();
        let model = lock(&self.model).clone();
        if value.is_empty() {
            *lock(&self.effort) = models::default_effort(&model);
            return Ok(self.info(&keys));
        }
        if !models::efforts(&model).iter().any(|id| id == value) {
            let label = models::label_of(&model).unwrap_or(model);
            return Err(format!("{value} is not an effort {label} offers."));
        }
        *lock(&self.effort) = Some(value.to_string());
        Ok(self.info(&keys))
    }
}

/// An explicit model selection survives discovery and picker filtering. If
/// access disappeared, starting that model fails without routing its prompts
/// to a different model. Only a teammate with no preference takes the first.
fn model_for(
    persona_model: Option<&str>,
    preferred: Option<&str>,
    choices: &[ConfigChoice],
) -> Option<String> {
    persona_model
        .or(preferred)
        .map(str::to_string)
        .or_else(|| choices.first().map(|choice| choice.id.clone()))
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
    output_limit: Option<u64>,
    context_limit: Option<u64>,
    effort: Option<String>,
    preamble: String,
    cwd: PathBuf,
    reach: Reach,
    history: Arc<AsyncMutex<Vec<Message>>>,
    stop: Arc<Stop>,
    steering: Arc<turn::Steering>,
    output_dir: PathBuf,
    mcp_tools: Vec<DynamicTool>,
    capability: Option<CapabilityLease>,
    /// Who runs this teammate's subagents, when this session may start them.
    delegate: Option<Arc<dyn crate::session::jobs::Delegate>>,
}

impl Turn {
    async fn run(&self, sender: &mpsc::Sender<Update>, message: Message) -> Result<(), Failure> {
        // Keep admitted input even if workspace or provider construction fails.
        self.history.lock().await.push(message);
        if self.stop.raised.load(Ordering::SeqCst) {
            send(
                sender,
                Update::Turn {
                    stop_reason: "aborted".into(),
                    usage: None,
                },
            )
            .await;
            return Ok(());
        }
        if let Some(capability) = &self.capability {
            capability.check()?;
        }
        let workspace = Workspace::open_with_capability(
            self.cwd.clone(),
            self.reach,
            self.output_dir.clone(),
            self.capability.clone(),
        )
        .map_err(|error| error.to_string())?;
        let mut tools = rig::tool::ToolSet::default();
        tools.add_tool(ListDirectory::new(workspace.clone()));
        tools.add_tool(ReadFile::new(workspace.clone()));
        tools.add_tool(SearchFiles::new(workspace.clone()));
        tools.add_tool(FindFiles::new(workspace.clone()));
        tools.add_tool(WriteFile::new(workspace.clone()));
        tools.add_tool(EditFile::new(workspace.clone()));
        let shell = tools::shell_available(self.reach)
            .is_ok()
            .then(|| RunCommand::new(workspace));
        for tool in &self.mcp_tools {
            tools.add_dynamic_tool(tool.clone());
        }
        let agent = agent_builder(
            &self.keys,
            &self.model,
            self.effort.as_deref(),
            self.output_limit,
            Reuse::Rounds,
        )
        .await?
        .build();
        let (max_tokens, additional_params) = request_settings(
            &self.keys,
            &self.model,
            self.effort.as_deref(),
            self.output_limit,
        )?;
        let request = rig::completion::CompletionRequest {
            preamble: Some(self.preamble.clone()),
            tools: tools.get_tool_definitions(),
            max_tokens,
            additional_params,
            model: None,
            chat_history: Vec::new(),
            documents: Vec::new(),
            temperature: None,
            tool_choice: None,
            output_schema: None,
            record_telemetry_content: false,
        };
        turn::run(agent.model_handle(), request, &tools, self, sender, shell).await
    }
}

/// Whether a provider takes an image block inside a tool result. Anthropic
/// and OpenAI do; OpenRouter refuses the message outright ("does not support
/// images in tool results") and the turn dies, so everyone else is handed a
/// placeholder line and the picture goes to the tape only.
fn images_to_model(provider: &str) -> bool {
    matches!(provider, "anthropic" | "openai" | "openai-codex")
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

/// The JSON merged into every request for this effort, when this client
/// carries `additional_params` that far. Copilot's chat-completions route
/// is applied in [`request_settings`]: this function returns the Responses
/// body Copilot shares with OpenAI and ChatGPT.
fn effort_params(client: Client, effort: &str) -> Option<serde_json::Value> {
    match client {
        Client::CustomOpenAi => None,
        Client::Anthropic => Some(serde_json::json!({
            "thinking": {"type": "adaptive"},
            "output_config": {"effort": effort}
        })),
        Client::OpenAi | Client::ChatGpt | Client::Copilot | Client::OpenRouter | Client::XAi => {
            Some(serde_json::json!({"reasoning": {"effort": effort}}))
        }
        Client::Gemini => match effort {
            // Rig's AdditionalParameters is camelCase; ThinkingLevel is
            // snake_case, so the catalogue id is the value. Uppercasing it
            // would fail the deserialize that puts additional_params on the
            // request.
            "minimal" | "low" | "medium" | "high" => Some(serde_json::json!({
                "generationConfig": {
                    "thinkingConfig": {"thinkingLevel": effort}
                }
            })),
            _ => None,
        },
        Client::Ollama | Client::OllamaCloud => match effort {
            "low" | "medium" | "high" | "max" => Some(serde_json::json!({"think": effort})),
            _ => None,
        },
        Client::Groq | Client::DeepSeek | Client::Mistral | Client::Zai | Client::ZaiCoding => {
            Some(serde_json::json!({"reasoning_effort": effort}))
        }
    }
}

/// One Effort picker when the model lists any, otherwise none.
fn effort_config(model_id: &str, current: Option<&str>) -> Vec<SessionConfig> {
    let options = models::effort_choices(model_id);
    if options.is_empty() {
        return Vec::new();
    }
    vec![SessionConfig {
        id: "effort".to_string(),
        name: "Effort".to_string(),
        category: Some(crate::contract::SessionConfigCategory::Effort),
        current_id: current.map(str::to_string),
        options,
    }]
}

/// How often the requests a builder makes go out. A turn resends the tools,
/// the preamble and the conversation so far on every round, so a provider
/// that caches only when asked is asked. A single answer is asked once, and a
/// cache write costs more than a plain read that nothing would follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reuse {
    Rounds,
    Once,
}

/// Claude over the API caches nothing unless the request asks. The tools and
/// the preamble are marked, and the top-level breakpoint moves forward with
/// the conversation, so each round reads what the round before it wrote.
fn anthropic_model(
    client: &anthropic::Client,
    model: &str,
    reuse: Reuse,
) -> anthropic::completion::CompletionModel {
    let model = client.completion_model(model);
    match reuse {
        Reuse::Rounds => model.with_prompt_caching().with_automatic_caching(),
        Reuse::Once => model,
    }
}

/// OpenRouter hands a cache marker on the preamble through to Claude, which
/// caches nothing without one. The other models it routes cache on their own
/// or not at all, so they are left as they were.
fn openrouter_model(
    client: &openrouter::Client,
    model: &str,
    reuse: Reuse,
) -> openrouter::CompletionModel {
    let completion = client.completion_model(model);
    if reuse == Reuse::Rounds && model.starts_with("anthropic/") {
        completion.with_prompt_caching()
    } else {
        completion
    }
}

/// The builder for one model on the provider whose credential the desk holds.
async fn agent_builder(
    keys: &HashMap<String, ProviderAuth>,
    model_id: &str,
    effort: Option<&str>,
    output_limit: Option<u64>,
    reuse: Reuse,
) -> Result<rig::agent::AgentBuilder, String> {
    let (provider, model) = model_id
        .split_once('/')
        .ok_or_else(|| format!("{model_id} is not a provider/model id"))?;
    let held = keys
        .get(provider)
        .ok_or_else(|| format!("No connection for {provider} is available on this desk."))?;
    let wiring = models::wiring(provider)
        .ok_or_else(|| format!("{provider} is not a provider Hotline Agent can use"))?;
    let builder = match (wiring.client, held) {
        (_, ProviderAuth::Unavailable(error)) => return Err(error.clone()),
        (
            Client::CustomOpenAi,
            ProviderAuth::Custom {
                base_url,
                api_key,
                config,
                ..
            },
        ) => {
            let client = crate::providers::custom::client(base_url, api_key.as_deref())?;
            match config.api {
                crate::contract::OpenAiApi::Responses => client.agent(model),
                crate::contract::OpenAiApi::ChatCompletions => {
                    client.completions_api().agent(model)
                }
            }
        }
        (Client::CustomOpenAi, _) | (_, ProviderAuth::Custom { .. }) => {
            return Err("This model needs its saved custom connection.".into());
        }
        (Client::ChatGpt, ProviderAuth::Login { token_dir }) => chatgpt::Client::builder()
            .oauth()
            .auth_file(token_dir.join("auth.json"))
            .allow_device_flow(false)
            .build()
            .map_err(text)?
            .agent(model),
        (Client::Copilot, ProviderAuth::Login { token_dir }) => {
            crate::providers::copilot::ensure_endpoints(token_dir).await;
            if crate::providers::copilot::wants_responses(token_dir, model) {
                crate::providers::copilot::responses_agent(token_dir, model).await?
            } else {
                copilot::Client::builder()
                    .oauth()
                    .token_dir(token_dir)
                    .allow_device_flow(false)
                    .build()
                    .map_err(text)?
                    .agent(model)
            }
        }
        (Client::OpenRouter, ProviderAuth::StoredLogin { tokens }) => {
            rig::agent::AgentBuilder::new(openrouter_model(
                &openrouter::Client::new(&crate::providers::openrouter_key(tokens)?)
                    .map_err(text)?,
                model,
                reuse,
            ))
        }
        (Client::XAi, ProviderAuth::StoredLogin { tokens }) => {
            crate::providers::xai::client(tokens)?.agent(model)
        }
        (Client::Ollama, ProviderAuth::Local { base_url }) => {
            crate::providers::ollama_client(base_url, "")?.agent(model)
        }
        (Client::OllamaCloud, ProviderAuth::ApiKey(key)) => {
            crate::providers::ollama_client(crate::providers::OLLAMA_CLOUD_URL, key)?.agent(model)
        }
        (_, ProviderAuth::Local { .. }) | (Client::Ollama, ProviderAuth::ApiKey(_)) => {
            return Err(format!("{provider} has an incompatible connection method."));
        }
        (Client::ChatGpt | Client::Copilot, ProviderAuth::ApiKey(_)) => {
            return Err(format!("{provider} needs a sign-in, not a key."));
        }
        (_, ProviderAuth::Login { .. } | ProviderAuth::StoredLogin { .. }) => {
            return Err(format!("{provider} needs a key, not a sign-in."));
        }
        (Client::Anthropic, ProviderAuth::ApiKey(key)) => rig::agent::AgentBuilder::new(
            anthropic_model(&anthropic::Client::new(key).map_err(text)?, model, reuse),
        ),
        (Client::OpenAi, ProviderAuth::ApiKey(key)) => {
            openai::Client::new(key).map_err(text)?.agent(model)
        }
        (Client::OpenRouter, ProviderAuth::ApiKey(key)) => rig::agent::AgentBuilder::new(
            openrouter_model(&openrouter::Client::new(key).map_err(text)?, model, reuse),
        ),
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
        (Client::Zai, ProviderAuth::ApiKey(key)) => zai::Client::builder()
            .api_key(key)
            .general()
            .build()
            .map_err(text)?
            .agent(model),
        (Client::ZaiCoding, ProviderAuth::ApiKey(key)) => zai::Client::builder()
            .api_key(key)
            .coding()
            .build()
            .map_err(text)?
            .agent(model),
    };
    let (ceiling, params) = request_settings(keys, model_id, effort, output_limit)?;
    let builder = match ceiling {
        Some(ceiling) => builder.max_tokens(ceiling),
        None => builder,
    };
    Ok(match params {
        Some(params) => builder.additional_params(params),
        None => builder,
    })
}

/// Whether a Copilot model goes over `/responses` from here: the account's
/// model list offered it nowhere else. Rig's own route takes the rest.
fn copilot_responses(keys: &HashMap<String, ProviderAuth>, provider: &str, model: &str) -> bool {
    models::wiring(provider).is_some_and(|wiring| wiring.client == Client::Copilot)
        && matches!(
            keys.get(provider),
            Some(ProviderAuth::Login { token_dir })
                if crate::providers::copilot::wants_responses(token_dir, model)
        )
}

fn request_settings(
    keys: &HashMap<String, ProviderAuth>,
    model_id: &str,
    effort: Option<&str>,
    output_limit: Option<u64>,
) -> Result<(Option<u64>, Option<Value>), String> {
    let (provider, model) = model_id.split_once('/').ok_or("Model has no provider")?;
    let wiring = models::wiring(provider).ok_or("Unknown provider")?;
    // Anthropic requires a ceiling even for a model absent from discovery.
    let ceiling = output_limit
        .or_else(|| models::output_limit(model_id))
        .or_else(|| (wiring.client == Client::Anthropic).then_some(4096));
    let params = effort.and_then(|effort| {
        let chat_route = wiring.client == Client::Copilot
            && !model.to_ascii_lowercase().contains("codex")
            && !copilot_responses(keys, provider, model);
        if chat_route {
            Some(serde_json::json!({"reasoning_effort": effort}))
        } else {
            effort_params(wiring.client, effort)
        }
    });
    Ok((ceiling, params))
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn lock<T>(held: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    held.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a quiet scheduled run is told it can do: its replies are kept out of
/// the conversation, and this is the one way to put something in front of
/// the person.
const TELL_PERSON: &str = "This is a quiet scheduled run: nothing you reply \
is shown to the person or notifies them. If you find something they need to \
see or act on, call this once with what you found. When this run ends you \
will be asked, in the open, to tell them, and that reply notifies them. If \
there is nothing worth their attention, do not call it.";

/// The one-turn tool a quiet scheduled run escalates through (BRO-96).
fn tell_person(escalation: Arc<dyn Escalate>) -> DynamicTool {
    DynamicTool::new(
        "tell_person",
        TELL_PERSON,
        serde_json::json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "What you found, with what the person needs to act on it."
                }
            },
            "required": ["message"]
        }),
        move |_context, arguments| {
            let escalation = escalation.clone();
            Box::pin(async move {
                let message = arguments
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .unwrap_or_default();
                if message.is_empty() {
                    return Err(ToolExecutionError::other("message is empty"));
                }
                escalation
                    .escalate(message)
                    .map(|()| {
                        ToolOutput::text(
                            "Handed over. When this run ends you will be asked to tell the person.",
                        )
                    })
                    .map_err(ToolExecutionError::other)
            })
        },
    )
}

/// The Rig adapter for one granted MCP tool. A transport death is already
/// on the ledger; the model hears it as the call's error.
fn mcp_dynamic(tool: mcp::McpTool) -> DynamicTool {
    DynamicTool::new(
        tool.name.clone(),
        tool.description.clone(),
        tool.parameters.clone(),
        move |_context, arguments| {
            let tool = tool.clone();
            Box::pin(async move {
                match tool.call(arguments).await {
                    Ok(content) => rig_output(content),
                    // A server that went away is on the ledger and in the
                    // tool's own result; the conversation does not narrate it.
                    Err(mcp::CallError::Transport { message, .. }) => {
                        Err(ToolExecutionError::other(message))
                    }
                    Err(mcp::CallError::Tool(message)) => Err(ToolExecutionError::other(message)),
                }
            })
        },
    )
}

/// The model-visible form of an MCP result: text as before, and every image
/// as an image block so a screenshot is not flattened away.
fn rig_output(content: mcp::CallContent) -> Result<ToolOutput, ToolExecutionError> {
    if content.images.is_empty() {
        return Ok(ToolOutput::text(content.text));
    }
    let mut blocks = Vec::new();
    if !content.text.is_empty() {
        blocks.push(ToolResultContent::text(content.text));
    }
    for image in content.images {
        blocks.push(ToolResultContent::image_base64(
            image.data,
            ImageMediaType::from_mime_type(&image.mime_type),
            None,
        ));
    }
    ToolOutput::content(blocks)
}

/// The transcript's view of a Rig tool result: text joined, images carried
/// alongside so the session can write a frame.
fn result_of(content: &[ToolResultContent]) -> (String, Vec<ToolImage>) {
    let mut texts = Vec::new();
    let mut images = Vec::new();
    for item in content {
        match item {
            ToolResultContent::Text(text) => texts.push(text.text.clone()),
            ToolResultContent::Json { .. } => texts.push("[structured result]".to_string()),
            ToolResultContent::Image(image) => {
                let mime = image
                    .media_type
                    .as_ref()
                    .map(|media| media.to_mime_type())
                    .unwrap_or("image")
                    .to_string();
                match &image.data {
                    DocumentSourceKind::Base64(data) => images.push(ToolImage {
                        data: data.clone(),
                        mime_type: mime,
                    }),
                    _ => texts.push(format!("[image {mime}]")),
                }
            }
        }
    }
    let output = with_image_placeholders(&texts.join("\n"), &images);
    (output, images)
}

/// The ledger reads what this session was actually built with, not what
/// the configuration promised. Built-ins are verified because Hotline handed
/// them to the agent; MCP tools are verified when the server listed them,
/// and absent — with the error as the reason — when it did not.
pub(crate) fn publish_ledger(
    persona: &Persona,
    missing: &[String],
    connected: &mcp::Connections,
    subagents: bool,
) {
    let mut ledger = ToolLedger::new(
        persona.id.clone(),
        AgentKind::Hotline,
        persona.backend_id.clone(),
    );
    let reach = persona.reach.unwrap_or_default();
    match tools::shell_available(reach) {
        Ok(()) => {
            ledger.all(
                crate::contract::ToolState::Verified,
                ToolSourceKind::Builtin,
                "hotline",
                tools::BUILTIN,
                "Hotline handed them to the agent",
            );
        }
        Err(reason) => {
            let without_shell: Vec<&str> = tools::BUILTIN
                .iter()
                .copied()
                .filter(|name| *name != "shell")
                .collect();
            ledger.all(
                crate::contract::ToolState::Verified,
                ToolSourceKind::Builtin,
                "hotline",
                &without_shell,
                "Hotline handed them to the agent",
            );
            ledger.absent(ToolSourceKind::Builtin, "hotline", "shell", reason);
        }
    }
    if tools::shell_available(reach).is_ok() || subagents {
        ledger.all(
            crate::contract::ToolState::Verified,
            ToolSourceKind::Builtin,
            "hotline",
            crate::session::jobs::CONTROL_TOOLS,
            "Hotline supervises jobs independently of model requests",
        );
    }
    if subagents {
        ledger.all(
            crate::contract::ToolState::Verified,
            ToolSourceKind::Builtin,
            "hotline",
            &[crate::session::jobs::SUBAGENT],
            "Hotline runs subagents as this teammate's jobs",
        );
    }
    // Hotline's own tools are built here, not connected to: the agent and the
    // server are the same process, so there is nothing to observe and nothing
    // that can have gone wrong between them.
    ledger.all(
        crate::contract::ToolState::Verified,
        ToolSourceKind::Builtin,
        mcp::server::SERVER_NAME,
        &mcp::server::TOOL_NAMES,
        "Hotline's own tools, called in this process",
    );
    for tool in &connected.tools {
        ledger.verified(
            ToolSourceKind::Mcp,
            &tool.origin,
            &tool.name,
            format!("attached from the {} MCP server", tool.server_name()),
        );
    }
    for failed in &connected.failed {
        ledger.absent(
            ToolSourceKind::Mcp,
            &failed.id,
            &failed.name,
            &failed.reason,
        );
    }
    for id in missing {
        ledger.absent(ToolSourceKind::Mcp, id, id, mcp::missing_reason(id));
    }
    ledger.publish();
}

/// A tool call as a line in the transcript: the name and the one argument
/// that says what it touched.
fn describe_tool(name: &str, arguments: &Value) -> String {
    let text = |key: &&str| arguments.get(*key).and_then(Value::as_str);
    let subject = if name == crate::session::jobs::SUBAGENT {
        // A subagent is named by the label it was given, else its task's
        // first line: the task itself is a brief, not a title.
        ["title", "task"]
            .iter()
            .filter_map(text)
            .map(|value| value.lines().next().unwrap_or(value).trim())
            .find(|value| !value.is_empty())
    } else {
        ["path", "command", "pattern", "query", "expression"]
            .iter()
            .find_map(text)
    }
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

    fn choice(id: &str) -> ConfigChoice {
        ConfigChoice {
            id: id.to_string(),
            name: id.to_string(),
            description: None,
            group: None,
        }
    }

    #[test]
    fn saved_model_and_room_preference_survive_missing_or_filtered_choices() {
        let choices = [choice("other")];
        assert_eq!(
            model_for(Some("chosen"), Some("default"), &choices).as_deref(),
            Some("chosen")
        );
        assert_eq!(
            model_for(None, Some("default"), &choices).as_deref(),
            Some("default")
        );
        assert_eq!(
            model_for(Some("chosen"), Some("default"), &[]).as_deref(),
            Some("chosen")
        );
        assert_eq!(
            model_for(None, Some("default"), &[]).as_deref(),
            Some("default")
        );
        assert_eq!(model_for(None, None, &choices).as_deref(), Some("other"));
        assert_eq!(model_for(None, None, &[]), None);
    }

    #[tokio::test]
    async fn native_anthropic_requests_use_live_limits_or_a_conservative_unknown_budget() {
        use axum::{Router, body::Bytes, routing::post};
        use rig::client::CompletionClient;
        use serde_json::json;
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let app = Router::new().route("/v1/messages", post(move |body: Bytes| {
            let tx = tx.clone();
            async move {
                tx.send(serde_json::from_slice::<serde_json::Value>(&body).unwrap()).await.unwrap();
                json!({"id":"msg_test", "type":"message", "role":"assistant", "model":"brand-new-claude", "content":[{"type":"text","text":"ok"}], "stop_reason":"end_turn", "stop_sequence":null, "usage":{"input_tokens":2,"output_tokens":1}}).to_string()
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = anthropic::Client::builder()
            .api_key("test-only-key")
            .base_url(&url)
            .build()
            .unwrap();
        let keys = HashMap::from([("anthropic".into(), ProviderAuth::ApiKey("test-key".into()))]);
        let known = models::catalog().providers["anthropic"]
            .models
            .keys()
            .next()
            .unwrap();
        for (id, live_limit, expected) in [
            ("brand-new-claude", None, 4096),
            ("brand-new-claude", Some(1234), 1234),
            (known.as_str(), Some(1234), 1234),
        ] {
            let model_id = format!("anthropic/{id}");
            let agent = agent_builder(&keys, &model_id, None, live_limit, Reuse::Once)
                .await
                .unwrap()
                .build()
                .with_model(client.completion_model(id));
            assert_eq!(agent.prompt("hello").await.unwrap(), "ok");
            let request = rx.recv().await.unwrap();
            assert_eq!(request["model"], id);
            assert_eq!(request["max_tokens"], expected);
        }
        assert_eq!(models::output_limit("anthropic/brand-new-claude"), None);
        assert!(models::efforts("anthropic/brand-new-claude").is_empty());
        server.abort();
    }

    /// A provider that answers every request on `path` with `answer` and
    /// hands each request's body to the test.
    async fn provider_at(
        path: &'static str,
        answer: serde_json::Value,
    ) -> (
        String,
        tokio::sync::mpsc::Receiver<serde_json::Value>,
        tokio::task::JoinHandle<()>,
    ) {
        use axum::{Router, body::Bytes, routing::post};
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        let answer = answer.to_string();
        let app = Router::new().route(
            path,
            post(move |body: Bytes| {
                let tx = tx.clone();
                let answer = answer.clone();
                async move {
                    tx.send(serde_json::from_slice(&body).unwrap())
                        .await
                        .unwrap();
                    answer
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (url, rx, server)
    }

    fn claude_answer(usage: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"id":"msg_test", "type":"message", "role":"assistant", "model":"claude-test", "content":[{"type":"text","text":"ok"}], "stop_reason":"end_turn", "stop_sequence":null, "usage":usage})
    }

    #[tokio::test]
    async fn a_turn_asks_claude_to_cache_and_a_single_answer_does_not() {
        let (url, mut requests, server) = provider_at(
            "/v1/messages",
            claude_answer(serde_json::json!({"input_tokens":2,"output_tokens":1})),
        )
        .await;
        let client = anthropic::Client::builder()
            .api_key("test-only-key")
            .base_url(&url)
            .build()
            .unwrap();
        for (reuse, cached) in [(Reuse::Rounds, true), (Reuse::Once, false)] {
            let agent =
                rig::agent::AgentBuilder::new(anthropic_model(&client, "claude-test", reuse))
                    .preamble("You are Ada.")
                    .max_tokens(64)
                    .build();
            assert_eq!(agent.prompt("hello").await.unwrap(), "ok");
            let request = requests.recv().await.unwrap();
            assert_eq!(
                request.get("cache_control").is_some(),
                cached,
                "the breakpoint that moves with the conversation, {reuse:?}: {request}"
            );
            assert_eq!(
                request["system"][0].get("cache_control").is_some(),
                cached,
                "the preamble's own marker, {reuse:?}: {request}"
            );
        }
        server.abort();
    }

    #[tokio::test]
    async fn what_claude_reads_from_its_cache_counts_as_input() {
        let (url, _requests, server) = provider_at(
            "/v1/messages",
            claude_answer(serde_json::json!({
                "input_tokens": 5,
                "cache_read_input_tokens": 1000,
                "cache_creation_input_tokens": 200,
                "output_tokens": 7
            })),
        )
        .await;
        let client = anthropic::Client::builder()
            .api_key("test-only-key")
            .base_url(&url)
            .build()
            .unwrap();
        let response = anthropic_model(&client, "claude-test", Reuse::Rounds)
            .completion_request("hello")
            .max_tokens(64)
            .send()
            .await
            .unwrap();
        let usage = turn::every_input_token(response.usage, "anthropic/claude-test");
        assert_eq!(
            (
                usage.input_tokens,
                usage.cached_input_tokens,
                usage.cache_creation_input_tokens
            ),
            (1205, 1000, 200)
        );
        assert_eq!(
            turn::every_input_token(response.usage, "openai/gpt-test").input_tokens,
            5,
            "a provider that counts cached input inside its input is taken as it reports"
        );
        server.abort();
    }

    #[tokio::test]
    async fn claude_on_openrouter_is_asked_to_cache_and_other_models_are_left_alone() {
        let (url, mut requests, server) = provider_at(
            "/chat/completions",
            serde_json::json!({
                "id":"gen-test","object":"chat.completion","created":1,"model":"routed",
                "choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],
                "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}
            }),
        )
        .await;
        let client = openrouter::Client::builder()
            .api_key("test-only-key")
            .base_url(&url)
            .build()
            .unwrap();
        for (model, reuse, cached) in [
            ("anthropic/claude-test", Reuse::Rounds, true),
            ("anthropic/claude-test", Reuse::Once, false),
            ("openai/gpt-test", Reuse::Rounds, false),
        ] {
            let agent = rig::agent::AgentBuilder::new(openrouter_model(&client, model, reuse))
                .preamble("You are Ada.")
                .build();
            assert_eq!(agent.prompt("hello").await.unwrap(), "ok");
            let request = requests.recv().await.unwrap();
            let preamble = &request["messages"][0];
            assert_eq!(preamble["role"], "system", "{request}");
            assert_eq!(
                preamble["content"][0].get("cache_control").is_some(),
                cached,
                "{model} {reuse:?}: {request}"
            );
        }
        server.abort();
    }

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
        assert_eq!(
            user_message("look", std::slice::from_ref(&attachment)),
            Message::user("look\n\nAttached files:\n/tmp/note.txt")
        );
    }

    /// A picture reaches the model as pixels beside the path, whoever the
    /// provider is; one too big to send, or missing, is named as such and
    /// stays a path.
    #[test]
    fn an_attached_image_reaches_the_model_as_pixels_and_as_a_path() {
        use crate::contract::AttachmentKind;
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("shot.png");
        image::RgbImage::new(8, 8).save(&png).unwrap();
        let image = |path: &Path| Attachment {
            kind: AttachmentKind::Image,
            name: "shot.png".to_string(),
            path: path.to_string_lossy().into_owned(),
            mime_type: Some("image/png".to_string()),
            size: None,
        };
        let Message::User { content } = user_message("see", &[image(&png)]) else {
            panic!("a user message");
        };
        assert_eq!(content.len(), 2);
        assert_eq!(
            content[0],
            UserContent::text(format!("see\n\nAttached files:\n{}", png.display()))
        );
        let UserContent::Image(sent) = &content[1] else {
            panic!("an image block");
        };
        assert_eq!(sent.media_type, Some(ImageMediaType::JPEG));

        // A photo-sized picture arrives as a JPEG no wider than the edge cap.
        let wide = dir.path().join("wide.png");
        image::RgbImage::from_fn(3000, 300, |x, _| image::Rgb([(x % 256) as u8, 40, 200]))
            .save(&wide)
            .unwrap();
        let Message::User { content } = user_message("see", &[image(&wide)]) else {
            panic!("a user message");
        };
        let UserContent::Image(sent) = &content[1] else {
            panic!("an image block");
        };
        assert_eq!(sent.media_type, Some(ImageMediaType::JPEG));
        let DocumentSourceKind::Base64(data) = &sent.data else {
            panic!("inline bytes");
        };
        use base64::{Engine, prelude::BASE64_STANDARD};
        let shrunk = image::load_from_memory(&BASE64_STANDARD.decode(data).unwrap()).unwrap();
        assert_eq!((shrunk.width(), shrunk.height()), (2000, 200));

        let missing = image(&dir.path().join("gone.png"));
        let Message::User { content } = user_message("see", std::slice::from_ref(&missing)) else {
            panic!("a user message");
        };
        assert_eq!(content.len(), 1);
        assert_eq!(
            content[0],
            UserContent::text(format!(
                "see\n\nAttached files:\n{} (not readable; on disk only)",
                missing.path
            ))
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

    #[test]
    fn only_providers_that_take_a_picture_in_a_tool_result_get_one() {
        assert!(images_to_model("anthropic"));
        assert!(images_to_model("openai-codex"));
        assert!(!images_to_model("openrouter"));
        assert!(!images_to_model("ollama"));
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
            "hotline-core-tool-output-{}-{nonce}",
            std::process::id()
        ));
        let workspace_dir = root.join("workspace");
        let output_dir = root.join("tool-output");
        std::fs::create_dir_all(&workspace_dir).unwrap();
        let body = "x".repeat(MODEL_TOOL_OUTPUT_BYTES + 64);
        std::fs::write(workspace_dir.join("big.txt"), &body).unwrap();
        let workspace = Workspace::open(workspace_dir, Reach::Machine, output_dir.clone()).unwrap();
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
            "hotline-core-rig-fail-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let history = Arc::new(AsyncMutex::new(Vec::new()));
        let turn = Turn {
            output_limit: None,
            context_limit: None,
            keys: HashMap::new(),
            model: "nope/none".to_string(),
            effort: None,
            preamble: "you are Ada".to_string(),
            cwd: root.clone(),
            reach: Reach::Workspace,
            history: history.clone(),
            stop: Arc::new(Stop::default()),
            steering: Arc::new(turn::Steering::default()),
            output_dir: root.join("out"),
            mcp_tools: Vec::new(),
            capability: None,
            delegate: None,
        };
        let (sender, _receiver) = mpsc::channel(8);
        let result = turn.run(&sender, Message::user("did the crane jam?")).await;
        assert!(result.is_err(), "{result:?}");
        let held = history.lock().await;
        assert_eq!(*held, vec![Message::user("did the crane jam?")]);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn login_scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "hotline-core-rig-login-{name}-{}-{}",
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
            complete(&keys, "openai-codex/gpt-5.6", "you are Ada", "hello", None),
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
            complete(
                &keys,
                "github-copilot/gpt-5-mini",
                "you are Ada",
                "hello",
                None,
            ),
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

    /// A Copilot model the account offers only on `/responses` is driven
    /// there, signed as Copilot expects, with the effort in the Responses
    /// shape; one it never named stays on Rig's `/chat/completions` route,
    /// on the same stored token.
    #[tokio::test]
    async fn a_responses_only_copilot_model_takes_the_responses_route() {
        use axum::{
            Router,
            body::Bytes,
            http::HeaderMap,
            routing::{get, post},
        };
        use rig::completion::CompletionModel;
        use serde_json::json;
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let responses = tx.clone();
        let listed = tx.clone();
        let app = Router::new()
            .route(
                "/models",
                get(move |headers: HeaderMap| {
                    let tx = listed.clone();
                    async move {
                        tx.send(("/models", headers, serde_json::Value::Null)).await.unwrap();
                        json!({"data": [
                            {"id":"grok-4.6","supported_endpoints":["/responses"]},
                            {"id":"claude-sonnet-5","supported_endpoints":["/chat/completions","/responses"]}
                        ]})
                        .to_string()
                    }
                }),
            )
            .route(
                "/responses",
                post(move |headers: HeaderMap, body: Bytes| {
                    let tx = responses.clone();
                    async move {
                        tx.send(("/responses", headers, serde_json::from_slice::<serde_json::Value>(&body).unwrap())).await.unwrap();
                        json!({
                            "id":"resp_1","object":"response","created_at":1,"status":"completed","model":"grok-4.6",
                            "output":[{"type":"message","id":"msg_1","role":"assistant","status":"completed",
                                "content":[{"type":"output_text","text":"over responses"}]}],
                            "usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}
                        }).to_string()
                    }
                }),
            )
            .route(
                "/chat/completions",
                post(move |headers: HeaderMap, body: Bytes| {
                    let tx = tx.clone();
                    async move {
                        tx.send(("/chat/completions", headers, serde_json::from_slice::<serde_json::Value>(&body).unwrap())).await.unwrap();
                        json!({
                            "id":"chatcmpl_1","object":"chat.completion","created":1,"model":"claude-sonnet-5",
                            "choices":[{"index":0,"message":{"role":"assistant","content":"over chat"},"finish_reason":"stop"}],
                            "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}
                        }).to_string()
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let dir = crate::providers::copilot::tests::seeded_login("route", &url);
        crate::providers::copilot::tests::record_endpoints(
            &dir,
            json!({"grok-4.6": ["/responses"], "claude-sonnet-5": ["/chat/completions"]}),
        );
        let keys = HashMap::from([(
            "github-copilot".to_string(),
            ProviderAuth::Login {
                token_dir: dir.clone(),
            },
        )]);

        let answer = complete(
            &keys,
            "github-copilot/grok-4.6",
            "you are Ada",
            "hello",
            None,
        )
        .await
        .unwrap();
        assert_eq!(answer, "over responses");
        let (path, headers, body) = rx.recv().await.unwrap();
        assert_eq!(path, "/responses");
        assert_eq!(body["model"], "grok-4.6");
        assert_eq!(headers["authorization"], "Bearer copilot-test-token");
        assert_eq!(headers["copilot-integration-id"], "vscode-chat");
        assert_eq!(headers["x-initiator"], "user");
        assert!(
            body["input"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["role"] == "system"),
            "the preamble rides as an input message, as on Rig's Copilot route: {body}"
        );

        let answer = complete(
            &keys,
            "github-copilot/claude-sonnet-5",
            "you are Ada",
            "hello",
            None,
        )
        .await
        .unwrap();
        assert_eq!(answer, "over chat");
        let (path, _, body) = rx.recv().await.unwrap();
        assert_eq!(path, "/chat/completions");
        assert_eq!(body["model"], "claude-sonnet-5");

        let (_, params) =
            request_settings(&keys, "github-copilot/grok-4.6", Some("high"), None).unwrap();
        assert_eq!(params, Some(json!({"reasoning": {"effort": "high"}})));
        let (_, params) =
            request_settings(&keys, "github-copilot/claude-sonnet-5", Some("high"), None).unwrap();
        assert_eq!(params, Some(json!({"reasoning_effort": "high"})));
        let (_, params) =
            request_settings(&keys, "github-copilot/gpt-5.3-codex", Some("high"), None).unwrap();
        assert_eq!(params, Some(json!({"reasoning": {"effort": "high"}})));

        // Tools on the Responses route are strict, as on Rig's own route.
        let agent = agent_builder(&keys, "github-copilot/grok-4.6", None, None, Reuse::Rounds)
            .await
            .unwrap()
            .build();
        let request = rig::completion::CompletionRequest {
            preamble: None,
            tools: vec![rig::completion::ToolDefinition {
                name: "read_file".into(),
                description: "Read a file".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"]
                }),
            }],
            max_tokens: None,
            additional_params: None,
            model: None,
            chat_history: vec![rig::message::Message::user("read it")],
            documents: Vec::new(),
            temperature: None,
            tool_choice: None,
            output_schema: None,
            record_telemetry_content: false,
        };
        agent.model_handle().completion(request).await.unwrap();
        let (path, _, body) = rx.recv().await.unwrap();
        assert_eq!(path, "/responses");
        assert_eq!(body["tools"][0]["name"], "read_file");
        assert_eq!(body["tools"][0]["strict"], true, "{body}");
        assert_eq!(
            body["tools"][0]["parameters"]["additionalProperties"],
            false
        );

        // A login from before endpoints were recorded fetches them on its
        // first turn and takes the Responses route without a Refresh.
        let older = crate::providers::copilot::tests::seeded_login("route-older", &url);
        let older_keys = HashMap::from([(
            "github-copilot".to_string(),
            ProviderAuth::Login {
                token_dir: older.clone(),
            },
        )]);
        let answer = complete(
            &older_keys,
            "github-copilot/grok-4.6",
            "you are Ada",
            "hello",
            None,
        )
        .await
        .unwrap();
        assert_eq!(answer, "over responses");
        assert_eq!(rx.recv().await.unwrap().0, "/models");
        assert_eq!(rx.recv().await.unwrap().0, "/responses");
        server.abort();
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&older);
    }

    fn echo_command() -> String {
        if let Ok(path) = std::env::var("CARGO_BIN_EXE_hotline_mcp_echo") {
            return path;
        }
        // A unit test is not told where its bins are, only an integration
        // test is; but it knows where it is itself, and the bins are one
        // directory up from `deps`, wherever the target directory lives.
        std::env::current_exe()
            .ok()
            .and_then(|exe| {
                let name = format!("hotline-mcp-echo{}", std::env::consts::EXE_SUFFIX);
                Some(exe.parent()?.parent()?.join(name))
            })
            .filter(|path| path.exists())
            .expect("hotline-mcp-echo should have been built with this test")
            .to_string_lossy()
            .into_owned()
    }

    /// The failing call returns the error to the model as before, and says
    /// nothing to the conversation: the ledger already carries it.
    #[tokio::test]
    async fn a_dead_mcp_transport_fails_the_call_and_nothing_else() {
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
        let tool = connected.tools[0].clone();
        let name = tool.name.clone();
        let set = ToolSet::from_dynamic_tools(vec![mcp_dynamic(tool)]);
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
    }

    /// A CallToolResult with text and an image becomes text plus an image
    /// in the Rig output, so the model sees the picture.
    #[test]
    fn an_mcp_image_block_reaches_the_rig_output() {
        use crate::mcp::{CallContent, CallImage};
        use rig::message::{DocumentSourceKind, ImageMediaType, ToolResultContent};

        let output = rig_output(CallContent {
            text: "the tree".into(),
            images: vec![CallImage {
                data: "AAAA".into(),
                mime_type: "image/png".into(),
            }],
        })
        .expect("mixed content is a valid tool output");
        let content = output.as_content();
        assert!(
            matches!(
                content,
                [
                    ToolResultContent::Text(text),
                    ToolResultContent::Image(image)
                ] if text.text == "the tree"
                    && image.media_type == Some(ImageMediaType::PNG)
                    && matches!(&image.data, DocumentSourceKind::Base64(data) if data == "AAAA")
            ),
            "{content:?}"
        );
    }

    #[test]
    fn effort_params_per_client() {
        assert_eq!(
            effort_params(Client::OllamaCloud, "max"),
            Some(serde_json::json!({"think": "max"}))
        );
        assert_eq!(
            effort_params(Client::Ollama, "medium"),
            Some(serde_json::json!({"think": "medium"}))
        );
        assert_eq!(effort_params(Client::OllamaCloud, "xhigh"), None);

        assert_eq!(
            effort_params(Client::Anthropic, "high"),
            Some(json!({
                "thinking": {"type": "adaptive"},
                "output_config": {"effort": "high"}
            }))
        );
        let reasoning = json!({"reasoning": {"effort": "high"}});
        assert_eq!(
            effort_params(Client::OpenAi, "high"),
            Some(reasoning.clone())
        );
        assert_eq!(
            effort_params(Client::ChatGpt, "high"),
            Some(reasoning.clone())
        );
        assert_eq!(
            effort_params(Client::Copilot, "high"),
            Some(reasoning.clone())
        );
        assert_eq!(
            effort_params(Client::OpenRouter, "high"),
            Some(reasoning.clone())
        );
        assert_eq!(effort_params(Client::XAi, "high"), Some(reasoning));
        assert_eq!(
            effort_params(Client::Gemini, "high"),
            Some(json!({
                "generationConfig": {"thinkingConfig": {"thinkingLevel": "high"}}
            }))
        );
        assert_eq!(effort_params(Client::Gemini, "xhigh"), None);
        let chat = json!({"reasoning_effort": "high"});
        assert_eq!(effort_params(Client::Groq, "high"), Some(chat.clone()));
        assert_eq!(effort_params(Client::DeepSeek, "high"), Some(chat.clone()));
        assert_eq!(effort_params(Client::Mistral, "high"), Some(chat));
    }
}
