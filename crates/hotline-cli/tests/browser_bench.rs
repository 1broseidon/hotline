//! How fast a server teammate's browser is to watch and drive from a linked
//! desktop, measured through the same bridge the desktop's viewer uses
//! (`remote::bridge`), against a real `hotline serve` with its browser
//! helper: input to the first picture that shows it (a click, a typed
//! character, a wheel notch), pictures a second while the wheel turns,
//! bytes a second idle and scrolling, and the helper's CPU.
//!
//! Two ways in: the snapshot commands (`browser.input` answered with a
//! picture, and `browser.snapshot {fresh}` polled every 400 ms, as the
//! desktop's `RemoteBrowser` and the phone did before the stream), and the
//! stream (`/browser/{persona}/ws`, the computer viewer protocol). Each
//! prints a table; nothing is asserted beyond the page answering, so a slow
//! runner reports slow numbers rather than failing. Run by hand:
//! `cargo test -p hotline-cli --test integration bench_browser -- --ignored --nocapture --test-threads 1`.
#![cfg(target_os = "linux")]

use crate::serve_browser::{Served, helper_of};
use futures_util::{SinkExt, StreamExt};
use hotline_core::remote::bridge::Bridge;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::Message;

/// A page with something to click (a button that recolours a block), a
/// field to type in, and a thousand rows to scroll; nothing on it moves by
/// itself, so an idle page draws nothing.
const PAGE: &str = r#"<!doctype html><html><head><title>Bench</title><style>
body{margin:0;font:16px sans-serif}
#tap{position:absolute;left:100px;top:100px;width:200px;height:60px}
#box{position:absolute;left:400px;top:100px;width:400px;height:300px;background:#c33}
#name{position:absolute;left:100px;top:200px;width:200px;height:30px}
#rows{position:absolute;top:500px;left:0;right:0}
.row{height:40px;border-bottom:1px solid #ccc;padding-left:20px}
</style></head><body>
<button id="tap" onclick="n++;document.getElementById('box').style.background=n%2?'#33c':'#c33';document.title='clicks:'+n">Tap me</button>
<div id="box"></div>
<input id="name" aria-label="Name" oninput="document.title='typed:'+this.value">
<div id="rows"></div>
<script>var n=0;var r=document.getElementById('rows');for(let i=0;i<1000;i++){const d=document.createElement('div');d.className='row';d.textContent='Row '+i+' - the quick brown fox jumps over the lazy dog';r.appendChild(d)}</script>
</body></html>"#;

const TRIES: usize = 12;

/// A web server on this machine answering every request with the page.
fn serve_page() -> u16 {
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
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{PAGE}",
                PAGE.len()
            );
        }
    });
    port
}

/// CPU ticks (user and system) of every process in a process group.
fn group_ticks(group: i32) -> u64 {
    let mut total = 0;
    for entry in std::fs::read_dir("/proc").unwrap().flatten() {
        let pid = entry.file_name().to_string_lossy().to_string();
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        let Some(close) = stat.rfind(')') else {
            continue;
        };
        let fields: Vec<&str> = stat[close + 2..].split(' ').collect();
        if fields.get(2).and_then(|pgid| pgid.parse::<i32>().ok()) == Some(group) {
            let utime: u64 = fields[11].parse().unwrap_or(0);
            let stime: u64 = fields[12].parse().unwrap_or(0);
            total += utime + stime;
        }
    }
    total
}

/// The helper's CPU over a stretch, as a percentage of one core.
struct Cpu {
    group: i32,
    ticks: u64,
    at: Instant,
}

impl Cpu {
    fn start(group: i32) -> Self {
        Self {
            group,
            ticks: group_ticks(group),
            at: Instant::now(),
        }
    }

