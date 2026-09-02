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
//! - **That the human pressed Stop.** The turn waits on a [`Notify`] beside
//!   the stream; waking it abandons the stream, which drops the tool future
//!   in flight, which kills that command's process group.

use super::{Driver, DriverInfo, MessageKind, Update, clip};
use crate::contract::{ConfigChoice, NoticeLevel, Persona, Reach, TokenUsage};
use crate::session::ProviderKeys;
use crate::tools::{
    EditFile, FindFiles, ListDirectory, ReadFile, RunCommand, SearchFiles, Workspace, WriteFile,
};
use async_trait::async_trait;
use futures_util::StreamExt;
use rig::agent::MultiTurnStreamItem;
use rig::agent::hook::{AgentHook, HookContext, ToolResultAction, ToolResultEvent};
use rig::message::{Message, ReasoningContent, ToolResultContent};
use rig::prelude::*;
use rig::providers::{anthropic, openai, openrouter};
use rig::streaming::{StreamedAssistantContent, StreamedUserContent};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::{Mutex as AsyncMutex, Notify, mpsc};

/// The providers Toad Agent can speak to, and the models it offers for each.
/// A model id on the wire is `provider/model`, the shape the room has always
/// stored, so a teammate's saved choice keeps meaning the same thing.
struct Provider {
    id: &'static str,
    name: &'static str,
    /// Model id and the label the picker shows for it.
    models: &'static [(&'static str, &'static str)],
}

const PROVIDERS: &[Provider] = &[
    Provider {
        id: "anthropic",
        name: "Anthropic",
        models: &[
            (anthropic::completion::CLAUDE_OPUS_4_8, "Claude Opus 4.8"),
            (
                anthropic::completion::CLAUDE_SONNET_4_6,
                "Claude Sonnet 4.6",
            ),
            (anthropic::completion::CLAUDE_HAIKU_4_5, "Claude Haiku 4.5"),
        ],
    },
    Provider {
        id: "openai",
        name: "OpenAI",
        models: &[(openai::completion::GPT_5_6, "GPT-5.6")],
    },
    Provider {
        id: "openrouter",
        name: "OpenRouter",
        models: &[
            (
                "anthropic/claude-sonnet-4.6",
                "Claude Sonnet 4.6 (OpenRouter)",
            ),
            ("openai/gpt-5.6", "GPT-5.6 (OpenRouter)"),
        ],
    },
];

/// How many rounds of tool calls one prompt may take. High enough that no
/// prompt reaches it: the human's Stop is the cap on a long-running turn, and
/// a ceiling that ends a turn on its own is a teammate that quits mid-job.
const MAX_TURNS: usize = 10_000;

/// The name a teammate on this driver answers to.
const AGENT_NAME: &str = "Toad Agent";

/// Only the tool's subject, not the whole command, goes in the title line.
const TITLE_CHARS: usize = 120;

/// The models the given provider keys unlock, as the picker lists them.
pub fn models(keys: &HashMap<String, String>) -> Vec<ConfigChoice> {
    PROVIDERS
        .iter()
        .filter(|provider| keys.contains_key(provider.id))
        .flat_map(|provider| {
            provider
                .models
                .iter()
                .map(move |(model, label)| ConfigChoice {
                    id: format!("{}/{model}", provider.id),
                    name: (*label).to_string(),
                    description: Some(provider.id.to_string()),
                    group: Some(format!("{} — API key", provider.name)),
                })
        })
        .collect()
}

fn label_of(model_id: &str) -> Option<String> {
    PROVIDERS
        .iter()
        .flat_map(|provider| {
            provider
                .models
                .iter()
                .map(move |(id, label)| (format!("{}/{id}", provider.id), *label))
        })
        .find(|(id, _)| id == model_id)
        .map(|(_, label)| label.to_string())
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
    cancel: Arc<Notify>,
}

impl InProcess {
    pub fn new(keys: Arc<dyn ProviderKeys>, preamble: String, said: Vec<Said>) -> Self {
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
            cancel: Arc::new(Notify::new()),
        }
    }

    fn info(&self, keys: &HashMap<String, String>) -> DriverInfo {
        let model = lock(&self.model).clone();
        DriverInfo {
            agent_name: AGENT_NAME.to_string(),
            models: models(keys),
            model_label: label_of(&model),
            current_model_id: model,
        }
    }
}

#[async_trait]
impl Driver for InProcess {
    async fn start(&self, persona: &Persona) -> Result<DriverInfo, String> {
        let keys = self.keys.provider_keys();
        let model = match &persona.model_id {
            Some(id) if keys.contains_key(id.split('/').next().unwrap_or("")) => id.clone(),
            _ => models(&keys)
                .first()
                .map(|model| model.id.clone())
                .ok_or_else(|| {
                    "No model key is available to Toad Agent. Add a provider key under Settings → Agents."
                        .to_string()
                })?,
        };
        *lock(&self.cwd) = PathBuf::from(&persona.cwd);
        *lock(&self.model) = model;
        Ok(self.info(&keys))
    }

