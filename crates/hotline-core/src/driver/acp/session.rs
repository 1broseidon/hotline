//! Starting the child and opening its session: the spawn, the ACP handshake,
//! `session/new` or a reopened one, the MCP servers it is handed, and the
//! configuration and disposition it reports back.

use super::*;
use crate::accounts::env::env_plan;
use crate::accounts::resolve::AccountRefusal;
use crate::driver::failure::Failure;
use crate::{paths, private_dir};

impl ChildAgent {
    /// Starts the child in a process group of its own and runs the handshake
    /// over its pipes. [`Driver::start`] bounds the whole of it.
    pub(super) async fn spawn(
        &self,
        persona: &Persona,
        launch: &registry::Launch,
    ) -> Result<DriverInfo, String> {
        let mut command = tokio::process::Command::new(&launch.command);
        env_plan(&launch.env, &[], self.account.as_ref()).apply(&mut command);
        command
            .args(&launch.args)
            .current_dir(&persona.cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);

        #[cfg(windows)]
        crate::process_windows::prepare(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| format!("Could not start {}: {error}", launch.command))?;
        #[cfg(unix)]
        if let Some(id) = child.id() {
            lock(&GROUPS).push(id as libc::pid_t);
        }
        #[cfg(windows)]
        let job = crate::process_windows::Job::attach(child.id())
            .map_err(|error| format!("Could not contain the agent process tree: {error}"))?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            end_group(&child);
            return Err(format!("{} gave no pipes to speak over.", launch.command));
        };
        pump_stderr(self.live.clone(), stderr);
        *lock(&self.child) = Some(child);
        #[cfg(windows)]
        {
            *lock(&self.job) = Some(job);
        }

