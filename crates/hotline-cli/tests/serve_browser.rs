//! A server's teammate browses, and the owner's phone drives its browser,
//! with real processes: `hotline serve` with `hotline-browser` beside it, a
//! teammate on the scripted ACP agent that navigates to a page on this
//! machine, and a phone paired as the owner that allows the address, takes
//! the browser over, taps, types, presses Enter and gives it back, all over
//! the phone's own Noise channel; visits past a certificate the browser
//! refused; and a helper killed again and again, which is started again
//! only once the wait its stops left is over. It needs `hotline-browser` and
//! `hotline-acp-fixture` built beside `hotline`, WebKitGTK 4.1, Xvfb and
//! `openssl`, so it runs only when asked, as the server-browser CI leg does.
#![cfg(target_os = "linux")]
use crate::common;

use common::{PATIENCE, free_address, hotline, listen_args, stop, until, wire};
use hotline_core::credentials::FileStore;
use hotline_core::remote::PairingPayload;
use hotline_core::remote::client::{Client, Session};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

const FORM: &str = r#"<!doctype html><html><head><title>Form</title><style>
body{margin:0;font:16px sans-serif}
#tap{position:absolute;left:100px;top:100px;width:200px;height:60px}
#name{position:absolute;left:100px;top:200px;width:300px;height:30px}
</style></head><body>
<button id="tap" onclick="document.title='tapped'">Tap me</button>
<form onsubmit="event.preventDefault();document.title='submitted:'+document.getElementById('name').value">
<input id="name" aria-label="Name" oninput="document.title='typed:'+this.value"></form>
</body></html>"#;

/// A binary built beside `hotline`.
fn beside(name: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_BIN_EXE_hotline")).with_file_name(name);
    assert!(path.is_file(), "needs {}: build it first", path.display());
    path
}

/// The scripted ACP agent in `root`'s catalogue, as a catalogue agent
/// unpacked from its archive is (hotline-core's `tests/common/acp_fixture.rs`).
fn install_fixture(root: &Path) {
    use sha2::Digest;
    let archive = "https://example.invalid/acp-fixture.tar.gz";
    let platform = format!("linux-{}", std::env::consts::ARCH);
    let catalogue = json!({
        "format": 2,
        "fetchedAt": chrono::Utc::now().timestamp_millis(),
        "agents": [{
            "id": "acp-fixture",
            "name": "ACP Fixture",
            "distribution": {"binary": {platform: {"archive": archive, "cmd": "./hotline-acp-fixture"}}},
        }],
    });
    std::fs::create_dir_all(root.join("cache")).unwrap();
    std::fs::write(root.join("cache/acp-registry.json"), catalogue.to_string()).unwrap();
    let folder = hex::encode(&sha2::Sha256::digest(archive.as_bytes())[..8]);
    let dir = root.join("acp-agents/acp-fixture").join(folder);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(
        beside("hotline-acp-fixture"),
        dir.join("hotline-acp-fixture"),
    )
    .unwrap();
}

/// A web server on this machine answering every request with the form.
fn serve_form() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut socket in listener.incoming().flatten() {
            let mut asked: Vec<u8> = Vec::new();
            let mut buffer = [0u8; 1024];
            while !asked.windows(4).any(|end| end == b"\r\n\r\n") {
                match socket.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => asked.extend(&buffer[..read]),
                }
            }
            let _ = write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{FORM}",
                FORM.len()
            );
        }
    });
    port
}

/// Every event on the teammate's tape so far, read from its files.
fn tape(root: &Path, persona: &str) -> Vec<Value> {
    let mut events = Vec::new();
    for entry in std::fs::read_dir(root.join("transcripts").join(persona))
        .into_iter()
        .flatten()
        .flatten()
    {
        let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
        events.extend(
            text.lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok()),
        );
    }
    events
}

