//! The desk restarted beside a real v2 computer, as two processes.
//!
//! The first run is a desk on a data root of its own and the OS store: it
//! makes a teammate's computer on Apple `container` from the 0.13.0-rc.3
//! probe image, takes its boot's attest key as it starts it, and hands it a
//! key Used with a fixture API; a job's request through the key proxy
//! reaches the API with the real key. That process ends and leaves the
//! computer running. The second run is a new desk process over the same
//! data root and store: it finds the computer with the bearer in its
//! settings, reads the key back from the store, is not refused at the key
//! door, and the same request runs again with nothing waiting on the desk.
//! The clean run removes the container and every OS store entry these runs
//! wrote, by the names they recorded, and nothing else.
//!
//! ```sh
//! root=~/Projects/scratch/attest-key/run-1 && mkdir -p "$root"
//! bin=<a copy of the hotline-core lib test binary>   # one binary for every phase
//! for phase in first second clean; do
//!   HOTLINE_E2E_ROOT="$root" HOTLINE_E2E_PHASE=$phase "$bin" \
//!     restart::a_desk_restart --ignored --nocapture --test-threads=1
//! done
//! ```
//!
//! One binary for every phase, because the keychain trusts the binary that
//! wrote an entry. `HOTLINE_E2E_IMAGE` names another probe image.

use super::*;
use crate::credentials::{NativeStore, SecretStore};

fn note(line: impl AsRef<str>) {
    eprintln!("[attest-restart-e2e] {}", line.as_ref());
}

/// The OS store, with every entry name it is asked about written to
/// `store-ops.log` (names only, never a value), so the clean run deletes
/// exactly what these runs wrote and a reader sees the store was read.
struct Recorded {
    native: NativeStore,
    log: PathBuf,
}

impl Recorded {
    fn record(&self, line: String) {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log)
            .unwrap();
        writeln!(file, "{line}").unwrap();
    }
}

impl SecretStore for Recorded {
    fn get(&self, key: &str) -> std::io::Result<Option<Vec<u8>>> {
        let got = self.native.get(key);
        let said = match &got {
            Ok(Some(_)) => "found",
            Ok(None) => "none",
            Err(_) => "error",
        };
        self.record(format!("get {key} {said}"));
        got
    }
    fn set(&self, key: &str, bytes: &[u8]) -> std::io::Result<()> {
        let set = self.native.set(key, bytes);
        self.record(format!(
            "set {key} {}",
            if set.is_ok() { "ok" } else { "error" }
        ));
        set
    }
    fn delete(&self, key: &str) -> std::io::Result<()> {
        self.record(format!("delete {key}"));
        self.native.delete(key)
    }
}

/// The wrapper the desk's `container` is found as: a create also hands the
/// computer the test CA its key proxy trusts upstream. TEST ONLY.
fn ca_shim(dir: &Path, real: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let script = format!(
        r#"#!/bin/sh
# TEST ONLY: attest-restart-e2e's wrapper around Apple container.
real='{real}'
dir='{dir}'
if [ "$1" = create ]; then
  shift
  printf '%s\n' "$*" >> "$dir/creates.log"
  HOTLINE_KEYPROXY_PROBE_ROOT="$(cat "$dir/ca.pem")"
  export HOTLINE_KEYPROXY_PROBE_ROOT
  exec "$real" create -e HOTLINE_KEYPROXY_PROBE_ROOT "$@"
fi
printf '%s\n' "$*" >> "$dir/calls.log"
exec "$real" "$@"
"#,
        real = real.display(),
        dir = dir.display()
    );
    let path = dir.join("container");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// What the first run leaves for the others: who the teammate is, where
/// the fixture API listens, and the made-up key it answers.
#[derive(serde::Serialize, serde::Deserialize)]
struct Plan {
    persona_id: String,
    image: String,
    api_ip: String,
    api_port: u16,
    key: String,
}

/// HTTPS at `ip:port` with a leaf the test CA signed: `/api/user` answers
/// only `key`, and every Authorization header that arrives is kept.
async fn api_at(ip: &str, port: u16, leaf: Vec<u8>, leaf_key: Vec<u8>, key: &str) -> Api {
    use http_body_util::Full;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use tokio_rustls::rustls;
    use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![CertificateDer::from(leaf)],
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key)),
    )
    .unwrap();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind((ip, port)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let want = format!("Bearer {key}");
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                continue;
            };
            let acceptor = acceptor.clone();
            let log = log.clone();
            let want = want.clone();
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let service = service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                    let auth = request
                        .headers()
                        .get("authorization")
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_owned();
                    lock(&log).push(auth.clone());
                    let (status, body) = if auth == want {
                        (200, r#"{"login":"rc-user"}"#)
                    } else {
                        (401, r#"{"message":"Bad credentials"}"#)
                    };
                    async move {
                        Ok::<_, std::convert::Infallible>(
                            hyper::Response::builder()
                                .status(status)
                                .header("content-type", "application/json")
                                .body(Full::new(bytes::Bytes::from(body)))
                                .unwrap(),
                        )
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(tls), service)
                    .await;
            });
        }
    });
    Api {
        origin: format!("https://{ip}:{port}"),
        host: format!("{ip}:{port}"),
        seen,
    }
}

