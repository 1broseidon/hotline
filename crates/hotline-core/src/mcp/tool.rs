//! One MCP tool as Hotline Agent calls it.
//!
//! The name is `{slug}__{remote}` so two servers that both expose `search`
//! do not collide, and the slug is the server's human name — a uuid is not
//! a namespace an agent can read. Description and input schema pass through
//! as the server listed them. A call is forwarded, and the result's content
//! is what the model sees: text blocks joined, plus every image block. A
//! transport failure or an `isError` result is `Err`; only the transport
//! case marks the origin absent, because that is the server going away, not
//! the tool answering.

use crate::contract::ToolSourceKind;
use crate::driver::CapabilityLease;
use crate::session::ledger;
use rmcp::RoleClient;
use rmcp::ServiceError;
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::Peer;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

use super::McpServer;

/// The strictest provider cap on a tool name: `^[a-zA-Z0-9_-]{1,64}$`.
const MAX_TOOL_NAME: usize = 64;
/// A prefix shorter than this is not a namespace an agent can recognise.
const MIN_PREFIX: usize = 8;
const SEAM: &str = "__";

/// Lowercase ASCII letters, digits and `_`. Every other run of characters
/// becomes one `_`; empty after trim is `server`.
pub(crate) fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut in_other = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            if in_other {
                out.push('_');
                in_other = false;
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            in_other = true;
        }
    }
    if in_other {
        out.push('_');
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "server".to_string()
    } else {
        trimmed.to_string()
    }
}

/// The tool-name prefix for each granted server, in this list's order —
/// grant order, which is the order [`super::connect`] walks.
///
/// The first server that slugs to a name keeps it; later collisions get
/// `_2`, `_3`… on that same base. A later server whose own slug is already
/// taken (a name that slugs to `foo_2` after two `Foo`s) takes the next
/// free suffix, so every prefix in one connect is unique.
pub(crate) fn prefixes(servers: &[McpServer]) -> Vec<String> {
    let mut used = HashSet::new();
    let mut out = Vec::with_capacity(servers.len());
    for server in servers {
        let base = slug(&server.name);
        let mut prefix = base.clone();
        let mut n = 2u32;
        while !used.insert(prefix.clone()) {
            prefix = format!("{base}_{n}");
            n += 1;
        }
        out.push(prefix);
    }
    out
}

/// `{prefix}__{remote}`. A name longer than 64 shortens the prefix, never
/// the remote; if that would leave fewer than eight characters of prefix,
/// the first eight of the prefix stay so the namespace is still readable.
pub(crate) fn tool_name(prefix: &str, remote: &str) -> String {
    let room = MAX_TOOL_NAME.saturating_sub(SEAM.len() + remote.len());
    let keep = if room < MIN_PREFIX {
        MIN_PREFIX.min(prefix.len())
    } else {
        prefix.len().min(room)
    };
    format!("{}{SEAM}{remote}", &prefix[..keep])
}

/// Why a call did not succeed. Both variants' text is what the model sees.
#[derive(Debug)]
pub enum CallError {
    /// The server answered this failure — a tool `isError` result or a
    /// JSON-RPC error — or the call failed without the origin disappearing
    /// (a timeout, a cancel, an unexpected response).
    Tool(String),
    /// The transport died. `notice` is the tape sentence the first time this
    /// origin has gone away this session.
    Transport {
        message: String,
        notice: Option<String>,
    },
}

impl CallError {
    fn message(&self) -> &str {
        match self {
            Self::Tool(message) | Self::Transport { message, .. } => message,
        }
    }
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for CallError {}

/// One image a tool returned: the server's base64 payload and the mime type
/// it named. The caller decides whether the model sees it and whether the
/// tape stores a frame; this is just what the server sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallImage {
    pub data: String,
    pub mime_type: String,
}

/// A successful tool result, still as content blocks rather than a string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallContent {
    pub text: String,
    pub images: Vec<CallImage>,
}

/// Shared across every tool of one session, so two tools from the same
/// dead server write one notice, not one each.
pub(crate) struct Watch {
    persona_id: String,
    announced: Mutex<HashSet<String>>,
}

impl Watch {
    pub(crate) fn new(persona_id: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            persona_id: persona_id.into(),
            announced: Mutex::new(HashSet::new()),
        })
    }

    fn gone(&self, origin: &str, name: &str, reason: &str) -> Option<String> {
        ledger::mark_absent(&self.persona_id, ToolSourceKind::Mcp, origin, reason);
        let mut announced = self
            .announced
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !announced.insert(origin.to_string()) {
            return None;
        }
        Some(format!(
            "The {name} MCP server went away: {reason}. Its tools are gone until the teammate restarts."
        ))
    }
}

