//! One Linux desktop per teammate, as a container Hotline starts and stops.
//!
//! The image is a contract, not a binary: anything that serves streamable
//! HTTP MCP at `http://127.0.0.1:<port>/mcp` with bearer `HOTLINE_COMPUTER_TOKEN`,
//! `/health` open, and the viewer page at `/` on the same port is a valid
//! computer. This module is the Hotline-side lifecycle — wake, idle stop,
//! hibernate — and the grant a session is handed. The agent and the image
//! are Hotline Computer, a repository of their own.

pub mod runtime;

use crate::contract::{
    ComputerReleases, ComputerState, ComputerStatus, Persona, RuntimeReport, RuntimeState,
};
use crate::mcp::{HttpAuth, McpServer, McpTransport};
use runtime::{BinSearch, Runtime};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tokio::process::Command;

/// The Hotline Computer release this desktop is built against: the floor. A
/// new computer is created on the newest published release at or above it
/// on the same major (see [`releases`]), and on this one when nothing newer
/// is known. Bumped here deliberately — never derived from the desktop
/// version, and never `latest`.
pub const COMPUTER_VERSION: &str = "0.6.0";

/// The MCP server id a session is granted, and the origin the ledger names.
pub const SERVER_ID: &str = "computer";

/// Idle minutes after a session stops → `stop`. Frees CPU and RAM; the rw
/// layer survives.
pub const COMPUTER_IDLE_STOP_MS: i64 = 30 * 60 * 1000;
/// Idle days → `rm`. Frees disk. Wake is a fresh container; the token was
/// process state and is gone with it anyway.
pub const COMPUTER_HIBERNATE_MS: i64 = 7 * 24 * 60 * 60 * 1000;

const MCP_PORT: u16 = 8787;
const WORKSPACE_MOUNT: &str = "/home/agent/workspace";
/// A named volume per teammate for the home itself, so what the teammate
/// prepared there outlives the container the hibernate cycle removes: the
/// environments it built for a workspace, its jobs and their output, its
/// shell history and its browser profile. The image keeps nothing of its
/// own in the home, so an empty volume is what a fresh container would
/// have had anyway. An extra mount may sit inside it; only one that would
/// cover it is refused, and that one already covers the workspace.
const HOME_MOUNT: &str = "/home/agent";
/// A named volume per teammate, so a checkout or a build it starts outlives
/// the container the hibernate cycle removes. The workspace is the person's
/// folder; this one is the teammate's.
const SCRATCH_MOUNT: &str = "/home/agent/src";
/// One Nix store for every teammate. Store paths are content-addressed and
/// immutable, so sharing costs nothing and a toolchain one teammate pulled
/// is a cache hit for the next. The image seeds an empty volume on first
/// use. Only `nix-collect-garbage` is a hazard across containers, since a
/// path in use by a process one container cannot see looks unused.
const NIX_MOUNT: &str = "/nix";
/// The glibc image seeds a writable store; old Alpine volumes remain available.
const NIX_VOLUME: &str = "hotline-nix-glibc";
const DEFAULT_MEMORY: &str = "4g";
const DEFAULT_PIDS: u32 = 1024;
const PULL_NOTICE: &str = "Pulling the computer image …";

const HEALTH_TIMEOUT: Duration = Duration::from_secs(30);
const HEALTH_PROBE: Duration = Duration::from_secs(2);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const PULL_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const STOP_TIMEOUT: Duration = Duration::from_secs(60);

/// What the grant and the in-process client need once the machine is up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ready {
    pub url: String,
    pub token: String,
}

pub mod cookies;
pub mod guide;
pub mod login;
pub mod releases;
pub mod secrets;

/// The image the desk's own release line is published as, at `release`.
pub fn image_at(release: &str) -> String {
    format!("{COMPUTER_REPOSITORY}:{release}")
}

/// The floor's image: what a computer is created on when nothing newer is
/// known.
pub fn default_image() -> String {
    image_at(COMPUTER_VERSION)
}

pub fn container_name(persona_id: &str) -> String {
    format!("hotline-computer-{persona_id}")
}

/// The MCP server a session is handed for this computer. Not part of
/// `mcpPolicy`: the computer is a per-teammate capability Hotline manages.
pub fn mcp_server(ready: &Ready) -> McpServer {
    McpServer {
        id: SERVER_ID.into(),
        name: "Computer".into(),
        transport: McpTransport::Http {
            url: ready.url.clone(),
            auth: HttpAuth::Bearer {
                token: ready.token.clone(),
            },
        },
        refuse: None,
    }
}

/// Hotline names the computer's tools `{slug}__{remote}` from the server's
/// human name "Computer", so the prefix an agent sees is `computer__`.
/// An ACP child may put that name in the call's title instead of its kind.
pub fn is_computer_tool(kind: &str, title: &str) -> bool {
    kind.contains("computer__") || title.contains("computer__")
}

pub fn preferred_runtime(settings: &serde_json::Map<String, Value>) -> Option<Runtime> {
    settings
        .get("computerRuntime")
        .and_then(Value::as_str)
        .and_then(Runtime::from_setting)
}

/// The room's default image, `computerImage`; blank means the pin.
pub fn preferred_image(settings: &serde_json::Map<String, Value>) -> Option<String> {
    settings
        .get("computerImage")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|image| !image.is_empty())
        .map(str::to_string)
}

/// Every teammate computer this process has woken, keyed by persona id.
///
/// The token is generated once per container and kept here. A container that
/// already exists from a previous run of Hotline has a token we no longer know
/// — it was process state, never a setting — so it is removed and recreated
/// rather than started with a bearer nobody can present.
#[derive(Clone)]
pub struct Computer {
    inner: Arc<Mutex<Inner>>,
    bins: BinSearch,
    /// Where published releases are listed; a test points this at its own.
    releases_url: String,
}

struct Inner {
    containers: HashMap<String, Live>,
    /// The newest release the desk has heard of, and when to ask again.
    known: releases::Known,
}

#[derive(Clone)]
struct Live {
    runtime: Runtime,
    cmd: PathBuf,
    token: String,
    mcp_port: u16,
    last_activity_ms: i64,
    /// The release the computer reported with its guide, once it has.
    release: Option<String>,
}

