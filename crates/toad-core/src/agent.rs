//! Toad Agent, on Rig.
//!
//! A session is one teammate's conversation with a model: its history, the
//! model it speaks to, and the tools it holds over its working directory.
//! The main process drives sessions over the wire (`rpc`) and hears about
//! everything they do as pushes — transcript events, streaming deltas,
//! session state — in the same vocabulary the main's other agent kind uses,
//! so nothing downstream knows which runtime answered.
//!
//! A turn is a Rig multi-turn stream: text and reasoning deltas, tool calls
//! the agent makes, their results, and a final response. Each of those
//! becomes the transcript event the main would have written itself. Nothing
//! asks permission: a teammate's one policy is how far its tools reach, and
//! the main says which with every prompt.
//!
//! The main's socket carries requests both ways. What only the main knows —
//! today the provider keys in its credential store — the runtime asks for
//! when it needs it (`ask_main`), rather than having it sent along with
//! every call: a key added or rotated on the desk is in force on the next
//! turn, and no request has to carry a secret it is not about.

use crate::contract::Reach;
use crate::log::{Log, StreamId};
use crate::tools::{
    EditFile, FindFiles, ListDirectory, ReadFile, RunCommand, SearchFiles, Workspace, WriteFile,
};
use futures_util::StreamExt;
use rig::agent::MultiTurnStreamItem;
use rig::message::{Message, ReasoningContent, ToolResultContent};
use rig::prelude::*;
use rig::providers::{anthropic, openai, openrouter};
use rig::streaming::{StreamedAssistantContent, StreamedUserContent};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex as AsyncMutex, Notify, mpsc, oneshot};

/// The providers Toad Agent can speak to, and the models it offers for each.
/// A model id on the wire is `provider/model`, the shape the main has always
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

const MAX_TURNS: usize = 200;
const TOOL_OUTPUT_CHARS: usize = 4_000;

pub fn providers() -> Vec<Value> {
    PROVIDERS
        .iter()
        .map(|provider| json!({ "id": provider.id, "name": provider.name }))
        .collect()
}

/// The models the given provider keys unlock, as the picker lists them.
pub fn models(keys: &HashMap<String, String>) -> Vec<Value> {
    PROVIDERS
        .iter()
        .filter(|provider| keys.contains_key(provider.id))
        .flat_map(|provider| {
            provider.models.iter().map(move |(model, label)| {
                json!({
                    "id": format!("{}/{model}", provider.id),
                    "name": label,
                    "description": provider.id,
                    "group": format!("{} — API key", provider.name),
                })
            })
        })
        .collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// What the main told us when it started the session.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct SessionStart {
    #[serde(rename = "personaId")]
    pub persona_id: String,
    pub name: String,
    pub cwd: PathBuf,
    pub preamble: String,
    #[serde(rename = "modelId")]
    pub model_id: Option<String>,
}

struct Session {
    start: SessionStart,
    model: Mutex<String>,
    /// Provider id to API key, as the main last answered. Refreshed at the
    /// start of every turn; read from here in between, so the picker can be
    /// drawn without a round trip.
    keys: Mutex<HashMap<String, String>>,
    history: AsyncMutex<Vec<Message>>,
    queue: Mutex<VecDeque<String>>,
    running: Mutex<bool>,
    cancel: Notify,
    /// How far the tools reach this turn; the main sends it with each prompt.
    reach: Mutex<Reach>,
}

impl Session {
    fn keys(&self) -> HashMap<String, String> {
        self.keys
            .lock()
            .map(|keys| keys.clone())
            .unwrap_or_default()
    }

    fn info(&self, state: &str, error: Option<String>) -> Value {
        let model = self
            .model
            .lock()
            .map(|model| model.clone())
            .unwrap_or_default();
        let label = PROVIDERS
            .iter()
            .flat_map(|provider| {
                provider
                    .models
                    .iter()
                    .map(move |(id, label)| (format!("{}/{id}", provider.id), *label))
            })
            .find(|(id, _)| *id == model)
            .map(|(_, label)| label.to_string());
        json!({
            "personaId": self.start.persona_id,
            "state": state,
            "agentName": "Toad Agent",
            "contextRestored": false,
            "models": models(&self.keys()),
            "currentModelId": model,
            "modelLabel": label,
            "modes": [],
            "configs": [],
            "slashCommands": [],
            "capabilities": {
                "loadSession": false,
                "resume": false,
                "fork": false,
                "mcpHttp": false,
                "image": false,
            },
            "error": error,
        })
    }
}

