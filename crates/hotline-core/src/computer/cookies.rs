//! Reading cookies from the browsers on the host, so the operator can hand a
//! teammate's computer the sessions it is signed in to.
//!
//! This is host-side and operator-only. Nothing here is an agent tool, and
//! nothing the model says reaches it: the wire commands that call it are
//! gated to the desk seat, the same as adding a mount. The agent cannot pull
//! its own cookies; it receives exactly the sites the operator ticked.
//!
//! The reading is done by the browser, not by us. For a Chromium browser the
//! desk launches it headless on a *copy* of its profile and asks it for its
//! own cookies over the DevTools protocol (`Storage.getCookies`), so the
//! browser decrypts with its own key and the result is uniform across Chrome,
//! Edge, Brave and Chromium and every version, app-bound encryption included.
//! We never touch the cookie crypto. The protocol runs over a pipe between the
//! desk and that browser (`--remote-debugging-pipe`, see [`devtools`]), never
//! a port, so no other process on the machine can connect and ask the
//! instance for the jar while it runs. Windows has no such pipe here yet, so
//! there a Chromium browser is listed as one Hotline cannot read, rather than
//! opened on a port. Firefox keeps its cookies in a plaintext SQLite file, so
//! it is read directly, no launch.
//!
//! A profile *copy* is used for two reasons: a second instance cannot open a
//! profile another process holds the lock on, and the operator's live browser
//! must not be disturbed. The copy is in a directory only the person's user
//! can open, from the moment it is made. A snap-confined browser can only
//! open a profile inside its own writable tree, so the copy for one goes
//! under `~/snap/<name>/…` rather than the system temp dir.

#[cfg(unix)]
mod devtools;
pub(crate) mod lending;

use crate::contract::{BrowserProfile, CookieSite, HostBrowser};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::time::Duration;
#[cfg(unix)]
use tokio::process::Command;

/// Cookies are read from a throwaway headless instance; if the browser has
/// not answered for them by now something is wrong, and we stop rather than
/// hang the operator's import.
#[cfg(unix)]
const LAUNCH_DEADLINE: Duration = Duration::from_secs(20);

/// Whether this desk can read a Chromium profile at all. It does so only over
/// the DevTools pipe, which Hotline wires up on Unix. Windows needs inherited
/// handles for it, which it does not have yet, and the debugging port it used
/// before is not opened instead: there a Chromium browser is listed as one
/// Hotline cannot read.
const CHROMIUM_READABLE: bool = cfg!(unix);

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
    /// the confinement the browser runs under. Snaps are a Linux packaging.
    #[cfg(target_os = "linux")]
    Snap(String),
}

