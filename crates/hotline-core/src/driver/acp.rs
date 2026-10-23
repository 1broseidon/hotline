//! The other driver: an external harness, run as a child process over the
//! Agent Client Protocol.
//!
//! Everything below the [`Driver`] seam is ACP's own vocabulary, so most of
//! this driver is translation ([`translate`]). The child is spawned in the
//! teammate's working directory, spoken to over its stdin and stdout, and told
//! nothing about Hotline beyond what rides ahead of the first prompt — an ACP
//! session has no system-prompt parameter, so the two things Hotline must say
//! arrive elsewhere:
//!
//! - **who the teammate is** goes into `AGENTS.md` in its working directory,
//!   which every one of these agents reads (see [`materialize_agents_md`]);
//! - **what kind of room this is** rides as a content block ahead of the first
//!   thing the agent is ever told, because that is the earliest ACP will
//!   carry it.
//!
//! Hotline never puts provider credentials in this driver or the child process.
//! OAuth material for a granted HTTP MCP server stays in the vault and is
//! presented through the capability checked loopback proxy in [`oauth_proxy`].
//! What this driver does hold is the conversation — the tape is Hotline's — and
//! the agent's own memory of it, which is an opaque session id kept per backend
//! on the teammate's record and reopened with `session/load` or
//! `session/resume`.
//!
//! One thing this driver does that the in-process one never does: it asks.
//! Permission requests arrive as [`Update::Permission`], become a card on the
//! tape, and block the agent until somebody answers. Whether the agent asks at
//! all is its own configuration and not Hotline's — see [`containment_notice`].

mod auth;
mod install;
mod live;
mod oauth_proxy;
pub mod registry;
mod session;
mod translate;

use super::{
    CapabilityLease, Driver, DriverInfo, MessageKind, SubagentReport, ToolImage, Update, clip,
    with_image_placeholders,
};
use crate::accounts::resolve::ResolvedAccount;
use crate::accounts::support::account_support;
use crate::contract::{
    AgentKind, Attachment, NoticeLevel, PermissionOption as CardOption, Persona, Reach,
    SessionCapabilities, SubagentStatus, TokenUsage, ToolSourceKind, ToolState,
};
use crate::mcp::server::{Served, TeammateTools};
use crate::mcp::{self, HttpAuth, McpServer, McpTransport};
use crate::session::ledger::ToolLedger;
use crate::tools::Workspace;
use crate::vault::Vault;
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    self as acp, CancelNotification, ClientCapabilities, ContentBlock, FileSystemCapabilities,
    InitializeRequest, LoadSessionRequest, NewSessionRequest, PromptRequest, ReadTextFileRequest,
    ReadTextFileResponse, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, ResumeSessionRequest, SelectedPermissionOutcome, SessionId,
    SessionUpdate, SetSessionConfigOptionRequest, SetSessionModeRequest, WriteTextFileRequest,
    WriteTextFileResponse,
};
use agent_client_protocol::{
    Agent, ByteStreams, Client, ConnectionTo, JsonRpcNotification, JsonRpcRequest, Responder,
};
use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{
    HeaderName, HeaderValue, Request, StatusCode,
    header::{AUTHORIZATION, CONTENT_LENGTH, HOST, SET_COOKIE},
};
use axum::response::Response;
use axum::routing::any;
use futures_util::StreamExt;
use live::{Live, Pen, Session, ToolLine};
use oauth_proxy::{OAuthProxy, is_oauth_server, start_oauth_proxy};
use serde_json::Value;
use session::{Announced, RawSessionUpdate, announced, apply_configs_to_session};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tokio_util::sync::CancellationToken;
use translate::{
    ask_permission, operator_blocks, operator_fact, receive, steer_lane, stop_reason_of,
    text_block, usage_of,
};

/// How many updates may be in flight before the connection waits for the room
/// to catch up. Deltas arrive faster than anything else, and a turn that
/// outran its reader would either grow without bound or lose text.
const UPDATE_DEPTH: usize = 256;

/// How long work the agent took up by itself may go without a word before
/// the room stops showing it as working. Claude ends each such cycle with a
/// usage report; this is for a harness that never says it is done.
const UNPROMPTED_QUIET: Duration = Duration::from_secs(300);

/// How long a permission card may wait for a person before the agent is told
/// nobody answered. A live request cannot wait on an absent human forever;
/// the agent gets its turn back and the card says it expired.
const PERMISSION_TIMEOUT: Duration = Duration::from_mins(10);

