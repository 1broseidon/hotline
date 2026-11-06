//! The server's browser for real: `hotline-browser` started by the desk's
//! end of the pipe, WebKitGTK drawing on its own Xvfb, driven as a teammate
//! and as the person on a phone drive it. It needs the helper built
//! (`cargo build -p hotline-browser`, or `HOTLINE_BROWSER_HELPER` naming
//! it), WebKitGTK 4.1 and Xvfb, so it runs only when asked, as the
//! server-browser CI leg does.
#![cfg(target_os = "linux")]

use hotline_core::browser::helper::Helper;
use hotline_core::browser::{Act, DeskBrowser, Gate, Reading};
use hotline_core::contract::BrowserInput;
use hotline_core::log::Log;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use url::Url;

use crate::common::until;

const STOPPED: &str = "Your browser stopped, and its page with it. Navigate to open a new one.";

/// The helper beside this test's binaries, or where the environment says.
fn program() -> PathBuf {
    let path = std::env::var_os("HOTLINE_BROWSER_HELPER").map_or_else(
        || {
            let exe = std::env::current_exe().unwrap();
            exe.parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("hotline-browser")
        },
        PathBuf::from,
    );
    assert!(
        path.is_file(),
        "needs {}: cargo build -p hotline-browser",
        path.display()
    );
    path
}

/// A form at known places, a link to the desk's own port, a link that opens
/// a popup, and a page long enough to scroll.
fn form(own: u16) -> String {
    format!(
        r#"<!doctype html><html><head><title>Form</title><style>
body{{margin:0;font:16px sans-serif}}
#tap{{position:absolute;left:100px;top:100px;width:200px;height:60px}}
#name{{position:absolute;left:100px;top:200px;width:300px;height:30px}}
#links{{position:absolute;left:100px;top:300px}}
</style></head><body>
<button id="tap" onclick="document.title='tapped'">Tap me</button>
<form onsubmit="event.preventDefault();document.title='submitted:'+document.getElementById('name').value">
<input id="name" name="name" aria-label="Name" oninput="document.title='typed:'+this.value"></form>
<p id="links"><a href="http://127.0.0.1:{own}/">Own port</a> <a href="/popup" target="_blank">Pop</a></p>
<div style="height:3000px"></div></body></html>"#
    )
}

const POPUP: &str =
    r#"<!doctype html><title>Popup</title><button onclick="window.close()">Close</button>"#;

/// A web server on loopback answering every request with `/popup`'s page
/// or the form.
async fn serve(own: u16) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let page = form(own);
            tokio::spawn(async move {
                let mut asked = Vec::new();
                let mut buffer = [0u8; 1024];
                while !asked.windows(4).any(|end| end == b"\r\n\r\n") {
                    match socket.read(&mut buffer).await {
                        Ok(0) | Err(_) => return,
                        Ok(read) => asked.extend(&buffer[..read]),
                    }
                }
                let body = if asked.starts_with(b"GET /popup") {
                    POPUP.to_string()
                } else {
                    page
                };
                let answer = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(answer.as_bytes()).await;
            });
        }
    });
    port
}

/// The ref a text read gave the control named `name`.
fn ref_of(text: &str, name: &str) -> String {
    let quoted = format!("\"{name}\" ");
    let at = text
        .find(&quoted)
        .unwrap_or_else(|| panic!("no {name} in {text}"))
        + quoted.len();
    text[at..].split([' ', ']']).next().unwrap().to_string()
}

/// A picture's size, and whether it shows more than one colour.
fn looked(png: &[u8]) -> (u32, u32, bool) {
    let image = image::load_from_memory(png).unwrap().to_rgb8();
    let first = *image.get_pixel(0, 0);
    let varied = image.pixels().any(|pixel| *pixel != first);
    (image.width(), image.height(), varied)
}

/// Every process of this test's own helpers: (pid, parent, group, name).
fn processes() -> Vec<(i32, i32, i32, String)> {
    let mut all = Vec::new();
    for entry in std::fs::read_dir("/proc").unwrap().flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        let (Some(open), Some(close)) = (stat.find('('), stat.rfind(')')) else {
            continue;
        };
        let fields: Vec<&str> = stat[close + 2..].split(' ').collect();
        all.push((
            pid,
            fields[1].parse().unwrap(),
            fields[2].parse().unwrap(),
            stat[open + 1..close].to_string(),
        ));
    }
    all
}

