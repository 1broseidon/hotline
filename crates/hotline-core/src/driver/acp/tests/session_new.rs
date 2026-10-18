use super::*;

/// What a child is told about tools: Hotline's own server on this session's
/// loopback endpoint behind this session's token, then every third-party
/// server the teammate was granted and this build can describe.
///
/// And the ledger says the same thing in the same breath, because a row
/// that disagrees with what was sent is worse than no row.
#[tokio::test(flavor = "multi_thread")]
async fn session_new_carries_hotlines_own_server_and_the_granted_ones() {
    let held = room("declared-room");
    let heard = Heard::default();
    let agent = scripted_agent(heard.clone(), false);
    let driver = ChildAgent::new(
        scratch("declared"),
        "cursor".to_string(),
        "you are Ada".to_string(),
        TeammateTools::new(&held, "declared"),
    )
    .with_mcp(
        vec![
            McpServer {
                id: "echo".to_string(),
                name: "Echo".to_string(),
                transport: McpTransport::Stdio {
                    command: "/usr/bin/echo".to_string(),
                    args: vec!["--mcp".to_string()],
                    env: HashMap::from([("TOKEN".to_string(), "shh".to_string())]),
                },
                refuse: None,
            },
            McpServer {
                id: "remote".to_string(),
                name: "Remote".to_string(),
                transport: McpTransport::Http {
                    url: "https://example.test/mcp".to_string(),
                    auth: crate::mcp::HttpAuth::None,
                },
                refuse: None,
            },
            McpServer {
                id: "locked".to_string(),
                name: "Locked".to_string(),
                transport: McpTransport::Http {
                    url: "https://example.test/oauth".to_string(),
                    auth: crate::mcp::HttpAuth::Oauth,
                },
                refuse: None,
            },
            McpServer {
                id: "computer".to_string(),
                name: "Computer".to_string(),
                transport: McpTransport::Http {
                    url: "http://127.0.0.1:8787/mcp".to_string(),
                    auth: crate::mcp::HttpAuth::Bearer {
                        token: "comp-token".to_string(),
                    },
                },
                refuse: None,
            },
        ],
        vec!["deleted".to_string()],
    );
    tokio::spawn(agent);

    let mut ada = persona(&scratch_cwd(), Vec::new());
    ada.id = "declared".to_string();
    driver.handshake(&ada, client_transport()).await.unwrap();

    let declared = heard.servers.lock().unwrap().clone();
    let names: Vec<&str> = declared
        .iter()
        .map(|server| match server {
            acp::McpServer::Http(http) => http.name.as_str(),
            acp::McpServer::Stdio(stdio) => stdio.name.as_str(),
            _ => "?",
        })
        .collect();
    assert_eq!(
        names,
        ["hotline", "Echo", "Remote", "Computer"],
        "the OAuth server this build cannot honour is not offered"
    );

    let acp::McpServer::Http(hotline) = &declared[0] else {
        panic!("Hotline's own server is reached over HTTP");
    };
    assert!(
        hotline.url.starts_with("http://127.0.0.1:"),
        "{}",
        hotline.url
    );
    assert_eq!(hotline.headers.len(), 1);
    assert_eq!(hotline.headers[0].name, "Authorization");
    assert!(
        hotline.headers[0].value.starts_with("Bearer "),
        "{}",
        hotline.headers[0].value
    );

    let acp::McpServer::Stdio(echo) = &declared[1] else {
        panic!("a stdio server is offered as stdio");
    };
    assert_eq!(echo.command, PathBuf::from("/usr/bin/echo"));
    assert_eq!(echo.args, ["--mcp"]);
    assert_eq!(echo.env[0].name, "TOKEN");

    let acp::McpServer::Http(computer) = &declared[3] else {
        panic!("the computer is reached over HTTP");
    };
    assert_eq!(computer.url, "http://127.0.0.1:8787/mcp");
    assert_eq!(computer.headers.len(), 1);
    assert_eq!(computer.headers[0].name, "Authorization");
    assert_eq!(computer.headers[0].value, "Bearer comp-token");

    let rows = crate::session::ledger::teammate_tools("declared")
        .expect("the child's ledger was published at start")
        .rows;
    let row = |name: &str| {
        rows.iter()
            .find(|row| row.name == name)
            .unwrap_or_else(|| panic!("{name} is on the ledger: {rows:?}"))
            .clone()
    };
    for tool in crate::mcp::server::TOOL_NAMES {
        let row = row(tool);
        assert_eq!(row.source, ToolSourceKind::Builtin);
        assert_eq!(row.origin, "hotline");
        assert_eq!(row.state, ToolState::Declared);
    }
    assert_eq!(row("Echo").state, ToolState::Declared);
    assert_eq!(row("Computer").state, ToolState::Declared);
    assert_eq!(row("Locked").state, ToolState::Absent);
    assert!(row("Locked").reason.contains("OAuth"));
    assert_eq!(row("deleted").state, ToolState::Absent);
    assert!(row("deleted").reason.contains("no longer exists"));

    // Listing tools on the endpoint is the one thing Hotline can watch a
    // child do, so it is the one thing that turns declared into verified.
    let listing = rmcp::model::ClientInfo::new(
        Default::default(),
        rmcp::model::Implementation::new("test", "1"),
    )
    .serve(
        rmcp::transport::streamable_http_client::StreamableHttpClientTransport::with_client(
            reqwest::Client::default(),
            rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(
                hotline.url.clone(),
            )
            .auth_header(
                hotline.headers[0]
                    .value
                    .strip_prefix("Bearer ")
                    .expect("the header is a bearer token")
                    .to_string(),
            ),
        ),
    )
    .await
    .expect("the token in session/new opens the endpoint");
    assert_eq!(
        listing.list_all_tools().await.expect("tools listed").len(),
        crate::mcp::server::TOOL_NAMES.len()
    );
    listing.cancel().await.ok();

    let seen = crate::session::ledger::teammate_tools("declared")
        .unwrap()
        .rows;
    for tool in crate::mcp::server::TOOL_NAMES {
        let row = seen.iter().find(|row| row.name == tool).unwrap();
        assert_eq!(row.state, ToolState::Verified, "{row:?}");
        assert!(row.reason.contains("own endpoint"), "{row:?}");
    }
}

