use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::App;

use super::{ToolResult, action_error, json_text, text};

#[derive(Deserialize)]
struct Input {
    action: String,
    #[serde(default)]
    name: String,
    duration: Option<u64>,
}

#[derive(Serialize, Deserialize)]
struct SavedLogin {
    name: String,
    browser: String,
    created_at: String,
    #[serde(default)]
    last_used_at: String,
    cookies: Value,
    storage: Value,
}

pub async fn call(app: &App, arguments: Value, holder: &str) -> ToolResult {
    let input: Input = serde_json::from_value(arguments).map_err(|error| error.to_string())?;
    match input.action.as_str() {
        "control" => control(app, holder, input.duration).await,
        "release" => release(app, holder).await,
        "login_list" => login_list(app).await,
        "snapshot_list" => snapshot_list(app).await,
        "login_save" | "login_load" | "login_delete" | "snapshot_save" | "snapshot_load"
        | "snapshot_delete" => {
            valid_name(&input.name)?;
            let _guard = app.access.mutate(holder).await?;
            match input.action.as_str() {
                "login_save" => login_save(app, &input.name).await,
                "login_load" => login_load(app, &input.name).await,
                "login_delete" => login_delete(app, &input.name).await,
                "snapshot_save" => snapshot_save(app, &input.name).await,
                "snapshot_load" => snapshot_load(app, &input.name).await,
                "snapshot_delete" => snapshot_delete(app, &input.name).await,
                _ => unreachable!(),
            }
        }
        action => Err(action_error(
            "state",
            action,
            &[
                "control",
                "release",
                "login_save",
                "login_load",
                "login_list",
                "login_delete",
                "snapshot_save",
                "snapshot_load",
                "snapshot_list",
                "snapshot_delete",
            ],
        )),
    }
}

async fn control(app: &App, holder: &str, duration: Option<u64>) -> ToolResult {
    let (duration, expires) = app.access.control(holder, duration).await?;
    let expires = expires
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    json_text(json!({"granted":true,"holder":holder,"expires_at":expires,"duration_s":duration}))
}

async fn release(app: &App, holder: &str) -> ToolResult {
    let released = app.access.release(holder).await?;
    let mut value = json!({"released":released,"holder":holder});
    if !released {
        value["detail"] = json!("no control lease was held");
    }
    json_text(value)
}

async fn login_save(app: &App, name: &str) -> ToolResult {
    let directory = app.config.home.join(".toad/logins");
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| error.to_string())?;
    let saved = SavedLogin {
        name: name.to_owned(),
        browser: "chromium".into(),
        created_at: now(),
        last_used_at: String::new(),
        cookies: app.browser.cookies().await?,
        storage: app.browser.local_storage().await?,
    };
    let data = serde_json::to_vec(&saved).map_err(|error| error.to_string())?;
    tokio::fs::write(directory.join(format!("{name}.json")), data)
        .await
        .map_err(|error| error.to_string())?;
    json_text(json!({"saved":true,"name":name}))
}

async fn login_load(app: &App, name: &str) -> ToolResult {
    let path = app
        .config
        .home
        .join(".toad/logins")
        .join(format!("{name}.json"));
    let data = tokio::fs::read(&path)
        .await
        .map_err(|_| format!("saved login {name:?} not found"))?;
    let mut saved: SavedLogin = serde_json::from_slice(&data).map_err(|error| error.to_string())?;
    app.browser
        .restore(saved.cookies.clone(), saved.storage.clone())
        .await?;
    saved.last_used_at = now();
    tokio::fs::write(
        &path,
        serde_json::to_vec(&saved).map_err(|error| error.to_string())?,
    )
    .await
    .map_err(|error| error.to_string())?;
    json_text(json!({"loaded":true,"name":name}))
}

async fn login_list(app: &App) -> ToolResult {
    let directory = app.config.home.join(".toad/logins");
    let mut entries = match tokio::fs::read_dir(directory).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(text("no saved logins"));
        }
        Err(error) => return Err(format!("login_list: {error}")),
    };
    let mut logins = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| error.to_string())?
    {
        if entry.path().extension().and_then(|part| part.to_str()) != Some("json") {
            continue;
        }
        if let Ok(data) = tokio::fs::read(entry.path()).await
            && let Ok(login) = serde_json::from_slice::<SavedLogin>(&data)
        {
            logins.push(json!({"name":login.name,"browser":login.browser,"created_at":login.created_at,"last_used_at":login.last_used_at}));
        }
    }
    if logins.is_empty() {
        Ok(text("no saved logins"))
    } else {
        json_text(logins)
    }
}