/// The helper this test started: its reaper, a child of this process.
fn reaper() -> i32 {
    let me = std::process::id() as i32;
    let reapers: Vec<i32> = processes()
        .into_iter()
        .filter(|(_, parent, _, name)| *parent == me && name == "hotline-browser")
        .map(|(pid, ..)| pid)
        .collect();
    assert_eq!(reapers.len(), 1, "{reapers:?}");
    reapers[0]
}

/// The memory of everything in the helper's process group, in kB: resident
/// and proportional, summed, with the processes counted.
fn memory(group: i32) -> (u64, u64, usize) {
    let read = |pid: i32, file: &str, key: &str| -> u64 {
        std::fs::read_to_string(format!("/proc/{pid}/{file}"))
            .ok()
            .and_then(|text| {
                text.lines()
                    .find(|line| line.starts_with(key))
                    .and_then(|line| line.split_whitespace().nth(1)?.parse().ok())
            })
            .unwrap_or(0)
    };
    let members: Vec<i32> = processes()
        .into_iter()
        .filter(|(_, _, pgid, _)| *pgid == group)
        .map(|(pid, ..)| pid)
        .collect();
    let rss = members
        .iter()
        .map(|pid| read(*pid, "status", "VmRSS:"))
        .sum();
    let pss = members
        .iter()
        .map(|pid| read(*pid, "smaps_rollup", "Pss:"))
        .sum();
    (rss, pss, members.len())
}

