//! The desks the window shows (BRO-145): this computer's, and every remote
//! desk it is paired with, each as an endpoint the window dials the same
//! way — the local Door, or the loopback bridge to a remote desk. The
//! window reads the list as `window.__hotlineDesks` (see `ui/src/desks.ts`).
//!
//! Remote desks are the ones this computer has paired with (`desks.json` in
//! the room, the keys in the OS credential store via `remote::client`), each
//! reached through a loopback bridge the shell starts at launch. For
//! development, `HOTLINE_EXTRA_DESKS` also adds desks by endpoint:
//! a JSON list of `{ "id", "name", "origin", "token" }`, for example a second
//! `hotline serve` on this machine, whose Door speaks the same wire contract
//! the bridge will.

use hotline_core::remote::PairingPayload;
use hotline_core::remote::bridge::Bridge;
use hotline_core::remote::client::{PairedDesk, State};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use tauri::{AppHandle, Emitter};

const REGISTRY: &str = "desks.json";
/// The event the window listens on for a new list (see `ui/src/desks.ts`).
const EVENT: &str = "hotline://desks";

/// The desks this window shows, and the bridges that reach the remote ones.
pub(crate) struct Host {
    registry: PathBuf,
    local: (String, String),
    paired: Mutex<Vec<PairedDesk>>,
    bridges: Mutex<HashMap<String, Live>>,
    app: OnceLock<AppHandle>,
}

/// A running bridge and the last state it reported.
struct Live {
    bridge: Bridge,
    state: State,
}