        self.handshake(
            persona,
            ByteStreams::new(stdin.compat_write(), stdout.compat()),
        )
        .await
    }

    /// A start that ran out its bound: the process group is killed, and the
    /// teammate comes up failed, with the sentence on its tape as an error
    /// card, the way an expired sign-in does. The next message starts it
    /// again, which is the retry.
    pub(super) fn stalled(&self, launch: &registry::Launch, bound: Duration) -> DriverInfo {
        self.kill_child();
        lock(&self.live.connection).take();
        registry::mark_started(&self.root, launch, false);
        let name = registry::known(&self.root, &self.backend_id)
            .map_or_else(|| self.backend_id.clone(), |backend| backend.name);
        let sentence = format!(
            "{name} didn't start within {} ({}). Check that the command runs in a terminal.{}",
            spoken(bound),
            launch.line(),
            self.live.stderr_hint()
        );
        let failure = crate::driver::failure::Failure::startup(sentence);
        // The details, which are redacted: the stderr tail is the agent's.
        eprintln!("[acp] {}", failure.details);
        *lock(&self.live.startup_failure) = Some(failure.notice());
        self.live.failed.store(true, Ordering::SeqCst);
        {
            let mut session = lock(&self.live.session);
            session.id = None;
            session.info.session_id = None;
        }
        self.live.publish_info()
    }

    /// The account's folder, checked again before every spawn: one swapped
    /// for a link since the driver was built would send the sign-in
    /// somewhere else. A refusal comes up failed the way [`Self::stalled`]
    /// does, in the public sentence, since the tape is not only the desk's.
    pub(super) fn refuse_unavailable_folder(&self, persona: &Persona) -> Option<DriverInfo> {
        let account = self.account.as_ref()?;
        let error = private_dir::checked_account_dir(&self.root, &account.id).err()?;
        let refusal = AccountRefusal::FolderUnavailable {
            account_id: account.id.clone(),
            name: account.name.clone(),
            path: paths::agent_account_dir(&self.root, &account.id)
                .unwrap_or_else(|| account.dir.clone()),
            kind: error.kind(),
        };
        let agent = registry::name_of(&self.root, &self.backend_id)
            .unwrap_or_else(|| self.backend_id.clone());
        let sentence = refusal.public_sentence(&persona.name, &agent);
        let failure = Failure::account_unavailable(&refusal, &agent, sentence, "acp_start");
        *lock(&self.live.startup_failure) = Some(failure.notice());
        self.live.failed.store(true, Ordering::SeqCst);
        {
            let mut session = lock(&self.live.session);
            session.id = None;
            session.info.session_id = None;
        }
        Some(self.live.publish_info())
    }

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
        self.check_capability()?;
        *lock(&self.persona) = Some(persona.clone());
        // ACP owns its child process and whatever permissions that process
        // advertises. Hotline still answers the protocol's file callbacks, and
        // that callback plane is always confined to the teammate workspace;
        // an old stored machine reach and an ACP mode string cannot widen it.
        let callback_workspace = Self::callback_workspace(persona, self.teammate.capability())?;
        let connection = connect(self.live.clone(), callback_workspace, transport).await?;
        self.check_capability()?;
        *lock(&self.live.connection) = Some(connection.clone());

        // Hotline's own tools go up before the session does, because the session
        // is where the child is told where to find them.
        let serving = self.open_hotline_endpoint().await;
        self.check_capability()?;
        self.prepare_oauth_proxies().await;
        self.check_capability()?;

        let initialized = connection
            .send_request(initialize_request())
            .block_task()
            .await;
        let initialized = match initialized {
            Ok(initialized) => initialized,
            Err(error) => {
                // A child that closes its pipe at the handshake has usually
                // said why on stderr — npm, a missing binary, a crash — and
                // the pump reads that a beat behind the close. The tail is
                // what tells the person what happened; "transport closed"
                // tells them nothing.
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                return Err(format!(
                    "The agent refused to start: {error}.{}",
                    self.live.stderr_hint()
                ));
            }
        };
        self.check_capability()?;

        lock(&self.live.auth_methods).clone_from(&initialized.auth_methods);
        *lock(&self.live.startup_failure) = None;
        let capabilities = capabilities_of(&initialized);
        self.live
            .steerable
            .store(capabilities.active_input, Ordering::SeqCst);
        {
            let mut session = lock(&self.live.session);
            // A person reads this name, so a backend Hotline lists goes by
            // the name it is listed under, and anything else by the title it
            // reports before its name: an adapter's name is its npm package,
            // `@agentclientprotocol/claude-agent-acp`.
            session.info.agent_name = registry::known(&self.root, &self.backend_id)
                .map(|backend| backend.name)
                .or_else(|| {
                    initialized.agent_info.as_ref().map(|info| {
                        info.title
                            .clone()
                            .filter(|title| !title.trim().is_empty())
                            .unwrap_or_else(|| info.name.clone())
                    })
                })
                .unwrap_or_else(|| self.backend_id.clone());
            session.info.agent_version = initialized
                .agent_info
                .as_ref()
                .map(|info| info.version.clone());
            session.info.capabilities = capabilities;
        }

        // Written before the session, because the session is where the child
        // is handed the endpoint: a child that lists Hotline's tools while
        // `session/new` is still in flight promotes rows that have to exist by
        // then, and a ledger published afterwards would overwrite what was
        // watched with "declared". An agent that refused to initialize was
        // given nothing and still gets no ledger at all; one that initialized
        // and then refused `session/new` keeps the ledger it was handed,
        // because the rows were declared to it whether or not it went on.
        if !self.teammate.in_work() {
            self.publish_ledger(persona, serving);
        }
        if let Err(error) = self.open_session(&connection, persona, capabilities).await {
            let failure = auth::failure(
                &self.live,
                &error,
                "",
                "acp_start",
                self.usage_limit_prefixes(),
                self.account_id(),
            );
            if failure.kind == crate::driver::failure::Kind::AgentAuth
                && (failure.sign_in.is_some() || failure.account_id.is_some())
            {
                *lock(&self.live.startup_failure) = Some(failure.notice());
                self.live.failed.store(true, Ordering::SeqCst);
                lock(&self.live.session).id = None;
                lock(&self.live.session).info.session_id = None;
                return Ok(self.live.publish_info());
            }
            return Err(error);
        }
        self.check_capability()?;
        self.adopt_disposition(persona).await;
        self.check_capability()?;
        // Supersede notifications received during session restore with the
        // final handshake state before the room attaches its watcher.
        Ok(self.live.publish_info())
    }

    /// Builds the client side of ACP's file callback boundary. The harness is
    /// externally trusted, but Hotline's own callbacks stay in the workspace and
    /// the folders the person granted, read-only unless a grant says it may be
    /// changed, for every ACP persona, including records that still carry the
    /// old machine reach value. Runtime ACP mode names never affect this choice.
    pub(super) fn callback_workspace(
        persona: &Persona,
        capability: Option<CapabilityLease>,
    ) -> Result<Workspace, String> {
        Workspace::open_with_capability(
            PathBuf::from(&persona.cwd),
            Reach::Workspace,
            PathBuf::from(&persona.cwd).join(".hotline-tool-output"),
            persona.folders.as_deref().unwrap_or_default(),
            capability,
        )
        .map_err(|error| format!("The ACP callback workspace could not be opened: {error}"))
    }

    /// Reopens the agent's own memory of this conversation when it can, and
    /// otherwise opens a new one.
    ///
    /// A failed restore is an implementation detail as long as a session opens
    /// — what must never happen is claiming the agent remembers when it does
    /// not, which is what `context_restored` is for.
    pub(super) async fn open_session(
        &self,
        connection: &ConnectionTo<Agent>,
        persona: &Persona,
        capabilities: SessionCapabilities,
    ) -> Result<(), String> {
        let cwd = PathBuf::from(&persona.cwd);
        // The person's extra folders, handed to a harness that says it takes
        // them. Hotline does not enforce what the harness does with them; it
        // enforces only its own file callbacks (see `callback_workspace`).
        let folders = if capabilities.additional_directories {
            granted_directories(persona)
        } else {
            Vec::new()
        };
        let previous = persona
            .session_checkpoints
            .iter()
            .find(|checkpoint| {
                checkpoint.backend_id == self.backend_id
                    && checkpoint.account_id.as_deref() == self.account_id()
            })
            .map(|checkpoint| checkpoint.session_id.clone());

        if let Some(previous) = previous {
            let id = SessionId::new(previous.as_str());
            if capabilities.resume {
                let resumed = connection
                    .send_request(
                        ResumeSessionRequest::new(id.clone(), cwd.clone())
                            .additional_directories(folders.clone())
                            .mcp_servers(self.declared_servers()),
                    )
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
                    .send_request(
                        LoadSessionRequest::new(id.clone(), cwd.clone())
                            .additional_directories(folders.clone())
                            .mcp_servers(self.declared_servers()),
                    )
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
            .send_request(
                NewSessionRequest::new(cwd)
                    .additional_directories(folders)
                    .mcp_servers(self.declared_servers()),
            )
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

    /// Puts Hotline's own tools on a loopback port for this child, answering
    /// with why it could not when it could not.
    ///
    /// A child is a separate process, so there is no way to hand it a
    /// function: the same handler Hotline Agent calls directly is served over
    /// HTTP, and the token is this session's alone.
    async fn open_hotline_endpoint(&self) -> Result<(), String> {
        match mcp::server::serve(self.teammate.clone()).await {
            Ok(served) => {
                *lock(&self.served) = Some(served);
                Ok(())
            }
            Err(error) => Err(format!(
                "Hotline could not open a loopback port for its own tools: {error}"
            )),
        }
    }

    /// ACP children own their MCP transports, so an OAuth descriptor cannot
    /// carry a one-time bearer token. A per-session loopback proxy asks the
    /// rmcp manager for a current token on every request and checks the live
    /// capability before forwarding it.
    async fn prepare_oauth_proxies(&self) {
        let Some(vault) = self.mcp_vault.clone() else {
            return;
        };
        for server in &self.mcp_servers {
            if !is_oauth_server(server) {
                continue;
            }
            let result = start_oauth_proxy(server, vault.clone(), self.capability.clone()).await;
            match result {
                Ok(proxy) => {
                    lock(&self.oauth_proxies).insert(server.id.clone(), proxy);
                }
                Err(error) => {
                    lock(&self.oauth_refused).insert(server.id.clone(), error);
                }
            }
        }
    }

    /// The MCP servers this session is opened with: Hotline's own first, then
    /// every third-party server the teammate's policy granted and this build
    /// can describe.
    ///
    /// Hotline does not connect any of these for a child. It says where they are
    /// and the child connects them, which is why a row about them can only
    /// ever say "declared".
    fn declared_servers(&self) -> Vec<acp::McpServer> {
        let mut declared = Vec::new();
        if let Some(served) = lock(&self.served).as_ref() {
            declared.push(acp::McpServer::Http(
                acp::McpServerHttp::new(mcp::server::SERVER_NAME, served.url()).headers(vec![
                    acp::HttpHeader::new("Authorization", format!("Bearer {}", served.token())),
                ]),
            ));
        }
        for server in &self.mcp_servers {
            if is_oauth_server(server) && !lock(&self.oauth_proxies).contains_key(&server.id) {
                continue;
            }
            if mcp::unsupported(server).is_some() && !is_oauth_server(server) {
                continue;
            }
            declared.push(match &server.transport {
                McpTransport::Stdio { command, args, env } => {
                    // Sorted, because a hash map's order is not a decision and
                    // two runs of the same room should send the same bytes.
                    let mut names: Vec<&String> = env.keys().collect();
                    names.sort();
                    acp::McpServer::Stdio(
                        acp::McpServerStdio::new(&server.name, command)
                            .args(args.clone())
                            .env(
                                names
                                    .into_iter()
                                    .map(|name| acp::EnvVariable::new(name, &env[name]))
                                    .collect(),
                            ),
                    )
                }
                McpTransport::Http { url, auth } => {
                    let proxies = lock(&self.oauth_proxies);
                    let proxy = proxies.get(&server.id);
                    let proxy_url = proxy.map_or(url.as_str(), |proxy| proxy.url.as_str());
                    let http = acp::McpServerHttp::new(&server.name, proxy_url);
                    acp::McpServer::Http(match auth {
                        HttpAuth::Oauth
                        | HttpAuth::OauthConfigured { .. }
                        | HttpAuth::Secret { .. } => {
                            let proxy = proxy.expect("only connected proxies are declared");
                            http.headers(vec![acp::HttpHeader::new(
                                "Authorization",
                                format!("Bearer {}", proxy.token),
                            )])
                        }
                        HttpAuth::Bearer { token } => http.headers(vec![acp::HttpHeader::new(
                            "Authorization",
                            format!("Bearer {token}"),
                        )]),
                        HttpAuth::None => http,
                    })
                }
            });
        }
        declared
    }

    /// What this teammate was given, written down before the child has had a
    /// chance to take any of it.
    ///
    /// Everything here is `declared`: Hotline hands a child a list and never
    /// sees the tools it ends up with. The one exception is Hotline's own
    /// server, which promotes its rows to `verified` the moment the child
    /// lists tools on the endpoint — see [`crate::mcp::server`].
    fn publish_ledger(&self, persona: &Persona, serving: Result<(), String>) {
        let mut ledger = ToolLedger::new(
            persona.id.clone(),
            AgentKind::Acp,
            persona.backend_id.clone(),
        );
        let hotline_tools = self.teammate.tool_names();
        match &serving {
            Ok(()) => ledger.all(
                ToolState::Declared,
                ToolSourceKind::Builtin,
                mcp::server::SERVER_NAME,
                &hotline_tools,
                "served on this teammate's own loopback endpoint and named in its session",
            ),
            Err(reason) => ledger.all(
                ToolState::Absent,
                ToolSourceKind::Builtin,
                mcp::server::SERVER_NAME,
                &hotline_tools,
                reason,
            ),
        };
        for server in &self.mcp_servers {
            if is_oauth_server(server) {
                let refused = lock(&self.oauth_refused).get(&server.id).cloned();
                if let Some(reason) = refused {
                    ledger.absent(ToolSourceKind::Mcp, &server.id, &server.name, reason);
                } else if lock(&self.oauth_proxies).contains_key(&server.id) {
                    ledger.declared(
                        ToolSourceKind::Mcp,
                        &server.id,
                        &server.name,
                        "authenticated through Hotline's per-session loopback OAuth proxy",
                    );
                } else {
                    ledger.absent(
                        ToolSourceKind::Mcp,
                        &server.id,
                        &server.name,
                        "MCP OAuth sign-in is required in Settings → Tools",
                    );
                }
                continue;
            }
            match mcp::unsupported(server) {
                Some(reason) => {
                    ledger.absent(ToolSourceKind::Mcp, &server.id, &server.name, reason)
                }
                None => ledger.declared(
                    ToolSourceKind::Mcp,
                    &server.id,
                    &server.name,
                    "named in the session this agent opened; Hotline does not connect it and cannot see the tools it supplied",
                ),
            };
        }
        for id in &self.mcp_missing {
            ledger.absent(ToolSourceKind::Mcp, id, id, mcp::missing_reason(id));
        }
        ledger.publish();
    }

    fn adopt(
        &self,
        id: SessionId,
        modes: Option<acp::SessionModeState>,
        configs: Option<Vec<acp::SessionConfigOption>>,
        restored: bool,
    ) {
        lock(&self.live.tools).clear();
        self.live.forget_subagents();
        let disposition = Disposition::of(modes.as_ref(), configs.as_deref());
        let mut session = lock(&self.live.session);
        session.id = Some(id.clone());
        session.model_config = disposition.model_config;
        session.mode_config = disposition.mode_config;
        session.dedicated_modes = disposition.dedicated_modes;
        session.info.session_id = Some(id.0.to_string());
        session.info.context_restored = restored;
        session.info.models = disposition.models;
        session.info.current_model_id = disposition.current_model_id.unwrap_or_default();
        session.info.model_label = disposition.model_label;
        session.info.modes = disposition.modes;
        session.info.current_mode_id = disposition.current_mode_id;
        session.info.mode_label = disposition.mode_label;
        session.info.configs = disposition.configs;
    }

    /// Puts the teammate's own model and mode back on.
    ///
    /// These are session-scoped for every agent Hotline drives, so a teammate that
    /// is not asked again arrives on whatever the harness defaults to — which
    /// is a teammate quietly losing its disposition on every restart. A
    /// refusal is not fatal: the session is up either way, and the picker will
    /// show what the agent actually settled on.
    pub(super) async fn adopt_disposition(&self, persona: &Persona) {
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
    pub(super) async fn set_config(
        &self,
        config_id: &str,
        value: &str,
    ) -> Result<DriverInfo, String> {
        self.check_capability()?;
        // One statement each: a tuple would keep the first guard alive while
        // the second is taken, and `set_mode` reaches for the same two. Two
        // callers holding one of these each is a wedge nothing recovers from.
        let connection = lock(&self.live.connection).clone();
        let session_id = lock(&self.live.session).id.clone();
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
        self.check_capability()?;
        self.apply_configs(&answered.config_options);
        Ok(self.live.publish_info())
    }

    fn apply_configs(&self, configs: &[acp::SessionConfigOption]) {
        let mut session = lock(&self.live.session);
        apply_configs_to_session(&mut session, configs);
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
    callback_workspace: Workspace,
    transport: ByteStreams<
        impl futures_util::AsyncWrite + Send + 'static,
        impl futures_util::AsyncRead + Send + 'static,
    >,
) -> Result<ConnectionTo<Agent>, String> {
    let (ready, started) = oneshot::channel();
    let updates = live.clone();
    let asked = live.clone();
    let read_workspace = callback_workspace.clone();
    let write_workspace = callback_workspace;
    let task_live = live.clone();
    let task = tokio::spawn(async move {
        let running = Client
            .builder()
            .name("Hotline")
            .on_receive_notification(
                move |notification: RawSessionUpdate, _cx| {
                    let live = updates.clone();
                    async move {
                        // Harness login is not a conversation, including unsolicited
                        // session/update notifications emitted by authenticate.
                        if !lock(&live.auth).as_ref().is_some_and(|a| a.running()) {
                            receive(&live, notification).await;
                        }
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
                    responder.respond(ReadTextFileResponse::new(read_text_file(
                        &read_workspace,
                        &request,
                    )?))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |request: WriteTextFileRequest, responder, _cx| {
                    write_text_file(&write_workspace, &request)?;
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
    *lock(&task_live.connection_task) = Some(task);
    started
        .await
        .map_err(|_| "The agent's connection ended before it opened.".to_string())
}

/// `session/update` as it comes off the wire, before it is read as one of the
/// updates the SDK knows. Subagents arrive as ACP's draft subagent updates
/// (agent-client-protocol#1992), which the SDK does not carry yet, and as
/// ordinary updates on sessions of their own.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, JsonRpcNotification)]
#[notification(method = "session/update")]
#[serde(rename_all = "camelCase")]
pub(super) struct RawSessionUpdate {
    pub(super) session_id: String,
    pub(super) update: Value,
}

/// `initialize`, with the capabilities the SDK has no field for yet.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, JsonRpcRequest)]
#[request(method = "initialize", response = acp::InitializeResponse)]
pub(super) struct RawInitialize(pub(super) Value);

/// A subagent's comings and goings, as the agent announces them on the
/// session that started it.
#[derive(Debug, PartialEq)]
pub(super) enum Announced {
    Spawned {
        child: String,
        title: String,
        task: String,
    },
    Ended {
        child: String,
        status: SubagentStatus,
    },
}

/// The announcement in an update, if it is one. Claude's and Codex's
/// adapters send `subagent_spawned` and `subagent_state_update`; ACP's RFD
/// describes a `subagent_update` that neither sends yet.
pub(super) fn announced(update: &Value) -> Option<Announced> {
    let text = |key: &str| update.get(key).and_then(Value::as_str).map(str::trim);
    let child = text("subagentSessionId")
        .filter(|child| !child.is_empty())?
        .to_string();
    match text("sessionUpdate")? {
        "subagent_spawned" => {
            let task = text("task").unwrap_or_default();
            Some(Announced::Spawned {
                title: clip(
                    text("name")
                        .filter(|name| !name.is_empty())
                        .unwrap_or("Subagent"),
                    TITLE_CHARS,
                ),
                // The adapter's own prompt is what the subagent was told; the
                // task is its summary of it.
                task: text("prompt")
                    .filter(|prompt| !prompt.is_empty())
                    .unwrap_or(task)
                    .to_string(),
                child,
            })
        }
        "subagent_state_update" => Some(Announced::Ended {
            status: match text("state")? {
                "completed" => SubagentStatus::Done,
                "failed" => SubagentStatus::Failed,
                // Disconnected is an outcome nobody saw, which a run that
                // outlived its process is too.
                _ => SubagentStatus::Cancelled,
            },
            child,
        }),
        _ => None,
    }
}

/// What Hotline says it can do when it meets an agent.
fn initialize_request() -> RawInitialize {
    let request = InitializeRequest::new(ProtocolVersion::V1).client_capabilities(
        // Hotline answers file reads and writes on the agent's behalf
        // so an agent that expects an editor to own the files gets
        // one. Login's PTY is desktop-owned, not an agent terminal tool.
        ClientCapabilities::new()
            .fs(FileSystemCapabilities::new()
                .read_text_file(true)
                .write_text_file(true))
            .auth(acp::AuthCapabilities::new().terminal(cfg!(unix))),
    );
    let mut request = serde_json::to_value(request).unwrap_or_default();
    // Subagents shown as sessions of their own, which each become a run with
    // its own transcript. An agent that does not know the field keeps
    // showing them as tool calls.
    if let Some(capabilities) = request
        .get_mut("clientCapabilities")
        .and_then(Value::as_object_mut)
    {
        capabilities.insert("subagents".into(), serde_json::json!({}));
    }
    RawInitialize(request)
}

/// The teammate's granted folders as ACP `additionalDirectories`: absolute
/// paths, in the order the person granted them. Whether each may be changed
/// is not something the protocol can say; the harness's own permissions
/// decide, and Hotline's callbacks refuse a write to a read-only one.
fn granted_directories(persona: &Persona) -> Vec<PathBuf> {
    persona
        .folders
        .iter()
        .flatten()
        .map(|folder| PathBuf::from(&folder.path))
        .collect()
}

/// What the agent said it can do. Taking a line into a running turn is not an
/// ACP capability yet; it rides in `initialize`'s `_meta`, where the steering
/// extension puts it.
fn capabilities_of(initialized: &acp::InitializeResponse) -> SessionCapabilities {
    let capabilities = &initialized.agent_capabilities;
    SessionCapabilities {
        active_input: initialized
            .meta
            .as_ref()
            .and_then(|meta| meta.get("steering"))
            .and_then(|steering| steering.get("supported"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        load_session: capabilities.load_session,
        resume: capabilities.session_capabilities.resume.is_some(),
        fork: capabilities.session_capabilities.fork.is_some(),
        mcp_http: capabilities.mcp_capabilities.http,
        image: capabilities.prompt_capabilities.image,
        additional_directories: capabilities
            .session_capabilities
            .additional_directories
            .is_some(),
    }
}

/// The model, runtime mode, and effort pickers Hotline draws, out of whichever
/// shape the agent sent them in. Other ACP config selectors stay private to
/// the harness until Hotline has a contract for them.
///
/// ACP has a dedicated `modes` field and a generic `configOptions` list, and
/// agents differ over which they use — Cursor sends modes, Claude Code's
/// adapter sends config options. Reading both here is what lets the header
/// stop caring which agent it is talking to.
#[derive(Default)]
pub(super) struct Disposition {
    models: Vec<crate::contract::ConfigChoice>,
    pub(super) current_model_id: Option<String>,
    pub(super) model_config: Option<String>,
    model_label: Option<String>,
    pub(super) modes: Vec<crate::contract::ConfigChoice>,
    pub(super) current_mode_id: Option<String>,
    pub(super) mode_config: Option<String>,
    mode_label: Option<String>,
    pub(super) configs: Vec<crate::contract::SessionConfig>,
    pub(super) dedicated_modes: bool,
}

impl Disposition {
    pub(super) fn of(
        modes: Option<&acp::SessionModeState>,
        configs: Option<&[acp::SessionConfigOption]>,
    ) -> Self {
        let mut disposition = Self {
            dedicated_modes: modes.is_some(),
            ..Self::default()
        };
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
                Some(acp::SessionConfigOptionCategory::Mode) if !disposition.dedicated_modes => {
                    disposition.modes = picker;
                    disposition.current_mode_id = Some(select.current_value.0.to_string());
                    disposition.mode_config = Some(option.id.0.to_string());
                    disposition.mode_label = Some(option.name.clone());
                }
                Some(acp::SessionConfigOptionCategory::ThoughtLevel) => {
                    disposition.configs.push(crate::contract::SessionConfig {
                        id: option.id.0.to_string(),
                        name: option.name.clone(),
                        category: Some(crate::contract::SessionConfigCategory::Effort),
                        current_id: Some(select.current_value.0.to_string()),
                        options: picker,
                    });
                }
                // Hotline has no stable UI contract for model sub-options or
                // arbitrary ACP settings. They remain harness-owned.
                Some(
                    acp::SessionConfigOptionCategory::Mode
                    | acp::SessionConfigOptionCategory::ModelConfig
                    | acp::SessionConfigOptionCategory::Other(_)
                    | _,
                )
                | None => {}
            }
        }
        disposition
    }
}

/// Merges an ACP config notification into the live driver state. Config
/// notifications carry the full set, so a ThoughtLevel change can be
/// reflected without disturbing a dedicated runtime mode or model picker.
pub(super) fn apply_configs_to_session(
    session: &mut Session,
    configs: &[acp::SessionConfigOption],
) {
    let disposition = Disposition::of(None, Some(configs));
    if disposition.model_config.is_some() {
        session.model_config = disposition.model_config;
        session.info.models = disposition.models;
        session.info.model_label = disposition.model_label;
        if let Some(current) = disposition.current_model_id {
            session.info.current_model_id = current;
        }
    } else {
        // ACP config notifications are the full set. A removed model option
        // must not leave an old picker and id usable in the room.
        session.model_config = None;
        session.info.models.clear();
        session.info.current_model_id.clear();
        session.info.model_label = None;
    }
    if disposition.mode_config.is_some() && !session.dedicated_modes {
        session.mode_config = disposition.mode_config;
        session.info.modes = disposition.modes;
        session.info.mode_label = disposition.mode_label;
        session.info.current_mode_id = disposition.current_mode_id;
    } else if !session.dedicated_modes {
        session.mode_config = None;
        session.info.modes.clear();
        session.info.current_mode_id = None;
        session.info.mode_label = None;
    }
    session.info.configs = disposition.configs;
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