/// One command from the phone and the desk's whole reply to it.
async fn ask(session: &mut Session, id: u64, cmd: &str, params: Value) -> Value {
    let frame = json!({"id": id, "cmd": cmd, "params": params});
    session.outgoing.send(frame.to_string()).await.unwrap();
    loop {
        let text = tokio::time::timeout(PATIENCE, session.incoming.recv())
            .await
            .unwrap_or_else(|_| panic!("no answer to {cmd}"))
            .expect("the phone's channel closed");
        let reply: Value = serde_json::from_str(&text).unwrap();
        if reply["id"] == id {
            return reply;
        }
    }
}

/// The resident and proportional memory of the helper's process group, in
/// kB, and how many processes it holds.
fn memory(group: i32) -> (u64, u64, usize) {
    let field = |pid: &str, file: &str, key: &str| -> u64 {
        std::fs::read_to_string(format!("/proc/{pid}/{file}"))
            .ok()
            .and_then(|text| {
                let line = text.lines().find(|line| line.starts_with(key))?.to_string();
                line.split_whitespace().nth(1)?.parse().ok()
            })
            .unwrap_or(0)
    };
    let mut total = (0, 0, 0);
    for entry in std::fs::read_dir("/proc").unwrap().flatten() {
        let pid = entry.file_name().to_string_lossy().to_string();
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        let Some(close) = stat.rfind(')') else {
            continue;
        };
        let pgid: i32 = stat[close + 2..]
            .split(' ')
            .nth(2)
            .unwrap()
            .parse()
            .unwrap();
        if pgid == group {
            total.0 += field(&pid, "status", "VmRSS:");
            total.1 += field(&pid, "smaps_rollup", "Pss:");
            total.2 += 1;
        }
    }
    total
}

/// The helper `hotline serve` started: its reaper, a child of the server.
fn helper_of(server: u32) -> Option<i32> {
    std::fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .find_map(|entry| {
            let pid: i32 = entry.file_name().to_string_lossy().parse().ok()?;
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            let name = &stat[stat.find('(')? + 1..stat.rfind(')')?];
            let parent: u32 = stat[stat.rfind(')')? + 2..]
                .split(' ')
                .nth(1)?
                .parse()
                .ok()?;
            (name == "hotline-browser" && parent == server).then_some(pid)
        })
}

/// A served desk with Ada, a teammate with no computer on the scripted
/// agent, and the owner's phone paired over the desk's own channel.
struct Served {
    dir: tempfile::TempDir,
    root: PathBuf,
    server: std::process::Child,
    ada: String,
    runtime: tokio::runtime::Runtime,
    session: Session,
    next: u64,
}