impl Computer {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                containers: HashMap::new(),
                known: releases::Known::default(),
            })),
            bins: BinSearch::from_env(),
            releases_url: releases::RELEASES_URL.to_string(),
        }
    }

    /// A runtime found only at `path`, and a releases endpoint nobody
    /// answers on, so a test that does not care about releases gets the
    /// floor without waiting on the network.
    #[cfg(test)]
    pub fn with_path(path: impl Into<std::ffi::OsString>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                containers: HashMap::new(),
                known: releases::Known::default(),
            })),
            bins: BinSearch::only(path),
            releases_url: "http://127.0.0.1:1/releases".to_string(),
        }
    }

    /// The same desk, asking `url` for releases.
    #[cfg(test)]
    pub fn with_releases(mut self, url: &str) -> Self {
        self.releases_url = url.to_string();
        self
    }

    /// Looks up the newest release when it is due — once at desk start,
    /// every [`releases::CHECK_EVERY_MS`] after, sooner after a failure —
    /// and answers what is known.
    pub async fn refresh_releases(&self, now_ms: i64) -> Option<String> {
        if !self.lock().known.due(now_ms) {
            return self.lock().known.newest.clone();
        }
        let answer = releases::lookup(&self.releases_url, COMPUTER_VERSION).await;
        let mut inner = self.lock();
        inner.known.record(answer, now_ms);
        inner.known.newest.clone()
    }

    /// Asks the endpoint now, whatever the clock says — the Settings
    /// button — and answers what is known after.
    pub async fn check_releases(&self, now_ms: i64) -> ComputerReleases {
        let answer = releases::lookup(&self.releases_url, COMPUTER_VERSION).await;
        let mut inner = self.lock();
        inner.known.record(answer, now_ms);
        known_releases(&inner.known)
    }

    /// The newest release the desk knows of, without asking.
    pub fn newest_known(&self) -> Option<String> {
        self.lock().known.newest.clone()
    }

    /// What the desk knows about releases, without asking.
    pub fn releases_known(&self) -> ComputerReleases {
        known_releases(&self.lock().known)
    }

    /// The image a computer for `persona` is created on now: the teammate's
    /// pin, else the room's, else the newest release known, else the floor.
    pub fn image_for(&self, persona: &Persona, room_image: Option<&str>) -> String {
        image_of(persona, room_image, self.newest_known().as_deref())
    }

    /// Removes an image the desk pulled, once no container is on it. A
    /// runtime that still has a container on the image refuses, and that
    /// refusal is the right answer.
    pub async fn forget_image(&self, image: &str, prefer: Option<Runtime>) {
        if let Ok((_, cmd)) = pick_runtime(prefer, &self.bins).await {
            let _ = run(&cmd, &["image", "rm", image], COMMAND_TIMEOUT).await;
        }
    }

    pub async fn runtimes(&self) -> Vec<RuntimeReport> {
        runtime::detect_with(&self.bins).await
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub async fn ensure_running(
        &self,
        persona: &Persona,
        workspace_cwd: &str,
        prefer: Option<Runtime>,
        room_image: Option<&str>,
        mut notice: impl FnMut(&str),
    ) -> Result<Ready, String> {
        let (runtime, cmd) = pick_runtime(prefer, &self.bins).await?;
        let name = container_name(&persona.id);
        let cwd = abs_cwd(workspace_cwd);
        let known_token = self
            .lock()
            .containers
            .get(&persona.id)
            .map(|live| live.token.clone());

        let mut inspection = inspect(&cmd, runtime, &name).await?;
        let known_token = inspection.token.clone().or(known_token);
        if inspection.exists && known_token.is_none() {
            // Older or foreign containers may not expose the expected bearer.
            // A desktop with no recoverable token cannot be authenticated.
            run(&cmd, &["rm", "-f", &name], COMMAND_TIMEOUT).await?;
            inspection.exists = false;
            inspection.running = false;
        }

        let token = known_token.clone().unwrap_or_else(new_token);
        if !inspection.exists {
            // A computer made now is made on the newest release, so a desk
            // that has not asked yet asks first. A pinned image never asks.
            if pinned_image(persona, room_image).is_none() {
                self.refresh_releases(now_ms()).await;
            }
            let image = self.image_for(persona, room_image);
            if !image_present(&cmd, runtime, &image).await {
                notice(PULL_NOTICE);
                pull(&cmd, runtime, &image).await?;
            }
            let args = create_args(runtime, &name, &image, &token, &cwd, persona)?;
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            run(&cmd, &arg_refs, COMMAND_TIMEOUT).await?;
            run(&cmd, &["start", &name], COMMAND_TIMEOUT).await?;
        } else if !inspection.running {
            run(&cmd, &["start", &name], COMMAND_TIMEOUT).await?;
        }

        let inspection = inspect(&cmd, runtime, &name).await?;
        let mcp_port = inspection.mcp_port.ok_or_else(|| {
            format!("Could not read the computer's published port for {MCP_PORT}/tcp")
        })?;
        self.lock().containers.insert(
            persona.id.clone(),
            Live {
                runtime,
                cmd: cmd.clone(),
                token: token.clone(),
                mcp_port,
                last_activity_ms: now_ms(),
                release: None,
            },
        );
        wait_healthy(mcp_port).await?;
        Ok(Ready {
            url: mcp_url(mcp_port),
            token,
        })
    }

    /// The endpoint of a teammate's computer while it is running and this
    /// process holds its token; `None` for one that is stopped, absent, or
    /// was made by a process that is gone. A stopped computer is handed
    /// nothing now and everything at its next start.
    pub async fn running(&self, persona_id: &str) -> Option<Ready> {
        let live = self.lock().containers.get(persona_id).cloned()?;
        let inspection = inspect(&live.cmd, live.runtime, &container_name(persona_id))
            .await
            .ok()?;
        inspection.running.then(|| Ready {
            url: mcp_url(live.mcp_port),
            token: live.token,
        })
    }

    /// Remembers the release a computer said it was, for the status the pane
    /// shows against the release it would be created on now.
    pub fn learned_release(&self, persona_id: &str, release: &str) {
        if let Some(live) = self.lock().containers.get_mut(persona_id) {
            live.release = Some(release.to_string());
        }
    }

    pub fn mark_idle(&self, persona_id: &str, at_ms: i64) {
        if let Some(live) = self.lock().containers.get_mut(persona_id) {
            live.last_activity_ms = at_ms;
        }
    }

    pub async fn stop(&self, persona_id: &str, prefer: Option<Runtime>) -> Result<(), String> {
        let name = container_name(persona_id);
        let known = self.lock().containers.get(persona_id).cloned();
        if let Some(live) = known {
            let inspection = inspect(&live.cmd, live.runtime, &name).await.ok();
            if inspection.is_some_and(|seen| seen.running) {
                run(&live.cmd, &["stop", &name], STOP_TIMEOUT).await?;
            }
            return Ok(());
        }
        let Ok((runtime, cmd)) = pick_runtime(prefer, &self.bins).await else {
            return Ok(());
        };
        let inspection = inspect(&cmd, runtime, &name).await?;
        if inspection.running {
            run(&cmd, &["stop", &name], STOP_TIMEOUT).await?;
        }
        Ok(())
    }

    pub async fn remove(&self, persona_id: &str, prefer: Option<Runtime>) -> Result<(), String> {
        let name = container_name(persona_id);
        let cmd = self
            .lock()
            .containers
            .remove(persona_id)
            .map(|live| live.cmd);
        if let Some(cmd) = cmd {
            let _ = run(&cmd, &["rm", "-f", &name], COMMAND_TIMEOUT).await;
            return Ok(());
        }
        if let Ok((_, cmd)) = pick_runtime(prefer, &self.bins).await {
            let _ = run(&cmd, &["rm", "-f", &name], COMMAND_TIMEOUT).await;
        }
        Ok(())
    }

    pub async fn status(
        &self,
        persona_id: &str,
        prefer: Option<Runtime>,
    ) -> Result<ComputerStatus, String> {
        let name = container_name(persona_id);
        let known = self.lock().containers.get(persona_id).cloned();
        if let Some(live) = known {
            let inspection = match inspect(&live.cmd, live.runtime, &name).await {
                Ok(seen) => seen,
                Err(_) => {
                    return Ok(ComputerStatus {
                        state: ComputerState::Absent,
                        url: None,
                        viewer: None,
                        release: None,
                        available: None,
                    });
                }
            };
            let mut status = status_of(inspection, Some((live.mcp_port, &live.token)));
            status.release = live.release.clone();
            return Ok(status);
        }
        let Ok((runtime, cmd)) = pick_runtime(prefer, &self.bins).await else {
            return Ok(ComputerStatus {
                state: ComputerState::Absent,
                url: None,
                viewer: None,
                release: None,
                available: None,
            });
        };
        let inspection = match inspect(&cmd, runtime, &name).await {
            Ok(seen) => seen,
            Err(_) => {
                return Ok(ComputerStatus {
                    state: ComputerState::Absent,
                    url: None,
                    viewer: None,
                    release: None,
                    available: None,
                });
            }
        };
        Ok(status_of(inspection, None))
    }

    /// Stop containers idle longer than [`COMPUTER_IDLE_STOP_MS`]; remove
    /// ones idle longer than [`COMPUTER_HIBERNATE_MS`]. Live sessions are
    /// skipped: a teammate still talking is not idle.
    pub async fn sweep(&self, now_ms: i64, live: &HashSet<String>) {
        let ids: Vec<(String, i64, PathBuf, Runtime)> = self
            .lock()
            .containers
            .iter()
            .filter(|(id, _)| !live.contains(id.as_str()))
            .map(|(id, live)| {
                (
                    id.clone(),
                    live.last_activity_ms,
                    live.cmd.clone(),
                    live.runtime,
                )
            })
            .collect();
        for (id, last, cmd, runtime) in ids {
            let idle = now_ms.saturating_sub(last);
            let name = container_name(&id);
            if idle >= COMPUTER_HIBERNATE_MS {
                let _ = run(&cmd, &["rm", "-f", &name], COMMAND_TIMEOUT).await;
                self.lock().containers.remove(&id);
            } else if idle >= COMPUTER_IDLE_STOP_MS
                && let Ok(seen) = inspect(&cmd, runtime, &name).await
                && seen.running
            {
                let _ = run(&cmd, &["stop", &name], STOP_TIMEOUT).await;
            }
        }
    }
}

