//! What a tool hands back on its way to the model and the tape: the quiet run's
//! one way to reach the person, the MCP tools as Rig tools, their results split
//! into words and pictures, the ledger of what this session can call, and the
//! one line that names a call.

use super::TITLE_CHARS;
use crate::contract::{AgentKind, Persona, ToolSourceKind};
use crate::driver::{Escalate, ToolImage, clip, with_image_placeholders};
use crate::mcp;
use crate::session::ledger::ToolLedger;
use crate::tools;
use rig::message::{DocumentSourceKind, ImageMediaType, MimeType, ToolResultContent};
use rig::tool::{DynamicTool, ToolExecutionError, ToolOutput};
use serde_json::Value;
use std::sync::Arc;

/// What a quiet scheduled run is told it can do: its replies are kept out of
/// the conversation, and this is the one way to put something in front of
/// the person.
const TELL_PERSON: &str = "This is a quiet scheduled run: nothing you reply \
is shown to the person or notifies them. If you find something they need to \
see or act on, call this once with what you found. When this run ends you \
will be asked, in the open, to tell them, and that reply notifies them. If \
there is nothing worth their attention, do not call it.";

/// The one-turn tool a quiet scheduled run escalates through (BRO-96).
pub(super) fn tell_person(escalation: Arc<dyn Escalate>) -> DynamicTool {
    DynamicTool::new(
        "tell_person",
        TELL_PERSON,
        serde_json::json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "What you found, with what the person needs to act on it."
                }
            },
            "required": ["message"]
        }),
        move |_context, arguments| {
            let escalation = escalation.clone();
            Box::pin(async move {
                let message = arguments
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .unwrap_or_default();
                if message.is_empty() {
                    return Err(ToolExecutionError::other("message is empty"));
                }
                escalation
                    .escalate(message)
                    .map(|()| {
                        ToolOutput::text(
                            "Handed over. When this run ends you will be asked to tell the person.",
                        )
                    })
                    .map_err(ToolExecutionError::other)
            })
        },
    )
}

/// The Rig adapter for one granted MCP tool. A transport death is already
/// on the ledger; the model hears it as the call's error.
pub(super) fn mcp_dynamic(tool: mcp::McpTool) -> DynamicTool {
    DynamicTool::new(
        tool.name.clone(),
        tool.description.clone(),
        tool.parameters.clone(),
        move |_context, arguments| {
            let tool = tool.clone();
            Box::pin(async move {
                match tool.call(arguments).await {
                    Ok(content) => rig_output(content),
                    // A server that went away is on the ledger and in the
                    // tool's own result; the conversation does not narrate it.
                    Err(mcp::CallError::Transport { message, .. }) => {
                        Err(ToolExecutionError::other(message))
                    }
                    Err(mcp::CallError::Tool(message)) => Err(ToolExecutionError::other(message)),
                }
            })
        },
    )
}

/// The model-visible form of an MCP result: text as before, and every image
/// as an image block so a screenshot is not flattened away.
pub(super) fn rig_output(content: mcp::CallContent) -> Result<ToolOutput, ToolExecutionError> {
    if content.images.is_empty() {
        return Ok(ToolOutput::text(content.text));
    }
    let mut blocks = Vec::new();
    if !content.text.is_empty() {
        blocks.push(ToolResultContent::text(content.text));
    }
    for image in content.images {
        blocks.push(ToolResultContent::image_base64(
            image.data,
            ImageMediaType::from_mime_type(&image.mime_type),
            None,
        ));
    }
    ToolOutput::content(blocks)
}

/// The transcript's view of a Rig tool result: text joined, images carried
/// alongside so the session can write a frame.
pub(super) fn result_of(content: &[ToolResultContent]) -> (String, Vec<ToolImage>) {
    let mut texts = Vec::new();
    let mut images = Vec::new();
    for item in content {
        match item {
            ToolResultContent::Text(text) => texts.push(text.text.clone()),
            ToolResultContent::Json { .. } => texts.push("[structured result]".to_string()),
            ToolResultContent::Image(image) => {
                let mime = image
                    .media_type
                    .as_ref()
                    .map_or("image", rig::message::MimeType::to_mime_type)
                    .to_string();
                match &image.data {
                    DocumentSourceKind::Base64(data) => images.push(ToolImage {
                        data: data.clone(),
                        mime_type: mime,
                    }),
                    _ => texts.push(format!("[image {mime}]")),
                }
            }
        }
    }
    let output = with_image_placeholders(&texts.join("\n"), &images);
    (output, images)
}