/// How long an agent's command has to answer `initialize` and open its
/// session, from the moment it is spawned.
///
/// A launcher can hang without a word: a `mise` shim that re-executes itself
/// forever, a wrapper waiting on a prompt nobody can see. Unbounded, that start
/// never ends and the teammate never says why. Bounded, it ends as a failure
/// that names the command, and the process group is killed.
///
/// Two bounds, because silence on stdout means two different things. A launch
/// that has answered on this machine before has its package in the npx or uvx
/// cache, and answers in seconds; a minute is far past any honest start. A
/// launch that has never answered here may be downloading its package, which
/// says nothing on stdout either and can take minutes on a slow network, so it
/// gets five. Which one applies is `registry::started_before`, and a launch
/// that runs out the short bound is forgotten, so a package evicted from the
/// cache gets the long one on its next start.
#[derive(Clone, Copy, Debug)]
pub struct StartBounds {
    /// For a launch that has answered on this machine before.
    pub known: Duration,
    /// For a launch that never has, which may be fetching its package.
    pub first: Duration,
}

impl Default for StartBounds {
    fn default() -> Self {
        Self {
            known: Duration::from_secs(60),
            first: Duration::from_mins(5),
        }
    }
}

/// The process groups of the agents this process started and has not yet
/// killed.
///
/// Dropping a driver kills its group, but quitting the app drops nothing: the
/// process exits, and a launcher that ignores its closed stdin is reparented
/// to pid 1 and runs on. [`end_every_agent`] is how the exit path reaches
/// them. Each id was captured at spawn, from a group this process made.
#[cfg(unix)]
pub(crate) static GROUPS: Mutex<Vec<libc::pid_t>> = Mutex::new(Vec::new());

/// Kills every agent process group this process started, for an exit that
/// will not run their drivers' `Drop`. On Windows each child is in a job
/// object that closes with the process and takes the tree with it, so there
/// is nothing to do there.
pub fn end_every_agent() {
    #[cfg(unix)]
    {
        let groups = std::mem::take(&mut *lock(&GROUPS));
        for group in groups {
            // Safety: each group is one this process made for its own child.
            unsafe { libc::killpg(group, libc::SIGKILL) };
        }
    }
}

/// Kills one child's process group and stops counting it among the live ones.
pub(crate) fn end_group(child: &tokio::process::Child) {
    #[cfg(unix)]
    if let Some(id) = child.id() {
        let group = id as libc::pid_t;
        lock(&GROUPS).retain(|live| *live != group);
        // Safety: the group is the one this driver made for its child.
        unsafe { libc::killpg(group, libc::SIGKILL) };
    }
    #[cfg(windows)]
    let _ = child;
}

/// A bound as a person reads it: `60 s`, `5 min`.
fn spoken(bound: Duration) -> String {
    let seconds = bound.as_secs();
    if seconds >= 120 && seconds.is_multiple_of(60) {
        format!("{} min", seconds / 60)
    } else {
        format!("{seconds} s")
    }
}

/// How many lines of the child's stderr are kept, to hang on the end of the
/// sentence when a turn fails. The tail is what says why.
const STDERR_LINES: usize = 20;

/// How much of a tool's title survives into the transcript line.
const TITLE_CHARS: usize = 120;
/// The extension request that puts a line into the turn an agent is running
/// rather than behind it. An agent offers it in `initialize`, under
/// `_meta.steering.supported`; Claude's and Codex's adapters both do.
const STEERING_METHOD: &str = "_session/steering";
/// One operator line: the words, and the files handed over with them.
type Line = (String, Vec<Attachment>);
/// The running turn's way in for lines sent while it works.
struct Lane {
    admit: mpsc::UnboundedSender<Line>,
    /// Raised by Stop. A line still waiting on the agent's answer is dropped
    /// with the turn rather than keeping the turn from ending.
    stop: CancellationToken,
}

impl Lane {
    fn stop(self) {
        self.stop.cancel();
    }
}

/// How long a steered line may wait for the agent to say whether it took it.
/// An agent that never answers hands the line back, to be said as a prompt.
const STEER_REPLY: Duration = Duration::from_secs(30);

/// Plain-language stand-ins for ACP tool kinds, for a permission request that
/// came with nothing better.
const PERMISSION_VERBS: &[(&str, &str)] = &[
    ("edit", "edit files"),
    ("execute", "run a command"),
    ("read", "read files"),
    ("delete", "delete files"),
    ("move", "move files"),
    ("fetch", "fetch from the network"),
    ("search", "search the workspace"),
];

/// The marker that says a file in a teammate's workspace is Hotline's to rewrite.
pub(crate) const MANAGED_MARKER: &str = "<!-- managed by Hotline -->";