/// An MCP tool as Hotline Agent sees it: a unique name, the server's own
/// description and schema, and a call that goes back to that server.
#[derive(Clone)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    /// The server this tool came from, as the ledger names it.
    pub origin: String,
    server_name: String,
    remote_name: String,
    peer: Peer<RoleClient>,
    watch: Arc<Watch>,
    capability: Option<CapabilityLease>,
}

impl McpTool {
    pub(crate) fn new(
        prefix: &str,
        server_id: &str,
        server_name: &str,
        definition: rmcp::model::Tool,
        peer: Peer<RoleClient>,
        watch: Arc<Watch>,
    ) -> Self {
        let remote_name = definition.name.to_string();
        Self {
            name: tool_name(prefix, &remote_name),
            description: definition.description.as_deref().unwrap_or("").to_string(),
            parameters: definition.schema_as_json_value(),
            origin: server_id.to_string(),
            server_name: server_name.to_string(),
            remote_name,
            peer,
            watch,
            capability: None,
        }
    }

    pub(crate) fn with_capability_opt(mut self, capability: Option<CapabilityLease>) -> Self {
        self.capability = capability;
        self
    }

    /// The server's human name, for a sentence about where the tool came
    /// from; the id is the ledger's key, not something a person reads.
    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// Forward the call and return the result's content. Errors as `Err`.
    pub async fn call(&self, arguments: Value) -> Result<CallContent, CallError> {
        if let Some(capability) = &self.capability
            && let Err(error) = capability.check()
        {
            return Err(CallError::Tool(error));
        }
        let arguments = match arguments {
            Value::Null => None,
            Value::Object(object) => Some(object),
            other => {
                return Err(CallError::Tool(format!(
                    "MCP tool '{}' expected a JSON object, got {other}",
                    self.name
                )));
            }
        };
        let mut request = CallToolRequestParams::new(self.remote_name.clone());
        request.arguments = arguments;
        let result = match self.peer.call_tool(request).await {
            Ok(result) => result,
            Err(error) => {
                let message = format!("MCP tool '{}' request failed: {error}", self.name);
                if is_transport(&error) {
                    let notice =
                        self.watch
                            .gone(&self.origin, &self.server_name, &error.to_string());
                    return Err(CallError::Transport { message, notice });
                }
                return Err(CallError::Tool(message));
            }
        };
        if result.is_error == Some(true) {
            let text = result_text(&result);
            Err(CallError::Tool(if text.is_empty() {
                format!("MCP tool '{}' reported an error", self.name)
            } else {
                text
            }))
        } else {
            Ok(result_content(&result))
        }
    }
}

/// The origin is gone: the process exited, the socket closed, the HTTP
/// endpoint could not be reached. A JSON-RPC error and an `isError` result
/// are answers, and do not take this path.
fn is_transport(error: &ServiceError) -> bool {
    matches!(
        error,
        ServiceError::TransportClosed | ServiceError::TransportSend(_)
    )
}

fn result_content(result: &CallToolResult) -> CallContent {
    let images = result
        .content
        .iter()
        .filter_map(|block| {
            block.as_image().map(|image| CallImage {
                data: image.data.clone(),
                mime_type: image.mime_type.clone(),
            })
        })
        .collect();
    CallContent {
        text: result_text(result),
        images,
    }
}

fn result_text(result: &CallToolResult) -> String {
    let texts: Vec<String> = result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|text| text.text.clone()))
        .collect();
    if !texts.is_empty() {
        return texts.join("\n");
    }
    result
        .structured_content
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::ContentBlock;

    #[test]
    fn a_text_block_and_an_image_block_are_both_kept() {
        let result = CallToolResult::success(vec![
            ContentBlock::text("the tree"),
            ContentBlock::image("AAAA", "image/png"),
        ]);
        let content = result_content(&result);
        assert_eq!(content.text, "the tree");
        assert_eq!(
            content.images,
            [CallImage {
                data: "AAAA".into(),
                mime_type: "image/png".into(),
            }]
        );
    }

    #[test]
    fn an_error_result_still_flattens_to_text() {
        let result = CallToolResult::error(vec![
            ContentBlock::text("nope"),
            ContentBlock::image("AAAA", "image/png"),
        ]);
        assert_eq!(result_text(&result), "nope");
    }
}
