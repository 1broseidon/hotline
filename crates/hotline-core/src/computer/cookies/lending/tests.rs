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
    crate::computer::fixtures::write_script(
        root.path(),
        "browser",
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nexec sleep 30\n",
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
