//! The desks the window shows (BRO-145): this computer's, and every remote
//! desk it is paired with, each as an endpoint the window dials the same
//! way — the local Door, or the loopback bridge to a remote desk. The
//! window reads the list as `window.__hotlineDesks` (see `ui/src/desks.ts`).
//!
//! Remote desks come from the pairing registry once the client exists. Until
//! then, and for development, `HOTLINE_EXTRA_DESKS` adds desks by endpoint:
//! a JSON list of `{ "id", "name", "origin", "token" }`, for example a second
//! `hotline serve` on this machine, whose Door speaks the same wire contract
//! the bridge will.

use serde_json::{Value, json};

/// The local desk, then any extra ones, as the window reads them.
pub(crate) fn listed(origin: &str, token: &str) -> Value {
    let mut desks = vec![json!({
        "id": "local",
        "name": "This computer",
        "kind": "local",
        "origin": origin,
        "token": token,
    })];
    desks.extend(extra(std::env::var("HOTLINE_EXTRA_DESKS").ok().as_deref()));
    Value::Array(desks)
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

/// Claims the invitation and starts the desk's bridge, answering its id.
/// This is where the desk client (`hotline_core::remote::client`, M's half of
/// BRO-145) plugs in; until it lands, the window says so plainly.
async fn claim(_payload: Value) -> Result<String, String> {
    Err("This build can read the invitation but can't claim it yet: the desk client is still being built (BRO-145).".into())
}

#[tauri::command]
pub async fn desk_pair_link(link: String) -> Result<String, String> {
    claim(payload_from_link(&link)?).await
}

#[tauri::command]
pub async fn desk_pair_ssh(target: String) -> Result<String, String> {
    let (payload, child) = payload_over_ssh(target.trim()).await?;
    let claimed = claim(payload).await;
    // The server's pair command ends by itself once the invitation is
    // claimed or expires; dropping the child ends it if it is still waiting.
    drop(child);
    claimed
}

#[tauri::command]
pub async fn desk_forget(desk_id: String) -> Result<(), String> {
    Err(format!(
        "Forgetting a desk ({desk_id}) arrives with the desk client (BRO-145)."
    ))
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