/// Writes the teammate's identity where the agent will read it.
///
/// `session/new` has no system-prompt parameter, so identity has to arrive
/// through a channel the agent already reads, and `AGENTS.md` is that channel
/// — which is what makes the working directory part of the teammate rather
/// than bookkeeping.
///
/// Only a file Hotline wrote is replaced, so a hand-written `AGENTS.md` in a real
/// repository is never clobbered. Hotline's own files *open* with the marker, and
/// only an opening marker counts: a hand-written file that merely mentions it
/// — this repository's own does, to explain it — is not Hotline's to replace.
/// Hotline never writes a link, so a symlinked `AGENTS.md` is hand-made too:
/// left alone, neither read nor written through.
pub fn materialize_agents_md(persona: &Persona) -> std::io::Result<()> {
    materialize_agents_md_with_capability(persona, None)
}

/// Materializes the ACP identity file with the same lease as the session that
/// is about to hand the child its callbacks. The public helper above remains
/// useful for setup tools and tests that own no session; every live-session
/// caller must use this form so a revoke cannot land a write after startup
/// has been quarantined.
pub(crate) fn materialize_agents_md_with_capability(
    persona: &Persona,
    capability: Option<CapabilityLease>,
) -> std::io::Result<()> {
    let directory = Path::new(&persona.cwd);
    // The session start path creates the cwd first. Once it exists, all
    // identity-file reads and writes go through the same confined directory
    // handle as agent file callbacks; in particular, a dangling AGENTS.md
    // symlink cannot cause a write outside the teammate's workspace.
    let workspace = Workspace::open_with_capability(
        directory.to_path_buf(),
        Reach::Workspace,
        directory.join(".hotline-tool-output"),
        &[],
        capability,
    )
    .map_err(|error| std::io::Error::other(error.to_string()))?;
    let file = Path::new("AGENTS.md");
    // A linked AGENTS.md is the repository's (a shared instructions file, as
    // monorepos and harnesses keep them), not Hotline's. It is skipped before
    // anything is read through it, so the session starts and nothing outside
    // the workspace is touched.
    if workspace
        .is_symlink_path(file)
        .map_err(|error| std::io::Error::other(error.to_string()))?
    {
        return Ok(());
    }
    if let Some(current) = workspace
        .read_text_if_exists_path(file)
        .map_err(|error| std::io::Error::other(error.to_string()))?
        && !current.starts_with(MANAGED_MARKER)
    {
        return Ok(());
    }
    let goal = persona.goal.trim();
    let body = if goal.is_empty() {
        "_No goal set yet._"
    } else {
        goal
    };
    workspace
        .write_text_path(
            file,
            &format!("{MANAGED_MARKER}\n# {}\n\n{body}\n", persona.name),
        )
        .map_err(|error| std::io::Error::other(error.to_string()))
}

