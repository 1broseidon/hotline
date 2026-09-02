//! The other driver: an external harness, run as a child process over the
//! Agent Client Protocol.
//!
//! Everything below the [`Driver`] seam is ACP's own vocabulary, so most of
//! this file is translation. The child is spawned in the teammate's working
//! directory, spoken to over its stdin and stdout, and told nothing about
//! Toad beyond what rides ahead of the first prompt — an ACP session has no
//! system-prompt parameter, so the two things Toad must say arrive elsewhere:
//!
//! - **who the teammate is** goes into `AGENTS.md` in its working directory,
//!   which every one of these agents reads (see [`materialize_agents_md`]);
//! - **what kind of room this is** rides as a content block ahead of the first
//!   thing the agent is ever told, because that is the earliest ACP will
//!   carry it.
//!
//! Toad holds no credentials here and cannot: these agents sign themselves in.
//! What it does hold is the conversation — the tape is Toad's — and the
//! agent's own memory of it, which is an opaque session id kept per backend on
//! the teammate's record and reopened with `session/load` or `session/resume`.
//!
//! One thing this driver does that the in-process one never does: it asks.
//! Permission requests arrive as [`Update::Permission`], become a card on the
//! tape, and block the agent until somebody answers. Whether the agent asks at
//! all is its own configuration and not Toad's — see [`containment_notice`].

pub mod registry;

use super::{Driver, DriverInfo, MessageKind, Update, clip};
use crate::contract::{
    Attachment, NoticeLevel, PermissionOption as CardOption, Persona, Reach, SessionCapabilities,
    TokenUsage,
};
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    self as acp, CancelNotification, ClientCapabilities, ContentBlock, FileSystemCapabilities,
    InitializeRequest, LoadSessionRequest, NewSessionRequest, PromptRequest, ReadTextFileRequest,
    ReadTextFileResponse, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, ResumeSessionRequest, SelectedPermissionOutcome, SessionId,
    SessionNotification, SessionUpdate, SetSessionConfigOptionRequest, SetSessionModeRequest,
    WriteTextFileRequest, WriteTextFileResponse,
};
use agent_client_protocol::{Agent, ByteStreams, Client, ConnectionTo, Responder};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

/// How many updates may be in flight before the connection waits for the room
/// to catch up. Deltas arrive faster than anything else, and a turn that
/// outran its reader would either grow without bound or lose text.
const UPDATE_DEPTH: usize = 256;

/// How long a permission card may wait for a person before the agent is told
/// nobody answered. A live request cannot wait on an absent human forever;
/// the agent gets its turn back and the card says it expired.
const PERMISSION_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// How many lines of the child's stderr are kept, to hang on the end of the
/// sentence when a turn fails. The tail is what says why.
const STDERR_LINES: usize = 20;

/// How much of a tool's title survives into the transcript line.
const TITLE_CHARS: usize = 120;

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

/// What Toad tells an agent about the room it is speaking in.
///
/// An agent's default register is the terminal: a headed report, bullets under
/// each heading, a summary of what it is about to do. That is the right shape
/// for a scrollback and the wrong shape for a conversation, and no agent can
/// know which one it is in unless it is told. This is a fact about Toad rather
/// than about the teammate, which is why it does not live in the teammate's
/// `AGENTS.md`: it is true of every teammate, and it has to arrive even when
/// the working directory is a real repository whose `AGENTS.md` Toad leaves
/// alone.
///
/// It asks for one acknowledgement before the work rather than banning one.
/// The typing indicator and "on it" do not say the same thing: dots mean
/// something is happening, "on it" means you were heard. And it says out loud
/// that brevity is about ceremony and not substance, because an agent told to
/// be short will otherwise shorten the explanation somebody asked for rather
/// than the packaging around it.
const HOUSE_STYLE: &str = "You are speaking in Toad, a desktop chat app. Your reply is shown as messages in a conversation, the way a person texts — not as a document.

There is a rhythm to that, and it matters more than anything else here. Before your first tool call, write one short line: \"on it\", \"let me check\", \"sure, one sec\". Then work in silence. Then say what came of it. The whole exchange should read like two colleagues — \"how many rust files are under crates?\" / \"let me check\" / \"41, all .rs\" — and never like one long report delivered after a minute of nothing. That opening line is not optional and it is not a summary of your plan; it is the word you would say to someone standing in your doorway.

After it, stay quiet until you have the answer. The person cannot see your tool calls, and a running commentary of what you are opening and what you found next is exactly what this app keeps off the screen.

Then say what came of it and stop. No recap of the steps, no list of the files you touched, no summary of what you just did. If it worked, saying so is enough; if it didn't, say what stopped you.

Write it the way you would text it. Lead with the answer. Plain sentences, no preamble, no restating the question, no sign-off. Keep paragraphs short: Toad sends each one as its own message, so two short messages read better than one dense block.

Being brief is about ceremony, not substance. A real question deserves a real answer — if someone asks how something works or why it broke, explain it properly. What gets cut is the packaging, never the thinking.

Formatting is available when the content is genuinely that shape — a fenced block for code, a list when there really are several items, a table when there are rows and columns, backticks for a filename or flag, bold for a term that carries weight. Headings render as plain bold text here, so they buy you very little; skip them unless a long reply truly needs a label. Reach for none of this to organise three sentences.";

/// The marker that says a file in a teammate's workspace is Toad's to rewrite.
const MANAGED_MARKER: &str = "<!-- managed by Toad -->";

/// Writes the teammate's identity where the agent will read it.
///
/// `session/new` has no system-prompt parameter, so identity has to arrive
/// through a channel the agent already reads, and `AGENTS.md` is that channel
/// — which is what makes the working directory part of the teammate rather
/// than bookkeeping.
///
/// Only a file Toad wrote is replaced, so a hand-written `AGENTS.md` in a real
/// repository is never clobbered. Toad's own files *open* with the marker, and
/// only an opening marker counts: a hand-written file that merely mentions it
/// — this repository's own does, to explain it — is not Toad's to replace.
pub fn materialize_agents_md(persona: &Persona) -> std::io::Result<()> {
    let directory = Path::new(&persona.cwd);
    std::fs::create_dir_all(directory)?;
    let file = directory.join("AGENTS.md");
    if let Ok(current) = std::fs::read_to_string(&file)
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
    std::fs::write(
        file,
        format!("{MANAGED_MARKER}\n# {}\n\n{body}\n", persona.name),
    )
}

