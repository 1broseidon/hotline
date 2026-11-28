//! The desk against a real Hotline Computer, with no agent and no window.
//!
//! One teammate's computer is made by the desk's own code on Apple
//! `container`: first from a tag the desk does not know as split (today's
//! create flags, so the computer boots legacy), then updated to the 0.13
//! release candidate (the split flags). Everything a person and an agent do
//! goes through the desk's real paths: the room hands the secrets over, the
//! agent's MCP calls go through the gate the session was handed, the cards
//! come from the room's own watch and are answered through the room.
//!
//! The "agent" is this test calling the gate's tools directly; the session's
//! driver is a script that never runs a turn, so no harness and no model is
//! ever started. Every secret is made up here.
//!
//! What it needs, and what it leaves:
//! - macOS with Apple `container` running, and the probe build of the
//!   release candidate (`hotline-computer:0.13.0-rc.3-probe`, or
//!   `HOTLINE_E2E_IMAGE`, a later one that attests v2; an rc.2 one attests
//!   v1, which this desk hands nothing): the release image with its key proxy also trusting a test CA
//!   upstream (`qa/0.13.0-rc.N/Dockerfile.probe` in the computer repo). The CA is made here and handed to the container by a
//!   test-only wrapper around `container create`, which also gives the
//!   computer a named home volume so the update keeps its home, as Docker's
//!   and Podman's home volume does.
//! - The container is `hotline-computer-hl-secrets-e2e-<hex>`, the volume
//!   and the legacy tag `hl-secrets-e2e-*`; all three are removed at the end,
//!   pass or fail. No other container is touched.
//!
//! Run it with `make secrets-e2e`.

use super::*;
use crate::contract::{
    ComputerProtection, CookieTransfer, PersonaComputer, SecretUseAnswer, SnapshotAnswer, UsedWith,
};
use crate::mcp::{HttpAuth, McpTransport};
use crate::vault::StoredSecret;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

mod off;
mod restart;

const SPLIT_IMAGE: &str = "hotline-computer:0.13.0-rc.3-probe";
const SITE_PORT: u16 = 8600;
const SITE: &str = "http://localhost:8600";
const USERNAME: &str = "rc-user";
/// The fixture site, from the computer repo's `tests/protected/jobs/site.py`.
const SITE_PY: &str = include_str!("real_computer_site.py");

fn say(line: impl AsRef<str>) {
    eprintln!("[secrets-e2e] {}", line.as_ref());
}

fn hex(bytes: usize) -> String {
    let mut digits = String::new();
    while digits.len() < bytes * 2 {
        digits.push_str(&uuid::Uuid::new_v4().simple().to_string());
    }
    digits[..bytes * 2].to_owned()
}

fn run(program: &str, args: &[&str]) -> std::process::Output {
    Command::new(program)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("{program} {args:?}: {error}"))
}

/// What the run made, removed whatever happens to the test.
struct Leftovers {
    runtime: PathBuf,
    container: String,
    volume: String,
    tag: String,
}

impl Drop for Leftovers {
    fn drop(&mut self) {
        let runtime = self.runtime.to_string_lossy().into_owned();
        let _ = run(&runtime, &["stop", &self.container]);
        let _ = run(&runtime, &["rm", "-f", &self.container]);
        let _ = run(&runtime, &["rm", "-f", &format!("{}-chown", self.volume)]);
        let _ = run(&runtime, &["volume", "rm", &self.volume]);
        let _ = run(&runtime, &["image", "rm", &self.tag]);
        say(format!(
            "removed {}, volume {} and tag {}",
            self.container, self.volume, self.tag
        ));
    }
}

/// The test CA the probe image's key proxy trusts upstream, and the
/// fixture API a protected key is bound to.
struct Ca {
    pem: String,
    issuer: rcgen::Issuer<'static, rcgen::KeyPair>,
}

fn ca() -> Ca {
    use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.distinguished_name.push(
        rcgen::DnType::CommonName,
        "hl-secrets-e2e fixture CA (test only)",
    );
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let cert = params.self_signed(&key).unwrap();
    Ca {
        pem: cert.pem(),
        issuer: rcgen::Issuer::new(params, key),
    }
}

/// HTTPS on the host's container-bridge address: `/api/user` answers only
/// the real key, and every Authorization header that arrives is kept.
struct Api {
    origin: String,
    host: String,
    seen: Arc<Mutex<Vec<String>>>,
}

async fn api(ca: &Ca, ip: &str, key: &str) -> Api {
    use http_body_util::Full;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use rcgen::{CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair};
    use tokio_rustls::rustls;
    use tokio_rustls::rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
    let leaf_key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(vec![ip.to_owned()]).unwrap();
    params.is_ca = IsCa::ExplicitNoCa;
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.use_authority_key_identifier_extension = true;
    let leaf = params.signed_by(&leaf_key, &ca.issuer).unwrap();
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![leaf.der().clone()],
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
    )
    .unwrap();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind((ip, 0)).await.unwrap();
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