/// One browser Hotline knows how to read, whether or not it is installed.
#[derive(Clone)]
pub(crate) struct Candidate {
    id: &'static str,
    name: &'static str,
    family: Family,
    /// Absolute binaries to try in order; the first that exists is used.
    binaries: Vec<PathBuf>,
    /// The user-data directory (Chromium) or the profiles root (Firefox).
    root: PathBuf,
    /// Where a profile copy may live; only a Unix desk makes one.
    #[cfg_attr(
        not(unix),
        expect(dead_code, reason = "Windows reads no Chromium profile copy yet")
    )]
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
/// Windows names its browsers by their install paths and has no use for it.
#[cfg(not(target_os = "windows"))]
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
pub(crate) fn candidates() -> Vec<Candidate> {
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
        // An app is in /Applications, or in ~/Applications when installed
        // for this user only.
        let app = |bin: &str| {
            let system = format!("/Applications/{bin}");
            let user = home.join("Applications").join(bin);
            resolve_binary(&[system.as_str(), user.to_str().unwrap_or_default()])
        };
        let chromium = |id, name, sub: &str, bin: &str| Candidate {
            id,
            name,
            family: Family::Chromium,
            binaries: app(bin),
            root: support.join(sub),
            confinement: Confinement::Native,
        };
        for (id, name, sub, bin) in [
            (
                "chrome",
                "Google Chrome",
                "Google/Chrome",
                "Google Chrome.app/Contents/MacOS/Google Chrome",
            ),
            (
                "chrome-beta",
                "Google Chrome Beta",
                "Google/Chrome Beta",
                "Google Chrome Beta.app/Contents/MacOS/Google Chrome Beta",
            ),
            (
                "chrome-dev",
                "Google Chrome Dev",
                "Google/Chrome Dev",
                "Google Chrome Dev.app/Contents/MacOS/Google Chrome Dev",
            ),
            (
                "chrome-canary",
                "Google Chrome Canary",
                "Google/Chrome Canary",
                "Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
            ),
            (
                "chromium",
                "Chromium",
                "Chromium",
                "Chromium.app/Contents/MacOS/Chromium",
            ),
            (
                "brave",
                "Brave",
                "BraveSoftware/Brave-Browser",
                "Brave Browser.app/Contents/MacOS/Brave Browser",
            ),
            (
                "edge",
                "Microsoft Edge",
                "Microsoft Edge",
                "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            ),
            (
                "edge-beta",
                "Microsoft Edge Beta",
                "Microsoft Edge Beta",
                "Microsoft Edge Beta.app/Contents/MacOS/Microsoft Edge Beta",
            ),
            (
                "edge-dev",
                "Microsoft Edge Dev",
                "Microsoft Edge Dev",
                "Microsoft Edge Dev.app/Contents/MacOS/Microsoft Edge Dev",
            ),
            (
                "edge-canary",
                "Microsoft Edge Canary",
                "Microsoft Edge Canary",
                "Microsoft Edge Canary.app/Contents/MacOS/Microsoft Edge Canary",
            ),
            ("arc", "Arc", "Arc/User Data", "Arc.app/Contents/MacOS/Arc"),
            (
                "vivaldi",
                "Vivaldi",
                "Vivaldi",
                "Vivaldi.app/Contents/MacOS/Vivaldi",
            ),
        ] {
            out.push(chromium(id, name, sub, bin));
        }
        // profiles.ini sits in the Firefox folder itself; its profiles are
        // under Profiles/, and the paths it lists say so.
        out.push(Candidate {
            id: "firefox",
            name: "Firefox",
            family: Family::Firefox,
            binaries: app("Firefox.app/Contents/MacOS/firefox"),
            root: support.join("Firefox"),
            confinement: Confinement::Native,
        });
    }

    // Windows. Chromium data under Local AppData\<vendor>\<browser>\User Data,
    // Firefox under Roaming\Mozilla\Firefox.
    #[cfg(target_os = "windows")]
    {
        let local = std::env::var_os("LOCALAPPDATA")
            .map_or_else(|| home.join("AppData/Local"), PathBuf::from);
        let roaming =
            std::env::var_os("APPDATA").map_or_else(|| home.join("AppData/Roaming"), PathBuf::from);
        let program_files = std::env::var_os("PROGRAMFILES")
            .map_or_else(|| PathBuf::from("C:/Program Files"), PathBuf::from);
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

    /// Whether the host refuses to let us read this browser's data. macOS
    /// keeps an app out of another app's data without Full Disk Access: the
    /// folder is there, but listing it or opening a file in it fails with
    /// "Operation not permitted", which reads as no profiles at all.
    fn refused(&self) -> bool {
        let probe = match self.family {
            Family::Chromium => std::fs::read_dir(&self.root).map(drop),
            Family::Firefox => std::fs::read(self.root.join("profiles.ini")).map(drop),
        };
        matches!(probe, Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied)
    }

    /// The browser as the operator's picker lists it: with its profiles, or
    /// marked blocked when the host will not let us look; none when it has no
    /// profile with cookies, or is not installed.
    fn listed(&self) -> Option<HostBrowser> {
        self.listed_where(CHROMIUM_READABLE)
    }

    /// [`Candidate::listed`] on a desk that can, or cannot, read Chromium.
    /// One that cannot lists a Chromium browser with profiles as blocked,
    /// saying why, so no card offers it and no share reads it.
    fn listed_where(&self, chromium_readable: bool) -> Option<HostBrowser> {
        if self.first_binary().is_none() && self.family != Family::Firefox {
            return None;
        }
        let profiles = self.profiles();
        if self.family == Family::Chromium && !chromium_readable {
            return (!profiles.is_empty()).then(|| HostBrowser {
                id: self.id.to_owned(),
                name: self.name.to_owned(),
                family: self.family.wire().to_owned(),
                profiles: Vec::new(),
                blocked: true,
                blocked_reason: Some(unreadable_here(self.name)),
            });
        }
        let blocked = profiles.is_empty() && self.root.is_dir() && self.refused();
        (!profiles.is_empty() || blocked).then(|| HostBrowser {
            id: self.id.to_owned(),
            name: self.name.to_owned(),
            family: self.family.wire().to_owned(),
            profiles,
            blocked,
            blocked_reason: blocked.then(|| access_notice(self.name, None)),
        })
    }
}

/// Both the chat and the picker name the process whose access must change.
/// The dev runner launches a bare `hotline-app-dev`, not Hotline.app.
pub(crate) fn access_notice(browser: &str, site: Option<&str>) -> String {
    let session = site.map_or_else(
        || "your sessions".into(),
        |site| format!("your session for {site}"),
    );
    let remedy = if cfg!(target_os = "macos") {
        let executable = std::env::current_exe().unwrap_or_default();
        let app = access_app_name(&executable);
        format!(
            "Allow {app} in System Settings → Privacy & Security → Full Disk Access, then reopen Hotline."
        )
    } else {
        "Allow Hotline to read this browser's data, then reopen Hotline.".into()
    };
    format!("Hotline can't read {browser}'s cookies, so it can't offer {session}. {remedy}")
}

/// What a desk that cannot read Chromium (Windows, for now) says of one.
fn unreadable_here(browser: &str) -> String {
    format!(
        "Hotline can't read {browser}'s cookies on Windows yet, so it can't offer your sessions from it. Sign in on the teammate's browser instead."
    )
}

fn access_app_name(executable: &Path) -> String {
    executable
        .ancestors()
        .find(|path| path.extension().is_some_and(|extension| extension == "app"))
        .and_then(Path::file_stem)
        .or_else(|| executable.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("Hotline")
        .to_owned()
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

/// The browsers on this host that have at least one profile with cookies,
/// and those the host will not let us look into, so the operator is told why.
pub fn detect() -> Vec<HostBrowser> {
    listed(&candidates())
}

/// [`detect`] over the browsers given, as the room asks it of its own
/// candidates (a test's fixtures among them).
pub(crate) fn listed(candidates: &[Candidate]) -> Vec<HostBrowser> {
    candidates.iter().filter_map(Candidate::listed).collect()
}

fn find(browser_id: &str) -> Result<Candidate, String> {
    candidates()
        .into_iter()
        .find(|candidate| candidate.id == browser_id)
        .ok_or_else(|| format!("No browser named {browser_id:?} on this machine."))
}

/// Every live cookie in one profile, as CDP cookie objects — the shape the
/// container's browser accepts back. This is the only function that opens a
/// value; its callers reduce it to a preview or hand the selection to the
/// container without it passing through anything the model can read.
async fn read_all(browser_id: &str, profile_id: &str) -> Result<Vec<Value>, String> {
    let candidate = find(browser_id)?;
    let cookies = match candidate.family {
        Family::Chromium => read_chromium(&candidate, profile_id).await,
        Family::Firefox => read_firefox(&candidate, profile_id),
    }?;
    Ok(unexpired(cookies, unix_now()))
}

/// Only what the browser would still send. A cookie past its expiry is one
/// the browser drops on its next look and a site would refuse anyway, so it
/// is left behind before the preview counts it or the computer takes it; a
/// session cookie has no expiry and stays. Firefox keeps expired rows until
/// its own sweep, and a profile that has not been opened in a while is
/// mostly those.
fn unexpired(cookies: Vec<Value>, now: i64) -> Vec<Value> {
    cookies
        .into_iter()
        .filter(
            |cookie| match cookie.get("expires").and_then(Value::as_f64) {
                Some(expires) if expires > 0.0 => expires > now as f64,
                _ => true,
            },
        )
        .collect()
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        })
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

/// Validate the laptop's selected sites before starting a computer or writing
/// any record. Values are forwarded only to that computer, never to the log.
pub(crate) fn validate_transfer(
    transfer: &crate::contract::CookieTransfer,
) -> Result<Vec<Value>, String> {
    if [
        &transfer.source_id,
        &transfer.browser_id,
        &transfer.profile_id,
    ]
    .iter()
    .any(|id| id.is_empty() || id.len() > 256)
        || transfer.cookies.is_empty()
        || transfer.cookies.len() > 10000
        || transfer.domains.is_empty()
        || transfer.domains.len() > 1000
        || serde_json::to_vec(transfer).map_or(true, |bytes| bytes.len() > 8 * 1024 * 1024)
    {
        return Err("The cookie transfer is empty or too large.".into());
    }
    for cookie in &transfer.cookies {
        let domain = cookie["domain"].as_str().ok_or("A cookie has no domain.")?;
        if !transfer.domains.contains(&normalize_domain(domain)) {
            return Err("A cookie is outside the sites selected for import.".into());
        }
        if ["name", "value", "path"]
            .iter()
            .any(|field| cookie[field].as_str().is_none())
        {
            return Err("A cookie is missing its name, value or path.".into());
        }
    }
    let cookies = transfer
        .cookies
        .iter()
        .map(|cookie| {
            let mut kept = serde_json::Map::new();
            for field in [
                "domain", "name", "value", "path", "expires", "httpOnly", "secure", "sameSite",
            ] {
                if let Some(value) = cookie.get(field) {
                    kept.insert(field.into(), value.clone());
                }
            }
            Value::Object(kept)
        })
        .collect();
    Ok(unexpired(cookies, unix_now()))
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

#[cfg(not(unix))]
async fn read_chromium(candidate: &Candidate, _profile_id: &str) -> Result<Vec<Value>, String> {
    Err(unreadable_here(candidate.name))
}

/// What one launch of the throwaway instance adds to the shipped one. Only
/// tests change it: a mock keychain for a profile of their own, or a shorter
/// deadline.
#[cfg(unix)]
struct Launch<'a> {
    extra: &'a [&'a str],
    deadline: Duration,
}

#[cfg(unix)]
const SHIPPED: Launch<'static> = Launch {
    extra: &[],
    deadline: LAUNCH_DEADLINE,
};

