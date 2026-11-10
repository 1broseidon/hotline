//! Signed updates use the same Tauri plugin and GitHub manifest as Prism.
//! The desktop owns installation; the room only provides an idle restart lease.
//!
//! A desk is on one of two tracks. Stable reads the manifest of the newest
//! final release and is offered final releases only. Beta reads a second
//! manifest, which holds the newest release of either kind, and is offered
//! betas too. Both addresses are fixed in the build; the desk stores only
//! which of the two it reads. Neither track is ever offered a version that
//! is not newer than the one running, so leaving Beta keeps the build the
//! desk has until a final release passes it.

use hotline_core::desk::Desk;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio_util::sync::CancellationToken;

const EVENT: &str = "hotline://update";
const INTERVAL: u64 = 6 * 60 * 60;
const FIRST_CHECK: u64 = 20;
const MAX_NOTES: usize = 16_000;
const MAX_CACHE: u64 = 32_000;
const CHECK_FAILED: &str = "Update check failed";
/// The Beta track's manifest: one file on a branch that holds nothing else,
/// replaced by `scripts/beta_pointer.py` after each release is published.
/// The Stable track's address is the plugin's own, in `tauri.conf.json`.
const BETA_MANIFEST: &str =
    "https://raw.githubusercontent.com/1Broseidon/hotline/update-tracks/beta.json";
const TRACK_FILE: &str = "update-track.json";
const MAX_TRACK_FILE: u64 = 1_000;

/// Which releases this desk is offered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Track {
    /// Final releases only.
    #[default]
    Stable,
    /// Final releases, and the betas cut from a release branch before them.
    Beta,
}

/// The desk's choice of track, in its data directory beside the shell's
/// other files. It names a track and nothing else: no address, no key.
#[derive(Serialize, Deserialize)]
struct TrackFile {
    track: Track,
}

