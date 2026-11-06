//! The desk's end of the server's browser, over the real pipe: a call and
//! its answer, the gate asked from the helper's side, the quick reads kept
//! from what the helper said, and the supervisor starting, losing and
//! starting a helper process again.
use super::*;
use crate::browser::pipe::{Link, Outbox, answer};
use crate::log::Log;
use std::os::unix::fs::PermissionsExt;

/// One teammate's browser, as the helper's side of the pipe calls it.
struct Fake {
    link: Link,
    page: Mutex<Option<String>>,
    size: Mutex<Option<(u32, u32)>>,
    /// What reached it besides loads, in order.
    heard: Mutex<Vec<String>>,
}

impl Fake {
    fn heard(&self, what: impl Into<String>) {
        lock(&self.heard).push(what.into());
    }

    fn here(&self) -> Page {
        Page {
            url: lock(&self.page).clone().unwrap_or_default(),
            title: "Example Domain".into(),
            loaded: true,
        }
    }
}

#[async_trait]
impl DeskBrowser for Fake {
    async fn open(&self, _persona_id: &str, url: &Url) -> Result<Page, String> {
        // As the helper's web view asks before a load.
        if !self.link.admits_frame(url.as_str()).await {
            return Err(format!("refused {url}"));
        }
        *lock(&self.page) = Some(url.to_string());
        Ok(self.here())
    }
    async fn step(&self, _persona_id: &str, step: Step) -> Result<Page, String> {
        self.heard(format!("{step:?}"));
        Ok(self.here())
    }
    async fn read(&self, _persona_id: &str, reading: Reading) -> Result<String, String> {
        Ok(format!("read {reading:?}"))
    }
    async fn act(&self, _persona_id: &str, act: &Act) -> Result<Acted, String> {
        Ok(Acted {
            page: self.here(),
            note: Some(format!("{act:?}")),
        })
    }
    async fn screenshot(&self, _persona_id: &str) -> Result<Vec<u8>, String> {
        Ok(vec![0x89, b'P', 0, 255])
    }
    async fn full_page(&self, _persona_id: &str, most: usize) -> Result<FullPage, String> {
        Ok(FullPage {
            pictures: vec![vec![1], vec![2]],
            more: most == 2,
        })
    }
    async fn field(&self, _persona_id: &str, target: &str) -> Result<Field, String> {
        Ok(Field {
            tag: "input".into(),
            kind: "password".into(),
            words: target.into(),
            ..Field::default()
        })
    }
    async fn showing(&self, _persona_id: &str) -> Result<Showing, String> {
        Ok(Showing {
            text: "Welcome".into(),
            fields: vec!["ada".into()],
        })
    }
    async fn viewport(&self, _persona_id: &str, size: Option<(u32, u32)>) -> Result<Page, String> {
        *lock(&self.size) = size;
        Ok(self.here())
    }
    fn size(&self, _persona_id: &str) -> Option<(u32, u32)> {
        *lock(&self.size)
    }
    async fn page_size(&self, _persona_id: &str) -> Option<(u32, u32)> {
        Some(lock(&self.size).unwrap_or((1280, 800)))
    }
    async fn person_input(&self, _persona_id: &str, input: &BrowserInput) -> Result<(), String> {
        if matches!(input, BrowserInput::Text { text } if text.is_empty()) {
            return Err("Nothing to type.".into());
        }
        self.heard(format!("{input:?}"));
        Ok(())
    }
    fn taken(&self, _persona_id: &str, taken: bool) {
        self.heard(format!("taken {taken}"));
    }
    fn current(&self, _persona_id: &str) -> Option<String> {
        lock(&self.page).clone()
    }
    fn needs(&self, _persona_id: &str) -> Option<String> {
        lock(&self.page)
            .as_deref()
            .filter(|page| page.contains("expired"))
            .map(|_| "Certificate expired".to_string())
    }
    fn name_teammate(&self, _persona_id: &str, name: &str) {
        self.heard(format!("name {name}"));
    }
    async fn close(&self, _persona_id: &str, wipe: bool) -> Result<(), String> {
        self.heard(format!("close {wipe}"));
        *lock(&self.page) = None;
        Ok(())
    }
}

/// A gate that never opens port 4545, the desk's own here.
fn gate(root: &Path) -> Arc<Gate> {
    let gate = Arc::new(Gate::default());
    gate.load(&Log::open(root), &[4545]);
    gate
}

/// A helper whose other end is `fake`, in this process, over a real socket.
async fn piped(root: &Path) -> (Helper, Arc<Fake>) {
    let (ours, theirs) = UnixStream::pair().unwrap();
    let (link, outbox): (Link, Outbox) = Link::new();
    let fake = Arc::new(Fake {
        link: link.clone(),
        page: Mutex::default(),
        size: Mutex::default(),
        heard: Mutex::default(),
    });
    let browser: Arc<dyn DeskBrowser> = fake.clone();
    tokio::spawn(answer(
        theirs,
        link,
        outbox,
        browser,
        Up::Ready { pid: 1, display: 0 },
    ));
    let helper = Helper::new(root.join("no-helper"), root.to_path_buf(), gate(root));
    let (conn, readied) = Conn::open(ours, helper.gate.clone(), helper.states.clone());
    readied.await.unwrap();
    *lock(&helper.conn) = Some(conn);
    (helper, fake)
}