/// A leaf for `ip` signed by `ca`, as DER: the certificate and its key.
fn leaf_for(ca: &Ca, ip: &str) -> (Vec<u8>, Vec<u8>) {
    use rcgen::{CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair};
    let leaf_key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(vec![ip.to_owned()]).unwrap();
    params.is_ca = IsCa::ExplicitNoCa;
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.use_authority_key_identifier_extension = true;
    let leaf = params.signed_by(&leaf_key, &ca.issuer).unwrap();
    (leaf.der().to_vec(), leaf_key.serialize_der())
}

struct Desk {
    room: Arc<Room>,
    agents: Arc<Fake>,
}

/// The desk over `root`, as `Desk::open_inner` wires one: the vault and the
/// computers on the one OS store, here recorded by name.
fn desk(root: &Path) -> Desk {
    let store = Arc::new(Recorded {
        native: NativeStore,
        log: root.join("store-ops.log"),
    });
    let log = Log::open(root.join("desk"));
    let vault = Arc::new(Vault::open_with_store(log.root(), log.clone(), store.clone()).unwrap());
    let agents = Fake::new(Scripted::new(Vec::new()));
    let room = Room::with_agents_computers_and_vault(
        log,
        Arc::new(DeskKeys),
        agents.clone(),
        crate::computer::Computer::with_path(root.join("runtime").as_os_str()).with_store(store),
        Some(vault),
    );
    Desk { room, agents }
}

/// The key granted protected, Used with the fixture API's host.
fn grant_the_key(room: &Room, persona_id: &str, host: &str, key: &str) {
    let vault = room.vault.as_ref().unwrap();
    vault
        .set_shared(
            "GH_TOKEN",
            StoredSecret::Variable {
                value: key.to_owned(),
                r#use: Some(
                    crate::vault::resolve_used_with(&UsedWith {
                        service: "other".into(),
                        host: Some(host.to_owned()),
                    })
                    .unwrap(),
                ),
            },
        )
        .unwrap();
    let stored = vault.shared_secrets().unwrap();
    let mut teammate = room.persona(persona_id).unwrap();
    grants(&mut teammate, &stored, &["GH_TOKEN"]);
    assert!(
        teammate
            .secret_grants
            .iter()
            .flatten()
            .any(|grant| grant.name == "GH_TOKEN" && !grant.readable),
        "granted protected"
    );
    enrol(&room.log, &teammate);
}

