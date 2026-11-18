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