impl Host {
    /// The local desk's endpoint and whatever this computer has paired with.
    pub(crate) fn open(root: &Path, origin: String, token: String) -> Arc<Self> {
        let registry = root.join(REGISTRY);
        let paired = std::fs::read(&registry)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<PairedDesk>>(&bytes).ok())
            .unwrap_or_default();
        Arc::new(Self {
            registry,
            local: (origin, token),
            paired: Mutex::new(paired),
            bridges: Mutex::new(HashMap::new()),
            app: OnceLock::new(),
        })
    }

    /// The data directory the registry lives in.
    pub(crate) fn root(&self) -> &Path {
        self.registry.parent().unwrap_or(Path::new("."))
    }

    /// Starts a bridge for every paired desk. A bridge is a local listener;
    /// it dials the server only once the window connects to it.
    pub(crate) async fn start_all(self: &Arc<Self>) {
        let paired = self
            .paired
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for desk in paired {
            if let Err(error) = self.start(&desk).await {
                eprintln!("[desks] {}: {error}", desk.name);
            }
        }
    }

    /// Where the window's list changes go, once there is a window.
    pub(crate) fn attach(&self, app: AppHandle) {
        let _ = self.app.set(app);
    }

    async fn start(self: &Arc<Self>, desk: &PairedDesk) -> Result<(), String> {
        let bridge = Bridge::start(desk).await?;
        let mut watch = bridge.state.clone();
        let state = *watch.borrow();
        self.bridges
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(desk.desk_id.clone(), Live { bridge, state });
        let host = Arc::downgrade(self);
        let id = desk.desk_id.clone();
        tauri::async_runtime::spawn(async move {
            while watch.changed().await.is_ok() {
                let Some(host) = host.upgrade() else { return };
                let now = *watch.borrow();
                if let Some(live) = host
                    .bridges
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get_mut(&id)
                {
                    live.state = now;
                }
                host.emit();
            }
        });
        Ok(())
    }

    /// The local desk, the paired ones, then any extra ones, as the window reads them.
    pub(crate) fn listed(&self) -> Value {
        let mut desks = vec![json!({
            "id": "local",
            "name": "This computer",
            "kind": "local",
            "origin": self.local.0,
            "token": self.local.1,
        })];
        let bridges = self.bridges.lock().unwrap_or_else(PoisonError::into_inner);
        for desk in self
            .paired
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
        {
            let Some(live) = bridges.get(&desk.desk_id) else {
                continue;
            };
            desks.push(json!({
                "id": desk.desk_id,
                "name": desk.name,
                "kind": "remote",
                "origin": live.bridge.origin,
                "token": live.bridge.token,
                "state": live.state,
            }));
        }
        desks.extend(extra(std::env::var("HOTLINE_EXTRA_DESKS").ok().as_deref()));
        Value::Array(desks)
    }

    fn emit(&self) {
        if let Some(app) = self.app.get() {
            let _ = app.emit(EVENT, self.listed());
        }
    }

    fn save(&self) -> Result<(), String> {
        let paired = self.paired.lock().unwrap_or_else(PoisonError::into_inner);
        let bytes = serde_json::to_vec_pretty(&*paired).map_err(|error| error.to_string())?;
        let staged = self
            .registry
            .with_extension(format!("json.{}", std::process::id()));
        std::fs::write(&staged, bytes)
            .and_then(|()| std::fs::rename(&staged, &self.registry))
            .map_err(|error| format!("Couldn't save the paired desks: {error}"))
    }

    /// Claims the invitation as this computer, keeps the desk, starts its
    /// bridge and tells the window. Pairing the same desk again replaces it.
    async fn claim(self: &Arc<Self>, payload: Value) -> Result<String, String> {
        let payload: PairingPayload = serde_json::from_value(payload).map_err(|_| {
            "The invitation isn't one this Hotline understands. Update Hotline on both ends."
                .to_string()
        })?;
        let desk = hotline_core::remote::client::pair(&payload, &device_name()).await?;
        self.bridges
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&desk.desk_id);
        {
            let mut paired = self.paired.lock().unwrap_or_else(PoisonError::into_inner);
            paired.retain(|known| known.desk_id != desk.desk_id);
            paired.push(desk.clone());
        }
        self.save()?;
        self.start(&desk).await?;
        self.emit();
        Ok(desk.desk_id)
    }

    /// Asks a desk on a server one thing, from the shell rather than the
    /// page, over its bridge: for what the page should never hold, like the
    /// cookie values `laptop` reads.
    pub(crate) async fn command(
        &self,
        desk_id: &str,
        cmd: &str,
        params: Value,
    ) -> Result<Value, String> {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        let url = {
            let bridges = self.bridges.lock().unwrap_or_else(PoisonError::into_inner);
            let live = bridges
                .get(desk_id)
                .ok_or("That desk is not paired with this computer.")?;
            format!(
                "{}/ws?token={}",
                live.bridge.origin.replacen("http", "ws", 1),
                live.bridge.token
            )
        };
        let unreachable =
            |error: &dyn std::fmt::Display| format!("The desk could not be reached: {error}");
        let (mut socket, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .map_err(|error| unreachable(&error))?;
        let frame = json!({"id": 1, "cmd": cmd, "params": params});
        socket
            .send(Message::text(frame.to_string()))
            .await
            .map_err(|error| unreachable(&error))?;
        let answer = tokio::time::timeout(std::time::Duration::from_secs(120), async {
            while let Some(message) = socket.next().await {
                let Ok(Message::Text(text)) = message else {
                    continue;
                };
                let Ok(reply) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                if reply["id"] == 1 {
                    return Some(reply);
                }
            }
            None
        })
        .await
        .ok()
        .flatten();
        let _ = socket.close(None).await;
        let reply = answer.ok_or("The desk did not answer.")?;
        if reply["ok"] == true {
            Ok(reply["result"].clone())
        } else {
            Err(reply["error"]
                .as_str()
                .unwrap_or("The desk refused that.")
                .to_string())
        }
    }

    /// Stops reaching a desk from this window and forgets it.
    fn forget(&self, desk_id: &str) -> Result<(), String> {
        self.bridges
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(desk_id);
        self.paired
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|known| known.desk_id != desk_id);
        self.save()?;
        self.emit();
        Ok(())
    }
}

