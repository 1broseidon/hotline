//! Epic-11 end to end (ADR 0006), with real processes: a server served as a
//! relay only opens no room, a desktop pairs with it as a relay and stands
//! in, a phone reaches the desktop through it, and the desktop's key there
//! opens nothing else. Served again with agents, the same pairing still
//! stands in, and is refused at every door of the desk with
//! `role_not_permitted`, never as a revoked key.
#![cfg(unix)]

use futures_util::{SinkExt, StreamExt};
use hotline_core::contract::DeviceRole;
use hotline_core::credentials::{FileStore, SecretStore};
use hotline_core::desk::Desk;
use hotline_core::remote::client::{Client, PairedDesk, PairedRelay};
use hotline_core::remote::{PairingPayload, Remote};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::net::SocketAddr;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_rustls::rustls;
use tokio_tungstenite::tungstenite::Message;

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
    command
}

/// The server, on a fixed address so the desktop and the phone dial the
/// same place after a restart.
fn serve(root: &Path, addr: SocketAddr, capabilities: &str) -> Child {
    let child = hotline(
        root,
        &[
            "serve",
            "--store",
            "file",
            "--capabilities",
            capabilities,
            "--listen",
            &addr.to_string(),
            "--public-url",
            &format!("https://{addr}"),
        ],
    )
    .stdout(Stdio::null())
    .stderr(Stdio::piped())
    .spawn()
    .unwrap();
    let door = root.join("door.json");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !door.exists() {
        assert!(
            Instant::now() < deadline,
            "the server never wrote door.json"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    child
}

/// Stops the server by the pid captured when it was spawned, and only that.
fn stop(mut child: Child) -> Output {
    // SAFETY: a signal to our own child.
    unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
    let deadline = Instant::now() + Duration::from_secs(40);
    while child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "the server ignored SIGTERM");
        std::thread::sleep(Duration::from_millis(100));
    }
    child.wait_with_output().unwrap()
}

fn said(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn wire(root: &Path, cmd: &str) -> Output {
    let mut child = hotline(root, &["wire", cmd])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"{}").unwrap();
    child.wait_with_output().unwrap()
}

/// `hotline pair` with `flags`, still waiting for its claim, and the first
/// line it printed.
fn invite(root: &Path, flags: &[&str]) -> (Child, String) {
    let mut args = vec!["pair"];
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
    (child, line.trim().to_owned())
}

