use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use crate::{App, a11y, x11};

use super::{ToolResult, action_error, command, json_text, text};

#[derive(Deserialize)]
struct Input {
    action: String,
    #[serde(default)]
    x: i32,
    #[serde(default)]
    y: i32,
    #[serde(default)]
    x2: i32,
    #[serde(default)]
    y2: i32,
    #[serde(default)]
    clicks: i32,
    #[serde(default)]
    text: String,
    #[serde(default)]
    combo: String,
    #[serde(default)]
    steps: Vec<Value>,
    stop_on_error: Option<bool>,
    settle_ms: Option<u64>,
    #[serde(default)]
    capture_after: String,
    capture_on_error: Option<bool>,
}

pub async fn call(app: &App, arguments: Value, holder: &str) -> ToolResult {
    let input: Input = serde_json::from_value(arguments).map_err(|error| error.to_string())?;
    if input.action == "clipboard_read" {
        let value = command(
            &app.config.display,
            "xclip",
            &["-selection".into(), "clipboard".into(), "-o".into()],
        )
        .await?;
        return Ok(text(value));
    }
    if input.action == "batch" {
        return batch(app, holder, input).await;
    }
    let _guard = app.access.mutate(holder).await?;
    one(
        app,
        &input.action,
        &json!({
            "x": input.x, "y": input.y, "x2": input.x2, "y2": input.y2,
            "clicks": input.clicks, "text": input.text, "combo": input.combo
        }),
    )
    .await
    .map(text)
}

