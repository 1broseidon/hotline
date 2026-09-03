use serde::Deserialize;
use serde_json::Value;

use crate::{App, x11};

use super::{ToolResult, action_error, command, json_text, text};

#[derive(Deserialize)]
struct Input {
    action: String,
    #[serde(default)]
    window_id: String,
    #[serde(default)]
    unmaximize: bool,
}

pub async fn call(app: &App, arguments: Value, holder: &str) -> ToolResult {
    let input: Input = serde_json::from_value(arguments).map_err(|error| error.to_string())?;
    if input.action == "list" {
        return json_text(x11::windows(&app.config.display)?);
    }
    let _guard = app.access.mutate(holder).await?;
    match input.action.as_str() {
        "focus" => run(app, &["windowactivate", "--sync", required_id(&input)?]).await,
        "close" => run(app, &["windowclose", required_id(&input)?]).await,
        "maximize" => {
            let id = required_id(&input)?;
            x11::maximize(&app.config.display, id, !input.unmaximize)?;
            Ok(text("ok"))
        }
        "tile" => tile(app).await,
        action => Err(action_error(
            "windows",
            action,
            &["list", "focus", "close", "maximize", "tile"],
        )),
    }
}

fn required_id(input: &Input) -> Result<&str, String> {
    (!input.window_id.is_empty())
        .then_some(input.window_id.as_str())
        .ok_or_else(|| "window_id is required".to_owned())
}

async fn run(app: &App, arguments: &[&str]) -> ToolResult {
    let arguments: Vec<String> = arguments.iter().map(ToString::to_string).collect();
    command(&app.config.display, "xdotool", &arguments).await?;
    Ok(text("ok"))
}

async fn tile(app: &App) -> ToolResult {
    let windows = x11::windows(&app.config.display)?;
    if windows.is_empty() {
        return Ok(text("no windows"));
    }
    let shot = x11::screenshot(&app.config.display)?;
    let midpoint = shot.width as i32 / 2;
    let browser = windows.iter().position(|window| {
        window.class.to_ascii_lowercase().contains("chromium")
            || window.title.to_ascii_lowercase().contains("chromium")
    });
    let right_count = windows.len() - usize::from(browser.is_some());
    let right_height = if right_count == 0 {
        shot.height as i32
    } else {
        shot.height as i32 / right_count as i32
    };
    let mut right_index = 0_i32;
    for (index, window) in windows.iter().enumerate() {
        let (x, y, width, height) = if Some(index) == browser {
            (0, 0, midpoint, shot.height as i32)
        } else {
            let geometry = (
                midpoint,
                right_index * right_height,
                shot.width as i32 - midpoint,
                right_height,
            );
            right_index += 1;
            geometry
        };
        x11::maximize(&app.config.display, &window.id, false)?;
        run(
            app,
            &["windowmove", &window.id, &x.to_string(), &y.to_string()],
        )
        .await?;
        run(
            app,
            &[
                "windowsize",
                &window.id,
                &width.to_string(),
                &height.to_string(),
            ],
        )
        .await?;
    }
    Ok(text(format!("tiled {} windows", windows.len())))
}