fn free_address() -> SocketAddr {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

fn top(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

/// The processes the server started: none, if it runs no agents.
fn children_of(pid: u32) -> String {
    let output = Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn uuid() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).unwrap();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// A person's desktop, in this process: a room, and Remote on.
async fn desktop(root: &Path) -> (Arc<Desk>, Arc<Remote>) {
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
    (desk, remote)
}

/// The desktop's own secret store, which also keeps the device key it
/// makes when it pairs, so this test can knock with that key on doors the
/// desktop itself never dials.
struct Keeping {
    store: FileStore,
    identity: Mutex<Option<Vec<u8>>>,
}
impl SecretStore for Keeping {
    fn get(&self, key: &str) -> std::io::Result<Option<Vec<u8>>> {
        self.store.get(key)
    }
    fn set(&self, key: &str, bytes: &[u8]) -> std::io::Result<()> {
        if key.starts_with("remote-client-") {
            *self.identity.lock().unwrap() = Some(bytes.to_vec());
        }
        self.store.set(key, bytes)
    }
    fn delete(&self, key: &str) -> std::io::Result<()> {
        self.store.delete(key)
    }
}

/// Waits until the desktop stands in on its chosen relay, and says where.
async fn standing(remote: &Remote) -> String {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(url) = remote.status().relay.and_then(|relay| relay.url) {
            return url;
        }
        assert!(
            Instant::now() < deadline,
            "never stood in: {:?}",
            remote.status().relay.and_then(|relay| relay.error)
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The first frame a session hears, if it is a hello within the timeout.
async fn hello(client: &Client, desk: &PairedDesk, within: Duration) -> Option<Value> {
    let mut session = client.connect(desk);
    let frame = tokio::time::timeout(within, session.incoming.recv())
        .await
        .ok()??;
    serde_json::from_str(&frame).ok()
}

/// What a hostile holder of the desktop's relay key gets at `path`: the
/// server's sealed Noise message 2, opened. This is the handshake the core's
/// client makes, written out so it can try doors the client never opens.
async fn knock(addr: SocketAddr, path: &str, identity: &[u8], payload: &str) -> String {
    let identity: Value = serde_json::from_slice(identity).unwrap();
    let key = |name: &str| -> Vec<u8> {
        serde_json::from_value::<Vec<u8>>(identity[name].clone()).unwrap()
    };
    let mut noise = snow::Builder::new("Noise_IK_25519_ChaChaPoly_SHA256".parse().unwrap())
        .prologue(b"hotline/2")
        .unwrap()
        .local_private_key(&key("private"))
        .unwrap()
        .remote_public_key(&key("desk"))
        .unwrap()
        .build_initiator()
        .unwrap();

    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyCertificate(provider)))
        .with_no_client_auth();
    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let tls = tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect("127.0.0.1".try_into().unwrap(), tcp)
        .await
        .unwrap();
    let (mut socket, _) = tokio_tungstenite::client_async(format!("wss://{addr}{path}"), tls)
        .await
        .unwrap();

    let mut buffer = [0u8; 4096];
    let size = noise
        .write_message(payload.as_bytes(), &mut buffer)
        .unwrap();
    socket
        .send(Message::Binary(buffer[..size].to_vec().into()))
        .await
        .unwrap();
    let Some(Ok(Message::Binary(reply))) = socket.next().await else {
        panic!("{path} sent no sealed answer");
    };
    let size = noise.read_message(&reply, &mut buffer).unwrap();
    String::from_utf8(buffer[..size].to_vec()).unwrap()
}

/// TLS here carries no trust: the pinned Noise key inside it does, exactly
/// as in the core's own dialer. Only this test's self-signed server is met.
#[derive(Debug)]
struct AnyCertificate(Arc<rustls::crypto::CryptoProvider>);
impl rustls::client::danger::ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        signature: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            signature,
            &self.0.signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        signature: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            signature,
            &self.0.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_relay_only_server_relays_for_a_desktop_that_can_do_nothing_else_there() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("pi");
    let addr = free_address();
    let http = reqwest::Client::builder()
        // Only this test's freshly generated certificate is untrusted.
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let route = |path: &'static str| {
        let http = http.clone();
        async move {
            http.get(format!("https://{addr}{path}"))
                .send()
                .await
                .unwrap()
                .status()
                .as_u16()
        }
    };

    // 1. Served as a relay only: Remote's files, the store, the lock and the
    // door, and no room; no child process; `status` says what it runs.
    let server = serve(&root, addr, "relay");
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
    assert_eq!(children_of(server.id()), "", "a relay-only boot spawned");
    let status = hotline(&root, &["status"]).output().unwrap();
    assert!(status.status.success(), "{status:?}");
    assert!(said(&status).contains("runs    relay"), "{}", said(&status));

    // 2. The desktop pairs from the link `hotline pair` prints there, keeps
    // the server where the shell does, in relays.json, and chooses it.
    let (pairing, link) = invite(&root, &["--link"]);
    let invitation = PairingPayload::from_link(&link).unwrap();
    assert_eq!(invitation.role, DeviceRole::Relay);
    let laptop_root = dir.path().join("laptop");
    let (_laptop_desk, laptop) = desktop(&laptop_root).await;
    let keeping = Arc::new(Keeping {
        store: FileStore::open(&laptop_root).unwrap(),
        identity: Mutex::new(None),
    });
    let pi: PairedRelay = Client::new(keeping.clone())
        .pair_relay(&invitation, "Laptop")
        .await
        .unwrap();
    assert!(said(&pairing.wait_with_output().unwrap()).contains("as relay"));
    let relay_key = keeping.identity.lock().unwrap().clone().unwrap();
    std::fs::write(
        laptop_root.join("relays.json"),
        serde_json::to_vec(&vec![pi.clone()]).unwrap(),
    )
    .unwrap();
    laptop.relay_through(Some(pi.desk_id.clone())).unwrap();
    let relayed = standing(&laptop).await;
    assert!(relayed.contains("/relay/"), "{relayed}");

    // 3. A phone paired with the desktop, which cannot reach the desktop's
    // own address, gets its hello through the relay.
    let invitation = laptop.pairing_v2(DeviceRole::Owner).unwrap().payload;
    assert_eq!(invitation.relay.as_deref(), Some(relayed.as_str()));
    let phone_root = dir.path().join("phone");
    std::fs::create_dir_all(&phone_root).unwrap();
    let phone = Client::new(Arc::new(FileStore::open(&phone_root).unwrap()));
    let mut away = phone.pair(&invitation, "Away phone").await.unwrap();
    away.url = "https://127.0.0.1:9".into();
    away.public = None;
    let said_hello = hello(&phone, &away, Duration::from_secs(20))
        .await
        .expect("the desktop did not answer through the relay");
    assert_eq!(said_hello["type"], "hello");
    assert_eq!(said_hello["deskKind"], "desktop");

    // 4. There is no desk here for the relay key to reach: the wire, the
    // computer and the link are not routes at all, before any Noise. The
    // door administers the relay and refuses what needs a room.
    for path in ["/v2", "/v2/computer/scout/ws", "/v2/link"] {
        assert_eq!(route(path).await, 404, "{path}");
    }
    assert_eq!(route("/v2/relay").await, 400, "the relay is a route");
    for refused in [
        hotline(&root, &["links"]).output().unwrap(),
        hotline(&root, &["unlink", "0123"]).output().unwrap(),
        wire(&root, "welcome"),
    ] {
        assert_eq!(refused.status.code(), Some(1), "{refused:?}");
        assert!(said(&refused).contains("runs only a relay"), "{refused:?}");
    }

    // 5. A phone that claims the relay's code as phones before ADR 0006 do,
    // with no purpose, is refused as a relay and makes no grant.
    let (pairing, line) = invite(&root, &["--json"]);
    let mut old_phone_reading: PairingPayload = serde_json::from_str(&line).unwrap();
    assert_eq!(old_phone_reading.role, DeviceRole::Relay);
    old_phone_reading.role = DeviceRole::Owner;
    let stray_root = dir.path().join("stray");
    std::fs::create_dir_all(&stray_root).unwrap();
    let stray = Client::new(Arc::new(FileStore::open(&stray_root).unwrap()));
    assert_eq!(
        stray
            .pair(&old_phone_reading, "Old phone")
            .await
            .unwrap_err(),
        "This invitation is for using the server as a relay."
    );
    // The list is stdout alone: onnxruntime, linked for the speech engine,
    // warns on stderr on a CPU it does not know, such as Linux on Apple
    // silicon.
    let devices = hotline(&root, &["devices"]).output().unwrap();
    let devices = String::from_utf8_lossy(&devices.stdout);
    assert_eq!(devices.trim().lines().count(), 1, "{devices}");
    assert!(devices.contains("relay  Laptop"), "{devices}");
    stop(pairing);

    // 6. Served again with agents: the same relay grant still stands in, and
    // the phone, still away, reaches the desktop through it as before.
    let stopped = stop(server);
    assert!(stopped.status.success(), "{stopped:?}");
    let server = serve(&root, addr, "agents,relay");
    let status = said(&hotline(&root, &["status"]).output().unwrap());
    assert!(status.contains("runs    agents, relay"), "{status}");
    let devices_again = hotline(&root, &["devices"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&devices_again.stdout), devices);
    let deadline = Instant::now() + Duration::from_secs(60);
    let said_hello = loop {
        if let Some(said) = hello(&phone, &away, Duration::from_secs(5)).await {
            break said;
        }
        assert!(
            Instant::now() < deadline,
            "the desktop never stood in again: {:?}",
            laptop.status().relay.and_then(|relay| relay.error)
        );
    };
    assert_eq!(said_hello["deskKind"], "desktop");
    assert_eq!(standing(&laptop).await, relayed);

    // Now there is a desk, and the relay key is refused at each of its doors
    // for its role: not revoked, so the desktop keeps a working pairing. It
    // cannot prompt or read a conversation (the wire), use a teammate's
    // computer, or link as a desk.
    for (path, payload) in [
        ("/v2", String::new()),
        (
            "/v2/computer/scout/ws",
            json!({"purpose": "computer", "personaId": "scout"}).to_string(),
        ),
        (
            "/v2/link",
            json!({"purpose": "link", "deskId": uuid()}).to_string(),
        ),
    ] {
        assert_eq!(
            knock(addr, path, &relay_key, &payload).await,
            r#"{"error":"role_not_permitted"}"#,
            "{path}"
        );
    }

    // An owner's hello says what the server now runs.
    let (pairing, line) = invite(&root, &["--json"]);
    let invitation: PairingPayload = serde_json::from_str(&line).unwrap();
    assert_eq!(invitation.role, DeviceRole::Owner);
    let owner_root = dir.path().join("owner");
    std::fs::create_dir_all(&owner_root).unwrap();
    let owner = Client::new(Arc::new(FileStore::open(&owner_root).unwrap()));
    let server_desk = owner.pair(&invitation, "Owner phone").await.unwrap();
    assert!(said(&pairing.wait_with_output().unwrap()).contains("as owner"));
    let said_hello = hello(&owner, &server_desk, Duration::from_secs(20))
        .await
        .expect("the server did not say hello to its owner");
    assert_eq!(said_hello["deskKind"], "server");
    assert_eq!(said_hello["serves"], json!(["agents", "relay"]));

    laptop.configure(false, "all").await.unwrap();
    let stopped = stop(server);
    assert!(stopped.status.success(), "{stopped:?}");
    assert!(
        said(&stopped).contains("capabilities: relay → agents,relay"),
        "{}",
        said(&stopped)
    );
}