/// A file that is missing, too long, or names anything but a track is Stable.
fn read_track(path: &Path) -> Track {
    let Ok(file) = fs::File::open(path) else {
        return Track::Stable;
    };
    let mut bytes = Vec::new();
    if file
        .take(MAX_TRACK_FILE + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > MAX_TRACK_FILE
    {
        return Track::Stable;
    }
    serde_json::from_slice::<TrackFile>(&bytes).map_or(Track::Stable, |file| file.track)
}

/// Replaces `path` whole, staged beside it, so a reader never sees half.
fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().expect("file has a parent"))?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
}
const INSTALL_FAILED: &str = "Installation failed";
const TRACK_NOT_SAVED: &str = "Couldn't save the track";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Available {
    version: String,
    notes: String,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
enum Phase {
    Idle,
    Checking,
    Downloading,
    Installing,
    Restarting,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    current: String,
    track: Track,
    available: Option<Available>,
    checked_at: Option<u64>,
    phase: Phase,
    downloaded: u64,
    total: Option<u64>,
    error: Option<Problem>,
    disabled_reason: Option<String>,
}

/// What went wrong, as the window shows it: a few words of ours, and the
/// words of whatever refused (the network, the package, the installer)
/// behind its disclosure. A refusal that is already a sentence of ours has
/// no detail.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Problem {
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

impl Problem {
    fn failed(label: &str, error: impl std::fmt::Display) -> Self {
        Self {
            message: label.into(),
            detail: Some(error.to_string()),
        }
    }
}

impl From<String> for Problem {
    fn from(message: String) -> Self {
        Self {
            message,
            detail: None,
        }
    }
}

impl From<&str> for Problem {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cache {
    current: String,
    /// The track the offer was found on. A cache from before tracks is Stable's.
    #[serde(default)]
    track: Track,
    checked_at: u64,
    available: Option<Available>,
}

struct Inner {
    status: Status,
    cancel: CancellationToken,
}

pub struct Updates {
    inner: Mutex<Inner>,
    operation: tokio::sync::Mutex<()>,
    cache: PathBuf,
    track_file: PathBuf,
    beta_manifest: String,
    target: Option<String>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// `beta.N`, the one pre-release the release workflow publishes.
fn is_beta(pre: &semver::Prerelease) -> bool {
    pre.as_str()
        .strip_prefix("beta.")
        .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether a desk on `track` running `current` is offered `candidate`. It
/// must be newer by semver on either track, which is what keeps a desk that
/// leaves Beta on its build: `0.49.0-beta.2` is newer than every `0.48`, and
/// older than `0.49.0`. Stable takes final releases only; Beta also takes
/// `beta.N`, and no other pre-release.
fn newer(current: &str, candidate: &str, track: Track) -> bool {
    match (
        semver::Version::parse(current),
        semver::Version::parse(candidate),
    ) {
        (Ok(current), Ok(candidate)) => {
            candidate > current
                && (candidate.pre.is_empty() || (track == Track::Beta && is_beta(&candidate.pre)))
        }
        _ => false,
    }
}

/// Explicit installer targets prevent the plugin's generic-platform fallback
/// from handing a deb installation an AppImage when a release is incomplete.
fn target_for(
    os: &str,
    arch: &str,
    bundle: Option<tauri::utils::config::BundleType>,
) -> Option<String> {
    use tauri::utils::config::BundleType;
    let suffix = match (os, bundle) {
        ("linux", Some(BundleType::Deb)) => "deb",
        ("linux", Some(BundleType::Rpm)) => "rpm",
        ("linux", Some(BundleType::AppImage)) => "appimage",
        ("macos", Some(BundleType::App | BundleType::Dmg)) => "app",
        ("windows", Some(BundleType::Nsis)) => "nsis",
        _ => return None,
    };
    if !matches!(
        (os, arch),
        ("windows", "x86_64") | ("linux" | "macos", "x86_64" | "aarch64")
    ) {
        return None;
    }
    Some(format!(
        "{}-{arch}-{suffix}",
        if os == "macos" { "darwin" } else { os }
    ))
}

fn next_check(checked: Option<u64>, at: u64) -> Duration {
    Duration::from_secs(match checked {
        Some(checked) if checked <= at => INTERVAL.saturating_sub(at - checked),
        _ => 0,
    })
}

fn read_cache(path: &Path, current: &str, track: Track, at: u64) -> Option<Cache> {
    let file = fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_CACHE {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_CACHE + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_CACHE {
        return None;
    }
    let cache: Cache = serde_json::from_slice(&bytes).ok()?;
    if cache.current != current || cache.track != track || cache.checked_at > at {
        return None;
    }
    if let Some(info) = &cache.available
        && (!newer(current, &info.version, track) || info.notes.len() > MAX_NOTES)
    {
        return None;
    }
    Some(cache)
}

impl Updates {
    pub fn new(root: &Path, current: String, development: bool) -> Self {
        let target = target_for(
            std::env::consts::OS,
            std::env::consts::ARCH,
            tauri::utils::platform::bundle_type(),
        );
        let disabled_reason = if development {
            Some("Updates are disabled in development builds.".to_string())
        } else if target.is_none() {
            Some("Update this installation from the release page.".to_string())
        } else {
            None
        };
        let cache = root.join("updater.json");
        let track_file = root.join(TRACK_FILE);
        let track = read_track(&track_file);
        let saved = if disabled_reason.is_none() {
            read_cache(&cache, &current, track, now())
        } else {
            None
        };
        Self {
            inner: Mutex::new(Inner {
                status: Status {
                    current,
                    track,
                    checked_at: saved.as_ref().map(|saved| saved.checked_at),
                    available: saved.and_then(|saved| saved.available),
                    phase: Phase::Idle,
                    downloaded: 0,
                    total: None,
                    error: None,
                    disabled_reason,
                },
                cancel: CancellationToken::new(),
            }),
            operation: tokio::sync::Mutex::new(()),
            cache,
            track_file,
            beta_manifest: BETA_MANIFEST.into(),
            target,
        }
    }

    /// The one address a desk on Beta reads, or none for Stable, which
    /// reads the address the plugin was configured with.
    fn manifest(&self, track: Track) -> Result<Option<tauri::Url>, Problem> {
        match track {
            Track::Stable => Ok(None),
            Track::Beta => self
                .beta_manifest
                .parse()
                .map(Some)
                .map_err(|error| Problem::failed(CHECK_FAILED, error)),
        }
    }

    /// Stores the choice, then forgets the offer and the check made on the
    /// other track: neither says anything about this one.
    fn set_track<R: Runtime>(&self, app: &AppHandle<R>, track: Track) -> Result<(), Problem> {
        if self.status().track == track {
            return Ok(());
        }
        let bytes = serde_json::to_vec(&TrackFile { track })
            .map_err(|error| Problem::failed(TRACK_NOT_SAVED, error))?;
        write_whole(&self.track_file, &bytes)
            .map_err(|error| Problem::failed(TRACK_NOT_SAVED, error))?;
        self.change(app, |status| {
            status.track = track;
            status.available = None;
            status.checked_at = None;
            status.error = None;
        });
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn status(&self) -> Status {
        self.lock().status.clone()
    }

    fn change<R: Runtime>(&self, app: &AppHandle<R>, change: impl FnOnce(&mut Status)) {
        let status = {
            let mut inner = self.lock();
            change(&mut inner.status);
            inner.status.clone()
        };
        let _ = app.emit(EVENT, status);
    }

    fn enabled(&self) -> Result<(), String> {
        self.status().disabled_reason.map_or(Ok(()), Err)
    }

    fn persist(&self) {
        let status = self.status();
        let Some(checked_at) = status.checked_at else {
            return;
        };
        let cache = Cache {
            current: status.current,
            track: status.track,
            checked_at,
            available: status.available,
        };
        // A failed cache write must not turn a successful check into a failed update.
        // It only means the next launch will check again.
        let result = serde_json::to_vec(&cache)
            .map_err(std::io::Error::from)
            .and_then(|bytes| write_whole(&self.cache, &bytes));
        if let Err(error) = result {
            eprintln!("[updater] could not remember check: {error}");
        }
    }
}

fn main_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("Updates belong to the main window.".into())
    }
}

#[tauri::command]
pub fn get_update_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Updates>,
) -> Result<Status, String> {
    main_window(&window)?;
    Ok(state.status())
}

/// Caller serializes this with installation. Persist check attempts too, so an
/// offline restart cannot hammer GitHub. A failed check retains the previous offer.
async fn check<R: Runtime>(app: &AppHandle<R>, state: &Updates) -> Result<Option<Update>, Problem> {
    state.enabled()?;
    state.change(app, |status| {
        status.phase = Phase::Checking;
        status.error = None;
    });
    let result: Result<Option<Update>, Problem> = async {
        let target = state.target.as_deref().ok_or("Unsupported installation.")?;
        let track = state.status().track;
        let mut builder = app
            .updater_builder()
            .target(target)
            .timeout(Duration::from_secs(30))
            .version_comparator(move |current, release| {
                newer(&current.to_string(), &release.version.to_string(), track)
            });
        if let Some(manifest) = state.manifest(track)? {
            builder = builder
                .endpoints(vec![manifest])
                .map_err(|error| Problem::failed(CHECK_FAILED, error))?;
        }
        let updater = builder
            .build()
            .map_err(|error| Problem::failed(CHECK_FAILED, error))?;
        let mut found = updater
            .check()
            .await
            .map_err(|error| Problem::failed(CHECK_FAILED, error))?;
        if let Some(update) = &mut found {
            validate_update(update, target)?;
            update.timeout = Some(Duration::from_mins(15));
        }
        Ok(found)
    }
    .await;
    state.change(app, |status| {
        status.phase = Phase::Idle;
        status.checked_at = Some(now());
        match &result {
            Ok(found) => {
                status.available = found.as_ref().map(|update| Available {
                    version: update.version.clone(),
                    notes: update
                        .body
                        .as_deref()
                        .unwrap_or_default()
                        .chars()
                        .take(4_000)
                        .collect(),
                });
            }
            Err(error) => status.error = Some(error.clone()),
        }
    });
    state.persist();
    result
}

/// The package each installer target downloads, as the release names it
/// (scripts/updater_manifest.py). An unknown target matches nothing.
fn package_suffix(target: &str) -> Option<&'static str> {
    Some(match target.rsplit_once('-')?.1 {
        "deb" => ".deb",
        "rpm" => ".rpm",
        "appimage" => ".AppImage",
        "app" => ".app.tar.gz",
        "nsis" => "-setup.exe",
        _ => return None,
    })
}

fn validate_update(update: &Update, target: &str) -> Result<(), String> {
    let matching = package_suffix(target).is_some_and(|suffix| {
        update.download_url.scheme() == "https"
            && update.download_url.path().ends_with(suffix)
            && update
                .raw_json
                .get("platforms")
                .and_then(|p| p.get(target))
                .is_some()
    });
    if !matching {
        return Err("This release does not include a matching update package. Open the release page instead.".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn check_update(window: tauri::WebviewWindow, app: AppHandle) -> Result<Status, Problem> {
    main_window(&window)?;
    let state = app.state::<Updates>();
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "An update operation is already running.")?;
    check(&app, &state).await?;
    Ok(state.status())
}

/// Moves the desk to a track and checks it at once, so the row shows what
/// that track offers. A check that fails leaves the choice stored and says
/// why, as any failed check does.
#[tauri::command]
pub async fn set_update_track(
    window: tauri::WebviewWindow,
    app: AppHandle,
    track: Track,
) -> Result<Status, Problem> {
    main_window(&window)?;
    let state = app.state::<Updates>();
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "An update operation is already running.")?;
    state.enabled()?;
    state.set_track(&app, track)?;
    let _ = check(&app, &state).await;
    Ok(state.status())
}

#[tauri::command]
pub fn cancel_update(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Updates>,
) -> Result<(), String> {
    main_window(&window)?;
    let inner = state.lock();
    if inner.status.phase != Phase::Downloading {
        return Err("Only a download can be cancelled.".into());
    }
    inner.cancel.cancel();
    Ok(())
}

#[tauri::command]
pub async fn install_update(
    window: tauri::WebviewWindow,
    app: AppHandle,
    version: String,
) -> Result<(), Problem> {
    main_window(&window)?;
    let state = app.state::<Updates>();
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "An update operation is already running.")?;
    state.enabled()?;
    let result = install(&app, &state, &version).await;
    if let Err(error) = &result {
        state.change(&app, |status| {
            status.phase = Phase::Idle;
            status.error = Some(error.clone());
        });
    }
    result
}

async fn install(app: &AppHandle, state: &Updates, version: &str) -> Result<(), Problem> {
    // Cached metadata is display-only. Recheck through the trusted endpoint,
    // and require the version the person actually reviewed.
    let update = check(app, state)
        .await?
        .ok_or("There is no newer update available.")?;
    if update.version != version {
        return Err("The release changed. Review the new release notes before installing.".into());
    }
    let held = app.state::<Arc<Desk>>().prepare_restart()?;
    let cancel = CancellationToken::new();
    state.lock().cancel = cancel.clone();
    state.change(app, |status| {
        status.phase = Phase::Downloading;
        status.downloaded = 0;
        status.total = None;
        status.error = None;
    });
    let mut downloaded = 0u64;
    let bytes = download_verified(&update, &cancel, |chunk, total| {
        downloaded = downloaded.saturating_add(chunk as u64);
        state.change(app, |status| {
            status.downloaded = downloaded;
            status.total = total;
        });
    })
    .await?;
    // Cancellation and crossing into installation are one decision under the
    // same lock; Cancel never reports success after the installer has begun.
    {
        let mut inner = state.lock();
        if cancel.is_cancelled() {
            return Err("Download cancelled. Hotline has not been changed.".into());
        }
        inner.status.phase = Phase::Installing;
    }
    state.change(app, |_| {});
    app.save_window_state(super::window_state_flags())
        .map_err(|error| Problem::failed(INSTALL_FAILED, error))?;
    let app = app.clone();
    #[cfg(target_os = "linux")]
    let package = state
        .target
        .as_deref()
        .and_then(super::linux_package::Package::for_target);
    tauri::async_runtime::spawn_blocking(move || {
        // This lease lives with the installer, even if its caller disappears.
        let _held = held;
        // A .deb or .rpm goes through the system's own pkexec, never the
        // plugin's PATH lookup and sudo fallbacks (see linux_package).
        #[cfg(target_os = "linux")]
        if let Some(package) = package {
            super::linux_package::install(package, &bytes)
                .map_err(|error| Problem::failed(INSTALL_FAILED, error))?;
            app.state::<Updates>()
                .change(&app, |status| status.phase = Phase::Restarting);
            app.restart();
        }
        update
            .install(bytes)
            .map_err(|error| Problem::failed(INSTALL_FAILED, error))?;
        app.state::<Updates>()
            .change(&app, |status| status.phase = Phase::Restarting);
        app.restart();
    })
    .await
    .map_err(|error| Problem::failed(INSTALL_FAILED, error))?
}

async fn download_verified(
    update: &Update,
    cancel: &CancellationToken,
    progress: impl FnMut(usize, Option<u64>),
) -> Result<Vec<u8>, Problem> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err("Download cancelled. Hotline has not been changed.".into()),
        result = update.download(progress, || {}) => result.map_err(|error| Problem::failed("Download failed", error)),
    }
}

