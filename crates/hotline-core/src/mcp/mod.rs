//! Hotline as an MCP **client**: the servers the room knows, the grant a
//! teammate gets, and the live connections a session holds.
//!
//! Servers are defined once, under the room setting `mcpServers`, and
//! teammates reference them by id. That split is deliberate: a server is a
//! piece of infrastructure with a command in it, while which teammate may
//! use it is a question about that teammate. A granted tool is named
//! `{slug}__{remote}` from the server's human name, because a uuid is not
//! a namespace an agent can read; the ledger origin stays the id. An id
//! that no longer names a server is dropped rather than treated as an
//! error — deleting a server should not break every teammate that
//! referenced it — and the drop is recorded on the ledger so it is not
//! silent.
//!
//! A half-written entry in settings costs that one server, never every
//! teammate's tools. A server whose env is not a map of strings is refused
//! — a non-object env, or a value that is not a string, named on the
//! ledger — rather than started without those variables. OAuth HTTP is
//! connected only after an operator sign-in through the protected vault;
//! static-header HTTP is refused with a sentence saying why, not connected
//! with a dead credential. Bearer HTTP is process state the computer module
//! constructs, never a setting. A server that dies after it was attached is
//! the same honesty later: the next call that hits a dead transport marks
//! that origin's rows absent and says so once on the tape.
//!
//! Hotline's own teammate tools are the other half of MCP, and they live
//! in [`server`].

mod oauth;
pub mod server;
mod tool;

pub(crate) use oauth::{McpOAuthService, connect_oauth, manager_for_server};
use tool::Watch;
pub use tool::{CallContent, CallError, CallImage, McpTool};

use crate::contract::{McpPolicy, PolicyMode};
use crate::driver::CapabilityLease;
use reqwest::header::{AUTHORIZATION, HeaderName, HeaderValue};
use rmcp::ServiceExt;
use rmcp::model::{ClientInfo, Implementation};
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;
use tokio::process::Command;

/// How long a server may take to finish the handshake and list its tools
/// before the row is absent rather than the session hanging.
const HANDSHAKE: Duration = Duration::from_secs(20);

/// One MCP server the room knows, after a settings entry has been
/// normalised.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpServer {
    pub id: String,
    pub name: String,
    pub transport: McpTransport,
    /// Why this server must not be started. A non-string env value is the
    /// case we have: starting the process without that variable is worse
    /// than not starting it, so the ledger names the key instead.
    pub refuse: Option<String>,
}

/// How Hotline reaches a server. Auth that this build cannot honour still
/// parses, so the ledger can name the server it refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpTransport {
    Stdio {
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    },
    Http {
        url: String,
        auth: HttpAuth,
    },
}

/// How an HTTP server authenticates.
///
/// [`HttpAuth::None`], [`HttpAuth::Secret`], [`HttpAuth::Oauth`] and
/// [`HttpAuth::OauthConfigured`] are what settings parse to. A secret server
/// sends a token the person pasted, kept in the protected vault and never in
/// settings; OAuth servers use the same vault and are refused until the
/// operator signs in. Either is refused, with a sentence, until the vault
/// holds what it needs.
///
/// [`HttpAuth::Bearer`] is process state, never settings. The computer module
/// is the only constructor: a settings entry whose `auth` carries a token
/// still parses to [`HttpAuth::Secret`] with the token dropped. Serialising
/// this into the room stream would write the container's token next to the
/// roster.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpAuth {
    None,
    /// A token from the vault on every request: `Authorization: Bearer
    /// <token>` when `header` is `None`, else `<header>: <token>`.
    Secret {
        header: Option<String>,
    },
    Oauth,
    /// OAuth settings that include a public client id and/or requested
    /// scopes. The client secret is always in the vault, never here.
    OauthConfigured {
        scopes: Vec<String>,
        resource: Option<String>,
        client_id: Option<String>,
        token_endpoint_auth_method: Option<String>,
    },
    Bearer {
        token: String,
    },
}

/// A server the grant named that could not be attached, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailedServer {
    pub id: String,
    pub name: String,
    pub reason: String,
}

/// Live clients for one session, and the tools they listed.
///
/// The running services stay here for as long as the session: dropping them
/// closes the transport, which kills a stdio child, and dropping the groups
/// beside them reaches everything that child started. Tools clone a peer
/// handle; they are only as live as this value.
pub struct Connections {
    pub tools: Vec<McpTool>,
    pub failed: Vec<FailedServer>,
    _live: Vec<RunningService<rmcp::RoleClient, ClientInfo>>,
    /// The process groups the stdio servers were spawned into, killed when
    /// this value goes. See [`ProcessGroup`].
    _groups: Vec<ProcessGroup>,
}

#[cfg(test)]
impl Connections {
    pub(crate) fn for_test(tools: Vec<McpTool>, failed: Vec<FailedServer>) -> Self {
        Self {
            tools,
            failed,
            _live: Vec::new(),
            _groups: Vec::new(),
        }
    }
}

/// A stdio server's process group, killed when this is dropped.
///
/// rmcp's child transport kills the process it spawned and nothing else, and
/// the command in an `mcpServers` entry is very often a launcher — `npx -y
/// some-server`, `uvx …` — whose real server is a grandchild. Killing only the
/// wrapper reparents that grandchild to pid 1, where it holds the port, the
/// file locks and the memory for as long as Hotline runs. So the child is spawned
/// into a group of its own and the group is what a stop reaches, the same way
/// the ACP child and the shell tool do it.
struct ProcessGroup {
    #[cfg_attr(not(unix), allow(dead_code))]
    id: Option<u32>,
    #[cfg(windows)]
    _job: crate::process_windows::Job,
}

#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            // Safety: `killpg` reads no memory, and the group is the one this
            // connection made with `process_group(0)`.
            unsafe { libc::killpg(id as libc::pid_t, libc::SIGKILL) };
        }
    }
}

/// The servers this teammate's policy selects, and the ids it named that
/// no longer exist.
pub struct Grant {
    pub servers: Vec<McpServer>,
    pub missing: Vec<String>,
}

/// The server list stored under `mcpServers`, with anything unusable
/// dropped. Settings are a file a person can edit.
pub fn normalize_servers(value: &Value) -> Vec<Value> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(normalize_server)
        .collect()
}

