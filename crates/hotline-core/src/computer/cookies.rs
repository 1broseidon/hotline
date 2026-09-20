//! Reading cookies from the browsers on the host, so the operator can hand a
//! teammate's computer the sessions it is signed in to.
//!
//! This is host-side and operator-only. Nothing here is an agent tool, and
//! nothing the model says reaches it: the wire commands that call it are
//! gated to the desk seat, the same as adding a mount. The agent cannot pull
//! its own cookies; it receives exactly the sites the operator ticked.
//!
//! The reading is done by the browser, not by us. For a Chromium browser the
//! desk launches it headless on a *copy* of its profile with a debugging
//! port and asks it for its own cookies over CDP `Network.getAllCookies`, so
//! the browser decrypts with its own key and the result is uniform across
//! Chrome, Edge, Brave and Chromium and every version, app-bound encryption
//! included. We never touch the cookie crypto. Firefox keeps its cookies in a
//! plaintext SQLite file, so it is read directly, no launch.
//!
//! A profile *copy* is used for two reasons: a second instance cannot open a
//! profile another process holds the lock on, and the operator's live browser
//! must not be disturbed. A snap-confined browser can only open a profile
//! inside its own writable tree, so the copy for one goes under
//! `~/snap/<name>/…` rather than the system temp dir.

use crate::contract::{BrowserProfile, CookieSite, HostBrowser};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;
use tokio_tungstenite::tungstenite::Message;

/// Cookies are read from a throwaway headless instance; if the browser has
/// not opened its debugging port by now something is wrong, and we stop
/// rather than hang the operator's import.
const LAUNCH_DEADLINE: Duration = Duration::from_secs(20);

/// Which family a browser belongs to, and so how its cookies are read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    Chromium,
    Firefox,
}

impl Family {
    fn wire(self) -> &'static str {
        match self {
            Family::Chromium => "chromium",
            Family::Firefox => "firefox",
        }
    }
}

/// How a browser is packaged, which decides where a profile copy may live for
/// the browser to be allowed to open it.
#[derive(Clone)]
enum Confinement {
    /// A system install: the copy may live in the system temp directory.
    Native,
    /// A snap: the copy must live under `~/snap/<name>/common` to be inside
    /// the confinement the browser runs under.
    Snap(String),
}

/// One browser Hotline knows how to read, whether or not it is installed.
#[derive(Clone)]
struct Candidate {
    id: &'static str,
    name: &'static str,
    family: Family,
    /// Absolute binaries to try in order; the first that exists is used.
    binaries: Vec<PathBuf>,
    /// The user-data directory (Chromium) or the profiles root (Firefox).
    root: PathBuf,
    confinement: Confinement,
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// The binaries that might be a given browser, tried in order. On Linux a
/// bare name is resolved against `PATH`; an absolute path is taken as is.
fn resolve_binary(names: &[&str]) -> Vec<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    let mut found = Vec::new();
    for name in names {
        let candidate = Path::new(name);
        if candidate.is_absolute() {
            if candidate.exists() {
                found.push(candidate.to_path_buf());
            }
            continue;
        }
        for dir in &dirs {
            let full = dir.join(name);
            if full.exists() {
                found.push(full);
                break;
            }
        }
    }
    found
}