/// Whether an X client with no cookie is let onto any display listening on
/// the abstract socket namespace: the connection setup's first byte is 1
/// only for a client the server accepts.
fn any_display_takes_a_stranger() -> Option<bool> {
    use std::io::{Read, Write};
    use std::os::linux::net::SocketAddrExt;
    let sockets = std::fs::read_to_string("/proc/net/unix").unwrap();
    let mut seen = None;
    for name in sockets
        .lines()
        .filter_map(|line| line.split_whitespace().nth(7))
    {
        let Some(abstract_name) = name
            .strip_prefix('@')
            .filter(|name| name.starts_with("/tmp/.X11-unix/X"))
        else {
            continue;
        };
        let address = std::os::unix::net::SocketAddr::from_abstract_name(abstract_name).unwrap();
        let Ok(mut socket) = std::os::unix::net::UnixStream::connect_addr(&address) else {
            continue;
        };
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        socket
            .write_all(&[b'l', 0, 11, 0, 0, 0, 0, 0, 0, 0, 0, 0])
            .unwrap();
        let mut answer = [0u8; 1];
        socket.read_exact(&mut answer).unwrap();
        seen = Some(seen.unwrap_or(false) || answer[0] == 1);
    }
    seen
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs hotline-browser built, WebKitGTK 4.1 and Xvfb: the server-browser CI leg runs it"]
async fn the_server_browser_loads_pictures_and_takes_the_teammates_and_the_persons_input() {
    let root = tempfile::tempdir().unwrap();
    let own = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let own_port = own.local_addr().unwrap().port();
    let gate = Arc::new(Gate::default());
    gate.load(&Log::open(root.path()), &[own_port]);
    let helper = Helper::new(program(), root.path().to_path_buf(), gate);
    let port = serve(own_port).await;
    let url = Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap();

    // The teammate's navigate: loaded, where the desk's quick read says.
    let page = helper.open("ada", &url).await.unwrap();
    assert_eq!((page.title.as_str(), page.loaded), ("Form", true));
    assert_eq!(helper.current("ada").as_deref(), Some(url.as_str()));
    let (width, height, varied) = looked(&helper.screenshot("ada").await.unwrap());
    assert_eq!((width, height), (1280, 800));
    assert!(varied, "the picture is one flat colour");
    let group = reaper();
    let (rss, pss, count) = memory(group);
    eprintln!("memory with one page: {count} processes, {rss} kB resident, {pss} kB proportional");

    // Nobody without the cookie gets onto its display.
    assert_eq!(any_display_takes_a_stranger(), Some(false));

    // The person's tap, typing and key, as the phone sends them.
    helper
        .person_input("ada", &BrowserInput::Tap { x: 200.0, y: 130.0 })
        .await
        .unwrap();
    assert_eq!(helper.read("ada", Reading::Title).await.unwrap(), "tapped");
    helper
        .person_input("ada", &BrowserInput::Tap { x: 250.0, y: 215.0 })
        .await
        .unwrap();
    helper
        .person_input("ada", &BrowserInput::Text { text: "Ada".into() })
        .await
        .unwrap();
    assert_eq!(
        helper.read("ada", Reading::Title).await.unwrap(),
        "typed:Ada"
    );
    helper
        .person_input(
            "ada",
            &BrowserInput::Key {
                key: "Enter".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        helper.read("ada", Reading::Title).await.unwrap(),
        "submitted:Ada"
    );
    helper
        .person_input(
            "ada",
            &BrowserInput::Scroll {
                dx: 0.0,
                dy: 500.0,
                x: None,
                y: None,
            },
        )
        .await
        .unwrap();

    // The teammate's acts on refs from its read.
    let text = helper.read("ada", Reading::Text).await.unwrap();
    let name = ref_of(&text, "Name");
    helper
        .act(
            "ada",
            &Act::Fill {
                target: name,
                text: "Grace".into(),
            },
        )
        .await
        .unwrap();
    helper
        .act(
            "ada",
            &Act::Press {
                key: "Enter".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        helper.read("ada", Reading::Title).await.unwrap(),
        "submitted:Grace"
    );

    // A link to the desk's own port never opens: the desk's gate said so.
    let refused = helper
        .act(
            "ada",
            &Act::Click {
                target: ref_of(&text, "Own port"),
            },
        )
        .await
        .unwrap_err();
    assert!(refused.contains("never opens"), "{refused}");
    assert_eq!(helper.current("ada").as_deref(), Some(url.as_str()));

    // A popup takes the teammate's reads until it closes itself.
    let opened = helper
        .act(
            "ada",
            &Act::Click {
                target: ref_of(&text, "Pop"),
            },
        )
        .await
        .unwrap();
    assert!(opened.note.unwrap_or_default().contains("opened a window"));
    until("the popup has loaded", || {
        helper
            .current("ada")
            .is_some_and(|at| at.ends_with("/popup"))
    })
    .await;
    let popup = helper.read("ada", Reading::Text).await.unwrap();
    let closed = helper
        .act(
            "ada",
            &Act::Click {
                target: ref_of(&popup, "Close"),
            },
        )
        .await
        .unwrap();
    assert!(closed.note.unwrap_or_default().contains("window closed"));
    assert_eq!(
        helper.read("ada", Reading::Title).await.unwrap(),
        "submitted:Grace"
    );

    // A phone's size, and the whole page from the top.
    helper.viewport("ada", Some((390, 844))).await.unwrap();
    assert_eq!(helper.size("ada"), Some((390, 844)));
    assert_eq!(helper.page_size("ada").await, Some((390, 844)));
    let (width, height, _) = looked(&helper.screenshot("ada").await.unwrap());
    assert_eq!((width, height), (390, 844));
    helper.viewport("ada", None).await.unwrap();
    let whole = helper.full_page("ada", 2).await.unwrap();
    assert_eq!((whole.pictures.len(), whole.more), (2, true));

    // A second teammate has a browser of its own beside the first.
    helper.open("kit", &url).await.unwrap();
    assert_eq!(helper.read("kit", Reading::Title).await.unwrap(), "Form");
    let (rss, pss, count) = memory(group);
    eprintln!(
        "memory with two teammates: {count} processes, {rss} kB resident, {pss} kB proportional"
    );

    // The browser dies: the call is told, and what it left is reaped.
    let browser = processes()
        .into_iter()
        .find(|(_, parent, _, name)| *parent == group && name == "hotline-browser")
        .map(|(pid, ..)| pid)
        .expect("the reaper's browser");
    // SAFETY: a process of this test's own helper, found by its parent.
    assert_eq!(unsafe { libc::kill(browser, libc::SIGKILL) }, 0);
    until("the desk hears the browser stop", || {
        helper.current("ada").is_none()
    })
    .await;
    assert_eq!(
        helper.read("ada", Reading::Title).await,
        Err(STOPPED.into())
    );
    until("everything the helper started is gone and reaped", || {
        // SAFETY: a signal check, sending nothing.
        let any = unsafe { libc::kill(-group, 0) };
        any != 0
    })
    .await;

    // The next page starts a helper again; a wipe takes the storage.
    helper.open("ada", &url).await.unwrap();
    assert_eq!(helper.read("ada", Reading::Title).await.unwrap(), "Form");
    let store = hotline_core::browser::store_dir(root.path(), "ada");
    assert!(store.exists());
    helper.close("ada", true).await.unwrap();
    assert!(!store.exists());
    assert_eq!(helper.current("ada"), None);
    drop(own);
}
