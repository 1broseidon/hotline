use super::*;

pub(crate) fn fixture(root: &Path, firefox: bool) -> Source {
    let family = if firefox {
        Family::Firefox
    } else {
        Family::Chromium
    };
    let dir = root.join(if firefox { "firefox" } else { "chromium" });
    std::fs::create_dir_all(dir.join("Default")).unwrap();
    let db = rusqlite::Connection::open(dir.join("Default").join(if firefox {
        "cookies.sqlite"
    } else {
        "Cookies"
    }))
    .unwrap();
    db.execute_batch(if firefox {
        "CREATE TABLE moz_cookies (host TEXT, name TEXT, value TEXT, path TEXT, expiry INTEGER, isSecure INTEGER, isHttpOnly INTEGER, sameSite INTEGER);
         INSERT INTO moz_cookies VALUES ('.porkbun.com','session','cookie-sentinel-273','/',0,1,1,1), ('account.porkbun.com','sub','cookie-sub-sentinel-273','/',0,1,1,1), ('unrelated.com','other','cookie-other-sentinel-273','/',0,1,1,1);"
    } else {
        // No encryption key or even value column exists: discovery can only
        // succeed by reading hosts. The binary is deliberately unlaunchable.
        "CREATE TABLE cookies (host_key TEXT); INSERT INTO cookies VALUES ('.porkbun.com'), ('account.porkbun.com'), ('unrelated.com');"
    }).unwrap();
    if firefox {
        std::fs::write(
            dir.join("profiles.ini"),
            "[Profile0]\nName=Default\nPath=Default\n",
        )
        .unwrap();
    }
    Source {
        candidate: Candidate {
            id: if firefox { "firefox" } else { "brave" },
            name: if firefox { "Firefox" } else { "Brave" },
            family,
            binaries: vec![dir.join("must-not-launch")],
            root: dir,
            confinement: Confinement::Native,
        },
        profile: BrowserProfile {
            id: "Default".into(),
            name: "Default".into(),
        },
    }
}

#[cfg(unix)]
pub(crate) fn candidate_fixture(root: &Path, firefox: bool) -> Candidate {
    fixture(root, firefox).candidate
}

/// Refuse the real directory probe, and restore access even after a failed
/// assertion so the temporary profile can be removed.
#[cfg(unix)]
pub(crate) struct RefusingBrowser {
    pub(crate) candidate: Candidate,
}

#[cfg(unix)]
impl RefusingBrowser {
    pub(crate) fn new(root: &Path, firefox: bool) -> Self {
        let browser = Self {
            candidate: candidate_fixture(root, firefox),
        };
        browser.refuse();
        browser
    }

    pub(crate) fn refuse(&self) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&self.candidate.root, std::fs::Permissions::from_mode(0o000))
            .unwrap();
        assert!(
            self.candidate.refused(),
            "the refusing fixture requires an unprivileged test runner"
        );
    }

    pub(crate) fn grant(&self) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&self.candidate.root, std::fs::Permissions::from_mode(0o755))
            .unwrap();
    }
}

#[cfg(unix)]
impl Drop for RefusingBrowser {
    fn drop(&mut self) {
        self.grant();
    }
}

#[cfg(unix)]
#[test]
fn discovery_keeps_refused_installed_browsers_and_finds_their_profiles_after_access_is_granted() {
    let root = tempfile::tempdir().unwrap();
    for firefox in [false, true] {
        let browser = RefusingBrowser::new(root.path(), firefox);
        let discovery = discover(vec![browser.candidate.clone()]);
        assert!(discovery.sources.is_empty());
        assert_eq!(discovery.refused.len(), 1);
        assert_eq!(discovery.refused[0].name, browser.candidate.name);
        assert_eq!(
            discovery.refused[0].blocked_reason,
            Some(access_notice(browser.candidate.name, None))
        );
        let mut absent = browser.candidate.clone();
        absent.binaries.clear();
        assert_eq!(discover(vec![absent]).refused, [] as [HostBrowser; 0]);
        browser.grant();
        let discovery = discover(vec![browser.candidate.clone()]);
        assert_eq!(discovery.refused, [] as [HostBrowser; 0]);
        assert_eq!(discovery.sources.len(), 1);
        assert!(
            discovery.sources[0]
                .sites()
                .unwrap()
                .contains("porkbun.com")
        );
    }
    let mut absent = candidate_fixture(&root.path().join("absent-profile"), false);
    absent.root = root.path().join("missing");
    assert_eq!(discover(vec![absent]).refused, [] as [HostBrowser; 0]);
}

