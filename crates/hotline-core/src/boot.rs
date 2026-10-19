//! The desk's boot, once, for the app and for `hotline serve`.
//!
//! The order is what a room needs: the room's lock is already held, so the
//! store is claimed (a room made with one secret store is refused under the
//! other rather than opened empty), the desk opens (which starts its
//! scheduler and settles what the last run left), Remote opens and restores
//! for phones, and the loopback Door binds behind a fresh token and starts
//! serving. Each step that fails is an error for the shell to report and exit
//! on; nothing here starts an empty room or rewrites a file it could not read.
//!
//! What differs between the shells is in the arguments: the store, and for a
//! server its network and whether it runs agents at all. A server that runs
//! only a relay opens Remote and a Door that administers it, and no room.

#[cfg(unix)]
use crate::credentials::FileStore;
use crate::credentials::{Backend, NativeStore, SecretStore, claim_backend};
use crate::desk::Desk;
use crate::remote::{Remote, ServeOptions};
use crate::room_lock::RoomLock;
use crate::wire::{Agents, Door, Parts};
use std::path::Path;
use std::sync::Arc;

/// How `hotline serve` differs from the app's desk.
pub struct Served {
    /// Whether the room opens; without it the server runs only a relay.
    pub agents: bool,
    pub options: ServeOptions,
}

/// What a boot stood up. The Door is already serving on `port`.
pub struct Booted {
    /// None only for a server that runs only a relay.
    pub desk: Option<Arc<Desk>>,
    pub remote: Arc<Remote>,
    /// The Door's token, for this process alone.
    pub token: String,
    pub port: u16,
}

/// Boots the room at `root`, whose lock the caller holds for as long as the
/// desk runs. Called inside a Tokio runtime, because the room spawns its
/// background work as it opens.
pub async fn boot(
    root: &Path,
    _held: &RoomLock,
    backend: Backend,
    served: Option<Served>,
) -> Result<Booted, String> {
    claim_backend(root, backend).map_err(|error| error.to_string())?;
    let store: Arc<dyn SecretStore> = match backend {
        #[cfg(unix)]
        Backend::File => Arc::new(
            FileStore::open(root)
                .map_err(|error| format!("the file store did not open: {error}"))?,
        ),
        // The file store is the headless desk's, and that is Unix only.
        #[cfg(not(unix))]
        Backend::File => return Err("the file store is only on Linux and macOS".into()),
        Backend::Native => Arc::new(NativeStore),
    };
    let agents = served.as_ref().is_none_or(|served| served.agents);
    let desk = if agents {
        Some(Arc::new(
            Desk::open_with_store(root, store.clone())
                .map_err(|error| format!("the desk did not open: {error}"))?,
        ))
    } else {
        None
    };
    let room = desk.as_ref().map(|desk| Agents {
        log: desk.log.clone(),
        room: desk.clone(),
        parts: desk.parts(),
    });
    let remote = match (served, room) {
        (Some(served), room) => Remote::open_served_with_store(root, room, store, served.options),
        (None, Some(room)) => Remote::open_with_store(root, room, store),
        (None, None) => unreachable!("a desk that is not served always opens its room"),
    }
    .map_err(|error| format!("remote access settings did not open: {error}"))?;
    remote.restore().await;

    let token = token().map_err(|error| format!("no token could be made: {error}"))?;
    let door = match &desk {
        Some(desk) => Door::bind(
            desk.log.clone(),
            token.clone(),
            desk.clone(),
            Parts {
                remote: Arc::downgrade(&remote),
                ..desk.parts()
            },
        ),
        None => Door::bind_relay(token.clone(), remote.clone()),
    }
    .map_err(|error| format!("the door did not bind: {error}"))?;
    let port = door.port();
    tokio::spawn(async move {
        if let Err(error) = door.run().await {
            eprintln!("[door] stopped: {error}");
        }
    });
    Ok(Booted {
        desk,
        remote,
        token,
        port,
    })
}

/// 32 bytes from the OS random source, as hex. When the source fails the
/// boot fails: a weaker token would let another local process into the room.
fn token() -> Result<String, getrandom::Error> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)?;
    Ok(hex::encode(bytes))
}

// On the file store, which keeps a test off the machine's own keychain.
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A boot over a `remote.json` it cannot read fails, binds nothing, and
    /// leaves the file as it was for the person to look at.
    #[tokio::test(flavor = "multi_thread")]
    async fn unreadable_remote_settings_stop_the_boot_and_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let held = RoomLock::take(root).unwrap();
        let broken = br#"{"enabled": tr"#;
        std::fs::write(root.join("remote.json"), broken).unwrap();
        let Err(error) = boot(root, &held, Backend::File, None).await else {
            panic!("the boot went on over unreadable settings")
        };
        assert!(
            error.starts_with("remote access settings did not open"),
            "{error}"
        );
        assert_eq!(std::fs::read(root.join("remote.json")).unwrap(), broken);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_desk_boots_with_its_door_serving_behind_a_full_token() {
        let dir = tempfile::tempdir().unwrap();
        let held = RoomLock::take(dir.path()).unwrap();
        let booted = boot(dir.path(), &held, Backend::File, None).await.unwrap();
        assert!(booted.desk.is_some());
        assert_eq!(booted.token.len(), 64);
        assert!(booted.token.bytes().all(|byte| byte.is_ascii_hexdigit()));
        std::net::TcpStream::connect(("127.0.0.1", booted.port)).unwrap();
    }
}