/// Whether this backend will actually stop and ask before it acts, when Toad
/// can tell — and the sentence to say when it will not.
///
/// Toad draws permission cards, but it does not get to decide whether the
/// agent sends the requests. That is the backend's own configuration, and when
/// it is set to approve everything Toad's card simply never appears. A person
/// who thinks they are behind a gate that is not there should be told.
///
/// `None` for everything but Cursor, and that is the honest answer rather than
/// a gap: each agent keeps its approval policy in its own format, Toad can
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
    /// What the room would have made a system prompt of: who the teammate is,
    /// where it stands, and what happened in the chapter that closed. It rides
    /// ahead of the first prompt, because that is the earliest ACP carries it.
    preamble: String,
    live: Arc<Live>,
}

impl ChildAgent {
    pub fn new(root: PathBuf, backend_id: String, preamble: String) -> Self {
        Self {
            root,
            backend_id,
            preamble,
            live: Arc::new(Live::default()),
        }
    }
}

/// Everything one connection owns, shared with the handlers running on it.
#[derive(Default)]
struct Live {
    connection: Mutex<Option<ConnectionTo<Agent>>>,
    session: Mutex<Session>,
    /// Where the running turn's updates go. `None` between turns.
    updates: Mutex<Option<mpsc::Sender<Update>>>,
    /// The message being streamed, buffered so the tape gets whole messages.
    open: Mutex<Option<OpenMessage>>,
    /// The last state written for each tool call. An update carries only what
    /// changed, so without somewhere to merge into, a status change arrives as
    /// a payload of blanks and erases the title.
    tools: Mutex<HashMap<String, ToolLine>>,
    /// Permission requests waiting on a person, by the id their card carries.
    pending: Mutex<HashMap<String, oneshot::Sender<Option<String>>>>,
    /// `session/load` replays the whole history; nothing is written while it
    /// does, or every restart would duplicate the conversation onto the tape.
    replaying: AtomicBool,
    /// Whether this connection has been told what kind of room it is in.
    /// Per-connection, so a restarted backend hears it again and a resumed one
    /// does not hear it twice in the same conversation.
    briefed: AtomicBool,
    stderr: Mutex<VecDeque<String>>,
    /// The child, so dropping this driver ends the process.
    child: Mutex<Option<tokio::process::Child>>,
}

/// What the agent said about the conversation it opened.
#[derive(Default)]
struct Session {
    id: Option<SessionId>,
    info: DriverInfo,
    /// The config option ids the model and mode pickers came from, when they
    /// arrived as generic config options rather than as ACP's dedicated
    /// `modes` field. Switching then goes through `session/set_config_option`.
    model_config: Option<String>,
    mode_config: Option<String>,
}

struct OpenMessage {
    id: String,
    kind: MessageKind,
    text: String,
}

/// A tool call as the transcript last saw it.
struct ToolLine {
    title: String,
    kind: String,
    /// The first path the call named, which is the detail a permission answer
    /// usually turns on.
    location: Option<String>,
}

impl Live {
    /// Hands one update to the running turn. An update with no turn behind it
    /// is dropped, which is what a `session/update` arriving between turns is.
    async fn emit(&self, update: Update) {
        let sender = lock(&self.updates).clone();
        if let Some(sender) = sender {
            let _ = sender.send(update).await;
        }
    }

    /// Adds text to the message being streamed, opening one — and closing any
    /// message of the other kind — when the agent changes voice.
    async fn chunk(&self, kind: MessageKind, text: &str) {
        if text.is_empty() {
            return;
        }
        // The message of the other voice is closed first, so the tape never
        // holds a message that changed halfway through from speech to thought.
        let closing = {
            let mut open = lock(&self.open);
            let changed = open.as_ref().is_none_or(|message| message.kind != kind);
            changed.then(|| open.take()).flatten()
        };
        self.close(closing).await;
        let id = {
            let mut open = lock(&self.open);
            let message = open.get_or_insert_with(|| OpenMessage {
                id: new_id(),
                kind,
                text: String::new(),
            });
            message.text.push_str(text);
            message.id.clone()
        };
        self.emit(Update::Delta {
            kind,
            message_id: id,
            text: text.to_string(),
        })
        .await;
    }

    /// Closes the streamed message: one durable [`Update::Message`] holding
    /// everything its deltas carried.
    async fn flush(&self) {
        let open = lock(&self.open).take();
        self.close(open).await;
    }

    async fn close(&self, open: Option<OpenMessage>) {
        let Some(message) = open else { return };
        if message.text.is_empty() {
            return;
        }
        self.emit(Update::Message {
            kind: message.kind,
            id: message.id,
            text: message.text,
        })
        .await;
    }

    /// Answers every permission still waiting, which is what the end of a turn
    /// and the end of a session both are: nobody is behind those buttons now.
    fn settle_permissions(&self) {
        for (_, waiting) in lock(&self.pending).drain() {
            let _ = waiting.send(None);
        }
    }

    fn stderr_hint(&self) -> String {
        let tail: Vec<String> = lock(&self.stderr)
            .iter()
            .rev()
            .take(3)
            .rev()
            .cloned()
            .collect();
        let tail = tail.join(" ");
        if tail.trim().is_empty() {
            String::new()
        } else {
            format!(" The backend said: {tail}")
        }
    }
}

/// The child goes when the driver does, and takes whatever it started with it.
///
/// The process is its own group leader (see `start`), so this reaches the real
/// agent behind a wrapper launcher — `npx` spawning node, `uvx` spawning
/// python — where killing the immediate child would only orphan it.
impl Drop for Live {
    fn drop(&mut self) {
        let Ok(mut held) = self.child.lock() else {
            return;
        };
        let Some(child) = held.as_mut() else { return };
        #[cfg(unix)]
        if let Some(id) = child.id() {
            // Safety: `killpg` reads no memory, and the group is the one this
            // driver made for its own child.
            unsafe { libc::killpg(id as libc::pid_t, libc::SIGKILL) };
        }
        let _ = child.start_kill();
    }
}

#[async_trait]
impl Driver for ChildAgent {
    async fn start(&self, persona: &Persona) -> Result<DriverInfo, String> {
        let launch = registry::launch(&self.root, &self.backend_id)?;
        let mut command = tokio::process::Command::new(&launch.command);
        command
            .args(&launch.args)
            .current_dir(&persona.cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            // Windows has no process group to put it in, so the child itself
            // is what a drop can kill there.
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);

        let mut child = command
            .spawn()
            .map_err(|error| format!("Could not start {}: {error}", launch.command))?;
        let (stdin, stdout, stderr) =
            match (child.stdin.take(), child.stdout.take(), child.stderr.take()) {
                (Some(stdin), Some(stdout), Some(stderr)) => (stdin, stdout, stderr),
                _ => return Err(format!("{} gave no pipes to speak over.", launch.command)),
            };
        pump_stderr(self.live.clone(), stderr);
        *lock(&self.live.child) = Some(child);

        self.handshake(
            persona,
            ByteStreams::new(stdin.compat_write(), stdout.compat()),
        )
        .await
    }