/// What the server lists this computer as among its devices.
fn device_name() -> String {
    let named = std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty() && name.len() <= 60);
    match named {
        Some(host) => format!("Hotline on {host}"),
        None => "Hotline desktop".to_string(),
    }
}

/// Desks from `HOTLINE_EXTRA_DESKS`. A malformed entry is skipped and said,
/// rather than stopping the window from opening.
fn extra(raw: Option<&str>) -> Vec<Value> {
    let Some(raw) = raw.filter(|raw| !raw.trim().is_empty()) else {
        return Vec::new();
    };
    let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(raw) else {
        eprintln!("[desks] HOTLINE_EXTRA_DESKS is not a JSON list; ignoring it");
        return Vec::new();
    };
    entries
        .into_iter()
        .filter_map(|entry| {
            let field = |name: &str| entry.get(name).and_then(Value::as_str).map(str::to_string);
            match (field("id"), field("name"), field("origin"), field("token")) {
                (Some(id), Some(name), Some(origin), Some(token)) if id != "local" => Some(json!({
                    "id": id,
                    "name": name,
                    "kind": "remote",
                    "origin": origin,
                    "token": token,
                    "state": "open",
                })),
                _ => {
                    eprintln!("[desks] skipping an extra desk without id, name, origin and token");
                    None
                }
            }
        })
        .collect()
}

// ---------------------------------------------------------------- pairing

/// How long SSH pairing waits for the server to print its invitation.
const SSH_PAYLOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// The payload in a `hotline://pair?p=<base64url(json)>` link, the same one
/// the phone's QR carries. Its fields are the server's to name; the window
/// only unwraps it.
fn payload_from_link(link: &str) -> Result<Value, String> {
    use base64::Engine;
    let rest = link
        .trim()
        .strip_prefix("hotline://pair?")
        .ok_or("That isn't a pairing link. On the server, run `hotline pair --link` and paste what it prints.")?;
    let encoded = rest
        .split('&')
        .find_map(|pair| pair.strip_prefix("p="))
        .ok_or("The pairing link is missing its invitation.")?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded.trim_end_matches('='))
        .map_err(|_| "The pairing link is damaged. Copy it again from the server.".to_string())?;
    let payload: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "The pairing link is damaged. Copy it again from the server.".to_string())?;
    if !payload.is_object() {
        return Err("The pairing link is damaged. Copy it again from the server.".into());
    }
    Ok(payload)
}

/// The first line of `output` that is a JSON object: `hotline pair --json`
/// prints its payload as one line and then waits for the claim, and a
/// login banner or MOTD may come before it.
fn payload_from_output(output: &str) -> Option<Value> {
    output
        .lines()
        .filter(|line| line.trim_start().starts_with('{'))
        .find_map(|line| {
            serde_json::from_str::<Value>(line.trim())
                .ok()
                .filter(Value::is_object)
        })
}

/// Runs `hotline pair --json` on `target` with the person's own `ssh`, and
/// answers the payload as soon as it is printed, with the ssh still running:
/// the server's pair command waits until the invitation is claimed (or runs
/// out), so the caller claims it and then lets the child finish.
async fn payload_over_ssh(target: &str) -> Result<(Value, tokio::process::Child), String> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    if target.is_empty() || target.starts_with('-') || target.contains(char::is_whitespace) {
        return Err("Enter the server as user@host, or a host from your SSH config.".into());
    }
    let mut child = tokio::process::Command::new("ssh")
        // Never a password prompt: this window has no terminal to show one.
        .args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=10",
            "--",
            target,
            "hotline",
            "pair",
            "--json",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("Couldn't run ssh: {error}"))?;
    let stdout = child.stdout.take().ok_or("ssh gave no output")?;
    let mut lines = BufReader::new(stdout).lines();
    let found = tokio::time::timeout(SSH_PAYLOAD_TIMEOUT, async {
        let mut seen = String::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(payload) = payload_from_output(&line) {
                return Some(payload);
            }
            seen.push_str(&line);
            seen.push('\n');
        }
        None
    })
    .await;
    match found {
        Ok(Some(payload)) => Ok((payload, child)),
        Ok(None) => {
            let output = child
                .wait_with_output()
                .await
                .map_err(|error| error.to_string())?;
            let said = String::from_utf8_lossy(&output.stderr);
            let said = said.trim();
            Err(
                if said.contains("Permission denied")
                    || said.contains("Host key verification failed")
                {
                    format!(
                        "ssh couldn't sign in to {target} without a prompt. Check you can run `ssh {target}` from a terminal first."
                    )
                } else if said.contains("command not found") || said.contains("unknown command") {
                    format!(
                        "{target} doesn't have a `hotline` new enough to pair a desktop. Update it with `hotline update`."
                    )
                } else if said.is_empty() {
                    format!("{target} didn't print an invitation.")
                } else {
                    format!("Pairing over ssh failed: {said}")
                },
            )
        }
        Err(_) => Err(format!(
            "{target} didn't print an invitation within {} seconds.",
            SSH_PAYLOAD_TIMEOUT.as_secs()
        )),
    }
}

