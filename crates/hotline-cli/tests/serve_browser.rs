//! A server's teammate browses, and the owner's phone drives its browser,
//! with real processes: `hotline serve` with `hotline-browser` beside it, a
//! teammate on the scripted ACP agent that navigates to a page on this
//! machine, and a phone paired as the owner that allows the address, takes
//! the browser over, taps, types, presses Enter and gives it back, all over
//! the phone's own Noise channel. It needs `hotline-browser` and
//! `hotline-acp-fixture` built beside `hotline`, WebKitGTK 4.1 and Xvfb,
//! so it runs only when asked, as the server-browser CI leg does.
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

/// One command from the phone and its reply.
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
            assert_eq!(reply["ok"], true, "{cmd}: {reply}");
            return reply["result"].clone();
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

#[test]
#[ignore = "needs hotline-browser and hotline-acp-fixture built beside hotline, WebKitGTK 4.1 and Xvfb: the server-browser CI leg runs it"]
fn the_owners_phone_takes_a_server_teammates_browser_over_taps_types_and_gives_it_back() {
    beside("hotline-browser");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("desk");
    std::fs::create_dir_all(root.join("workspace")).unwrap();
    install_fixture(&root);
    let page = format!("http://127.0.0.1:{}/", serve_form());
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

    // A teammate with no computer, on the scripted agent.
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

    // The owner's phone, paired over the server's own channel.
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
    let mut session = runtime.block_on(async { phone.connect(&desk) });

    // The teammate navigates to a page on this machine; the phone allows it.
    let prompt = format!(
        "CALL browser {}",
        json!({"action": "navigate", "url": page})
    );
    let prompted = wire(
        &root,
        "session.prompt",
        &json!({"personaId": ada, "text": prompt}).to_string(),
    );
    assert!(
        prompted.status.success(),
        "{}",
        String::from_utf8_lossy(&prompted.stderr)
    );
    let mut card = Value::Null;
    until("the browser's local address card is up", || {
        card = tape(&root, &ada)
            .into_iter()
            .rfind(|event| {
                event["kind"] == "permission"
                    && event["requestId"]
                        .as_str()
                        .is_some_and(|id| id.starts_with("browser:"))
            })
            .unwrap_or(Value::Null);
        !card.is_null()
    });
    let mut next = 0;
    let mut call = |session: &mut Session, cmd: &str, params: Value| {
        next += 1;
        runtime.block_on(ask(session, next, cmd, params))
    };
    call(
        &mut session,
        "session.answer_permission",
        json!({"personaId": ada, "requestId": card["requestId"], "optionId": "allow"}),
    );
    until("the teammate's navigate answers", || {
        tape(&root, &ada).iter().any(|event| {
            event["kind"] == "agent"
                && event["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("browser ok"))
        })
    });
    let group = helper_of(server.id()).expect("serve started the helper");
    let (rss, pss, count) = memory(group);
    eprintln!(
        "the helper with one page, through serve: {count} processes, {rss} kB resident, {pss} kB proportional"
    );

    // Take control, tap, type, press Enter, and look.
    let view = call(&mut session, "browser.take_over", json!({"personaId": ada}));
    assert_eq!(view["phase"], "person", "{view}");
    let tap = |x: f64, y: f64| json!({"personaId": ada, "input": {"kind": "tap", "x": x, "y": y}});
    let shot = call(&mut session, "browser.input", tap(200.0, 130.0));
    assert_eq!(shot["title"], "tapped", "{shot}");
    call(&mut session, "browser.input", tap(250.0, 215.0));
    let typed = json!({"personaId": ada, "input": {"kind": "text", "text": "Ada"}});
    assert_eq!(
        call(&mut session, "browser.input", typed)["title"],
        "typed:Ada"
    );
    let enter = json!({"personaId": ada, "input": {"kind": "key", "key": "Enter"}});
    assert_eq!(
        call(&mut session, "browser.input", enter)["title"],
        "submitted:Ada"
    );
    let fresh = call(
        &mut session,
        "browser.snapshot",
        json!({"personaId": ada, "fresh": true}),
    );
    assert_eq!(fresh["size"], json!({"width": 1280, "height": 800}));
    assert_eq!(fresh["url"], page);
    assert!(
        fresh["jpeg"]
            .as_str()
            .is_some_and(|jpeg| jpeg.starts_with("/9j/")),
        "a JPEG"
    );

    // Give it back: the teammate drives again and reads what the person did.
    let view = call(&mut session, "browser.hand_back", json!({"personaId": ada}));
    assert_eq!(view["phase"], "teammate", "{view}");
    let read =
        json!({"personaId": ada, "text": format!("CALL browser {}", json!({"action": "text"}))});
    assert!(
        wire(&root, "session.prompt", &read.to_string())
            .status
            .success()
    );
    until("the teammate reads the page the person typed in", || {
        tape(&root, &ada).iter().any(|event| {
            event["kind"] == "agent"
                && event["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("submitted:Ada"))
        })
    });

    // Stopped, the server takes its browser with it.
    drop(session);
    stop(server);
    until("the helper and everything it started are gone", || {
        // SAFETY: a signal check of the helper's group, sending nothing.
        let any = unsafe { libc::kill(-group, 0) };
        any != 0
    });
}