#[tokio::test]
async fn every_call_crosses_the_pipe_and_comes_back_as_the_browser_answered_it() {
    let root = tempfile::tempdir().unwrap();
    let (helper, fake) = piped(root.path()).await;
    let url = Url::parse("https://example.com/").unwrap();
    let page = helper.open("ada", &url).await.unwrap();
    assert_eq!(page.url, "https://example.com/");
    // Where it is was said before the reply, so the quick read has it.
    assert_eq!(
        helper.current("ada").as_deref(),
        Some("https://example.com/")
    );
    assert_eq!(helper.needs("ada"), None);

    assert_eq!(
        helper.step("ada", Step::Back).await.unwrap().title,
        "Example Domain"
    );
    assert_eq!(
        helper.read("ada", Reading::Links).await.unwrap(),
        "read Links"
    );
    let fill = Act::Fill {
        target: "e3".into(),
        text: "hunter2".into(),
    };
    let note = helper.act("ada", &fill).await.unwrap().note.unwrap();
    assert!(note.contains("e3") && !note.contains("hunter2"), "{note}");
    assert_eq!(
        helper.screenshot("ada").await.unwrap(),
        vec![0x89, b'P', 0, 255]
    );
    let whole = helper.full_page("ada", 2).await.unwrap();
    assert_eq!((whole.pictures, whole.more), (vec![vec![1], vec![2]], true));
    assert_eq!(helper.field("ada", "e3").await.unwrap().kind, "password");
    assert_eq!(
        helper.showing("ada").await.unwrap().fields,
        vec!["ada".to_string()]
    );
    helper.viewport("ada", Some((390, 844))).await.unwrap();
    assert_eq!(helper.size("ada"), Some((390, 844)));
    assert_eq!(helper.page_size("ada").await, Some((390, 844)));

    let tap = BrowserInput::Tap { x: 10.0, y: 20.5 };
    helper.person_input("ada", &tap).await.unwrap();
    let empty = BrowserInput::Text {
        text: String::new(),
    };
    assert_eq!(
        helper.person_input("ada", &empty).await,
        Err("Nothing to type.".into())
    );
    // Told, never answered: each reaches the browser before the next call.
    helper.taken("ada", true);
    helper.name_teammate("ada", "Ada");
    helper.name_teammate("ada", "Ada");
    helper.read("ada", Reading::Title).await.unwrap();
    assert_eq!(
        *lock(&fake.heard),
        ["Back", "Tap { x: 10.0, y: 20.5 }", "taken true", "name Ada"]
    );

    helper.close("ada", true).await.unwrap();
    assert_eq!(helper.current("ada"), None);
    assert_eq!(helper.size("ada"), None);
    assert!(lock(&fake.heard).contains(&"close true".to_string()));
}

#[tokio::test]
async fn every_load_the_helper_asks_about_is_judged_by_the_desks_own_gate() {
    let root = tempfile::tempdir().unwrap();
    let (helper, _fake) = piped(root.path()).await;
    for refused in [
        "http://127.0.0.1:4545/",
        "http://localhost:11434/api",
        "file:///etc/passwd",
    ] {
        let url = Url::parse(refused).unwrap();
        assert_eq!(
            helper.open("ada", &url).await,
            Err(format!("refused {url}"))
        );
    }
    let open = Url::parse("http://127.0.0.1:8080/").unwrap();
    assert!(helper.open("ada", &open).await.is_ok());
    let expired = Url::parse("https://expired.example/").unwrap();
    helper.open("ada", &expired).await.unwrap();
    assert_eq!(helper.needs("ada").as_deref(), Some("Certificate expired"));
}