#[test]
fn full_disk_access_names_the_installed_app_or_the_actual_dev_binary() {
    for (path, name) in [
        (
            "/Applications/Hotline.app/Contents/MacOS/hotline-app",
            "Hotline",
        ),
        ("/checkout/target/debug/hotline-app-dev", "hotline-app-dev"),
    ] {
        assert_eq!(access_app_name(Path::new(path)), name);
    }
    let message = access_notice("Brave", Some("porkbun.com"));
    assert!(message.starts_with(
        "Hotline can't read Brave's cookies, so it can't offer your session for porkbun.com. "
    ));
    if cfg!(target_os = "macos") {
        assert!(message.contains(
            "in System Settings → Privacy & Security → Full Disk Access, then reopen Hotline."
        ));
    }
}

#[test]
fn detection_reads_chromium_and_firefox_host_columns_without_launch_or_keychain() {
    let root = tempfile::tempdir().unwrap();
    for firefox in [false, true] {
        let source = fixture(root.path(), firefox);
        let sites = source.sites().unwrap();
        assert_eq!(
            sites,
            HashSet::from(["porkbun.com".into(), "unrelated.com".into()])
        );
        assert!(!source.candidate.binaries[0].exists());
        assert!(!source.candidate.root.join("Local State").exists());
        assert!(!format!("{sites:?}").contains("sentinel"));
    }
}

#[test]
fn only_the_registrable_site_and_its_subdomains_can_be_lent() {
    assert_eq!(
        site_for_origin("https://accounts.EXAMPLE.co.uk/path"),
        Some("example.co.uk".into())
    );
    assert_eq!(
        site_for_origin("https://alice.github.io"),
        Some("alice.github.io".into())
    );
    for origin in [
        "chrome://settings",
        "https://127.0.0.1",
        "https://co.uk",
        "about:blank",
    ] {
        assert_eq!(site_for_origin(origin), None);
    }
    let cookies = [
        ".example.co.uk",
        "a.example.co.uk",
        "badexample.co.uk",
        "co.uk",
        "example.com",
    ]
    .into_iter()
    .map(|domain| json!({"domain":domain,"value":"secret"}))
    .collect();
    let kept = for_site(cookies, "example.co.uk");
    assert_eq!(kept.len(), 2);
    assert_eq!(kept[1]["domain"], "a.example.co.uk");
}

/// The stand-in browser's only decrypt operation is behind its CDP read.
/// Its launcher and keychain spy must remain untouched during discovery.
#[cfg(unix)]
#[tokio::test]
async fn discovery_never_launches_or_calls_the_browser_keychain_until_use() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let root = tempfile::tempdir().unwrap();
    let mut source = fixture(root.path(), false);
    let args = root.path().join("browser-args");
    // The redirect makes the file before printf fills it, so the arguments
    // are written beside it and renamed in: the file exists only when whole.
    crate::computer::fixtures::write_script(
        root.path(),
        "browser",
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{0}.partial'\nmv '{0}.partial' '{0}'\nexec sleep 30\n",
            args.display()
        ),
    );
    source.candidate.binaries = vec![root.path().join("browser")];
    let keychain_calls = Arc::new(AtomicUsize::new(0));
    let ws = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws.local_addr().unwrap().port();
    let calls = keychain_calls.clone();
    let cdp = tokio::spawn(async move {
        let (stream, _) = ws.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let request = socket.next().await.unwrap().unwrap().into_text().unwrap();
        let request: Value = serde_json::from_str(&request).unwrap();
        assert_eq!(request["method"], "Network.getAllCookies");
        // The fake keychain is accessed only when the fake browser decrypts.
        calls.fetch_add(1, Ordering::SeqCst);
        socket.send(Message::Text(json!({"id":1,"result":{"cookies":[{"name":"sid","domain":".porkbun.com","value":"fake-keychain-cookie"}]}}).to_string().into())).await.unwrap();
        let _ = socket.next().await;
    });
    assert!(source.sites().unwrap().contains("porkbun.com"));
    assert!(!args.exists(), "detection must not launch a browser");
    assert_eq!(
        keychain_calls.load(Ordering::SeqCst),
        0,
        "detection must not ask the browser's keychain"
    );

    let reading = tokio::spawn(async move { source.read("porkbun.com").await });
    crate::test_support::until("Use launches the profile copy", || args.exists()).await;
    let arguments = std::fs::read_to_string(&args).unwrap();
    let port: u16 = arguments
        .lines()
        .find_map(|arg| arg.strip_prefix("--remote-debugging-port="))
        .unwrap()
        .parse()
        .unwrap();
    let http = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    let app = axum::Router::new().route(
        "/json",
        axum::routing::get(move || async move {
            (
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                json!([{"type":"page","webSocketDebuggerUrl":format!("ws://127.0.0.1:{ws_port}")}])
                    .to_string(),
            )
        }),
    );
    let serving = tokio::spawn(async move { axum::serve(http, app).await.unwrap() });
    let result = reading.await.unwrap().unwrap();
    assert_eq!(keychain_calls.load(Ordering::SeqCst), 1);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0]["value"], "fake-keychain-cookie");
    cdp.await.unwrap();
    serving.abort();
}
