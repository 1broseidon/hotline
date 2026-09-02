use super::{ToolError, Workspace};
use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;
use serde_json::json;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const OUTPUT_LIMIT: usize = 16 * 1024;

#[derive(Deserialize)]
pub struct RunCommandArgs {
    command: String,
}

/// A shell command, run in the teammate's working directory.
pub struct RunCommand {
    workspace: Workspace,
}

impl RunCommand {
    pub fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }
}

impl Tool for RunCommand {
    const NAME: &'static str = "run_command";
    type Error = ToolError;
    type Args = RunCommandArgs;
    type Output = String;

    fn description(&self) -> String {
        "Run a shell command in the working directory and return its output. Commands are killed after two minutes."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command line to run, as it would be typed at a shell prompt."
                }
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    fn map_error(&self, error: Self::Error) -> ToolExecutionError {
        error.into_execution_error()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let command = args.command.trim().to_string();
        if command.is_empty() {
            return Err(ToolError::new("A command is required."));
        }
        let mut process = if cfg!(target_os = "windows") {
            let mut process = Command::new("cmd");
            process.args(["/C", &command]);
            process
        } else {
            let mut process = Command::new("sh");
            process.args(["-c", &command]);
            process
        };
        let output = tokio::time::timeout(
            COMMAND_TIMEOUT,
            process
                .current_dir(self.workspace.display_root())
                .stdin(Stdio::null())
                .output(),
        )
        .await
        .map_err(|_| ToolError::other("The command did not finish within two minutes."))?
        .map_err(|error| ToolError::other(format!("The command could not start: {error}")))?;

        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        let errors = String::from_utf8_lossy(&output.stderr);
        if !errors.trim().is_empty() {
            text.push_str("\n[stderr]\n");
            text.push_str(&errors);
        }
        if text.len() > OUTPUT_LIMIT {
            let cut = text
                .char_indices()
                .map(|(index, _)| index)
                .find(|index| *index >= OUTPUT_LIMIT)
                .unwrap_or(text.len());
            text.truncate(cut);
            text.push_str("\n[output truncated]");
        }
        let status = output.status.code().unwrap_or(-1);
        if status != 0 {
            text.push_str(&format!("\n[exit status {status}]"));
        }
        Ok(text)
    }
}
