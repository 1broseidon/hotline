//! The teammate's own desk-side tools, as the voice front holds them.
//!
//! The front runs these itself, on the call: it looks things up in the
//! teammate's conversation and schedules, searches the web, and calls the MCP
//! servers the teammate's policy grants. Each goes through the handler a
//! typed turn uses ([`TeammateTools`], [`McpTool`]), bound to the teammate's
//! current capability lease, so a grant taken away on the card refuses the
//! front's next call. A result goes back to the model as the function's
//! output, clipped; nothing is posted to the DM.
//!
//! The registry names tools and runs them and knows nothing of a provider,
//! so a front whose model takes no session tools can answer its own
//! delegations from it.

use super::front::Tool;
use crate::driver::CapabilityLease;
use crate::mcp::server::TeammateTools;
use crate::mcp::{self, McpServer, McpTool};
use crate::session::{FrontReach, Room};
use rig::tool::{DynamicTool, ToolOutput};
use serde_json::{Map, Value, json};
use std::sync::{Arc, Weak};
use std::time::Duration;

pub const POST_TO_CHAT: &str = "post_to_chat";
/// The most one post to the chat may say; a long account is the working
/// session's to write.
const POST_CHARS: usize = 2_000;

/// The most a tool's output may be before the model hears it. A voice cannot
/// use more, and the model's context is spent on what it reads.
pub const OUTPUT_CHARS: usize = 4_000;
/// A tool that has not answered by now is given up on, so the front can say so.
const CALL_LIMIT: Duration = Duration::from_secs(30);
/// The servers a call connects, all of them or none, before it opens.
const CONNECT_LIMIT: Duration = Duration::from_secs(10);

pub struct FrontTools {
    room: Weak<Room>,
    call_id: String,
    persona_id: String,
    teammate: TeammateTools,
    capability: CapabilityLease,
    granted: Vec<McpTool>,
    /// Keeps the servers' connections open for as long as the call.
    _connections: Option<mcp::Connections>,
}

impl FrontTools {
    /// Connects the servers the teammate is granted and holds them for the
    /// call. A server that does not answer in time is left out, and the
    /// front is not given its tools.
    pub(crate) async fn open(
        room: Weak<Room>,
        call_id: &str,
        persona_id: &str,
        reach: FrontReach,
    ) -> Self {
        let FrontReach {
            tools,
            servers,
            vault,
            capability,
        } = reach;
        let connections = if servers.is_empty() {
            None
        } else {
            connect(persona_id, &servers, &capability, vault).await
        };
        let granted = connections
            .as_ref()
            .map(|connections| connections.tools.clone())
            .unwrap_or_default();
        Self {
            room,
            call_id: call_id.into(),
            persona_id: persona_id.into(),
            teammate: tools,
            capability,
            granted,
            _connections: connections,
        }
    }

    /// Every function the front is given besides its own two.
    pub fn definitions(&self) -> Vec<Tool> {
        let built_in = TeammateTools::front_descriptors().into_iter().map(
            |(name, description, parameters)| Tool {
                name,
                description,
                parameters,
            },
        );
        let post = Tool {
            name: POST_TO_CHAT.into(),
            description: "Put something in the written conversation with the person, when they ask for it (\"put that in the chat\"). Posts the text as from this call; it is not how you hand them work.".into(),
            parameters: json!({
                "type": "object",
                "properties": {"text": {"type": "string", "description": "What to post, as the person should read it."}},
                "required": ["text"],
            }),
        };
        let granted = self.granted.iter().map(|tool| Tool {
            name: tool.name.clone(),
            description: tool.description.clone(),
            parameters: tool.parameters.clone(),
        });
        built_in.chain([post]).chain(granted).collect()
    }

    /// These as tools a desk-side model calls, for a front whose model calls
    /// none itself.
    pub fn dynamic(self: &Arc<Self>) -> Vec<DynamicTool> {
        self.definitions()
            .into_iter()
            .map(|tool| {
                let tools = self.clone();
                let name = tool.name.clone();
                DynamicTool::new(
                    tool.name,
                    tool.description,
                    tool.parameters,
                    move |_, arguments| {
                        let tools = tools.clone();
                        let name = name.clone();
                        Box::pin(async move {
                            Ok(ToolOutput::text(
                                tools.call(&name, &arguments.to_string()).await,
                            ))
                        })
                    },
                )
            })
            .collect()
    }

    /// Whether `name` is one of these.
    pub fn has(&self, name: &str) -> bool {
        name == POST_TO_CHAT
            || self.granted.iter().any(|tool| tool.name == name)
            || TeammateTools::front_descriptors()
                .iter()
                .any(|(known, _, _)| known == name)
    }

    /// Runs one tool and returns what the model is told: the output, or the
    /// refusal in words, clipped.
    pub async fn call(&self, name: &str, arguments: &str) -> String {
        let arguments: Value = if arguments.trim().is_empty() {
            Value::Object(Map::default())
        } else {
            match serde_json::from_str(arguments) {
                Ok(arguments) => arguments,
                Err(_) => return format!("The arguments to {name} were not JSON."),
            }
        };
        if let Err(refusal) = self.capability.check() {
            return refusal;
        }
        if name == POST_TO_CHAT {
            return self.post(&arguments);
        }
        let run = async {
            match self.granted.iter().find(|tool| tool.name == name) {
                Some(tool) => tool
                    .call(arguments)
                    .await
                    .map(|content| content.text)
                    .map_err(|error| error.to_string()),
                None => self.teammate.call(name, &arguments).await,
            }
        };
        let answer = match tokio::time::timeout(CALL_LIMIT, run).await {
            Ok(Ok(output)) => output,
            Ok(Err(error)) => error,
            Err(_) => format!("{name} did not answer in time."),
        };
        crate::driver::clip(&answer, OUTPUT_CHARS)
    }
}

impl FrontTools {
    /// Posts the person's request to the teammate's DM, tagged as from the call.
    fn post(&self, arguments: &Value) -> String {
        let Some(text) = arguments["text"]
            .as_str()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        else {
            return "post_to_chat needs the text to post.".into();
        };
        let Some(room) = self.room.upgrade() else {
            return "The chat is not available.".into();
        };
        let text = crate::driver::clip(text, POST_CHARS);
        room.call_posted(
            &self.persona_id,
            &self.call_id,
            &format!("From the call: {text}"),
        );
        "Posted to the chat.".into()
    }
}

async fn connect(
    persona_id: &str,
    servers: &[McpServer],
    capability: &CapabilityLease,
    vault: Option<std::sync::Arc<crate::vault::Vault>>,
) -> Option<mcp::Connections> {
    tokio::time::timeout(
        CONNECT_LIMIT,
        mcp::connect_with_capability_and_vault(
            persona_id,
            servers,
            Some(capability.clone()),
            vault,
        ),
    )
    .await
    .ok()
}