/// A helper program: a shell script that adds a line to `starts` each time
/// it runs, then does `body`.
fn script(root: &Path, body: &str) -> (PathBuf, PathBuf) {
    let starts = root.join("starts");
    let program = root.join("hotline-browser");
    std::fs::write(
        &program,
        format!("#!/bin/sh\necho \"$*\" >> '{}'\n{body}\n", starts.display()),
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    (program, starts)
}

fn starts(file: &Path) -> Vec<String> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

#[tokio::test]
async fn a_helper_that_cannot_start_is_said_and_the_next_page_tries_again() {
    let root = tempfile::tempdir().unwrap();
    let (program, started) = script(root.path(), "exit 3");
    let helper = Helper::new(program, root.path().join("data"), gate(root.path()));
    let url = Url::parse("https://example.com/").unwrap();
    for _ in 0..2 {
        let error = helper.open("ada", &url).await.unwrap_err();
        assert!(error.contains("could not start"), "{error}");
    }
    let data = root.path().join("data");
    assert_eq!(
        starts(&started),
        vec![format!("--data {}", data.display()); 2]
    );
    // No page ever opened: the rest says so, and nothing was started for it.
    assert_eq!(helper.read("ada", Reading::Text).await, Err(NO_PAGE.into()));
    assert_eq!(helper.current("ada"), None);
    assert_eq!(starts(&started).len(), 2);
}

#[tokio::test]
async fn a_helper_that_dies_fails_the_call_in_flight_and_the_next_page_starts_another() {
    let root = tempfile::tempdir().unwrap();
    // Ready on the pipe it was handed, then gone at the first call.
    let (program, started) = script(
        root.path(),
        "printf '{\"kind\":\"ready\",\"pid\":%s,\"display\":0}\\n' $$ >&3\nread call <&3\nexit 1",
    );
    let helper = Helper::new(program, root.path().to_path_buf(), gate(root.path()));
    let url = Url::parse("https://example.com/").unwrap();
    assert_eq!(helper.open("ada", &url).await, Err(STOPPED.into()));
    assert_eq!(helper.read("ada", Reading::Text).await, Err(STOPPED.into()));
    assert_eq!(helper.current("ada"), None);
    helper.taken("ada", true);
    assert_eq!(helper.open("ada", &url).await, Err(STOPPED.into()));
    assert_eq!(starts(&started).len(), 2);
}

#[tokio::test]
async fn a_wiped_teammate_with_no_helper_running_loses_its_storage_on_disk() {
    let root = tempfile::tempdir().unwrap();
    let (program, started) = script(root.path(), "exit 3");
    let helper = Helper::new(program, root.path().to_path_buf(), gate(root.path()));
    let store = crate::browser::store_dir(root.path(), "ada");
    std::fs::create_dir_all(store.join("cookies")).unwrap();
    helper.close("ada", false).await.unwrap();
    assert!(store.exists(), "a close that keeps storage keeps it");
    helper.close("ada", true).await.unwrap();
    assert!(!store.exists());
    helper.close("ada", true).await.unwrap();
    assert!(starts(&started).is_empty(), "closing started no helper");
}

#[tokio::test]
async fn the_probe_answers_what_the_helper_said_or_why_it_cannot_run() {
    let root = tempfile::tempdir().unwrap();
    let (fine, _) = script(
        root.path(),
        "echo 'hotline-browser 1.2.3, WebKitGTK 2.50.4'",
    );
    assert_eq!(
        probe(&fine).await,
        Ok("hotline-browser 1.2.3, WebKitGTK 2.50.4".into())
    );
    let (missing, _) = script(
        root.path(),
        "echo 'hotline-browser: error while loading shared libraries: libwebkit2gtk-4.1.so.0: cannot open shared object file' >&2\nexit 127",
    );
    assert_eq!(
        probe(&missing).await,
        Err("WebKitGTK is not installed (the libwebkit2gtk-4.1-0 package)".into())
    );
    let (no_xvfb, _) = script(
        root.path(),
        "echo 'Xvfb is not installed (the xvfb package)' >&2\nexit 1",
    );
    assert_eq!(
        probe(&no_xvfb).await,
        Err("Xvfb is not installed (the xvfb package)".into())
    );
    let absent = probe(&root.path().join("nowhere")).await.unwrap_err();
    assert!(
        absent.starts_with("hotline-browser did not run"),
        "{absent}"
    );
}

#[test]
fn only_a_helper_of_this_desks_own_version_is_used() {
    let version = env!("CARGO_PKG_VERSION");
    assert_eq!(
        ours(&format!("hotline-browser {version}, WebKitGTK 2.50.4")),
        Ok("WebKitGTK 2.50.4".into())
    );
    let older = ours("hotline-browser 0.0.1, WebKitGTK 2.50.4").unwrap_err();
    assert!(older.contains("not this desk's version"), "{older}");
    assert!(ours("WebKitGTK 2.50.4").is_err());
}

#[tokio::test]
async fn whatever_a_dead_helper_left_in_its_group_goes_with_it() {
    let root = tempfile::tempdir().unwrap();
    let left = root.path().join("left");
    // Its first process killed outright, as if nothing reaped below it: a
    // process it started is left running in its group.
    let (program, _) = script(
        root.path(),
        &format!(
            "sleep 300 &\necho $! > '{}'\nprintf '{{\"kind\":\"ready\",\"pid\":%s,\"display\":0}}\\n' $$ >&3\nread call <&3\nkill -9 $$",
            left.display()
        ),
    );
    let helper = Helper::new(program, root.path().to_path_buf(), gate(root.path()));
    let url = Url::parse("https://example.com/").unwrap();
    assert_eq!(helper.open("ada", &url).await, Err(STOPPED.into()));
    let sleeper: libc::pid_t = std::fs::read_to_string(&left)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    crate::test_support::until("what the helper left is gone", || {
        // SAFETY: a signal check of the process the script started, sending nothing.
        unsafe { libc::kill(sleeper, 0) != 0 }
    })
    .await;
}
