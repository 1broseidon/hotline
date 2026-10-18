//! The real `hotline` binary, on a temporary room, with no display and no
//! session bus: it serves, keeps its room to itself, keeps a key in the file
//! store across a restart, and stops cleanly on SIGTERM. Served as a relay
//! only, it opens no room and still carries a desktop's visitors; served
//! again in another mode, every pairing keeps its role. A room it cannot
//! read is refused with an unsuccessful exit, and left as it was.
#![cfg(unix)]

use hotline_core::credentials::{FileStore, SecretStore};
use hotline_core::desk::Desk;
use hotline_core::remote::client::{Client, PairedDesk, PairedRelay};
use hotline_core::remote::{PairingPayload, Remote};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::net::SocketAddr;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

const HOTLINE: &str = env!("CARGO_BIN_EXE_hotline");

fn hotline(root: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(HOTLINE);
    command
        .args(args)
        .arg("--data")
        .arg(root)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .env_remove("HOTLINE_DATA_DIR");
    if args.first() == Some(&"serve") && !args.contains(&"--listen") {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        command.args([
            "--listen",
            &addr.to_string(),
            "--public-url",
            &format!("https://{addr}"),
        ]);
    }
    command
}

fn serve(root: &Path) -> Child {
    serve_with(root, &[])
}

/// Serves with more flags: `--capabilities`, or a `--listen` to keep.
fn serve_with(root: &Path, extra: &[&str]) -> Child {
    let mut args = vec!["serve", "--store", "file"];
    args.extend(extra);
    let child = hotline(root, &args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let door = root.join("door.json");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !door.exists() {
        assert!(Instant::now() < deadline, "the desk never wrote door.json");
        std::thread::sleep(Duration::from_millis(100));
    }
    child
}

fn wire(root: &Path, cmd: &str, params: &str) -> Output {
    let mut child = hotline(root, &["wire", cmd])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(params.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn stop(mut child: Child) -> Output {
    // SAFETY: a signal to our own child.
    unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "the desk did not stop on SIGTERM"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn a_served_desk_keeps_its_room_its_keys_and_stops_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("room");

    let desk = serve(&root);
    let status = hotline(&root, &["status"]).output().unwrap();
    assert!(status.status.success(), "{status:?}");
    let said = String::from_utf8_lossy(&status.stdout);
    assert!(
        said.contains("running") && said.contains("store   file"),
        "{said}"
    );

    // A second desk on the same room is refused while the first runs.
    let second = hotline(&root, &["serve", "--store", "file"])
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&second.stderr).contains("Another Hotline desk"));

    let saved = wire(
        &root,
        "credential.create",
        r#"{"providerId":"openrouter","label":"harness","secret":"sk-or-harness-0123456789"}"#,
    );
    assert!(saved.status.success(), "{saved:?}");

    let stopped = stop(desk);
    assert!(stopped.status.success(), "{stopped:?}");
    assert!(!root.join("door.json").exists());
    let down = hotline(&root, &["status"]).output().unwrap();
    assert_eq!(down.status.code(), Some(3));

    // The key is only in the file store, and survives the restart.
    for entry in walk(&root) {
        if entry.starts_with(root.join("secrets")) {
            continue;
        }
        let bytes = std::fs::read(&entry).unwrap_or_default();
        assert!(
            !String::from_utf8_lossy(&bytes).contains("sk-or-harness"),
            "{} holds the key in the clear",
            entry.display()
        );
    }
    let desk = serve(&root);
    let listed = wire(&root, "credential.list", "{}");
    assert!(String::from_utf8_lossy(&listed.stdout).contains("harness"));
    let status = hotline(&root, &["status"]).output().unwrap();
    assert!(String::from_utf8_lossy(&status.stdout).contains("OpenRouter"));
    assert!(stop(desk).status.success());

    // Started on the other store, the room is refused rather than emptied.
    let native = hotline(&root, &["serve", "--store", "native"])
        .output()
        .unwrap();
    assert_eq!(native.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&native.stderr).contains("file store"));
}

/// A boot that cannot read the room's Remote settings ends unsuccessfully,
/// says which step failed, advertises no Door, and keeps the file it could
/// not read rather than starting over without the person's pairings.
#[test]
fn a_room_whose_remote_settings_cannot_be_read_is_not_served() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("room");
    std::fs::create_dir_all(&root).unwrap();
    let broken = br#"{"enabled": tr"#;
    std::fs::write(root.join("remote.json"), broken).unwrap();

    let refused = hotline(&root, &["serve", "--store", "file"])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(1), "{refused:?}");
    let said = String::from_utf8_lossy(&refused.stderr);
    assert!(
        said.contains("remote access settings did not open"),
        "{said}"
    );
    assert!(!said.contains("panicked"), "{said}");
    assert!(!root.join("door.json").exists());
    assert_eq!(std::fs::read(root.join("remote.json")).unwrap(), broken);
}

fn walk(root: &Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found
}

