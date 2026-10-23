mod accounts;
mod config;
mod oauth_proxy;
mod session_new;
mod steering;
mod turns;
mod unprompted;

use super::oauth_proxy::{OAuthProxyState, ProxyCredential, oauth_proxy_request};
use super::session::{Disposition, RawInitialize};
use super::translate::{SUBAGENT_LAUNCH, finished, tool_kind, translate};
use super::*;
use crate::accounts::resolve::ResolvedAccount;
use crate::accounts::support::CLAUDE;
use crate::contract::SkillPolicy;
use crate::contract::{McpPolicy, PolicyMode, SessionCheckpoint};
use agent_client_protocol::schema::v1::SessionNotification;
use agent_client_protocol::schema::v1::{
    AgentCapabilities, ContentChunk, Implementation, InitializeResponse, LoadSessionResponse,
    NewSessionResponse, PermissionOptionKind, PromptResponse, SessionMode, SessionModeState,
    StopReason, TextContent, ToolCall, ToolCallId, ToolCallStatus, ToolCallUpdate,
    ToolCallUpdateFields, ToolKind,
};
use rmcp::ServiceExt;
use std::sync::atomic::AtomicUsize;

/// A desk with no provider key: nothing in these tests reaches a model.
struct NoKeys;

impl crate::session::ProviderKeys for NoKeys {
    fn provider_auth(&self) -> HashMap<String, crate::session::ProviderAuth> {
        HashMap::new()
    }
}

/// A room the driver's teammate tools point back at. Held by the test,
/// because the tools hold it weakly.
pub(super) fn room(name: &str) -> Arc<crate::session::Room> {
    crate::session::Room::new(crate::log::Log::open(scratch(name)), Arc::new(NoKeys))
}

/// A directory that exists on every platform; `/tmp` is not one on Windows.
pub(super) fn scratch_cwd() -> String {
    std::env::temp_dir().to_string_lossy().into_owned()
}

pub(super) fn persona(cwd: &str, checkpoints: Vec<SessionCheckpoint>) -> Persona {
    Persona {
        node: None,
        id: "ada".to_string(),
        name: "Ada".to_string(),
        goal: "Keep the harbour running.".to_string(),
        avatar: None,
        team: None,
        backend_id: "cursor".to_string(),
        cwd: cwd.to_string(),
        reach: None,
        model_id: None,
        mode_id: None,
        effort_id: None,
        harness_override: None,
        account_id: None,
        hop_notice: None,
        mcp_policy: McpPolicy {
            mode: PolicyMode::All,
            server_ids: Vec::new(),
        },
        skill_policy: SkillPolicy::default(),
        background_work: false,
        allowed_senders: Vec::new(),
        web_search_policy: None,
        folders: None,
        computer: None,
        voice: None,
        session_checkpoints: checkpoints,
        last_session_id: None,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
    }
}

pub(super) fn resolved(id: &str, dir: PathBuf) -> ResolvedAccount {
    ResolvedAccount {
        id: id.to_string(),
        backend_id: "claude-acp".to_string(),
        name: "Account 2".to_string(),
        dir,
        support: &CLAUDE,
    }
}

pub(super) fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "hotline-core-acp-{name}-{}-{}",
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
    /// The MCP servers the session was opened with, which is the only
    /// place a child ever hears about them.
    servers: Arc<Mutex<Vec<acp::McpServer>>>,
    /// Whether this agent says it takes `additionalDirectories`, and
    /// the ones its session was opened with.
    takes_directories: bool,
    directories: Arc<Mutex<Vec<PathBuf>>>,
}

/// The agent half of an in-memory duplex, with the client half left where
/// [`client_transport`] will find it: a test only ever holds one thing.
///
/// Every scripted agent below is the crate's own agent side over one of
/// these, so what runs is the same JSON on the same protocol a real
/// harness would send.
pub(super) fn agent_pipes() -> ByteStreams<
    impl futures_util::AsyncWrite + Send + 'static,
    impl futures_util::AsyncRead + Send + 'static,
> {
    let (client_writer, agent_reader) = tokio::io::duplex(64 * 1024);
    let (agent_writer, client_reader) = tokio::io::duplex(64 * 1024);
    CLIENT_PIPES.with(|pipes| {
        *pipes.borrow_mut() = Some((client_writer, client_reader));
    });
    ByteStreams::new(agent_writer.compat_write(), agent_reader.compat())
}

/// An ACP agent that answers the handshake and then plays one turn: a
/// spoken chunk, a tool call, a permission request it waits on, the tool's
/// result, and the end of the turn.
fn scripted_agent(
    heard: Heard,
    loadable: bool,
) -> impl std::future::Future<Output = ()> + Send + 'static {
    let transport = agent_pipes();
    async move {
        let opened = heard.opened.clone();
        let loaded = heard.opened.clone();
        let prompted = heard.prompted.clone();
        let servers = heard.servers.clone();
        let directories = heard.directories.clone();
        let takes_directories = heard.takes_directories;
        let running = agent_client_protocol::Agent
            .builder()
            .name("scripted")
            .on_receive_request(
                async move |request: InitializeRequest, responder, _cx| {
                    let mut capabilities = AgentCapabilities::new().load_session(loadable);
                    if takes_directories {
                        capabilities = capabilities.session_capabilities(
                            acp::SessionCapabilities::new().additional_directories(
                                acp::SessionAdditionalDirectoriesCapabilities::new(),
                            ),
                        );
                    }
                    responder.respond(
                        InitializeResponse::new(request.protocol_version)
                            .agent_capabilities(capabilities)
                            .agent_info(Implementation::new("scripted", "1.2.3")),
                    )
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                move |request: NewSessionRequest, responder: Responder<NewSessionResponse>, _cx| {
                    let opened = opened.clone();
                    let servers = servers.clone();
                    let directories = directories.clone();
                    async move {
                        opened.lock().unwrap().push("session/new".to_string());
                        *servers.lock().unwrap() = request.mcp_servers.clone();
                        *directories.lock().unwrap() = request.additional_directories.clone();
                        responder.respond(
                            NewSessionResponse::new(SessionId::new("fresh-session")).modes(
                                SessionModeState::new(
                                    acp::SessionModeId::new("ask"),
                                    vec![
                                        SessionMode::new(acp::SessionModeId::new("ask"), "Ask"),
                                        SessionMode::new(acp::SessionModeId::new("agent"), "Agent"),
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
                            cx.send_notification(SessionNotification::new(session.clone(), update))
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

pub(super) fn client_transport() -> ByteStreams<
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

async fn turn_end(updates: &mut mpsc::Receiver<Update>) -> String {
    loop {
        if let Update::Turn { stop_reason, .. } = next(updates).await {
            return stop_reason;
        }
    }
}