/// Every session this shell runs, and the one socket their news goes out on
/// and the main's answers come back on.
pub struct Runtime {
    log: Log,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    main: Mutex<Option<mpsc::UnboundedSender<String>>>,
    /// Questions asked of the main and not yet answered, by the id this side
    /// gave them. The main numbers its own requests; a frame with a `method`
    /// is one of those, a frame with an `ok` is an answer to one of these.
    asked: Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>,
    next_ask: AtomicU64,
}

impl Runtime {
    pub fn new(log: Log) -> Self {
        Self {
            log,
            sessions: Mutex::new(HashMap::new()),
            main: Mutex::new(None),
            asked: Mutex::new(HashMap::new()),
            next_ask: AtomicU64::new(1),
        }
    }

    pub fn root(&self) -> &Path {
        self.log.root()
    }

    /// The main's socket is where pushes and questions go; there is one main.
    pub fn attach_main(&self, sender: mpsc::UnboundedSender<String>) {
        if let Ok(mut slot) = self.main.lock() {
            *slot = Some(sender);
        }
    }

    /// A main that is gone answers nothing: every question still waiting is
    /// failed here, by dropping the side that would have answered it.
    pub fn detach_main(&self) {
        if let Ok(mut slot) = self.main.lock() {
            *slot = None;
        }
        if let Ok(mut asked) = self.asked.lock() {
            asked.clear();
        }
    }

    fn send_to_main(&self, frame: String) -> Result<(), String> {
        let slot = self.main.lock().map_err(|_| "runtime poisoned")?;
        let sender = slot
            .as_ref()
            .ok_or_else(|| "no main process is attached".to_string())?;
        sender
            .send(frame)
            .map_err(|_| "the main's socket is closed".to_string())
    }

    fn push(&self, persona_id: &str, payload: Value) {
        let mut payload = payload;
        payload["personaId"] = Value::String(persona_id.to_string());
        let _ = self.send_to_main(json!({ "push": "agent", "payload": payload }).to_string());
    }

    /// Asks the main something only it knows, in the same `{id, method,
    /// params}` frame it asks this side with, and waits for the answer.
    pub async fn ask_main(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_ask.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = oneshot::channel();
        if let Ok(mut asked) = self.asked.lock() {
            asked.insert(id, answer);
        }
        if let Err(error) =
            self.send_to_main(json!({ "id": id, "method": method, "params": params }).to_string())
        {
            if let Ok(mut asked) = self.asked.lock() {
                asked.remove(&id);
            }
            return Err(error);
        }
        answered
            .await
            .map_err(|_| "the main went away before answering".to_string())?
    }