#[cfg(unix)]
async fn read_chromium(candidate: &Candidate, profile_id: &str) -> Result<Vec<Value>, String> {
    read_chromium_with(candidate, profile_id, &SHIPPED).await
}

#[cfg(unix)]
async fn read_chromium_with(
    candidate: &Candidate,
    profile_id: &str,
    launch: &Launch<'_>,
) -> Result<Vec<Value>, String> {
    let binary = candidate
        .first_binary()
        .ok_or_else(|| format!("{} is not installed on this machine.", candidate.name))?
        .to_path_buf();
    if chromium_cookie_store(&candidate.root, profile_id).is_none() {
        return Err(format!("{} has no profile {profile_id:?}.", candidate.name));
    }
    let staging = Staging::new(&candidate.confinement)?;
    copy_chromium_profile(&candidate.root, profile_id, staging.path())?;

    let mut command = Command::new(&binary);
    command
        .args(chromium_args(staging.path(), profile_id))
        .args(launch.extra)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        // Its own process group, so its helpers can be stopped with it.
        .process_group(0)
        .kill_on_drop(true);
    let (mut child, mut pipe) = devtools::spawn(&mut command)
        .map_err(|error| format!("Could not launch {}: {error}", candidate.name))?;
    // Declared after `child`, so that if this read is dropped part way the
    // group is killed while the browser is still unreaped.
    let mut launched = Launched {
        group: child.id().and_then(|id| libc::pid_t::try_from(id).ok()),
        staging: Some(staging),
    };

    let result = tokio::time::timeout(launch.deadline, all_cookies(&mut pipe)).await;
    drop(pipe);
    launched.kill();
    child.kill().await.ok();
    // The staging copy goes here regardless of the outcome.
    drop(launched);
    match result {
        Ok(cookies) => cookies,
        Err(_) => Err(format!(
            "{} did not answer for its cookies in time.",
            candidate.name
        )),
    }
}