impl Served {
    fn start() -> Self {
        beside("hotline-browser");
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("desk");
        std::fs::create_dir_all(root.join("workspace")).unwrap();
        install_fixture(&root);
        let address = free_address();
        let log = dir.path().join("serve.log");
        let mut args = vec!["serve".to_string(), "--store".into(), "file".into()];
        args.extend(listen_args(&address));
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let server = hotline(&root, &args)
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log).unwrap())
            .spawn()
            .unwrap();
        until("the server writes door.json", || {
            root.join("door.json").exists()
        });
        let said = std::fs::read_to_string(&log).unwrap();
        assert!(said.contains("Browser: on"), "{said}");

        let created = wire(
            &root,
            "persona.create",
            &json!({"draft": {"name": "Ada", "goal": "Fill in forms.", "cwd": root.join("workspace"), "backendId": "acp-fixture"}}).to_string(),
        );
        assert!(
            created.status.success(),
            "{}",
            String::from_utf8_lossy(&created.stderr)
        );
        let ada = serde_json::from_slice::<Value>(&created.stdout).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        let persona = json!({"personaId": ada}).to_string();
        assert!(wire(&root, "session.start", &persona).status.success());

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut pairing = hotline(&root, &["pair", "--json"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(pairing.stdout.as_mut().unwrap())
            .read_line(&mut line)
            .unwrap();
        let invitation: PairingPayload = serde_json::from_str(line.trim()).unwrap();
        let phone_root = dir.path().join("phone");
        std::fs::create_dir_all(&phone_root).unwrap();
        let phone = Client::new(Arc::new(FileStore::open(&phone_root).unwrap()));
        let desk = runtime.block_on(phone.pair(&invitation, "Phone")).unwrap();
        let _ = pairing.wait();
        let session = runtime.block_on(async { phone.connect(&desk) });
        Self {
            dir,
            root,
            server,
            ada,
            runtime,
            session,
            next: 0,
        }
    }

    /// What the desk wrote to its log so far.
    fn log(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("serve.log")).unwrap_or_default()
    }

    /// The phone's command and the desk's whole reply.
    fn reply(&mut self, cmd: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        self.runtime
            .block_on(ask(&mut self.session, id, cmd, params))
    }

    /// The phone's command, which must succeed, and its result.
    fn call(&mut self, cmd: &str, params: Value) -> Value {
        let reply = self.reply(cmd, params);
        assert_eq!(reply["ok"], true, "{cmd}: {reply}");
        reply["result"].clone()
    }

    /// Ada's own `browser` call, and what the agent said of its answer:
    /// `browser ok: …` or `browser refused: …`. A local address's card, if
    /// one comes up, is allowed from the phone first.
    fn browse(&mut self, arguments: Value) -> String {
        let said_before = self.browser_lines().len();
        let cards_before = self.cards().len();
        let prompt = format!("CALL browser {arguments}");
        let prompted = wire(
            &self.root,
            "session.prompt",
            &json!({"personaId": self.ada, "text": prompt}).to_string(),
        );
        assert!(
            prompted.status.success(),
            "{}",
            String::from_utf8_lossy(&prompted.stderr)
        );
        let mut card = Value::Null;
        until("the browser answers, or a card comes up", || {
            if let Some(new) = self.cards().into_iter().nth(cards_before) {
                card = new;
            }
            !card.is_null() || self.browser_lines().len() > said_before
        });
        if !card.is_null() {
            let (ada, request) = (self.ada.clone(), card["requestId"].clone());
            self.call(
                "session.answer_permission",
                json!({"personaId": ada, "requestId": request, "optionId": "allow"}),
            );
        }
        until("the browser answers", || {
            self.browser_lines().len() > said_before
        });
        self.browser_lines()[said_before].clone()
    }

    /// What the agent said of each `browser` answer, in order: the scripted
    /// agent's turn is `RESULTS` and then its one call's answer.
    fn browser_lines(&self) -> Vec<String> {
        tape(&self.root, &self.ada)
            .into_iter()
            .filter(|event| event["kind"] == "agent")
            .filter_map(|event| {
                let text = event["text"].as_str()?;
                text.strip_prefix("RESULTS\n")
                    .filter(|answer| answer.starts_with("browser "))
                    .map(str::to_string)
            })
            .collect()
    }

    /// The browser's local address cards so far.
    fn cards(&self) -> Vec<Value> {
        tape(&self.root, &self.ada)
            .into_iter()
            .filter(|event| {
                event["kind"] == "permission"
                    && event["requestId"]
                        .as_str()
                        .is_some_and(|id| id.starts_with("browser:"))
            })
            .collect()
    }

    /// Stops the desk; its browser goes with it. What the browser wrote to
    /// the desk's log is shown, for a run that needs reading.
    fn stop(self, group: Option<i32>) {
        for line in self.log().lines().filter(|line| line.contains("[browser]")) {
            eprintln!("serve: {line}");
        }
        drop(self.session);
        stop(self.server);
        if let Some(group) = group {
            until("the helper and everything it started are gone", || {
                // SAFETY: a signal check of the helper's group, sending nothing.
                let any = unsafe { libc::kill(-group, 0) };
                any != 0
            });
        }
    }
}

