mod browser;
mod capture;
mod files;
mod input;
mod shell;
mod state;
mod wait;
mod windows;

use std::sync::Arc;

use rmcp::model::{ContentBlock, JsonObject, Tool};
use serde_json::{Value, json};

use crate::App;

pub const NAMES: [&str; 8] = [
    "capture", "input", "browser", "shell", "files", "windows", "wait", "state",
];

pub type ToolResult = Result<Vec<ContentBlock>, String>;

pub fn descriptors(home: &str) -> Vec<Tool> {
    vec![
        Tool::new(
            "capture",
            "See the screen — the way in for native apps (web content reads better through browser text). Default returns a screenshot plus the AT-SPI accessibility tree as structured text: windows, elements, roles, coordinates, values, and states. mode=png saves a raw image and returns its path. There is no OCR.",
            schema(json!({
                "type":"object","properties":{
                    "mode":{"type":"string","enum":["tree","png"],"description":"tree (default): screenshot plus accessibility tree; png: save a raw PNG"},
                    "path":{"type":"string","description":"png only: optional output path"}
                },"additionalProperties":false
            })),
        ),
        Tool::new(
            "input",
            "Drive the mouse, keyboard, and clipboard on the desktop — for native apps, with coordinates from capture. For the web browser, prefer browser text and refs. type is per-character; paste sets the clipboard and presses Ctrl+V. batch runs a short scripted sequence under one machine lock.",
            schema(json!({
                "type":"object","properties":{
                    "action":{"type":"string","enum":["click","double_click","right_click","move","drag","scroll","type","key","paste","clipboard_read","clipboard_write","batch"]},
                    "x":{"type":"integer"},"y":{"type":"integer"},"x2":{"type":"integer"},"y2":{"type":"integer"},
                    "clicks":{"type":"integer"},"text":{"type":"string"},"combo":{"type":"string"},
                    "steps":{"type":"array","items":{"type":"object"}},"stop_on_error":{"type":"boolean","default":true},
                    "settle_ms":{"type":"integer","minimum":0,"default":40},"capture_after":{"type":"string","enum":["final","each","none"],"default":"final"},
                    "capture_on_error":{"type":"boolean","default":true}
                },"required":["action"],"additionalProperties":false
            })),
        ),
        Tool::new(
            "browser",
            "The managed Chromium, semantically. text returns an accessibility-style page snapshot with element refs; click_ref, fill, select, check, and hover act on those refs. Refs are stable within one snapshot. Also provides navigation, history, tabs, uploads, dialogs, and downloads.",
            schema(json!({
                "type":"object","properties":{
                    "action":{"type":"string","enum":["navigate","text","links","eval","click_ref","fill","select","check","hover","tabs","tab_new","tab_select","tab_close","upload","dialog_accept","dialog_dismiss","downloads","back","forward","reload"]},
                    "url":{"type":"string"},"js":{"type":"string"},"ref":{"type":"string"},"button":{"type":"string"},
                    "text":{"type":"string"},"value":{"type":"string"},"uncheck":{"type":"boolean"},"index":{"type":"integer","minimum":0},"path":{"type":"string"}
                },"required":["action"],"additionalProperties":false
            })),
        ),
        Tool::new(
            "shell",
            "Run commands. exec is synchronous — stdout, stderr, exit code, duration — and is the escape hatch for everything without a tool. launch starts a GUI app on the desktop and returns its PID.",
            schema(json!({
                "type":"object","properties":{
                    "action":{"type":"string","enum":["exec","launch"],"default":"exec"},"command":{"type":"string"},
                    "args":{"type":"array","items":{"type":"string"}},"cwd":{"type":"string","default":home},
                    "timeout":{"type":"integer","minimum":1,"maximum":60,"default":30},"max_output":{"type":"integer","minimum":1,"maximum":1048576,"default":65536}
                },"required":["command"],"additionalProperties":false
            })),
        ),
        Tool::new(
            "files",
            format!(
                "Move files across the machine boundary over MCP. get returns text, or base64 when bytes are not UTF-8; put accepts UTF-8 or encoding=base64; list shows a directory. Paths stay under {home}. Cap 50MB."
            ),
            schema(json!({
                "type":"object","properties":{
                    "action":{"type":"string","enum":["get","put","list"]},"path":{"type":"string"},"content":{"type":"string"},"encoding":{"type":"string","enum":["utf8","text","base64"]}
                },"required":["action","path"],"additionalProperties":false
            })),
        ),
        Tool::new(
            "windows",
            "Manage desktop windows: list them with IDs, classes, bounds, and focus; focus, close, maximize or restore; or auto-tile them with Chromium left and the rest stacked right.",
            schema(json!({
                "type":"object","properties":{
                    "action":{"type":"string","enum":["list","focus","close","maximize","tile"]},"window_id":{"type":"string"},"unmaximize":{"type":"boolean"}
                },"required":["action"],"additionalProperties":false
            })),
        ),
        Tool::new(
            "wait",
            "Poll every 500ms until text appears in the desktop accessibility tree or the managed browser page. Returns when found or after timeout; use it after input or navigation to verify the expected state.",
            schema(json!({
                "type":"object","properties":{"text":{"type":"string"},"timeout":{"type":"integer","minimum":1,"maximum":60,"default":10}},
                "required":["text"],"additionalProperties":false
            })),
        ),
        Tool::new(
            "state",
            "Durable machine state. control leases the desktop to this holder until release or expiry, and other holders' mutating tools are refused. Only the holder can release it. login_* saves and restores browser cookies and storage by name. snapshot_* archives and restores the home directory.",
            schema(json!({
                "type":"object","properties":{
                    "action":{"type":"string","enum":["control","release","login_save","login_load","login_list","login_delete","snapshot_save","snapshot_load","snapshot_list","snapshot_delete"]},
                    "name":{"type":"string"},"duration":{"type":"integer","minimum":1,"maximum":600,"default":300}
                },"required":["action"],"additionalProperties":false
            })),
        ),
    ]
}

fn schema(value: Value) -> Arc<JsonObject> {
    Arc::new(value.as_object().expect("schemas are objects").clone())
}

pub async fn call(app: &App, name: &str, arguments: Value, holder: &str) -> ToolResult {
    match name {
        "capture" => capture::call(app, arguments).await,
        "input" => input::call(app, arguments, holder).await,
        "browser" => browser::call(app, arguments, holder).await,
        "shell" => shell::call(app, arguments, holder).await,
        "files" => files::call(app, arguments).await,
        "windows" => windows::call(app, arguments, holder).await,
        "wait" => wait::call(app, arguments).await,
        "state" => state::call(app, arguments, holder).await,
        _ => Err(format!("unknown tool {name:?}")),
    }
}

pub fn text(value: impl Into<String>) -> Vec<ContentBlock> {
    vec![ContentBlock::text(value)]
}

pub fn json_text(value: impl serde::Serialize) -> ToolResult {
    serde_json::to_string(&value)
        .map(text)
        .map_err(|error| error.to_string())
}

pub fn action_error(tool: &str, action: &str, actions: &[&str]) -> String {
    format!("{tool}: unknown action {action:?} (one of: {actions:?})")
}

pub async fn command(display: &str, program: &str, arguments: &[String]) -> Result<String, String> {
    let output = tokio::process::Command::new(program)
        .args(arguments)
        .env("DISPLAY", display)
        .output()
        .await
        .map_err(|error| format!("{program}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{program}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