impl Default for Computer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
struct Inspection {
    exists: bool,
    running: bool,
    mcp_port: Option<u16>,
    token: Option<String>,
}

/// The private runtime metadata preserves the bearer across app restarts.
/// It is returned only in the authenticated viewer URL, never in diagnostics.
fn status_of(inspection: Inspection, known: Option<(u16, &str)>) -> ComputerStatus {
    if !inspection.exists {
        return ComputerStatus {
            state: ComputerState::Absent,
            url: None,
            viewer: None,
            release: None,
            available: None,
        };
    }
    if !inspection.running {
        return ComputerStatus {
            state: ComputerState::Stopped,
            url: None,
            viewer: None,
            release: None,
            available: None,
        };
    }
    let mcp = inspection.mcp_port.or(known.map(|known| known.0));
    ComputerStatus {
        state: ComputerState::Running,
        release: None,
        available: None,
        url: mcp.map(mcp_url),
        viewer: match (
            mcp,
            inspection
                .token
                .as_deref()
                .or(known.map(|(_, token)| token)),
        ) {
            (Some(port), Some(token)) => Some(viewer_url(port, token)),
            _ => None,
        },
    }
}

fn mcp_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/mcp")
}

/// The bearer rides in the fragment: the page reads it, the server never
/// sees it in a request line or a log.
fn viewer_url(port: u16, token: &str) -> String {
    format!("http://127.0.0.1:{port}/#{token}")
}

/// The teammate's own image, else the room's, else the pin.
/// The image a person pinned for this teammate's computer: the teammate's
/// own, else the room's. Nothing when the desk chooses the release.
pub fn pinned_image(persona: &Persona, room_image: Option<&str>) -> Option<String> {
    persona
        .computer
        .as_ref()
        .and_then(|computer| computer.image.as_deref())
        .map(str::trim)
        .filter(|image| !image.is_empty())
        .or(room_image)
        .map(str::to_string)
}

/// The repository every release's image is under; a tag on it is a pin.
pub const COMPUTER_REPOSITORY: &str = "ghcr.io/1broseidon/hotline-computer";

/// The wire's view of what is known about releases.
fn known_releases(known: &releases::Known) -> ComputerReleases {
    ComputerReleases {
        floor: COMPUTER_VERSION.to_string(),
        repository: COMPUTER_REPOSITORY.to_string(),
        newest: known.newest.clone(),
        releases: known.releases.clone(),
        checked_at: known.checked_ms,
        error: known.error.clone(),
    }
}