/// An ACP agent that lists Hotline's own tools while `session/new` is still in
/// flight, which is the earliest a child can: the endpoint and its token
/// are in the request it is answering.
fn agent_that_lists_hotlines_tools_during_session_new(
    listed: Arc<Mutex<usize>>,
) -> impl std::future::Future<Output = ()> + Send + 'static {
    let transport = agent_pipes();
    async move {
        let running = agent_client_protocol::Agent
            .builder()
            .name("eager")
            .on_receive_request(
                async move |request: InitializeRequest, responder, _cx| {
                    responder.respond(
                        InitializeResponse::new(request.protocol_version)
                            .agent_info(Implementation::new("eager", "1")),
                    )
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                move |request: NewSessionRequest,
                      responder: Responder<NewSessionResponse>,
                      _cx| {
                    let listed = listed.clone();
                    async move {
                        let hotline = request
                            .mcp_servers
                            .iter()
                            .find_map(|server| match server {
                                acp::McpServer::Http(http)
                                    if http.name == mcp::server::SERVER_NAME =>
                                {
                                    Some(http.clone())
                                }
                                _ => None,
                            })
                            .expect("Hotline names its own endpoint in session/new");
                        let token = hotline.headers[0]
                            .value
                            .strip_prefix("Bearer ")
                            .expect("a bearer token")
                            .to_string();
                        let client = rmcp::model::ClientInfo::new(
                            Default::default(),
                            rmcp::model::Implementation::new("eager", "1"),
                        )
                        .serve(
                            rmcp::transport::streamable_http_client::StreamableHttpClientTransport::with_client(
                                reqwest::Client::default(),
                                rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(hotline.url.clone())
                                    .auth_header(token),
                            ),
                        )
                        .await
                        .expect("the endpoint is open before the session is");
                        *listed.lock().unwrap() =
                            client.list_all_tools().await.expect("tools listed").len();
                        client.cancel().await.ok();
                        responder.respond(NewSessionResponse::new(SessionId::new("eager")))
                    }
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_to(transport)
            .await;
        if let Err(error) = running {
            eprintln!("the eager agent ended: {error}");
        }
    }
}

/// A child that takes Hotline's tools during `session/new` is a child Hotline
/// watched take them.
///
/// The endpoint is handed over inside that request, so the ledger has to
/// exist before it is sent. Published afterwards, the promotion the
/// listing makes lands on nothing, and the rows read `declared` for a
/// session whose agent was seen listing them.
#[tokio::test(flavor = "multi_thread")]
async fn tools_listed_during_session_new_are_verified_on_the_ledger() {
    let held = room("eager-room");
    let listed = Arc::new(Mutex::new(0usize));
    let agent = agent_that_lists_hotlines_tools_during_session_new(listed.clone());
    let driver = ChildAgent::new(
        scratch("eager"),
        "cursor".to_string(),
        String::new(),
        TeammateTools::new(&held, "eager"),
    );
    tokio::spawn(agent);

    let mut ada = persona(&scratch_cwd(), Vec::new());
    ada.id = "eager".to_string();
    driver.handshake(&ada, client_transport()).await.unwrap();

    assert_eq!(*listed.lock().unwrap(), mcp::server::TOOL_NAMES.len());
    let rows = crate::session::ledger::teammate_tools("eager")
        .expect("the ledger exists before the session does")
        .rows;
    for tool in mcp::server::TOOL_NAMES {
        let row = rows
            .iter()
            .find(|row| row.name == tool)
            .unwrap_or_else(|| panic!("{tool} is on the ledger: {rows:?}"));
        assert_eq!(row.state, ToolState::Verified, "{row:?}");
    }
}

/// Hotline rewrites only the file it wrote. A hand-written AGENTS.md — even
/// one that merely mentions the marker, as this repository's own does — is
/// left exactly as it was.
#[test]
fn only_a_file_opening_with_the_marker_is_hotlines_to_replace() {
    let root = scratch("agents-md");
    let mut ada = persona(&root.to_string_lossy(), Vec::new());
    let file = root.join("AGENTS.md");

    materialize_agents_md(&ada).unwrap();
    let written = std::fs::read_to_string(&file).unwrap();
    assert!(written.starts_with(MANAGED_MARKER));
    assert!(written.contains("# Ada"));
    assert!(written.contains("Keep the harbour running."));

    // Hotline's own file is rewritten when the goal moves.
    ada.goal = "Count the boats.".to_string();
    materialize_agents_md(&ada).unwrap();
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .contains("Count the boats.")
    );

    let by_hand = format!("# A real repository\n\nIt explains {MANAGED_MARKER} in a sentence.\n");
    std::fs::write(&file, &by_hand).unwrap();
    ada.goal = "Something else entirely.".to_string();
    materialize_agents_md(&ada).unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), by_hand);
}