/// The throwaway browser's process group and its staged copy, which go
/// together however the read ends: answered, timed out, or dropped part way.
/// A helper killed a moment ago may still be writing into the copy, which
/// makes one removal fail or miss a file, so what is left is removed again
/// until it is gone, off the async thread, for a few seconds at most.
#[cfg(unix)]
struct Launched {
    group: Option<libc::pid_t>,
    staging: Option<Staging>,
}

#[cfg(unix)]
impl Launched {
    /// Kills the whole group, once. Only ever while its leader is unreaped,
    /// so the group's id cannot have been taken by another.
    fn kill(&mut self) {
        if let Some(group) = self.group.take() {
            // SAFETY: `killpg` takes a process group id and a signal and
            // touches no memory of ours.
            unsafe {
                libc::killpg(group, libc::SIGKILL);
            }
        }
    }
}

#[cfg(unix)]
impl Drop for Launched {
    fn drop(&mut self) {
        self.kill();
        let Some(staging) = self.staging.take() else {
            return;
        };
        let path = staging.path().to_path_buf();
        drop(staging);
        if std::fs::symlink_metadata(&path).is_err() {
            return;
        }
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while std::fs::symlink_metadata(&path).is_ok() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
                std::fs::remove_dir_all(&path).ok();
            }
        });
    }
}