pub(crate) fn validate_http_url(raw: &str) -> std::io::Result<()> {
    let url = url::Url::parse(raw)
        .map_err(|_| std::io::Error::other("Enter a full HTTP or HTTPS tool-source URL."))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(std::io::Error::other(
            "Tool-source URLs cannot contain credentials, a query, or a fragment. Use the authentication fields for secrets.",
        ));
    }
    Ok(())
}

/// Whether a stdio source still keeps its launch values on the room stream,
/// which the move into the credential store has not reached yet: a locked
/// store, or a room line it could not read, held it back.
pub(crate) fn launch_values_pending(server: &Value) -> bool {
    server["args"]
        .as_array()
        .is_some_and(|args| !args.is_empty())
        || server
            .get("env")
            .is_some_and(|env| env.as_object().is_none_or(|env| !env.is_empty()))
}

/// A locked store may leave legacy configuration on disk for recovery. Those
/// values still must not be copied into a window subscription or response.
pub(crate) fn public_servers(value: &Value) -> Value {
    let mut servers = normalize_servers(value);
    for server in &mut servers {
        if server["type"] == "http" {
            if validate_http_url(server["url"].as_str().unwrap_or_default()).is_err() {
                server["url"] = json!("");
                server["urlNeedsRepair"] = json!(true);
            }
            continue;
        }
        if launch_values_pending(server) {
            server["args"] = json!([]);
            server
                .as_object_mut()
                .expect("normalized server")
                .remove("env");
            server["launchValuesPending"] = json!(true);
        }
    }
    Value::Array(servers)
}

pub(crate) fn public_room_event(mut event: Value) -> Value {
    if event["kind"] == "setting" && event["id"] == "mcpServers" && event.get("value").is_some() {
        event["value"] = public_servers(&event["value"]);
    }
    event
}

/// Typed servers from the room's settings fold. A missing or unusable
/// `mcpServers` key is no servers, not an error.
pub fn servers(settings: &Map<String, Value>) -> Vec<McpServer> {
    settings
        .get("mcpServers")
        .map(normalize_servers)
        .unwrap_or_default()
        .iter()
        .filter_map(parse_server)
        .collect()
}

/// Which of `available` this policy actually gets.
///
/// An id that no longer names a server is listed in [`Grant::missing`]
/// rather than treated as a start failure: deleting a server should not
/// break every teammate that referenced it, and the ledger is where the
/// drop is said out loud.
pub fn grant(available: &[McpServer], policy: &McpPolicy) -> Grant {
    match policy.mode {
        PolicyMode::None => Grant {
            servers: Vec::new(),
            missing: Vec::new(),
        },
        PolicyMode::All => Grant {
            servers: available.to_vec(),
            missing: Vec::new(),
        },
        PolicyMode::Some => {
            let known: HashMap<&str, &McpServer> = available
                .iter()
                .map(|server| (server.id.as_str(), server))
                .collect();
            let mut servers = Vec::new();
            let mut missing = Vec::new();
            for id in &policy.server_ids {
                match known.get(id.as_str()) {
                    Some(server) => servers.push((*server).clone()),
                    None => missing.push(id.clone()),
                }
            }
            Grant { servers, missing }
        }
    }
}

/// Connect every granted server. A refusal or a handshake failure becomes a
/// [`FailedServer`]; the rest list their tools. `persona_id` is who the
/// ledger names when a transport dies after this, so a silent mid-session
/// absence is still a named one.
pub async fn connect(persona_id: &str, servers: &[McpServer]) -> Connections {
    connect_with_capability(persona_id, servers, None).await
}

/// Connects granted servers while binding every listed tool to the session's
/// capability generation. The optional lease keeps this function usable by
/// standalone MCP tests and by callers that deliberately have no room
/// session, while live drivers always pass one.
pub(crate) async fn connect_with_capability(
    persona_id: &str,
    servers: &[McpServer],
    capability: Option<CapabilityLease>,
) -> Connections {
    connect_with_capability_and_vault(persona_id, servers, capability, None).await
}

/// Connect granted servers with access to the protected MCP OAuth store.
/// Standalone callers without a vault retain the old refusal behavior.
pub(crate) async fn connect_with_capability_and_vault(
    persona_id: &str,
    servers: &[McpServer],
    capability: Option<CapabilityLease>,
    vault: Option<std::sync::Arc<crate::vault::Vault>>,
) -> Connections {
    let mut tools = Vec::new();
    let mut failed = Vec::new();
    let mut live = Vec::new();
    let watch = Watch::new(persona_id);
    let mut groups = Vec::new();
    // Prefixes for the whole grant, before any handshake, so a server that
    // fails to start still consumes its slot and the survivors keep the
    // names the policy's order promised.
    let prefixes = tool::prefixes(servers);
    for (server, prefix) in servers.iter().zip(&prefixes) {
        if capability
            .as_ref()
            .is_some_and(|capability| !capability.is_current())
        {
            break;
        }
        match connect_one(server, vault.clone()).await {
            Ok((client, listed, group)) => {
                if capability
                    .as_ref()
                    .is_some_and(|capability| !capability.is_current())
                {
                    drop(client);
                    break;
                }
                let peer = client.peer().clone();
                for definition in listed {
                    tools.push(
                        McpTool::new(
                            prefix,
                            &server.id,
                            &server.name,
                            definition,
                            peer.clone(),
                            watch.clone(),
                        )
                        .with_capability_opt(capability.clone()),
                    );
                }
                live.push(client);
                groups.extend(group);
            }
            Err(reason) => failed.push(FailedServer {
                id: server.id.clone(),
                name: server.name.clone(),
                reason,
            }),
        }
    }
    Connections {
        tools,
        failed,
        _live: live,
        _groups: groups,
    }
}

/// What the ledger says about a policy id that no longer names a server.
///
/// Both drivers write this row, and it has to read the same on either: the
/// question a person asks is "why does this teammate not have that tool",
/// and the answer does not depend on which agent they asked it about.
pub fn missing_reason(id: &str) -> String {
    format!(
        "this teammate's MCP policy names the server {id}, which no longer exists in app settings — every tool it supplied is gone"
    )
}

