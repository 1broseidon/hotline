//! `hotline serve`: the desk, run in the foreground for systemd.
//!
//! The same boot the app makes, in the order a room needs: take the room's
//! lock before anything reads or writes it, check the room was made with this
//! secret store, open the desk (which starts its scheduler and settles what
//! the last run left), restore Remote for phones, then bind the loopback Door
//! and say where it is in `door.json`. There is no window, no tray, no
//! updater, and nothing that needs a display or a session bus.
//!
//! The environment is what the unit gives it. Unlike the app, `serve` does
//! not start a login shell to recover PATH: the unit sets `HOME`, `PATH` and
//! `HOTLINE_DATA_DIR` itself (see `packaging/hotline.service`).
//!
//! What a server runs is its capabilities (ADR 0006): `agents`, the desk
//! above, and `relay`, the relay desktops stand in on. A server that runs
//! only a relay takes the room's lock and claims the secret store, because
//! Remote's identities live there, then opens Remote and a Door that
//! administers it, and nothing else: no room, no scheduler, no agent, no
//! provider key. Its room is left on disk as it was, for a later
//! `--capabilities agents,relay` to open.
//!
//! On SIGTERM or SIGINT the desk stops for a restart (`Desk::stop_for_restart`):
//! new work is refused, the person's lines still waiting behind a turn are
//! kept for the next start, running turns get up to 30 seconds to finish, and
//! any still running after that are stopped and marked on their tape.

use crate::door;
use hotline_core::credentials::{
    Backend, FileStore, NativeStore, SecretStore, atomic_write, claim_backend,
};
use hotline_core::desk::Desk;
use hotline_core::remote::{Agents, Remote, ServeOptions};
use hotline_core::room_lock::RoomLock;
use hotline_core::wire::Door;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

const DRAIN: Duration = Duration::from_secs(30);

/// Where a served data directory keeps what it runs, so the mode belongs to
/// the data and not to the unit file. It is not in the room stream because
/// it must be known before the room is opened, or whether it is.
const SERVE_FILE: &str = "serve.json";

/// What a server runs. At least one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub agents: bool,
    pub relay: bool,
}

impl Capabilities {
    /// What every server ran before there was a choice.
    const BOTH: Self = Self {
        agents: true,
        relay: true,
    };

    /// `agents`, `relay`, or both, comma-separated, in either order.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut capabilities = Self {
            agents: false,
            relay: false,
        };
        for name in text.split(',').map(str::trim) {
            match name {
                "agents" => capabilities.agents = true,
                "relay" => capabilities.relay = true,
                _ => {
                    return Err(format!(
                        "--capabilities is agents, relay or agents,relay, not {text}"
                    ));
                }
            }
        }
        Ok(capabilities)
    }

    pub fn names(self) -> Vec<&'static str> {
        let mut names = Vec::new();
        if self.agents {
            names.push("agents");
        }
        if self.relay {
            names.push("relay");
        }
        names
    }

    fn describe(self) -> String {
        self.names().join(",")
    }
}

#[derive(Serialize, Deserialize)]
struct ServeFile {
    capabilities: Vec<String>,
}

/// What this boot runs: the flag when one is given, recorded if it differs
/// from the record; else the record; else both.
fn capabilities(root: &Path, flag: Option<Capabilities>) -> Result<Capabilities, String> {
    let path = root.join(SERVE_FILE);
    let recorded = match std::fs::read(&path) {
        Ok(bytes) => {
            let file: ServeFile = serde_json::from_slice(&bytes)
                .map_err(|error| format!("{} could not be read: {error}", path.display()))?;
            Some(
                Capabilities::parse(&file.capabilities.join(","))
                    .map_err(|error| format!("{}: {error}", path.display()))?,
            )
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("{} could not be read: {error}", path.display())),
    };
    let Some(flag) = flag else {
        return Ok(recorded.unwrap_or(Capabilities::BOTH));
    };
    if recorded != Some(flag) {
        let file = ServeFile {
            capabilities: flag.names().into_iter().map(str::to_owned).collect(),
        };
        let bytes = serde_json::to_vec(&file).map_err(|error| error.to_string())?;
        atomic_write(&path, &bytes)
            .map_err(|error| format!("{} could not be written: {error}", path.display()))?;
        // With no record the server ran both, so a change from that is said too.
        let before = recorded.unwrap_or(Capabilities::BOTH);
        if before != flag {
            eprintln!("capabilities: {} → {}", before.describe(), flag.describe());
        }
    }
    Ok(flag)
}

pub fn run(
    root: PathBuf,
    store: &str,
    capabilities: Option<Capabilities>,
    options: ServeOptions,
) -> ExitCode {
    let backend = match store {
        "file" => Backend::File,
        "native" => Backend::Native,
        other => {
            eprintln!("hotline: --store is file or native, not {other}");
            return ExitCode::from(2);
        }
    };
    match serve(root, backend, capabilities, options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("hotline: {error}");
            ExitCode::from(1)
        }
    }
}

