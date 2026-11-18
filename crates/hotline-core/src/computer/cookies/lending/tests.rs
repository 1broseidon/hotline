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
        gate: Gate::default(),
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
        let discovery = discover(vec![browser.candidate.clone()], &Gate::default());
        assert!(discovery.sources.is_empty());
        assert_eq!(discovery.refused.len(), 1);
        assert_eq!(discovery.refused[0].name, browser.candidate.name);
        assert_eq!(
            discovery.refused[0].blocked_reason,
            Some(access_notice(browser.candidate.name, None))
        );
        let mut absent = browser.candidate.clone();
        absent.binaries.clear();
        assert_eq!(
            discover(vec![absent], &Gate::default()).refused,
            [] as [HostBrowser; 0]
        );
        browser.grant();
        let discovery = discover(vec![browser.candidate.clone()], &Gate::default());
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
    assert_eq!(
        discover(vec![absent], &Gate::default()).refused,
        [] as [HostBrowser; 0]
    );
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
        registrable("accounts.EXAMPLE.co.uk"),
        Some("example.co.uk".into())
    );
    assert_eq!(
        registrable("alice.github.io"),
        Some("alice.github.io".into())
    );
    for host in ["127.0.0.1", "co.uk", "[::1]"] {
        assert_eq!(registrable(host), None);
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

/// The stand-in browser's only decrypt operation is behind its CDP read, which
/// arrives on its DevTools pipe. Its launcher and that read must remain
/// untouched during discovery.
#[cfg(unix)]
#[tokio::test]
async fn discovery_never_launches_or_calls_the_browser_keychain_until_use() {
    let root = tempfile::tempdir().unwrap();
    let mut source = fixture(root.path(), false);
    let fake = super::super::tests::FakeChromium::new(
        root.path(),
        &json!({"id":1,"result":{"cookies":[{"name":"sid","domain":".porkbun.com","value":"fake-keychain-cookie"}]}}),
    );
    source.candidate.binaries = vec![fake.binary.clone()];
    assert!(source.sites().unwrap().contains("porkbun.com"));
    assert!(!fake.args.exists(), "detection must not launch a browser");
    assert!(
        !fake.request.exists(),
        "detection must not ask the browser's keychain"
    );

    let result = source.read("porkbun.com").await.unwrap();
    // The fake keychain is reached only when the fake browser is asked.
    let request = std::fs::read(&fake.request).unwrap();
    assert!(String::from_utf8_lossy(&request).contains("\"Storage.getCookies\""));
    assert!(
        fake.arguments()
            .iter()
            .any(|arg| arg == "--remote-debugging-pipe")
    );
    assert_eq!(result.len(), 1);
    assert_eq!(result[0]["value"], "fake-keychain-cookie");
}

/// Names in the temp dir a cookie read stages: the profile copy and, on
/// macOS, the directory its pipes are made in.
#[cfg(unix)]
fn staged_now() -> HashSet<String> {
    std::fs::read_dir(std::env::temp_dir())
        .unwrap()
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.starts_with("hotline-cookie-") || name.starts_with("hotline-devtools-"))
        .collect()
}