#[cfg(unix)]
#[test]
fn acp_callbacks_stay_in_workspace_for_legacy_machine_personas() {
    use std::os::unix::fs::symlink;

    let root = scratch("callbacks-root");
    let outside = scratch("callbacks-outside");
    let inside_file = root.join("inside.txt");
    let outside_file = outside.join("secret.txt");
    std::fs::write(&inside_file, "inside\nsecond\n").unwrap();
    std::fs::write(&outside_file, "secret\n").unwrap();
    symlink(&outside_file, root.join("escape.txt")).unwrap();

    let mut ada = persona(&root.to_string_lossy(), Vec::new());
    ada.reach = Some(Reach::Machine);
    let workspace = ChildAgent::callback_workspace(&ada, None).unwrap();
    let allowed = ReadTextFileRequest::new(SessionId::new("s"), inside_file.clone()).limit(1);
    assert_eq!(read_text_file(&workspace, &allowed).unwrap(), "inside");

    let outside_request = ReadTextFileRequest::new(SessionId::new("s"), outside_file.clone());
    assert!(read_text_file(&workspace, &outside_request).is_err());
    let symlink_request = ReadTextFileRequest::new(SessionId::new("s"), root.join("escape.txt"));
    assert!(read_text_file(&workspace, &symlink_request).is_err());

    let nested = root.join("new").join("file.txt");
    let write = WriteTextFileRequest::new(SessionId::new("s"), nested.clone(), "written\n");
    write_text_file(&workspace, &write).unwrap();
    assert_eq!(std::fs::read_to_string(&nested).unwrap(), "written\n");

    let outside_write =
        WriteTextFileRequest::new(SessionId::new("s"), outside_file.clone(), "nope\n");
    assert!(write_text_file(&workspace, &outside_write).is_err());
    assert_eq!(std::fs::read_to_string(&outside_file).unwrap(), "secret\n");
}