    async fn prompt(&self, text: String, reach: Reach) -> mpsc::Receiver<Update> {
        let (sender, receiver) = mpsc::channel(UPDATE_DEPTH);
        let turn = Turn {
            keys: self.keys.provider_keys(),
            model: lock(&self.model).clone(),
            preamble: self.preamble.clone(),
            cwd: lock(&self.cwd).clone(),
            reach,
            history: self.history.clone(),
            cancel: self.cancel.clone(),
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
        self.cancel.notify_waiters();
    }

    async fn set_model(&self, model_id: &str) -> Result<DriverInfo, String> {
        let keys = self.keys.provider_keys();
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

/// Everything one turn needs, taken from the session at the moment it starts
/// so the turn owns it and the driver stays free to answer other calls.
struct Turn {
    keys: HashMap<String, String>,
    model: String,
    preamble: String,
    cwd: PathBuf,
    reach: Reach,
    history: Arc<AsyncMutex<Vec<Message>>>,
    cancel: Arc<Notify>,
}

impl Turn {
    async fn run(&self, sender: &mpsc::Sender<Update>, text: String) -> Result<(), String> {
        let workspace =
            Workspace::open(self.cwd.clone(), self.reach).map_err(|error| error.to_string())?;
        let outcomes = ToolOutcomes::default();
        let agent = agent_builder(&self.keys, &self.model)?
            .preamble(&self.preamble)
            .tool(ListDirectory::new(workspace.clone()))
            .tool(ReadFile::new(workspace.clone()))
            .tool(SearchFiles::new(workspace.clone()))
            .tool(FindFiles::new(workspace.clone()))
            .tool(WriteFile::new(workspace.clone()))
            .tool(EditFile::new(workspace.clone()))
            .tool(RunCommand::new(workspace))
            .add_hook(outcomes.clone())
            .default_max_turns(MAX_TURNS)
            .build();

        let mut history = self.history.lock().await;
        let mut stream = agent
            .stream_chat(text.as_str(), history.clone())
            .max_turns(MAX_TURNS)
            .await;

        let mut open: Option<OpenMessage> = None;
        let mut usage = (0i64, 0i64, 0i64);
        let mut ended = false;

        loop {
            let item = tokio::select! {
                _ = self.cancel.notified() => {
                    flush(sender, &mut open).await;
                    send(sender, Update::Turn { stop_reason: "aborted".to_string(), usage: None }).await;
                    history.push(Message::user(text));
                    return Ok(());
                }
                item = stream.next() => item,
            };
            let Some(item) = item else { break };
            match item.map_err(|error| error.to_string())? {
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
            return Err("the model ended the turn without a response".to_string());
        }
        Ok(())
    }
}

/// Which tool calls failed, by the id the stream will name them with.
///
/// Rig hands the canonical result — the one carrying the disposition — only to
/// a hook, so this is registered as one and the turn loop reads it back.
#[derive(Clone, Default)]
struct ToolOutcomes(Arc<Mutex<HashMap<String, bool>>>);

impl ToolOutcomes {
    /// Whether that call succeeded. A call the hook never saw reads as
    /// succeeded: the transcript's job is to mark the failures it knows about,
    /// not to accuse a tool of failing because Rig went quiet.
    fn take(&self, internal_call_id: &str) -> bool {
        lock(&self.0).remove(internal_call_id).unwrap_or(true)
    }
}

impl AgentHook for ToolOutcomes {
    async fn on_tool_result(
        &self,
        _context: &HookContext,
        event: ToolResultEvent<'_>,
    ) -> ToolResultAction {
        lock(&self.0).insert(
            event.internal_call_id.to_string(),
            event.raw_result.is_success(),
        );
        ToolResultAction::Keep
    }
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

/// The builder for one model on the provider whose key the desk holds.
fn agent_builder(
    keys: &HashMap<String, String>,
    model_id: &str,
) -> Result<rig::agent::AgentBuilder, String> {
    let (provider, model) = model_id
        .split_once('/')
        .ok_or_else(|| format!("{model_id} is not a provider/model id"))?;
    let key = keys
        .get(provider)
        .ok_or_else(|| format!("No key for {provider} is available on this desk."))?;
    let builder = match provider {
        "anthropic" => anthropic::Client::new(key.as_str())
            .map_err(|error| error.to_string())?
            .agent(model),
        "openai" => openai::Client::new(key.as_str())
            .map_err(|error| error.to_string())?
            .agent(model),
        "openrouter" => openrouter::Client::new(key.as_str())
            .map_err(|error| error.to_string())?
            .agent(model),
        _ => return Err(format!("{provider} is not a provider Toad Agent can use")),
    };
    Ok(builder)
}

fn lock<T>(held: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    held.lock().unwrap_or_else(PoisonError::into_inner)
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
    use serde_json::json;

    #[test]
    fn models_follow_the_keys_the_desk_holds() {
        let mut keys = HashMap::new();
        assert!(models(&keys).is_empty());
        keys.insert("anthropic".to_string(), "k".to_string());
        let listed = models(&keys);
        assert!(
            listed
                .iter()
                .all(|model| model.id.starts_with("anthropic/"))
        );
        assert_eq!(listed[0].group.as_deref(), Some("Anthropic — API key"));
        assert_eq!(label_of(&listed[0].id).as_deref(), Some("Claude Opus 4.8"));
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
        let outcomes = ToolOutcomes::default();
        assert!(outcomes.take("call-1"));
        lock(&outcomes.0).insert("call-1".to_string(), false);
        assert!(!outcomes.take("call-1"));
        assert!(outcomes.take("call-1"));
    }
}
