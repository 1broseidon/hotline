//! The tools of the servers a teammate was granted, by name.
//!
//! Every tool definition rides in every request, and a large server — dozens
//! of tools, each with its own schema — can cost more than the rest of the
//! request put together, on every round. So a granted server's tools are not
//! definitions here. The preamble lists them, one line each like the skills
//! index; `tool_schema` hands over one tool's parameters when the agent is
//! about to use it; and `call_tool` calls it. The two stand in for every
//! granted tool, so the request's tool list is the same whatever is granted,
//! and a server's tools arriving or leaving never touches it.
//!
//! Hotline's own tools, the file tools and the computer's stay definitions:
//! they are the teammate's hands, used on nearly every turn.

use super::*;

/// Reads one granted tool's description and parameters.
pub(super) const SCHEMA: &str = "tool_schema";
/// Calls one granted tool.
pub(super) const CALL: &str = "call_tool";

/// How much of a tool's description its index line keeps.
const LINE_CHARS: usize = 120;

/// Whether a connected tool comes from a server the person granted, rather
/// than the teammate's own computer.
pub(super) fn is_granted(tool: &mcp::McpTool) -> bool {
    tool.origin != crate::computer::SERVER_ID
}

/// The preamble's list of the granted tools, and how to reach them. Empty
/// when nothing is granted.
pub(super) fn index(tools: &[mcp::McpTool]) -> String {
    if tools.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = tools
        .iter()
        .map(|tool| match summary(&tool.description) {
            summary if summary.is_empty() => format!("- {}", tool.name),
            summary => format!("- {}: {summary}", tool.name),
        })
        .collect();
    format!(
        "The servers you were granted have these tools. They are not in your tool list: before you first use one, read its parameters with `{SCHEMA}`, then call it with `{CALL}`, its arguments written as a JSON object.\n{}",
        lines.join("\n")
    )
}

/// A description's first sentence, on one line and short.
fn summary(description: &str) -> String {
    let line = description
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    let sentence = match line.find(". ") {
        Some(end) => &line[..=end],
        None => line,
    };
    clip(sentence, LINE_CHARS)
}

/// `tool_schema` and `call_tool`, over these tools.
pub(super) fn tools(tools: Vec<mcp::McpTool>) -> [DynamicTool; 2] {
    let tools: Arc<HashMap<String, mcp::McpTool>> = Arc::new(
        tools
            .into_iter()
            .map(|tool| (tool.name.clone(), tool))
            .collect(),
    );
    let known = tools.clone();
    let schema = DynamicTool::new(
        SCHEMA,
        "Read a granted tool's description and parameters, before you first call it with call_tool.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "tool": {"type": "string", "description": "The tool's name, as your instructions list it."}
            },
            "required": ["tool"]
        }),
        move |_context, arguments| {
            let known = known.clone();
            Box::pin(async move {
                let tool = find(&known, &arguments)?;
                Ok(ToolOutput::text(parameters_of(tool)))
            })
        },
    );
    let call = DynamicTool::new(
        CALL,
        "Call a granted tool. Read its parameters with tool_schema first.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "tool": {"type": "string", "description": "The tool's name, as your instructions list it."},
                "arguments": {"type": "string", "description": "The tool's arguments: a JSON object, written out as a string. \"{}\" when it takes none."}
            },
            "required": ["tool", "arguments"]
        }),
        move |_context, arguments| {
            let tools = tools.clone();
            Box::pin(async move {
                let tool = find(&tools, &arguments)?.clone();
                let inner = inner_arguments(&arguments).map_err(ToolExecutionError::other)?;
                match tool.call(inner).await {
                    Ok(content) => rig_output(content),
                    Err(mcp::CallError::Transport { message, .. }) => {
                        Err(ToolExecutionError::other(message))
                    }
                    // Called without reading the parameters, or with the
                    // wrong ones: the next try has them in hand.
                    Err(mcp::CallError::Tool(message)) => Err(ToolExecutionError::other(format!(
                        "{message}\n\n{}",
                        parameters_of(&tool)
                    ))),
                }
            })
        },
    );
    [schema, call]
}