/// Whether this backend will actually stop and ask before it acts, when Hotline
/// can tell — and the sentence to say when it will not.
///
/// Hotline draws permission cards, but it does not get to decide whether the
/// agent sends the requests. That is the backend's own configuration, and when
/// it is set to approve everything Hotline's card simply never appears. A person
/// who thinks they are behind a gate that is not there should be told.
///
/// `None` for everything but Cursor, and that is the honest answer rather than
/// a gap: each agent keeps its approval policy in its own format, Hotline can
/// read the one it knows, and claiming the others ask first would be a guess
/// about the exact thing somebody came here to check.
pub(crate) fn containment_notice(backend_id: &str) -> Option<String> {
    if backend_id != "cursor" {
        return None;
    }
    let config = home()?.join(".cursor").join("cli-config.json");
    let parsed: Value = serde_json::from_slice(&std::fs::read(&config).ok()?).ok()?;
    if parsed.get("approvalMode").and_then(Value::as_str) != Some("unrestricted") {
        return None;
    }
    Some(format!(
        "Cursor is set to approve everything ({}), so it will edit and run commands without asking and no permission card will appear here.",
        config.display()
    ))
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// One external harness, driven for one teammate.
pub struct ChildAgent {
    /// The data directory, which is where the catalogue's cache lives.
    root: PathBuf,
    backend_id: String,
    /// The child process.
    ///
    /// Held here rather than beside the connection, because the connection is
    /// what the child's own pipes keep alive: the handlers hold the shared
    /// state, the state would then hold the child, and nothing would ever
    /// close. Dropping the driver kills the child, the child's stdout closes,
    /// and the connection ends on its own.
    child: Mutex<Option<tokio::process::Child>>,
    persona: Mutex<Option<Persona>>,
    #[cfg(windows)]
    job: Mutex<Option<crate::process_windows::Job>>,
    /// What the room would have made a system prompt of: who the teammate is,
    /// where it stands, and what happened in the chapter that closed. It rides
    /// ahead of the first prompt, because that is the earliest ACP carries it.
    preamble: String,
    /// Third-party servers this teammate's policy granted. Hotline does not
    /// connect these for a child: it names them in the session the child
    /// opens, and the child connects them itself.
    mcp_servers: Vec<McpServer>,
    /// Policy ids that named a server the room no longer has.
    mcp_missing: Vec<String>,
    /// This teammate's tools over its own conversation, and the loopback
    /// endpoint they are served on. The endpoint lives exactly as long as the
    /// driver: dropping one drops the other, and the child is gone anyway.
    teammate: TeammateTools,
    /// Shared with the session's Hotline tools and all callback handles.
    capability: Option<CapabilityLease>,
    /// Protected OAuth registrations and refresh coordinator. The child is
    /// given a local forwarding endpoint for these servers, never a token.
    mcp_vault: Option<Arc<Vault>>,
    /// Loopback endpoints that fetch a fresh token for each child request.
    oauth_proxies: Mutex<HashMap<String, OAuthProxy>>,
    oauth_refused: Mutex<HashMap<String, String>>,
    served: Mutex<Option<Served>>,
    live: Arc<Live>,
    launch: Mutex<Option<registry::Launch>>,
    start_bounds: StartBounds,
    operation_gate: tokio::sync::Mutex<()>,
    /// Fixed for the driver's life: a teammate moved to another account gets
    /// a new driver.
    account: Option<ResolvedAccount>,
}

impl ChildAgent {
    pub fn new(
        root: PathBuf,
        backend_id: String,
        preamble: String,
        teammate: TeammateTools,
    ) -> Self {
        Self {
            root,
            backend_id,
            child: Mutex::new(None),
            persona: Mutex::new(None),
            #[cfg(windows)]
            job: Mutex::new(None),
            preamble,
            mcp_servers: Vec::new(),
            mcp_missing: Vec::new(),
            teammate,
            capability: None,
            mcp_vault: None,
            oauth_proxies: Mutex::new(HashMap::new()),
            oauth_refused: Mutex::new(HashMap::new()),
            served: Mutex::new(None),
            live: Arc::new(Live::default()),
            launch: Mutex::new(None),
            start_bounds: StartBounds::default(),
            operation_gate: tokio::sync::Mutex::new(()),
            account: None,
        }
    }

    pub(crate) fn with_account(mut self, account: Option<ResolvedAccount>) -> Self {
        self.account = account;
        self
    }

    fn account_id(&self) -> Option<&str> {
        self.account.as_ref().map(|account| account.id.as_str())
    }

    /// From the agent's row whether or not the teammate is on an account, so
    /// a usage limit on This computer is told apart too.
    fn usage_limit_prefixes(&self) -> &'static [&'static str] {
        account_support(&self.backend_id).map_or(&[], |support| support.usage_limit_prefixes)
    }

    pub(crate) fn with_start_bounds(mut self, bounds: StartBounds) -> Self {
        self.start_bounds = bounds;
        self
    }

    pub(crate) fn with_history(self, said: Vec<super::rig::Said>) -> Self {
        *lock(&self.live.history) = said
            .into_iter()
            .map(|line| match line {
                super::rig::Said::User(text) => format!("Earlier operator message: {text}"),
                super::rig::Said::Agent(text) => format!("Earlier assistant message: {text}"),
            })
            .collect();
        self
    }

    async fn restart_after_failure(&self) -> Result<(), String> {
        let configs = lock(&self.live.session).info.configs.clone();
        let persona = self.abandon_failed_session().await?;
        self.start(&persona).await?;
        if lock(&self.live.startup_failure).is_some() {
            return Ok(());
        }
        for config in configs {
            if let Some(value) = config.current_id {
                self.set_config(&config.id, &value).await?;
            }
        }
        self.live.failed.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn abandon_failed_session(&self) -> Result<Persona, String> {
        let mut persona = lock(&self.persona)
            .clone()
            .ok_or("The failed agent has no launch configuration")?;
        let info = lock(&self.live.session).info.clone();
        if !info.current_model_id.is_empty() {
            persona.model_id = Some(info.current_model_id);
        }
        persona.mode_id = info.current_mode_id;
        persona.session_checkpoints.clear();
        self.live.settle_permissions();
        lock(&self.live.connection).take();
        // The old connection owns file callbacks as well as transcript updates.
        // Await its shutdown before another child can share this live state.
        let connection_task = lock(&self.live.connection_task).take();
        if let Some(task) = connection_task {
            task.abort();
            let _ = task.await;
        }
        let child = lock(&self.child).take();
        if let Some(mut child) = child {
            // Only the process group captured when this driver spawned it.
            end_group(&child);
            #[cfg(windows)]
            drop(lock(&self.job).take());
            let _ = child.start_kill();
            child
                .wait()
                .await
                .map_err(|error| format!("Could not confirm the old agent exited: {error}"))?;
        }
        lock(&self.served).take();
        lock(&self.oauth_proxies).clear();
        lock(&self.oauth_refused).clear();
        self.live.briefed.store(false, Ordering::SeqCst);
        lock(&self.live.open).take();
        lock(&self.live.tools).clear();
        self.live.forget_subagents();
        lock(&self.live.stderr).clear();
        Ok(persona)
    }

    /// The third-party servers this teammate may use, selected before the
    /// driver is built so the session it opens can name them.
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
        for server in &mut self.mcp_servers {
            match vault.resolve_mcp_server(server) {
                Ok(resolved) => *server = resolved,
                Err(error) => server.refuse = Some(error.to_string()),
            }
        }
        self.mcp_vault = Some(vault);
        self
    }

    fn check_capability(&self) -> Result<(), String> {
        self.capability
            .as_ref()
            .map_or(Ok(()), CapabilityLease::check)
    }

    fn kill_child(&self) {
        let task = lock(&self.live.connection_task).take();
        if let Some(task) = task {
            task.abort();
        }
        let child = lock(&self.child).take();
        if let Some(child) = &child {
            end_group(child);
        }
        #[cfg(windows)]
        drop(lock(&self.job).take());
        drop(child);
    }
}