/// Hotline's callbacks for a harness honour the person's folders as the
/// built-in tools do: a read anywhere in a granted folder, a write only
/// in one that may be changed, and nothing through a link out of one.
#[cfg(unix)]
#[test]
fn acp_callbacks_honour_granted_folders() {
    use std::os::unix::fs::symlink;

    let root = scratch("callbacks-folders-root");
    let read_only = scratch("callbacks-folders-read").canonicalize().unwrap();
    let writable = scratch("callbacks-folders-write").canonicalize().unwrap();
    let outside = scratch("callbacks-folders-outside");
    std::fs::write(read_only.join("notes.txt"), "granted\n").unwrap();
    std::fs::write(outside.join("secret.txt"), "secret\n").unwrap();
    symlink(outside.join("secret.txt"), read_only.join("escape.txt")).unwrap();
    let mut ada = persona(&root.to_string_lossy(), Vec::new());

    // No field, no folders: the record from before the field reaches
    // only its workspace.
    let without = ChildAgent::callback_workspace(&ada, None).unwrap();
    let notes = ReadTextFileRequest::new(SessionId::new("s"), read_only.join("notes.txt"));
    assert!(read_text_file(&without, &notes).is_err());

    ada.folders = Some(vec![
        crate::contract::FolderGrant {
            path: read_only.to_string_lossy().into_owned(),
            writable: false,
        },
        crate::contract::FolderGrant {
            path: writable.to_string_lossy().into_owned(),
            writable: true,
        },
    ]);
    let workspace = ChildAgent::callback_workspace(&ada, None).unwrap();
    assert_eq!(read_text_file(&workspace, &notes).unwrap(), "granted\n");
    let escape = ReadTextFileRequest::new(SessionId::new("s"), read_only.join("escape.txt"));
    assert!(read_text_file(&workspace, &escape).is_err());
    let elsewhere = ReadTextFileRequest::new(SessionId::new("s"), outside.join("secret.txt"));
    assert!(read_text_file(&workspace, &elsewhere).is_err());

    let refused = WriteTextFileRequest::new(
        SessionId::new("s"),
        read_only.join("new").join("file.txt"),
        "nope\n",
    );
    let error = write_text_file(&workspace, &refused).unwrap_err();
    assert!(error.to_string().contains("read but not change"), "{error}");
    assert!(
        !read_only.join("new").exists(),
        "no directory is made for a refused write"
    );

    let allowed = WriteTextFileRequest::new(
        SessionId::new("s"),
        writable.join("new").join("file.txt"),
        "written\n",
    );
    write_text_file(&workspace, &allowed).unwrap();
    assert_eq!(
        std::fs::read_to_string(writable.join("new/file.txt")).unwrap(),
        "written\n"
    );
}

/// A harness that says it takes extra roots is opened with the granted
/// folders, and the session says so; one that does not is not sent them.
#[tokio::test(flavor = "multi_thread")]
async fn session_new_hands_the_granted_folders_to_a_harness_that_takes_them() {
    let folder = scratch("directories-folder").canonicalize().unwrap();
    for takes in [true, false] {
        let held = room("directories-room");
        let heard = Heard {
            takes_directories: takes,
            ..Heard::default()
        };
        let agent = scripted_agent(heard.clone(), false);
        let driver = ChildAgent::new(
            scratch("directories"),
            "cursor".to_string(),
            "you are Ada".to_string(),
            TeammateTools::new(&held, "directories"),
        );
        tokio::spawn(agent);
        let mut ada = persona(&scratch_cwd(), Vec::new());
        ada.id = "directories".to_string();
        ada.folders = Some(vec![crate::contract::FolderGrant {
            path: folder.to_string_lossy().into_owned(),
            writable: false,
        }]);
        let info = driver.handshake(&ada, client_transport()).await.unwrap();
        assert_eq!(info.capabilities.additional_directories, takes);
        let sent = heard.directories.lock().unwrap().clone();
        if takes {
            assert_eq!(sent, vec![folder.clone()]);
        } else {
            assert!(sent.is_empty(), "{sent:?}");
        }
    }
}

#[cfg(unix)]
#[test]
fn callback_workspace_accepts_the_selected_symlinked_root_alias() {
    use std::os::unix::fs::symlink;

    let parent = scratch("callbacks-alias");
    let actual = parent.join("actual");
    let alias = parent.join("alias");
    std::fs::create_dir(&actual).unwrap();
    symlink(&actual, &alias).unwrap();
    let file = actual.join("inside.txt");
    std::fs::write(&file, "through alias\n").unwrap();

    let ada = persona(&alias.to_string_lossy(), Vec::new());
    let workspace = ChildAgent::callback_workspace(&ada, None).unwrap();
    let request = ReadTextFileRequest::new(SessionId::new("s"), alias.join("inside.txt"));
    assert_eq!(
        read_text_file(&workspace, &request).unwrap(),
        "through alias\n"
    );
}

#[cfg(unix)]
#[test]
fn agents_md_refuses_external_and_dangling_symlinks() {
    use std::os::unix::fs::symlink;

    let root = scratch("agents-symlink");
    let outside = scratch("agents-symlink-outside");
    let outside_file = outside.join("AGENTS.md");
    std::fs::write(&outside_file, "keep me\n").unwrap();
    symlink(&outside_file, root.join("AGENTS.md")).unwrap();
    let ada = persona(&root.to_string_lossy(), Vec::new());
    assert!(materialize_agents_md(&ada).is_err());
    assert_eq!(std::fs::read_to_string(&outside_file).unwrap(), "keep me\n");

    std::fs::remove_file(root.join("AGENTS.md")).unwrap();
    let missing = outside.join("created-by-dangling-link.md");
    symlink(&missing, root.join("AGENTS.md")).unwrap();
    assert!(materialize_agents_md(&ada).is_err());
    assert!(!missing.exists());
}