/// The image a computer for this teammate is created on now: the pin when
/// there is one, else the newest release known, else the floor.
pub fn image_of(persona: &Persona, room_image: Option<&str>, newest: Option<&str>) -> String {
    pinned_image(persona, room_image)
        .unwrap_or_else(|| image_at(newest.unwrap_or(COMPUTER_VERSION)))
}

/// The release an image reference names: its tag, or `latest` when it has
/// none. The registry's own port is not a tag.
pub fn release_of(image: &str) -> String {
    let name = image.rsplit('/').next().unwrap_or(image);
    match name.split_once(':') {
        Some((_, tag)) if !tag.is_empty() => tag.to_string(),
        _ => "latest".to_string(),
    }
}

fn abs_cwd(cwd: &str) -> String {
    std::fs::canonicalize(cwd)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| cwd.to_string())
}

fn new_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

async fn pick_runtime(
    prefer: Option<Runtime>,
    bins: &BinSearch,
) -> Result<(Runtime, PathBuf), String> {
    let reports = runtime::detect_with(bins).await;
    if let Some(want) = prefer {
        let report = reports
            .iter()
            .find(|report| report.runtime == want.wire())
            .ok_or_else(|| format!("{} is not a runtime Hotline looks for.", want.name()))?;
        if report.state.ready() {
            let cmd = bins
                .resolve(want.command())
                .ok_or_else(|| format!("{} not found on PATH", want.command()))?;
            return Ok((want, cmd));
        }
        // The sentence is enough here: the runtime's own words are on the
        // Computer settings page, where the person chose this runtime.
        return Err(match report.state {
            RuntimeState::NotInstalled => format!("{} is not installed.", want.name()),
            RuntimeState::NotRunning => format!("{} is not running.", want.name()),
            RuntimeState::NotResponding => format!("{} is not responding.", want.name()),
            RuntimeState::Unsupported => format!("{} is macOS only.", want.name()),
            RuntimeState::Failed | RuntimeState::Ready => {
                format!("{} failed its check.", want.name())
            }
        });
    }
    let best = reports
        .iter()
        .find(|report| report.state.ready())
        .ok_or_else(|| "No container runtime was found; install Docker or Podman.".to_string())?;
    let runtime = Runtime::from_wire(best.runtime);
    let cmd = bins
        .resolve(runtime.command())
        .ok_or_else(|| format!("{} not found on PATH", runtime.command()))?;
    Ok((runtime, cmd))
}

fn create_args(
    runtime: Runtime,
    name: &str,
    image: &str,
    token: &str,
    cwd: &str,
    persona: &Persona,
) -> Result<Vec<String>, String> {
    let mut args = vec![
        "create".to_string(),
        "--name".to_string(),
        name.to_string(),
        // Nothing in the image runs as root, so nothing is added back.
        "--cap-drop=ALL".to_string(),
    ];
    // Apple container does not implement these Docker/Podman hardening flags.
    if runtime != Runtime::AppleContainer {
        args.extend([
            "--security-opt".into(),
            "no-new-privileges".into(),
            "--pids-limit".into(),
            pids_of(persona),
        ]);
    }
    args.extend([
        "--memory".into(),
        memory_of(persona)?,
        "--shm-size".into(),
        "1g".into(),
    ]);
    // Named volumes are Docker's and Podman's; Apple container gets the
    // bind mounts only, and its rw layer is what it has.
    if runtime != Runtime::AppleContainer {
        args.extend([
            "-v".into(),
            format!("{}:{HOME_MOUNT}", home_volume(&persona.id)),
            "-v".into(),
            format!("{NIX_VOLUME}:{NIX_MOUNT}"),
            "-v".into(),
            format!("{}:{SCRATCH_MOUNT}", scratch_volume(&persona.id)),
        ]);
    }
    for mount in mount_args(persona)? {
        args.extend(["-v".into(), mount]);
    }
    let mcp_bind = if runtime == Runtime::AppleContainer {
        format!("127.0.0.1:{}:{MCP_PORT}", free_loopback_port()?)
    } else {
        format!("127.0.0.1:0:{MCP_PORT}")
    };
    args.extend([
        "-p".into(),
        mcp_bind,
        "-e".into(),
        format!("HOTLINE_COMPUTER_TOKEN={token}"),
        "-e".into(),
        format!("TZ={}", host_time_zone()),
        "-v".into(),
        format!("{cwd}:{WORKSPACE_MOUNT}"),
        image.into(),
    ]);
    Ok(args)
}

/// The zone the computer's clock shows: the host's, so the bar reads the
/// same as the person's own clock. `TZ` in the desk's environment wins;
/// a host whose zone cannot be read gets UTC rather than an empty `TZ`.
fn host_time_zone() -> String {
    std::env::var("TZ")
        .ok()
        .map(|zone| zone.trim().to_owned())
        .filter(|zone| !zone.is_empty())
        .or_else(|| iana_time_zone::get_timezone().ok())
        .unwrap_or_else(|| "UTC".to_owned())
}

fn home_volume(persona_id: &str) -> String {
    format!("hotline-home-{persona_id}")
}

fn scratch_volume(persona_id: &str) -> String {
    format!("hotline-src-{persona_id}")
}

/// The teammate's memory limit as the runtime spells it, else 2g. A size
/// is digits and at most one unit letter; anything else is refused before
/// the runtime can misread it.
fn memory_of(persona: &Persona) -> Result<String, String> {
    let memory = persona
        .computer
        .as_ref()
        .and_then(|computer| computer.memory.as_deref())
        .map(str::trim)
        .filter(|memory| !memory.is_empty())
        .unwrap_or(DEFAULT_MEMORY);
    let digits = memory.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let unit = &memory[digits.len()..];
    let valid = !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && matches!(
            unit.to_ascii_lowercase().as_str(),
            "" | "b" | "k" | "m" | "g"
        );
    if !valid {
        return Err(format!(
            "The computer's memory limit {memory:?} is not a size like 2g or 512m."
        ));
    }
    Ok(memory.to_string())
}

/// The teammate's process limit, else 512; zero is the runtime's unlimited.
fn pids_of(persona: &Persona) -> String {
    match persona.computer.as_ref().and_then(|computer| computer.pids) {
        None => DEFAULT_PIDS.to_string(),
        Some(0) => "-1".to_string(),
        Some(pids) => pids.to_string(),
    }
}

