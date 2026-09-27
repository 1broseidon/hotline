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
//! On SIGTERM or SIGINT the desk stops for a restart (`Desk::stop_for_restart`):
//! new work is refused, the person's lines still waiting behind a turn are
//! kept for the next start, running turns get up to 30 seconds to finish, and
//! any still running after that are stopped and marked on their tape.

use crate::door;
use hotline_core::credentials::{Backend, FileStore, NativeStore, SecretStore, claim_backend};
use hotline_core::desk::Desk;
use hotline_core::remote::Remote;
use hotline_core::room_lock::RoomLock;
use hotline_core::wire::Door;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

const DRAIN: Duration = Duration::from_secs(30);

pub fn run(root: PathBuf, store: &str) -> ExitCode {
    let backend = match store {
        "file" => Backend::File,
        "native" => Backend::Native,
        other => {
            eprintln!("hotline: --store is file or native, not {other}");
            return ExitCode::from(2);
        }
    };
    match serve(root, backend) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("hotline: {error}");
            ExitCode::from(1)
        }
    }
}

fn serve(root: PathBuf, backend: Backend) -> Result<(), String> {
    let root = std::path::absolute(&root).map_err(|error| error.to_string())?;
    let _room = RoomLock::take(&root).map_err(|error| error.to_string())?;
    claim_backend(&root, backend).map_err(|error| error.to_string())?;
    let store: Arc<dyn SecretStore> = match backend {
        Backend::File => Arc::new(FileStore::open(&root).map_err(|error| error.to_string())?),
        Backend::Native => Arc::new(NativeStore),
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    runtime.block_on(async move {
        let desk = Arc::new(
            Desk::open_with_store(&root, store.clone())
                .map_err(|error| format!("the desk did not open: {error}"))?,
        );
        let remote = Remote::open_with_store(&root, desk.log.clone(), desk.clone(), store)
            .map_err(|error| format!("remote access settings did not open: {error}"))?;
        remote.restore().await;

        let token = token();
        let front = Door::bind(desk.log.clone(), token.clone(), desk.clone())
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
            },
        )
        .map_err(|error| format!("door.json could not be written: {error}"))?;
        eprintln!(
            "Hotline {} serving {} (store: {}). `hotline status` to check on it.",
            env!("CARGO_PKG_VERSION"),
            root.display(),
            backend.name()
        );

        stopped().await;
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
        door::remove(&root);
        drop(stopped);
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
