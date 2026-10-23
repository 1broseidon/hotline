//! Hotline Agent: the driver that runs the model in this process, on Rig.
//!
//! The shared loop owns each ordinary Rig model request and tool dispatch.
//! Steering replaces an inference attempt while preserving completed context;
//! Stop ends the activity. Provider construction and wire formats remain Rig's.

mod build;
mod granted;
mod history;
mod images;
mod recovery;
mod strict;
mod tool_output;
mod turn;

use super::failure::{Failure, Kind};
use build::{
    Reuse, agent_builder, completion_model, copilot_responses, effort_config, request_settings,
};
use history::{chunk_into, elide, flush, hand_to_model, images_to_model, keep, plain, send};
use images::Input;
#[cfg(test)]
use images::user_message;
pub(crate) use tool_output::publish_ledger;
use tool_output::{describe_tool, mcp_dynamic, result_of, rig_output};
pub(crate) use turn::process_ended;

use super::{CapabilityLease, Driver, DriverInfo, MessageKind, ToolImage, Update, clip};
use crate::contract::{
    Attachment, AttachmentKind, ConfigChoice, FolderGrant, NoticeLevel, Persona, Reach,
};
use crate::mcp::server::TeammateTools;
use crate::mcp::{self, McpServer};
use crate::models::{self, Client};
use crate::session::{ProviderAuth, ProviderKeys};
use crate::tools::{
    self, EditFile, FindFiles, ListDirectory, ReadFile, RunCommand, SearchFiles, Workspace,
    WriteFile,
};
use crate::vault::Vault;
use async_trait::async_trait;
use futures_util::StreamExt;
use hotline_room::provider_auth::ChosenAccount;
use rig::message::{
    DocumentSourceKind, Image, ImageMediaType, Message, ToolResultContent, UserContent,
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
pub(crate) const AGENT_NAME: &str = "Hotline Agent";

/// Only the tool's subject, not the whole command, goes in the title line.
const TITLE_CHARS: usize = 120;

/// How much of a result the model is shown, from its start and from its end.
/// Anything longer is kept whole on disk beside the tape, and the model is
/// told where, so it can search or read the rest. What the model is shown
/// stays in the conversation and goes out again with every later request in
/// the chapter, so each kind of result gets only what it needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Budget {
    head: usize,
    tail: usize,
}

impl Budget {
    /// A command's output. A build or a test run says how it went at the
    /// end, so most of this comes from there: a megabyte of log costs the
    /// conversation 16 KiB.
    const COMMAND: Self = Self {
        head: 4 * 1024,
        tail: 12 * 1024,
    };
    /// What a granted server or the computer answers. The computer pages its
    /// own output well inside this; the budget stops a server that doesn't.
    const EXTERNAL: Self = Self {
        head: 32 * 1024,
        tail: 32 * 1024,
    };
    /// Hotline's own tools, and a subagent's report. A file read already
    /// stops at 2000 lines and says where to go on, and cutting it shorter
    /// would only cost more reads.
    const BUILT_IN: Self = Self {
        head: 128 * 1024,
        tail: 128 * 1024,
    };

    /// A tool's budget by its name: a granted server's tools, the
    /// computer's among them, are named `{server}__{tool}`, and a granted
    /// tool may also be reached through `call_tool`.
    fn of(tool: &str) -> Self {
        if tool.contains("__") || tool == granted::CALL {
            Self::EXTERNAL
        } else {
            Self::BUILT_IN
        }
    }

    fn bytes(self) -> usize {
        self.head + self.tail
    }

    /// `bytes` of a command's output, weighted to its end like the whole.
    fn share(bytes: usize) -> Self {
        Self {
            head: bytes / 4,
            tail: bytes - bytes / 4,
        }
    }
}