/// The teammate's extra bind mounts as `host:path[:ro]`. A host folder
/// must exist now, because the runtime would otherwise create it as root;
/// a container path may not shadow or sit inside one of Hotline's own.
fn mount_args(persona: &Persona) -> Result<Vec<String>, String> {
    let Some(computer) = persona.computer.as_ref() else {
        return Ok(Vec::new());
    };
    let reserved = [WORKSPACE_MOUNT, SCRATCH_MOUNT, NIX_MOUNT];
    let mut args = Vec::new();
    for mount in computer.mounts.iter().flatten() {
        let host = expand_home(mount.host.trim());
        let host_path = Path::new(&host);
        if !host_path.is_absolute() {
            return Err(format!(
                "The computer mount {:?} is not an absolute host path.",
                mount.host
            ));
        }
        if !host_path.is_dir() {
            return Err(format!(
                "The computer mount {:?} is not a folder on this machine.",
                mount.host
            ));
        }
        let path = mount.path.trim().trim_end_matches('/');
        if !path.starts_with('/') || path.is_empty() {
            return Err(format!(
                "The computer mount path {:?} is not an absolute path inside the container.",
                mount.path
            ));
        }
        let overlaps = |taken: &str| {
            path == taken
                || path.starts_with(&format!("{taken}/"))
                || taken.starts_with(&format!("{path}/"))
        };
        if reserved.iter().any(|taken| overlaps(taken)) {
            return Err(format!(
                "The computer mount path {path:?} overlaps a path Hotline reserves: {}.",
                reserved.join(", ")
            ));
        }
        let host = abs_cwd(&host);
        args.push(if mount.readonly {
            format!("{host}:{path}:ro")
        } else {
            format!("{host}:{path}")
        });
    }
    Ok(args)
}

fn expand_home(path: &str) -> String {
    if (path == "~" || path.starts_with("~/"))
        && let Some(home) = std::env::var_os("HOME")
    {
        return format!("{}{}", home.to_string_lossy(), &path[1..]);
    }
    path.to_string()
}

fn free_loopback_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("Could not allocate a loopback port: {error}"))?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|error| format!("Could not allocate a loopback port: {error}"))
}

async fn image_present(cmd: &Path, _runtime: Runtime, image: &str) -> bool {
    run(cmd, &["image", "inspect", image], COMMAND_TIMEOUT)
        .await
        .is_ok()
}

async fn pull(cmd: &Path, runtime: Runtime, image: &str) -> Result<String, String> {
    match runtime {
        Runtime::AppleContainer => run(cmd, &["image", "pull", image], PULL_TIMEOUT).await,
        _ => run(cmd, &["pull", image], PULL_TIMEOUT).await,
    }
}

async fn inspect(cmd: &Path, runtime: Runtime, name: &str) -> Result<Inspection, String> {
    match run(cmd, &["inspect", name], COMMAND_TIMEOUT).await {
        Ok(stdout) => parse_inspect(runtime, &stdout),
        Err(error) if is_not_found(&error) => Ok(Inspection {
            exists: false,
            running: false,
            mcp_port: None,
            token: None,
        }),
        Err(error) => Err(error),
    }
}

fn parse_inspect(runtime: Runtime, stdout: &str) -> Result<Inspection, String> {
    let value: Value = serde_json::from_str(stdout)
        .map_err(|error| format!("The runtime's inspect output was not JSON: {error}"))?;
    let object = value
        .as_array()
        .and_then(|items| items.first())
        .or(Some(&value))
        .ok_or_else(|| "The runtime's inspect output named no container.".to_string())?;
    match runtime {
        Runtime::AppleContainer => parse_apple(object),
        _ => parse_docker(object),
    }
}

fn parse_docker(object: &Value) -> Result<Inspection, String> {
    let running = object
        .get("State")
        .and_then(|state| state.get("Running"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(Inspection {
        exists: true,
        running,
        mcp_port: docker_host_port(object, MCP_PORT),
        token: token_from_environment(object.pointer("/Config/Env")),
    })
}

fn token_from_environment(environment: Option<&Value>) -> Option<String> {
    environment?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .find_map(|entry| {
            entry
                .strip_prefix("HOTLINE_COMPUTER_TOKEN=")
                .filter(|token| !token.is_empty())
                .map(str::to_owned)
        })
}

fn docker_host_port(object: &Value, container_port: u16) -> Option<u16> {
    let key = format!("{container_port}/tcp");
    let from_network = object
        .get("NetworkSettings")
        .and_then(|network| network.get("Ports"))
        .and_then(|ports| ports.get(&key));
    let from_bindings = object
        .get("HostConfig")
        .and_then(|config| config.get("PortBindings"))
        .and_then(|ports| ports.get(&key));
    host_port_from(from_network).or_else(|| host_port_from(from_bindings))
}

fn host_port_from(value: Option<&Value>) -> Option<u16> {
    let first = value
        .and_then(Value::as_array)
        .and_then(|items| items.first())?;
    match first.get("HostPort") {
        Some(Value::String(text)) => text.parse().ok(),
        Some(Value::Number(number)) => number.as_u64().and_then(|n| u16::try_from(n).ok()),
        _ => None,
    }
}

fn parse_apple(object: &Value) -> Result<Inspection, String> {
    let running = object
        .get("status")
        .and_then(|status| status.get("state"))
        .and_then(Value::as_str)
        == Some("running");
    let mut mcp_port = None;
    if let Some(ports) = object
        .get("configuration")
        .and_then(|config| config.get("publishedPorts"))
        .and_then(Value::as_array)
    {
        for port in ports {
            let container = port.get("containerPort").and_then(Value::as_u64);
            let host = port
                .get("hostPort")
                .and_then(Value::as_u64)
                .and_then(|n| u16::try_from(n).ok());
            let proto = port.get("proto").and_then(Value::as_str).unwrap_or("tcp");
            if proto != "tcp" {
                continue;
            }
            if let (Some(port), Some(host)) = (container, host)
                && port == u64::from(MCP_PORT)
            {
                mcp_port = Some(host);
            }
        }
    }
    Ok(Inspection {
        exists: true,
        running,
        mcp_port,
        token: token_from_environment(object.pointer("/configuration/initProcess/environment")),
    })
}

fn is_not_found(error: &str) -> bool {
    let message = error.to_ascii_lowercase();
    message.contains("no such object")
        || message.contains("no such container")
        || message.contains("no such image")
        || message.contains("container not found")
        || message.contains("image not found")
        || message.contains("no container with")
}

async fn run(cmd: &Path, args: &[&str], timeout: Duration) -> Result<String, String> {
    let child = Command::new(cmd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("{} could not be started: {error}", cmd.display()))?;
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) if output.status.success() => {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        }
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let detail = stderr.trim();
            let detail = if detail.is_empty() {
                stdout.trim()
            } else {
                detail
            };
            Err(format!(
                "{} {} failed ({}): {detail}",
                cmd.display(),
                args.first().copied().unwrap_or("command"),
                output.status.code().unwrap_or(-1)
            ))
        }
        Ok(Err(error)) => Err(format!("{} could not be started: {error}", cmd.display())),
        Err(_) => Err(format!(
            "{} {} timed out",
            cmd.display(),
            args.first().copied().unwrap_or("command")
        )),
    }
}

