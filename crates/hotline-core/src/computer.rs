//! One Linux desktop per teammate, as a container Hotline starts and stops.
//!
//! The image is a contract, not a binary: anything that serves streamable
//! HTTP MCP at `http://127.0.0.1:<port>/mcp` with bearer `HOTLINE_COMPUTER_TOKEN`,
//! `/health` open, and the viewer page at `/` on the same port is a valid
//! computer. This module is the Hotline-side lifecycle — wake, idle stop,
//! hibernate — and the grant a session is handed. The agent and the image
//! are Hotline Computer, a repository of their own.

mod capacity;
mod container;
pub mod runtime;

use crate::contract::{
    ComputerCapacity, ComputerReleases, ComputerState, ComputerStatus, Persona, RuntimeReport,
};
use crate::mcp::{HttpAuth, McpServer, McpTransport};
pub(crate) use container::folder_mounts;
use container::{
    create_args, folder_mount_args, image_present, inspect, inspect_within, limits_of,
    pick_runtime, pull, run, wait_healthy,
};
pub(crate) use hotline_room::computer::HOME_MOUNT;
use runtime::{BinSearch, Runtime};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// The Hotline Computer release this desktop is built against: the floor. A
/// new computer is created on the newest published release at or above it
/// on the same major (see [`releases`]), and on this one when nothing newer
/// is known. Bumped here deliberately — never derived from the desktop
/// version, and never `latest`.
pub const COMPUTER_VERSION: &str = "0.11.0";

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
/// Where the teammate's granted folders appear, one directory each, named
/// after the folder. Reserved, so a declared mount cannot cover one.
pub(crate) const FOLDERS_MOUNT: &str = "/home/agent/folders";
/// The environment entry that records which folders a container was made
/// with, so a container made with others is made again rather than kept.
const FOLDERS_ENV: &str = "HOTLINE_FOLDERS";
/// The environment entry that records the memory and CPU limits a
/// container was made with, which only a new container can change.
const LIMITS_ENV: &str = "HOTLINE_LIMITS";
/// The image's release survives the desk that created the container.
const RELEASE_LABEL: &str = "hotline.release";
/// The glibc image seeds a writable store; old Alpine volumes remain available.
const NIX_VOLUME: &str = "hotline-nix-glibc";
const DEFAULT_MEMORY: &str = "4g";
const DEFAULT_PIDS: u32 = 1024;

const HEALTH_TIMEOUT: Duration = Duration::from_secs(30);
const HEALTH_PROBE: Duration = Duration::from_secs(2);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
/// How long `status` lets one inspect take. The panes poll it every few
/// seconds, so a wedged daemon should read as absent rather than hold the
/// answer for as long as a create is allowed to run.
const STATUS_TIMEOUT: Duration = Duration::from_secs(3);
/// How long `status` trusts its last runtime pick. The panes ask every few
/// seconds; probing every runtime each time spawns a handful of CLIs a tick.
const PICK_FOR_STATUS: Duration = Duration::from_secs(30);
/// How long the list of installed runtimes is trusted. Probing runs each CLI,
/// and the settings pane that asks is not worth that every time it opens.
const RUNTIMES_CACHE: Duration = Duration::from_secs(30);
const PULL_TIMEOUT: Duration = Duration::from_mins(10);
const STOP_TIMEOUT: Duration = Duration::from_secs(60);

/// One step of an image pull, as the runtime's own output tells it: how
/// many layers it named, how many have landed, and whether it is over.
///
/// The desk never asks the registry for sizes; it reads the lines `docker
/// pull` and `podman pull` print for a terminal that is not one, which name
/// each layer as it starts and as it finishes. A runtime whose lines the
/// desk does not read leaves `layers_total` at zero: the pull is still
/// reported, as started and as over, with nothing to count in between.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullReport {
    pub image: String,
    pub layers_done: u32,
    pub layers_total: u32,
    pub outcome: PullOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PullOutcome {
    Pulling,
    Done { elapsed_ms: i64 },
    Failed,
}

/// What the grant and the in-process client need once the machine is up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ready {
    pub url: String,
    pub token: String,
}

pub mod cookies;
pub(crate) mod files;
pub(crate) mod gate;
pub mod guide;
pub mod login;
pub mod passkeys;
pub mod releases;
pub mod secrets;
pub(crate) mod uses;

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