/// Why this build cannot hand this server to an agent, or `None` when it can.
///
/// One sentence, in one place, because two agents refuse for the same reason:
/// Hotline connects the server itself for the in-process agent and names it in
/// `session/new` for a child, and neither can honour a credential this build
/// does not keep.
pub fn unsupported(server: &McpServer) -> Option<String> {
    if let Some(reason) = &server.refuse {
        return Some(reason.clone());
    }
    match &server.transport {
        McpTransport::Http {
            auth: HttpAuth::Oauth,
            ..
        }
        | McpTransport::Http {
            auth: HttpAuth::OauthConfigured { .. },
            ..
        } => Some("OAuth HTTP servers require an operator sign-in in Settings; this server was not connected.".to_string()),
        // Bearer is constructed by the computer module, not by settings, and
        // is the one HTTP credential this build can actually present.
        _ => None,
    }
}

/// One server, connected. A stdio server also hands back the group it was
/// spawned into, which is what the caller has to hold on to.
async fn connect_one(
    server: &McpServer,
    vault: Option<std::sync::Arc<crate::vault::Vault>>,
) -> Result<
    (
        RunningService<rmcp::RoleClient, ClientInfo>,
        Vec<rmcp::model::Tool>,
        Option<ProcessGroup>,
    ),
    String,
> {
    let resolved;
    let server = if let Some(vault) = &vault {
        resolved = vault
            .resolve_mcp_server(server)
            .map_err(|error| error.to_string())?;
        &resolved
    } else {
        server
    };
    if matches!(
        &server.transport,
        McpTransport::Http {
            auth: HttpAuth::Oauth | HttpAuth::OauthConfigured { .. },
            ..
        }
    ) {
        let Some(vault) = vault else {
            return Err(
                "OAuth HTTP servers require an operator sign-in in Settings; this session has no credential vault."
                    .to_string(),
            );
        };
        let (client, listed) = connect_oauth(server, vault).await?;
        return Ok((client, listed, None));
    }
    if let Some(refusal) = unsupported(server) {
        return Err(refusal);
    }
    // OAuth was refused above. What is left of an HTTP server is a URL, and
    // a token: one the person pasted into the vault, or one the computer
    // module minted in this process.
    match &server.transport {
        McpTransport::Http { url, auth } => {
            let presented = match auth {
                HttpAuth::Secret { header } => {
                    let Some(token) = vault
                        .as_ref()
                        .ok_or_else(|| NO_SAVED_TOKEN.to_string())?
                        .mcp_secret(&server.id, url)
                        .map_err(|error| error.to_string())?
                    else {
                        return Err(NO_SAVED_TOKEN.to_string());
                    };
                    Some(Presented::new(header.as_deref(), &token)?)
                }
                HttpAuth::Bearer { token } => Some(Presented::new(None, token)?),
                _ => None,
            };
            let transport = http_transport(url, presented.as_ref());
            let (client, listed) = handshake(hotline_client().serve(transport)).await?;
            Ok((client, listed, None))
        }
        McpTransport::Stdio { command, args, env } => {
            let transport = TokioChildProcess::new(Command::new(command).configure(|cmd| {
                cmd.args(args);
                cmd.envs(env);
                #[cfg(windows)]
                crate::process_windows::prepare(cmd);
                #[cfg(unix)]
                cmd.process_group(0);
            }))
            .map_err(|error| {
                format!(
                    "{} ({}) could not be started: {error}",
                    server.name, server.id
                )
            })?;
            let group = ProcessGroup {
                id: transport.id(),
                #[cfg(windows)]
                _job: crate::process_windows::Job::attach(transport.id())
                    .map_err(|error| format!("Could not contain the MCP process tree: {error}"))?,
            };
            let (client, listed) = handshake(hotline_client().serve(transport)).await?;
            Ok((client, listed, Some(group)))
        }
    }
}

async fn handshake<E>(
    serve: impl Future<Output = Result<RunningService<rmcp::RoleClient, ClientInfo>, E>>,
) -> Result<
    (
        RunningService<rmcp::RoleClient, ClientInfo>,
        Vec<rmcp::model::Tool>,
    ),
    String,
>
where
    E: std::fmt::Display,
{
    let client = tokio::time::timeout(HANDSHAKE, serve)
        .await
        .map_err(|_| "the server did not finish the MCP handshake in time".to_string())?
        .map_err(|error| error.to_string())?;
    let listed = tokio::time::timeout(HANDSHAKE, client.list_all_tools())
        .await
        .map_err(|_| "the server did not list its tools in time".to_string())?
        .map_err(|error| error.to_string())?;
    Ok((client, listed))
}

fn hotline_client() -> ClientInfo {
    ClientInfo::new(
        Default::default(),
        Implementation::new("Hotline", env!("CARGO_PKG_VERSION")),
    )
}

/// A secret server whose token the vault does not hold yet.
pub const NO_SAVED_TOKEN: &str =
    "This server has no saved token; paste one in Settings → Tools. It was not connected.";

/// The credential one request carries: the header name and its whole value.
/// A bearer is `Authorization: Bearer <token>`; a named header is the token
/// as given. Built once per connection so a token that is not a legal
/// header value is refused before any request is sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Presented {
    pub(crate) name: HeaderName,
    pub(crate) value: HeaderValue,
}

impl Presented {
    pub(crate) fn new(header: Option<&str>, token: &str) -> Result<Self, String> {
        let (name, value) = match header {
            Some(name) => (
                HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
                    format!("{name:?} is not a header name; this server was not connected.")
                })?,
                HeaderValue::from_str(token),
            ),
            None => (
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {token}")),
            ),
        };
        let mut value = value.map_err(|_| {
            "The saved token is not a legal header value; this server was not connected."
                .to_string()
        })?;
        value.set_sensitive(true);
        Ok(Self { name, value })
    }
}

/// The streamable-HTTP client for one URL, with the credential on every
/// request. rmcp's `custom_headers` carries it whole, so a bearer and a named
/// header are one path.
pub(crate) fn http_config(
    url: &str,
    presented: Option<&Presented>,
) -> StreamableHttpClientTransportConfig {
    let mut config = StreamableHttpClientTransportConfig::with_uri(url.to_string());
    if let Some(presented) = presented {
        config
            .custom_headers
            .insert(presented.name.clone(), presented.value.clone());
    }
    config
}

fn http_transport(
    url: &str,
    presented: Option<&Presented>,
) -> StreamableHttpClientTransport<reqwest::Client> {
    StreamableHttpClientTransport::with_client(
        reqwest::Client::default(),
        http_config(url, presented),
    )
}

