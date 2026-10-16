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
use crate::session::FrontReach;
use serde_json::Value;
use std::time::Duration;

/// The most a tool's output may be before the model hears it. A voice cannot
/// use more, and the model's context is spent on what it reads.
pub const OUTPUT_CHARS: usize = 4_000;
/// A tool that has not answered by now is given up on, so the front can say so.
const CALL_LIMIT: Duration = Duration::from_secs(30);
/// The servers a call connects, all of them or none, before it opens.
const CONNECT_LIMIT: Duration = Duration::from_secs(10);

pub struct FrontTools {
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
    pub(crate) async fn open(persona_id: &str, reach: FrontReach) -> Self {
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
        let granted = self.granted.iter().map(|tool| Tool {
            name: tool.name.clone(),
            description: tool.description.clone(),
            parameters: tool.parameters.clone(),
        });
        built_in.chain(granted).collect()
    }

    /// Whether `name` is one of these.
    pub fn has(&self, name: &str) -> bool {
        self.granted.iter().any(|tool| tool.name == name)
            || TeammateTools::front_descriptors()
                .iter()
                .any(|(known, _, _)| known == name)
    }

    /// Runs one tool and returns what the model is told: the output, or the
    /// refusal in words, clipped.
    pub async fn call(&self, name: &str, arguments: &str) -> String {
        let arguments: Value = if arguments.trim().is_empty() {
            Value::Object(Default::default())
        } else {
            match serde_json::from_str(arguments) {
                Ok(arguments) => arguments,
                Err(_) => return format!("The arguments to {name} were not JSON."),
            }
        };
        if let Err(refusal) = self.capability.check() {
            return refusal;
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