/// How the throwaway instance is launched: headless on the staged copy, with
/// DevTools on the pipe only. No `--remote-debugging-port`, so nothing on the
/// machine can connect to it, and so no `--remote-allow-origins` either.
#[cfg(unix)]
fn chromium_args(staging: &Path, profile_id: &str) -> Vec<std::ffi::OsString> {
    let mut user_data = std::ffi::OsString::from("--user-data-dir=");
    user_data.push(staging);
    let mut args = vec![
        "--headless=new".into(),
        user_data,
        format!("--profile-directory={profile_id}").into(),
        devtools::PIPE_ARG.into(),
    ];
    args.extend(
        [
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-gpu",
            "--disable-sync",
            "--disable-background-networking",
            "--disable-features=Translate",
            "--no-sandbox",
            "about:blank",
        ]
        .map(Into::into),
    );
    args
}

/// Copy just what the browser needs to open the profile read-only and decrypt
/// its cookies: the key reference in `Local State`, and the profile's cookie
/// store and preferences. Caches and history are left behind, so the copy is
/// small and quick.
#[cfg(unix)]
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

/// Ask the browser for its whole cookie jar over the pipe. The browser has
/// already decrypted it; we only carry it. `Storage.getCookies` is the
/// browser-level call for the default context's jar, which is the profile it
/// was opened on, in the same cookie objects `Network.getAllCookies` gives.
#[cfg(unix)]
async fn all_cookies(pipe: &mut devtools::Pipe) -> Result<Vec<Value>, String> {
    let result = pipe
        .call("Storage.getCookies", json!({}))
        .await?
        .map_err(|error| format!("The browser refused its cookies: {error}"))?;
    Ok(result
        .get("cookies")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

/// A temporary user-data directory for the throwaway instance, placed where
/// the browser's confinement allows it to be opened, and removed on drop. It
/// is made 0700 in the one call that creates it, under a name no one can
/// guess, so no other user ever sees into it, not even for a moment.
#[cfg(unix)]
struct Staging {
    dir: tempfile::TempDir,
}

#[cfg(unix)]
impl Staging {
    fn new(confinement: &Confinement) -> Result<Staging, String> {
        use std::os::unix::fs::PermissionsExt;
        let base = match confinement {
            Confinement::Native => std::env::temp_dir(),
            // A snap can only open a profile under its own writable home.
            #[cfg(target_os = "linux")]
            Confinement::Snap(name) => home().join("snap").join(name).join("common"),
        };
        std::fs::create_dir_all(&base)
            .map_err(|error| format!("Could not make a staging directory: {error}"))?;
        let dir = tempfile::Builder::new()
            .prefix("hotline-cookie-")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir_in(&base)
            .map_err(|error| format!("Could not make a staging directory: {error}"))?;
        Ok(Staging { dir })
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }
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
    fn expired_cookies_are_left_behind_and_session_cookies_stay() {
        let now = 1_700_000_000;
        let cookies = vec![
            json!({"domain": "a.test", "name": "old", "expires": 1_600_000_000}),
            json!({"domain": "a.test", "name": "live", "expires": 1_800_000_000.5}),
            json!({"domain": "a.test", "name": "cdp-session", "expires": -1}),
            json!({"domain": "a.test", "name": "no-expiry"}),
            json!({"domain": "a.test", "name": "this-second", "expires": 1_700_000_000}),
        ];
        let kept: Vec<String> = unexpired(cookies, now)
            .iter()
            .map(|cookie| cookie["name"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(kept, ["live", "cdp-session", "no-expiry"]);
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
        let wanted: std::collections::HashSet<&str> = std::iter::once("example.com").collect();
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

    fn installed(family: Family, root: &Path) -> Candidate {
        Candidate {
            id: "test",
            name: "Test Browser",
            family,
            // Any file that exists stands in for the browser's binary.
            binaries: vec![std::env::current_exe().unwrap()],
            root: root.to_path_buf(),
            confinement: Confinement::Native,
        }
    }

    /// A Chromium browser with a profile is listed with it where its pipe is
    /// wired; on Windows it is listed as one Hotline can't read, offering no
    /// profile. A Firefox one is listed with its profile everywhere.
    #[test]
    fn a_browser_is_listed_with_its_profiles_and_left_out_without_any() {
        let dir = tempfile::tempdir().unwrap();
        let candidate = installed(Family::Chromium, dir.path());
        assert_eq!(candidate.listed(), None);

        std::fs::create_dir_all(dir.path().join("Default")).unwrap();
        std::fs::write(dir.path().join("Default").join("Cookies"), b"x").unwrap();
        let listed = candidate.listed().unwrap();
        if CHROMIUM_READABLE {
            assert_eq!(listed.profiles.len(), 1);
            assert!(!listed.blocked);
        } else {
            assert_eq!(listed.profiles, [] as [BrowserProfile; 0]);
            assert!(listed.blocked);
        }

        let dir = tempfile::tempdir().unwrap();
        let firefox = installed(Family::Firefox, dir.path());
        assert_eq!(firefox.listed(), None);
        std::fs::write(
            dir.path().join("profiles.ini"),
            "[Profile0]\nName=default-release\nIsRelative=1\nPath=Profiles/abc\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("Profiles/abc")).unwrap();
        std::fs::write(dir.path().join("Profiles/abc/cookies.sqlite"), b"x").unwrap();
        let listed = firefox.listed().unwrap();
        assert_eq!(listed.profiles.len(), 1);
        assert!(!listed.blocked);
    }

    /// macOS refuses an app another app's data without Full Disk Access; a
    /// folder the desk may not list stands in for that here.
    #[cfg(unix)]
    #[test]
    fn a_browser_whose_data_the_host_refuses_is_listed_as_blocked() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Brave-Browser");
        std::fs::create_dir_all(root.join("Default")).unwrap();
        std::fs::write(root.join("Default").join("Cookies"), b"x").unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o000)).unwrap();
        let refused = std::fs::read_dir(&root).is_err();

        let listed = installed(Family::Chromium, &root).listed();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        if !refused {
            return; // Running as root: nothing is refused, so there is nothing to prove.
        }
        let listed = listed.unwrap();
        assert!(listed.blocked);
        assert_eq!(listed.profiles, [] as [BrowserProfile; 0]);
        assert_eq!(serde_json::to_value(&listed).unwrap()["blocked"], true);
    }

    #[test]
    fn firefox_profiles_are_found_from_profiles_ini_in_the_firefox_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("profiles.ini"),
            "[Profile0]\nName=default-release\nIsRelative=1\nPath=Profiles/abc.default-release\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("Profiles/abc.default-release")).unwrap();
        std::fs::write(
            root.join("Profiles/abc.default-release/cookies.sqlite"),
            b"x",
        )
        .unwrap();

        let listed = installed(Family::Firefox, root).listed().unwrap();
        assert_eq!(listed.profiles.len(), 1);
        assert_eq!(listed.profiles[0].id, "Profiles/abc.default-release");
        assert_eq!(listed.profiles[0].name, "default-release");
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

    /// A stand-in Chromium for the launch: a script that writes its
    /// arguments to `args`, copies whatever arrives on descriptor 3 to
    /// `request`, and once something has arrived answers `answer` on
    /// descriptor 4, NUL-ended, as `--remote-debugging-pipe` does. Launched
    /// with a debugging port instead it never answers. The arguments file
    /// appears only when whole.
    #[cfg(unix)]
    pub(super) struct FakeChromium {
        pub(super) binary: PathBuf,
        pub(super) args: PathBuf,
        pub(super) request: PathBuf,
    }

    #[cfg(unix)]
    impl FakeChromium {
        pub(super) fn new(dir: &Path, answer: &Value) -> FakeChromium {
            let fake = FakeChromium {
                binary: dir.join("browser"),
                args: dir.join("browser-args"),
                request: dir.join("browser-request"),
            };
            let answer = answer.to_string();
            assert!(!answer.contains('\''));
            crate::computer::fixtures::write_script(
                dir,
                "browser",
                &format!(
                    "#!/bin/sh\n\
                     printf '%s\\n' \"$@\" > '{args}.partial'\n\
                     mv '{args}.partial' '{args}'\n\
                     cat <&3 > '{request}' &\n\
                     while [ ! -s '{request}' ]; do sleep 0.05; done\n\
                     printf '%s\\000' '{answer}' >&4\n\
                     exec sleep 30\n",
                    args = fake.args.display(),
                    request = fake.request.display(),
                ),
            );
            fake
        }

        pub(super) fn arguments(&self) -> Vec<String> {
            std::fs::read_to_string(&self.args)
                .unwrap()
                .lines()
                .map(str::to_owned)
                .collect()
        }
    }

    /// H-04: the throwaway browser holding the person's decrypted cookies
    /// is reached over a pipe from the desk, never a loopback port
    /// another process could connect to.
    #[cfg(unix)]
    #[tokio::test]
    async fn read_chromium_opens_no_debugging_port() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("profile");
        std::fs::create_dir_all(root.join("Default")).unwrap();
        std::fs::write(root.join("Default").join("Cookies"), b"x").unwrap();
        let fake = FakeChromium::new(
            dir.path(),
            &json!({"id": 1, "result": {"cookies": [
                {"name": "sid", "domain": ".example.com", "value": "over-the-pipe"}
            ]}}),
        );
        let candidate = Candidate {
            binaries: vec![fake.binary.clone()],
            ..installed(Family::Chromium, &root)
        };

        let cookies = read_chromium(&candidate, "Default").await.unwrap();

        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0]["value"], "over-the-pipe");
        let arguments = fake.arguments();
        assert!(arguments.iter().any(|arg| arg == "--remote-debugging-pipe"));
        for arg in &arguments {
            assert!(!arg.starts_with("--remote-debugging-port"), "{arg}");
            assert!(!arg.starts_with("--remote-allow-origins"), "{arg}");
        }
        let request = std::fs::read(&fake.request).unwrap();
        assert_eq!(request.last(), Some(&0), "a pipe message ends in NUL");
        let request: Value = serde_json::from_slice(&request[..request.len() - 1]).unwrap();
        assert_eq!(request["id"], 1);
        assert_eq!(request["method"], "Storage.getCookies");
    }

    /// H-04: the profile copy is private to the person's user from the call
    /// that makes it, and gone once the read is over.
    #[cfg(unix)]
    #[test]
    fn staging_dir_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let staging = Staging::new(&Confinement::Native).unwrap();
        let path = staging.path().to_path_buf();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "{mode:o}");
        assert!(path.starts_with(std::env::temp_dir()));
        drop(staging);
        assert!(!path.exists());
    }

    /// Where the pipe is not wired (Windows), a Chromium browser with
    /// profiles is listed as one Hotline cannot read, so it is offered to no
    /// card and read by no share; Firefox is unaffected.
    #[test]
    fn a_chromium_browser_is_blocked_where_its_pipe_is_not_wired() {
        let dir = tempfile::tempdir().unwrap();
        let candidate = installed(Family::Chromium, dir.path());
        assert_eq!(candidate.listed_where(false), None);

        std::fs::create_dir_all(dir.path().join("Default")).unwrap();
        std::fs::write(dir.path().join("Default").join("Cookies"), b"x").unwrap();
        let listed = candidate.listed_where(false).unwrap();
        assert!(listed.blocked);
        assert_eq!(listed.profiles, [] as [BrowserProfile; 0]);
        assert_eq!(
            listed.blocked_reason.as_deref(),
            Some(unreadable_here("Test Browser").as_str())
        );
        assert!(!candidate.listed_where(true).unwrap().blocked);
        assert_eq!(candidate.listed().unwrap().blocked, !CHROMIUM_READABLE);
        let discovery = lending::discover(vec![candidate], &lending::Gate::default());
        assert_eq!(discovery.sources.is_empty(), !CHROMIUM_READABLE);
    }

    /// The pipe against a real Chromium, on an EMPTY profile of its own (a
    /// mock keychain, never the person's profile or keychain): it sets one
    /// cookie, reads the jar back the way a share does, and the browser
    /// listens on no TCP port meanwhile. Run with
    /// `HOTLINE_COOKIE_PIPE_BROWSER=/path/to/chromium … -- --ignored`.
    #[cfg(unix)]
    #[tokio::test]
    #[ignore = "needs a Chromium binary on the host"]
    async fn a_real_chromium_answers_over_the_pipe_on_an_empty_profile() {
        let binary = std::env::var("HOTLINE_COOKIE_PIPE_BROWSER")
            .expect("HOTLINE_COOKIE_PIPE_BROWSER names a Chromium binary");
        let staging = Staging::new(&Confinement::Native).unwrap();
        let mut command = Command::new(binary);
        command
            .args(chromium_args(staging.path(), "Default"))
            .arg("--use-mock-keychain")
            .arg("--password-store=basic")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let (mut child, mut pipe) = devtools::spawn(&mut command).unwrap();
        let set = tokio::time::timeout(
            LAUNCH_DEADLINE,
            pipe.call(
                "Storage.setCookies",
                json!({"cookies": [{
                    "name": "probe", "value": "pipe-probe", "domain": "example.test", "path": "/",
                    "expires": unix_now() + 3600
                }]}),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(set.is_ok(), "{set:?}");
        let cookies = tokio::time::timeout(LAUNCH_DEADLINE, all_cookies(&mut pipe))
            .await
            .unwrap()
            .unwrap();
        assert!(
            cookies
                .iter()
                .any(|cookie| cookie["name"] == "probe" && cookie["value"] == "pipe-probe"),
            "{cookies:?}"
        );
        let pid = child.id().unwrap().to_string();
        let listening = std::process::Command::new("lsof")
            .args(["-nP", "-a", "-p", &pid, "-iTCP", "-sTCP:LISTEN"])
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&listening.stdout),
            "",
            "the browser listens on no TCP port"
        );
        drop(pipe);
        child.kill().await.ok();
    }
}
