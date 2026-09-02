use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::Value;

use crate::{App, a11y, x11};

use super::{ToolResult, text};

#[derive(Deserialize)]
struct Input {
    text: String,
    timeout: Option<u64>,
}

pub async fn call(app: &App, arguments: Value) -> ToolResult {
    let input: Input = serde_json::from_value(arguments).map_err(|error| error.to_string())?;
    if input.text.is_empty() {
        return Err("text is required".to_owned());
    }
    let timeout = input.timeout.unwrap_or(10).min(60);
    let deadline = Instant::now() + Duration::from_secs(timeout);
    loop {
        let windows = x11::windows(&app.config.display).unwrap_or_default();
        let tree = a11y::tree(&windows).await;
        let browser = app.browser.page_text_if_running().await.unwrap_or_default();
        if tree.contains(&input.text) || browser.contains(&input.text) {
            return Ok(text(format!("found {:?}", input.text)));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "text {:?} did not appear within {timeout}s",
                input.text
            ));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