#[test]
#[ignore = "needs hotline-browser and hotline-acp-fixture built beside hotline, WebKitGTK 4.1 and Xvfb: the server-browser CI leg runs it"]
fn the_owners_phone_takes_a_server_teammates_browser_over_taps_types_and_gives_it_back() {
    let mut served = Served::start();
    let page = format!("http://127.0.0.1:{}/", serve_form());
    let ada = served.ada.clone();

    // The teammate navigates to a page on this machine; the phone allows it.
    let opened = served.browse(json!({"action": "navigate", "url": page}));
    assert!(opened.starts_with("browser ok"), "{opened}");
    let group = helper_of(served.server.id()).expect("serve started the helper");
    let (rss, pss, count) = memory(group);
    eprintln!(
        "the helper with one page, through serve: {count} processes, {rss} kB resident, {pss} kB proportional"
    );

    // Take control, tap, type, press Enter, and look.
    let view = served.call("browser.take_over", json!({"personaId": ada}));
    assert_eq!(view["phase"], "person", "{view}");
    let tap = |x: f64, y: f64| json!({"personaId": ada, "input": {"kind": "tap", "x": x, "y": y}});
    let shot = served.call("browser.input", tap(200.0, 130.0));
    assert_eq!(shot["title"], "tapped", "{shot}");
    served.call("browser.input", tap(250.0, 215.0));
    let typed = json!({"personaId": ada, "input": {"kind": "text", "text": "Ada"}});
    assert_eq!(served.call("browser.input", typed)["title"], "typed:Ada");
    let enter = json!({"personaId": ada, "input": {"kind": "key", "key": "Enter"}});
    assert_eq!(
        served.call("browser.input", enter)["title"],
        "submitted:Ada"
    );
    let fresh = served.call("browser.snapshot", json!({"personaId": ada, "fresh": true}));
    assert_eq!(fresh["size"], json!({"width": 1280, "height": 800}));
    assert_eq!(fresh["url"], page);
    assert!(
        fresh["jpeg"]
            .as_str()
            .is_some_and(|jpeg| jpeg.starts_with("/9j/")),
        "a JPEG"
    );

    // Give it back: the teammate drives again and reads what the person did.
    let view = served.call("browser.hand_back", json!({"personaId": ada}));
    assert_eq!(view["phase"], "teammate", "{view}");
    let read = served.browse(json!({"action": "text"}));
    assert!(read.contains("submitted:Ada"), "{read}");

    // Stopped, the server takes its browser with it.
    served.stop(Some(group));
}

/// A page on this machine behind a certificate nobody vouches for, served
/// by `openssl s_server`, which this test stops.
struct Untrusted {
    server: std::process::Child,
    port: u16,
    _dir: tempfile::TempDir,
}

impl Untrusted {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (key, cert) = (dir.path().join("key.pem"), dir.path().join("cert.pem"));
        let made = std::process::Command::new("openssl")
            .args([
                "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
            ])
            .args(["-subj", "/CN=untrusted.example", "-keyout"])
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("needs openssl");
        assert!(made.success());
        let port = free_address().port();
        let server = std::process::Command::new("openssl")
            .args(["s_server", "-quiet", "-www", "-accept"])
            .arg(port.to_string())
            .arg("-cert")
            .arg(&cert)
            .arg("-key")
            .arg(&key)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        until("openssl answers", || {
            std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
        });
        Self {
            server,
            port,
            _dir: dir,
        }
    }
}

impl Drop for Untrusted {
    fn drop(&mut self) {
        let _ = self.server.kill();
        let _ = self.server.wait();
    }
}