fn find<'a>(
    tools: &'a HashMap<String, mcp::McpTool>,
    arguments: &Value,
) -> Result<&'a mcp::McpTool, ToolExecutionError> {
    let name = arguments
        .get("tool")
        .and_then(Value::as_str)
        .unwrap_or_default();
    tools.get(name).ok_or_else(|| {
        ToolExecutionError::other(format!(
            "No granted tool is named `{name}`. Your instructions list the ones there are."
        ))
    })
}

/// What `tool_schema` answers: enough to call the tool right the first time.
fn parameters_of(tool: &mcp::McpTool) -> String {
    format!(
        "{}: {}\nParameters (JSON Schema): {}",
        tool.name,
        tool.description.trim(),
        tool.parameters
    )
}

/// The granted tool's own arguments out of a `call_tool` call: a JSON object
/// written as a string, which any route can carry, or the object itself from
/// a model that sent one.
pub(super) fn inner_arguments(arguments: &Value) -> Result<Value, String> {
    match arguments.get("arguments") {
        None | Some(Value::Null) => Ok(Value::Object(Default::default())),
        Some(Value::String(text)) if text.trim().is_empty() => {
            Ok(Value::Object(Default::default()))
        }
        Some(Value::String(text)) => match serde_json::from_str::<Value>(text) {
            Ok(object @ Value::Object(_)) => Ok(object),
            _ => Err(format!(
                "`arguments` is the tool's arguments as a JSON object, written out as a string; this is not one: {}",
                clip(text, TITLE_CHARS)
            )),
        },
        Some(object @ Value::Object(_)) => Ok(object.clone()),
        Some(other) => Err(format!(
            "`arguments` is the tool's arguments as a JSON object, written out as a string, not {other}."
        )),
    }
}

/// A call as the transcript names it: a `call_tool` is the granted tool it
/// called, with that tool's arguments, so the line says what was touched.
pub(super) fn shown(name: &str, arguments: &Value) -> (String, Value) {
    if name == CALL
        && let Some(inner) = arguments.get("tool").and_then(Value::as_str)
    {
        let inner_arguments = inner_arguments(arguments).unwrap_or(Value::Null);
        return (inner.to_string(), inner_arguments);
    }
    (name.to_string(), arguments.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_granted_tool_is_one_line_of_its_first_sentence() {
        assert_eq!(
            summary("Create a new issue. Use the team's key.\nMore detail."),
            "Create a new issue."
        );
        assert_eq!(summary("\n  List projects\n"), "List projects");
        assert_eq!(summary(""), "");
        assert!(summary(&"word ".repeat(100)).chars().count() <= LINE_CHARS + 1);
    }

    #[test]
    fn a_call_carries_its_arguments_as_a_string_or_an_object() {
        let call = |arguments: Value| json!({"tool": "linear__get_issue", "arguments": arguments});
        assert_eq!(
            inner_arguments(&call(json!("{\"id\":\"BRO-7\"}"))).unwrap(),
            json!({"id": "BRO-7"})
        );
        assert_eq!(
            inner_arguments(&call(json!({"id": "BRO-7"}))).unwrap(),
            json!({"id": "BRO-7"})
        );
        assert_eq!(inner_arguments(&call(json!(""))).unwrap(), json!({}));
        assert_eq!(
            inner_arguments(&json!({"tool": "linear__list_teams"})).unwrap(),
            json!({})
        );
        assert!(inner_arguments(&call(json!("[1, 2]"))).is_err());
        assert!(inner_arguments(&call(json!("not json"))).is_err());
        assert!(inner_arguments(&call(json!(3))).is_err());
    }

    #[test]
    fn the_transcript_names_the_granted_tool_a_call_reached() {
        assert_eq!(
            shown(
                CALL,
                &json!({"tool": "linear__get_issue", "arguments": "{\"query\":\"BRO-7\"}"})
            ),
            ("linear__get_issue".to_string(), json!({"query": "BRO-7"}))
        );
        assert_eq!(
            shown("read", &json!({"path": "x"})),
            ("read".to_string(), json!({"path": "x"}))
        );
    }
}