/// An answer and when it was got, behind a lock that is held while the next
/// one is fetched, so overlapping askers share the fetch.
type Remembered<T> = tokio::sync::Mutex<Option<(Instant, T)>>;

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
    capacity: Arc<capacity::Cache>,
    /// The last status each teammate's computer answered, for
    /// [`Computer::status`] to share between polls that overlap.
    statuses: Arc<Mutex<HashMap<String, Arc<Remembered<ComputerStatus>>>>>,
    /// The runtimes the last probe found, for [`RUNTIMES_CACHE`]. The lock is
    /// held across the probe so overlapping asks share it.
    runtimes: Arc<Remembered<Vec<RuntimeReport>>>,
    /// Where published releases are listed; a test points this at its own.
    releases_url: String,
}

struct Inner {
    containers: HashMap<String, Live>,
    /// The newest release the desk has heard of, and when to ask again.
    known: releases::Known,
    /// The runtime `status` last picked, for [`PICK_FOR_STATUS`].
    picked: Option<Picked>,
}

struct Picked {
    prefer: Option<Runtime>,
    at: Instant,
    answer: Result<(Runtime, PathBuf), String>,
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
                picked: None,
            })),
            capacity: Arc::new(capacity::Cache::default()),
            statuses: Arc::default(),
            runtimes: Arc::default(),
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
                picked: None,
            })),
            capacity: Arc::new(capacity::Cache::default()),
            statuses: Arc::default(),
            runtimes: Arc::default(),
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

    /// Capacity is advisory for the next create, not a live container limit.
    /// Cache each runtime preference for a minute and share in-flight probes.
    pub async fn capacity(&self, prefer: Option<Runtime>) -> ComputerCapacity {
        self.capacity
            .get(prefer, &capacity::SystemProbe(&self.bins))
            .await
    }

    pub async fn runtimes(&self) -> Vec<RuntimeReport> {
        let mut last = self.runtimes.lock().await;
        if let Some((at, reports)) = &*last
            && at.elapsed() < RUNTIMES_CACHE
        {
            return reports.clone();
        }
        let reports = runtime::detect_with(&self.bins).await;
        *last = Some((Instant::now(), reports.clone()));
        reports
    }

    /// `pick_runtime`, remembered for [`PICK_FOR_STATUS`]. Only `status` uses
    /// it: it is what the panes poll, and a stale answer there costs a few
    /// seconds of an old state, where starting a computer needs a fresh one.
    async fn pick_for_status(&self, prefer: Option<Runtime>) -> Result<(Runtime, PathBuf), String> {
        if let Some(picked) = &self.lock().picked
            && picked.prefer == prefer
            && picked.at.elapsed() < PICK_FOR_STATUS
        {
            return picked.answer.clone();
        }
        let answer = pick_runtime(prefer, &self.bins).await;
        self.lock().picked = Some(Picked {
            prefer,
            at: Instant::now(),
            answer: answer.clone(),
        });
        answer
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Downloads the image a computer for `persona` would be created on now,
    /// without touching the one running. An update pulls ahead of the swap,
    /// so the teammate keeps its computer for as long as the download takes.
    pub async fn prepare(
        &self,
        persona: &Persona,
        prefer: Option<Runtime>,
        room_image: Option<&str>,
        report: impl FnMut(PullReport),
    ) -> Result<(), String> {
        let (runtime, cmd) = pick_runtime(prefer, &self.bins).await?;
        if pinned_image(persona, room_image).is_none() {
            self.refresh_releases(now_ms()).await;
        }
        let image = self.image_for(persona, room_image);
        if !image_present(&cmd, runtime, &image).await {
            pull(&cmd, runtime, &image, report).await?;
        }
        Ok(())
    }

    pub async fn ensure_running(
        &self,
        persona: &Persona,
        workspace_cwd: &str,
        prefer: Option<Runtime>,
        room_image: Option<&str>,
        report: impl FnMut(PullReport),
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

        // The folders a container can see are fixed when it is made, so one
        // made with other folders than the teammate has now is made again:
        // a folder taken away must not stay mounted. Named volumes keep the
        // home, the store and scratch; the container's own layer goes.
        // Memory and CPUs are the same: a runtime takes them at create, so
        // a container made with others is made again. What one made before
        // the limits were recorded has is unknown, so it is made again the
        // next time it starts from stopped; one already running is kept.
        let folders = folder_mount_args(persona)?.join("\n");
        let limits = limits_of(persona, runtime)?;
        let other_limits = inspection
            .limits
            .as_ref()
            .map_or(!inspection.running, |made| *made != limits);
        if inspection.exists && (inspection.folders != folders || other_limits) {
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
                pull(&cmd, runtime, &image, report).await?;
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

    /// A teammate's computer as it is now. Polls that overlap share one
    /// inspect: a caller that waited behind another takes the answer that
    /// one produced, because it was asked for after this caller did. With
    /// the teammate's settings, it says whether the running computer's
    /// limits are theirs.
    pub async fn status(
        &self,
        persona_id: &str,
        persona: Option<&Persona>,
        prefer: Option<Runtime>,
    ) -> Result<ComputerStatus, String> {
        let asked = Instant::now();
        let slot = self
            .statuses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(persona_id.to_string())
            .or_default()
            .clone();
        let mut last = slot.lock().await;
        if let Some((at, status)) = &*last
            && *at >= asked
        {
            return Ok(status.clone());
        }
        let status = self.inspect_status(persona_id, persona, prefer).await?;
        *last = Some((Instant::now(), status.clone()));
        Ok(status)
    }

    async fn inspect_status(
        &self,
        persona_id: &str,
        persona: Option<&Persona>,
        prefer: Option<Runtime>,
    ) -> Result<ComputerStatus, String> {
        let name = container_name(persona_id);
        let known = self.lock().containers.get(persona_id).cloned();
        if let Some(live) = known {
            let Ok(inspection) =
                inspect_within(&live.cmd, live.runtime, &name, STATUS_TIMEOUT).await
            else {
                return Ok(ComputerStatus {
                    state: ComputerState::Absent,
                    url: None,
                    viewer: None,
                    release: None,
                    available: None,
                    update_failed: None,
                    limits_changed: false,
                });
            };
            let limits_changed = limits_changed(&inspection, persona, live.runtime);
            let mut status = status_of(inspection, Some((live.mcp_port, &live.token)));
            status.release = live.release.or(status.release);
            status.limits_changed = limits_changed;
            return Ok(status);
        }
        let Ok((runtime, cmd)) = self.pick_for_status(prefer).await else {
            return Ok(ComputerStatus {
                state: ComputerState::Absent,
                url: None,
                viewer: None,
                release: None,
                available: None,
                update_failed: None,
                limits_changed: false,
            });
        };
        let Ok(inspection) = inspect_within(&cmd, runtime, &name, STATUS_TIMEOUT).await else {
            return Ok(ComputerStatus {
                state: ComputerState::Absent,
                url: None,
                viewer: None,
                release: None,
                available: None,
                update_failed: None,
                limits_changed: false,
            });
        };
        let limits_changed = limits_changed(&inspection, persona, runtime);
        let mut status = status_of(inspection, None);
        status.limits_changed = limits_changed;
        Ok(status)
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
    /// The folder mounts it was made with; empty for one made before
    /// folders, which had none.
    folders: String,
    /// The memory and CPU limits it was made with; `None` for one made
    /// before they were recorded.
    limits: Option<String>,
    /// The creation label, or the runtime's image tag for older containers.
    release: Option<String>,
}

/// Whether a running computer was made with other limits than the
/// teammate's settings give now. One made before limits were recorded is
/// not said to differ: what it has is unknown, and its next start from
/// stopped remakes it anyway.
fn limits_changed(inspection: &Inspection, persona: Option<&Persona>, runtime: Runtime) -> bool {
    let (Some(made), Some(persona)) = (&inspection.limits, persona) else {
        return false;
    };
    inspection.running && limits_of(persona, runtime).ok().as_ref() != Some(made)
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
            update_failed: None,
            limits_changed: false,
        };
    }
    if !inspection.running {
        return ComputerStatus {
            state: ComputerState::Stopped,
            url: None,
            viewer: None,
            release: inspection.release,
            available: None,
            update_failed: None,
            limits_changed: false,
        };
    }
    let mcp = inspection.mcp_port.or(known.map(|known| known.0));
    ComputerStatus {
        state: ComputerState::Running,
        release: inspection.release,
        available: None,
        update_failed: None,
        limits_changed: false,
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

/// `dunce` rather than `std::fs`: on Windows the latter returns a `\\?\`
/// verbatim path, which `docker create -v` rejects as not a valid path.
fn abs_cwd(cwd: &str) -> String {
    dunce::canonicalize(cwd).map_or_else(
        |_| cwd.to_string(),
        |path| path.to_string_lossy().into_owned(),
    )
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

/// A runtime a test can drive: a shell script that plays `docker`, and the
/// scratch directory it keeps its state in. Shared with the session tests,
/// which start a teammate on a computer this script reports.
#[cfg(all(test, unix))]
pub(crate) mod fixtures;

// These fixtures execute POSIX shell scripts; native process jobs have Windows coverage.
#[cfg(all(test, unix))]
mod tests;