#[tauri::command]
pub async fn desk_pair_link(
    host: tauri::State<'_, Arc<Host>>,
    link: String,
) -> Result<String, String> {
    host.inner().claim(payload_from_link(&link)?).await
}

#[tauri::command]
pub async fn desk_pair_ssh(
    host: tauri::State<'_, Arc<Host>>,
    target: String,
) -> Result<String, String> {
    let (payload, child) = payload_over_ssh(target.trim()).await?;
    let claimed = host.inner().claim(payload).await;
    // The server's pair command ends by itself once the invitation is
    // claimed or expires; dropping the child ends it if it is still waiting.
    drop(child);
    claimed
}

/// The desks as they are now. The list the page was given at creation is
/// stale after a reload once a desk was paired or forgotten since.
#[tauri::command]
pub fn desk_list(host: tauri::State<'_, Arc<Host>>) -> Value {
    host.listed()
}

#[tauri::command]
pub async fn desk_forget(host: tauri::State<'_, Arc<Host>>, desk_id: String) -> Result<(), String> {
    host.forget(&desk_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_unwraps_to_its_payload_and_anything_else_says_what_to_do() {
        use base64::Engine;
        let payload = r#"{"v":1,"url":"https://desk.example:9443"}"#;
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload);
        let unwrapped = payload_from_link(&format!("  hotline://pair?p={encoded}\n")).unwrap();
        assert_eq!(unwrapped["url"], "https://desk.example:9443");
        assert!(
            payload_from_link("https://example.com")
                .unwrap_err()
                .contains("hotline pair --link")
        );
        assert!(
            payload_from_link("hotline://pair?p=%%%")
                .unwrap_err()
                .contains("damaged")
        );
        assert!(
            payload_from_link("hotline://pair?x=1")
                .unwrap_err()
                .contains("missing")
        );
    }

    #[test]
    fn ssh_output_yields_the_first_json_line_after_any_banner() {
        let output =
            "Welcome to grizzly\nLast login: today\n{\"v\":1,\"url\":\"https://d\"}\nwaiting…\n";
        assert_eq!(payload_from_output(output).unwrap()["v"], 1);
        assert!(payload_from_output("no json here\n{not json\n").is_none());
    }

    #[test]
    fn the_local_desk_comes_first_and_extra_desks_are_remote() {
        let desks = extra(Some(
            r#"[{"id":"lab","name":"Lab","origin":"http://127.0.0.1:9","token":"t"},{"id":"broken"}]"#,
        ));
        assert_eq!(desks.len(), 1);
        assert_eq!(desks[0]["kind"], "remote");
        assert_eq!(desks[0]["id"], "lab");
        assert!(extra(Some("not json")).is_empty());
        assert!(extra(None).is_empty());
        // An extra desk cannot pose as the local one.
        assert!(
            extra(Some(
                r#"[{"id":"local","name":"x","origin":"o","token":"t"}]"#
            ))
            .is_empty()
        );
    }
}