/// A command's output cut to the command budget. The shell writes stdout,
/// then stderr after a line of its own, and each keeps its own start and
/// end: a test run's failures on stdout outlast a long compile on stderr.
fn command_view(output: &str) -> String {
    let budget = Budget::COMMAND.bytes();
    if output.len() <= budget {
        return output.to_string();
    }
    let Some((stdout, stderr)) = output.split_once(crate::tools::STDERR_LINE) else {
        return elide(output, Budget::COMMAND);
    };
    let half = budget / 2;
    let out = if stderr.len() < half {
        budget - stderr.len()
    } else {
        stdout.len().min(half)
    };
    format!(
        "{}{}{}",
        elide(stdout, Budget::share(out)),
        crate::tools::STDERR_LINE,
        elide(stderr, Budget::share(budget - out))
    )
}

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
    let agent = completion_builder(keys, model_id, output_limit)
        .await?
        .preamble(system)
        .build();
    agent
        .prompt(prompt)
        .await
        .map(|response| response.output())
        .map_err(|error| error.to_string())
}

/// Shared provider construction for bounded room services that own no teammate.
pub(crate) async fn completion_builder(
    keys: &HashMap<String, ProviderAuth>,
    model_id: &str,
    output_limit: Option<u64>,
) -> Result<rig::agent::AgentBuilder, String> {
    completion_builder_with_effort(keys, model_id, None, output_limit).await
}