fn normalize_server(value: &Value) -> Option<Value> {
    let candidate = value.as_object()?;
    let id = candidate
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let name = candidate
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if name.is_empty() {
        return None;
    }

    if candidate.get("type").and_then(Value::as_str) == Some("http") {
        let url = candidate
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if url.is_empty() {
            return None;
        }
        let legacy_headers = string_map(candidate.get("headers"));
        let auth = normalize_http_auth(candidate.get("auth"), legacy_headers);
        return Some(json!({ "id": id, "type": "http", "name": name, "url": url, "auth": auth }));
    }

    let command = candidate
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if command.is_empty() {
        return None;
    }
    let args: Vec<&str> = candidate
        .get("args")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut server =
        json!({ "id": id, "type": "stdio", "name": name, "command": command, "args": args });
    // Keep env even when it is not an object of strings, so parse can
    // refuse the server rather than starting it without the variables.
    if let Some(env) = candidate.get("env") {
        server
            .as_object_mut()
            .expect("just built as an object")
            .insert("env".to_string(), env.clone());
    }
    if let Some(reference) = candidate.get("credentialRef") {
        server["credentialRef"] = reference.clone();
    }
    Some(server)
}

/// A legacy `headers` map (the old app kept header values in settings) keeps
/// its shape and loses its values: `Authorization` becomes bearer mode and
/// one other header becomes header mode, and the token has to be pasted
/// again in Settings, into the vault this time.
fn normalize_http_auth(value: Option<&Value>, legacy: Option<&Map<String, Value>>) -> Value {
    if let Some(legacy) = legacy {
        let mut names = legacy.keys().filter(|name| !name.is_empty());
        return match (names.next(), names.next()) {
            (Some(name), None) if !name.eq_ignore_ascii_case("authorization") => {
                json!({ "mode": "header", "name": name })
            }
            (Some(_), _) => json!({ "mode": "bearer" }),
            (None, _) => json!({ "mode": "none" }),
        };
    }
    let Some(candidate) = value.and_then(Value::as_object) else {
        return json!({ "mode": "none" });
    };
    match candidate.get("mode").and_then(Value::as_str) {
        Some("bearer") => json!({ "mode": "bearer" }),
        Some("header") => match candidate
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            Some(name) => json!({ "mode": "header", "name": name }),
            None => json!({ "mode": "none" }),
        },
        Some("oauth") => {
            let scopes: Vec<String> = unique_keep_first(
                candidate
                    .get("scopes")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|scope| !scope.is_empty())
                    .map(str::to_string),
            );
            let resource = candidate
                .get("resource")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let raw_client = candidate.get("client").and_then(Value::as_object);
            let method = raw_client
                .and_then(|client| client.get("tokenEndpointAuthMethod"))
                .and_then(Value::as_str)
                .filter(|method| {
                    matches!(
                        *method,
                        "none" | "client_secret_basic" | "client_secret_post"
                    )
                });
            let client = raw_client
                .and_then(|client| client.get("clientId").and_then(Value::as_str))
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(|client_id| {
                    let mut object = Map::new();
                    object.insert("clientId".to_string(), json!(client_id));
                    if let Some(method) = method {
                        object.insert("tokenEndpointAuthMethod".to_string(), json!(method));
                    }
                    Value::Object(object)
                });
            let mut auth = Map::new();
            auth.insert("mode".to_string(), json!("oauth"));
            auth.insert("scopes".to_string(), json!(scopes));
            if let Some(resource) = resource {
                auth.insert("resource".to_string(), json!(resource));
            }
            if let Some(client) = client {
                auth.insert("client".to_string(), client);
            }
            Value::Object(auth)
        }
        _ => json!({ "mode": "none" }),
    }
}