    /// An answer frame from the main, matched to the question it answers.
    pub fn settle(&self, frame: &Value) {
        let Some(id) = frame.get("id").and_then(Value::as_u64) else {
            return;
        };
        let Some(answer) = self
            .asked
            .lock()
            .ok()
            .and_then(|mut asked| asked.remove(&id))
        else {
            return;
        };
        let outcome = if frame.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(frame.get("result").cloned().unwrap_or(Value::Null))
        } else {
            Err(frame
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("the main refused")
                .to_string())
        };
        let _ = answer.send(outcome);
    }

    /// Provider id to API key, for every provider this desk holds a key for.
    /// The keys live in the main's credential store, and are asked for fresh.
    async fn provider_keys(&self) -> Result<HashMap<String, String>, String> {
        let answer = self.ask_main("credentialProviderKeys", json!({})).await?;
        Ok(answer
            .as_object()
            .map(|keys| {
                keys.iter()
                    .filter_map(|(id, key)| key.as_str().map(|key| (id.clone(), key.to_string())))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// The models this desk's keys unlock, as the picker lists them.
    pub async fn models_for_desk(&self) -> Result<Vec<Value>, String> {
        Ok(models(&self.provider_keys().await?))
    }

    fn append(&self, persona_id: &str, event: Value) {
        self.push(persona_id, json!({ "kind": "append", "event": event }));
    }

    fn update(&self, persona_id: &str, event: Value) {
        self.push(persona_id, json!({ "kind": "update", "event": event }));
    }

    fn delta(&self, persona_id: &str, message_id: &str, kind: &str, text: &str) {
        self.push(
            persona_id,
            json!({ "kind": "delta", "messageId": message_id, "deltaKind": kind, "text": text }),
        );
    }

    fn info(&self, session: &Session, state: &str, error: Option<String>) {
        self.push(
            &session.start.persona_id,
            json!({ "kind": "info", "info": session.info(state, error) }),
        );
    }

    fn notice(&self, persona_id: &str, level: &str, text: String) {
        self.append(
            persona_id,
            json!({ "kind": "notice", "id": new_id(), "ts": now_ms(), "level": level, "text": text }),
        );
    }

    fn session(&self, persona_id: &str) -> Result<Arc<Session>, String> {
        self.sessions
            .lock()
            .ok()
            .and_then(|sessions| sessions.get(persona_id).cloned())
            .ok_or_else(|| "That teammate is not running.".to_string())
    }

    /// Starts a session, seeding its memory from the tape so the agent
    /// remembers the conversation the way the transcript tells it.
    pub async fn start(&self, start: SessionStart) -> Result<Value, String> {
        let keys = self.provider_keys().await?;
        let model = match &start.model_id {
            Some(id) if keys.contains_key(id.split('/').next().unwrap_or("")) => id.clone(),
            _ => models(&keys)
                .first()
                .and_then(|model| model["id"].as_str().map(str::to_string))
                .ok_or_else(|| {
                    "No model key is available to Toad Agent. Add a provider key under Settings → Agents."
                        .to_string()
                })?,
        };
        let history = self
            .log
            .load(&StreamId::Tape(start.persona_id.clone()))
            .into_iter()
            .filter_map(|event| {
                let text = event.get("text")?.as_str()?.to_string();
                match event.get("kind")?.as_str()? {
                    "user" => Some(Message::user(text)),
                    "agent" => Some(Message::assistant(text)),
                    _ => None,
                }
            })
            .collect();
        let session = Arc::new(Session {
            start,
            model: Mutex::new(model),
            keys: Mutex::new(keys),
            history: AsyncMutex::new(history),
            queue: Mutex::new(VecDeque::new()),
            running: Mutex::new(false),
            cancel: Notify::new(),
            reach: Mutex::new(Reach::Workspace),
        });
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.insert(session.start.persona_id.clone(), session.clone());
        }
        Ok(session.info("ready", None))
    }

    pub fn stop(&self, persona_id: &str) -> Result<(), String> {
        let session = self
            .sessions
            .lock()
            .ok()
            .and_then(|mut sessions| sessions.remove(persona_id));
        if let Some(session) = session {
            session.cancel.notify_waiters();
        }
        Ok(())
    }

    pub async fn set_model(&self, persona_id: &str, model_id: &str) -> Result<Value, String> {
        let session = self.session(persona_id)?;
        let keys = self.provider_keys().await?;
        let provider = model_id.split('/').next().unwrap_or("");
        if !keys.contains_key(provider) {
            return Err(format!("No key for {provider} is available on this desk."));
        }
        if let Ok(mut held) = session.keys.lock() {
            *held = keys;
        }
        if let Ok(mut model) = session.model.lock() {
            *model = model_id.to_string();
        }
        Ok(session.info("ready", None))
    }

    /// Hands the agent a message. Queued behind a running turn, run at once
    /// otherwise; the answer arrives as pushes.
    pub fn prompt(
        self: &Arc<Self>,
        persona_id: &str,
        text: String,
        reach: Reach,
    ) -> Result<(), String> {
        let session = self.session(persona_id)?;
        if let Ok(mut current) = session.reach.lock() {
            *current = reach;
        }
        let already_running = {
            let mut running = session.running.lock().map_err(|_| "session poisoned")?;
            if *running {
                true
            } else {
                *running = true;
                false
            }
        };
        if already_running {
            if let Ok(mut queue) = session.queue.lock() {
                queue.push_back(text);
            }
            return Ok(());
        }
        let runtime = self.clone();
        tokio::spawn(async move { runtime.run_turns(session, text).await });
        Ok(())
    }

    pub fn cancel(&self, persona_id: &str) -> Result<(), String> {
        let session = self.session(persona_id)?;
        if let Ok(mut queue) = session.queue.lock() {
            queue.clear();
        }
        session.cancel.notify_waiters();
        Ok(())
    }

    async fn run_turns(self: Arc<Self>, session: Arc<Session>, first: String) {
        let mut next = Some(first);
        while let Some(text) = next.take() {
            self.info(&session, "thinking", None);
            if let Err(error) = self.run_turn(&session, text).await {
                self.notice(
                    &session.start.persona_id,
                    "error",
                    format!("Turn failed: {error}"),
                );
            }
            next = session
                .queue
                .lock()
                .ok()
                .and_then(|mut queue| queue.pop_front());
        }
        if let Ok(mut running) = session.running.lock() {
            *running = false;
        }
        self.info(&session, "ready", None);
    }

    async fn run_turn(
        self: &Arc<Self>,
        session: &Arc<Session>,
        text: String,
    ) -> Result<(), String> {
        let model = session
            .model
            .lock()
            .map(|model| model.clone())
            .unwrap_or_default();
        let reach = session.reach.lock().map(|reach| *reach).unwrap_or_default();
        let keys = self.provider_keys().await?;
        if let Ok(mut held) = session.keys.lock() {
            *held = keys.clone();
        }
        let workspace =
            Workspace::open(session.start.cwd.clone(), reach).map_err(|error| error.to_string())?;
        let agent = agent_builder(&keys, &model)?
            .preamble(&session.start.preamble)
            .tool(ListDirectory::new(workspace.clone()))
            .tool(ReadFile::new(workspace.clone()))
            .tool(SearchFiles::new(workspace.clone()))
            .tool(FindFiles::new(workspace.clone()))
            .tool(WriteFile::new(workspace.clone()))
            .tool(EditFile::new(workspace.clone()))
            .tool(RunCommand::new(workspace))
            .default_max_turns(MAX_TURNS)
            .build();

        let mut history = session.history.lock().await;
        let mut stream = agent
            .stream_chat(text.as_str(), history.clone())
            .max_turns(MAX_TURNS)
            .await;

        let persona_id = session.start.persona_id.as_str();
        let mut open: Option<OpenMessage> = None;
        let mut tools: HashMap<String, Value> = HashMap::new();
        let mut usage = (0u64, 0u64, 0u64);
        let mut ended = false;

        loop {
            let item = tokio::select! {
                _ = session.cancel.notified() => {
                    self.flush(persona_id, &mut open);
                    for (_, mut event) in tools.drain() {
                        if matches!(event["status"].as_str(), Some("in_progress" | "pending")) {
                            event["status"] = json!("failed");
                            self.update(persona_id, event);
                        }
                    }
                    self.append(persona_id, json!({ "kind": "turn", "id": new_id(), "ts": now_ms(), "stopReason": "aborted" }));
                    history.push(Message::user(text));
                    return Ok(());
                }
                item = stream.next() => item,
            };
            let Some(item) = item else { break };
            match item.map_err(|error| error.to_string())? {
                MultiTurnStreamItem::StreamAssistantItem(content) => match content {
                    StreamedAssistantContent::Text(chunk) => {
                        self.chunk(persona_id, &mut open, "agent", &chunk.text);
                    }
                    StreamedAssistantContent::ReasoningDelta { reasoning, .. } => {
                        self.chunk(persona_id, &mut open, "thought", &reasoning);
                    }
                    StreamedAssistantContent::Reasoning { reasoning, .. } => {
                        for part in reasoning.content {
                            if let ReasoningContent::Text { text, .. } = part {
                                self.chunk(persona_id, &mut open, "thought", &text);
                            }
                        }
                    }
                    StreamedAssistantContent::ToolCall {
                        tool_call,
                        internal_call_id,
                    } => {
                        self.flush(persona_id, &mut open);
                        let event = json!({
                            "kind": "tool",
                            "id": format!("tool:{internal_call_id}"),
                            "ts": now_ms(),
                            "toolCallId": internal_call_id,
                            "title": describe_tool(&tool_call.function.name, &tool_call.function.arguments),
                            "toolKind": tool_call.function.name,
                            "status": "in_progress",
                        });
                        tools.insert(internal_call_id, event.clone());
                        self.append(persona_id, event);
                    }
                    _ => {}
                },
                MultiTurnStreamItem::StreamUserItem(StreamedUserContent::ToolResult {
                    tool_result,
                    internal_call_id,
                }) => {
                    if let Some(mut event) = tools.remove(&internal_call_id) {
                        let text = tool_result
                            .content
                            .iter()
                            .map(|item| match item {
                                ToolResultContent::Text(text) => text.text.clone(),
                                ToolResultContent::Json { .. } => "[structured result]".to_string(),
                                _ => "[non-text result]".to_string(),
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        event["status"] = json!("completed");
                        event["output"] =
                            json!([{ "type": "text", "text": clip(&text, TOOL_OUTPUT_CHARS) }]);
                        self.update(persona_id, event);
                    }
                }
                MultiTurnStreamItem::CompletionCall(call) => {
                    usage.0 += call.usage.input_tokens;
                    usage.1 += call.usage.output_tokens;
                    usage.2 += call.usage.total_tokens;
                }
                MultiTurnStreamItem::FinalResponse(response) => {
                    self.flush(persona_id, &mut open);
                    self.append(
                        persona_id,
                        json!({
                            "kind": "turn",
                            "id": new_id(),
                            "ts": now_ms(),
                            "stopReason": "end_turn",
                            "usage": { "inputTokens": usage.0, "outputTokens": usage.1, "totalTokens": usage.2 },
                        }),
                    );
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
        self.flush(persona_id, &mut open);
        if !ended {
            return Err("the model ended the turn without a response".to_string());
        }
        Ok(())
    }

    fn chunk(
        &self,
        persona_id: &str,
        open: &mut Option<OpenMessage>,
        kind: &'static str,
        text: &str,
    ) {
        if text.is_empty() {
            return;
        }
        if open.as_ref().is_none_or(|message| message.kind != kind) {
            self.flush(persona_id, open);
            *open = Some(OpenMessage {
                id: new_id(),
                kind,
                text: String::new(),
            });
        }
        if let Some(message) = open.as_mut() {
            message.text.push_str(text);
            self.delta(persona_id, &message.id, kind, text);
        }
    }

    /// Writes the streamed message to the transcript as one event.
    fn flush(&self, persona_id: &str, open: &mut Option<OpenMessage>) {
        let Some(message) = open.take() else { return };
        if message.text.is_empty() {
            return;
        }
        self.append(
            persona_id,
            json!({ "kind": message.kind, "id": message.id, "ts": now_ms(), "text": message.text }),
        );
    }

    /// One answer with no tools and no session: the chapter summariser's
    /// completion, and anything else that asks a model a single question.
    pub async fn complete(
        &self,
        model_id: &str,
        system: &str,
        prompt: &str,
    ) -> Result<String, String> {
        let keys = self.provider_keys().await?;
        let agent = agent_builder(&keys, model_id)?.preamble(system).build();
        agent
            .prompt(prompt)
            .await
            .map_err(|error| error.to_string())
    }
}

struct OpenMessage {
    id: String,
    kind: &'static str,
    text: String,
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

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept}…")
}

/// A tool call as a line in the transcript: the name and the one argument
/// that says what it touched.
fn describe_tool(name: &str, arguments: &Value) -> String {
    let subject = ["path", "command", "pattern", "query", "expression"]
        .iter()
        .find_map(|key| arguments.get(*key).and_then(Value::as_str))
        .map(|value| clip(value, 120));
    match subject {
        Some(subject) => format!("{name} {subject}"),
        None => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_follow_the_keys_the_desk_holds() {
        let mut keys = HashMap::new();
        assert!(models(&keys).is_empty());
        keys.insert("anthropic".to_string(), "k".to_string());
        let listed = models(&keys);
        assert!(
            listed
                .iter()
                .all(|model| model["id"].as_str().unwrap().starts_with("anthropic/"))
        );
        assert_eq!(listed[0]["group"], "Anthropic — API key");
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
}
