//! A scripted ACP agent the harnesses spawn as a real child, so a test can
//! drive the ACP path end to end: the handshake, the desk's MCP server named
//! in `session/new`, and tool calls over it.
//!
//! It does what its prompt says. Each line `CALL <tool> <json arguments>`
//! anywhere in a prompt is one call to that tool on Hotline's MCP server, made
//! in order; the turn's one message is a `RESULTS` line followed by one line
//! per call, `<tool> ok: <text>` or `<tool> refused: <text>`. A prompt with no
//! calls is answered "On it." A REPLY_JSON line supplies a JSON string as
//! the answer, so reply-format scenarios need not echo session instructions.
//! REPORT_TOOLS also emits ACP tool notifications around those MCP calls.
//!
//! A prompt line `REPORT_ENV <name>...` is answered instead with one
//! `ENV <name>=<value>` or `ENV <name> unset` line per name, then the same
//! lines, each after `CHILD_ENV `, as printed by a child of its own run with
//! `--print-env <name>...`, so a harness sees what the agent and its
//! subprocesses inherit. A value is printed only under the agent-accounts
//! harness, which sets `HOTLINE_TEST_AGENT_ACCOUNTS`; elsewhere a name that is
//! set reads `ENV <name> set`.
//!
//! With `--cli` anywhere in its arguments it is instead the agent's own command
//! line for an account, `auth status [--json]` and `auth login …`, answered
//! from the folder in `CLAUDE_CONFIG_DIR`; a sign-in writes only into a folder
//! inside `agent-accounts`. Every run there writes the names of
//! its environment, never a value, to `<that folder>/fixture-<status|login>-env.json`;
//! without the variable it writes nothing, so it never reaches the home folder.
//! `FIXTURE_LOGIN_MODE` (`ok`, `fail`, `silent`, `hang`, `prompt`) picks how a
//! sign-in ends.

use agent_client_protocol::schema::v1::{
    AgentCapabilities, ContentBlock, ContentChunk, InitializeRequest, InitializeResponse,
    McpServer, NewSessionRequest, NewSessionResponse, PromptRequest, PromptResponse, SessionId,
    SessionNotification, SessionUpdate, StopReason, TextContent, ToolCall, ToolCallStatus,
    ToolCallUpdate, ToolCallUpdateFields,
};
use agent_client_protocol::{Agent, ByteStreams, Client, ConnectionTo, Responder};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

/// Where Hotline's own MCP server is for a session: its URL and bearer.
#[derive(Clone)]
struct Desk {
    url: String,
    token: String,
}

type Sessions = Arc<Mutex<HashMap<String, Desk>>>;

