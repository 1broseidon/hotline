use serde::Deserialize;
use serde_json::Value;

use crate::App;

use super::{ToolResult, action_error, text};

#[derive(Deserialize)]
struct Input {
    action: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    js: String,
    #[serde(default)]
    r#ref: String,
    #[serde(default)]
    button: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    value: String,
    #[serde(default)]
    uncheck: bool,
    index: Option<usize>,
    #[serde(default)]
    path: String,
}

pub async fn call(app: &App, arguments: Value, holder: &str) -> ToolResult {
    let input: Input = serde_json::from_value(arguments).map_err(|error| error.to_string())?;
    let mutating = !matches!(
        input.action.as_str(),
        "text" | "links" | "eval" | "tabs" | "downloads"
    );
    let _guard = if mutating {
        Some(app.access.mutate(holder).await?)
    } else {
        None
    };
    let result = match input.action.as_str() {
        "navigate" => app.browser.navigate(&input.url).await,
        "text" => app.browser.text().await,
        "links" => app.browser.links().await,
        "eval" => app.browser.eval(&input.js).await,
        "click_ref" => {
            app.browser
                .click_ref(
                    &input.r#ref,
                    matches!(input.button.as_str(), "dbl" | "double"),
                )
                .await
        }
        "fill" => app.browser.fill(&input.r#ref, &input.text).await,
        "select" => app.browser.select(&input.r#ref, &input.value).await,
        "check" => app.browser.check(&input.r#ref, !input.uncheck).await,
        "hover" => app.browser.hover(&input.r#ref).await,
        "tabs" => app.browser.tabs().await,
        "tab_new" => app.browser.tab_new(&input.url).await,
        "tab_select" => {
            app.browser
                .tab_select(input.index.ok_or_else(|| "index is required".to_owned())?)
                .await
        }
        "tab_close" => app.browser.tab_close(input.index).await,
        "upload" => {
            if input.path.is_empty() {
                Err("path is required".into())
            } else {
                app.browser.upload(&input.r#ref, input.path.as_ref()).await
            }
        }
        "dialog_accept" => app.browser.dialog(true, &input.text).await,
        "dialog_dismiss" => app.browser.dialog(false, "").await,
        "downloads" => downloads(app).await,
        "back" | "forward" | "reload" => app.browser.history(&input.action).await,
        action => Err(action_error(
            "browser",
            action,
            &[
                "navigate",
                "text",
                "links",
                "eval",
                "click_ref",
                "fill",
                "select",
                "check",
                "hover",
                "tabs",
                "tab_new",
                "tab_select",
                "tab_close",
                "upload",
                "dialog_accept",
                "dialog_dismiss",
                "downloads",
                "back",
                "forward",
                "reload",
            ],
        )),
    }?;
    Ok(text(result))
}

async fn downloads(app: &App) -> Result<String, String> {
    let path = app.config.home.join("Downloads");
    let mut entries = match tokio::fs::read_dir(&path).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok("no downloads".into());
        }
        Err(error) => return Err(format!("downloads: {error}")),
    };
    let mut names = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| error.to_string())?
    {
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    Ok(if names.is_empty() {
        "no downloads".into()
    } else {
        names.join("\n")
    })
}