async fn login_delete(app: &App, name: &str) -> ToolResult {
    let path = app
        .config
        .home
        .join(".toad/logins")
        .join(format!("{name}.json"));
    tokio::fs::remove_file(&path)
        .await
        .map_err(|_| format!("saved login {name:?} not found"))?;
    json_text(json!({"deleted":true,"name":name}))
}

fn snapshot_path(app: &App, name: &str) -> PathBuf {
    app.config
        .home
        .join(".toad/snapshots")
        .join(format!("{name}.tar.gz"))
}

async fn snapshot_save(app: &App, name: &str) -> ToolResult {
    let path = snapshot_path(app, name);
    tokio::fs::create_dir_all(path.parent().expect("snapshot has parent"))
        .await
        .map_err(|error| error.to_string())?;
    tar(&[
        "czf",
        path_str(&path)?,
        "--exclude=.toad/snapshots",
        "-C",
        path_str(&app.config.home)?,
        ".",
    ])
    .await?;
    let size = tokio::fs::metadata(&path)
        .await
        .map_err(|error| error.to_string())?
        .len();
    if size > 500 * 1024 * 1024 {
        tokio::fs::remove_file(&path).await.ok();
        return Err(format!(
            "snapshot too large: {:.1} MB (max 500 MB)",
            size as f64 / 1_048_576.0
        ));
    }
    json_text(
        json!({"saved":true,"name":name,"size_mb":format!("{:.1}",size as f64/1_048_576.0),"browser_state":true}),
    )
}

async fn snapshot_load(app: &App, name: &str) -> ToolResult {
    if !desktop_fresh(&app.config.home).await? {
        return Err(format!(
            "snapshot_load requires a fresh desktop — user files already exist in {}/",
            app.config.home.display()
        ));
    }
    let path = snapshot_path(app, name);
    if !path.is_file() {
        return Err(format!("snapshot {name:?} not found"));
    }
    tar(&[
        "xzf",
        path_str(&path)?,
        "-C",
        path_str(&app.config.home)?,
        "--overwrite",
    ])
    .await?;
    json_text(json!({"loaded":true,"name":name,"browser_restored":true}))
}

async fn snapshot_list(app: &App) -> ToolResult {
    let directory = app.config.home.join(".toad/snapshots");
    let mut entries = match tokio::fs::read_dir(directory).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(text("no snapshots"));
        }
        Err(error) => return Err(format!("snapshot_list: {error}")),
    };
    let mut snapshots = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| error.to_string())?
    {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let Some(name) = file_name.strip_suffix(".tar.gz") else {
            continue;
        };
        let size = entry
            .metadata()
            .await
            .map_err(|error| error.to_string())?
            .len();
        snapshots.push(json!({"name":name,"size_mb":size as f64/1_048_576.0}));
    }
    if snapshots.is_empty() {
        Ok(text("no snapshots"))
    } else {
        json_text(snapshots)
    }
}

async fn snapshot_delete(app: &App, name: &str) -> ToolResult {
    tokio::fs::remove_file(snapshot_path(app, name))
        .await
        .map_err(|_| format!("snapshot {name:?} not found"))?;
    json_text(json!({"deleted":true,"name":name}))
}

async fn tar(arguments: &[&str]) -> Result<(), String> {
    let output = tokio::process::Command::new("tar")
        .args(arguments)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|error| format!("tar: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "tar: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

async fn desktop_fresh(home: &Path) -> Result<bool, String> {
    let mut entries = tokio::fs::read_dir(home)
        .await
        .map_err(|error| error.to_string())?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| error.to_string())?
    {
        if !entry.file_name().to_string_lossy().starts_with('.') {
            return Ok(false);
        }
    }
    Ok(true)
}

fn valid_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("name is required".into());
    }
    if !name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err("name may contain only letters, numbers, '-' and '_'".into());
    }
    Ok(())
}

fn path_str(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| format!("path is not UTF-8: {}", path.display()))
}

fn now() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_cannot_escape_the_state_directories() {
        assert!(valid_name("github-work").is_ok());
        assert!(valid_name("../outside").is_err());
    }
}
