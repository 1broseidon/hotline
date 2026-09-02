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
        "focus" => run(app, &["-i", "-a", required_id(&input)?]).await,
        "close" => run(app, &["-i", "-c", required_id(&input)?]).await,
        "maximize" => {
            let verb = if input.unmaximize {
                "remove,maximized_vert,maximized_horz"
            } else {
                "add,maximized_vert,maximized_horz"
            };
            run(app, &["-i", "-r", required_id(&input)?, "-b", verb]).await
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
    command(&app.config.display, "wmctrl", &arguments).await?;
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
        run(
            app,
            &[
                "-i",
                "-r",
                &window.id,
                "-b",
                "remove,maximized_vert,maximized_horz",
            ],
        )
        .await?;
        run(
            app,
            &[
                "-i",
                "-r",
                &window.id,
                "-e",
                &format!("0,{x},{y},{width},{height}"),
            ],
        )
        .await?;
    }
    Ok(text(format!("tiled {} windows", windows.len())))
}
