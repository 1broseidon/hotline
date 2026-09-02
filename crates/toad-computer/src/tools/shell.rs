use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::App;

use super::{ToolResult, action_error, json_text, text};

#[derive(Deserialize)]
struct Input {
    #[serde(default)]
    action: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    cwd: Option<String>,
    timeout: Option<u64>,
    max_output: Option<usize>,
}

#[derive(Serialize)]
struct ExecResult {
    stdout: String,
    stderr: String,
    exit_code: i32,
    duration_ms: u128,
    truncated: bool,
}

pub async fn call(app: &App, arguments: Value, holder: &str) -> ToolResult {
    let input: Input = serde_json::from_value(arguments).map_err(|error| error.to_string())?;
    if input.command.is_empty() {
        return Err("command is required".into());
    }
    let _guard = app.access.mutate(holder).await?;
    match input.action.as_str() {
        "" | "exec" => exec(app, input).await,
        "launch" => launch(app, input).await,
        action => Err(action_error("shell", action, &["exec", "launch"])),
    }
}

async fn exec(app: &App, input: Input) -> ToolResult {
    let timeout = input.timeout.unwrap_or(30).min(60);
    let max_output = input.max_output.unwrap_or(65_536).min(1_048_576);
    let started = Instant::now();
    let mut command = tokio::process::Command::new(&input.command);
    command
        .args(&input.args)
        .current_dir(
            input
                .cwd
                .as_deref()
                .unwrap_or_else(|| app.config.home.to_str().unwrap_or("/home/agent")),
        )
        .env("DISPLAY", &app.config.display)
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command
        .spawn()
        .map_err(|error| format!("{}: {error}", input.command))?;
    let output =
        match tokio::time::timeout(Duration::from_secs(timeout), child.wait_with_output()).await {
            Ok(result) => result.map_err(|error| error.to_string())?,
            Err(_) => {
                return json_text(ExecResult {
                    stdout: String::new(),
                    stderr: format!("exec timed out after {timeout}s"),
                    exit_code: 255,
                    duration_ms: started.elapsed().as_millis(),
                    truncated: false,
                });
            }
        };
    let (stdout, stdout_truncated) = truncate(output.stdout, max_output);
    let (stderr, stderr_truncated) = truncate(output.stderr, max_output);
    json_text(ExecResult {
        stdout,
        stderr,
        exit_code: output.status.code().unwrap_or(-1),
        duration_ms: started.elapsed().as_millis(),
        truncated: stdout_truncated || stderr_truncated,
    })
}

async fn launch(app: &App, input: Input) -> ToolResult {
    let child = std::process::Command::new(&input.command)
        .args(&input.args)
        .current_dir(
            input
                .cwd
                .as_deref()
                .unwrap_or_else(|| app.config.home.to_str().unwrap_or("/home/agent")),
        )
        .env("DISPLAY", &app.config.display)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("{}: {error}", input.command))?;
    Ok(text(format!(
        "launched {} (pid {})",
        input.command,
        child.id()
    )))
}

fn truncate(bytes: Vec<u8>, max: usize) -> (String, bool) {
    if bytes.len() <= max {
        (String::from_utf8_lossy(&bytes).into_owned(), false)
    } else {
        (String::from_utf8_lossy(&bytes[..max]).into_owned(), true)
    }
}