    fn percent(&self) -> f64 {
        // SAFETY: a constant read.
        let per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
        let used = (group_ticks(self.group) - self.ticks) as f64 / per_second;
        100.0 * used / self.at.elapsed().as_secs_f64()
    }
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn p90(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[(values.len() * 9 / 10).min(values.len() - 1)]
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn row(name: &str, values: &[f64]) {
    eprintln!(
        "  {name:<28} median {:>7.1} ms   p90 {:>7.1} ms   ({} tries)",
        median(values.to_vec()),
        p90(values.to_vec()),
        values.len()
    );
}

/// The wire through the bridge, as the desktop's window holds it: commands
/// with ids, replies routed back, and every byte received counted.
#[derive(Clone)]
struct Wire {
    write: Arc<
        tokio::sync::Mutex<
            futures_util::stream::SplitSink<
                tokio_tungstenite::WebSocketStream<
                    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
                >,
                Message,
            >,
        >,
    >,
    waiting: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    bytes: Arc<AtomicU64>,
    next: Arc<AtomicU64>,
}

impl Wire {
    async fn open(bridge: &Bridge) -> Self {
        let base = bridge.origin.replace("http://", "ws://");
        let (socket, _) =
            tokio_tungstenite::connect_async(format!("{base}/ws?token={}", bridge.token))
                .await
                .unwrap();
        let (write, mut read) = socket.split();
        let wire = Self {
            write: Arc::new(tokio::sync::Mutex::new(write)),
            waiting: Arc::default(),
            bytes: Arc::default(),
            next: Arc::new(AtomicU64::new(1000)),
        };
        let (waiting, bytes) = (wire.waiting.clone(), wire.bytes.clone());
        tokio::spawn(async move {
            while let Some(Ok(message)) = read.next().await {
                let Message::Text(text) = message else {
                    continue;
                };
                bytes.fetch_add(text.len() as u64, Ordering::Relaxed);
                let Ok(value) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                if let Some(id) = value["id"].as_u64()
                    && let Some(answer) = waiting.lock().unwrap().remove(&id)
                {
                    let _ = answer.send(value);
                }
            }
        });
        wire
    }

    async fn ask(&self, cmd: &str, params: Value) -> Value {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = oneshot::channel();
        self.waiting.lock().unwrap().insert(id, answer);
        let frame = json!({"id": id, "cmd": cmd, "params": params});
        self.write
            .lock()
            .await
            .send(Message::text(frame.to_string()))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(60), answered)
            .await
            .unwrap_or_else(|_| panic!("no answer to {cmd}"))
            .unwrap()
    }

    async fn ok(&self, cmd: &str, params: Value) -> Value {
        let reply = self.ask(cmd, params).await;
        assert_eq!(reply["ok"], true, "{cmd}: {reply}");
        reply["result"].clone()
    }
}

/// The teammate on the bench page, the browser taken over through the
/// bridge, and the helper's process group.
fn prepared() -> (Served, Bridge, i32) {
    let mut served = Served::start();
    let page = format!("http://127.0.0.1:{}/", serve_page());
    let opened = served.browse(json!({"action": "navigate", "url": page}));
    assert!(opened.starts_with("browser ok"), "{opened}");
    let group = helper_of(served.server.id()).expect("serve started the helper");
    let bridge = served.runtime.block_on(async {
        Bridge::with_client(&served.desk, served.phone.clone())
            .await
            .unwrap()
    });
    (served, bridge, group)
}

#[test]
#[ignore = "a measurement, by hand: needs hotline-browser and hotline-acp-fixture built beside hotline, WebKitGTK 4.1 and Xvfb"]
fn bench_browser_snapshots() {
    let (served, bridge, group) = prepared();
    let ada = served.ada.clone();
    served.runtime.block_on(async {
        let wire = Wire::open(&bridge).await;
        let view = wire
            .ok("browser.take_over", json!({"personaId": ada}))
            .await;
        assert_eq!(view["phase"], "person", "{view}");
        let input = |input: Value| json!({"personaId": ada, "input": input});

        // A click: the answer is the picture after it.
        let mut clicks = Vec::new();
        for n in 1..=TRIES {
            let started = Instant::now();
            let shot = wire
                .ok(
                    "browser.input",
                    input(json!({"kind": "tap", "x": 200, "y": 130})),
                )
                .await;
            clicks.push(millis(started.elapsed()));
            assert_eq!(shot["title"], format!("clicks:{n}"), "{shot}");
        }
        // Typing, a character at a time, into the field.
        wire.ok(
            "browser.input",
            input(json!({"kind": "tap", "x": 200, "y": 215})),
        )
        .await;
        let mut typed = Vec::new();
        for _ in 0..TRIES {
            let started = Instant::now();
            wire.ok("browser.input", input(json!({"kind": "text", "text": "a"})))
                .await;
            typed.push(millis(started.elapsed()));
        }
        // A wheel notch's worth of scroll.
        let mut scrolled = Vec::new();
        for _ in 0..TRIES {
            let started = Instant::now();
            wire.ok(
                "browser.input",
                input(json!({"kind": "scroll", "dx": 0, "dy": 53, "x": 640, "y": 600})),
            )
            .await;
            scrolled.push(millis(started.elapsed()));
        }

        // Scrolling for five seconds, as the desktop's viewer drives it: the
        // wheel's notches joined while one input is on its way, and a fresh
        // picture asked for every 400 ms beside them.
        let pictures = Arc::new(Mutex::new(std::collections::BTreeSet::new()));
        let window = Duration::from_secs(5);
        let poll = |until: Instant| {
            let (wire, pictures, ada) = (wire.clone(), pictures.clone(), ada.clone());
            tokio::spawn(async move {
                while Instant::now() < until {
                    let started = Instant::now();
                    let shot = wire
                        .ask("browser.snapshot", json!({"personaId": ada, "fresh": true}))
                        .await;
                    if let Some(at) = shot["result"]["at"].as_i64() {
                        pictures.lock().unwrap().insert(at);
                    }
                    tokio::time::sleep(Duration::from_millis(400).saturating_sub(started.elapsed()))
                        .await;
                }
            })
        };
        let bytes_before = wire.bytes.load(Ordering::Relaxed);
        let cpu = Cpu::start(group);
        let until = Instant::now() + window;
        let poller = poll(until);
        let mut last = Instant::now();
        while Instant::now() < until {
            let notches = (last.elapsed().as_secs_f64() * 60.0).max(1.0);
            last = Instant::now();
            let shot = wire
                .ok(
                    "browser.input",
                    input(json!({"kind": "scroll", "dx": 0, "dy": (notches * 53.0) as i64, "x": 640, "y": 600})),
                )
                .await;
            if let Some(at) = shot["at"].as_i64() {
                pictures.lock().unwrap().insert(at);
            }
        }
        poller.await.unwrap();
        let scroll_cpu = cpu.percent();
        let scroll_bytes = wire.bytes.load(Ordering::Relaxed) - bytes_before;
        let scroll_pictures = pictures.lock().unwrap().len();

        // Idle: the person has the browser and nothing moves; the viewer
        // still polls.
        tokio::time::sleep(Duration::from_secs(1)).await;
        pictures.lock().unwrap().clear();
        let idle = Duration::from_secs(10);
        let bytes_before = wire.bytes.load(Ordering::Relaxed);
        let cpu = Cpu::start(group);
        poll(Instant::now() + idle).await.unwrap();
        let idle_cpu = cpu.percent();
        let idle_bytes = wire.bytes.load(Ordering::Relaxed) - bytes_before;

        wire.ok("browser.hand_back", json!({"personaId": ada})).await;
        eprintln!("\nsnapshots (browser.input + browser.snapshot fresh every 400 ms), through the bridge:");
        row("click to picture", &clicks);
        row("typed character to picture", &typed);
        row("wheel notch to picture", &scrolled);
        eprintln!(
            "  scrolling: {:.1} pictures/s, {:.0} kB/s, helper CPU {:.0}%",
            scroll_pictures as f64 / window.as_secs_f64(),
            scroll_bytes as f64 / 1000.0 / window.as_secs_f64(),
            scroll_cpu
        );
        eprintln!(
            "  idle: {:.1} kB/s, helper CPU {:.0}%",
            idle_bytes as f64 / 1000.0 / idle.as_secs_f64(),
            idle_cpu
        );
        let (rss, pss, count) = crate::serve_browser::memory(group);
        eprintln!("  helper: {count} processes, {rss} kB resident, {pss} kB proportional");
    });
    drop(bridge);
    served.stop(Some(group));
}

/// What the stream sent, as it arrived: a frame and the rectangle it
/// covers, or a JSON message, and its size.
#[derive(Clone)]
struct Arrived {
    at: Instant,
    rect: Option<(u16, u16, u16, u16)>,
    text: Option<Value>,
    bytes: usize,
}

type Log = Arc<Mutex<Vec<Arrived>>>;

/// The first frame after `after` that covers some of `region` (any frame
/// when there is none), and when it came.
async fn frame_after(log: &Log, after: Instant, region: Option<(u16, u16, u16, u16)>) -> Instant {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let found = log.lock().unwrap().iter().find_map(|arrived| {
            let (x, y, width, height) = arrived.rect?;
            let covers = region.is_none_or(|(rx, ry, rw, rh)| {
                x < rx + rw && rx < x + width && y < ry + rh && ry < y + height
            });
            (arrived.at >= after && covers).then_some(arrived.at)
        });
        if let Some(at) = found {
            return at;
        }
        assert!(Instant::now() < deadline, "no frame came");
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

/// The frames and bytes that arrived between `from` and `to`.
fn between(log: &Log, from: Instant, to: Instant) -> (usize, usize) {
    let log = log.lock().unwrap();
    let inside = log
        .iter()
        .filter(|arrived| arrived.at >= from && arrived.at < to);
    inside.fold((0, 0), |(frames, bytes), arrived| {
        (
            frames + usize::from(arrived.rect.is_some()),
            bytes + arrived.bytes,
        )
    })
}

#[test]
#[ignore = "a measurement, by hand: needs hotline-browser and hotline-acp-fixture built beside hotline, WebKitGTK 4.1 and Xvfb"]
fn bench_browser_stream() {
    use hotline_core::remote::bridge::Viewer;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let (served, bridge, group) = prepared();
    let ada = served.ada.clone();
    served.runtime.block_on(async {
        let token = bridge.viewer_token(&ada, Viewer::Browser);
        let mut request = format!("{}/browser/{ada}/ws", bridge.origin.replace("http:", "ws:"))
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "sec-websocket-protocol",
            format!("hotline-viewer.{token}").parse().unwrap(),
        );
        let opened = Instant::now();
        let (socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        let (mut write, mut read) = socket.split();
        let log: Log = Arc::default();
        let kept = log.clone();
        tokio::spawn(async move {
            while let Some(Ok(message)) = read.next().await {
                let at = Instant::now();
                let arrived = match message {
                    Message::Binary(bytes) => {
                        let field = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
                        Arrived {
                            at,
                            rect: Some((field(0), field(2), field(4), field(6))),
                            text: None,
                            bytes: bytes.len(),
                        }
                    }
                    Message::Text(text) => Arrived {
                        at,
                        rect: None,
                        text: serde_json::from_str(&text).ok(),
                        bytes: text.len(),
                    },
                    _ => continue,
                };
                kept.lock().unwrap().push(arrived);
            }
        });
        let served = &served;
        let mut say = async |message: Value| {
            if let Err(error) = write.send(Message::text(message.to_string())).await {
                for line in served
                    .log()
                    .lines()
                    .filter(|line| line.contains("[browser]"))
                {
                    eprintln!("serve: {line}");
                }
                panic!("the screen closed: {error}");
            }
        };
        let first = frame_after(&log, opened, None).await;
        let first_bytes = log
            .lock()
            .unwrap()
            .iter()
            .find(|arrived| arrived.rect.is_some())
            .map_or(0, |arrived| arrived.bytes);

        say(json!({"t": "control", "take": true})).await;
        let deadline = Instant::now() + Duration::from_secs(10);
        while !log.lock().unwrap().iter().any(|arrived| {
            arrived
                .text
                .as_ref()
                .is_some_and(|said| said["t"] == "state" && said["driving"] == true)
        }) {
            assert!(Instant::now() < deadline, "never driving");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        // A click on the button, which recolours the block beside it.
        say(json!({"t": "move", "x": 200, "y": 130})).await;
        tokio::time::sleep(Duration::from_millis(400)).await;
        let mut clicks = Vec::new();
        for _ in 0..TRIES {
            let started = Instant::now();
            say(json!({"t": "button", "b": 1, "down": true})).await;
            say(json!({"t": "button", "b": 1, "down": false})).await;
            let shown = frame_after(&log, started, Some((400, 100, 400, 300))).await;
            clicks.push(millis(shown - started));
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        // Typing into the field, a key at a time.
        say(json!({"t": "move", "x": 200, "y": 215})).await;
        say(json!({"t": "button", "b": 1, "down": true})).await;
        say(json!({"t": "button", "b": 1, "down": false})).await;
        tokio::time::sleep(Duration::from_millis(400)).await;
        let mut typed = Vec::new();
        for _ in 0..TRIES {
            let started = Instant::now();
            say(json!({"t": "key", "key": "a", "down": true})).await;
            say(json!({"t": "key", "key": "a", "down": false})).await;
            let shown = frame_after(&log, started, Some((100, 200, 200, 30))).await;
            typed.push(millis(shown - started));
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        // Out of the field, then a wheel notch at a time over the rows.
        say(json!({"t": "move", "x": 60, "y": 450})).await;
        say(json!({"t": "button", "b": 1, "down": true})).await;
        say(json!({"t": "button", "b": 1, "down": false})).await;
        say(json!({"t": "move", "x": 640, "y": 600})).await;
        tokio::time::sleep(Duration::from_millis(400)).await;
        let mut scrolled = Vec::new();
        for _ in 0..TRIES {
            let started = Instant::now();
            say(json!({"t": "wheel", "dy": 1})).await;
            let shown = frame_after(&log, started, None).await;
            scrolled.push(millis(shown - started));
            tokio::time::sleep(Duration::from_millis(250)).await;
        }

        // Scrolling for five seconds: a notch every 16 ms, as a wheel spun
        // steadily turns.
        let window = Duration::from_secs(5);
        let cpu = Cpu::start(group);
        let from = Instant::now();
        while from.elapsed() < window {
            say(json!({"t": "wheel", "dy": 1})).await;
            tokio::time::sleep(Duration::from_millis(16)).await;
        }
        let to = Instant::now();
        let scroll_cpu = cpu.percent();
        let (scroll_frames, scroll_bytes) = between(&log, from, to);

        // Idle: the person has the browser and nothing moves.
        tokio::time::sleep(Duration::from_secs(2)).await;
        let idle = Duration::from_secs(10);
        let cpu = Cpu::start(group);
        let from = Instant::now();
        tokio::time::sleep(idle).await;
        let idle_cpu = cpu.percent();
        let (idle_frames, idle_bytes) = between(&log, from, Instant::now());
        if std::env::var_os("BENCH_IDLE_RECTS").is_some() {
            let log = log.lock().unwrap();
            let mut rects: HashMap<(u16, u16, u16, u16), usize> = HashMap::new();
            for arrived in log.iter().filter(|arrived| arrived.at >= from) {
                if let Some(rect) = arrived.rect {
                    *rects.entry(rect).or_default() += 1;
                }
            }
            eprintln!("  idle frames by rectangle: {rects:?}");
        }

        say(json!({"t": "control", "take": false})).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        eprintln!(
            "\nstream (/browser/{{persona}}/ws, the computer viewer protocol), through the bridge:"
        );
        eprintln!(
            "  first frame (the whole page) {:.0} ms after opening, {:.0} kB",
            millis(first - opened),
            first_bytes as f64 / 1000.0
        );
        row("click to frame", &clicks);
        row("typed character to frame", &typed);
        row("wheel notch to frame", &scrolled);
        eprintln!(
            "  scrolling: {:.1} frames/s, {:.0} kB/s, helper CPU {:.0}%",
            scroll_frames as f64 / window.as_secs_f64(),
            scroll_bytes as f64 / 1000.0 / window.as_secs_f64(),
            scroll_cpu
        );
        eprintln!(
            "  idle: {} frames, {:.2} kB/s, helper CPU {:.0}%",
            idle_frames,
            idle_bytes as f64 / 1000.0 / idle.as_secs_f64(),
            idle_cpu
        );
        let (rss, pss, count) = crate::serve_browser::memory(group);
        eprintln!("  helper: {count} processes, {rss} kB resident, {pss} kB proportional");
    });
    drop(bridge);
    served.stop(Some(group));
}