/// Every browser Hotline can read on this platform, installed or not. The
/// caller keeps only those whose `root` exists and holds a profile with a
/// cookie store.
fn candidates() -> Vec<Candidate> {
    let home = home();
    let mut out = Vec::new();

    // Linux. Chromium data dirs sit under ~/.config; Firefox under ~/.mozilla;
    // snaps keep both under ~/snap/<name>/common.
    #[cfg(target_os = "linux")]
    {
        let config = home.join(".config");
        let chromium = |id, name, sub: &str, bins: &[&str]| Candidate {
            id,
            name,
            family: Family::Chromium,
            binaries: resolve_binary(bins),
            root: config.join(sub),
            confinement: Confinement::Native,
        };
        out.push(chromium(
            "chrome",
            "Google Chrome",
            "google-chrome",
            &["google-chrome", "google-chrome-stable"],
        ));
        out.push(chromium(
            "chromium",
            "Chromium",
            "chromium",
            &["chromium", "chromium-browser"],
        ));
        out.push(chromium(
            "brave",
            "Brave",
            "BraveSoftware/Brave-Browser",
            &["brave-browser", "brave"],
        ));
        out.push(chromium(
            "edge",
            "Microsoft Edge",
            "microsoft-edge",
            &["microsoft-edge", "microsoft-edge-stable"],
        ));
        // Snap Chromium: binary at /snap/bin/chromium, profile under the
        // snap's own writable home.
        out.push(Candidate {
            id: "chromium-snap",
            name: "Chromium (snap)",
            family: Family::Chromium,
            binaries: resolve_binary(&["/snap/bin/chromium"]),
            root: home.join("snap/chromium/common/chromium"),
            confinement: Confinement::Snap("chromium".into()),
        });
        out.push(Candidate {
            id: "firefox",
            name: "Firefox",
            family: Family::Firefox,
            binaries: resolve_binary(&["firefox"]),
            root: home.join(".mozilla/firefox"),
            confinement: Confinement::Native,
        });
        out.push(Candidate {
            id: "firefox-snap",
            name: "Firefox (snap)",
            family: Family::Firefox,
            binaries: resolve_binary(&["/snap/bin/firefox"]),
            root: home.join("snap/firefox/common/.mozilla/firefox"),
            confinement: Confinement::Snap("firefox".into()),
        });
    }

    // macOS. Application Support for Chromium data, Firefox under its own
    // Profiles directory; binaries inside the .app bundles.
    #[cfg(target_os = "macos")]
    {
        let support = home.join("Library/Application Support");
        let app = |bin: &str| PathBuf::from(format!("/Applications/{bin}"));
        let chromium = |id, name, sub: &str, bin: &str| Candidate {
            id,
            name,
            family: Family::Chromium,
            binaries: resolve_binary(&[app(bin).to_str().unwrap_or_default()]),
            root: support.join(sub),
            confinement: Confinement::Native,
        };
        out.push(chromium(
            "chrome",
            "Google Chrome",
            "Google/Chrome",
            "Google Chrome.app/Contents/MacOS/Google Chrome",
        ));
        out.push(chromium(
            "brave",
            "Brave",
            "BraveSoftware/Brave-Browser",
            "Brave Browser.app/Contents/MacOS/Brave Browser",
        ));
        out.push(chromium(
            "edge",
            "Microsoft Edge",
            "Microsoft Edge",
            "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        ));
        out.push(Candidate {
            id: "firefox",
            name: "Firefox",
            family: Family::Firefox,
            binaries: resolve_binary(&["/Applications/Firefox.app/Contents/MacOS/firefox"]),
            root: support.join("Firefox/Profiles"),
            confinement: Confinement::Native,
        });
    }

    // Windows. Chromium data under Local AppData\<vendor>\<browser>\User Data,
    // Firefox under Roaming\Mozilla\Firefox.
    #[cfg(target_os = "windows")]
    {
        let local = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Local"));
        let roaming = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Roaming"));
        let program_files = std::env::var_os("PROGRAMFILES")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:/Program Files"));
        let chromium = |id, name, sub: &str, bin: PathBuf| Candidate {
            id,
            name,
            family: Family::Chromium,
            binaries: if bin.exists() { vec![bin] } else { Vec::new() },
            root: local.join(sub),
            confinement: Confinement::Native,
        };
        out.push(chromium(
            "chrome",
            "Google Chrome",
            "Google/Chrome/User Data",
            program_files.join("Google/Chrome/Application/chrome.exe"),
        ));
        out.push(chromium(
            "brave",
            "Brave",
            "BraveSoftware/Brave-Browser/User Data",
            program_files.join("BraveSoftware/Brave-Browser/Application/brave.exe"),
        ));
        out.push(chromium(
            "edge",
            "Microsoft Edge",
            "Microsoft/Edge/User Data",
            program_files.join("Microsoft/Edge/Application/msedge.exe"),
        ));
        out.push(Candidate {
            id: "firefox",
            name: "Firefox",
            family: Family::Firefox,
            binaries: if program_files.join("Mozilla Firefox/firefox.exe").exists() {
                vec![program_files.join("Mozilla Firefox/firefox.exe")]
            } else {
                Vec::new()
            },
            root: roaming.join("Mozilla/Firefox"),
            confinement: Confinement::Native,
        });
    }

    out
}

impl Candidate {
    /// The profiles on disk that have a cookie store, in the shape the wire
    /// hands the operator.
    fn profiles(&self) -> Vec<BrowserProfile> {
        match self.family {
            Family::Chromium => chromium_profiles(&self.root),
            Family::Firefox => firefox_profiles(&self.root),
        }
    }

    fn first_binary(&self) -> Option<&Path> {
        self.binaries.first().map(PathBuf::as_path)
    }
}

/// The Chromium profiles under a user-data directory, named the way the
/// browser names them in `Local State`, falling back to the directory name.
fn chromium_profiles(root: &Path) -> Vec<BrowserProfile> {
    let names = local_state_names(root);
    // Prefer the order Local State lists, then any other directory on disk.
    let mut order: Vec<String> = names.keys().cloned().collect();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if entry.path().is_dir()
                && let Some(dir) = entry.file_name().to_str()
                && !order.iter().any(|seen| seen == dir)
            {
                order.push(dir.to_owned());
            }
        }
    }
    // Keep only the ones with a cookie store, named the way the browser does.
    order
        .into_iter()
        .filter(|dir| chromium_cookie_store(root, dir).is_some())
        .map(|dir| BrowserProfile {
            name: names.get(&dir).cloned().unwrap_or_else(|| dir.clone()),
            id: dir,
        })
        .collect()
}