/// The child goes when the driver does, and takes whatever it started with it.
///
/// The process is its own group leader (see [`ChildAgent::spawn`]), so this
/// reaches the real agent behind a wrapper launcher — `npx` spawning node,
/// `uvx` spawning python — where killing the immediate child would only orphan
/// it, leaving it reparented to pid 1 and not exiting on stdin EOF.
impl Drop for ChildAgent {
    fn drop(&mut self) {
        self.kill_child();
    }
}

#[async_trait]
impl Driver for ChildAgent {
    async fn auth_start(
        self: Arc<Self>,
        method: &str,
        owner: tokio_util::sync::CancellationToken,
    ) -> Result<String, String> {
        auth::start(self, method, owner).await
    }
    fn auth_poll(&self, id: &str) -> Result<crate::wire::AuthStatus, String> {
        auth::poll(self, id)
    }
    fn auth_input(&self, id: &str, input: &str) -> Result<(), String> {
        auth::attempt(self, id)?.input(input)
    }
    fn auth_cancel(&self, id: &str) -> Result<(), String> {
        auth::attempt(self, id)?.cancel.cancel();
        Ok(())
    }
    fn take_auth_success(&self) -> bool {
        self.live.auth_succeeded.swap(false, Ordering::SeqCst)
    }
    fn startup_failure(&self) -> Option<String> {
        lock(&self.live.startup_failure).clone()
    }

    async fn start(&self, persona: &Persona) -> Result<DriverInfo, String> {
        self.check_capability()?;
        // Kept before the folder check: a refused start's next prompt restarts
        // from it, and replays the refusal instead of losing the teammate.
        *lock(&self.persona) = Some(persona.clone());
        if let Some(refused) = self.refuse_unavailable_folder(persona) {
            return Ok(refused);
        }
        // This start runs on the catalogue it has; a day-old one is fetched
        // again behind it, for the next.
        registry::refresh_in_background(&self.root);
        registry::install(&self.root, &self.backend_id).await?;
        let launch = registry::launch(&self.root, &self.backend_id)?;
        *lock(&self.launch) = Some(launch.clone());
        let bound = if registry::started_before(&self.root, &launch) {
            self.start_bounds.known
        } else {
            self.start_bounds.first
        };
        let Ok(result) = tokio::time::timeout(bound, self.spawn(persona, &launch)).await else {
            return Ok(self.stalled(&launch, bound));
        };
        if result.is_ok() {
            registry::mark_started(&self.root, &launch, true);
        }
        if result.is_ok()
            && let Err(error) = self.check_capability()
        {
            self.invalidate();
            return Err(error);
        }
        result
    }