#[tokio::main]
async fn main() {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--print-env") {
        print_env(&args[1..]);
        return;
    }
    if let Some(at) = args.iter().position(|arg| arg == "--cli") {
        std::process::exit(cli(&args[at + 1..]));
    }
    let sessions: Sessions = Arc::default();
    let opened = sessions.clone();
    let histories: Arc<Mutex<HashMap<String, Vec<String>>>> = Arc::default();
    let transport = ByteStreams::new(
        tokio::io::stdout().compat_write(),
        tokio::io::stdin().compat(),
    );
    let running = Agent
        .builder()
        .name("hotline-acp-fixture")
        .on_receive_request(
            async |request: InitializeRequest, responder, _cx| {
                responder.respond(
                    InitializeResponse::new(request.protocol_version)
                        .agent_capabilities(AgentCapabilities::new()),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            move |request: NewSessionRequest, responder: Responder<NewSessionResponse>, _cx| {
                let opened = opened.clone();
                async move {
                    let id = uuid::Uuid::new_v4().to_string();
                    if let Some(desk) = hotline_server(&request.mcp_servers) {
                        opened.lock().unwrap().insert(id.clone(), desk);
                    }
                    responder.respond(NewSessionResponse::new(SessionId::new(id)))
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            move |request: PromptRequest,
                  responder: Responder<PromptResponse>,
                  cx: ConnectionTo<Client>| {
                let histories = histories.clone();
                let desk = sessions
                    .lock()
                    .unwrap()
                    .get(&request.session_id.0.to_string())
                    .cloned();
                async move {
                    let said: String = request
                        .prompt
                        .iter()
                        .filter_map(|block| match block {
                            ContentBlock::Text(text) => Some(text.text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    let session = request.session_id.clone();
                    let replying = cx.clone();
                    // The calls run off the connection's own loop, which has
                    // to stay free to carry the session's other messages.
                    cx.spawn(async move {
                        let reply = answer(&said, desk, &replying, &session).await;
                        if let Some(path) = said
                            .lines()
                            .find_map(|line| line.trim().strip_prefix("WAIT_FOR "))
                        {
                            while !std::path::Path::new(path).exists() {
                                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                            }
                        }
                        let reply = {
                            let mut histories = histories.lock().unwrap();
                            let history = histories.entry(session.0.to_string()).or_default();
                            history.push(said.clone());
                            if let Some(reply) = said
                                .lines()
                                .find_map(|line| line.trim().strip_prefix("REPLY_JSON "))
                            {
                                serde_json::from_str::<String>(reply)
                                    .expect("REPLY_JSON is a JSON string")
                            } else if said.contains("ECHO_HISTORY") {
                                history.join("\n")
                            } else {
                                reply
                            }
                        };
                        replying.send_notification(SessionNotification::new(
                            session,
                            SessionUpdate::AgentMessageChunk(ContentChunk::new(
                                ContentBlock::Text(TextContent::new(reply)),
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
        eprintln!("hotline-acp-fixture ended: {error}");
    }
}

/// The server Hotline named for itself in `session/new`.
fn hotline_server(servers: &[McpServer]) -> Option<Desk> {
    servers.iter().find_map(|server| match server {
        McpServer::Http(http) if http.name == "hotline" => {
            let token = http.headers.iter().find_map(|header| {
                (header.name == "Authorization")
                    .then(|| header.value.trim_start_matches("Bearer ").to_string())
            })?;
            Some(Desk {
                url: http.url.clone(),
                token,
            })
        }
        _ => None,
    })
}

/// The calls a prompt asks for, in order.
fn calls(said: &str) -> Vec<(String, Value)> {
    let said = said
        .split("The whole call so far (conversation data):")
        .next()
        .unwrap_or(said);
    if said.contains("Like we said on the call") {
        let Some((_, tail)) = said.rsplit_once("(thread call:") else {
            return Vec::new();
        };
        let key = tail.split(')').next().unwrap_or_default();
        return vec![(
            "read_thread".into(),
            serde_json::json!({"thread":format!("call:{key}")}),
        )];
    }
    said.lines()
        .filter_map(|line| {
            let rest = &line[line.find("CALL ")? + "CALL ".len()..];
            let (tool, arguments) = rest.split_once(' ')?;
            Some((tool.to_string(), serde_json::from_str(arguments).ok()?))
        })
        .collect()
}

/// Makes the calls the prompt asks for and says what came back.
async fn answer(
    said: &str,
    desk: Option<Desk>,
    connection: &ConnectionTo<Client>,
    session: &SessionId,
) -> String {
    if let Some(names) = reported(said) {
        return report(&names).await;
    }
    let calls = calls(said);
    if calls.is_empty() {
        return "On it.".to_string();
    }
    let Some(desk) = desk else {
        return "RESULTS\nno hotline server was named".to_string();
    };
    let transport = StreamableHttpClientTransport::with_client(
        reqwest::Client::default(),
        StreamableHttpClientTransportConfig::with_uri(desk.url).auth_header(desk.token),
    );
    let client = ClientInfo::new(
        ClientCapabilities::default(),
        Implementation::new("hotline-acp-fixture", env!("CARGO_PKG_VERSION")),
    );
    let connected = match client.serve(transport).await {
        Ok(connected) => connected,
        Err(error) => return format!("RESULTS\nthe hotline server refused: {error}"),
    };
    let mut lines = vec!["RESULTS".to_string()];
    let report = said.contains("REPORT_TOOLS");
    for (tool, arguments) in calls {
        if matches!(tool.as_str(), "write" | "edit") {
            lines.push(write_file(connection, session, &tool, &arguments).await);
            continue;
        }

        let id = uuid::Uuid::new_v4().to_string();
        if report {
            connection
                .send_notification(SessionNotification::new(
                    session.clone(),
                    SessionUpdate::ToolCall(
                        ToolCall::new(id.clone(), tool.clone())
                            .status(ToolCallStatus::InProgress)
                            .raw_input(arguments.clone()),
                    ),
                ))
                .expect("tool notification reaches the client");
        }
        let mut request = CallToolRequestParams::new(tool.clone());
        if let Value::Object(arguments) = arguments {
            request = request.with_arguments(arguments);
        }
        let result = connected.peer().call_tool(request).await;
        let status = if result
            .as_ref()
            .is_ok_and(|result| result.is_error != Some(true))
        {
            ToolCallStatus::Completed
        } else {
            ToolCallStatus::Failed
        };
        lines.push(match result {
            Ok(result) => {
                let text = result
                    .content
                    .iter()
                    .filter_map(|block| block.as_text().map(|text| text.text.clone()))
                    .collect::<Vec<_>>()
                    .join(" ");
                let outcome = if result.is_error == Some(true) {
                    "refused"
                } else {
                    "ok"
                };
                format!("{tool} {outcome}: {text}")
            }
            Err(error) => format!("{tool} failed: {error}"),
        });
        if report {
            connection
                .send_notification(SessionNotification::new(
                    session.clone(),
                    SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                        id,
                        ToolCallUpdateFields::new().status(status),
                    )),
                ))
                .expect("tool completion reaches the client");
        }
    }
    connected.cancel().await.ok();
    lines.join("\n")
}

/// The fixture's file tool exercises the delegated filesystem and reports the
/// same structured diff an ACP harness reports for its own successful edit.
async fn write_file(
    connection: &ConnectionTo<Client>,
    session: &SessionId,
    tool: &str,
    arguments: &Value,
) -> String {
    use agent_client_protocol::schema::v1::{ReadTextFileRequest, WriteTextFileRequest};
    let path = std::path::PathBuf::from(arguments["path"].as_str().expect("fixture path"));
    let before = connection
        .send_request(ReadTextFileRequest::new(session.clone(), path.clone()))
        .block_task()
        .await
        .ok()
        .map(|reply| reply.content);
    let after = if tool == "edit" {
        before.as_deref().unwrap_or_default().replacen(
            arguments["old_text"].as_str().unwrap(),
            arguments["new_text"].as_str().unwrap(),
            1,
        )
    } else {
        arguments["content"].as_str().unwrap().to_string()
    };
    let id = uuid::Uuid::new_v4().to_string();
    let content = serde_json::from_value(
        serde_json::json!({"type":"diff", "path":path, "oldText":before, "newText":after}),
    )
    .unwrap();
    connection
        .send_notification(SessionNotification::new(
            session.clone(),
            SessionUpdate::ToolCall(
                ToolCall::new(id.clone(), tool)
                    .kind(agent_client_protocol::schema::v1::ToolKind::Edit)
                    .status(ToolCallStatus::InProgress)
                    .content(vec![content]),
            ),
        ))
        .unwrap();
    let result = connection
        .send_request(WriteTextFileRequest::new(session.clone(), path, after))
        .block_task()
        .await;
    let status = if result.is_ok() {
        ToolCallStatus::Completed
    } else {
        ToolCallStatus::Failed
    };
    connection
        .send_notification(SessionNotification::new(
            session.clone(),
            SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                id,
                ToolCallUpdateFields::new().status(status),
            )),
        ))
        .unwrap();
    format!("{tool}: {status:?}")
}

fn cli(args: &[OsString]) -> i32 {
    let args: Vec<String> = args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let dir = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from);
    match words.as_slice() {
        ["auth", "status"] | ["auth", "status", "--json"] => auth_status(dir.as_deref()),
        ["auth", "login", rest @ ..] => auth_login(dir.as_deref(), rest),
        _ => 2,
    }
}

fn report_env_names(dir: &Path, run: &str) {
    let mut names: Vec<String> = std::env::vars_os()
        .map(|(name, _)| name.to_string_lossy().into_owned())
        .collect();
    names.sort();
    let report = serde_json::to_vec(&names).expect("names serialize");
    std::fs::write(dir.join(format!("fixture-{run}-env.json")), report)
        .expect("the account folder is writable");
}

fn auth_status(dir: Option<&Path>) -> i32 {
    let Some(dir) = dir else {
        println!(r#"{{"loggedIn":false}}"#);
        return 1;
    };
    report_env_names(dir, "status");
    if std::env::var("FIXTURE_STATUS_MODE").as_deref() == Ok("hang") {
        std::thread::sleep(Duration::from_secs(600));
    }
    let config_directory = dir.to_string_lossy();
    let answer = match std::env::var("FIXTURE_AUTH").as_deref() {
        Ok("api_key") => json!({
            "loggedIn": true,
            "authMethod": "api_key",
            "apiProvider": "firstParty",
            "apiKeySource": "ANTHROPIC_API_KEY",
            "configDirectory": config_directory,
        }),
        Ok("bedrock") => json!({
            "loggedIn": true,
            "authMethod": "third_party",
            "apiProvider": "bedrock",
            "configDirectory": config_directory,
        }),
        _ => {
            let credentials = dir.join(".credentials.json");
            let logged_in = credentials.exists();
            let email = std::fs::read(&credentials)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .and_then(|saved| saved["email"].as_str().map(str::to_string));
            json!({
                "loggedIn": logged_in,
                "authMethod": if logged_in { "claude.ai" } else { "none" },
                "apiProvider": "firstParty",
                "email": email,
                "subscriptionType": "max",
                "configDirectory": config_directory,
            })
        }
    };
    println!("{answer}");
    if answer["loggedIn"] == true { 0 } else { 1 }
}

fn auth_login(dir: Option<&Path>, args: &[&str]) -> i32 {
    let Some(dir) = dir
        .filter(|dir| dir.parent().and_then(Path::file_name) == Some(OsStr::new("agent-accounts")))
    else {
        return 1;
    };
    report_env_names(dir, "login");
    let given = serde_json::to_vec(args).expect("arguments serialize");
    std::fs::write(dir.join("fixture-login-args.json"), given)
        .expect("the account folder is writable");
    println!("Opening the browser to sign in.");
    let email = args
        .iter()
        .position(|arg| *arg == "--email")
        .and_then(|at| args.get(at + 1))
        .copied()
        .unwrap_or("fixture@example.invalid");
    let sign_in = || {
        let credentials = json!({ "email": email }).to_string();
        std::fs::write(dir.join(".credentials.json"), credentials)
            .expect("the account folder is writable");
        0
    };
    match std::env::var("FIXTURE_LOGIN_MODE")
        .as_deref()
        .unwrap_or("ok")
    {
        "ok" => sign_in(),
        "fail" => 7,
        "silent" => 0,
        "hang" => {
            println!("waiting");
            std::thread::sleep(Duration::from_secs(600));
            0
        }
        "prompt" => {
            println!("Paste code:");
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            if line.trim() == "yes" { sign_in() } else { 3 }
        }
        _ => 2,
    }
}

fn print_env(names: &[OsString]) {
    for name in names {
        println!("{}", env_line(&name.to_string_lossy()));
    }
}

fn env_line(name: &str) -> String {
    match std::env::var_os(name) {
        Some(value) if std::env::var_os("HOTLINE_TEST_AGENT_ACCOUNTS").is_some() => {
            format!("{name}={}", value.to_string_lossy())
        }
        Some(_) => format!("{name} set"),
        None => format!("{name} unset"),
    }
}

fn reported(said: &str) -> Option<Vec<String>> {
    let line = said.lines().find(|line| line.contains("REPORT_ENV "))?;
    let names = &line[line.find("REPORT_ENV ")? + "REPORT_ENV ".len()..];
    Some(names.split_whitespace().map(str::to_string).collect())
}

async fn report(names: &[String]) -> String {
    let mut lines: Vec<String> = names
        .iter()
        .map(|name| format!("ENV {}", env_line(name)))
        .collect();
    match printed_by_a_child(names).await {
        Ok(printed) => lines.extend(printed.lines().map(|line| format!("CHILD_ENV {line}"))),
        Err(error) => lines.push(format!("CHILD_ENV failed: {error}")),
    }
    lines.join("\n")
}

async fn printed_by_a_child(names: &[String]) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut command = tokio::process::Command::new(exe);
    command
        .arg("--print-env")
        .args(names)
        .stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output().await.map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!("it exited with {}", output.status));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