fn serve(
    root: PathBuf,
    backend: Backend,
    flag: Option<Capabilities>,
    mut options: ServeOptions,
) -> Result<(), String> {
    let root = std::path::absolute(&root).map_err(|error| error.to_string())?;
    let _room = RoomLock::take(&root).map_err(|error| error.to_string())?;
    claim_backend(&root, backend).map_err(|error| error.to_string())?;
    let store: Arc<dyn SecretStore> = match backend {
        Backend::File => Arc::new(FileStore::open(&root).map_err(|error| error.to_string())?),
        Backend::Native => Arc::new(NativeStore),
    };
    let capabilities = capabilities(&root, flag)?;
    options.relay = capabilities.relay;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    runtime.block_on(async move {
        // The room is opened only where agents run.
        let desk = match capabilities.agents {
            true => Some(Arc::new(
                Desk::open_with_store(&root, store.clone())
                    .map_err(|error| format!("the desk did not open: {error}"))?,
            )),
            false => None,
        };
        let agents = desk.as_ref().map(|desk| Agents {
            log: desk.log.clone(),
            room: desk.clone(),
        });
        let remote = Remote::open_served_with_store(&root, agents, store, options)
            .map_err(|error| format!("remote access settings did not open: {error}"))?;
        if let Some(desk) = &desk {
            desk.set_remote(&remote);
        }
        remote.restore().await;

        let token = token();
        let front = match &desk {
            Some(desk) => Door::bind(desk.log.clone(), token.clone(), desk.clone()),
            None => Door::bind_relay(token.clone(), remote.clone()),
        }
        .map_err(|error| format!("the door did not bind: {error}"))?;
        let port = front.port();
        tokio::spawn(async move {
            if let Err(error) = front.run().await {
                eprintln!("[door] stopped: {error}");
            }
        });
        door::write(
            &root,
            &door::DoorFile {
                pid: std::process::id(),
                port,
                token,
                version: env!("CARGO_PKG_VERSION").to_string(),
                started_at: chrono::Utc::now().to_rfc3339(),
                data_dir: root.display().to_string(),
                store: backend.name().to_string(),
                capabilities: capabilities
                    .names()
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            },
        )
        .map_err(|error| format!("door.json could not be written: {error}"))?;
        eprintln!(
            "Hotline {} serving {} (runs: {}; store: {}). `hotline status` to check on it.",
            env!("CARGO_PKG_VERSION"),
            root.display(),
            capabilities.describe(),
            backend.name()
        );

        stopped().await;
        if let Some(desk) = desk {
            eprintln!(
                "Stopping: waiting up to {}s for running work to finish.",
                DRAIN.as_secs()
            );
            let stopped = desk.stop_for_restart(DRAIN).await;
            if stopped.kept > 0 {
                eprintln!(
                    "Kept {} waiting message(s) for the next start.",
                    stopped.kept
                );
            }
            if !stopped.interrupted.is_empty() {
                eprintln!(
                    "Stopped {} running turn(s); each teammate's conversation says so.",
                    stopped.interrupted.len()
                );
            }
            // The process ends without dropping the desk; every agent's process
            // group goes now rather than outliving it under pid 1.
            hotline_core::driver::acp::end_every_agent();
            drop(stopped);
        }
        door::remove(&root);
        eprintln!("Stopped.");
        Ok(())
    })
}

async fn stopped() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("SIGTERM can be watched");
    let mut interrupt = signal(SignalKind::interrupt()).expect("SIGINT can be watched");
    tokio::select! {
        _ = term.recv() => {}
        _ = interrupt.recv() => {}
    }
}

/// The Door's token for this process alone: 32 random bytes as hex.
fn token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the OS random source is available");
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_are_agents_relay_or_both_and_never_none() {
        assert_eq!(Capabilities::parse("agents,relay"), Ok(Capabilities::BOTH));
        assert_eq!(Capabilities::parse("relay,agents"), Ok(Capabilities::BOTH));
        assert_eq!(Capabilities::parse("relay").unwrap().names(), ["relay"]);
        assert_eq!(Capabilities::parse("agents").unwrap().names(), ["agents"]);
        for refused in ["", "relay,", "computer", "agents, voice"] {
            assert!(Capabilities::parse(refused).is_err(), "{refused}");
        }
    }

    #[test]
    fn the_flag_is_recorded_and_the_record_is_used_without_it() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(capabilities(root.path(), None), Ok(Capabilities::BOTH));
        assert!(!root.path().join(SERVE_FILE).exists());
        let relay = Capabilities::parse("relay").unwrap();
        assert_eq!(capabilities(root.path(), Some(relay)), Ok(relay));
        assert_eq!(
            std::fs::read_to_string(root.path().join(SERVE_FILE)).unwrap(),
            r#"{"capabilities":["relay"]}"#
        );
        assert_eq!(capabilities(root.path(), None), Ok(relay));
        assert_eq!(
            capabilities(root.path(), Some(Capabilities::BOTH)),
            Ok(Capabilities::BOTH)
        );
        assert_eq!(capabilities(root.path(), None), Ok(Capabilities::BOTH));
    }
}