/// A certificate the browser refused stays refused for the teammate, whose
/// page and answer say the person can visit it anyway from the phone; the
/// owner's phone can, only once it has the browser, and the page then loads
/// for the teammate too.
#[test]
#[ignore = "needs hotline-browser and hotline-acp-fixture built beside hotline, WebKitGTK 4.1, Xvfb and openssl: the server-browser CI leg runs it"]
fn the_owners_phone_visits_a_server_teammates_page_past_a_refused_certificate() {
    let untrusted = Untrusted::start();
    let mut served = Served::start();
    let ada = served.ada.clone();
    let page = format!("https://127.0.0.1:{}/", untrusted.port);

    let refused = served.browse(json!({"action": "navigate", "url": page}));
    assert!(refused.starts_with("browser refused"), "{refused}");
    assert!(refused.contains("security certificate"), "{refused}");
    assert!(
        refused.contains("from their phone or a linked desktop"),
        "{refused}"
    );
    let warning = served.browse(json!({"action": "text"}));
    assert!(
        warning
            .contains("take control from your phone or a linked desktop and choose Visit anyway"),
        "{warning}"
    );
    assert!(!warning.contains("above"), "{warning}");

    let early = served.reply("browser.visit_anyway", json!({"personaId": ada}));
    assert_eq!(early["ok"], false, "{early}");
    assert!(
        early["error"].to_string().contains("Take over first"),
        "{early}"
    );

    served.call("browser.take_over", json!({"personaId": ada}));
    let visited = served.call("browser.visit_anyway", json!({"personaId": ada}));
    assert_eq!(visited["url"], page, "{visited}");
    let again = served.reply("browser.visit_anyway", json!({"personaId": ada}));
    assert_eq!(again["ok"], false, "nothing left to visit past: {again}");
    served.call("browser.hand_back", json!({"personaId": ada}));

    // `s_server -www` answers a page of its own TLS session.
    let read = served.browse(json!({"action": "text"}));
    assert!(read.contains("s_server"), "{read}");
    let group = helper_of(served.server.id());
    served.stop(group);
}

/// A helper killed as soon as it is up is not started again at the next
/// page: the teammate is told how long it waits, each kill in a row doubles
/// the wait, nothing is started meanwhile, and once the wait is over the
/// next page starts a new one.
#[test]
#[ignore = "needs hotline-browser and hotline-acp-fixture built beside hotline, WebKitGTK 4.1 and Xvfb: the server-browser CI leg runs it"]
fn a_server_helper_that_keeps_dying_is_started_again_only_after_its_wait() {
    let mut served = Served::start();
    let page = format!("http://127.0.0.1:{}/", serve_form());
    let navigate = json!({"action": "navigate", "url": page});
    let opened = served.browse(navigate.clone());
    assert!(opened.starts_with("browser ok"), "{opened}");

    // Killed each time it is up, until a page comes before its wait is
    // over: each kill doubles the wait, so that is soon, however slow the
    // prompt's way to the browser is.
    let mut kills = 0;
    let waiting = loop {
        let helper = helper_of(served.server.id()).expect("a helper runs");
        // SAFETY: SIGKILL to the helper serve started, by the pid read for it.
        unsafe { libc::kill(helper, libc::SIGKILL) };
        kills += 1;
        until("the killed helper is gone", || {
            helper_of(served.server.id()) != Some(helper)
        });
        let answer = served.browse(navigate.clone());
        if answer.contains("waits") {
            break answer;
        }
        assert!(answer.starts_with("browser ok"), "{answer}");
        assert!(kills < 8, "never refused after {kills} kills: {answer}");
    };
    assert!(
        waiting.contains("Your browser stopped again soon after it started"),
        "{waiting}"
    );
    assert_eq!(helper_of(served.server.id()), None, "nothing started");
    let seconds: u64 = waiting
        .split("waits ")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|number| number.parse().ok())
        .unwrap_or_else(|| panic!("how long: {waiting}"));
    eprintln!("refused after {kills} kills, waiting {seconds} s");
    assert!(seconds <= 1 << (kills - 1), "{waiting}");

    std::thread::sleep(std::time::Duration::from_secs(seconds));
    let opened = served.browse(navigate);
    assert!(opened.starts_with("browser ok"), "{opened}");
    let group = helper_of(served.server.id()).expect("a new helper runs");
    let log = served.log();
    assert!(log.contains("[browser] the helper"), "{log}");
    served.stop(Some(group));
}