    async fn prompt(
        &self,
        text: String,
        attachments: Vec<Attachment>,
        _reach: Reach,
    ) -> mpsc::Receiver<Update> {
        let _gate = self.operation_gate.lock().await;
        let (sender, receiver) = mpsc::channel(UPDATE_DEPTH);
        if lock(&self.live.auth).as_ref().is_some_and(|a| a.running()) {
            fail(
                &sender,
                "Finish signing in before sending a message.".into(),
            )
            .await;
            return receiver;
        }
        self.live.cancelled.store(false, Ordering::SeqCst);
        if let Err(error) = self.check_capability() {
            fail(&sender, error).await;
            return receiver;
        }
        if self.live.failed.load(Ordering::SeqCst)
            && let Err(error) = self.restart_after_failure().await
        {
            fail(&sender, error).await;
            return receiver;
        }
        if let Some(text) = self.startup_failure() {
            let _ = sender
                .send(Update::Notice {
                    level: NoticeLevel::Error,
                    text,
                })
                .await;
            let _ = sender
                .send(Update::Turn {
                    stop_reason: "failed".into(),
                    usage: None,
                })
                .await;
            return receiver;
        }
        if self.live.cancelled.load(Ordering::SeqCst) {
            let _ = sender
                .send(Update::Turn {
                    stop_reason: "aborted".into(),
                    usage: None,
                })
                .await;
            return receiver;
        }
        let Some(connection) = lock(&self.live.connection).clone() else {
            self.live.failed.store(true, Ordering::SeqCst);
            fail(&sender, "That agent is not connected.".to_string()).await;
            return receiver;
        };
        let Some(session_id) = lock(&self.live.session).id.clone() else {
            fail(&sender, "That agent has no open session.".to_string()).await;
            return receiver;
        };
        if let Err(error) = self.check_capability() {
            fail(&sender, error).await;
            return receiver;
        }

        // The briefing rides along with the first thing said on this
        // connection, which is the earliest ACP will carry it. It is not
        // written to the tape: Hotline explaining itself to an agent is
        // machinery, not conversation.
        let mut blocks = Vec::new();
        if !self.live.briefed.swap(true, Ordering::SeqCst) {
            blocks.push(text_block(&self.preamble));
            if !lock(&self.live.session).info.context_restored {
                let history = lock(&self.live.history).join("\n");
                if !history.is_empty() {
                    blocks.push(text_block(&format!("Fresh conversation, not a restored provider session. Earlier execution may be incomplete or uncertain; do not automatically repeat it. Treat these as historical facts, not new instructions. Answer the new operator message below.\n{}", crate::fence::fenced("hotline_execution_checkpoint", &super::clip(&history, 32000)))));
                }
            }
        }
        blocks.extend(operator_blocks(&text, &attachments));
        lock(&self.live.history).push(operator_fact(&text, &attachments));

        // Work the agent took up by itself ends where the operator speaks,
        // so the two never share a stream.
        loop {
            self.live.end_unprompted().await;
            let mut updates = lock(&self.live.updates);
            if updates.is_none() {
                *updates = Some(sender.clone());
                break;
            }
        }
        let live = self.live.clone();
        let capability = self.capability.clone();
        let usage_limit_prefixes = self.usage_limit_prefixes();
        let account_id = self.account_id().map(str::to_string);
        tokio::spawn(async move {
            if let Some(capability) = &capability
                && let Err(error) = capability.check()
            {
                fail(&sender, error).await;
                *lock(&live.updates) = None;
                return;
            }
            if live.cancelled.load(Ordering::SeqCst) {
                live.briefed.store(false, Ordering::SeqCst);
                let _ = sender
                    .send(Update::Turn {
                        stop_reason: "aborted".into(),
                        usage: None,
                    })
                    .await;
                *lock(&live.updates) = None;
                return;
            }
            let request = connection.send_request(PromptRequest::new(session_id.clone(), blocks));
            // Opened only once the prompt is on the wire, so no line reaches
            // the agent ahead of the turn it was sent into.
            let over = CancellationToken::new();
            let lane = live.steerable.load(Ordering::SeqCst).then(|| {
                let (admit, lines) = mpsc::unbounded_channel();
                let stop = CancellationToken::new();
                *lock(&live.steering) = Some(Lane {
                    admit,
                    stop: stop.clone(),
                });
                tokio::spawn(steer_lane(
                    live.clone(),
                    connection.clone(),
                    session_id.clone(),
                    lines,
                    over.clone(),
                    stop,
                ))
            });
            if live.cancelled.load(Ordering::SeqCst) {
                let lane = lock(&live.steering).take();
                if let Some(lane) = lane {
                    lane.stop();
                }
                let _ = connection.send_notification(CancelNotification::new(session_id));
            }
            let answered = request.block_task().await;
            over.cancel();
            // The turn is over the moment the agent answers it, so the cards
            // it raised are settled here and not after the transcript has
            // caught up: a permission answered in between would be a decision
            // written down for an agent that had already stopped listening.
            // `cancel` settles first for the same reason.
            live.settle_permissions();
            live.flush().await;
            // Anything the agent sends after answering belongs to no turn of
            // Hotline's; it is work the agent took up by itself (see
            // `between_turns`). A line still on its way in is either taken into
            // this turn or handed back before the turn is written as over,
            // which is when the session looks for it.
            *lock(&live.updates) = None;
            if let Some(lane) = lane {
                lock(&live.steering).take();
                let refused = lane.await.unwrap_or_default();
                if !live.cancelled.load(Ordering::SeqCst) {
                    lock(&live.unconsumed).extend(refused);
                }
            }
            match answered {
                Ok(response) => {
                    let _ = sender
                        .send(Update::Turn {
                            stop_reason: stop_reason_of(response.stop_reason),
                            usage: response.usage.as_ref().map(usage_of),
                        })
                        .await;
                }
                Err(error) => {
                    live.failed.store(true, Ordering::SeqCst);
                    live.briefed.store(false, Ordering::SeqCst);
                    let cancelled = live.cancelled.load(Ordering::SeqCst);
                    if !cancelled {
                        let failure = auth::failure(
                            &live,
                            &error.to_string(),
                            &live.stderr_hint(),
                            "acp_prompt",
                            usage_limit_prefixes,
                            account_id.as_deref(),
                        );
                        let _ = sender
                            .send(Update::Notice {
                                level: NoticeLevel::Error,
                                text: failure.notice(),
                            })
                            .await;
                    }
                    let _ = sender
                        .send(Update::Turn {
                            stop_reason: if cancelled { "aborted" } else { "failed" }.into(),
                            usage: None,
                        })
                        .await;
                }
            }
            *lock(&live.updates) = None;
        });
        receiver
    }