async fn wait_healthy(port: u16) -> Result<(), String> {
    let url = format!("http://127.0.0.1:{port}/health");
    let client = reqwest::Client::builder()
        .timeout(HEALTH_PROBE)
        .build()
        .map_err(|error| error.to_string())?;
    let deadline = tokio::time::Instant::now() + HEALTH_TIMEOUT;
    loop {
        if let Ok(response) = client.get(&url).send().await
            && response.status().is_success()
        {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("The computer did not become healthy in time.".into());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// A runtime a test can drive: a shell script that plays `docker`, and the
/// scratch directory it keeps its state in. Shared with the session tests,
/// which start a teammate on a computer this script reports.
#[cfg(all(test, unix))]
pub(crate) mod fixtures {
    use std::fs;
    use std::path::{Path, PathBuf};

    pub(crate) fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "hotline-core-computer-{name}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_millis()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    /// A child writes the script, not this process. A file one thread holds
    /// open for writing is inherited by every fork another thread makes in
    /// that moment, and until that child has exec'd, running the file fails
    /// with "Text file busy"; under a full parallel test run that is one
    /// run in three. `sh` holds the file instead, and has exited before
    /// this returns.
    pub(crate) fn write_script(dir: &Path, name: &str, body: &str) {
        use std::io::Write;
        let path = dir.join(name);
        let mut child = std::process::Command::new("sh")
            .arg("-c")
            .arg(r#"cat > "$1" && chmod 755 "$1""#)
            .arg("sh")
            .arg(&path)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(body.as_bytes())
            .unwrap();
        let status = child.wait().unwrap();
        assert!(status.success(), "writing {}", path.display());
    }

    /// The fake runtime belongs to one test: each writes it into its own
    /// directory with the log it appends to, the state file it reads and the
    /// port it reports baked in. Nothing about it is process-wide, so these
    /// tests run beside each other instead of queueing behind a lock on the
    /// environment. The paths are single quoted and a scratch path holds no
    /// quote to close them with.
    pub(crate) fn fake_runtime(dir: &Path, mcp_port: u16) {
        let head = format!(
            "#!/bin/sh\nLOG='{}'\nSTATE='{}'\nMCP={mcp_port}\n",
            dir.join("argv.log").display(),
            dir.join("state").display(),
        );
        let body = r#"{
  printf '%s' "$1"
  i=1
  for a in "$@"; do
    if [ "$i" -gt 1 ]; then printf ' %s' "$a"; fi
    i=$((i + 1))
  done
  printf '\n'
} >> "$LOG"
cmd=$1
shift || true
case "$cmd" in
  version) echo "fake 1"; exit 0 ;;
  info) echo '[]'; exit 0 ;;
  image) exit 0 ;;
  pull) exit 0 ;;
  inspect)
    state=$(cat "$STATE")
    if [ "$state" = absent ]; then
      echo "Error: No such container: $1" >&2
      exit 1
    fi
    running=false
    [ "$state" = running ] && running=true
    environment='[]'
    if [ -f "${STATE}.token" ]; then environment='["HOTLINE_COMPUTER_TOKEN=fixture-persisted-token"]'; fi
    printf '[{"Config":{"Env":%s},"State":{"Running":%s},"NetworkSettings":{"Ports":{"8787/tcp":[{"HostPort":"%s"}]}}}]\n' "$environment" "$running" "$MCP"
    exit 0
    ;;
  create) echo stopped > "$STATE"; exit 0 ;;
  start) echo running > "$STATE"; exit 0 ;;
  stop) echo stopped > "$STATE"; exit 0 ;;
  rm) echo absent > "$STATE"; exit 0 ;;
  *) echo "unknown: $cmd" >&2; exit 1 ;;
esac
"#;
        write_script(dir, "docker", &format!("{head}{body}"));
    }
}

