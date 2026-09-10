//! Signed updates use the same Tauri plugin and GitHub manifest as Prism.
//! The desktop owns installation; the room only provides an idle restart lease.

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
use toad_core::desk::Desk;
use tokio_util::sync::CancellationToken;

const EVENT: &str = "toad://update";
const INTERVAL: u64 = 6 * 60 * 60;
const FIRST_CHECK: u64 = 20;
const MAX_NOTES: usize = 16_000;
const MAX_CACHE: u64 = 32_000;

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
    available: Option<Available>,
    checked_at: Option<u64>,
    phase: Phase,
    downloaded: u64,
    total: Option<u64>,
    error: Option<String>,
    disabled_reason: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cache {
    current: String,
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
    target: Option<String>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn newer(current: &str, candidate: &str) -> bool {
    match (
        semver::Version::parse(current),
        semver::Version::parse(candidate),
    ) {
        (Ok(current), Ok(candidate)) => candidate.pre.is_empty() && candidate > current,
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
        ("linux" | "windows", "x86_64") | ("macos", "x86_64" | "aarch64")
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

fn read_cache(path: &Path, current: &str, at: u64) -> Option<Cache> {
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
    if cache.current != current || cache.checked_at > at {
        return None;
    }
    if let Some(info) = &cache.available
        && (!newer(current, &info.version) || info.notes.len() > MAX_NOTES)
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
        let saved = if disabled_reason.is_none() {
            read_cache(&cache, &current, now())
        } else {
            None
        };
        Self {
            inner: Mutex::new(Inner {
                status: Status {
                    current,
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
            target,
        }
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
            checked_at,
            available: status.available,
        };
        // A failed cache write must not turn a successful check into a failed update.
        // It only means the next launch will check again.
        let result = (|| -> std::io::Result<()> {
            let mut temp =
                tempfile::NamedTempFile::new_in(self.cache.parent().expect("cache has a parent"))?;
            temp.write_all(&serde_json::to_vec(&cache)?)?;
            temp.as_file().sync_all()?;
            temp.persist(&self.cache).map_err(|error| error.error)?;
            Ok(())
        })();
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
async fn check<R: Runtime>(app: &AppHandle<R>, state: &Updates) -> Result<Option<Update>, String> {
    state.enabled()?;
    state.change(app, |status| {
        status.phase = Phase::Checking;
        status.error = None;
    });
    let result: Result<Option<Update>, String> = async {
        let updater = app
            .updater_builder()
            .target(state.target.as_ref().ok_or("Unsupported installation.")?)
            .timeout(Duration::from_secs(30))
            .version_comparator(|current, release| {
                newer(&current.to_string(), &release.version.to_string())
            })
            .build()
            .map_err(|error| error.to_string())?;
        let mut found = updater.check().await.map_err(|error| {
            format!("Could not check for updates. Check your connection and try again. {error}")
        })?;
        if let Some(update) = &mut found {
            validate_update(update, state.target.as_deref().unwrap())?;
            update.timeout = Some(Duration::from_secs(15 * 60));
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
                })
            }
            Err(error) => status.error = Some(error.clone()),
        }
    });
    state.persist();
    result
}

fn validate_update(update: &Update, target: &str) -> Result<(), String> {
    let extension = if target.ends_with("-deb") {
        ".deb"
    } else if target.ends_with("-rpm") {
        ".rpm"
    } else if target.ends_with("-appimage") {
        ".AppImage"
    } else {
        ".app.tar.gz"
    };
    if update.download_url.scheme() != "https"
        || !update.download_url.path().ends_with(extension)
        || update
            .raw_json
            .get("platforms")
            .and_then(|p| p.get(target))
            .is_none()
    {
        return Err("This release does not include a matching update package. Open the release page instead.".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn check_update(window: tauri::WebviewWindow, app: AppHandle) -> Result<Status, String> {
    main_window(&window)?;
    let state = app.state::<Updates>();
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "An update operation is already running.")?;
    check(&app, &state).await?;
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
) -> Result<(), String> {
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

async fn install(app: &AppHandle, state: &Updates, version: &str) -> Result<(), String> {
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
            return Err("Download cancelled. Toad has not been changed.".into());
        }
        inner.status.phase = Phase::Installing;
    }
    state.change(app, |_| {});
    app.save_window_state(super::window_state_flags())
        .map_err(|error| error.to_string())?;
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // This lease lives with the installer, even if its caller disappears.
        let _held = held;
        update.install(bytes).map_err(|error| {
            format!("Installation failed. Toad is still running; try again. {error}")
        })?;
        app.state::<Updates>()
            .change(&app, |status| status.phase = Phase::Restarting);
        app.restart();
    })
    .await
    .map_err(|error| error.to_string())?
}

async fn download_verified(
    update: &Update,
    cancel: &CancellationToken,
    progress: impl FnMut(usize, Option<u64>),
) -> Result<Vec<u8>, String> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err("Download cancelled. Toad has not been changed.".into()),
        result = update.download(progress, || {}) => result.map_err(|error| format!("The update could not be downloaded or verified. Try again. {error}")),
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
        let mut context = mock_context(noop_assets());
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
        let state = Updates::new(root, "0.1.0".into(), false);
        // Mock runtime has no installed package. Only fixture checks/downloads are enabled.
        state.lock().status.disabled_reason = None;
        Updates {
            target: Some("linux-x86_64-deb".into()),
            ..state
        }
    }

    fn release(version: &str, target: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "version": version, "notes": "A fixture release.",
            "platforms": { target: {"url":"https://github.com/1Broseidon/toad/releases/download/desktop-v0.2.0/toad.deb", "signature": SIGNATURE.trim()} }
        })).unwrap()
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
        let read = read_cache(&state.cache, "0.1.0", 160).unwrap();
        assert_eq!(read.checked_at, 100);
        assert_eq!(read.available.unwrap().version, "0.2.0");
        assert!(read_cache(&state.cache, "0.2.0", 160).is_none());
        assert!(read_cache(&state.cache, "0.1.0", 90).is_none());
        fs::write(&state.cache, b"broken json").unwrap();
        assert!(read_cache(&state.cache, "0.1.0", 160).is_none());
        fs::write(&state.cache, vec![b' '; MAX_CACHE as usize + 1]).unwrap();
        assert!(read_cache(&state.cache, "0.1.0", 160).is_none());
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
        assert!(target_for("linux", "aarch64", Some(Deb)).is_none());
        assert_eq!(
            target_for("windows", "x86_64", Some(Nsis)).as_deref(),
            Some("windows-x86_64-nsis")
        );
        assert!(target_for("windows", "aarch64", Some(Nsis)).is_none());
        assert!(target_for("windows", "x86_64", Some(Msi)).is_none());
        assert!(target_for("linux", "x86_64", None).is_none());
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
            assert!(state.status().error.is_some());
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
        assert!(result.unwrap_err().contains("cancelled"));
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
                .contains("development")
        );
        assert!(!state.cache.exists());
    }
}