    /// A line sent while the agent works goes into its turn when the agent
    /// offered steering; otherwise it waits in the session for the next.
    fn steer(&self, text: String, attachments: Vec<Attachment>) -> bool {
        if self.check_capability().is_err() || self.live.cancelled.load(Ordering::SeqCst) {
            return false;
        }
        lock(&self.live.steering)
            .as_ref()
            .is_some_and(|lane| lane.admit.send((text, attachments)).is_ok())
    }

    fn take_unconsumed(&self) -> Vec<(String, Vec<Attachment>)> {
        std::mem::take(&mut *lock(&self.live.unconsumed))
    }

    fn cancel(&self) {
        self.live.cancelled.store(true, Ordering::SeqCst);
        let lane = lock(&self.live.steering).take();
        if let Some(lane) = lane {
            lane.stop();
        }
        self.live.settle_permissions();
        // Work the agent took up by itself has no prompt to answer the
        // cancel, so it is ended here, and what it was saying goes with it.
        let unprompted = {
            let mut updates = lock(&self.live.updates);
            lock(&self.live.unprompted_open)
                .take()
                .and_then(|_| updates.take())
        };
        if let Some(sender) = unprompted {
            lock(&self.live.open).take();
            let _ = sender.try_send(Update::Turn {
                stop_reason: "aborted".into(),
                usage: None,
            });
        }
        let Some(connection) = lock(&self.live.connection).clone() else {
            return;
        };
        let Some(session_id) = lock(&self.live.session).id.clone() else {
            return;
        };
        // A cancelled turn still answers `session/prompt`, so the turn's own
        // end is where the transcript is settled.
        let _ = connection.send_notification(CancelNotification::new(session_id));
    }

    fn checkpoint_valid(&self) -> bool {
        !self.live.failed.load(Ordering::SeqCst)
    }

    fn invalidate(&self) {
        if let Some(attempt) = lock(&self.live.auth).as_ref() {
            attempt.cancel.cancel();
        }
        if let Some(capability) = &self.capability {
            capability.revoke();
        }
        self.cancel();
        self.live.settle_permissions();
        lock(&self.live.updates).take();
        // Closing the watch wakes the room's metadata task even when the
        // session object is retained briefly by an in-flight turn.
        lock(&self.live.info_changes).take();
        lock(&self.live.connection).take();
        lock(&self.served).take();
        lock(&self.oauth_proxies).clear();
        lock(&self.oauth_refused).clear();
        self.kill_child();
    }