/// The display names Chromium records for its profiles, dir name → name.
fn local_state_names(root: &Path) -> std::collections::BTreeMap<String, String> {
    let mut names = std::collections::BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(root.join("Local State")) else {
        return names;
    };
    let Ok(state) = serde_json::from_str::<Value>(&text) else {
        return names;
    };
    if let Some(cache) = state
        .get("profile")
        .and_then(|profile| profile.get("info_cache"))
        .and_then(Value::as_object)
    {
        for (dir, info) in cache {
            let name = info
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(dir)
                .to_owned();
            names.insert(dir.clone(), name);
        }
    }
    names
}

/// The cookie SQLite for one Chromium profile: the newer `Network/Cookies`,
/// else the older `Cookies`.
fn chromium_cookie_store(root: &Path, dir: &str) -> Option<PathBuf> {
    let network = root.join(dir).join("Network").join("Cookies");
    if network.exists() {
        return Some(network);
    }
    let flat = root.join(dir).join("Cookies");
    flat.exists().then_some(flat)
}

/// The Firefox profiles with a `cookies.sqlite`, read from `profiles.ini`.
fn firefox_profiles(root: &Path) -> Vec<BrowserProfile> {
    let Ok(text) = std::fs::read_to_string(root.join("profiles.ini")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let (mut path, mut name) = (None::<String>, None::<String>);
    let mut flush = |path: &mut Option<String>, name: &mut Option<String>| {
        if let Some(rel) = path.take() {
            let store = root.join(&rel).join("cookies.sqlite");
            if store.exists() {
                out.push(BrowserProfile {
                    id: rel.clone(),
                    name: name.take().unwrap_or(rel),
                });
            }
            *name = None;
        }
    };
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            flush(&mut path, &mut name);
        } else if let Some(value) = line.strip_prefix("Path=") {
            path = Some(value.trim().to_owned());
        } else if let Some(value) = line.strip_prefix("Name=") {
            name = Some(value.trim().to_owned());
        }
    }
    flush(&mut path, &mut name);
    out
}

/// The browsers on this host that have at least one profile with cookies.
pub fn detect() -> Vec<HostBrowser> {
    candidates()
        .into_iter()
        .filter(|candidate| {
            candidate.first_binary().is_some() || candidate.family == Family::Firefox
        })
        .filter_map(|candidate| {
            let profiles = candidate.profiles();
            (!profiles.is_empty()).then(|| HostBrowser {
                id: candidate.id.to_owned(),
                name: candidate.name.to_owned(),
                family: candidate.family.wire().to_owned(),
                profiles,
            })
        })
        .collect()
}