fn parse_server(value: &Value) -> Option<McpServer> {
    let object = value.as_object()?;
    let id = object.get("id")?.as_str()?.to_string();
    let name = object.get("name")?.as_str()?.to_string();
    let transport = match object.get("type").and_then(Value::as_str) {
        Some("http") => {
            let url = object.get("url")?.as_str()?.to_string();
            let auth = match object
                .get("auth")
                .and_then(Value::as_object)
                .and_then(|auth| auth.get("mode"))
                .and_then(Value::as_str)
            {
                Some("bearer") => HttpAuth::Secret { header: None },
                Some("header") => {
                    let name = object
                        .get("auth")
                        .and_then(Value::as_object)
                        .and_then(|auth| auth.get("name"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .map(str::to_string);
                    match name {
                        Some(name) => HttpAuth::Secret { header: Some(name) },
                        None => HttpAuth::None,
                    }
                }
                Some("oauth") => {
                    let auth = object
                        .get("auth")
                        .and_then(Value::as_object)
                        .expect("oauth auth was an object");
                    let scopes: Vec<String> = auth
                        .get("scopes")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect();
                    let resource = auth
                        .get("resource")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    let client = auth.get("client").and_then(Value::as_object);
                    let client_id = client
                        .and_then(|client| client.get("clientId"))
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    let token_endpoint_auth_method = client
                        .and_then(|client| client.get("tokenEndpointAuthMethod"))
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    if scopes.is_empty()
                        && resource.is_none()
                        && client_id.is_none()
                        && token_endpoint_auth_method.is_none()
                    {
                        HttpAuth::Oauth
                    } else {
                        HttpAuth::OauthConfigured {
                            scopes,
                            resource,
                            client_id,
                            token_endpoint_auth_method,
                        }
                    }
                }
                _ => HttpAuth::None,
            };
            McpTransport::Http { url, auth }
        }
        _ => {
            let command = object.get("command")?.as_str()?.to_string();
            let args = object
                .get("args")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            let (env, refuse) = stdio_env(object.get("env"));
            return Some(McpServer {
                id,
                name,
                transport: McpTransport::Stdio { command, args, env },
                refuse,
            });
        }
    };
    let refuse = match &transport {
        McpTransport::Http { url, .. } => {
            validate_http_url(url).err().map(|error| error.to_string())
        }
        _ => None,
    };
    Some(McpServer {
        id,
        name,
        transport,
        refuse,
    })
}

/// Env values have to be strings: a number in the map used to drop the whole
/// env, which starts the server without the token that number was standing
/// in for.
fn stdio_env(value: Option<&Value>) -> (HashMap<String, String>, Option<String>) {
    let Some(value) = value else {
        return (HashMap::new(), None);
    };
    let Some(object) = value.as_object() else {
        return (
            HashMap::new(),
            Some("env is not a map of strings; the server was not started.".to_string()),
        );
    };
    let mut env = HashMap::new();
    for (key, value) in object {
        match value.as_str() {
            Some(text) => {
                env.insert(key.clone(), text.to_string());
            }
            None => {
                return (
                    HashMap::new(),
                    Some(format!(
                        "The env value for {key} is not a string; the server was not started."
                    )),
                );
            }
        }
    }
    (env, None)
}

fn string_map(value: Option<&Value>) -> Option<&Map<String, Value>> {
    let object = value?.as_object()?;
    object.values().all(Value::is_string).then_some(object)
}

fn unique_keep_first(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::CapabilityEpoch;
    use rmcp::handler::server::ServerHandler;
    use rmcp::model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
    };
    use rmcp::service::RequestContext;
    use serde_json::json;
    use std::future::Future;

    fn shout_schema() -> std::sync::Arc<rmcp::model::JsonObject> {
        let schema = json!({
            "type": "object",
            "properties": { "text": { "type": "string" } },
            "required": ["text"],
        });
        std::sync::Arc::new(schema.as_object().unwrap().clone())
    }

    struct Echo;

    impl ServerHandler for Echo {
        fn get_info(&self) -> ServerInfo {
            ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
        }

        fn list_tools(
            &self,
            _request: Option<PaginatedRequestParams>,
            _context: RequestContext<rmcp::RoleServer>,
        ) -> impl Future<Output = Result<ListToolsResult, rmcp::ErrorData>> + Send + '_ {
            std::future::ready(Ok(ListToolsResult::with_all_items(vec![Tool::new(
                "shout",
                "Echo the text back in upper case.",
                shout_schema(),
            )])))
        }

        fn call_tool(
            &self,
            request: CallToolRequestParams,
            _context: RequestContext<rmcp::RoleServer>,
        ) -> impl Future<Output = Result<CallToolResponse, rmcp::ErrorData>> + Send + '_ {
            let text = request
                .arguments
                .as_ref()
                .and_then(|args| args.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if text == "fail" {
                return std::future::ready(Ok(CallToolResult::error(vec![ContentBlock::text(
                    "the tool refused",
                )])
                .into()));
            }
            if text == "rpc" {
                return std::future::ready(Err(rmcp::ErrorData::invalid_params(
                    "the tool rejected the arguments",
                    None,
                )));
            }
            if text == "picture" {
                return std::future::ready(Ok(CallToolResult::success(vec![
                    ContentBlock::text("the tree"),
                    ContentBlock::image("AAAA", "image/png"),
                ])
                .into()));
            }
            std::future::ready(Ok(CallToolResult::success(vec![ContentBlock::text(
                text.to_uppercase(),
            )])
            .into()))
        }
    }

    async fn echo_on_duplex(
        persona_id: &str,
    ) -> (
        tokio::task::JoinHandle<()>,
        rmcp::service::RunningService<rmcp::RoleClient, ClientInfo>,
        McpTool,
    ) {
        let (client_to_server, server_from_client) = tokio::io::duplex(8192);
        let server = tokio::spawn(async move {
            let _ = Echo
                .serve(server_from_client)
                .await
                .expect("echo server starts")
                .waiting()
                .await;
        });
        let client = hotline_client()
            .serve(client_to_server)
            .await
            .expect("client handshake");
        let listed = client.list_all_tools().await.expect("tools listed");
        let tool = McpTool::new(
            "echo",
            "echo",
            "Echo",
            listed.into_iter().next().unwrap(),
            client.peer().clone(),
            Watch::new(persona_id),
        );
        (server, client, tool)
    }

    fn publish_echo(persona_id: &str) {
        use crate::contract::{AgentKind, ToolSourceKind};
        use crate::session::ledger::ToolLedger;
        let mut ledger = ToolLedger::new(persona_id, AgentKind::Hotline, "hotline");
        ledger
            .verified(
                ToolSourceKind::Mcp,
                "echo",
                "echo__shout",
                "attached from the echo MCP server",
            )
            .verified(
                ToolSourceKind::Mcp,
                "echo",
                "echo__whisper",
                "attached from the echo MCP server",
            )
            .verified(ToolSourceKind::Builtin, "hotline", "read", "a built-in")
            .publish();
    }

    fn echo_row(persona_id: &str, name: &str) -> crate::contract::ToolLedgerRow {
        crate::session::ledger::teammate_tools(persona_id)
            .expect("published")
            .rows
            .into_iter()
            .find(|row| row.name == name)
            .unwrap_or_else(|| panic!("{name} is on the ledger"))
    }

    fn named(id: &str, name: &str) -> McpServer {
        McpServer {
            id: id.into(),
            name: name.into(),
            transport: McpTransport::Stdio {
                command: "echo".into(),
                args: Vec::new(),
                env: HashMap::new(),
            },
            refuse: None,
        }
    }

    #[test]
    fn a_server_name_slugs_to_lowercase_ascii_letters_digits_and_underscore() {
        use super::tool::{slug, tool_name};
        assert_eq!(slug("GitHub Search"), "github_search");
        assert_eq!(slug("v2 API"), "v2_api");
        assert_eq!(slug("foo-bar.baz"), "foo_bar_baz");
        assert_eq!(slug("  Foo  Bar  "), "foo_bar");
        assert_eq!(slug("___Hello!!World___"), "hello_world");
        // é and the emoji are one run of non-ASCII, so they become one `_`.
        assert_eq!(slug("Café ☕ Search"), "caf_search");
        assert_eq!(slug("日本語"), "server");
        assert_eq!(slug("---"), "server");
        assert_eq!(slug(""), "server");
        assert_eq!(slug("___"), "server");

        assert_eq!(
            tool_name("github_search", "search"),
            "github_search__search"
        );
        // 20-char prefix + __ + 50-char remote is 72; the prefix keeps 12.
        let remote_50 = "r".repeat(50);
        assert_eq!(
            tool_name(&"p".repeat(20), &remote_50),
            format!("{}__{remote_50}", "p".repeat(12))
        );
        // 64 - 2 - 56 = 6, under eight, so the first eight of the prefix stay.
        let remote_56 = "r".repeat(56);
        let name = tool_name(&"p".repeat(20), &remote_56);
        assert_eq!(name, format!("{}__{remote_56}", "p".repeat(8)));
        assert!(name.len() > 64);
        // A short prefix is kept whole when the remote leaves room.
        assert_eq!(tool_name("echo", "shout"), "echo__shout");
    }

    #[test]
    fn colliding_slugs_take_a_suffix_in_grant_order() {
        let servers = [
            named("a", "GitHub Search"),
            named("b", "GitHub Search"),
            named("c", "github_search"),
            named("d", "Other"),
        ];
        assert_eq!(
            super::tool::prefixes(&servers),
            [
                "github_search",
                "github_search_2",
                "github_search_3",
                "other"
            ]
        );
        // A name that slugs to a suffix already taken skips that number.
        let taken = [named("a", "foo_2"), named("b", "Foo"), named("c", "Foo")];
        assert_eq!(super::tool::prefixes(&taken), ["foo_2", "foo", "foo_3"]);
    }

    #[test]
    fn how_to_use_says_granted_tools_are_named_server_tool() {
        assert!(
            server::HOW_TO_USE.contains("<server>__<tool>"),
            "{}",
            server::HOW_TO_USE
        );
    }

    #[test]
    fn a_hand_edited_bad_entry_costs_that_entry_not_the_list() {
        let raw = json!([
            { "id": "no-name", "type": "stdio", "command": "echo" },
            { "id": "good", "type": "stdio", "name": "Good", "command": "run" },
            { "id": "no-url", "type": "http", "name": "Remote" },
        ]);
        let servers = normalize_servers(&raw);
        let ids: Vec<&str> = servers
            .iter()
            .map(|server| server["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["good"]);
    }

    #[test]
    fn a_legacy_header_bearing_server_normalises_with_no_header_key() {
        let raw = json!([{
            "id": "srv1",
            "type": "http",
            "name": "Legacy",
            "url": "https://example.test",
            "headers": { "Authorization": "Bearer secret" },
        }]);
        let server = &normalize_servers(&raw)[0];
        assert_eq!(
            *server,
            json!({
                "id": "srv1",
                "type": "http",
                "name": "Legacy",
                "url": "https://example.test",
                "auth": { "mode": "bearer" },
            })
        );
        assert!(server.get("headers").is_none());
    }

    #[test]
    fn a_policy_of_none_selects_no_servers() {
        let available = vec![McpServer {
            id: "echo".into(),
            name: "Echo".into(),
            transport: McpTransport::Stdio {
                command: "echo".into(),
                args: Vec::new(),
                env: HashMap::new(),
            },
            refuse: None,
        }];
        let granted = grant(
            &available,
            &McpPolicy {
                mode: PolicyMode::None,
                server_ids: vec!["echo".into()],
            },
        );
        assert!(granted.servers.is_empty());
        assert!(granted.missing.is_empty());
    }

    #[test]
    fn a_some_policy_names_the_ids_that_no_longer_exist() {
        let available = vec![McpServer {
            id: "echo".into(),
            name: "Echo".into(),
            transport: McpTransport::Stdio {
                command: "echo".into(),
                args: Vec::new(),
                env: HashMap::new(),
            },
            refuse: None,
        }];
        let granted = grant(
            &available,
            &McpPolicy {
                mode: PolicyMode::Some,
                server_ids: vec!["echo".into(), "gone".into()],
            },
        );
        assert_eq!(granted.servers.len(), 1);
        assert_eq!(granted.missing, ["gone"]);
    }

    #[tokio::test]
    async fn a_duplex_server_lists_its_tool_and_a_scripted_call_reaches_it() {
        let (client_to_server, server_from_client) = tokio::io::duplex(8192);
        let server = tokio::spawn(async move {
            Echo.serve(server_from_client)
                .await
                .expect("echo server starts")
                .waiting()
                .await
                .expect("echo server ends cleanly");
        });

        let client = hotline_client()
            .serve(client_to_server)
            .await
            .expect("client handshake");
        let listed = client.list_all_tools().await.expect("tools listed");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name.as_ref(), "shout");

        let tool = McpTool::new(
            "echo",
            "3f9c1b2e-0000-4000-8000-000000000001",
            "Echo",
            listed.into_iter().next().unwrap(),
            client.peer().clone(),
            Watch::new("duplex-echo"),
        );
        assert_eq!(tool.name, "echo__shout");
        assert_eq!(tool.origin, "3f9c1b2e-0000-4000-8000-000000000001");
        let shouted = tool
            .call(json!({ "text": "harbour" }))
            .await
            .expect("the call reached the server");
        assert_eq!(shouted.text, "HARBOUR");
        assert!(shouted.images.is_empty());

        client.cancel().await.ok();
        let _ = server.await;
    }

    #[tokio::test]
    async fn a_server_dropped_after_the_first_call_marks_its_rows_absent_once() {
        use crate::contract::ToolState;
        let persona_id = "mcp-gone-duplex";
        publish_echo(persona_id);
        let (server, client, tool) = echo_on_duplex(persona_id).await;
        let shouted = tool
            .call(json!({ "text": "harbour" }))
            .await
            .expect("the first call reached the server");
        assert_eq!(shouted.text, "HARBOUR");
        assert_eq!(
            echo_row(persona_id, "echo__shout").state,
            ToolState::Verified
        );

        drop(client);
        server.abort();

        let err = tool
            .call(json!({ "text": "harbour" }))
            .await
            .expect_err("the dropped server cannot answer");
        let CallError::Transport {
            notice: Some(text), ..
        } = &err
        else {
            panic!("a dead transport is a transport error, not {err:?}");
        };
        assert!(text.starts_with("The Echo MCP server went away:"), "{text}");
        assert!(
            text.contains("Its tools are gone until the teammate restarts."),
            "{text}"
        );

        let shout = echo_row(persona_id, "echo__shout");
        assert_eq!(shout.state, ToolState::Absent);
        assert!(!shout.reason.is_empty());
        assert_eq!(
            echo_row(persona_id, "echo__whisper").state,
            ToolState::Absent
        );
        assert_eq!(echo_row(persona_id, "read").state, ToolState::Verified);

        let err = tool
            .call(json!({ "text": "harbour" }))
            .await
            .expect_err("still gone");
        assert!(
            matches!(err, CallError::Transport { notice: None, .. }),
            "the notice lands once: {err:?}"
        );
    }

    #[tokio::test]
    async fn a_tool_level_error_leaves_the_ledger_verified() {
        use crate::contract::ToolState;
        let persona_id = "mcp-tool-error";
        publish_echo(persona_id);
        let (server, client, tool) = echo_on_duplex(persona_id).await;

        let err = tool
            .call(json!({ "text": "fail" }))
            .await
            .expect_err("the tool refused");
        assert!(
            matches!(&err, CallError::Tool(message) if message.contains("refused")),
            "{err:?}"
        );
        assert_eq!(
            echo_row(persona_id, "echo__shout").state,
            ToolState::Verified
        );

        let err = tool
            .call(json!({ "text": "rpc" }))
            .await
            .expect_err("the server rejected the call");
        assert!(matches!(err, CallError::Tool(_)), "{err:?}");
        assert_eq!(
            echo_row(persona_id, "echo__shout").state,
            ToolState::Verified
        );

        client.cancel().await.ok();
        let _ = server.await;
    }

    #[tokio::test]
    async fn a_revoked_mcp_tool_refuses_before_calling_its_server() {
        let (server, client, tool) = echo_on_duplex("mcp-revoked").await;
        let epoch = CapabilityEpoch::default();
        let guarded = tool.with_capability_opt(Some(epoch.lease()));
        epoch.invalidate();

        let error = guarded
            .call(json!({ "text": "canary" }))
            .await
            .expect_err("a handle from a revoked session cannot call MCP");
        assert!(
            matches!(&error, CallError::Tool(message) if message.contains("capabilities have been revoked")),
            "{error:?}"
        );

        client.cancel().await.ok();
        let _ = server.await;
    }

    #[test]
    fn a_token_is_one_header_on_the_transport_bearer_or_named() {
        let bearer = Presented::new(None, "secret-token").unwrap();
        let config = super::http_config("http://127.0.0.1:9/mcp", Some(&bearer));
        assert_eq!(
            config
                .custom_headers
                .get(&AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer secret-token")
        );
        let named = Presented::new(Some("X-API-Key"), "secret-token").unwrap();
        let config = super::http_config("http://127.0.0.1:9/mcp", Some(&named));
        assert_eq!(
            config
                .custom_headers
                .get(&HeaderName::from_static("x-api-key"))
                .and_then(|value| value.to_str().ok()),
            Some("secret-token")
        );
        assert!(named.value.is_sensitive());
        assert!(Presented::new(Some("not a header"), "token").is_err());
        assert!(Presented::new(None, "line\nbreak").is_err());
        let none = super::http_config("http://127.0.0.1:9/mcp", None);
        assert!(none.custom_headers.is_empty());
    }

    #[test]
    fn bearer_never_appears_when_settings_are_parsed() {
        let mut settings = Map::new();
        settings.insert(
            "mcpServers".into(),
            json!([
                {
                    "id": "plain",
                    "type": "http",
                    "name": "Plain",
                    "url": "https://example.test/mcp",
                },
                {
                    "id": "claimed",
                    "type": "http",
                    "name": "Claimed",
                    "url": "https://example.test/mcp",
                    "auth": { "mode": "bearer", "token": "secret" },
                },
                {
                    "id": "legacy",
                    "type": "http",
                    "name": "Legacy",
                    "url": "https://example.test/mcp",
                    "headers": { "Authorization": "Bearer secret" },
                },
            ]),
        );
        let listed = servers(&settings);
        assert_eq!(listed.len(), 3);
        for server in &listed {
            match &server.transport {
                McpTransport::Http {
                    auth: HttpAuth::Bearer { .. },
                    ..
                } => panic!("{} parsed as Bearer from settings", server.id),
                McpTransport::Http {
                    auth: HttpAuth::None,
                    ..
                } if server.id == "plain" => {}
                McpTransport::Http {
                    auth: HttpAuth::Secret { header: None },
                    ..
                } if server.id == "claimed" || server.id == "legacy" => {}
                other => panic!("{} parsed as {other:?}", server.id),
            }
        }
    }

    #[tokio::test]
    async fn oauth_and_a_secret_server_without_its_token_are_refused_with_a_sentence() {
        let oauth = McpServer {
            id: "oauth".into(),
            name: "OAuth".into(),
            transport: McpTransport::Http {
                url: "https://example.test".into(),
                auth: HttpAuth::Oauth,
            },
            refuse: None,
        };
        let bearer = McpServer {
            id: "bearer".into(),
            name: "Bearer".into(),
            transport: McpTransport::Http {
                url: "https://example.test".into(),
                auth: HttpAuth::Secret { header: None },
            },
            refuse: None,
        };
        let connected = connect("oauth-refuse", &[oauth, bearer]).await;
        assert!(connected.tools.is_empty());
        assert_eq!(connected.failed.len(), 2);
        assert!(connected.failed[0].reason.contains("OAuth"));
        assert_eq!(connected.failed[1].reason, NO_SAVED_TOKEN);
    }

    #[test]
    fn a_legacy_header_map_keeps_its_shape_and_drops_its_values() {
        let one = Map::from_iter([("X-API-Key".to_string(), json!("secret"))]);
        assert_eq!(
            normalize_http_auth(None, Some(&one)),
            json!({ "mode": "header", "name": "X-API-Key" })
        );
        let auth = Map::from_iter([("Authorization".to_string(), json!("Bearer secret"))]);
        assert_eq!(
            normalize_http_auth(None, Some(&auth)),
            json!({ "mode": "bearer" })
        );
        let several = Map::from_iter([
            ("X-A".to_string(), json!("1")),
            ("X-B".to_string(), json!("2")),
        ]);
        assert_eq!(
            normalize_http_auth(None, Some(&several)),
            json!({ "mode": "bearer" })
        );
        assert_eq!(
            normalize_http_auth(Some(&json!({ "mode": "header" })), None),
            json!({ "mode": "none" })
        );
    }

    #[test]
    fn a_non_string_env_value_refuses_the_server_and_names_the_key() {
        let mut settings = Map::new();
        settings.insert(
            "mcpServers".into(),
            json!([{
                "id": "needs-token",
                "type": "stdio",
                "name": "Needs token",
                "command": "/bin/true",
                "env": { "API_TOKEN": 1, "OTHER": "ok" },
            }]),
        );
        let listed = servers(&settings);
        assert_eq!(listed.len(), 1);
        let reason = unsupported(&listed[0]).expect("the server should be refused");
        assert!(
            reason.contains("API_TOKEN"),
            "the refusal did not name the key: {reason}"
        );
    }

    #[test]
    fn a_non_object_env_refuses_the_server() {
        let mut settings = Map::new();
        settings.insert(
            "mcpServers".into(),
            json!([{
                "id": "needs-token",
                "type": "stdio",
                "name": "Needs token",
                "command": "/bin/true",
                "env": ["A=b"],
            }]),
        );
        let listed = servers(&settings);
        assert_eq!(listed.len(), 1);
        let reason = unsupported(&listed[0]).expect("the server should be refused");
        assert_eq!(
            reason,
            "env is not a map of strings; the server was not started."
        );
    }

    #[tokio::test]
    async fn a_non_string_env_value_is_an_absent_row_not_a_started_server() {
        let mut settings = Map::new();
        settings.insert(
            "mcpServers".into(),
            json!([{
                "id": "needs-token",
                "type": "stdio",
                "name": "Needs token",
                "command": "/bin/true",
                "env": { "API_TOKEN": 1 },
            }]),
        );
        let listed = servers(&settings);
        let connected = connect("needs-token", &listed).await;
        assert!(connected.tools.is_empty());
        assert_eq!(connected.failed.len(), 1);
        assert_eq!(connected.failed[0].id, "needs-token");
        assert!(
            connected.failed[0].reason.contains("API_TOKEN"),
            "{}",
            connected.failed[0].reason
        );
    }

    #[tokio::test]
    async fn a_server_that_fails_to_start_is_a_failed_row() {
        let missing = McpServer {
            id: "gone".into(),
            name: "Gone".into(),
            transport: McpTransport::Stdio {
                command: format!(
                    "/no-such-hotline-mcp-server-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ),
                args: Vec::new(),
                env: HashMap::new(),
            },
            refuse: None,
        };
        let connected = connect("gone", &[missing]).await;
        assert!(connected.tools.is_empty());
        assert_eq!(connected.failed.len(), 1);
        assert!(!connected.failed[0].reason.trim().is_empty());
    }

    #[tokio::test]
    async fn a_picture_result_keeps_the_image_beside_the_text() {
        let (server, client, tool) = echo_on_duplex("mcp-picture").await;
        let content = tool
            .call(json!({ "text": "picture" }))
            .await
            .expect("the call reached the server");
        assert_eq!(content.text, "the tree");
        assert_eq!(
            content.images,
            [CallImage {
                data: "AAAA".into(),
                mime_type: "image/png".into(),
            }]
        );
        client.cancel().await.ok();
        let _ = server.await;
    }

    /// The eight desktop tools, as the computer image lists them.
    const COMPUTER_TOOLS: [&str; 8] = [
        "capture", "input", "browser", "shell", "files", "windows", "wait", "state",
    ];

    struct Desktop;

    impl ServerHandler for Desktop {
        fn get_info(&self) -> ServerInfo {
            ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
        }

        fn list_tools(
            &self,
            _request: Option<PaginatedRequestParams>,
            _context: RequestContext<rmcp::RoleServer>,
        ) -> impl Future<Output = Result<ListToolsResult, rmcp::ErrorData>> + Send + '_ {
            let schema = shout_schema();
            std::future::ready(Ok(ListToolsResult::with_all_items(
                COMPUTER_TOOLS
                    .iter()
                    .map(|name| Tool::new(*name, *name, schema.clone()))
                    .collect(),
            )))
        }

        fn call_tool(
            &self,
            _request: CallToolRequestParams,
            _context: RequestContext<rmcp::RoleServer>,
        ) -> impl Future<Output = Result<CallToolResponse, rmcp::ErrorData>> + Send + '_ {
            std::future::ready(Ok(CallToolResult::success(vec![
                ContentBlock::image("AAAA", "image/png"),
                ContentBlock::text("the tree"),
            ])
            .into()))
        }
    }

    fn test_persona(id: &str) -> crate::contract::Persona {
        crate::contract::Persona {
            node: None,
            id: id.to_string(),
            name: "Ada".to_string(),
            goal: String::new(),
            face: None,
            team: None,
            backend_id: "hotline".to_string(),
            cwd: "/".to_string(),
            reach: Some(crate::contract::Reach::Machine),
            model_id: None,
            mode_id: None,
            effort_id: None,
            harness_override: None,
            hop_notice: None,
            mcp_policy: crate::contract::McpPolicy {
                mode: crate::contract::PolicyMode::All,
                server_ids: Vec::new(),
            },
            skill_policy: Default::default(),
            background_work: false,
            allowed_senders: Vec::new(),
            web_search_policy: None,
            computer: None,
            session_checkpoints: Vec::new(),
            last_session_id: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[tokio::test]
    async fn a_computer_server_writes_its_eight_tools_on_the_ledger_under_origin_computer() {
        use crate::contract::{ToolSourceKind, ToolState};
        use crate::driver::rig::publish_ledger;

        let persona_id = "mcp-computer-ledger";
        let (client_to_server, server_from_client) = tokio::io::duplex(8192);
        let server = tokio::spawn(async move {
            let _ = Desktop
                .serve(server_from_client)
                .await
                .expect("desktop server starts")
                .waiting()
                .await;
        });
        let client = hotline_client()
            .serve(client_to_server)
            .await
            .expect("client handshake");
        let listed = client.list_all_tools().await.expect("tools listed");
        let watch = Watch::new(persona_id);
        let tools: Vec<McpTool> = listed
            .into_iter()
            .map(|definition| {
                McpTool::new(
                    "computer",
                    crate::computer::SERVER_ID,
                    "Computer",
                    definition,
                    client.peer().clone(),
                    watch.clone(),
                )
            })
            .collect();
        assert_eq!(tools.len(), 8, "{}", tools.len());

        publish_ledger(
            &test_persona(persona_id),
            &[],
            &Connections::for_test(tools, Vec::new()),
            false,
        );
        let rows = crate::session::ledger::teammate_tools(persona_id)
            .expect("published")
            .rows;
        let computer: Vec<_> = rows
            .iter()
            .filter(|row| row.origin == crate::computer::SERVER_ID)
            .collect();
        assert_eq!(computer.len(), 8, "{rows:?}");
        for name in COMPUTER_TOOLS {
            let row = computer
                .iter()
                .find(|row| row.name == format!("computer__{name}"))
                .unwrap_or_else(|| panic!("computer__{name} is on the ledger: {rows:?}"));
            assert_eq!(row.source, ToolSourceKind::Mcp);
            assert_eq!(row.state, ToolState::Verified);
            assert!(!row.reason.is_empty());
        }

        client.cancel().await.ok();
        let _ = server.await;
    }
}