    async fn set_model(&self, model_id: &str) -> Result<DriverInfo, String> {
        self.check_capability()?;
        let config_id = lock(&self.live.session)
            .model_config
            .clone()
            .ok_or_else(|| "This agent does not offer a model picker.".to_string())?;
        self.set_config(&config_id, model_id).await
    }

    async fn set_config(&self, config_id: &str, value: &str) -> Result<DriverInfo, String> {
        self.check_capability()?;
        ChildAgent::set_config(self, config_id, value).await
    }

    async fn set_mode(&self, mode_id: &str) -> Result<DriverInfo, String> {
        self.check_capability()?;
        let connection = lock(&self.live.connection).clone();
        let (session_id, config_id) = {
            let session = lock(&self.live.session);
            (session.id.clone(), session.mode_config.clone())
        };
        if let Some(config_id) = config_id {
            return self.set_config(&config_id, mode_id).await;
        }
        let (Some(connection), Some(session_id)) = (connection, session_id) else {
            return Err("That agent is not connected.".to_string());
        };
        let mode = acp::SessionModeId::new(mode_id);
        connection
            .send_request(SetSessionModeRequest::new(session_id, mode))
            .block_task()
            .await
            .map_err(|error| format!("The mode could not be changed: {error}"))?;
        self.check_capability()?;
        lock(&self.live.session).info.current_mode_id = Some(mode_id.to_string());
        Ok(self.live.publish_info())
    }

    fn answer_permission(&self, request_id: &str, option_id: &str) -> bool {
        if self.check_capability().is_err() {
            return false;
        }
        let Some(waiting) = lock(&self.live.pending).remove(request_id) else {
            return false;
        };
        waiting.send(Some(option_id.to_string())).is_ok()
    }

    fn subscribe_info(&self) -> Option<watch::Receiver<DriverInfo>> {
        Some(self.live.subscribe_info())
    }

    fn subscribe_unprompted(&self) -> Option<mpsc::UnboundedReceiver<mpsc::Receiver<Update>>> {
        Some(self.live.subscribe_unprompted())
    }
    fn subscribe_subagents(&self) -> Option<mpsc::UnboundedReceiver<SubagentReport>> {
        Some(self.live.subscribe_subagents())
    }
}

// -- process plumbing -------------------------------------------------------

fn pump_stderr(live: Arc<Live>, stderr: tokio::process::ChildStderr) {
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut stderr = stderr;
        let mut bytes = [0; 4096];
        // Raw bytes go to a running sign-in, which may prompt without a
        // newline. Otherwise the tail keeps whole, non-blank lines, as before.
        let mut line = Vec::new();
        while let Ok(n) = stderr.read(&mut bytes).await {
            if n == 0 {
                break;
            }
            if let Some(attempt) = lock(&live.auth).as_ref().filter(|a| a.running()) {
                attempt.output(&bytes[..n]);
                line.clear();
                continue;
            }
            for &byte in &bytes[..n] {
                if byte != b'\n' && line.len() < 4096 {
                    line.push(byte);
                    continue;
                }
                if byte == b'\n' {
                    let text = String::from_utf8_lossy(&line).trim_end().to_string();
                    line.clear();
                    if text.trim().is_empty() {
                        continue;
                    }
                    let mut tail = lock(&live.stderr);
                    tail.push_back(text);
                    if tail.len() > STDERR_LINES {
                        tail.pop_front();
                    }
                }
            }
        }
    });
}

fn read_text_file(
    workspace: &Workspace,
    request: &ReadTextFileRequest,
) -> Result<String, agent_client_protocol::Error> {
    workspace
        .read_text_path(&request.path, request.line, request.limit)
        .map_err(agent_client_protocol::Error::into_internal_error)
}

fn write_text_file(
    workspace: &Workspace,
    request: &WriteTextFileRequest,
) -> Result<(), agent_client_protocol::Error> {
    workspace
        .write_text_path(&request.path, &request.content)
        .map_err(agent_client_protocol::Error::into_internal_error)
}

async fn fail(sender: &mpsc::Sender<Update>, text: String) {
    let text = super::failure::Failure::classify(&text, None, "acp_start").notice();
    let _ = sender
        .send(Update::Notice {
            level: NoticeLevel::Error,
            text,
        })
        .await;
    let _ = sender
        .send(Update::Turn {
            stop_reason: "failed".into(),
            usage: None,
        })
        .await;
}

fn lock<T>(held: &Mutex<T>) -> MutexGuard<'_, T> {
    held.lock().unwrap_or_else(PoisonError::into_inner)
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests;