async fn one(app: &App, action: &str, step: &Value) -> Result<String, String> {
    let integer = |name: &str| {
        step.get(name)
            .and_then(Value::as_i64)
            .unwrap_or_default()
            .to_string()
    };
    let string = |name: &str| {
        step.get(name)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    match action {
        "click" => xdo(
            app,
            &[
                "mousemove",
                "--sync",
                &integer("x"),
                &integer("y"),
                "click",
                "1",
            ],
        )
        .await
        .map(|_| "clicked".into()),
        "double_click" | "dclick" => xdo(
            app,
            &[
                "mousemove",
                "--sync",
                &integer("x"),
                &integer("y"),
                "click",
                "--repeat",
                "2",
                "--delay",
                "50",
                "1",
            ],
        )
        .await
        .map(|_| "double-clicked".into()),
        "right_click" | "rclick" => xdo(
            app,
            &[
                "mousemove",
                "--sync",
                &integer("x"),
                &integer("y"),
                "click",
                "3",
            ],
        )
        .await
        .map(|_| "right-clicked".into()),
        "move" => xdo(app, &["mousemove", "--sync", &integer("x"), &integer("y")])
            .await
            .map(|_| "moved".into()),
        "drag" => {
            xdo(
                app,
                &[
                    "mousemove",
                    "--sync",
                    &integer("x"),
                    &integer("y"),
                    "mousedown",
                    "1",
                    "mousemove",
                    "--sync",
                    &integer("x2"),
                    &integer("y2"),
                    "mouseup",
                    "1",
                ],
            )
            .await?;
            Ok("dragged".into())
        }
        "scroll" => {
            let clicks = step
                .get("clicks")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            let button = if clicks > 0 { "4" } else { "5" };
            xdo(
                app,
                &[
                    "mousemove",
                    "--sync",
                    &integer("x"),
                    &integer("y"),
                    "click",
                    "--repeat",
                    &clicks.unsigned_abs().to_string(),
                    button,
                ],
            )
            .await?;
            Ok("scrolled".into())
        }
        "type" => xdo(app, &["type", "--delay", "0", "--", &string("text")])
            .await
            .map(|_| "typed".into()),
        "key" => {
            let combo = string("combo");
            if combo.is_empty() {
                return Err("combo is required".into());
            }
            xdo(app, &["key", "--clearmodifiers", &combo])
                .await
                .map(|_| format!("sent {combo}"))
        }
        "paste" => {
            clipboard_write(app, &string("text")).await?;
            xdo(app, &["key", "--clearmodifiers", "ctrl+v"]).await?;
            Ok("pasted".into())
        }
        "clipboard_write" => clipboard_write(app, &string("text"))
            .await
            .map(|_| "clipboard set".into()),
        "focus" => {
            let id = string("window_id");
            command(
                &app.config.display,
                "wmctrl",
                &["-i".into(), "-a".into(), id],
            )
            .await?;
            Ok("focused".into())
        }
        "navigate" => app.browser.navigate(&string("url")).await,
        "wait" => {
            let wanted = string("text");
            let timeout = step
                .get("timeout")
                .and_then(Value::as_u64)
                .unwrap_or(10)
                .min(60);
            let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);
            loop {
                let windows = x11::windows(&app.config.display).unwrap_or_default();
                let tree = a11y::tree(&windows).await;
                let page = app.browser.page_text_if_running().await.unwrap_or_default();
                if tree.contains(&wanted) || page.contains(&wanted) {
                    break Ok("found".into());
                }
                if tokio::time::Instant::now() >= deadline {
                    break Err(format!("text {wanted:?} did not appear within {timeout}s"));
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
        other => Err(action_error(
            "input batch",
            other,
            &[
                "click", "dclick", "rclick", "paste", "key", "type", "scroll", "drag", "move",
                "focus", "navigate", "wait",
            ],
        )),
    }
}

async fn batch(app: &App, holder: &str, input: Input) -> ToolResult {
    if input.steps.is_empty() {
        return Err("steps is required".into());
    }
    if input.steps.len() > 10 {
        return Err(format!("too many steps: got {}, max 10", input.steps.len()));
    }
    let _permit = app.access.run(holder).await?;
    let stop = input.stop_on_error.unwrap_or(true);
    let settle = Duration::from_millis(input.settle_ms.unwrap_or(40));
    let capture_on_error = input.capture_on_error.unwrap_or(true);
    let capture_after = if input.capture_after.is_empty() {
        "final"
    } else {
        &input.capture_after
    };
    if !["final", "each", "none"].contains(&capture_after) {
        return Err("capture_after must be one of final, each, none".into());
    }
    let started = tokio::time::Instant::now();
    let mut results = Vec::new();
    let mut final_capture = None;
    for (index, step) in input.steps.iter().enumerate() {
        let step_started = tokio::time::Instant::now();
        let action = step
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| "step action is required".to_owned())?;
        let outcome = one(app, action, step).await;
        let mut result = json!({"index":index,"ok":outcome.is_ok(),"duration_ms":step_started.elapsed().as_millis()});
        if let Err(error) = &outcome {
            result["error"] = json!(error);
            if capture_on_error {
                let windows = x11::windows(&app.config.display).unwrap_or_default();
                final_capture = Some(a11y::tree(&windows).await);
            }
        }
        if capture_after == "each" {
            let windows = x11::windows(&app.config.display).unwrap_or_default();
            result["capture"] = json!(a11y::tree(&windows).await);
        }
        results.push(result);
        if outcome.is_err() && stop {
            break;
        }
        if index + 1 < input.steps.len() {
            tokio::time::sleep(settle).await;
        }
    }
    if capture_after == "final" {
        let windows = x11::windows(&app.config.display).unwrap_or_default();
        final_capture = Some(a11y::tree(&windows).await);
    }
    json_text(
        json!({"steps":results,"capture":final_capture,"total_duration_ms":started.elapsed().as_millis()}),
    )
}

async fn xdo(app: &App, arguments: &[&str]) -> Result<(), String> {
    let arguments: Vec<String> = arguments.iter().map(ToString::to_string).collect();
    command(&app.config.display, "xdotool", &arguments)
        .await
        .map(|_| ())
}

async fn clipboard_write(app: &App, content: &str) -> Result<(), String> {
    let mut child = tokio::process::Command::new("xclip")
        .args(["-selection", "clipboard"])
        .env("DISPLAY", &app.config.display)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("xclip: {error}"))?;
    child
        .stdin
        .take()
        .expect("piped above")
        .write_all(content.as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    let status = child.wait().await.map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("xclip failed".into())
    }
}