    async fn prompt(
        &self,
        text: String,
        attachments: Vec<Attachment>,
        _reach: Reach,
    ) -> mpsc::Receiver<Update> {
        let (sender, receiver) = mpsc::channel(UPDATE_DEPTH);
        let Some(connection) = lock(&self.live.connection).clone() else {
            fail(&sender, "That agent is not connected.".to_string()).await;
            return receiver;
        };
        let Some(session_id) = lock(&self.live.session).id.clone() else {
            fail(&sender, "That agent has no open session.".to_string()).await;
            return receiver;
        };

        // The briefing rides along with the first thing said on this
        // connection, which is the earliest ACP will carry it. It is not
        // written to the tape: Toad explaining itself to an agent is
        // machinery, not conversation.
        let mut blocks = Vec::new();
        if !self.live.briefed.swap(true, Ordering::SeqCst) {
            blocks.push(text_block(&self.preamble));
            blocks.push(text_block(HOUSE_STYLE));
        }
        // Attachments lead the message the way they do in a mail client, and
        // travel as links: a coding agent already has the filesystem, and a
        // path costs nothing to send.
        for attachment in &attachments {
            blocks.push(ContentBlock::ResourceLink(acp::ResourceLink::new(
                attachment.name.clone(),
                file_uri(&attachment.path),
            )));
        }
        blocks.push(text_block(&text));

        *lock(&self.live.updates) = Some(sender.clone());
        let live = self.live.clone();
        tokio::spawn(async move {
            let answered = connection
                .send_request(PromptRequest::new(session_id, blocks))
                .block_task()
                .await;
            live.flush().await;
            match answered {
                Ok(response) => {
                    let _ = sender
                        .send(Update::Turn {
                            stop_reason: stop_reason_of(response.stop_reason),
                            usage: usage_of(response.usage.as_ref()),
                        })
                        .await;
                }
                Err(error) => {
                    let _ = sender
                        .send(Update::Notice {
                            level: NoticeLevel::Error,
                            text: format!("Turn failed: {error}{}", live.stderr_hint()),
                        })
                        .await;
                }
            }
            // A card whose turn is over is a button nobody is behind.
            live.settle_permissions();
            *lock(&live.updates) = None;
        });
        receiver
    }

