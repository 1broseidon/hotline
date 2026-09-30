//! `door.json`, and the commands that knock on the running desk's Door.
//!
//! `serve` writes `door.json` beside `desk.lock` once its Door is up: the
//! loopback port, a token made for this process alone, and a little about the
//! process, written 0600 and renamed into place so a reader never sees half
//! of it. It is removed when the desk stops cleanly. One left behind by a
//! desk that crashed names a process that is gone, and is read as "not
//! running". The token is the desk seat's, so the file is exactly as secret
//! as the account that owns the data directory; it never leaves loopback.

use crate::room::Room;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::{self, IsTerminal, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

pub const DOOR_FILE: &str = "door.json";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoorFile {
    pub pid: u32,
    pub port: u16,
    pub token: String,
    pub version: String,
    pub started_at: String,
    pub data_dir: String,
    pub store: String,
}

pub fn path(root: &Path) -> PathBuf {
    root.join(DOOR_FILE)
}

pub fn write(root: &Path, door: &DoorFile) -> io::Result<()> {
    let staged = root.join(format!("{DOOR_FILE}.{}", std::process::id()));
    let _ = fs::remove_file(&staged);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&staged)?;
    file.write_all(&serde_json::to_vec_pretty(door)?)?;
    file.sync_all()?;
    fs::rename(&staged, path(root))
}

/// Removes `door.json` if it is still this process's.
pub fn remove(root: &Path) {
    if read(root).is_ok_and(|door| door.pid == std::process::id()) {
        let _ = fs::remove_file(path(root));
    }
}

fn read(root: &Path) -> io::Result<DoorFile> {
    let bytes = fs::read(path(root))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn alive(pid: u32) -> bool {
    // SAFETY: signal 0 checks for the process without sending anything.
    let answer = unsafe { libc::kill(pid as libc::pid_t, 0) };
    answer == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// What a data folder holds for a client that wants to knock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Presence {
    /// A `door.json` naming a process that is running.
    Live,
    /// A `door.json` with no desk behind it, or one that cannot be understood.
    Stale,
    /// A folder or file this account may not enter.
    Unreadable,
    /// No `door.json`.
    Missing,
}

pub(crate) fn presence(root: &Path) -> Presence {
    match read(root) {
        Ok(door) if alive(door.pid) => Presence::Live,
        Ok(_) => Presence::Stale,
        Err(error) => match error.kind() {
            io::ErrorKind::PermissionDenied => Presence::Unreadable,
            io::ErrorKind::NotFound | io::ErrorKind::NotADirectory => Presence::Missing,
            _ => Presence::Stale,
        },
    }
}

/// The running desk's Door, or why there is none.
pub(crate) fn running(room: &Room) -> Result<DoorFile, String> {
    let root = &room.root;
    match read(root) {
        Ok(door) if alive(door.pid) => Ok(door),
        Ok(door) => Err(format!(
            "Hotline is not running on {} (door.json names process {}, which is gone).",
            root.display(),
            door.pid
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Err(format!(
            "Hotline is not running on {} (no door.json). Is the service started? Try `systemctl status hotline`.{}",
            root.display(),
            room.served_hint()
        )),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => Err(room.refused(&error)),
        Err(error) => Err(format!(
            "{} could not be read: {error}",
            path(root).display()
        )),
    }
}

/// Sends one command on the desk seat and returns its result.
pub async fn ask(door: &DoorFile, cmd: &str, params: Value) -> Result<Value, String> {
    let url = format!("ws://127.0.0.1:{}/ws?token={}", door.port, door.token);
    let (mut socket, _) = tokio::time::timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(url),
    )
    .await
    .map_err(|_| "The desk did not answer its door within 5 seconds.".to_string())?
    .map_err(|error| format!("The desk's door refused the connection: {error}"))?;
    let frame = json!({ "id": 1, "cmd": cmd, "params": params });
    socket
        .send(Message::Text(frame.to_string().into()))
        .await
        .map_err(|error| error.to_string())?;
    let answer = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(message) = socket.next().await {
            let Ok(Message::Text(text)) = message else {
                continue;
            };
            let Ok(frame) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if frame.get("id").and_then(Value::as_i64) == Some(1) {
                return Some(frame);
            }
        }
        None
    })
    .await
    .map_err(|_| format!("The desk did not answer {cmd} within 30 seconds."))?
    .ok_or_else(|| "The desk closed the door before answering.".to_string())?;
    let _ = socket.close(None).await;
    if answer.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(answer.get("result").cloned().unwrap_or(Value::Null))
    } else {
        Err(answer
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("The desk refused that command.")
            .to_string())
    }
}

pub(crate) fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime starts")
}

pub fn status(room: &Room) -> ExitCode {
    let door = match running(room) {
        Ok(door) => door,
        Err(problem) => {
            println!("{problem}");
            return ExitCode::from(3);
        }
    };
    let since = chrono::DateTime::parse_from_rfc3339(&door.started_at)
        .map(|at| describe(chrono::Utc::now().signed_duration_since(at)))
        .unwrap_or_else(|_| "unknown".into());
    println!(
        "Hotline {} · running · pid {} · up {since}",
        door.version, door.pid
    );
    println!("  data    {}", door.data_dir);
    println!("  store   {}", door.store);
    match runtime().block_on(ask(&door, "welcome", json!({}))) {
        Ok(welcome) => {
            let providers = welcome["providers"]
                .as_array()
                .map(|all| {
                    all.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            println!(
                "  teammates  {}",
                welcome["teammates"].as_u64().unwrap_or(0)
            );
            println!(
                "  models     {}",
                if providers.is_empty() {
                    "none connected".to_string()
                } else {
                    providers
                }
            );
            if welcome["canRun"].as_bool() == Some(false) {
                println!("  ! No teammate can run yet: connect a model provider.");
            }
            ExitCode::SUCCESS
        }
        Err(problem) => {
            println!("  ! The desk did not answer: {problem}");
            ExitCode::from(1)
        }
    }
}

fn describe(elapsed: chrono::TimeDelta) -> String {
    let minutes = elapsed.num_minutes().max(0);
    match minutes {
        0 => format!("{}s", elapsed.num_seconds().max(0)),
        1..60 => format!("{minutes}m"),
        60..1440 => format!("{}h {}m", minutes / 60, minutes % 60),
        _ => format!("{}d {}h", minutes / 1440, (minutes % 1440) / 60),
    }
}

pub fn wire(room: &Room, cmd: &str) -> ExitCode {
    let door = match running(room) {
        Ok(door) => door,
        Err(problem) => {
            eprintln!("{problem}");
            return ExitCode::from(3);
        }
    };
    let mut input = String::new();
    if !io::stdin().is_terminal() && io::stdin().read_to_string(&mut input).is_err() {
        eprintln!("stdin could not be read");
        return ExitCode::from(2);
    }
    let params = if input.trim().is_empty() {
        json!({})
    } else {
        match serde_json::from_str::<Value>(&input) {
            Ok(params) => params,
            Err(error) => {
                eprintln!("stdin is not JSON: {error}");
                return ExitCode::from(2);
            }
        }
    };
    match runtime().block_on(ask(&door, cmd, params)) {
        Ok(result) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&result).unwrap_or_default()
            );
            ExitCode::SUCCESS
        }
        Err(problem) => {
            eprintln!("{problem}");
            ExitCode::from(1)
        }
    }
}
