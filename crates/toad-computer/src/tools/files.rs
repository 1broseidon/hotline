use std::path::{Component, Path, PathBuf};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::App;

use super::{ToolResult, action_error, json_text, text};

const MAX_FILE_SIZE: u64 = 50 << 20;

#[derive(Deserialize)]
struct Input {
    action: String,
    path: PathBuf,
    #[serde(default)]
    content: String,
    #[serde(default)]
    encoding: String,
}

#[derive(Serialize)]
struct Entry {
    name: String,
    size: u64,
    is_dir: bool,
    modified: u64,
}

pub async fn call(app: &App, arguments: Value) -> ToolResult {
    let input: Input = serde_json::from_value(arguments).map_err(|error| error.to_string())?;
    match input.action.as_str() {
        "get" => get(app, &input.path).await,
        "put" => put(app, &input.path, &input.content, &input.encoding).await,
        "list" => list(app, &input.path).await,
        action => Err(action_error("files", action, &["get", "put", "list"])),
    }
}

async fn get(app: &App, path: &Path) -> ToolResult {
    let path = existing_path(app, path).await?;
    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|_| format!("file not found: {}", path.display()))?;
    if metadata.is_dir() {
        return Err("path is a directory, use files action=list".to_owned());
    }
    if metadata.len() > MAX_FILE_SIZE {
        return Err(format!(
            "file too large ({} bytes, max {MAX_FILE_SIZE})",
            metadata.len()
        ));
    }
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|error| error.to_string())?;
    match String::from_utf8(bytes) {
        Ok(value) => Ok(text(value)),
        Err(error) => Ok(text(format!(
            "encoding=base64\npath={}\n{}",
            path.display(),
            base64::engine::general_purpose::STANDARD.encode(error.into_bytes())
        ))),
    }
}

async fn put(app: &App, path: &Path, content: &str, encoding: &str) -> ToolResult {
    let bytes = match encoding.to_ascii_lowercase().as_str() {
        "" | "utf8" | "text" => content.as_bytes().to_vec(),
        "base64" => base64::engine::general_purpose::STANDARD
            .decode(content)
            .map_err(|error| format!("invalid base64: {error}"))?,
        _ => return Err("encoding must be utf8 or base64".to_owned()),
    };
    if bytes.len() as u64 > MAX_FILE_SIZE {
        return Err(format!(
            "file too large ({} bytes, max {MAX_FILE_SIZE})",
            bytes.len()
        ));
    }
    let path = writable_path(app, path).await?;
    tokio::fs::write(&path, &bytes)
        .await
        .map_err(|error| error.to_string())?;
    Ok(text(format!(
        "wrote {} ({} bytes)",
        path.display(),
        bytes.len()
    )))
}

async fn list(app: &App, path: &Path) -> ToolResult {
    let path = existing_path(app, path).await?;
    let mut directory = tokio::fs::read_dir(&path)
        .await
        .map_err(|_| format!("cannot read directory: {}", path.display()))?;
    let mut entries = Vec::new();
    while let Some(entry) = directory
        .next_entry()
        .await
        .map_err(|error| error.to_string())?
    {
        let metadata = entry.metadata().await.map_err(|error| error.to_string())?;
        entries.push(Entry {
            name: entry.file_name().to_string_lossy().into_owned(),
            size: metadata.len(),
            is_dir: metadata.is_dir(),
            modified: metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |duration| duration.as_secs()),
        });
    }
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    json_text(entries)
}

async fn existing_path(app: &App, requested: &Path) -> Result<PathBuf, String> {
    let home = tokio::fs::canonicalize(&app.config.home)
        .await
        .map_err(|error| format!("home: {error}"))?;
    let path = tokio::fs::canonicalize(requested)
        .await
        .map_err(|_| format!("path must be under {}/", home.display()))?;
    if path != home && !path.starts_with(&home) {
        return Err(format!("path must be under {}/", home.display()));
    }
    Ok(path)
}

async fn writable_path(app: &App, requested: &Path) -> Result<PathBuf, String> {
    let home = tokio::fs::canonicalize(&app.config.home)
        .await
        .map_err(|error| format!("home: {error}"))?;
    match tokio::fs::symlink_metadata(requested).await {
        Ok(_) => {
            let path = tokio::fs::canonicalize(requested)
                .await
                .map_err(|_| format!("path must be under {}/", home.display()))?;
            require_under_home(&home, &path)?;
            return Ok(path);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }

    let file_name = requested
        .file_name()
        .ok_or_else(|| "path has no file name".to_owned())?;
    let mut ancestor = requested
        .parent()
        .ok_or_else(|| "path has no parent".to_owned())?
        .to_path_buf();
    if ancestor
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(format!("path must be under {}/", home.display()));
    }

    let mut missing = Vec::new();
    let existing = loop {
        match tokio::fs::symlink_metadata(&ancestor).await {
            Ok(_) => {
                break tokio::fs::canonicalize(&ancestor)
                    .await
                    .map_err(|_| format!("path must be under {}/", home.display()))?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(
                    ancestor
                        .file_name()
                        .ok_or_else(|| "path has no parent".to_owned())?
                        .to_owned(),
                );
                ancestor = ancestor
                    .parent()
                    .ok_or_else(|| "path has no parent".to_owned())?
                    .to_path_buf();
            }
            Err(error) => return Err(error.to_string()),
        }
    };
    require_under_home(&home, &existing)?;

    let mut parent = existing;
    for component in missing.iter().rev() {
        parent.push(component);
    }
    tokio::fs::create_dir_all(&parent)
        .await
        .map_err(|error| error.to_string())?;
    let parent = tokio::fs::canonicalize(parent)
        .await
        .map_err(|error| error.to_string())?;
    require_under_home(&home, &parent)?;
    Ok(parent.join(file_name))
}

fn require_under_home(home: &Path, path: &Path) -> Result<(), String> {
    if path == home || path.starts_with(home) {
        Ok(())
    } else {
        Err(format!("path must be under {}/", home.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    #[tokio::test]
    async fn refuses_a_symlink_that_leaves_home() {
        let home = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), home.path().join("escape")).unwrap();
        let app = App::new(Config {
            addr: String::new(),
            token: None,
            home: home.path().to_owned(),
            display: ":0".to_owned(),
        });
        let error = put(&app, &home.path().join("escape/file"), "no", "")
            .await
            .unwrap_err();
        assert!(error.contains("path must be under"));
    }

    #[tokio::test]
    async fn refuses_before_creating_a_parent_outside_home() {
        let home = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let app = App::new(Config {
            addr: String::new(),
            token: None,
            home: home.path().to_owned(),
            display: ":0".to_owned(),
        });
        let parent = outside.path().join("evil");
        let error = put(&app, &parent.join("file"), "no", "").await.unwrap_err();
        assert!(error.contains("path must be under"));
        assert!(!parent.exists());
    }

    #[tokio::test]
    async fn refuses_an_existing_file_symlink_that_leaves_home() {
        let home = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("target");
        std::fs::write(&target, "safe").unwrap();
        let link = home.path().join("file");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let app = App::new(Config {
            addr: String::new(),
            token: None,
            home: home.path().to_owned(),
            display: ":0".to_owned(),
        });
        let error = put(&app, &link, "no", "").await.unwrap_err();
        assert!(error.contains("path must be under"));
        assert_eq!(std::fs::read_to_string(target).unwrap(), "safe");
    }
}