fn find(browser_id: &str) -> Result<Candidate, String> {
    candidates()
        .into_iter()
        .find(|candidate| candidate.id == browser_id)
        .ok_or_else(|| format!("No browser named {browser_id:?} on this machine."))
}

/// Every cookie in one profile, as CDP cookie objects — the shape the
/// container's browser accepts back. This is the only function that opens a
/// value; its callers reduce it to a preview or hand the selection to the
/// container without it passing through anything the model can read.
async fn read_all(browser_id: &str, profile_id: &str) -> Result<Vec<Value>, String> {
    let candidate = find(browser_id)?;
    match candidate.family {
        Family::Chromium => read_chromium(&candidate, profile_id).await,
        Family::Firefox => read_firefox(&candidate, profile_id),
    }
}

/// The sites in a profile and how many cookies each has — names and counts,
/// never a value. This is what the operator's picker shows.
pub async fn preview(browser_id: &str, profile_id: &str) -> Result<Vec<CookieSite>, String> {
    let cookies = read_all(browser_id, profile_id).await?;
    Ok(sites(&cookies))
}

/// The cookies in a profile whose registrable site the operator ticked, ready
/// to hand to the container. `domains` are the `CookieSite::domain` values
/// chosen; a cookie is included when its own domain matches one of them.
pub async fn select(
    browser_id: &str,
    profile_id: &str,
    domains: &[String],
) -> Result<Vec<Value>, String> {
    let wanted: std::collections::HashSet<&str> = domains.iter().map(String::as_str).collect();
    let cookies = read_all(browser_id, profile_id).await?;
    Ok(cookies
        .into_iter()
        .filter(|cookie| {
            cookie
                .get("domain")
                .and_then(Value::as_str)
                .map(normalize_domain)
                .is_some_and(|domain| wanted.contains(domain.as_str()))
        })
        .collect())
}

/// A cookie's `domain` reduced to the key the preview groups by: a leading
/// dot is dropped, so `.example.com` and `example.com` are one site.
fn normalize_domain(domain: &str) -> String {
    domain.trim_start_matches('.').to_ascii_lowercase()
}

/// The sites present in a set of cookies, for reporting what an import
/// actually carried. Names and counts only.
pub fn summarize(cookies: &[Value]) -> Vec<CookieSite> {
    sites(cookies)
}

/// Group cookies by site for the preview, most cookies first then by name.
fn sites(cookies: &[Value]) -> Vec<CookieSite> {
    let mut counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for cookie in cookies {
        if let Some(domain) = cookie.get("domain").and_then(Value::as_str) {
            *counts.entry(normalize_domain(domain)).or_default() += 1;
        }
    }
    let mut out: Vec<CookieSite> = counts
        .into_iter()
        .map(|(domain, cookies)| CookieSite { domain, cookies })
        .collect();
    out.sort_by(|a, b| {
        b.cookies
            .cmp(&a.cookies)
            .then_with(|| a.domain.cmp(&b.domain))
    });
    out
}

// ---- Chromium: ask the browser for its own cookies over CDP ---------------

async fn read_chromium(candidate: &Candidate, profile_id: &str) -> Result<Vec<Value>, String> {
    let binary = candidate
        .first_binary()
        .ok_or_else(|| format!("{} is not installed on this machine.", candidate.name))?
        .to_path_buf();
    if chromium_cookie_store(&candidate.root, profile_id).is_none() {
        return Err(format!("{} has no profile {profile_id:?}.", candidate.name));
    }
    let staging = Staging::new(&candidate.confinement)?;
    copy_chromium_profile(&candidate.root, profile_id, staging.path())?;

    let port = free_loopback_port()?;
    let mut child = Command::new(&binary)
        .arg("--headless=new")
        .arg(format!("--user-data-dir={}", staging.path().display()))
        .arg(format!("--profile-directory={profile_id}"))
        .arg(format!("--remote-debugging-port={port}"))
        // Since Chrome 111 the DevTools socket refuses a cross-origin upgrade
        // unless the origin is allowed; this instance is loopback and lives
        // for one read, so any origin is fine.
        .arg("--remote-allow-origins=*")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-gpu")
        .arg("--disable-sync")
        .arg("--disable-background-networking")
        .arg("--disable-features=Translate")
        .arg("--no-sandbox")
        .arg("about:blank")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("Could not launch {}: {error}", candidate.name))?;

    let result = tokio::time::timeout(LAUNCH_DEADLINE, cdp_all_cookies(port)).await;
    child.kill().await.ok();
    // The staging copy drops here regardless of the outcome.
    drop(staging);
    match result {
        Ok(cookies) => cookies,
        Err(_) => Err(format!(
            "{} did not answer for its cookies in time.",
            candidate.name
        )),
    }
}