/// The ledger reads what this session was actually built with, not what
/// the configuration promised. Built-ins are verified because Hotline handed
/// them to the agent; MCP tools are verified when the server listed them,
/// and absent — with the error as the reason — when it did not.
pub(crate) fn publish_ledger(
    persona: &Persona,
    missing: &[String],
    connected: &mcp::Connections,
    subagents: bool,
    hotline_tools: &[&str],
) {
    let mut ledger = ToolLedger::new(
        persona.id.clone(),
        AgentKind::Hotline,
        persona.backend_id.clone(),
    );
    let reach = persona.reach.unwrap_or_default();
    match tools::shell_available(reach) {
        Ok(()) => {
            ledger.all(
                crate::contract::ToolState::Verified,
                ToolSourceKind::Builtin,
                "hotline",
                tools::BUILTIN,
                "Hotline handed them to the agent",
            );
        }
        Err(reason) => {
            let without_shell: Vec<&str> = tools::BUILTIN
                .iter()
                .copied()
                .filter(|name| *name != "shell")
                .collect();
            ledger.all(
                crate::contract::ToolState::Verified,
                ToolSourceKind::Builtin,
                "hotline",
                &without_shell,
                "Hotline handed them to the agent",
            );
            ledger.absent(ToolSourceKind::Builtin, "hotline", "shell", reason);
        }
    }
    if tools::shell_available(reach).is_ok() || subagents {
        ledger.all(
            crate::contract::ToolState::Verified,
            ToolSourceKind::Builtin,
            "hotline",
            crate::session::jobs::CONTROL_TOOLS,
            "Hotline supervises jobs independently of model requests",
        );
    }
    if subagents {
        ledger.all(
            crate::contract::ToolState::Verified,
            ToolSourceKind::Builtin,
            "hotline",
            &[crate::session::jobs::SUBAGENT],
            "Hotline runs subagents as this teammate's jobs",
        );
    }
    // Hotline's own tools are built here, not connected to: the agent and the
    // server are the same process, so there is nothing to observe and nothing
    // that can have gone wrong between them.
    ledger.all(
        crate::contract::ToolState::Verified,
        ToolSourceKind::Builtin,
        mcp::server::SERVER_NAME,
        hotline_tools,
        "Hotline's own tools, called in this process",
    );
    for tool in &connected.tools {
        ledger.verified(
            ToolSourceKind::Mcp,
            &tool.origin,
            &tool.name,
            format!("attached from the {} MCP server", tool.server_name()),
        );
    }
    for failed in &connected.failed {
        ledger.absent(
            ToolSourceKind::Mcp,
            &failed.id,
            &failed.name,
            &failed.reason,
        );
    }
    for id in missing {
        ledger.absent(ToolSourceKind::Mcp, id, id, mcp::missing_reason(id));
    }
    ledger.publish();
}

/// A tool call as a line in the transcript: the name and the one argument
/// that says what it touched.
pub(super) fn describe_tool(name: &str, arguments: &Value) -> String {
    let text = |key: &&str| arguments.get(*key).and_then(Value::as_str);
    let subject = if name == crate::session::jobs::SUBAGENT {
        // A subagent is named by the label it was given, else its task's
        // first line: the task itself is a brief, not a title.
        ["title", "task"]
            .iter()
            .filter_map(text)
            .map(|value| value.lines().next().unwrap_or(value).trim())
            .find(|value| !value.is_empty())
    } else {
        ["path", "command", "pattern", "query", "expression"]
            .iter()
            .find_map(text)
    }
    .map(|value| clip(value, TITLE_CHARS));
    match subject {
        Some(subject) => format!("{name} {subject}"),
        None => name.to_string(),
    }
}