/// The agent's side: the gate the session was handed, called as an agent
/// calls it.
#[derive(Clone, Debug, PartialEq)]
struct Gate {
    url: String,
    token: String,
}

impl Gate {
    fn of(agents: &Fake) -> Option<Gate> {
        agents.latest_mcps().into_iter().find_map(|server| {
            (server.id == crate::computer::SERVER_ID).then_some(())?;
            match server.transport {
                McpTransport::Http {
                    url,
                    auth: HttpAuth::Bearer { token },
                } => Some(Gate { url, token }),
                _ => None,
            }
        })
    }

    /// One tool call: its text, and whether it was an error.
    async fn call(&self, tool: &str, args: Value) -> (String, bool) {
        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(self.url.clone())
                .auth_header(self.token.clone()),
        );
        let service = ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("hl-secrets-e2e", "0"),
        )
        .serve(transport)
        .await
        .expect("the gate opens an MCP session");
        let answered = service
            .call_tool(
                CallToolRequestParams::new(tool.to_owned())
                    .with_arguments(args.as_object().cloned().unwrap_or_default()),
            )
            .await;
        service.cancel().await.ok();
        match answered {
            Ok(result) => (
                result
                    .content
                    .iter()
                    .filter_map(|block| block.as_text().map(|text| text.text.clone()))
                    .collect::<Vec<_>>()
                    .join("\n"),
                result.is_error == Some(true),
            ),
            Err(error) => (error.to_string(), true),
        }
    }

    /// A job's exit code and what it printed.
    async fn sh(&self, command: &str) -> (Option<i64>, String) {
        let (text, _) = self
            .call(
                "shell",
                json!({"action": "exec", "command": command, "wait_ms": 45_000}),
            )
            .await;
        match serde_json::from_str::<Value>(&text) {
            Ok(done) => (
                done["exit_code"].as_i64(),
                format!(
                    "{}{}",
                    done["stdout"].as_str().unwrap_or_default(),
                    done["stderr"].as_str().unwrap_or_default()
                ),
            ),
            Err(_) => (None, text),
        }
    }

    /// `browser text` until it shows `want`, for a page that is still
    /// loading or a tab hidden until its next document.
    async fn look(&self, want: &str, seconds: u64) -> String {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        loop {
            let (text, _) = self.call("browser", json!({"action": "text"})).await;
            if text.contains(want) || Instant::now() > deadline {
                return text;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

/// The ref of the first field of `kind` in a `browser text` answer whose
/// line has `words`.
fn ref_for(text: &str, kind: &str, words: &str) -> Option<String> {
    let pattern = regex::Regex::new(r"\[(e\d+)\] \[(\w+)\]([^\n]*)").unwrap();
    pattern.captures_iter(text).find_map(|found| {
        (&found[2] == kind && found[3].to_lowercase().contains(&words.to_lowercase()))
            .then(|| found[1].to_owned())
    })
}

/// The host as the container sees it: its default gateway.
async fn host_address(gate: &Gate) -> String {
    let (_, routes) = gate.sh("cat /proc/net/route").await;
    let gateway = routes
        .lines()
        .map(str::split_whitespace)
        .find_map(|mut fields| {
            let _ = fields.next()?;
            (fields.next()? == "00000000").then(|| fields.next().map(str::to_owned))?
        })
        .expect("the container has a default route");
    let raw = u32::from_str_radix(&gateway, 16).unwrap();
    std::net::Ipv4Addr::from(raw.to_le_bytes()).to_string()
}

/// The latest word of each secret-use card on the tape, oldest first.
fn use_cards(room: &Room, persona: &str) -> Vec<Value> {
    let mut cards: Vec<Value> = Vec::new();
    for event in tape(room, persona) {
        if event["kind"] != "secret_use_ask" {
            continue;
        }
        match cards.iter_mut().find(|card| card["id"] == event["id"]) {
            Some(card) => *card = event,
            None => cards.push(event),
        }
    }
    cards
}

/// Polls the room's own watch until a pending card matches.
async fn pending_card(
    room: &Arc<Room>,
    persona: &str,
    seconds: u64,
    matches: impl Fn(&Value) -> bool,
) -> Option<Value> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        room.poll_secret_uses(persona).await;
        if let Some(card) = use_cards(room, persona)
            .into_iter()
            .find(|card| card["status"] == "pending" && matches(card))
        {
            return Some(card);
        }
        if Instant::now() > deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn eventually<F, Fut>(what: &str, seconds: u64, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while !check().await {
        assert!(Instant::now() < deadline, "never: {what}");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// A login fill as the agent makes it on an Ask-me login: held at first,
/// the card answered through the room, then the fill made again. Answers
/// what each card asked, in order.
async fn fill_asking(
    room: &Arc<Room>,
    persona: &str,
    gate: &Gate,
    reference: &str,
    secret: &str,
) -> Vec<Value> {
    let mut asked = Vec::new();
    for _ in 0..3 {
        let (text, _) = gate
            .call(
                "browser",
                json!({"action": "fill", "ref": reference, "secret": secret}),
            )
            .await;
        say(format!("fill {secret}: {text}"));
        if !text.contains("Nothing was typed") {
            assert!(
                !text.to_lowercase().contains("error"),
                "{secret} was filled: {text}"
            );
            return asked;
        }
        let card = pending_card(room, persona, 30, |card| card["secret"] == "SITE")
            .await
            .unwrap_or_else(|| panic!("a card for {secret}: {text}"));
        let id = card["useId"].as_str().unwrap().to_owned();
        let answer = if card["attested"] == true {
            SecretUseAnswer::Allow
        } else {
            // Ten minutes are only for the broker's own sign-in.
            assert!(
                room.secrets_use_answer(persona, &id, SecretUseAnswer::Allow)
                    .await
                    .is_err(),
                "Allow is refused for an unattested use: {card}"
            );
            SecretUseAnswer::Approve
        };
        room.secrets_use_answer(persona, &id, answer)
            .await
            .unwrap_or_else(|error| panic!("answered {card}: {error}"));
        asked.push(card);
    }
    panic!("{secret} was never filled");
}

/// The wrapper the desk's `container` is found as: every call goes to the
/// real CLI, and a create also hands the computer the test CA (by name; the
/// value is in the CLI's environment) and the home volume, and is logged.
fn shim(dir: &Path, real: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let script = format!(
        r#"#!/bin/sh
# TEST ONLY: hl-secrets-e2e's wrapper around Apple container.
real='{real}'
dir='{dir}'
if [ "$1" = create ]; then
  shift
  printf '%s\n' "$*" >> "$dir/creates.log"
  HOTLINE_KEYPROXY_PROBE_ROOT="$(cat "$dir/ca.pem")"
  export HOTLINE_KEYPROXY_PROBE_ROOT
  exec "$real" create -e HOTLINE_KEYPROXY_PROBE_ROOT -v "$(cat "$dir/home"):/home/agent" "$@"
fi
exec "$real" "$@"
"#,
        real = real.display(),
        dir = dir.display()
    );
    let path = dir.join("container");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn grants(persona: &mut Persona, stored: &[SharedSecret], names: &[&str]) {
    persona.secrets = Some(names.iter().map(ToString::to_string).collect());
    persona.secret_grants = crate::session::grants::settled(persona, stored);
}

fn sha(value: &str) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(value.as_bytes()))
}

fn base32_seed() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let bytes = [uuid::Uuid::new_v4(), uuid::Uuid::new_v4()]
        .iter()
        .flat_map(|id| id.as_bytes().to_vec())
        .collect::<Vec<_>>();
    bytes[..32]
        .iter()
        .map(|byte| ALPHABET[usize::from(*byte) % 32] as char)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "drives a real Hotline Computer on Apple container: make secrets-e2e"]
async fn the_desk_and_a_real_computer_keep_secrets_together() {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use sha2::Digest;

    let image = std::env::var("HOTLINE_E2E_IMAGE").unwrap_or_else(|_| SPLIT_IMAGE.to_owned());
    let real = PathBuf::from(
        String::from_utf8(run("/usr/bin/which", &["container"]).stdout)
            .unwrap()
            .trim(),
    );
    assert!(real.is_file(), "Apple container is on PATH");
    let runtime = real.to_string_lossy().into_owned();
    say(format!(
        "containers already here; none is touched:\n{}",
        String::from_utf8_lossy(&run(&runtime, &["list", "--all"]).stdout)
    ));
    let present = run(&runtime, &["image", "inspect", &image]);
    assert!(present.status.success(), "{image} is built locally");

    let run_id = hex(3);
    let persona_id = format!("hl-secrets-e2e-{run_id}");
    let volume = format!("hl-secrets-e2e-home-{run_id}");
    let legacy_image = format!("hl-secrets-e2e-legacy-{run_id}");
    let _leftovers = Leftovers {
        runtime: real.clone(),
        container: crate::computer::container_name(&persona_id),
        volume: volume.clone(),
        tag: format!("{legacy_image}:latest"),
    };

    // A tag the desk does not know as a release: today's create flags.
    assert!(
        run(&runtime, &["image", "tag", &image, &legacy_image])
            .status
            .success()
    );
    // A home that outlives the container, owned by the agent, as the
    // computer repo's probes make one on Apple container.
    assert!(
        run(&runtime, &["volume", "create", &volume])
            .status
            .success()
    );
    let chowned = run(
        &runtime,
        &[
            "run",
            "--rm",
            "--name",
            &format!("{volume}-chown"),
            "--user",
            "0",
            "--entrypoint",
            "/bin/sh",
            "-v",
            &format!("{volume}:/home/agent"),
            &image,
            "-c",
            "chown 1000:1000 /home/agent && chmod 0700 /home/agent && { rmdir /home/agent/lost+found 2>/dev/null; true; }",
        ],
    );
    assert!(
        chowned.status.success(),
        "{}",
        String::from_utf8_lossy(&chowned.stderr)
    );

    let root = std::env::temp_dir().join(format!("hl-secrets-e2e-{run_id}"));
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
    std::fs::write(shim_dir.join("home"), &volume).unwrap();
    shim(&shim_dir, &real);

    // Made-up secrets, none of them anything's.
    let readable = format!("hl-e2e-readable-{}", hex(8));
    let key = format!("ghp_{}", hex(18));
    let password = format!("hl-e2e-pw-{}", hex(6));
    let seed = base32_seed();
    let passkey = rcgen::KeyPair::generate().unwrap();
    let point = passkey.public_key_raw().to_vec();
    let credential = STANDARD.encode(uuid::Uuid::new_v4().as_bytes());

    let log = scratch(&format!("secrets-e2e-{run_id}"));
    let vault = Arc::new(
        Vault::open_with_store(
            log.root(),
            log.clone(),
            Arc::new(crate::credentials::MemoryStore::default()),
        )
        .unwrap(),
    );
    vault.set_shared_secret("READABLE_VAR", &readable).unwrap();
    vault
        .set_shared(
            "SITE",
            StoredSecret::Login {
                r#use: crate::contract::SecretUse::Ask,
                sites: vec![SITE.into()],
                username: USERNAME.into(),
                password: password.clone(),
                totp: Some(seed.clone()),
            },
        )
        .unwrap();
    vault
        .set_shared(
            "SITEKEY",
            StoredSecret::Passkey {
                r#use: crate::contract::SecretUse::Auto,
                rp_id: "localhost".into(),
                credential_id: credential.clone(),
                private_key: STANDARD.encode(passkey.serialize_der()),
                user_handle: Some(STANDARD.encode(b"rc-user-1")),
                user_name: Some(USERNAME.into()),
                user_display_name: None,
            },
        )
        .unwrap();

    let mut teammate = super::persona(&persona_id);
    teammate.name = "E2E".into();
    teammate.reach = Some(crate::contract::Reach::Workspace);
    teammate.cwd = work.to_string_lossy().into_owned();
    teammate.computer = Some(PersonaComputer {
        cpus: None,
        enabled: true,
        image: Some(legacy_image.clone()),
        memory: None,
        pids: None,
        mounts: None,
    });
    let stored = vault.shared_secrets().unwrap();
    grants(&mut teammate, &stored, &["READABLE_VAR", "SITE", "SITEKEY"]);
    enrol(&log, &teammate);
    let agents = Fake::new(Scripted::new(Vec::new()));
    let room = Room::with_agents_computers_and_vault(
        log,
        Arc::new(DeskKeys),
        agents.clone(),
        crate::computer::Computer::with_path(shim_dir.as_os_str()),
        Some(vault.clone()),
    );

    // ---- 1. Today's flags: the computer runs legacy and says so. ----------
    say(format!(
        "starting {persona_id} on {legacy_image} (today's flags)"
    ));
    room.start(&persona_id).await.unwrap();
    assert!(
        room.computer_in_use(&persona_id),
        "the computer is attached: {:?}",
        tape(&room, &persona_id)
    );
    let creates = std::fs::read_to_string(shim_dir.join("creates.log")).unwrap();
    assert!(!creates.contains("--user"), "today's flags: {creates}");
    let gate = Gate::of(&agents).expect("the session was handed the computer's gate");
    let host = host_address(&gate).await;
    let fixture = api(&authority, &host, &key).await;
    say(format!(
        "fixture API on {} for the protected key",
        fixture.origin
    ));

    // The key, Used with the fixture's host, granted protected.
    vault
        .set_shared(
            "GH_TOKEN",
            StoredSecret::Variable {
                value: key.clone(),
                r#use: Some(
                    crate::vault::resolve_used_with(&UsedWith {
                        service: "other".into(),
                        host: Some(fixture.host.clone()),
                    })
                    .unwrap(),
                ),
            },
        )
        .unwrap();
    let stored = vault.shared_secrets().unwrap();
    let mut teammate = room.persona(&persona_id).unwrap();
    grants(
        &mut teammate,
        &stored,
        &["READABLE_VAR", "SITE", "SITEKEY", "GH_TOKEN"],
    );
    assert!(
        teammate
            .secret_grants
            .iter()
            .flatten()
            .any(|grant| grant.name == "GH_TOKEN" && !grant.readable),
        "a key Used with a service is granted protected"
    );
    enrol(&room.log, &teammate);
    room.secrets_changed().await;

    let status = room.computer_status(&persona_id).await.unwrap();
    say(format!(
        "legacy status: protection {:?}, release {:?}, available {:?}",
        status.protection, status.release, status.available
    ));
    assert_eq!(status.protection, Some(ComputerProtection::NeedsUpdate));
    assert_eq!(status.migration, None);
    let (_, env) = gate
        .sh(r#"printf '%s|%s' "$(printf %s "$READABLE_VAR" | sha256sum | cut -c1-64)" "${GH_TOKEN:-none}""#)
        .await;
    assert_eq!(
        env,
        format!("{}|none", sha(&readable)),
        "a readable variable as today, the protected key held back"
    );
    let (info, _) = gate.call("state", json!({"action": "info"})).await;
    let catalog = serde_json::from_str::<Value>(&info)
        .map_or(info.clone(), |info| info["secrets"].to_string());
    for name in ["SITE", "SITEKEY", "READABLE_VAR"] {
        assert!(
            catalog.contains(name),
            "{name} delivered as today: {catalog}"
        );
    }
    assert!(!catalog.contains("GH_TOKEN"), "{catalog}");
    let (saved, failed) = gate
        .call(
            "state",
            json!({"action": "snapshot_save", "name": "before"}),
        )
        .await;
    assert!(!failed, "a snapshot from before the split: {saved}");

    // ---- 2. Update to the release candidate: the split flags. -------------
    let old_gate = gate;
    let old_bearer = room.computers.running(&persona_id).await.unwrap().token;
    let mut teammate = room.persona(&persona_id).unwrap();
    teammate.computer.as_mut().unwrap().image = Some(image.clone());
    enrol(&room.log, &teammate);
    let status = room.computer_status(&persona_id).await.unwrap();
    assert_eq!(
        status.available.as_deref(),
        Some(crate::computer::release_of(&image).as_str()),
        "the pane offers the update"
    );
    say(format!("updating to {image} (split flags)"));
    room.computer_update(&persona_id).await.unwrap();
    eventually(
        "the split computer is attached and proved itself",
        240,
        || {
            let room = room.clone();
            let agents = agents.clone();
            let old = old_gate.clone();
            let id = persona_id.clone();
            async move {
                Gate::of(&agents).is_some_and(|gate| gate != old)
                    && room.computer_status(&id).await.unwrap().protection
                        == Some(ComputerProtection::Protected)
            }
        },
    )
    .await;
    let creates = std::fs::read_to_string(shim_dir.join("creates.log")).unwrap();
    let split_create = creates.lines().last().unwrap();
    assert!(
        split_create.contains("--cap-drop=ALL --user 0 --cap-add SETUID --cap-add SETGID"),
        "the split flags: {split_create}"
    );
    let gate = Gate::of(&agents).unwrap();

    // The computer the update made has a bearer of its own, and the one
    // before it, which that computer's programs could read, opens nothing
    // on it: not its proof, its key door, or its secrets.
    let ready = room.computers.running(&persona_id).await.unwrap();
    assert_ne!(ready.token, old_bearer, "a new bearer");
    let base = ready.url.strip_suffix("/mcp").unwrap().to_owned();
    let client = reqwest::Client::new();
    for (method, path) in [
        (reqwest::Method::GET, "/capabilities?nonce=old"),
        (reqwest::Method::POST, "/capabilities/key"),
        (reqwest::Method::PUT, "/secrets"),
    ] {
        let refused = client
            .request(method, format!("{base}{path}"))
            .bearer_auth(&old_bearer)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body("{}")
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(refused, 401, "{path} under the old bearer");
    }

    let status = room.computer_status(&persona_id).await.unwrap();
    say(format!(
        "split status: protection {:?}, release {:?}, available {:?}, migration {:?}",
        status.protection, status.release, status.available, status.migration
    ));
    assert_eq!(status.available, None, "nothing more to update to");

    // What the computer attests with: v2 (rc.3 on), the only proof this
    // desk takes. It handed its boot's key to the desk as it started, so it
    // hands it to nobody now, and names what it cannot keep from its
    // programs: the loopback login and passkey, and the key used with an
    // "Other" host.
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
    say(format!(
        "/capabilities: protected {}, version {}",
        said["protected"], said["version"]
    ));
    assert_eq!(said["protected"]["v"], 2, "{said}");
    let again = reqwest::Client::new()
        .post(format!("{base}/capabilities/key"))
        .bearer_auth(&ready.token)
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(again, 409, "the desk took this boot's key first");
    let mut said_readable = status.readable.clone();
    said_readable.sort();
    assert_eq!(said_readable, ["GH_TOKEN", "SITE", "SITEKEY"], "{status:?}");

    // The first split boot of a home with a snapshot asks once.
    let migration = status
        .migration
        .expect("the old snapshot waits for an answer");
    assert_eq!(
        migration
            .snapshots
            .iter()
            .map(|snapshot| snapshot.name.as_str())
            .collect::<Vec<_>>(),
        ["before"]
    );
    let ready = room.computers.running(&persona_id).await.unwrap();
    let computer_says = |ready: crate::computer::Ready| async move {
        let base = ready.url.strip_suffix("/mcp").unwrap().to_owned();
        reqwest::Client::new()
            .get(format!("{base}/migration"))
            .bearer_auth(&ready.token)
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap()
    };
    room.computer_migration_answer(&persona_id, SnapshotAnswer::Keep)
        .await
        .unwrap();
    let kept = computer_says(ready.clone()).await;
    say(format!("/migration after keep: {kept}"));
    assert_eq!(
        room.computer_status(&persona_id).await.unwrap().migration,
        None,
        "an answered notice asks no more, keep included: the computer says {kept}"
    );
    room.computer_migration_answer(&persona_id, SnapshotAnswer::Clean)
        .await
        .unwrap();
    let cleaned = computer_says(ready).await;
    assert_eq!(cleaned["state"], "done", "{cleaned}");
    assert_eq!(cleaned["answer"], "clean", "{cleaned}");

    // ---- 3. Delivery v2 and the key proxy, through the agent's gate. ------
    let (_, env) = gate
        .sh(r#"printf '%s|%s' "$(printf %s "$READABLE_VAR" | sha256sum | cut -c1-64)" "$GH_TOKEN""#)
        .await;
    let (seen_readable, stand_in) = env.split_once('|').unwrap();
    assert_eq!(
        seen_readable,
        sha(&readable),
        "a readable variable is readable"
    );
    assert!(
        stand_in.starts_with("ghp_") && stand_in.len() == key.len() && stand_in != key,
        "a job holds a stand-in of the key's shape: {stand_in}"
    );
    let (code, said) = gate
        .sh(&format!(
            r#"curl -fsS {}/api/user -H "Authorization: Bearer $GH_TOKEN""#,
            fixture.origin
        ))
        .await;
    assert_eq!(code, Some(0), "the key reaches its service: {said}");
    assert!(said.contains("rc-user"), "{said}");
    assert!(
        lock(&fixture.seen)
            .iter()
            .any(|auth| *auth == format!("Bearer {key}")),
        "the service saw the real key"
    );
    assert!(!said.contains(&key), "and the job never did");

    // ---- 4. Sign in: Ask me, a sign-in to finish, then the code. ----------
    let salt = hex(8);
    let step = chrono::Utc::now().timestamp() as u64 / 30;
    let codes: Vec<String> = (step - 2..step + 120)
        .map(|at| {
            let code = crate::browser::totp_code(&seed, at * 30).unwrap();
            format!("{:x}", sha2::Sha256::digest(code.as_bytes()))
        })
        .collect();
    let conf = json!({
        "port": SITE_PORT, "username": USERNAME, "pw_salt": salt,
        "pw_hash": format!("{:x}", sha2::Sha256::digest(format!("{salt}{password}").as_bytes())),
        "totp_hashes": codes, "cred_id": credential,
        "pub_x": point[1..33].iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "pub_y": point[33..].iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "report": "id,amount\n1,42\n",
    });
    gate.sh("mkdir -p ~/probe").await;
    for (path, content) in [
        ("probe/site.py", SITE_PY.to_owned()),
        ("probe/site.json", conf.to_string()),
    ] {
        let (put, failed) = gate
            .call(
                "files",
                json!({"action": "put", "path": path, "content": content}),
            )
            .await;
        assert!(!failed, "{path}: {put}");
    }
    gate.call(
        "shell",
        json!({"action": "start", "command": "exec python3 ~/probe/site.py ~/probe/site.json",
               "label": "Fixture site (test only)", "wait_ms": 500}),
    )
    .await;
    eventually("the fixture site answers", 20, || {
        let gate = gate.clone();
        async move {
            gate.sh(&format!("curl -s -o /dev/null -w '%{{http_code}}' {SITE}/"))
                .await
                .1
                .contains("200")
        }
    })
    .await;

    gate.call(
        "browser",
        json!({"action": "navigate", "url": format!("{SITE}/login")}),
    )
    .await;
    let page = gate.look("Password", 20).await;
    let user = ref_for(&page, "text", "").expect(&page);
    let pass = ref_for(&page, "password", "").expect(&page);
    let submit = ref_for(&page, "button", "sign in").expect(&page);
    let mut asked = fill_asking(&room, &persona_id, &gate, &user, "SITE.username").await;
    let first_fill = asked.len();
    asked.extend(fill_asking(&room, &persona_id, &gate, &pass, "SITE.password").await);
    let card = asked.first().expect("Ask me held a fill for the person");
    say(format!("the Ask-me card: {card}"));
    assert_eq!(card["operation"]["kind"], "signin", "{card}");
    assert_eq!(card["operation"]["origin"], SITE, "{card}");
    assert_eq!(card["operation"]["account"], USERNAME, "{card}");
    if card["attested"] == true && first_fill > 0 {
        assert_eq!(
            asked.len(),
            1,
            "Allow 10 min covers the same sign-in: {asked:?}"
        );
    }

    // Not submitted: 30 s on the same document is a sign-in to finish.
    let finish = pending_card(&room, &persona_id, 60, |card| {
        card["operation"]["kind"] == "finish"
    })
    .await
    .expect("a sign-in left on its page is the person's to finish");
    say(format!("the finish card: {finish}"));
    assert_eq!(finish["secret"], "SITE");
    assert_eq!(finish["operation"]["origin"], SITE);
    assert!(
        room.secrets_use_answer(
            &persona_id,
            finish["useId"].as_str().unwrap(),
            SecretUseAnswer::Approve
        )
        .await
        .is_err(),
        "a finish card has no answer, only Open"
    );

    // ---- 5. Open Browser: the site's screen, for the person only. ---------
    let viewer = room
        .computer_status(&persona_id)
        .await
        .unwrap()
        .viewer
        .expect("a running computer has a viewer");
    let (port, token) = viewer
        .strip_prefix("http://127.0.0.1:")
        .and_then(|rest| rest.split_once("/#"))
        .unwrap();
    // The window opens the finish card's origin, as the phone's bridge
    // names it.
    let site = finish["operation"]["origin"].as_str().unwrap().to_owned();
    assert_eq!(
        crate::computer::site_host(&site).as_deref(),
        Some(SITE),
        "the phone can name a loopback http site too"
    );
    let screen = format!("ws://127.0.0.1:{port}/sites/ws?token={token}&site={site}");
    let (mut socket, _) = tokio_tungstenite::connect_async(&screen)
        .await
        .expect("the person's site screen connects");
    let frame = tokio::time::timeout(Duration::from_secs(20), async {
        use futures_util::StreamExt;
        loop {
            match socket.next().await {
                Some(Ok(tokio_tungstenite::tungstenite::Message::Binary(frame))) => {
                    return Ok(frame.len());
                }
                Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text)))
                    if text.contains("error") =>
                {
                    return Err(text.to_string());
                }
                Some(Ok(_)) => {}
                other => return Err(format!("{other:?}")),
            }
        }
    })
    .await
    .expect("a frame within 20 s");
    assert!(frame.is_ok_and(|bytes| bytes > 12), "a picture of the site");
    drop(socket);
    let gate_base = gate.url.strip_suffix("/mcp").unwrap_or(&gate.url);
    let through_gate = format!(
        "{}/sites/ws?token={token}&site={site}",
        gate_base.replace("http://", "ws://")
    );
    let mut request =
        tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
            through_gate.as_str(),
        )
        .unwrap();
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", gate.token).parse().unwrap(),
    );
    assert!(
        tokio_tungstenite::connect_async(request).await.is_err(),
        "the agent's gate never opens the site screen"
    );
    let plain = reqwest::Client::new()
        .get(format!("{gate_base}/sites/ws?token={token}&site={site}"))
        .bearer_auth(&gate.token)
        .send()
        .await
        .unwrap();
    assert!(
        !plain.status().is_success(),
        "nor answers it as a page: {}",
        plain.status()
    );

    // Submitting moves the tab on: the finish card goes, and the code is
    // typed by the computer.
    gate.call("browser", json!({"action": "click_ref", "ref": submit}))
        .await;
    let page = gate.look("Two-step verification", 20).await;
    let code = ref_for(&page, "text", "").expect(&page);
    let verify = ref_for(&page, "button", "verify").expect(&page);
    fill_asking(&room, &persona_id, &gate, &code, "SITE.code").await;
    gate.call("browser", json!({"action": "click_ref", "ref": verify}))
        .await;
    let page = gate.look("Signed in as", 20).await;
    assert!(
        page.contains("Signed in as rc-user (password and code)"),
        "{page}"
    );
    eventually("the finish card is settled", 20, || {
        let room = room.clone();
        let id = persona_id.clone();
        let finish = finish["id"].clone();
        async move {
            room.poll_secret_uses(&id).await;
            use_cards(&room, &id)
                .iter()
                .any(|card| card["id"] == finish && card["status"] != "pending")
        }
    })
    .await;

    // ---- 6. The passkey, signed in the signed-in browser. -----------------
    gate.call(
        "browser",
        json!({"action": "navigate", "url": format!("{SITE}/logout")}),
    )
    .await;
    gate.look("Signed out", 20).await;
    gate.call(
        "browser",
        json!({"action": "navigate", "url": format!("{SITE}/passkey")}),
    )
    .await;
    let page = gate.look("Sign in with a passkey", 20).await;
    let button = ref_for(&page, "button", "passkey").expect(&page);
    gate.call("browser", json!({"action": "click_ref", "ref": button}))
        .await;
    let page = gate.look("Signed in as", 30).await;
    assert!(page.contains("Signed in as rc-user (passkey)"), "{page}");

    // ---- 7. Use Cookies into the signed-in browser, and back out. ---------
    let cookie = format!("hl-e2e-cookie-{}", hex(6));
    let sites = room
        .computer_cookies_push(
            &persona_id,
            CookieTransfer {
                source_id: "hl-secrets-e2e".into(),
                browser_id: "chrome".into(),
                profile_id: "Default".into(),
                domains: vec!["example.com".into()],
                cookies: vec![json!({
                    "name": "sid", "value": cookie, "domain": ".example.com", "path": "/",
                    "secure": true, "httpOnly": true,
                })],
            },
        )
        .await
        .expect("Use Cookies reaches the signed-in browser");
    assert_eq!(sites.len(), 1, "{sites:?}");
    let (code, said) = gate
        .sh(&format!(
            // In two halves, so the job's own record of its command is no match.
            "ls ~/.hotline/logins 2>/dev/null; grep -rIl -- \"$(printf %s%s '{}' '{}')\" ~ 2>/dev/null | head -3",
            &cookie[..8],
            &cookie[8..]
        ))
        .await;
    assert!(
        code.is_some() && said.trim().is_empty(),
        "no saved-login file, and the cookie nowhere in the agent's home: {said}"
    );
    let import = room::cookie_imports(&room.log, &persona_id)
        .into_iter()
        .next()
        .expect("the import is recorded");
    let ready = room.computers.running(&persona_id).await.unwrap();
    let name = crate::computer::login::name_for(&import.browser_id, &import.profile_id);
    let forgotten = crate::computer::login::forget(&ready, &name, &[])
        .await
        .expect("taking the share back clears it");
    assert_eq!(
        forgotten.forgotten, 1,
        "the signed-in browser had the cookie"
    );

    // ---- 8. Revoking the key: its stand-in is dead. -----------------------
    // A job already running when the key goes, calling every half second.
    gate.call(
        "shell",
        json!({"action": "start", "label": "Key loop (test only)", "wait_ms": 500, "command": format!(
            r#"while true; do curl -s -o /dev/null -w '%{{http_code}}\n' {}/api/user -H "Authorization: Bearer $GH_TOKEN" >> ~/probe/loop.log; sleep 0.5; done"#,
            fixture.origin
        )}),
    )
    .await;
    eventually("the running loop reaches the service", 20, || {
        let gate = gate.clone();
        async move { gate.sh("grep -c 200 ~/probe/loop.log").await.1.trim() != "0" }
    })
    .await;
    let mut teammate = room.persona(&persona_id).unwrap();
    let stored = vault.shared_secrets().unwrap();
    grants(&mut teammate, &stored, &["READABLE_VAR", "SITE", "SITEKEY"]);
    enrol(&room.log, &teammate);
    room.secrets_changed().await;
    let before = lock(&fixture.seen).len();
    let (code, said) = gate
        .sh(&format!(
            r#"printf '%s ' "${{GH_TOKEN:-none}}"; curl -sS -o /dev/null -w '%{{http_code}}' {}/api/user -H "Authorization: Bearer {stand_in}""#,
            fixture.origin
        ))
        .await;
    say(format!("after revoke: {code:?} {said}"));
    assert!(said.starts_with("none "), "a new job has no key: {said}");
    assert!(
        !said.ends_with("200"),
        "the old stand-in opens nothing: {said}"
    );
    tokio::time::sleep(Duration::from_secs(3)).await;
    let (_, looped) = gate.sh("tail -n 3 ~/probe/loop.log").await;
    say(format!("the running loop after revoke: {looped:?}"));
    assert!(
        !looped.lines().any(|line| line.trim() == "200"),
        "revoking stops a running loop: {looped}"
    );
    assert!(
        lock(&fixture.seen)[before..]
            .iter()
            .all(|auth| *auth != format!("Bearer {key}")),
        "the service never saw the key again"
    );

    // Nothing on the tape, the room or the workspace ever held a value.
    let on_tape = serde_json::to_string(&tape(&room, &persona_id)).unwrap();
    let on_room = std::fs::read_to_string(room.log.root().join("room.jsonl")).unwrap();
    for value in [&key, &password, &readable, &cookie, &seed] {
        assert!(!on_tape.contains(value.as_str()), "a value on the tape");
        assert!(!on_room.contains(value.as_str()), "a value in the room");
    }

    room.stop(&persona_id).unwrap();
    room.computer_remove(&persona_id).await.unwrap();
    let _ = std::fs::remove_dir_all(&root);
    say("passed");
}