/// Copy just what the browser needs to open the profile read-only and decrypt
/// its cookies: the key reference in `Local State`, and the profile's cookie
/// store and preferences. Caches and history are left behind, so the copy is
/// small and quick.
fn copy_chromium_profile(root: &Path, profile_id: &str, staging: &Path) -> Result<(), String> {
    let map = |from: PathBuf, to: PathBuf| -> Result<(), String> {
        if !from.exists() {
            return Ok(());
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::copy(&from, &to)
            .map(|_| ())
            .map_err(|error| format!("Could not stage {}: {error}", from.display()))
    };
    map(root.join("Local State"), staging.join("Local State"))?;
    let dir = root.join(profile_id);
    let dest = staging.join(profile_id);
    map(dir.join("Cookies"), dest.join("Cookies"))?;
    map(
        dir.join("Network").join("Cookies"),
        dest.join("Network").join("Cookies"),
    )?;
    map(dir.join("Preferences"), dest.join("Preferences"))?;
    Ok(())
}

/// Open a CDP session on the loopback debugging port and ask for the whole
/// cookie jar. The browser has already decrypted it; we only carry it.
async fn cdp_all_cookies(port: u16) -> Result<Vec<Value>, String> {
    let ws_url = cdp_target(port).await?;
    let (mut socket, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .map_err(|error| format!("Could not open the browser's debugging socket: {error}"))?;
    socket
        .send(Message::Text(
            json!({"id": 1, "method": "Network.getAllCookies"})
                .to_string()
                .into(),
        ))
        .await
        .map_err(|error| format!("Could not ask the browser for its cookies: {error}"))?;
    while let Some(message) = socket.next().await {
        let message =
            message.map_err(|error| format!("The browser's debugging socket failed: {error}"))?;
        let Message::Text(text) = message else {
            continue;
        };
        let reply: Value = match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if reply.get("id").and_then(Value::as_i64) != Some(1) {
            continue;
        }
        if let Some(error) = reply.get("error") {
            return Err(format!("The browser refused its cookies: {error}"));
        }
        let cookies = reply
            .get("result")
            .and_then(|result| result.get("cookies"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        socket.close(None).await.ok();
        return Ok(cookies);
    }
    Err("The browser closed its debugging socket before answering.".into())
}

/// Poll the DevTools HTTP endpoint until a page target's websocket URL is
/// available, then return it.
async fn cdp_target(port: u16) -> Result<String, String> {
    let client = reqwest::Client::new();
    let list = format!("http://127.0.0.1:{port}/json");
    let version = format!("http://127.0.0.1:{port}/json/version");
    let deadline = std::time::Instant::now() + LAUNCH_DEADLINE;
    let mut last = String::from("the browser did not open its debugging port");
    while std::time::Instant::now() < deadline {
        if let Ok(response) = client.get(&list).send().await
            && let Ok(targets) = response.json::<Value>().await
            && let Some(url) = targets.as_array().and_then(|targets| {
                targets
                    .iter()
                    .find(|target| target.get("type").and_then(Value::as_str) == Some("page"))
                    .and_then(|target| target.get("webSocketDebuggerUrl"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
        {
            return Ok(url);
        }
        // No page target yet; the browser-level socket can answer too.
        if let Ok(response) = client.get(&version).send().await
            && let Ok(info) = response.json::<Value>().await
            && let Some(url) = info.get("webSocketDebuggerUrl").and_then(Value::as_str)
        {
            last = format!("only a browser-level target at {url}");
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    Err(format!(
        "Could not reach the browser's debugging port: {last}."
    ))
}

/// A temporary user-data directory for the throwaway instance, placed where
/// the browser's confinement allows it to be opened, and removed on drop.
struct Staging {
    path: PathBuf,
}

impl Staging {
    fn new(confinement: &Confinement) -> Result<Staging, String> {
        let base = match confinement {
            Confinement::Native => std::env::temp_dir(),
            // A snap can only open a profile under its own writable home.
            Confinement::Snap(name) => home().join("snap").join(name).join("common"),
        };
        let unique = format!(
            "hotline-cookie-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_nanos())
                .unwrap_or_default()
        );
        let path = base.join(unique);
        std::fs::create_dir_all(&path)
            .map_err(|error| format!("Could not make a staging directory: {error}"))?;
        Ok(Staging { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).ok();
    }
}

fn free_loopback_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("Could not allocate a loopback port: {error}"))?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| format!("Could not allocate a loopback port: {error}"))
}

// ---- Firefox: read the plaintext cookie store -----------------------------

fn read_firefox(candidate: &Candidate, profile_id: &str) -> Result<Vec<Value>, String> {
    let store = candidate.root.join(profile_id).join("cookies.sqlite");
    if !store.exists() {
        return Err(format!("{} has no profile {profile_id:?}.", candidate.name));
    }
    firefox_cookies(&store)
}

/// Read `moz_cookies` and map each row to the same CDP cookie shape the
/// Chromium path produces, so the container consumes both the same way. The
/// database is opened read-only and immutable, so a running Firefox holding
/// the file does not block the read.
fn firefox_cookies(store: &Path) -> Result<Vec<Value>, String> {
    use rusqlite::OpenFlags;
    let uri = format!(
        "file:{}?immutable=1",
        store.to_string_lossy().replace('?', "%3f")
    );
    let connection = rusqlite::Connection::open_with_flags(
        &uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|error| format!("Could not open the Firefox cookie store: {error}"))?;
    let mut statement = connection
        .prepare(
            "select host, name, value, path, expiry, isSecure, isHttpOnly, sameSite \
             from moz_cookies",
        )
        .map_err(|error| format!("Could not read the Firefox cookie store: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            let host: String = row.get(0)?;
            let name: String = row.get(1)?;
            let value: String = row.get(2)?;
            let path: String = row.get(3)?;
            let expiry: i64 = row.get(4).unwrap_or(0);
            let secure: bool = row.get::<_, i64>(5).unwrap_or(0) != 0;
            let http_only: bool = row.get::<_, i64>(6).unwrap_or(0) != 0;
            let same_site = match row.get::<_, i64>(7).unwrap_or(0) {
                1 => "Lax",
                2 => "Strict",
                _ => "None",
            };
            let mut cookie = json!({
                "name": name,
                "value": value,
                "domain": host,
                "path": path,
                "secure": secure,
                "httpOnly": http_only,
                "sameSite": same_site,
            });
            // A zero expiry in Firefox is a session cookie; anything else is
            // an absolute time in seconds, which is what CDP wants too.
            if expiry > 0 {
                cookie["expires"] = json!(expiry);
                cookie["session"] = json!(false);
            } else {
                cookie["session"] = json!(true);
            }
            Ok(cookie)
        })
        .map_err(|error| format!("Could not read the Firefox cookie store: {error}"))?;
    let mut cookies = Vec::new();
    for row in rows {
        cookies.push(row.map_err(|error| error.to_string())?);
    }
    Ok(cookies)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sites_group_by_registrable_domain_and_drop_values() {
        let cookies = vec![
            json!({"domain": ".example.com", "name": "a", "value": "secret1"}),
            json!({"domain": "example.com", "name": "b", "value": "secret2"}),
            json!({"domain": ".other.test", "name": "c", "value": "secret3"}),
        ];
        let sites = sites(&cookies);
        assert_eq!(sites.len(), 2);
        // example.com has two, listed first; the value never appears.
        assert_eq!(sites[0].domain, "example.com");
        assert_eq!(sites[0].cookies, 2);
        assert_eq!(sites[1].domain, "other.test");
        let rendered = serde_json::to_string(&sites).unwrap();
        assert!(!rendered.contains("secret"));
    }

    #[test]
    fn a_firefox_store_reads_as_cdp_cookies() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("cookies.sqlite");
        let connection = rusqlite::Connection::open(&store).unwrap();
        connection
            .execute_batch(
                "create table moz_cookies (host text, name text, value text, path text, \
                 expiry integer, isSecure integer, isHttpOnly integer, sameSite integer); \
                 insert into moz_cookies values \
                 ('.example.com','sid','token-value','/',2000000000,1,1,1), \
                 ('accounts.example.com','csrf','v2','/',0,0,0,2);",
            )
            .unwrap();
        let cookies = firefox_cookies(&store).unwrap();
        assert_eq!(cookies.len(), 2);
        let sid = &cookies[0];
        assert_eq!(sid["name"], "sid");
        assert_eq!(sid["domain"], ".example.com");
        assert_eq!(sid["httpOnly"], true);
        assert_eq!(sid["secure"], true);
        assert_eq!(sid["sameSite"], "Lax");
        assert_eq!(sid["expires"], 2_000_000_000_i64);
        assert_eq!(sid["session"], false);
        // The zero-expiry row is a session cookie with no absolute expiry.
        let csrf = &cookies[1];
        assert_eq!(csrf["session"], true);
        assert_eq!(csrf.get("expires"), None);
        assert_eq!(csrf["sameSite"], "Strict");
        // And it reduces to two sites.
        let sites = sites(&cookies);
        assert_eq!(sites.len(), 2);
    }

    #[test]
    fn chromium_profiles_are_named_from_local_state_and_need_a_cookie_store() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("Local State"),
            json!({"profile": {"info_cache": {
                "Default": {"name": "Personal"},
                "Profile 1": {"name": "Work"}
            }}})
            .to_string(),
        )
        .unwrap();
        // Default has a cookie store; Profile 1 is named but has none.
        std::fs::create_dir_all(root.join("Default").join("Network")).unwrap();
        std::fs::write(root.join("Default").join("Network").join("Cookies"), b"x").unwrap();
        let profiles = chromium_profiles(root);
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].id, "Default");
        assert_eq!(profiles[0].name, "Personal");
    }

    #[test]
    fn select_keeps_only_the_ticked_sites() {
        let cookies = vec![
            json!({"domain": ".example.com", "name": "a", "value": "1"}),
            json!({"domain": "sub.example.com", "name": "b", "value": "2"}),
            json!({"domain": ".other.test", "name": "c", "value": "3"}),
        ];
        let wanted: std::collections::HashSet<&str> = ["example.com"].into_iter().collect();
        let kept: Vec<Value> = cookies
            .into_iter()
            .filter(|cookie| {
                cookie
                    .get("domain")
                    .and_then(Value::as_str)
                    .map(normalize_domain)
                    .is_some_and(|domain| wanted.contains(domain.as_str()))
            })
            .collect();
        // `.example.com` matches; `sub.example.com` is a different site.
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0]["name"], "a");
    }

    /// Proves the shipped path against a real browser on the host: launch it
    /// headless on a copy of its profile and read its own cookies over CDP.
    /// Ignored by default because it needs a browser installed and signed in;
    /// run with `HOTLINE_COOKIE_TEST_BROWSER=chrome … -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "needs a real browser on the host"]
    async fn reads_a_real_browser() {
        let browser =
            std::env::var("HOTLINE_COOKIE_TEST_BROWSER").unwrap_or_else(|_| "chrome".into());
        let profile =
            std::env::var("HOTLINE_COOKIE_TEST_PROFILE").unwrap_or_else(|_| "Default".into());
        let found = detect();
        eprintln!(
            "detected: {:?}",
            found
                .iter()
                .map(|b| (&b.id, b.profiles.len()))
                .collect::<Vec<_>>()
        );
        let sites = preview(&browser, &profile).await.expect("preview");
        let total: u32 = sites.iter().map(|site| site.cookies).sum();
        eprintln!(
            "{browser}/{profile}: {} sites, {total} cookies",
            sites.len()
        );
        // A signed-in browser has cookies; the values are never printed.
        assert!(
            !sites.is_empty(),
            "no cookies read from {browser}/{profile}"
        );
    }
}
