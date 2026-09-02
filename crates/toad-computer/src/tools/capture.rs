use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use rmcp::model::ContentBlock;
use serde::Deserialize;
use serde_json::Value;

use crate::{App, a11y, x11};

use super::{ToolResult, action_error, text};

#[derive(Default, Deserialize)]
struct Input {
    #[serde(default)]
    mode: String,
    path: Option<String>,
}

pub async fn call(app: &App, arguments: Value) -> ToolResult {
    let input: Input = serde_json::from_value(arguments).map_err(|error| error.to_string())?;
    match input.mode.as_str() {
        "" | "tree" => {
            let windows = x11::windows(&app.config.display)?;
            let (png, tree) = tokio::join!(
                async { x11::scaled_png(&app.config.display, 1568) },
                a11y::tree(&windows)
            );
            let png = png?;
            Ok(vec![
                ContentBlock::image(
                    base64::engine::general_purpose::STANDARD.encode(png),
                    "image/png",
                ),
                ContentBlock::text(tree),
            ])
        }
        "png" => {
            let path = input.path.map_or_else(
                || {
                    let timestamp = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis();
                    std::env::temp_dir().join(format!("toad-computer-{timestamp}.png"))
                },
                Into::into,
            );
            let bytes = x11::raw_png(&app.config.display)?;
            tokio::fs::write(&path, bytes)
                .await
                .map_err(|error| format!("write {}: {error}", path.display()))?;
            Ok(text(path.to_string_lossy()))
        }
        mode => Err(action_error("capture", mode, &["tree", "png"])),
    }
}