/// The job's request through the key proxy: the job holds a stand-in, the
/// API sees the real key.
async fn the_use_runs(gate: &Gate, api: &Api, key: &str) {
    let (_, stand_in) = gate.sh(r#"printf '%s' "${GH_TOKEN:-none}""#).await;
    note(format!(
        "job's GH_TOKEN: {} chars, starts {:?}, is the key: {}",
        stand_in.len(),
        &stand_in[..stand_in.len().min(4)],
        stand_in == key
    ));
    assert!(
        stand_in.starts_with("ghp_") && stand_in.len() == key.len() && stand_in != key,
        "a stand-in of the key's shape: {stand_in}"
    );
    let before = lock(&api.seen).len();
    let (code, said) = gate
        .sh(&format!(
            r#"curl -fsS {}/api/user -H "Authorization: Bearer $GH_TOKEN""#,
            api.origin
        ))
        .await;
    note(format!(
        "curl {}/api/user: exit {code:?}, {said}",
        api.origin
    ));
    assert_eq!(code, Some(0), "the key reaches its service: {said}");
    assert!(said.contains("rc-user"), "{said}");
    assert!(
        lock(&api.seen)[before..]
            .iter()
            .any(|auth| *auth == format!("Bearer {key}")),
        "the service saw the real key"
    );
    assert!(!said.contains(key), "and the job never did");
}

async fn shows_protected(room: &Room, persona_id: &str) {
    let status = room.computer_status(persona_id).await.unwrap();
    note(format!(
        "computer.status: state {:?}, release {:?}, protection {:?}",
        status.state, status.release, status.protection
    ));
    assert_eq!(status.protection, Some(ComputerProtection::Protected));
    let said = lock(&room.secret_notices).clone();
    note(format!("desk notices while handing secrets: {said:?}"));
    assert!(
        !said.iter().any(|text| text.contains("on this desk")),
        "nothing waits on the desk: {said:?}"
    );
}

fn runtime_path() -> PathBuf {
    PathBuf::from(
        String::from_utf8(run("/usr/bin/which", &["container"]).stdout)
            .unwrap()
            .trim(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "drives a real Hotline Computer on Apple container across a desk restart: see the module"]
async fn a_desk_restart_keeps_a_real_v2_computer_protected() {
    let root = PathBuf::from(std::env::var("HOTLINE_E2E_ROOT").expect("HOTLINE_E2E_ROOT"));
    let phase = std::env::var("HOTLINE_E2E_PHASE").expect("HOTLINE_E2E_PHASE");
    note(format!(
        "phase {phase}, pid {}, root {}",
        std::process::id(),
        root.display()
    ));
    match phase.as_str() {
        "first" => first(&root).await,
        "second" => second(&root).await,
        "clean" => clean(&root),
        other => panic!("no phase {other}"),
    }
}

async fn first(root: &Path) {
    let image = std::env::var("HOTLINE_E2E_IMAGE").unwrap_or_else(|_| SPLIT_IMAGE.to_owned());
    let real = runtime_path();
    assert!(real.is_file(), "Apple container is on PATH");
    assert!(
        run(&real.to_string_lossy(), &["image", "inspect", &image])
            .status
            .success(),
        "{image} is built locally"
    );
    assert!(!root.join("plan.json").exists(), "a fresh root");
    let shim_dir = root.join("runtime");
    let work = root.join("work");
    std::fs::create_dir_all(&shim_dir).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&work, std::fs::Permissions::from_mode(0o777)).unwrap();
    }
    let authority = ca();
    std::fs::write(shim_dir.join("ca.pem"), &authority.pem).unwrap();
    ca_shim(&shim_dir, &real);

    let persona_id = format!("hl-attest-e2e-{}", hex(3));
    let Desk { room, agents } = desk(root);
    let mut teammate = super::super::persona(&persona_id);
    teammate.name = "Attest E2E".into();
    teammate.reach = Some(crate::contract::Reach::Workspace);
    teammate.cwd = work.to_string_lossy().into_owned();
    teammate.computer = Some(PersonaComputer {
        cpus: None,
        enabled: true,
        image: Some(image.clone()),
        memory: None,
        pids: None,
        mounts: None,
    });
    enrol(&room.log, &teammate);
    note(format!("starting {persona_id} on {image}"));
    room.start(&persona_id).await.unwrap();
    let gate = Gate::of(&agents).expect("the session was handed the computer's gate");
    let host = host_address(&gate).await;
    let (leaf, leaf_key) = leaf_for(&authority, &host);
    let key = format!("ghp_{}", hex(18));
    let api = api_at(&host, 0, leaf.clone(), leaf_key.clone(), &key).await;
    let port: u16 = api.host.rsplit_once(':').unwrap().1.parse().unwrap();
    {
        use std::os::unix::fs::OpenOptionsExt;
        let dir = root.join("api");
        std::fs::create_dir_all(&dir).unwrap();
        for (name, bytes) in [("leaf.der", &leaf), ("leaf-key.der", &leaf_key)] {
            use std::io::Write;
            std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(dir.join(name))
                .unwrap()
                .write_all(bytes)
                .unwrap();
        }
    }
    let plan = Plan {
        persona_id: persona_id.clone(),
        image,
        api_ip: host.clone(),
        api_port: port,
        key: key.clone(),
    };
    std::fs::write(
        root.join("plan.json"),
        serde_json::to_vec_pretty(&plan).unwrap(),
    )
    .unwrap();
    note(format!("fixture API on {}", api.origin));

    grant_the_key(&room, &persona_id, &api.host, &key);
    room.secrets_changed().await;
    shows_protected(&room, &persona_id).await;
    the_use_runs(&gate, &api, &key).await;
    let ready = room.computers.running(&persona_id).await.unwrap();
    note(format!(
        "container {} left running for the next desk; bearer sha256 {}",
        crate::computer::container_name(&persona_id),
        sha(&ready.token)
    ));
    note("first desk process ends here");
}

async fn second(root: &Path) {
    let plan: Plan =
        serde_json::from_slice(&std::fs::read(root.join("plan.json")).unwrap()).unwrap();
    let persona_id = plan.persona_id.clone();
    let real = runtime_path();
    let listed = String::from_utf8_lossy(&run(&real.to_string_lossy(), &["list", "--all"]).stdout)
        .lines()
        .filter(|line| line.contains(&persona_id))
        .collect::<Vec<_>>()
        .join("\n");
    note(format!("before this desk starts: {listed}"));
    assert!(listed.contains("running"), "the computer is still running");
    let leaf = std::fs::read(root.join("api/leaf.der")).unwrap();
    let leaf_key = std::fs::read(root.join("api/leaf-key.der")).unwrap();
    let api = api_at(&plan.api_ip, plan.api_port, leaf, leaf_key, &plan.key).await;
    note(format!("fixture API on {} again", api.origin));

    let ops_before = std::fs::read_to_string(root.join("store-ops.log")).unwrap_or_default();
    let creates_before = std::fs::read_to_string(root.join("runtime/creates.log")).unwrap();
    let Desk { room, agents } = desk(root);
    let started = Instant::now();
    room.start(&persona_id).await.unwrap();
    note(format!(
        "teammate started on the running computer in {} ms",
        started.elapsed().as_millis()
    ));
    let creates = std::fs::read_to_string(root.join("runtime/creates.log")).unwrap();
    assert_eq!(
        creates, creates_before,
        "the same container, not made again"
    );
    let ops = std::fs::read_to_string(root.join("store-ops.log")).unwrap();
    let attest_ops: Vec<&str> = ops[ops_before.len()..]
        .lines()
        .filter(|line| line.contains("computer-attest/"))
        .collect();
    note(format!(
        "this desk's store calls for the attest key: {attest_ops:?}"
    ));
    assert!(
        attest_ops
            .iter()
            .any(|line| line.starts_with("get ") && line.ends_with(" found")),
        "the kept key was read back"
    );
    assert!(
        !attest_ops.iter().any(|line| line.starts_with("set ")),
        "and not taken again"
    );
    shows_protected(&room, &persona_id).await;
    let gate = Gate::of(&agents).expect("the session was handed the computer's gate");
    the_use_runs(&gate, &api, &plan.key).await;

    // The key door said its key was taken long ago, and this desk did not
    // need it: asked now, it still says 409.
    let ready = room.computers.running(&persona_id).await.unwrap();
    let base = ready.url.strip_suffix("/mcp").unwrap().to_owned();
    let said = reqwest::Client::new()
        .get(format!("{base}/capabilities?nonce=e2e-{}", hex(4)))
        .bearer_auth(&ready.token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    note(format!(
        "/capabilities: version {}, protected {}",
        said["version"], said["protected"]
    ));
    let again = reqwest::Client::new()
        .post(format!("{base}/capabilities/key"))
        .bearer_auth(&ready.token)
        .send()
        .await
        .unwrap()
        .status();
    note(format!("POST /capabilities/key now: {again}"));
    assert_eq!(again, 409);
    note(format!("bearer sha256 {}", sha(&ready.token)));
    note("second desk process ends here");
}

/// Removes the container and every OS store entry these runs wrote, by the
/// names they recorded; the root and its logs stay as the record.
fn clean(root: &Path) {
    let real = runtime_path();
    let runtime = real.to_string_lossy().into_owned();
    if let Ok(plan) = std::fs::read(root.join("plan.json")) {
        let plan: Plan = serde_json::from_slice(&plan).unwrap();
        let name = crate::computer::container_name(&plan.persona_id);
        let _ = run(&runtime, &["stop", &name]);
        let removed = run(&runtime, &["rm", "-f", &name]);
        note(format!("removed {name}: {}", removed.status));
    }
    let ops = std::fs::read_to_string(root.join("store-ops.log")).unwrap_or_default();
    let mut names: Vec<&str> = ops
        .lines()
        .filter_map(|line| line.strip_prefix("set "))
        .filter_map(|rest| rest.rsplit_once(' ').map(|(name, _)| name))
        .collect();
    names.sort_unstable();
    names.dedup();
    for name in names {
        NativeStore.delete(name).unwrap();
        let left = NativeStore.get(name).unwrap();
        note(format!(
            "deleted store entry {name}: gone {}",
            left.is_none()
        ));
        assert!(left.is_none());
    }
}