// These fixtures execute POSIX shell scripts; native process jobs have Windows coverage.
#[cfg(all(test, unix))]
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::contract::{ComputerMount, McpPolicy, PersonaComputer, PolicyMode};
    use std::fs;
    use std::path::Path;

    fn persona(id: &str, cwd: &str) -> Persona {
        Persona {
            node: None,
            id: id.to_string(),
            name: "Ada".to_string(),
            goal: String::new(),
            face: None,
            team: None,
            backend_id: "hotline".to_string(),
            cwd: cwd.to_string(),
            reach: None,
            model_id: None,
            mode_id: None,
            effort_id: None,
            harness_override: None,
            hop_notice: None,
            mcp_policy: McpPolicy {
                mode: PolicyMode::All,
                server_ids: Vec::new(),
            },
            skill_policy: Default::default(),
            background_work: false,
            allowed_senders: Vec::new(),
            web_search_policy: None,
            computer: Some(PersonaComputer {
                enabled: true,
                image: Some("hotline-computer:test".into()),
                memory: None,
                pids: None,
                mounts: None,
                secrets: None,
            }),
            subagents: None,
            session_checkpoints: Vec::new(),
            last_session_id: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    async fn health_on() -> u16 {
        let app = axum::Router::new().route("/health", axum::routing::get(|| async { "ok" }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        port
    }

    fn log_text(path: &Path) -> String {
        fs::read_to_string(path).unwrap_or_default()
    }

    fn create_line(log: &str) -> &str {
        log.lines()
            .find(|line| line.starts_with("create "))
            .unwrap_or_else(|| panic!("no create in {log}"))
    }

    fn in_order(line: &str, needles: &[&str]) {
        let mut from = 0;
        for needle in needles {
            let at = line[from..]
                .find(needle)
                .unwrap_or_else(|| panic!("{needle} missing after {from} in {line}"));
            from += at + needle.len();
        }
    }

    #[tokio::test]
    async fn ensure_running_on_an_absent_container_creates_with_the_hardened_args() {
        let root = scratch("create");
        let cwd = root.join("work");
        fs::create_dir_all(&cwd).unwrap();
        let port = health_on().await;
        fake_runtime(&root, port);
        let log = root.join("argv.log");
        let state = root.join("state");
        fs::write(&state, "absent").unwrap();
        let computers = Computer::with_path(root.as_os_str());
        let ada = persona("ada", cwd.to_str().unwrap());
        let ready = computers
            .ensure_running(&ada, cwd.to_str().unwrap(), None, None, |_| {})
            .await
            .expect("ensure");
        assert_eq!(ready.url, format!("http://127.0.0.1:{port}/mcp"));
        assert!(!ready.token.is_empty());

        let recorded = log_text(&log);
        let create = create_line(&recorded);
        in_order(
            create,
            &[
                "--cap-drop=ALL",
                "--security-opt",
                "no-new-privileges",
                "--pids-limit",
                "1024",
                "--memory",
                "4g",
                "--shm-size",
                "1g",
                "-v",
                "hotline-home-ada:/home/agent",
                "-v",
                "hotline-nix-glibc:/nix",
                "-v",
                "hotline-src-ada:/home/agent/src",
                "-p",
                "127.0.0.1:0:8787",
                "-e",
                "HOTLINE_COMPUTER_TOKEN=",
                "-e",
                "TZ=",
                "-v",
                &format!(
                    "{}:{WORKSPACE_MOUNT}",
                    cwd.canonicalize().unwrap().display()
                ),
                "hotline-computer:test",
            ],
        );
        assert!(create.contains("--name hotline-computer-ada"), "{create}");
        assert!(
            recorded.contains("start hotline-computer-ada"),
            "{recorded}"
        );
    }

    #[test]
    fn the_computers_clock_is_set_to_the_hosts_zone() {
        let zone = host_time_zone();
        assert!(
            !zone.is_empty() && !zone.contains(char::is_whitespace),
            "{zone}"
        );
    }

    #[test]
    fn limits_are_the_teammates_else_the_defaults() {
        let mut ada = persona("ada", "/tmp");
        assert_eq!(memory_of(&ada).unwrap(), "4g");
        assert_eq!(pids_of(&ada), "1024");
        let computer = ada.computer.as_mut().unwrap();
        computer.memory = Some(" 8G ".into());
        computer.pids = Some(0);
        assert_eq!(memory_of(&ada).unwrap(), "8G");
        assert_eq!(pids_of(&ada), "-1");
        ada.computer.as_mut().unwrap().pids = Some(4096);
        assert_eq!(pids_of(&ada), "4096");
        for bad in ["8 gigs", "g", "2gb", "-1"] {
            ada.computer.as_mut().unwrap().memory = Some(bad.into());
            assert!(memory_of(&ada).is_err(), "{bad}");
        }
    }

    #[test]
    fn mounts_are_existing_folders_outside_hotlines_own_paths() {
        let root = scratch("mounts");
        let checkout = root.join("checkout");
        fs::create_dir_all(&checkout).unwrap();
        let mut ada = persona("ada", "/tmp");
        let host = checkout.to_str().unwrap().to_string();
        let mount = |path: &str, readonly: bool| ComputerMount {
            host: host.clone(),
            path: path.into(),
            readonly,
        };
        ada.computer.as_mut().unwrap().mounts = Some(vec![
            mount("/home/agent/hotline-next", true),
            mount("/mnt/data/", false),
        ]);
        let canonical = checkout.canonicalize().unwrap().display().to_string();
        assert_eq!(
            mount_args(&ada).unwrap(),
            vec![
                format!("{canonical}:/home/agent/hotline-next:ro"),
                format!("{canonical}:/mnt/data"),
            ]
        );
        for taken in [
            "/nix",
            "/nix/store",
            "/home/agent",
            "/home/agent/workspace/x",
            "/",
        ] {
            ada.computer.as_mut().unwrap().mounts = Some(vec![mount(taken, true)]);
            assert!(mount_args(&ada).is_err(), "{taken}");
        }
        ada.computer.as_mut().unwrap().mounts = Some(vec![mount("relative", true)]);
        assert!(mount_args(&ada).is_err());
        ada.computer.as_mut().unwrap().mounts = Some(vec![ComputerMount {
            host: root.join("missing").to_str().unwrap().into(),
            path: "/mnt/missing".into(),
            readonly: true,
        }]);
        assert!(mount_args(&ada).is_err());
    }

    #[tokio::test]
    async fn ensure_running_on_a_stopped_container_starts() {
        let root = scratch("start");
        let cwd = root.join("work");
        fs::create_dir_all(&cwd).unwrap();
        let port = health_on().await;
        fake_runtime(&root, port);
        let log = root.join("argv.log");
        let state = root.join("state");
        fs::write(&state, "absent").unwrap();
        let computers = Computer::with_path(root.as_os_str());
        let ada = persona("ada", cwd.to_str().unwrap());
        computers
            .ensure_running(&ada, cwd.to_str().unwrap(), None, None, |_| {})
            .await
            .expect("first");
        computers.stop("ada", None).await.expect("stop");
        fs::write(&log, "").unwrap();
        computers
            .ensure_running(&ada, cwd.to_str().unwrap(), None, None, |_| {})
            .await
            .expect("second");
        let recorded = log_text(&log);
        assert!(
            recorded.lines().any(|line| line.starts_with("start ")),
            "{recorded}"
        );
        assert!(
            !recorded.lines().any(|line| line.starts_with("create ")),
            "a known stopped container must not be recreated: {recorded}"
        );
    }

    #[test]
    fn apple_inspection_recovers_only_the_computer_environment_token() {
        let seen = parse_apple(&serde_json::json!({
            "status":{"state":"running"},
            "configuration":{
                "initProcess":{"environment":["PATH=/usr/bin", "HOTLINE_COMPUTER_TOKEN=fixture-apple-token"]},
                "publishedPorts":[{"containerPort":8787,"hostPort":12345,"proto":"tcp"}]
            }
        })).unwrap();
        assert!(
            status_of(seen, None)
                .viewer
                .is_some_and(|viewer| viewer == "http://127.0.0.1:12345/#fixture-apple-token")
        );
        assert!(
            token_from_environment(Some(&serde_json::json!(["HOTLINE_COMPUTER_TOKEN="]))).is_none()
        );
    }

    #[tokio::test]
    async fn an_existing_desktop_remains_viewable_and_is_not_recreated_after_restart() {
        let root = scratch("recover-token");
        let cwd = root.join("work");
        fs::create_dir_all(&cwd).unwrap();
        let port = health_on().await;
        fake_runtime(&root, port);
        fs::write(root.join("state"), "running").unwrap();
        fs::write(root.join("state.token"), "present").unwrap();
        let computers = Computer::with_path(root.as_os_str());
        let status = computers.status("ada", None).await.unwrap();
        assert!(
            status
                .viewer
                .is_some_and(|url| url.ends_with("/#fixture-persisted-token"))
        );
        let ready = computers
            .ensure_running(
                &persona("ada", cwd.to_str().unwrap()),
                cwd.to_str().unwrap(),
                None,
                None,
                |_| {},
            )
            .await
            .unwrap();
        assert!(ready.token == "fixture-persisted-token");
        let recorded = log_text(&root.join("argv.log"));
        assert!(
            !recorded.lines().any(|line| line.starts_with("rm ")
                || line.starts_with("create ")
                || line.starts_with("start ")),
            "a running desktop was mutated: {recorded}"
        );
    }

    #[tokio::test]
    async fn an_unknown_token_existing_container_is_removed_and_recreated() {
        let root = scratch("unknown");
        let cwd = root.join("work");
        fs::create_dir_all(&cwd).unwrap();
        let port = health_on().await;
        fake_runtime(&root, port);
        let log = root.join("argv.log");
        let state = root.join("state");
        fs::write(&state, "running").unwrap();
        let computers = Computer::with_path(root.as_os_str());
        let ada = persona("ada", cwd.to_str().unwrap());
        computers
            .ensure_running(&ada, cwd.to_str().unwrap(), None, None, |_| {})
            .await
            .expect("ensure");
        let recorded = log_text(&log);
        assert!(
            recorded.lines().any(|line| line.starts_with("rm ")),
            "{recorded}"
        );
        assert!(
            recorded.lines().any(|line| line.starts_with("create ")),
            "{recorded}"
        );
    }

    #[tokio::test]
    async fn stop_and_remove_send_the_right_subcommands() {
        let root = scratch("stop-rm");
        let cwd = root.join("work");
        fs::create_dir_all(&cwd).unwrap();
        let port = health_on().await;
        fake_runtime(&root, port);
        let log = root.join("argv.log");
        let state = root.join("state");
        fs::write(&state, "absent").unwrap();
        let computers = Computer::with_path(root.as_os_str());
        let ada = persona("ada", cwd.to_str().unwrap());
        computers
            .ensure_running(&ada, cwd.to_str().unwrap(), None, None, |_| {})
            .await
            .expect("ensure");
        fs::write(&log, "").unwrap();
        computers.stop("ada", None).await.expect("stop");
        computers.remove("ada", None).await.expect("remove");
        let recorded = log_text(&log);
        assert!(
            recorded
                .lines()
                .any(|line| line == "stop hotline-computer-ada"),
            "{recorded}"
        );
        assert!(
            recorded
                .lines()
                .any(|line| line == "rm -f hotline-computer-ada"),
            "{recorded}"
        );
    }

    #[tokio::test]
    async fn sweep_stops_then_removes_by_idle_age() {
        let root = scratch("sweep");
        let cwd = root.join("work");
        fs::create_dir_all(&cwd).unwrap();
        let port = health_on().await;
        fake_runtime(&root, port);
        let log = root.join("argv.log");
        let state = root.join("state");
        fs::write(&state, "absent").unwrap();
        let computers = Computer::with_path(root.as_os_str());
        let ada = persona("ada", cwd.to_str().unwrap());
        computers
            .ensure_running(&ada, cwd.to_str().unwrap(), None, None, |_| {})
            .await
            .expect("ensure");
        fs::write(&log, "").unwrap();
        let now = now_ms();
        computers
            .sweep(now + COMPUTER_IDLE_STOP_MS + 1, &HashSet::new())
            .await;
        let after_idle = log_text(&log);
        assert!(
            after_idle
                .lines()
                .any(|line| line == "stop hotline-computer-ada"),
            "{after_idle}"
        );
        fs::write(&log, "").unwrap();
        computers
            .sweep(now + COMPUTER_HIBERNATE_MS + 1, &HashSet::new())
            .await;
        let after_hibernate = log_text(&log);
        assert!(
            after_hibernate
                .lines()
                .any(|line| line == "rm -f hotline-computer-ada"),
            "{after_hibernate}"
        );
    }

    #[test]
    fn a_computer_tool_is_named_computer_underscore_underscore() {
        assert!(is_computer_tool("computer__capture", "see the screen"));
        assert!(is_computer_tool("other", "computer__capture"));
        assert!(!is_computer_tool("echo__shout", "echo__shout"));
        assert!(!is_computer_tool("capture", "capture"));
    }

    #[test]
    fn the_image_is_the_teammates_else_the_rooms_else_the_pin() {
        let mut ada = persona("ada", "/tmp");
        ada.computer = Some(PersonaComputer {
            enabled: true,
            image: None,
            memory: None,
            pids: None,
            mounts: None,
            secrets: None,
        });
        assert_eq!(image_of(&ada, None, None), default_image());
        assert_eq!(image_of(&ada, None, Some("0.5.3")), image_at("0.5.3"));
        assert_eq!(
            image_of(&ada, Some("room/image:1"), Some("0.5.3")),
            "room/image:1"
        );
        ada.computer = Some(PersonaComputer {
            enabled: true,
            image: Some("  own/image:2  ".into()),
            memory: None,
            pids: None,
            mounts: None,
            secrets: None,
        });
        assert_eq!(image_of(&ada, Some("room/image:1"), None), "own/image:2");
    }

    #[tokio::test]
    async fn no_runtime_is_the_install_sentence() {
        let root = scratch("none");
        let computers = Computer::with_path(root.as_os_str());
        let ada = persona("ada", root.to_str().unwrap());
        let error = computers
            .ensure_running(&ada, root.to_str().unwrap(), None, None, |_| {})
            .await
            .expect_err("no runtime");
        assert_eq!(
            error,
            "No container runtime was found; install Docker or Podman."
        );
    }
}