    fn cancel(&self) {
        self.live.settle_permissions();
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

    async fn set_model(&self, model_id: &str) -> Result<DriverInfo, String> {
        let config_id = lock(&self.live.session)
            .model_config
            .clone()
            .ok_or_else(|| "This agent does not offer a model picker.".to_string())?;
        self.set_config(&config_id, model_id).await
    }

    async fn set_mode(&self, mode_id: &str) -> Result<DriverInfo, String> {
        let (connection, session_id, config_id) = {
            let session = lock(&self.live.session);
            (
                lock(&self.live.connection).clone(),
                session.id.clone(),
                session.mode_config.clone(),
            )
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
        let mut session = lock(&self.live.session);
        session.info.current_mode_id = Some(mode_id.to_string());
        Ok(session.info.clone())
    }

    fn answer_permission(&self, request_id: &str, option_id: &str) -> bool {
        let Some(waiting) = lock(&self.live.pending).remove(request_id) else {
            return false;
        };
        waiting.send(Some(option_id.to_string())).is_ok()
    }
}

impl ChildAgent {
    /// Everything after the child exists: the connection, the handshake, and
    /// the conversation this teammate is joining.
    ///
    /// Split from spawning because a pipe is a pipe. A test drives a scripted
    /// agent over an in-memory duplex through exactly this path, and what it
    /// proves about the translation is true of a real harness on stdio.
    pub(crate) async fn handshake(
        &self,
        persona: &Persona,
        transport: ByteStreams<
            impl futures_util::AsyncWrite + Send + 'static,
            impl futures_util::AsyncRead + Send + 'static,
        >,
    ) -> Result<DriverInfo, String> {
        let connection = connect(self.live.clone(), transport).await?;
        *lock(&self.live.connection) = Some(connection.clone());

        let initialized = connection
            .send_request(
                InitializeRequest::new(ProtocolVersion::V1).client_capabilities(
                    // Toad answers file reads and writes on the agent's behalf
                    // so an agent that expects an editor to own the files gets
                    // one; nothing here is a terminal, so none is offered.
                    ClientCapabilities::new().fs(FileSystemCapabilities::new()
                        .read_text_file(true)
                        .write_text_file(true)),
                ),
            )
            .block_task()
            .await
            .map_err(|error| format!("The agent refused to start: {error}"))?;

        let capabilities = capabilities_of(&initialized.agent_capabilities);
        {
            let mut session = lock(&self.live.session);
            session.info.agent_name = initialized
                .agent_info
                .as_ref()
                .map_or_else(|| self.backend_id.clone(), |info| info.name.clone());
            session.info.agent_version = initialized
                .agent_info
                .as_ref()
                .map(|info| info.version.clone());
            session.info.capabilities = capabilities;
        }

        self.open_session(&connection, persona, capabilities)
            .await?;
        self.adopt_disposition(persona).await;
        Ok(lock(&self.live.session).info.clone())
    }

    /// Reopens the agent's own memory of this conversation when it can, and
    /// otherwise opens a new one.
    ///
    /// A failed restore is an implementation detail as long as a session opens
    /// — what must never happen is claiming the agent remembers when it does
    /// not, which is what `context_restored` is for.
    async fn open_session(
        &self,
        connection: &ConnectionTo<Agent>,
        persona: &Persona,
        capabilities: SessionCapabilities,
    ) -> Result<(), String> {
        let cwd = PathBuf::from(&persona.cwd);
        let previous = persona
            .session_checkpoints
            .iter()
            .find(|checkpoint| checkpoint.backend_id == self.backend_id)
            .map(|checkpoint| checkpoint.session_id.clone());

        if let Some(previous) = previous {
            let id = SessionId::new(previous.as_str());
            if capabilities.resume {
                let resumed = connection
                    .send_request(ResumeSessionRequest::new(id.clone(), cwd.clone()))
                    .block_task()
                    .await;
                if let Ok(response) = resumed {
                    self.adopt(id, response.modes, response.config_options, true);
                    return Ok(());
                }
                // Some agents advertise both and can only resume particular
                // sessions; `session/load` is still a valid fallback.
            }
            if capabilities.load_session {
                self.live.replaying.store(true, Ordering::SeqCst);
                let loaded = connection
                    .send_request(LoadSessionRequest::new(id.clone(), cwd.clone()))
                    .block_task()
                    .await;
                self.live.replaying.store(false, Ordering::SeqCst);
                if let Ok(response) = loaded {
                    self.adopt(id, response.modes, response.config_options, true);
                    return Ok(());
                }
                // A stale or backend-invalid checkpoint degrades to a new
                // session, which is exactly what "Fresh" means.
            }
        }

        let opened = connection
            .send_request(NewSessionRequest::new(cwd))
            .block_task()
            .await
            .map_err(|error| format!("The agent would not open a session: {error}"))?;
        self.adopt(
            opened.session_id,
            opened.modes,
            opened.config_options,
            false,
        );
        Ok(())
    }

    fn adopt(
        &self,
        id: SessionId,
        modes: Option<acp::SessionModeState>,
        configs: Option<Vec<acp::SessionConfigOption>>,
        restored: bool,
    ) {
        lock(&self.live.tools).clear();
        let disposition = Disposition::of(modes.as_ref(), configs.as_deref());
        let mut session = lock(&self.live.session);
        session.id = Some(id.clone());
        session.model_config = disposition.model_config;
        session.mode_config = disposition.mode_config;
        session.info.session_id = Some(id.0.to_string());
        session.info.context_restored = restored;
        session.info.models = disposition.models;
        session.info.current_model_id = disposition.current_model_id.unwrap_or_default();
        session.info.model_label = disposition.model_label;
        session.info.modes = disposition.modes;
        session.info.current_mode_id = disposition.current_mode_id;
        session.info.mode_label = disposition.mode_label;
    }

    /// Puts the teammate's own model and mode back on.
    ///
    /// These are session-scoped for every agent Toad drives, so a teammate that
    /// is not asked again arrives on whatever the harness defaults to — which
    /// is a teammate quietly losing its disposition on every restart. A
    /// refusal is not fatal: the session is up either way, and the picker will
    /// show what the agent actually settled on.
    async fn adopt_disposition(&self, persona: &Persona) {
        let (mode, model) = {
            let session = lock(&self.live.session);
            (
                persona
                    .mode_id
                    .clone()
                    .filter(|id| Some(id) != session.info.current_mode_id.as_ref()),
                persona
                    .model_id
                    .clone()
                    .filter(|id| *id != session.info.current_model_id),
            )
        };
        if let Some(mode) = mode {
            let _ = self.set_mode(&mode).await;
        }
        if let Some(model) = model {
            let _ = self.set_model(&model).await;
        }
    }

    /// Sets one config option and takes the agent's whole answer back, since
    /// a change to one picker can move another.
    async fn set_config(&self, config_id: &str, value: &str) -> Result<DriverInfo, String> {
        let (connection, session_id) = (
            lock(&self.live.connection).clone(),
            lock(&self.live.session).id.clone(),
        );
        let (Some(connection), Some(session_id)) = (connection, session_id) else {
            return Err("That agent is not connected.".to_string());
        };
        let answered = connection
            .send_request(SetSessionConfigOptionRequest::new(
                session_id.clone(),
                acp::SessionConfigId::new(config_id),
                acp::SessionConfigValueId::new(value),
            ))
            .block_task()
            .await
            .map_err(|error| format!("The agent refused that setting: {error}"))?;
        self.apply_configs(&answered.config_options);
        Ok(lock(&self.live.session).info.clone())
    }

    fn apply_configs(&self, configs: &[acp::SessionConfigOption]) {
        let disposition = Disposition::of(None, Some(configs));
        let mut session = lock(&self.live.session);
        if disposition.model_config.is_some() {
            session.model_config = disposition.model_config;
            session.info.models = disposition.models;
            session.info.model_label = disposition.model_label;
            if let Some(current) = disposition.current_model_id {
                session.info.current_model_id = current;
            }
        }
        if disposition.mode_config.is_some() {
            session.mode_config = disposition.mode_config;
            session.info.modes = disposition.modes;
            session.info.mode_label = disposition.mode_label;
            session.info.current_mode_id = disposition.current_mode_id;
        }
    }
}

/// Starts the JSON-RPC connection on its own task and hands back the handle
/// every later call speaks through.
///
/// The SDK drives one connection from one future: handlers run on it, and the
/// closure it is given is what keeps it alive. So the closure does nothing but
/// pass the handle out and wait for the child's stdout to close, which is the
/// child exiting.
async fn connect(
    live: Arc<Live>,
    transport: ByteStreams<
        impl futures_util::AsyncWrite + Send + 'static,
        impl futures_util::AsyncRead + Send + 'static,
    >,
) -> Result<ConnectionTo<Agent>, String> {
    let (ready, started) = oneshot::channel();
    let updates = live.clone();
    let asked = live.clone();
    tokio::spawn(async move {
        let running = Client
            .builder()
            .name("Toad")
            .on_receive_notification(
                move |notification: SessionNotification, _cx| {
                    let live = updates.clone();
                    async move {
                        translate(&live, notification.update).await;
                        Ok(())
                    }
                },
                agent_client_protocol::on_receive_notification!(),
            )
            .on_receive_request(
                move |request: RequestPermissionRequest, responder, _cx| {
                    let live = asked.clone();
                    async move {
                        ask_permission(live, request, responder).await;
                        Ok(())
                    }
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |request: ReadTextFileRequest, responder, _cx| {
                    responder.respond(ReadTextFileResponse::new(read_text_file(&request)?))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |request: WriteTextFileRequest, responder, _cx| {
                    write_text_file(&request)?;
                    responder.respond(WriteTextFileResponse::new())
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_with(transport, async |cx: ConnectionTo<Agent>| {
                let _ = ready.send(cx.clone());
                cx.incoming_closed().await;
                Ok(())
            })
            .await;
        if let Err(error) = running {
            eprintln!("an ACP connection ended: {error}");
        }
        live.settle_permissions();
    });
    started
        .await
        .map_err(|_| "The agent's connection ended before it opened.".to_string())
}

/// One `session/update`, as the room's vocabulary sees it.
async fn translate(live: &Live, update: SessionUpdate) {
    if live.replaying.load(Ordering::SeqCst) {
        return;
    }
    match update {
        SessionUpdate::AgentMessageChunk(chunk) => {
            if let ContentBlock::Text(text) = chunk.content {
                live.chunk(MessageKind::Agent, &text.text).await;
            }
        }
        SessionUpdate::AgentThoughtChunk(chunk) => {
            if let ContentBlock::Text(text) = chunk.content {
                live.chunk(MessageKind::Thought, &text.text).await;
            }
        }
        SessionUpdate::ToolCall(call) => {
            live.flush().await;
            let line = ToolLine {
                title: clip(&call.title, TITLE_CHARS),
                kind: kind_of(call.kind),
                location: first_location(&call.locations),
            };
            live.emit(Update::ToolCall {
                call_id: call.tool_call_id.0.to_string(),
                title: line.title.clone(),
                kind: line.kind.clone(),
            })
            .await;
            let finished = finished(call.status, &call.content);
            lock(&live.tools).insert(call.tool_call_id.0.to_string(), line);
            if let Some((ok, output)) = finished {
                live.emit(Update::ToolResult {
                    call_id: call.tool_call_id.0.to_string(),
                    ok,
                    output,
                })
                .await;
            }
        }
        SessionUpdate::ToolCallUpdate(update) => {
            let call_id = update.tool_call_id.0.to_string();
            let fields = update.fields;
            // Absent means unchanged, so anything the update left out falls
            // back to what the call was announced with.
            let line = {
                let mut tools = lock(&live.tools);
                let previous = tools.get(&call_id);
                let line = ToolLine {
                    title: fields
                        .title
                        .as_deref()
                        .map(|title| clip(title, TITLE_CHARS))
                        .or_else(|| previous.map(|line| line.title.clone()))
                        .unwrap_or_default(),
                    kind: fields
                        .kind
                        .map(kind_of)
                        .or_else(|| previous.map(|line| line.kind.clone()))
                        .unwrap_or_default(),
                    location: fields
                        .locations
                        .as_deref()
                        .and_then(first_location)
                        .or_else(|| previous.and_then(|line| line.location.clone())),
                };
                let announced = ToolLine {
                    title: line.title.clone(),
                    kind: line.kind.clone(),
                    location: line.location.clone(),
                };
                tools.insert(call_id.clone(), line);
                announced
            };
            let content = fields.content.unwrap_or_default();
            match fields.status.and_then(|status| finished(status, &content)) {
                Some((ok, output)) => {
                    live.emit(Update::ToolResult {
                        call_id,
                        ok,
                        output,
                    })
                    .await;
                }
                // Still running: the line is written again so a title or a
                // path the agent only learned now reaches the transcript.
                None => {
                    live.emit(Update::ToolCall {
                        call_id,
                        title: line.title,
                        kind: line.kind,
                    })
                    .await;
                }
            }
        }
        // TranscriptEvent::Plan exists and the window does not draw it yet, so
        // the plan arrives as the one thing the window does draw. When the
        // window grows a plan panel this becomes an Update of its own.
        SessionUpdate::Plan(plan) => {
            live.flush().await;
            let lines: Vec<String> = plan
                .entries
                .iter()
                .map(|entry| format!("- {} ({:?})", entry.content, entry.status))
                .collect();
            if lines.is_empty() {
                return;
            }
            live.emit(Update::Notice {
                level: NoticeLevel::Info,
                text: format!("Plan:\n{}", lines.join("\n")),
            })
            .await;
        }
        SessionUpdate::CurrentModeUpdate(mode) => {
            lock(&live.session).info.current_mode_id = Some(mode.current_mode_id.0.to_string());
        }
        _ => {}
    }
}

/// The agent is asking to be allowed to do something. The card goes on the
/// tape now; the answer arrives later, over the wire, as its own command.
async fn ask_permission(
    live: Arc<Live>,
    request: RequestPermissionRequest,
    responder: Responder<RequestPermissionResponse>,
) {
    let request_id = new_id();
    let title = describe_request(&live, &request.tool_call);
    let options: Vec<CardOption> = request
        .options
        .iter()
        .map(|option| CardOption {
            option_id: option.option_id.0.to_string(),
            name: option.name.clone(),
            kind: serde_json::to_value(option.kind)
                .ok()
                .and_then(|kind| kind.as_str().map(str::to_string)),
        })
        .collect();

    let (answer, answered) = oneshot::channel();
    lock(&live.pending).insert(request_id.clone(), answer);
    live.flush().await;
    live.emit(Update::Permission {
        request_id,
        title,
        options,
    })
    .await;

    tokio::spawn(async move {
        let chosen = match tokio::time::timeout(PERMISSION_TIMEOUT, answered).await {
            Ok(Ok(chosen)) => chosen,
            _ => None,
        };
        let outcome = match chosen {
            Some(option_id) => RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                acp::PermissionOptionId::new(option_id.as_str()),
            )),
            None => RequestPermissionOutcome::Cancelled,
        };
        let _ = responder.respond(RequestPermissionResponse::new(outcome));
    });
}

/// What the agent is actually asking to be allowed to do.
///
/// The tool call on a permission request is a partial that points back at a
/// call already announced, so on its own it can carry nothing but an id and a
/// kind. "The agent is asking for permission" is not a question anyone can
/// answer, so this recovers the detail: the command if there is one, otherwise
/// the announced title, otherwise at least the kind of thing being attempted.
fn describe_request(live: &Live, call: &acp::ToolCallUpdate) -> String {
    let tools = lock(&live.tools);
    let known = tools.get(call.tool_call_id.0.as_ref());

    if let Some(command) = call
        .fields
        .raw_input
        .as_ref()
        .and_then(|input| input.get("command"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty())
    {
        return format!("Run {}", clip(command, TITLE_CHARS));
    }

    // Agents title an edit "Editing files" and leave which file to the
    // locations, which is the one detail the answer turns on.
    let where_ = call
        .fields
        .locations
        .as_deref()
        .and_then(first_location)
        .or_else(|| known.and_then(|line| line.location.clone()));
    let named = |what: String| match &where_ {
        Some(file) if !what.contains(file.as_str()) => format!("{what} — {file}"),
        _ => what,
    };

    let title = call
        .fields
        .title
        .clone()
        .or_else(|| known.map(|line| line.title.clone()))
        .filter(|title| !title.is_empty());
    if let Some(title) = title {
        return named(clip(&title, TITLE_CHARS));
    }

    let kind = call
        .fields
        .kind
        .map(kind_of)
        .or_else(|| known.map(|line| line.kind.clone()))
        .filter(|kind| !kind.is_empty());
    match kind {
        None => "The agent is asking for permission".to_string(),
        Some(kind) => named(format!(
            "Allow the agent to {}",
            PERMISSION_VERBS
                .iter()
                .find(|(name, _)| *name == kind)
                .map_or_else(|| format!("use {kind}"), |(_, verb)| (*verb).to_string())
        )),
    }
}

// -- translation helpers ----------------------------------------------------

/// The room's view of a tool call's disposition, or `None` while it is still
/// running.
fn finished(
    status: acp::ToolCallStatus,
    content: &[acp::ToolCallContent],
) -> Option<(bool, String)> {
    let ok = match status {
        acp::ToolCallStatus::Completed => true,
        acp::ToolCallStatus::Failed => false,
        _ => return None,
    };
    Some((ok, output_of(content)))
}

/// A tool's output as the transcript keeps it: the text it produced, and for
/// an edit the file and what it now says.
fn output_of(content: &[acp::ToolCallContent]) -> String {
    content
        .iter()
        .filter_map(|item| match item {
            acp::ToolCallContent::Content(inner) => match &inner.content {
                ContentBlock::Text(text) => Some(text.text.clone()),
                _ => None,
            },
            acp::ToolCallContent::Diff(diff) => {
                Some(format!("{}\n{}", diff.path.display(), diff.new_text))
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn first_location(locations: &[acp::ToolCallLocation]) -> Option<String> {
    locations
        .first()
        .and_then(|location| location.path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
}

/// An ACP enum as the string the wire spells it with, which is the string the
/// window's icons and the tape are keyed by.
fn kind_of(kind: acp::ToolKind) -> String {
    word_of(&kind)
}

fn stop_reason_of(reason: acp::StopReason) -> String {
    word_of(&reason)
}

fn word_of<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn usage_of(usage: Option<&acp::Usage>) -> Option<TokenUsage> {
    usage.map(|usage| TokenUsage {
        input_tokens: Some(usage.input_tokens as i64),
        output_tokens: Some(usage.output_tokens as i64),
        total_tokens: Some(usage.total_tokens as i64),
    })
}

fn capabilities_of(capabilities: &acp::AgentCapabilities) -> SessionCapabilities {
    SessionCapabilities {
        load_session: capabilities.load_session,
        resume: capabilities.session_capabilities.resume.is_some(),
        fork: capabilities.session_capabilities.fork.is_some(),
        mcp_http: capabilities.mcp_capabilities.http,
        image: capabilities.prompt_capabilities.image,
    }
}

/// The two pickers Toad draws, out of whichever shape the agent sent them in.
///
/// ACP has a dedicated `modes` field and a generic `configOptions` list, and
/// agents differ over which they use — Cursor sends modes, Claude Code's
/// adapter sends config options. Reading both here is what lets the header
/// stop caring which agent it is talking to.
#[derive(Default)]
struct Disposition {
    models: Vec<crate::contract::ConfigChoice>,
    current_model_id: Option<String>,
    model_config: Option<String>,
    model_label: Option<String>,
    modes: Vec<crate::contract::ConfigChoice>,
    current_mode_id: Option<String>,
    mode_config: Option<String>,
    mode_label: Option<String>,
}

impl Disposition {
    fn of(
        modes: Option<&acp::SessionModeState>,
        configs: Option<&[acp::SessionConfigOption]>,
    ) -> Self {
        let mut disposition = Self::default();
        if let Some(state) = modes {
            disposition.modes = state
                .available_modes
                .iter()
                .map(|mode| crate::contract::ConfigChoice {
                    id: mode.id.0.to_string(),
                    name: mode.name.clone(),
                    description: mode.description.clone(),
                    group: None,
                })
                .collect();
            disposition.current_mode_id = Some(state.current_mode_id.0.to_string());
        }
        for option in configs.unwrap_or_default() {
            let acp::SessionConfigKind::Select(select) = &option.kind else {
                continue;
            };
            let picker = choices_of(select);
            match option.category {
                Some(acp::SessionConfigOptionCategory::Model) => {
                    disposition.models = picker;
                    disposition.current_model_id = Some(select.current_value.0.to_string());
                    disposition.model_config = Some(option.id.0.to_string());
                    disposition.model_label = Some(option.name.clone());
                }
                Some(acp::SessionConfigOptionCategory::Mode)
                | Some(acp::SessionConfigOptionCategory::ThoughtLevel) => {
                    disposition.modes = picker;
                    disposition.current_mode_id = Some(select.current_value.0.to_string());
                    disposition.mode_config = Some(option.id.0.to_string());
                    disposition.mode_label = Some(option.name.clone());
                }
                _ => {}
            }
        }
        disposition
    }
}

fn choices_of(select: &acp::SessionConfigSelect) -> Vec<crate::contract::ConfigChoice> {
    let options = match &select.options {
        acp::SessionConfigSelectOptions::Ungrouped(options) => options.clone(),
        acp::SessionConfigSelectOptions::Grouped(groups) => groups
            .iter()
            .flat_map(|group| group.options.clone())
            .collect(),
        _ => Vec::new(),
    };
    options
        .into_iter()
        .map(|option| crate::contract::ConfigChoice {
            id: option.value.0.to_string(),
            name: option.name,
            description: option.description,
            group: None,
        })
        .collect()
}

// -- process plumbing -------------------------------------------------------

fn pump_stderr(live: Arc<Live>, stderr: tokio::process::ChildStderr) {
    tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut lines = tokio::io::BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.trim().is_empty() {
                continue;
            }
            let mut tail = lock(&live.stderr);
            tail.push_back(line);
            if tail.len() > STDERR_LINES {
                tail.pop_front();
            }
        }
    });
}

fn read_text_file(request: &ReadTextFileRequest) -> Result<String, agent_client_protocol::Error> {
    let text = std::fs::read_to_string(&request.path)
        .map_err(agent_client_protocol::Error::into_internal_error)?;
    if request.line.is_none() && request.limit.is_none() {
        return Ok(text);
    }
    let from = request.line.unwrap_or(1).saturating_sub(1) as usize;
    let lines: Vec<&str> = text.split('\n').skip(from).collect();
    let kept = match request.limit {
        Some(limit) => &lines[..lines.len().min(limit as usize)],
        None => &lines[..],
    };
    Ok(kept.join("\n"))
}

fn write_text_file(request: &WriteTextFileRequest) -> Result<(), agent_client_protocol::Error> {
    if let Some(directory) = request.path.parent() {
        std::fs::create_dir_all(directory)
            .map_err(agent_client_protocol::Error::into_internal_error)?;
    }
    std::fs::write(&request.path, &request.content)
        .map_err(agent_client_protocol::Error::into_internal_error)
}

async fn fail(sender: &mpsc::Sender<Update>, text: String) {
    let _ = sender
        .send(Update::Notice {
            level: NoticeLevel::Error,
            text,
        })
        .await;
}

fn text_block(text: &str) -> ContentBlock {
    ContentBlock::Text(acp::TextContent::new(text))
}

/// A path as the `file:` URI a resource link carries.
fn file_uri(path: &str) -> String {
    format!("file://{path}")
}

fn lock<T>(held: &Mutex<T>) -> MutexGuard<'_, T> {
    held.lock().unwrap_or_else(PoisonError::into_inner)
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{McpPolicy, PolicyMode, SessionCheckpoint};
    use agent_client_protocol::schema::v1::{
        AgentCapabilities, ContentChunk, Implementation, InitializeResponse, LoadSessionResponse,
        NewSessionResponse, PermissionOptionKind, PromptResponse, SessionMode, SessionModeState,
        StopReason, TextContent, ToolCall, ToolCallId, ToolCallStatus, ToolCallUpdate,
        ToolCallUpdateFields, ToolKind,
    };

    fn persona(cwd: &str, checkpoints: Vec<SessionCheckpoint>) -> Persona {
        Persona {
            node: None,
            id: "ada".to_string(),
            name: "Ada".to_string(),
            goal: "Keep the harbour running.".to_string(),
            face: None,
            team: None,
            backend_id: "cursor".to_string(),
            cwd: cwd.to_string(),
            reach: None,
            model_id: None,
            mode_id: None,
            harness_override: None,
            hop_notice: None,
            mcp_policy: McpPolicy {
                mode: PolicyMode::All,
                server_ids: Vec::new(),
            },
            web_search_policy: None,
            computer: None,
            subagents: None,
            session_checkpoints: checkpoints,
            last_session_id: None,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "toad-core-acp-{name}-{}-{}",
            std::process::id(),
            chrono::Local::now()
                .timestamp_nanos_opt()
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// What the scripted agent did, so a test can say what it was asked.
    #[derive(Clone, Default)]
    struct Heard {
        opened: Arc<Mutex<Vec<String>>>,
        prompted: Arc<Mutex<Vec<Vec<String>>>>,
    }

    /// An ACP agent that answers the handshake and then plays one turn: a
    /// spoken chunk, a tool call, a permission request it waits on, the tool's
    /// result, and the end of the turn.
    ///
    /// It is the crate's own agent side over an in-memory duplex, so what runs
    /// here is the same JSON on the same protocol a real harness would send.
    fn scripted_agent(
        heard: Heard,
        loadable: bool,
    ) -> impl std::future::Future<Output = ()> + Send + 'static {
        let (client_writer, agent_reader) = tokio::io::duplex(64 * 1024);
        let (agent_writer, client_reader) = tokio::io::duplex(64 * 1024);
        // The client half is handed back through the same channel the caller
        // reads, so a test only ever holds one thing.
        CLIENT_PIPES.with(|pipes| {
            *pipes.borrow_mut() = Some((client_writer, client_reader));
        });
        let transport = ByteStreams::new(agent_writer.compat_write(), agent_reader.compat());
        async move {
            let opened = heard.opened.clone();
            let loaded = heard.opened.clone();
            let prompted = heard.prompted.clone();
            let running = agent_client_protocol::Agent
                .builder()
                .name("scripted")
                .on_receive_request(
                    async move |request: InitializeRequest, responder, _cx| {
                        responder.respond(
                            InitializeResponse::new(request.protocol_version)
                                .agent_capabilities(AgentCapabilities::new().load_session(loadable))
                                .agent_info(Implementation::new("scripted", "1.2.3")),
                        )
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    move |_request: NewSessionRequest,
                          responder: Responder<NewSessionResponse>,
                          _cx| {
                        let opened = opened.clone();
                        async move {
                            opened.lock().unwrap().push("session/new".to_string());
                            responder.respond(
                                NewSessionResponse::new(SessionId::new("fresh-session")).modes(
                                    SessionModeState::new(
                                        acp::SessionModeId::new("ask"),
                                        vec![
                                            SessionMode::new(acp::SessionModeId::new("ask"), "Ask"),
                                            SessionMode::new(
                                                acp::SessionModeId::new("agent"),
                                                "Agent",
                                            ),
                                        ],
                                    ),
                                ),
                            )
                        }
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    move |request: LoadSessionRequest,
                          responder: Responder<LoadSessionResponse>,
                          cx: ConnectionTo<Client>| {
                        let loaded = loaded.clone();
                        async move {
                            loaded
                                .lock()
                                .unwrap()
                                .push(format!("session/load {}", request.session_id.0));
                            // A load replays the conversation; nothing it says
                            // may reach the tape a second time.
                            cx.send_notification(SessionNotification::new(
                                request.session_id.clone(),
                                SessionUpdate::AgentMessageChunk(ContentChunk::new(
                                    ContentBlock::Text(TextContent::new("replayed")),
                                )),
                            ))?;
                            responder.respond(LoadSessionResponse::new())
                        }
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    move |request: PromptRequest,
                          responder: Responder<PromptResponse>,
                          cx: ConnectionTo<Client>| {
                        let prompted = prompted.clone();
                        async move {
                            prompted.lock().unwrap().push(
                                request
                                    .prompt
                                    .iter()
                                    .map(|block| match block {
                                        ContentBlock::Text(text) => text.text.clone(),
                                        ContentBlock::ResourceLink(link) => link.uri.clone(),
                                        _ => "?".to_string(),
                                    })
                                    .collect(),
                            );
                            let session = request.session_id.clone();
                            let say = |update| {
                                cx.send_notification(SessionNotification::new(
                                    session.clone(),
                                    update,
                                ))
                            };
                            say(SessionUpdate::AgentMessageChunk(ContentChunk::new(
                                ContentBlock::Text(TextContent::new("on it")),
                            )))?;
                            say(SessionUpdate::ToolCall(
                                ToolCall::new(ToolCallId::new("c1"), "read harbour.log")
                                    .kind(ToolKind::Read)
                                    .status(ToolCallStatus::InProgress),
                            ))?;
                            let asking = cx.clone();
                            cx.spawn(async move {
                                let answer = asking
                                    .send_request(RequestPermissionRequest::new(
                                        session.clone(),
                                        ToolCallUpdate::new(
                                            ToolCallId::new("c1"),
                                            ToolCallUpdateFields::new(),
                                        ),
                                        vec![
                                            acp::PermissionOption::new(
                                                "once",
                                                "Allow once",
                                                PermissionOptionKind::AllowOnce,
                                            ),
                                            acp::PermissionOption::new(
                                                "never",
                                                "Deny",
                                                PermissionOptionKind::RejectOnce,
                                            ),
                                        ],
                                    ))
                                    .block_task()
                                    .await?;
                                let allowed =
                                    matches!(answer.outcome, RequestPermissionOutcome::Selected(_));
                                asking.send_notification(SessionNotification::new(
                                    session,
                                    SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                                        ToolCallId::new("c1"),
                                        ToolCallUpdateFields::new()
                                            .status(if allowed {
                                                ToolCallStatus::Completed
                                            } else {
                                                ToolCallStatus::Failed
                                            })
                                            .content(vec![acp::ToolCallContent::Content(
                                                acp::Content::new(ContentBlock::Text(
                                                    TextContent::new("all clear"),
                                                )),
                                            )]),
                                    )),
                                ))?;
                                responder.respond(PromptResponse::new(StopReason::EndTurn))
                            })?;
                            Ok(())
                        }
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_to(transport)
                .await;
            if let Err(error) = running {
                eprintln!("the scripted agent ended: {error}");
            }
        }
    }

    thread_local! {
        static CLIENT_PIPES: std::cell::RefCell<Option<(tokio::io::DuplexStream, tokio::io::DuplexStream)>> =
            const { std::cell::RefCell::new(None) };
    }

    fn client_transport() -> ByteStreams<
        impl futures_util::AsyncWrite + Send + 'static,
        impl futures_util::AsyncRead + Send + 'static,
    > {
        let (writer, reader) = CLIENT_PIPES
            .with(|pipes| pipes.borrow_mut().take())
            .expect("the scripted agent was built first");
        ByteStreams::new(writer.compat_write(), reader.compat())
    }

    async fn next(updates: &mut mpsc::Receiver<Update>) -> Update {
        tokio::time::timeout(Duration::from_secs(5), updates.recv())
            .await
            .expect("the turn stalled")
            .expect("the turn ended early")
    }

    /// One turn end to end: what the agent streams becomes the room's
    /// vocabulary, in the order it happened, and the permission in the middle
    /// blocks the tool until it is answered.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_turn_becomes_updates_in_the_order_the_agent_sent_them() {
        let root = scratch("turn");
        let heard = Heard::default();
        let agent = scripted_agent(heard.clone(), false);
        let driver = ChildAgent::new(root, "cursor".to_string(), "you are Ada".to_string());
        tokio::spawn(agent);

        let info = driver
            .handshake(&persona("/tmp", Vec::new()), client_transport())
            .await
            .unwrap();
        assert_eq!(info.agent_name, "scripted");
        assert_eq!(info.agent_version.as_deref(), Some("1.2.3"));
        assert_eq!(info.session_id.as_deref(), Some("fresh-session"));
        assert!(!info.context_restored);
        assert_eq!(
            info.modes
                .iter()
                .map(|mode| mode.id.as_str())
                .collect::<Vec<_>>(),
            ["ask", "agent"]
        );
        assert_eq!(info.current_mode_id.as_deref(), Some("ask"));
        assert_eq!(heard.opened.lock().unwrap().as_slice(), ["session/new"]);

        let mut updates = driver
            .prompt(
                "how is the harbour".to_string(),
                Vec::new(),
                Reach::Workspace,
            )
            .await;

        assert!(matches!(next(&mut updates).await, Update::Delta { text, .. } if text == "on it"));
        assert!(
            matches!(next(&mut updates).await, Update::Message { text, kind, .. }
                if text == "on it" && kind == MessageKind::Agent)
        );
        let Update::ToolCall {
            call_id,
            title,
            kind,
        } = next(&mut updates).await
        else {
            panic!("the tool call did not arrive");
        };
        assert_eq!(
            (call_id.as_str(), title.as_str(), kind.as_str()),
            ("c1", "read harbour.log", "read")
        );

        let Update::Permission {
            request_id,
            options,
            ..
        } = next(&mut updates).await
        else {
            panic!("the permission did not arrive");
        };
        assert_eq!(
            options
                .iter()
                .map(|option| option.name.as_str())
                .collect::<Vec<_>>(),
            ["Allow once", "Deny"]
        );
        assert!(driver.answer_permission(&request_id, "once"));
        // Answered once and only once: the second answer has nothing behind it.
        assert!(!driver.answer_permission(&request_id, "once"));

        assert!(matches!(
            next(&mut updates).await,
            Update::ToolResult { ok: true, output, .. } if output == "all clear"
        ));
        assert!(matches!(
            next(&mut updates).await,
            Update::Turn { stop_reason, .. } if stop_reason == "end_turn"
        ));

        // The briefing rides ahead of the first words and is never said again.
        let prompted = heard.prompted.lock().unwrap().clone();
        assert_eq!(prompted[0][0], "you are Ada");
        assert!(prompted[0][1].starts_with("You are speaking in Toad"));
        assert_eq!(prompted[0][2], "how is the harbour");
    }

    /// A teammate with a checkpoint for this backend rejoins its own session,
    /// and the history the load replays is not written down a second time.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_checkpoint_reopens_the_agents_own_session_without_replaying_it() {
        let root = scratch("load");
        let heard = Heard::default();
        let agent = scripted_agent(heard.clone(), true);
        let driver = ChildAgent::new(root, "cursor".to_string(), "you are Ada".to_string());
        tokio::spawn(agent);

        let ada = persona(
            "/tmp",
            vec![SessionCheckpoint {
                backend_id: "cursor".to_string(),
                session_id: "old-session".to_string(),
            }],
        );
        let info = driver.handshake(&ada, client_transport()).await.unwrap();
        assert_eq!(info.session_id.as_deref(), Some("old-session"));
        assert!(info.context_restored);
        assert_eq!(
            heard.opened.lock().unwrap().as_slice(),
            ["session/load old-session"]
        );

        // Nothing the replay said reached a turn: the first update of the
        // first turn is that turn's own first word.
        let mut updates = driver
            .prompt("carry on".to_string(), Vec::new(), Reach::Workspace)
            .await;
        assert!(matches!(next(&mut updates).await, Update::Delta { text, .. } if text == "on it"));
    }

    /// Toad rewrites only the file it wrote. A hand-written AGENTS.md — even
    /// one that merely mentions the marker, as this repository's own does — is
    /// left exactly as it was.
    #[test]
    fn only_a_file_opening_with_the_marker_is_toads_to_replace() {
        let root = scratch("agents-md");
        let mut ada = persona(&root.to_string_lossy(), Vec::new());
        let file = root.join("AGENTS.md");

        materialize_agents_md(&ada).unwrap();
        let written = std::fs::read_to_string(&file).unwrap();
        assert!(written.starts_with(MANAGED_MARKER));
        assert!(written.contains("# Ada"));
        assert!(written.contains("Keep the harbour running."));

        // Toad's own file is rewritten when the goal moves.
        ada.goal = "Count the boats.".to_string();
        materialize_agents_md(&ada).unwrap();
        assert!(
            std::fs::read_to_string(&file)
                .unwrap()
                .contains("Count the boats.")
        );

        let by_hand =
            format!("# A real repository\n\nIt explains {MANAGED_MARKER} in a sentence.\n");
        std::fs::write(&file, &by_hand).unwrap();
        ada.goal = "Something else entirely.".to_string();
        materialize_agents_md(&ada).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), by_hand);
    }
}