/// The processes still running from a launch: any with a staged profile on
/// its command line, or whose program is in the browser's own install.
#[cfg(unix)]
fn browsers_now(install: &str) -> Vec<String> {
    let listed = std::process::Command::new("ps")
        .args(["-A", "-ww", "-o", "pid=", "-o", "args="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter(|line| {
            let args = line
                .trim_start()
                .split_once(' ')
                .map_or("", |(_, args)| args);
            args.contains("hotline-cookie-") || args.starts_with(install)
        })
        .map(str::to_owned)
        .collect()
}

/// Waits a few seconds for the browser's processes to go and its staging to
/// be removed, then says what is left.
#[cfg(unix)]
async fn nothing_left_behind(before: &HashSet<String>, install: &str, after: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let processes = browsers_now(install);
        let staged: Vec<String> = staged_now().difference(before).cloned().collect();
        if processes.is_empty() && staged.is_empty() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "after {after}: processes {processes:#?}, staged {staged:?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// Shield's pre-release smoke against a real Chromium, on a profile of its
/// own with a non-empty jar (never the person's): the jar is written by the
/// browser, read back through the shipped launch, copy and pipe, and only the
/// chosen site's cookies are kept, as `Source::read` keeps them. A read that
/// times out, and one dropped part way, leave no browser process and no
/// staged copy behind. The jar is encrypted with Chromium's mock keychain and
/// basic password store, the only flags added to the shipped launch, so the
/// person's keychain is never asked. Wanted natively on Linux; run with
/// `HOTLINE_COOKIE_PIPE_BROWSER=/path/to/chromium cargo test -p hotline-core
/// --lib a_real_chromium_jar -- --ignored`.
#[cfg(unix)]
#[tokio::test]
#[ignore = "needs a Chromium binary on the host"]
async fn a_real_chromium_jar_gives_only_the_chosen_site_and_leaves_nothing_behind() {
    use std::time::Duration;
    const MOCK: &[&str] = &["--use-mock-keychain", "--password-store=basic"];
    let binary = PathBuf::from(
        std::env::var("HOTLINE_COOKIE_PIPE_BROWSER")
            .expect("HOTLINE_COOKIE_PIPE_BROWSER names a Chromium binary"),
    );
    // A macOS browser's helpers live elsewhere in its bundle: match the
    // folder that holds the bundle; elsewhere the binary's own folder.
    let install = binary
        .ancestors()
        .find(|path| path.extension().is_some_and(|extension| extension == "app"))
        .and_then(Path::parent)
        .or_else(|| binary.parent())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let profiles = tempfile::tempdir().unwrap();
    let root = profiles.path().join("chromium");
    std::fs::create_dir_all(&root).unwrap();
    let before = staged_now();

    // The jar: three cookies on two sites, set through the browser and
    // written to disk as it closes.
    let mut user_data = std::ffi::OsString::from("--user-data-dir=");
    user_data.push(&root);
    let mut command = tokio::process::Command::new(&binary);
    command
        .args(["--headless=new", "--profile-directory=Default"])
        .arg(user_data)
        .arg(super::super::devtools::PIPE_ARG)
        .args([
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-gpu",
        ])
        .args(MOCK)
        .arg("about:blank")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let (mut child, mut pipe) = super::super::devtools::spawn(&mut command).unwrap();
    // What finds what is left behind finds a running browser.
    assert_ne!(browsers_now(&install).len(), 0);
    let expires = unix_now() + 86_400;
    let set = tokio::time::timeout(
        LAUNCH_DEADLINE,
        pipe.call(
            "Storage.setCookies",
            json!({"cookies": [
                {"name": "sid", "value": "wanted-sentinel", "domain": ".wanted.test", "path": "/", "expires": expires},
                {"name": "pref", "value": "wanted-sub-sentinel", "domain": "app.wanted.test", "path": "/", "expires": expires},
                {"name": "other", "value": "unwanted-sentinel", "domain": ".unwanted.test", "path": "/", "expires": expires}
            ]}),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(set.is_ok(), "{set:?}");
    // The browser may close the pipe before it answers.
    let _ = pipe.call("Browser.close", json!({})).await;
    tokio::time::timeout(LAUNCH_DEADLINE, child.wait())
        .await
        .unwrap()
        .unwrap();
    drop(pipe);
    assert!(
        chromium_cookie_store(&root, "Default").is_some(),
        "the jar is on disk"
    );
    nothing_left_behind(&before, &install, "writing the jar").await;

    let candidate = Candidate {
        id: "test",
        name: "Test Chromium",
        family: Family::Chromium,
        binaries: vec![binary.clone()],
        root: root.clone(),
        confinement: Confinement::Native,
    };
    let shipped = Launch {
        extra: MOCK,
        deadline: LAUNCH_DEADLINE,
    };
    let all = read_chromium_with(&candidate, "Default", &shipped)
        .await
        .unwrap();
    assert!(
        all.iter()
            .any(|cookie| cookie["value"] == "unwanted-sentinel"),
        "the whole jar was read and decrypted: {} cookies",
        all.len()
    );
    let mut kept: Vec<String> = for_site(unexpired(all, unix_now()), "wanted.test")
        .iter()
        .map(|cookie| cookie["value"].as_str().unwrap().to_owned())
        .collect();
    kept.sort();
    assert_eq!(kept, ["wanted-sentinel", "wanted-sub-sentinel"]);
    nothing_left_behind(&before, &install, "a read").await;

    for wait in [1, 250, 750] {
        let short = Launch {
            extra: MOCK,
            deadline: Duration::from_millis(wait),
        };
        let _ = read_chromium_with(&candidate, "Default", &short).await;
        nothing_left_behind(&before, &install, &format!("a {wait} ms deadline")).await;
        let dropped = tokio::time::timeout(
            Duration::from_millis(wait),
            read_chromium_with(&candidate, "Default", &shipped),
        )
        .await;
        drop(dropped);
        nothing_left_behind(&before, &install, &format!("a read dropped at {wait} ms")).await;
    }
}