use tauri_plugin_window_state::AppHandleExt;

pub fn start(app: AppHandle) {
    if app.state::<Updates>().enabled().is_err() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(FIRST_CHECK)).await;
        loop {
            let state = app.state::<Updates>();
            let wait = next_check(state.status().checked_at, now());
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
                continue;
            }
            if let Ok(_operation) = state.operation.try_lock() {
                let _ = check(&app, &state).await;
            }
            tokio::time::sleep(Duration::from_secs(FIRST_CHECK)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::net::TcpListener;
    use tauri::test::{mock_builder, mock_context, noop_assets};

    const PUBLIC: &str = include_str!("../tests/fixtures/updater/public.key");
    const SIGNATURE: &str = include_str!("../tests/fixtures/updater/signed-update.txt.sig");
    const PAYLOAD: &[u8] = include_bytes!("../tests/fixtures/updater/signed-update.txt");

    fn serve(body: Vec<u8>, code: u16) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = [0; 8192];
            assert!(socket.read(&mut request).unwrap() > 0);
            write!(socket, "HTTP/1.1 {code} Fixture\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            socket.write_all(&body).unwrap();
        });
        format!("http://{address}/latest.json")
    }

    fn app(endpoint: &str) -> tauri::App<tauri::test::MockRuntime> {
        app_at(endpoint, "0.1.0")
    }

    /// A build of `version` whose Stable manifest is `endpoint`.
    fn app_at(endpoint: &str, version: &str) -> tauri::App<tauri::test::MockRuntime> {
        let mut context = mock_context(noop_assets());
        context.package_info_mut().version = version.parse().unwrap();
        context.config_mut().plugins.0.insert(
            "updater".into(),
            json!({
                "pubkey": PUBLIC.trim(), "endpoints": [endpoint],
            }),
        );
        mock_builder()
            .plugin(tauri_plugin_updater::Builder::new().build())
            .build(context)
            .unwrap()
    }

    fn state(root: &Path) -> Updates {
        state_at(root, "0.1.0")
    }

    fn state_at(root: &Path, current: &str) -> Updates {
        let state = Updates::new(root, current.into(), false);
        // Mock runtime has no installed package. Only fixture checks/downloads are enabled.
        state.lock().status.disabled_reason = None;
        Updates {
            target: Some("linux-x86_64-deb".into()),
            ..state
        }
    }

    fn release(version: &str, target: &str) -> Vec<u8> {
        release_at(
            version,
            target,
            "https://github.com/1Broseidon/hotline/releases/download/desktop-v0.2.0/hotline.deb",
        )
    }

    fn release_at(version: &str, target: &str, url: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "version": version, "notes": "A fixture release.",
            "platforms": { target: {"url": url, "signature": SIGNATURE.trim()} }
        }))
        .unwrap()
    }

    #[test]
    fn checks_keep_their_six_hour_cadence_across_launches_and_clock_changes() {
        assert_eq!(next_check(None, 100), Duration::ZERO);
        assert_eq!(
            next_check(Some(100), 160),
            Duration::from_secs(INTERVAL - 60)
        );
        assert_eq!(next_check(Some(100), INTERVAL + 100), Duration::ZERO);
        assert_eq!(next_check(Some(200), 100), Duration::ZERO);
        let root = tempfile::tempdir().unwrap();
        let state = state(root.path());
        state.lock().status.checked_at = Some(100);
        state.lock().status.available = Some(Available {
            version: "0.2.0".into(),
            notes: "new".into(),
        });
        state.persist();
        let read = read_cache(&state.cache, "0.1.0", Track::Stable, 160).unwrap();
        assert_eq!(read.checked_at, 100);
        assert_eq!(read.available.unwrap().version, "0.2.0");
        assert!(read_cache(&state.cache, "0.2.0", Track::Stable, 160).is_none());
        assert!(read_cache(&state.cache, "0.1.0", Track::Stable, 90).is_none());
        fs::write(&state.cache, b"broken json").unwrap();
        assert!(read_cache(&state.cache, "0.1.0", Track::Stable, 160).is_none());
        fs::write(&state.cache, vec![b' '; MAX_CACHE as usize + 1]).unwrap();
        assert!(read_cache(&state.cache, "0.1.0", Track::Stable, 160).is_none());
    }

    #[test]
    fn only_shipped_installer_and_architecture_pairs_are_installable() {
        use tauri::utils::config::BundleType::*;
        assert_eq!(
            target_for("linux", "x86_64", Some(Deb)).as_deref(),
            Some("linux-x86_64-deb")
        );
        assert_eq!(
            target_for("linux", "x86_64", Some(Rpm)).as_deref(),
            Some("linux-x86_64-rpm")
        );
        assert_eq!(
            target_for("linux", "x86_64", Some(AppImage)).as_deref(),
            Some("linux-x86_64-appimage")
        );
        assert_eq!(
            target_for("macos", "aarch64", Some(App)).as_deref(),
            Some("darwin-aarch64-app")
        );
        assert_eq!(
            target_for("macos", "x86_64", Some(Dmg)).as_deref(),
            Some("darwin-x86_64-app")
        );
        assert_eq!(
            target_for("linux", "aarch64", Some(Deb)).as_deref(),
            Some("linux-aarch64-deb")
        );
        assert_eq!(
            target_for("linux", "aarch64", Some(AppImage)).as_deref(),
            Some("linux-aarch64-appimage")
        );
        assert!(target_for("linux", "riscv64", Some(Deb)).is_none());
        assert_eq!(
            target_for("windows", "x86_64", Some(Nsis)).as_deref(),
            Some("windows-x86_64-nsis")
        );
        assert!(target_for("windows", "aarch64", Some(Nsis)).is_none());
        assert!(target_for("windows", "x86_64", Some(Msi)).is_none());
        assert!(target_for("linux", "x86_64", None).is_none());
    }

    #[test]
    fn every_installable_target_names_the_package_the_release_ships() {
        use tauri::utils::config::BundleType::*;
        for (os, arch, bundle, suffix) in [
            ("linux", "x86_64", Deb, ".deb"),
            ("linux", "x86_64", Rpm, ".rpm"),
            ("linux", "x86_64", AppImage, ".AppImage"),
            ("linux", "aarch64", Deb, ".deb"),
            ("linux", "aarch64", Rpm, ".rpm"),
            ("linux", "aarch64", AppImage, ".AppImage"),
            ("macos", "aarch64", App, ".app.tar.gz"),
            ("macos", "x86_64", Dmg, ".app.tar.gz"),
            ("windows", "x86_64", Nsis, "-setup.exe"),
        ] {
            let target = target_for(os, arch, Some(bundle)).unwrap();
            assert_eq!(package_suffix(&target), Some(suffix), "{target}");
        }
        assert_eq!(package_suffix("windows-x86_64-msi"), None);
        assert_eq!(package_suffix("nonsense"), None);
    }

    /// A Windows desk once required an `.app.tar.gz` and so refused every release.
    #[tokio::test]
    async fn each_installer_accepts_only_its_own_package() {
        let base = "https://github.com/1Broseidon/hotline/releases/download/desktop-v0.2.0";
        for (target, file, accepted) in [
            (
                "windows-x86_64-nsis",
                "hotline_0.2.0_windows_x86_64-setup.exe",
                true,
            ),
            (
                "windows-x86_64-nsis",
                "hotline_0.2.0_macos_aarch64.app.tar.gz",
                false,
            ),
            (
                "darwin-aarch64-app",
                "hotline_0.2.0_macos_aarch64.app.tar.gz",
                true,
            ),
            (
                "darwin-aarch64-app",
                "hotline_0.2.0_windows_x86_64-setup.exe",
                false,
            ),
            ("linux-x86_64-deb", "hotline_0.2.0_linux_x86_64.deb", true),
            (
                "linux-x86_64-deb",
                "hotline_0.2.0_linux_x86_64.AppImage",
                false,
            ),
        ] {
            let root = tempfile::tempdir().unwrap();
            let body = release_at("0.2.0", target, &format!("{base}/{file}"));
            let app = app(&serve(body, 200));
            let state = Updates {
                target: Some(target.into()),
                ..state(root.path())
            };
            let result = check(app.handle(), &state).await;
            assert_eq!(result.is_ok(), accepted, "{target} {file}");
            assert_eq!(state.status().available.is_some(), accepted);
            if let Err(error) = result {
                assert!(error.message.contains("matching update package"));
            }
        }
    }

    #[tokio::test]
    async fn the_native_plugin_discovers_only_newer_stable_releases() {
        for (version, expected) in [
            ("0.2.0", true),
            ("0.1.0", false),
            ("0.0.9", false),
            ("0.3.0-beta.1", false),
        ] {
            let root = tempfile::tempdir().unwrap();
            let app = app(&serve(release(version, "linux-x86_64-deb"), 200));
            let state = state(root.path());
            let found = check(app.handle(), &state).await.unwrap();
            assert_eq!(found.is_some(), expected);
            assert_eq!(state.status().available.is_some(), expected);
            assert_eq!(state.status().phase, Phase::Idle);
            assert!(state.status().checked_at.is_some());
        }
    }

    #[test]
    fn a_track_is_offered_only_its_releases_and_never_an_older_one() {
        for (current, candidate, stable, beta) in [
            // A final release reaches both tracks.
            ("0.48.3", "0.49.0", true, true),
            ("0.49.0-beta.1", "0.49.0", true, true),
            // A beta reaches Beta alone, the first and each one after it.
            ("0.48.3", "0.49.0-beta.1", false, true),
            ("0.49.0-beta.1", "0.49.0-beta.2", false, true),
            ("0.49.0-beta.9", "0.49.0-beta.10", false, true),
            ("0.49.0", "0.50.0-beta.1", false, true),
            // Nothing older or the same is offered, whatever the track.
            ("0.49.0-beta.2", "0.48.4", false, false),
            ("0.49.0-beta.2", "0.49.0-beta.1", false, false),
            ("0.49.0-beta.2", "0.49.0-beta.2", false, false),
            ("0.49.0", "0.49.0-beta.3", false, false),
            ("0.49.0", "0.49.0", false, false),
            // Beta takes `beta.N` and no other pre-release.
            ("0.48.3", "0.49.0-nightly.1", false, false),
            ("0.48.3", "0.49.0-beta", false, false),
            ("0.48.3", "0.49.0-beta.x", false, false),
            ("0.48.3", "0.49.0-rc.1", false, false),
            ("0.48.3", "not a version", false, false),
        ] {
            assert_eq!(
                newer(current, candidate, Track::Stable),
                stable,
                "Stable on {current} offered {candidate}"
            );
            assert_eq!(
                newer(current, candidate, Track::Beta),
                beta,
                "Beta on {current} offered {candidate}"
            );
        }
    }

    #[test]
    fn stable_reads_the_latest_final_release_and_beta_one_fixed_address() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(
            config["plugins"]["updater"]["endpoints"],
            json!(["https://github.com/1Broseidon/hotline/releases/latest/download/latest.json"])
        );
        let root = tempfile::tempdir().unwrap();
        let state = Updates::new(root.path(), "0.1.0".into(), false);
        assert_eq!(state.status().track, Track::Stable);
        assert!(state.manifest(Track::Stable).unwrap().is_none());
        assert_eq!(
            state.manifest(Track::Beta).unwrap().unwrap().as_str(),
            "https://raw.githubusercontent.com/1Broseidon/hotline/update-tracks/beta.json"
        );
    }

    #[tokio::test]
    async fn each_track_checks_its_own_manifest() {
        for (track, offered) in [(Track::Stable, "0.2.0"), (Track::Beta, "0.3.0-beta.1")] {
            let root = tempfile::tempdir().unwrap();
            let app = app(&serve(release("0.2.0", "linux-x86_64-deb"), 200));
            let state = Updates {
                beta_manifest: serve(release("0.3.0-beta.1", "linux-x86_64-deb"), 200),
                ..state(root.path())
            };
            state.set_track(app.handle(), track).unwrap();
            let found = check(app.handle(), &state).await.unwrap().unwrap();
            assert_eq!(found.version, offered, "{track:?}");
            assert_eq!(state.status().available.unwrap().version, offered);
        }
    }

    #[tokio::test]
    async fn a_beta_is_offered_its_final_and_later_betas() {
        for (manifest, offered) in [
            ("0.2.0-beta.2", true),
            ("0.2.0", true),
            ("0.3.0-beta.1", true),
            ("0.2.0-beta.1", false),
            ("0.1.9", false),
        ] {
            let root = tempfile::tempdir().unwrap();
            let app = app_at("http://127.0.0.1:1/stable-is-not-read", "0.2.0-beta.1");
            let state = Updates {
                beta_manifest: serve(release(manifest, "linux-x86_64-deb"), 200),
                ..state_at(root.path(), "0.2.0-beta.1")
            };
            state.set_track(app.handle(), Track::Beta).unwrap();
            let found = check(app.handle(), &state).await.unwrap();
            assert_eq!(found.is_some(), offered, "{manifest}");
            assert_eq!(state.status().available.is_some(), offered, "{manifest}");
        }
    }

    #[tokio::test]
    async fn a_desk_back_on_stable_keeps_its_beta_until_a_final_release_passes_it() {
        for (manifest, offered) in [
            ("0.1.9", false),
            ("0.2.0-beta.2", false),
            ("0.3.0-beta.1", false),
            ("0.2.0", true),
        ] {
            let root = tempfile::tempdir().unwrap();
            let app = app_at(
                &serve(release(manifest, "linux-x86_64-deb"), 200),
                "0.2.0-beta.1",
            );
            let state = Updates {
                beta_manifest: "http://127.0.0.1:1/beta-is-not-read".into(),
                ..state_at(root.path(), "0.2.0-beta.1")
            };
            state.set_track(app.handle(), Track::Beta).unwrap();
            state.set_track(app.handle(), Track::Stable).unwrap();
            let found = check(app.handle(), &state).await.unwrap();
            assert_eq!(found.is_some(), offered, "{manifest}");
            assert_eq!(state.status().current, "0.2.0-beta.1");
        }
    }

    #[test]
    fn the_track_is_stored_with_the_desk_and_anything_unreadable_is_stable() {
        let root = tempfile::tempdir().unwrap();
        let app = app("http://127.0.0.1:1/not-contacted");
        let state = state(root.path());
        assert_eq!(state.status().track, Track::Stable);
        assert!(!state.track_file.exists());
        state.set_track(app.handle(), Track::Beta).unwrap();
        assert_eq!(
            fs::read_to_string(&state.track_file).unwrap(),
            r#"{"track":"beta"}"#
        );
        // The next launch of this desk is on Beta; a desk elsewhere is not.
        let again = Updates::new(root.path(), "0.1.0".into(), false);
        assert_eq!(again.status().track, Track::Beta);
        let elsewhere = tempfile::tempdir().unwrap();
        let other = Updates::new(elsewhere.path(), "0.1.0".into(), false);
        assert_eq!(other.status().track, Track::Stable);
        for unreadable in [
            r#"{"track":"nightly"}"#.to_string(),
            r#"{"track":"https://example.com/latest.json"}"#.to_string(),
            "beta".to_string(),
            String::new(),
            format!(
                r#"{{"track":"beta"{}}}"#,
                " ".repeat(MAX_TRACK_FILE as usize)
            ),
        ] {
            fs::write(&state.track_file, &unreadable).unwrap();
            let read = Updates::new(root.path(), "0.1.0".into(), false);
            assert_eq!(read.status().track, Track::Stable, "{unreadable:.40}");
        }
    }

    #[test]
    fn changing_track_forgets_the_other_tracks_offer() {
        let root = tempfile::tempdir().unwrap();
        let app = app("http://127.0.0.1:1/not-contacted");
        let state = state(root.path());
        state.set_track(app.handle(), Track::Beta).unwrap();
        state.lock().status.checked_at = Some(100);
        state.lock().status.available = Some(Available {
            version: "0.2.0-beta.1".into(),
            notes: "a beta".into(),
        });
        state.persist();
        assert!(read_cache(&state.cache, "0.1.0", Track::Beta, 160).is_some());
        // The same choice again keeps what is known.
        state.set_track(app.handle(), Track::Beta).unwrap();
        assert!(state.status().available.is_some());
        state.set_track(app.handle(), Track::Stable).unwrap();
        assert_eq!(state.status().available, None);
        assert_eq!(state.status().checked_at, None);
        // The offer kept on disk was Beta's, so a launch on Stable shows none.
        assert!(read_cache(&state.cache, "0.1.0", Track::Stable, 160).is_none());
        let relaunched = Updates::new(root.path(), "0.1.0".into(), false);
        assert_eq!(relaunched.status().track, Track::Stable);
    }

    #[tokio::test]
    async fn offline_rate_limited_and_missing_package_checks_keep_the_previous_offer() {
        for (body, code) in [
            (b"rate limited".to_vec(), 429),
            (b"unavailable".to_vec(), 503),
            (release("0.2.0", "linux-x86_64-appimage"), 200),
        ] {
            let root = tempfile::tempdir().unwrap();
            let app = app(&serve(body, code));
            let state = state(root.path());
            let previous = Available {
                version: "0.2.0".into(),
                notes: "previous".into(),
            };
            state.lock().status.available = Some(previous.clone());
            assert!(check(app.handle(), &state).await.is_err());
            assert_eq!(state.status().available, Some(previous));
            // The label is ours; what the network said is only the detail.
            let error = state.status().error.unwrap();
            assert_eq!(error.message, CHECK_FAILED);
            assert!(error.detail.is_some());
            assert_eq!(state.status().phase, Phase::Idle);
        }
    }

    #[tokio::test]
    async fn downloads_are_verified_by_tauri_and_never_installed_by_tests() {
        for (payload, valid) in [
            (PAYLOAD.to_vec(), true),
            (b"tampered package".to_vec(), false),
        ] {
            let root = tempfile::tempdir().unwrap();
            let app = app(&serve(release("0.2.0", "linux-x86_64-deb"), 200));
            let state = state(root.path());
            let mut update = check(app.handle(), &state).await.unwrap().unwrap();
            // Only this test replaces the production HTTPS URL with a loopback fixture.
            update.download_url = serve(payload, 200).parse().unwrap();
            let mut received = 0;
            let result = update.download(|chunk, _| received += chunk, || {}).await;
            assert_eq!(result.is_ok(), valid);
            assert!(received > 0);
            if valid {
                assert_eq!(result.unwrap(), PAYLOAD);
            }
        }
    }

    #[tokio::test]
    async fn a_download_can_be_cancelled_before_any_installer_runs() {
        let root = tempfile::tempdir().unwrap();
        let app = app(&serve(release("0.2.0", "linux-x86_64-deb"), 200));
        let state = state(root.path());
        let mut update = check(app.handle(), &state).await.unwrap().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        update.download_url = format!("http://{}/package", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let (started, received) = tokio::sync::oneshot::channel();
        let (finish, wait) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 8192];
            assert!(socket.read(&mut request).unwrap() > 0);
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: 10000\r\n\r\npartial"
            )
            .unwrap();
            started.send(()).unwrap();
            let _ = wait.recv_timeout(Duration::from_secs(5));
        });
        let cancel = CancellationToken::new();
        let signal = cancel.clone();
        let download =
            tokio::spawn(async move { download_verified(&update, &signal, |_, _| {}).await });
        tokio::time::timeout(Duration::from_secs(5), received)
            .await
            .unwrap()
            .unwrap();
        cancel.cancel();
        let result = tokio::time::timeout(Duration::from_secs(5), download)
            .await
            .unwrap()
            .unwrap();
        assert!(result.unwrap_err().message.contains("cancelled"));
        finish.send(()).unwrap();
        server.join().unwrap();
    }

    #[tokio::test]
    async fn development_builds_refuse_checks_before_opening_a_connection() {
        let root = tempfile::tempdir().unwrap();
        let app = app("http://127.0.0.1:1/not-contacted");
        let state = Updates::new(root.path(), "0.1.0".into(), true);
        assert!(
            check(app.handle(), &state)
                .await
                .err()
                .unwrap()
                .message
                .contains("development")
        );
        assert!(!state.cache.exists());
    }
}