/// A fixed address for a server served more than once, so a device paired
/// on one boot dials the same place on the next.
fn listen_args(addr: &SocketAddr) -> [String; 4] {
    [
        "--listen".into(),
        addr.to_string(),
        "--public-url".into(),
        format!("https://{addr}"),
    ]
}

fn free_address() -> SocketAddr {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

/// `hotline pair --json` with `flags`, still waiting for its claim, and the
/// invitation it printed.
fn invite(root: &Path, flags: &[&str]) -> (Child, PairingPayload) {
    let mut args = vec!["pair", "--json"];
    args.extend(flags);
    let mut child = hotline(root, &args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.as_mut().unwrap())
        .read_line(&mut line)
        .unwrap();
    match serde_json::from_str(&line) {
        Ok(payload) => (child, payload),
        Err(_) => panic!("no invitation: {line} {:?}", child.wait_with_output()),
    }
}

/// A device, with a store of its own, that claims an invitation.
async fn claim(dir: &Path, payload: &PairingPayload, name: &str) -> (Client, PairedDesk) {
    std::fs::create_dir_all(dir).unwrap();
    let client = Client::new(Arc::new(FileStore::open(dir).unwrap()));
    let paired = client.pair(payload, name).await.unwrap();
    (client, paired)
}

/// A desktop, with a store of its own, that claims a relay's invitation and
/// keeps the server where the shell does: beside its desks, not among them.
async fn claim_relay(dir: &Path, payload: &PairingPayload, name: &str) -> PairedRelay {
    std::fs::create_dir_all(dir).unwrap();
    let client = Client::new(Arc::new(FileStore::open(dir).unwrap()));
    let paired = client.pair_relay(payload, name).await.unwrap();
    std::fs::write(
        dir.join("relays.json"),
        serde_json::to_vec(&vec![paired.clone()]).unwrap(),
    )
    .unwrap();
    paired
}

/// The pair command, once its window is claimed, says who paired as what.
fn paired_as(child: Child) -> String {
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The first frame an owner's session hears: the hello.
async fn hello(client: &Client, desk: &PairedDesk) -> Value {
    let mut session = client.connect(desk);
    let hello = tokio::time::timeout(Duration::from_secs(20), session.incoming.recv())
        .await
        .expect("the desk never said hello")
        .unwrap();
    serde_json::from_str(&hello).unwrap()
}

fn said(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// A person's desktop, in this process: a room, and Remote on.
async fn desktop(root: &Path) -> (Arc<Desk>, Arc<Remote>, Arc<dyn SecretStore>) {
    std::fs::create_dir_all(root).unwrap();
    let store: Arc<dyn SecretStore> = Arc::new(FileStore::open(root).unwrap());
    let desk = Arc::new(Desk::open_with_store(root, store.clone()).unwrap());
    // On loopback, on a socket bound here: no network needed, and no other
    // test can take its port first.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let remote = Remote::open_on(
        listener,
        root,
        desk.log.clone(),
        desk.clone(),
        store.clone(),
    )
    .unwrap();
    desk.set_remote(&remote);
    remote.configure(true, "all").await.unwrap();
    (desk, remote, store)
}

/// The entries at the top of a data directory, sorted.
fn top(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[tokio::test(flavor = "multi_thread")]
async fn a_relay_only_server_opens_no_room_and_carries_a_desktops_visitors() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("pi");
    let server = serve_with(&root, &["--capabilities", "relay"]);

    // Remote's own files, the store, the lock and the door: no room.
    for name in top(&root) {
        assert!(
            name.starts_with("remote")
                || matches!(
                    name.as_str(),
                    "desk.lock" | "door.json" | "serve.json" | "secrets" | "store.json"
                ),
            "a relay-only boot made {name}: {:?}",
            top(&root)
        );
    }
    assert_eq!(
        std::fs::read_to_string(root.join("serve.json")).unwrap(),
        r#"{"capabilities":["relay"]}"#
    );
    let status = hotline(&root, &["status"]).output().unwrap();
    assert!(status.status.success(), "{status:?}");
    assert!(said(&status).contains("runs    relay"), "{}", said(&status));
    assert!(!said(&status).contains("teammates"), "{}", said(&status));

    // The door administers the relay, and refuses what needs a room.
    for refused in [
        hotline(&root, &["links"]).output().unwrap(),
        hotline(&root, &["unlink", "0123"]).output().unwrap(),
        wire(&root, "welcome", "{}"),
        hotline(&root, &["pair", "--companion"]).output().unwrap(),
    ] {
        assert_eq!(refused.status.code(), Some(1), "{refused:?}");
        assert!(said(&refused).contains("runs only a relay"), "{refused:?}");
    }

    // `hotline pair` makes the only grant there is: relay. A desktop claims
    // it and stands in.
    let (pairing, invitation) = invite(&root, &[]);
    assert_eq!(invitation.role, hotline_core::contract::DeviceRole::Relay);
    let (_desk, laptop, _) = desktop(&dir.path().join("laptop")).await;
    let laptop_store = dir.path().join("laptop");
    let pi = claim_relay(&laptop_store, &invitation, "Laptop").await;
    assert!(paired_as(pairing).contains("as relay"));
    laptop.relay_through(Some(pi.desk_id.clone())).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let url = loop {
        if let Some(url) = laptop.status().relay.and_then(|relay| relay.url) {
            break url;
        }
        assert!(
            Instant::now() < deadline,
            "never stood in: {:?}",
            laptop.status().relay.and_then(|relay| relay.error)
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(url.contains("/relay/"), "{url}");

    // A phone that cannot reach the laptop's own address visits it through
    // the relay, and the laptop answers.
    let invitation = laptop
        .pairing_v2(hotline_core::contract::DeviceRole::Owner)
        .unwrap()
        .payload;
    assert_eq!(invitation.relay.as_deref(), Some(url.as_str()));
    let (phone, mut laptop_desk) =
        claim(&dir.path().join("phone"), &invitation, "Away phone").await;
    laptop_desk.url = "https://127.0.0.1:9".into();
    laptop_desk.public = None;
    let hello = hello(&phone, &laptop_desk).await;
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["deskKind"], "desktop");

    let devices = said(&hotline(&root, &["devices"]).output().unwrap());
    assert!(devices.contains("relay  Laptop"), "{devices}");
    let laptop_id = devices.split_whitespace().next().unwrap();
    let revoked = hotline(&root, &["revoke", laptop_id]).output().unwrap();
    assert!(revoked.status.success(), "{revoked:?}");
    assert!(said(&hotline(&root, &["devices"]).output().unwrap()).contains("No paired devices"));
    laptop.configure(false, "all").await.unwrap();
    let stopped = stop(server);
    assert!(stopped.status.success(), "{stopped:?}");
    assert!(
        !said(&stopped).contains("running work"),
        "{}",
        said(&stopped)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn serving_in_another_mode_keeps_every_pairing_and_its_role() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("server");
    let addr = free_address();
    let listen = listen_args(&addr);
    let listen: Vec<&str> = listen.iter().map(String::as_str).collect();

    // Agents and a relay, as every server before the choice.
    let server = serve_with(&root, &listen);
    assert!(!root.join("serve.json").exists());
    let (pairing, invitation) = invite(&root, &[]);
    let (phone, desk) = claim(&dir.path().join("phone"), &invitation, "Phone").await;
    assert!(paired_as(pairing).contains("as owner"));
    let laptop_root = dir.path().join("laptop");
    let (_laptop_desk, laptop, _) = desktop(&laptop_root).await;
    let (pairing, invitation) = invite(&root, &["--relay"]);
    let server_desk = claim_relay(&laptop_root, &invitation, "Laptop").await;
    assert!(paired_as(pairing).contains("as relay"));
    let said_hello = hello(&phone, &desk).await;
    assert_eq!(said_hello["deskKind"], "server");
    assert_eq!(said_hello["serves"], json!(["agents", "relay"]));
    let devices = said(&hotline(&root, &["devices"]).output().unwrap());
    assert!(stop(server).status.success());
    let room: Vec<String> = top(&root)
        .into_iter()
        .filter(|name| name != "door.json")
        .collect();

    // Relay only: the room stays on disk, unopened; every grant is kept.
    let mut flags = listen.clone();
    flags.extend(["--capabilities", "relay"]);
    let server = serve_with(&root, &flags);
    assert_eq!(
        said(&hotline(&root, &["devices"]).output().unwrap()),
        devices
    );
    assert!(devices.contains("owner  Phone") && devices.contains("relay  Laptop"));
    let mut now: Vec<String> = top(&root);
    now.retain(|name| name != "door.json" && name != "serve.json");
    assert_eq!(now, room);
    // The relay grant made on the last boot stands in on this one.
    laptop
        .relay_through(Some(server_desk.desk_id.clone()))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while laptop.status().relay.and_then(|relay| relay.url).is_none() {
        assert!(
            Instant::now() < deadline,
            "never stood in: {:?}",
            laptop.status().relay.and_then(|relay| relay.error)
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    laptop.relay_through(None).unwrap();
    let stopped = stop(server);
    assert!(
        said(&stopped).contains("capabilities: agents,relay → relay"),
        "{}",
        said(&stopped)
    );

    // Agents only: the owner is back on the wire, told the relay is gone.
    let mut flags = listen.clone();
    flags.extend(["--capabilities", "agents"]);
    let server = serve_with(&root, &flags);
    assert_eq!(
        said(&hotline(&root, &["devices"]).output().unwrap()),
        devices
    );
    let said_hello = hello(&phone, &desk).await;
    assert_eq!(said_hello["serves"], json!(["agents"]));
    let refused = hotline(&root, &["pair", "--relay"]).output().unwrap();
    assert!(
        said(&refused).contains("does not run a relay"),
        "{refused:?}"
    );
    let stopped = stop(server);
    assert!(
        said(&stopped).contains("capabilities: relay → agents"),
        "{}",
        said(&stopped)
    );
}