/// The same, at a thinking level the model lists; `None` leaves it to the
/// model.
pub(crate) async fn completion_builder_with_effort(
    keys: &HashMap<String, ProviderAuth>,
    model_id: &str,
    effort: Option<&str>,
    output_limit: Option<u64>,
) -> Result<rig::agent::AgentBuilder, String> {
    agent_builder(keys, model_id, effort, output_limit, Reuse::Once).await
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
    /// Learned at `start` with the cwd: the folders the person granted
    /// besides it. A change to them restarts the session, so a handle built
    /// from these never outlives the grant it was built from.
    folders: Mutex<Vec<FolderGrant>>,
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
    /// Shared with every tool handle this session created.
    capability: Option<CapabilityLease>,
    /// Protected MCP OAuth registrations and tokens. None is retained for
    /// standalone test drivers, which keep OAuth servers refused.
    mcp_vault: Option<Arc<Vault>>,
    /// Commands this session's turns left running when they replied. They
    /// die with the driver, and with [`Driver::invalidate`].
    processes: Arc<crate::session::jobs::Processes>,
    /// Named on every ask for the keys, whatever the model: the keys put it
    /// in place of its own provider's default and leave the others alone.
    account: Option<ChosenAccount>,
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
            folders: Mutex::new(Vec::new()),
            model: Mutex::new(String::new()),
            effort: Mutex::new(None),
            history: Arc::new(AsyncMutex::new(history)),
            history_origin: Arc::new(Mutex::new(None)),
            stop: Mutex::new(Arc::new(Stop::default())),
            steering: Mutex::new(None),
            unconsumed: Arc::new(Mutex::new(Vec::new())),
            output_dir: output_dir.clone(),
            mcp_servers: Vec::new(),
            mcp_missing: Vec::new(),
            mcp: Mutex::new(None),
            teammate,
            capability: None,
            mcp_vault: None,
            processes: crate::session::jobs::Processes::new(output_dir),
            account: None,
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

    pub(crate) fn with_account(mut self, account: Option<ChosenAccount>) -> Self {
        self.account = account;
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
        Some(self.info(&self.keys.provider_auth_for(self.account.as_ref())))
    }

    /// A started driver has a model unless the desk had no provider to give
    /// it one.
    fn startup_failure(&self) -> Option<String> {
        lock(&self.model)
            .is_empty()
            .then(|| Failure::no_provider().notice())
    }

    async fn start(&self, persona: &Persona) -> Result<DriverInfo, String> {
        if let Some(capability) = &self.capability {
            capability.check()?;
        }
        let keys = self.keys.provider_auth_for(self.account.as_ref());
        // With no provider there is nothing to run a model on. The teammate
        // still comes up, with no model, so it can say why before anyone
        // sends it a word: see `startup_failure`.
        if keys.is_empty() {
            return Ok(self.info(&keys));
        }
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
        *lock(&self.folders) = persona.folders.clone().unwrap_or_default();
        lock(&self.model).clone_from(&model);
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
        if !self.teammate.in_run() && !self.teammate.in_work() {
            publish_ledger(
                persona,
                &self.mcp_missing,
                &connected,
                self.teammate.offers_subagents(),
                &self.teammate.tool_names(),
            );
        }
        *lock(&self.mcp) = Some(connected);
        Ok(self.info(&keys))
    }

    async fn prompt(
        &self,
        text: String,
        attachments: Vec<Attachment>,
        reach: Reach,
    ) -> mpsc::Receiver<Update> {
        self.start(Some(Input::Attachments(text, attachments)), reach)
            .await
    }

    /// The failed turn again on the history it left: the person's line is
    /// already its last message (with any reply that was shown before the
    /// failure, and the note saying so), so nothing is added twice. A history
    /// that no longer ends there, after a restart that lost it, is asked the
    /// words again.
    async fn retry(
        &self,
        text: String,
        attachments: Vec<Attachment>,
        reach: Reach,
    ) -> mpsc::Receiver<Update> {
        let resumable = matches!(self.history.lock().await.last(), Some(Message::User { .. }));
        let message = (!resumable).then(|| Input::Attachments(text, attachments));
        self.start(message, reach).await
    }

    fn steer(&self, text: String, attachments: Vec<Attachment>) -> bool {
        lock(&self.steering)
            .as_ref()
            .is_some_and(|steering| steering.admit(Input::Attachments(text, attachments)))
    }

    fn take_unconsumed(&self) -> Vec<(String, Vec<Attachment>)> {
        std::mem::take(&mut *lock(&self.unconsumed))
    }

    fn subscribe_wakes(&self) -> Option<mpsc::UnboundedReceiver<String>> {
        Some(self.processes.subscribe())
    }

    fn processes(&self) -> Option<Arc<crate::session::jobs::Processes>> {
        Some(self.processes.clone())
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
        self.processes.stop_all();
        // Dropping the live connections closes transports and kills granted
        // stdio server groups immediately. Cloned McpTool handles still carry
        // the lease and refuse calls after the room advances it.
        lock(&self.mcp).take();
    }

    async fn set_model(&self, model_id: &str) -> Result<DriverInfo, String> {
        let keys = self.keys.provider_auth_for(self.account.as_ref());
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
        let keys = self.keys.provider_auth_for(self.account.as_ref());
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

impl InProcess {
    /// One turn: a new message, or none to go again on the history as it is.
    async fn start(&self, message: Option<Input>, reach: Reach) -> mpsc::Receiver<Update> {
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
        // The computer's tools are definitions; a granted server's are named
        // in the preamble and reached through two fixed tools.
        let (granted, computer): (Vec<_>, Vec<_>) = lock(&self.mcp)
            .as_ref()
            .map(|connected| connected.tools.clone())
            .unwrap_or_default()
            .into_iter()
            .partition(granted::is_granted);
        mcp_tools.extend(computer.into_iter().map(mcp_dynamic));
        let index = granted::index(&granted);
        if !granted.is_empty() {
            mcp_tools.extend(granted::tools(granted));
        }
        let model = lock(&self.model).clone();
        let metadata = self.keys.model_metadata();
        let output_limit = metadata.get(&model).and_then(|model| model.output_limit);
        let keys = self.keys.provider_auth_for(self.account.as_ref());
        let meter = turn::ChatMeter::for_model(self.keys.budget(), &keys, &model, &metadata);
        let turn = Turn {
            keys,
            model: model.clone(),
            output_limit,
            context_limit: metadata.get(&model).and_then(|m| m.context_limit),
            meter,
            effort: lock(&self.effort).clone(),
            preamble: if index.is_empty() {
                self.preamble.clone()
            } else {
                format!("{}\n\n{index}", self.preamble)
            },
            cwd: lock(&self.cwd).clone(),
            folders: lock(&self.folders).clone(),
            reach,
            history: self.history.clone(),
            stop,
            steering,
            output_dir: self.output_dir.clone(),
            mcp_tools,
            capability: self.capability.clone(),
            delegate: self.teammate.delegate(),
            processes: self.processes.clone(),
        };
        let history_origin = self.history_origin.clone();
        let unconsumed = self.unconsumed.clone();
        let provider_keys = self.keys.clone();
        let account = self.account.clone();
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
            let message = match message {
                Some(message) => Some(message.prepare(&turn.stop).await),
                None => None,
            };
            let result = turn.run(&sender, message).await;
            let pending = turn.steering.close();
            if !turn.stop.raised.load(Ordering::SeqCst) {
                for input in pending {
                    if let Input::Attachments(text, attachments) = input {
                        lock(&unconsumed).push((text, attachments));
                    }
                }
            }
            if let Err(error) = result {
                let mut error = error.checked_against_sign_in(&turn.model, &turn.keys).await;
                error.account_id =
                    account_on_notice(provider_keys.as_ref(), account.as_ref(), &turn.model);
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
}

/// The credential a failed turn on `model` names: only one there was a choice
/// of, so a room with one account per provider writes the notice it always
/// has. Asked only once a turn has failed, since the default's id costs
/// another read of the keychain.
fn account_on_notice(
    keys: &dyn ProviderKeys,
    account: Option<&ChosenAccount>,
    model: &str,
) -> Option<String> {
    let provider = model.split('/').next().unwrap_or_default();
    match account {
        Some(account) if account.provider_id == provider => Some(account.id.clone()),
        _ => keys
            .default_account(provider)
            .and_then(|(id, others)| others.then_some(id)),
    }
}

/// Everything one turn needs, taken from the session at the moment it starts
/// so the turn owns it and the driver stays free to answer other calls.
struct Turn {
    keys: HashMap<String, ProviderAuth>,
    model: String,
    /// The Chat budget this turn's rounds are metered against; `None` for a
    /// model that costs nothing per token, or a desk without budgets.
    meter: Option<turn::ChatMeter>,
    output_limit: Option<u64>,
    context_limit: Option<u64>,
    effort: Option<String>,
    preamble: String,
    cwd: PathBuf,
    folders: Vec<FolderGrant>,
    reach: Reach,
    history: Arc<AsyncMutex<Vec<Message>>>,
    stop: Arc<Stop>,
    steering: Arc<turn::Steering>,
    output_dir: PathBuf,
    mcp_tools: Vec<DynamicTool>,
    capability: Option<CapabilityLease>,
    /// Who runs this teammate's subagents, when this session may start them.
    delegate: Option<Arc<dyn crate::session::jobs::Delegate>>,
    /// Where a command the turn leaves running goes.
    processes: Arc<crate::session::jobs::Processes>,
}

impl Turn {
    /// A turn on the history, after `message` when there is one; a retry has
    /// none, because the line it answers is already the history's last.
    #[expect(clippy::result_large_err, reason = "Failure is the whole diagnosis")]
    async fn run(
        &self,
        sender: &mpsc::Sender<Update>,
        message: Option<Message>,
    ) -> Result<(), Failure> {
        // Keep admitted input even if workspace or provider construction fails.
        if let Some(message) = message {
            self.history.lock().await.push(message);
        }
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
            &self.folders,
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
        let model = completion_model(&self.keys, &self.model, Reuse::Rounds).await?;
        let template = turn::Template {
            tools: tools.tool_definitions(),
            settings: request_settings(
                &self.keys,
                &self.model,
                self.effort.as_deref(),
                self.output_limit,
                Reuse::Rounds,
            )?,
        };
        turn::run(&model, template, &tools, self, sender, shell).await
    }
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn lock<T>(held: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    held.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
