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
use container::{
    create, create_args, folder_mount_args, image_present, inspect, inspect_within, is_not_found,
    limits_of, pick_runtime, pull, run, wait_healthy,
};
pub(crate) use container::{folder_mounts, splits_users};
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
/// A computer is its address and its bearer; `attest` is the key its boot
/// handed this desk, shared by every `Ready` of it (see
/// [`secrets::AttestKey`]), and not part of which computer it is.
/// `made_to_split` is whether this desk made the container to split its
/// users (a release from 0.13, see [`splits_users`]), read from
/// the runtime's label, which nothing inside the container can change: only
/// such a computer can be believed when it says it keeps secrets from its
/// jobs.
#[derive(Clone, Debug)]
pub struct Ready {
    pub url: String,
    pub token: String,
    pub attest: secrets::AttestKey,
    pub made_to_split: bool,
}

impl Ready {
    /// A computer at `url` with `token`, holding no attest key yet, and not
    /// made to split.
    pub fn new(url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            token: token.into(),
            attest: secrets::AttestKey::default(),
            made_to_split: false,
        }
    }
}

impl PartialEq for Ready {
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url && self.token == other.token
    }
}

impl Eq for Ready {}

/// A client for the desk's own requests to a computer. It follows no
/// redirect: every door is on the computer's loopback port and answers
/// there, so a redirect means something else is answering that port, and
/// following it would carry the bearer, a set of secrets or a passkey answer
/// to wherever it points.
pub(crate) fn door_client() -> reqwest::ClientBuilder {
    reqwest::Client::builder().redirect(reqwest::redirect::Policy::none())
}

pub mod cookies;
pub(crate) mod files;
pub(crate) mod gate;
pub mod guide;
pub mod login;
pub mod migration;
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

/// A site of a protected computer's signed-in browser as its screen is asked
/// for: a host, with a port if it has one, in lower case, from a host or an
/// `https://` origin. A login may also be for a site on the computer's own
/// loopback over plain http, which the computer names by its origin (a bare
/// host there means https), so such an origin stays one. Nothing else is a
/// site there.
pub fn site_host(site: &str) -> Option<String> {
    let site = site.trim();
    if let Some(local) = site.strip_prefix("http://") {
        if local.contains("://") {
            return None;
        }
        let host = site_host(local)?;
        let name = host
            .rsplit_once(':')
            .map_or(host.as_str(), |(name, _)| name);
        return matches!(name, "localhost" | "127.0.0.1").then(|| format!("http://{host}"));
    }
    let host = site
        .strip_prefix("https://")
        .unwrap_or(site)
        .trim_end_matches('/')
        .to_ascii_lowercase();
    let (name, port) = match host.rsplit_once(':') {
        Some((name, port)) => (name, Some(port)),
        None => (host.as_str(), None),
    };
    let named = !name.is_empty()
        && name.len() <= 253
        && !name.starts_with(['.', '-'])
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-');
    let ported = port.is_none_or(|port| port.parse::<u16>().is_ok_and(|port| port != 0));
    (named && ported).then_some(host)
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

/// Every teammate computer this process has woken or adopted, keyed by
/// persona id.
///
/// The token is generated for each container when it is made, passed in the
/// runtime CLI's environment, and kept here for its life: a container made
/// again, for new folders, limits or an update, gets a new one, because an
/// older computer's programs could read the old one and a newer computer
/// must not open to it. The desk keeps no copy of its own: a container a
/// previous run of Hotline left has its token read back from the container's
/// own settings by `inspect` and is adopted with it ([`Computer::adopt`]),
/// and its v2 attest key from the OS store ([`secrets::AttestKey`]). Only a
/// container whose token cannot be read, or one retired because the runtime
/// would not remove it, is removed and made again rather than started with a
/// bearer nobody can present.
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
    /// Where each computer's attest key is kept across a desk restart: the
    /// OS store the desk was opened with. None keeps it in memory alone.
    store: Option<Arc<dyn crate::credentials::SecretStore>>,
}

struct Inner {
    containers: HashMap<String, Live>,
    /// One turn at a time per teammate's container: whoever makes, starts,
    /// stops or removes it holds this teammate's lock, so two callers can
    /// never both `create --name` it or `rm -f` what the other just made.
    turns: HashMap<String, Arc<tokio::sync::Mutex<()>>>,
    /// Teammates whose container the runtime has been asked about since the
    /// desk started, so a container a previous desk left behind is found once
    /// (see [`Computer::adopt`]).
    looked: HashSet<String>,
    /// The newest release the desk has heard of, and when to ask again.
    known: releases::Known,
    /// The runtime `status` last picked, for [`PICK_FOR_STATUS`].
    picked: Option<Picked>,
    /// Teammates whose container was to go but the runtime would not
    /// remove: it is never adopted or started again with the bearer it
    /// has, only removed before the next one is made.
    retired: HashSet<String>,
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
    /// The release its image was named when it was made (the label): what
    /// an update would make it on again, which a release candidate's guide,
    /// saying its version without the `-rc.N`, does not tell.
    made_on: Option<String>,
    /// What it proved about keeping secrets at the last handing over.
    protection: Option<crate::contract::ComputerProtection>,
    /// The snapshots its move to split mode left for the person, while
    /// they have not answered.
    migration: Option<crate::contract::ComputerMigration>,
    /// What it said at the last handing over it holds but cannot keep from
    /// its programs, while it proved itself protected.
    readable: Vec<String>,
    /// The key its boot handed this desk, kept in memory and, with a
    /// store, in the OS store under the bearer's hash; forgotten with the
    /// bearer.
    attest: secrets::AttestKey,
}

impl Computer {
    /// The loopback ports this desk's running computers are published on,
    /// which a teammate's desk browser never opens (`crate::browser`).
    pub fn live_ports(&self) -> Vec<u16> {
        self.lock()
            .containers
            .values()
            .map(|live| live.mcp_port)
            .collect()
    }

    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                containers: HashMap::new(),
                turns: HashMap::new(),
                looked: HashSet::new(),
                known: releases::Known::default(),
                picked: None,
                retired: HashSet::new(),
            })),
            capacity: Arc::new(capacity::Cache::default()),
            statuses: Arc::default(),
            runtimes: Arc::default(),
            bins: BinSearch::from_env(),
            releases_url: releases::RELEASES_URL.to_string(),
            store: None,
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
                turns: HashMap::new(),
                looked: HashSet::new(),
                known: releases::Known::default(),
                picked: None,
                retired: HashSet::new(),
            })),
            capacity: Arc::new(capacity::Cache::default()),
            statuses: Arc::default(),
            runtimes: Arc::default(),
            bins: BinSearch::only(path),
            releases_url: "http://127.0.0.1:1/releases".to_string(),
            store: None,
        }
    }

    /// The same desk, asking `url` for releases.
    #[cfg(test)]
    pub fn with_releases(mut self, url: &str) -> Self {
        self.releases_url = url.to_string();
        self
    }

    /// The same desk, keeping each computer's attest key in `store` (see
    /// [`secrets::AttestKey`]), so a restart does not lose it.
    pub fn with_store(mut self, store: Arc<dyn crate::credentials::SecretStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// A new attest key for `persona_id`'s computer under `token`: kept in
    /// the store when this desk has one, in memory alone when not.
    fn attest_for(&self, persona_id: &str, token: &str) -> secrets::AttestKey {
        match &self.store {
            Some(store) => secrets::AttestKey::kept(store.clone(), persona_id, token),
            None => secrets::AttestKey::default(),
        }
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

    /// The lock for one teammate's container.
    fn turn_of(&self, persona_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.lock()
            .turns
            .entry(persona_id.to_string())
            .or_default()
            .clone()
    }

    /// Callers waiting on, or holding, a teammate's container lock.
    #[cfg(all(test, unix))]
    fn queued(&self, persona_id: &str) -> usize {
        self.lock()
            .turns
            .get(persona_id)
            .map_or(0, |turn| Arc::strong_count(turn) - 1)
    }

    /// Makes the teammate's computer and starts it, or finds it running.
    /// Calls for one teammate go one at a time: the second sees what the
    /// first made.
    pub async fn ensure_running(
        &self,
        persona: &Persona,
        workspace_cwd: &str,
        prefer: Option<Runtime>,
        room_image: Option<&str>,
        report: impl FnMut(PullReport),
    ) -> Result<Ready, String> {
        // A container's workspace mount is made from this path, and a runtime
        // follows a link there as every other holder would.
        if let Some(refused) = crate::session::moved_workspace(persona, workspace_cwd)
            .or_else(|| crate::session::moved_folder(persona))
        {
            return Err(refused);
        }
        let _turn = self.turn_of(&persona.id).lock_owned().await;
        self.ensure_running_held(persona, workspace_cwd, prefer, room_image, report)
            .await
    }

    async fn ensure_running_held(
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
        let known = self.lock().containers.get(&persona.id).cloned();
        let known_token = known.as_ref().map(|live| live.token.clone());

        let mut inspection = inspect(&cmd, runtime, &name).await?;
        let known_token = inspection.token.clone().or(known_token);
        let retired = self.lock().retired.contains(&persona.id);
        if inspection.exists && (known_token.is_none() || retired) {
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
        // A container kept keeps its bearer; one made now gets its own, never
        // the one the container it replaces had.
        let token = match known_token {
            Some(token) if inspection.exists => token,
            _ => new_token(),
        };
        let mut made_on = inspection.release.clone();
        // A container started now is on a boot nobody has asked the key of.
        let started = !inspection.running;
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
            made_on = Some(release_of(&image));
            let args = create_args(runtime, &name, &image, &cwd, persona)?;
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            create(&cmd, &arg_refs, &token).await?;
            self.lock().retired.remove(&persona.id);
            run(&cmd, &["start", &name], COMMAND_TIMEOUT).await?;
        } else if !inspection.running {
            run(&cmd, &["start", &name], COMMAND_TIMEOUT).await?;
        }

        let inspection = inspect(&cmd, runtime, &name).await?;
        let mcp_port = inspection.mcp_port.ok_or_else(|| {
            format!("Could not read the computer's published port for {MCP_PORT}/tcp")
        })?;
        // The attest key goes with the bearer: kept for the container kept,
        // forgotten, entry and all, for one made again.
        let attest = match known {
            Some(live) if live.token == token => live.attest,
            Some(live) => {
                live.attest.forget().await;
                self.attest_for(&persona.id, &token)
            }
            None => self.attest_for(&persona.id, &token),
        };
        self.lock().containers.insert(
            persona.id.clone(),
            Live {
                runtime,
                cmd: cmd.clone(),
                token: token.clone(),
                mcp_port,
                last_activity_ms: now_ms(),
                release: None,
                made_on: made_on.clone(),
                protection: None,
                migration: None,
                readable: Vec::new(),
                attest: attest.clone(),
            },
        );
        wait_healthy(mcp_port).await?;
        let ready = Ready {
            url: mcp_url(mcp_port),
            token,
            attest,
            made_to_split: made_to_split(made_on.as_deref()),
        };
        // A boot hands its attest key once, to the first bearer holder who
        // asks: the desk asks before any session is handed the computer. A
        // computer left running holds on to the boot whose key this desk
        // kept, so it is not asked again: its door would answer 409. One not
        // made to split is never believed, so it is not asked at all.
        if ready.made_to_split && (started || !ready.attest.holds().await) {
            secrets::take_attest_key(&ready).await;
        }
        Ok(ready)
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
            attest: live.attest,
            made_to_split: made_to_split(live.made_on.as_deref()),
        })
    }

    /// Remembers the release a computer said it was, for the status the pane
    /// shows against the release it would be created on now.
    pub fn learned_release(&self, persona_id: &str, release: &str) {
        if let Some(live) = self.lock().containers.get_mut(persona_id) {
            live.release = Some(release.to_string());
        }
    }

    /// The release a teammate's running computer was made on, as its image
    /// named it, while this desk holds it.
    pub fn made_on(&self, persona_id: &str) -> Option<String> {
        self.lock()
            .containers
            .get(persona_id)
            .and_then(|live| live.made_on.clone())
    }

    /// Remembers what the computer proved at its last handing over, for
    /// the pane's one notice when grants wait for an update.
    pub fn learned_protection(
        &self,
        persona_id: &str,
        protection: crate::contract::ComputerProtection,
    ) {
        if let Some(live) = self.lock().containers.get_mut(persona_id) {
            live.protection = Some(protection);
        }
    }

    /// What a protected computer said at its last handing over it holds but
    /// cannot keep from its programs; empty for any other.
    pub fn learned_readable(&self, persona_id: &str, readable: Vec<String>) {
        if let Some(live) = self.lock().containers.get_mut(persona_id) {
            live.readable = readable;
        }
    }

    /// What the running computer proved at its last handing over, while
    /// this desk holds it.
    pub fn protection(&self, persona_id: &str) -> Option<crate::contract::ComputerProtection> {
        self.lock()
            .containers
            .get(persona_id)
            .and_then(|live| live.protection)
    }

    /// Remembers what the computer's move to split mode left for the person
    /// to decide, for the pane's upgrade notice.
    pub fn learned_migration(
        &self,
        persona_id: &str,
        migration: Option<crate::contract::ComputerMigration>,
    ) {
        if let Some(live) = self.lock().containers.get_mut(persona_id) {
            live.migration = migration;
        }
    }

    pub fn mark_idle(&self, persona_id: &str, at_ms: i64) {
        if let Some(live) = self.lock().containers.get_mut(persona_id) {
            live.last_activity_ms = at_ms;
        }
    }

    pub async fn stop(&self, persona_id: &str, prefer: Option<Runtime>) -> Result<(), String> {
        let _turn = self.turn_of(persona_id).lock_owned().await;
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

    /// Removes the teammate's computer and forgets its bearer. One the
    /// runtime will not remove is stopped if it can be, retired so that
    /// nothing adopts or starts it again with that bearer, and said.
    pub async fn remove(&self, persona_id: &str, prefer: Option<Runtime>) -> Result<(), String> {
        let _turn = self.turn_of(persona_id).lock_owned().await;
        let name = container_name(persona_id);
        let removed = self.lock().containers.remove(persona_id);
        if let Some(live) = &removed {
            live.attest.forget().await;
        }
        let cmd = match removed.map(|live| live.cmd) {
            Some(cmd) => cmd,
            None => match pick_runtime(prefer, &self.bins).await {
                Ok((_, cmd)) => cmd,
                Err(_) => return Ok(()),
            },
        };
        if let Err(error) = run(&cmd, &["rm", "-f", &name], COMMAND_TIMEOUT).await
            && !is_not_found(&error)
        {
            self.lock().retired.insert(persona_id.to_string());
            let _ = run(&cmd, &["stop", &name], STOP_TIMEOUT).await;
            return Err(format!("The computer could not be removed: {error}"));
        }
        self.lock().retired.remove(persona_id);
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
                    protection: None,
                    readable: Vec::new(),
                    migration: None,
                });
            };
            let limits_changed = limits_changed(&inspection, persona, live.runtime);
            let mut status = status_of(inspection, Some((live.mcp_port, &live.token)));
            status.release = live.release.or(status.release);
            // A stopped computer starts again as what it proved, so the pane
            // can say before a grant whether logins go to it readable.
            if status.state != ComputerState::Absent {
                status.protection = live.protection;
                status.readable = live.readable;
            }
            if status.state == ComputerState::Running {
                status.migration = live.migration;
            }
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
                protection: None,
                readable: Vec::new(),
                migration: None,
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
                protection: None,
                readable: Vec::new(),
                migration: None,
            });
        };
        let limits_changed = limits_changed(&inspection, persona, runtime);
        let mut status = status_of(inspection, None);
        status.limits_changed = limits_changed;
        Ok(status)
    }

    /// Finds the containers a previous desk left behind. A container is in no
    /// map after a restart, so nothing would ever idle or hibernate it; each
    /// teammate in `ids` is asked about once, and one that exists is taken
    /// into the map with its idle clock starting at `now_ms`. A container
    /// whose bearer cannot be read cannot be talked to: it is stopped, and
    /// the next start of that teammate makes it again.
    pub async fn adopt(&self, ids: &[String], prefer: Option<Runtime>, now_ms: i64) {
        let todo: Vec<String> = {
            let inner = self.lock();
            ids.iter()
                .filter(|id| {
                    !inner.containers.contains_key(*id)
                        && !inner.looked.contains(*id)
                        && !inner.retired.contains(*id)
                })
                .cloned()
                .collect()
        };
        if todo.is_empty() {
            return;
        }
        let Ok((runtime, cmd)) = pick_runtime(prefer, &self.bins).await else {
            return;
        };
        for id in todo {
            // A teammate being brought up is not left over from anything.
            let Ok(_turn) = self.turn_of(&id).try_lock_owned() else {
                continue;
            };
            if self.lock().containers.contains_key(&id) {
                continue;
            }
            let name = container_name(&id);
            let Ok(seen) = inspect(&cmd, runtime, &name).await else {
                continue;
            };
            let unreachable = {
                let mut inner = self.lock();
                inner.looked.insert(id.clone());
                match (seen.exists, seen.token) {
                    (false, _) => false,
                    (true, Some(token)) => {
                        let attest = self.attest_for(&id, &token);
                        inner.containers.insert(
                            id,
                            Live {
                                runtime,
                                cmd: cmd.clone(),
                                token,
                                mcp_port: seen.mcp_port.unwrap_or(0),
                                last_activity_ms: now_ms,
                                made_on: seen.release.clone(),
                                release: seen.release,
                                protection: None,
                                migration: None,
                                readable: Vec::new(),
                                attest,
                            },
                        );
                        false
                    }
                    (true, None) => seen.running,
                }
            };
            if unreachable {
                let _ = run(&cmd, &["stop", &name], STOP_TIMEOUT).await;
            }
        }
    }

    /// Stop containers idle longer than [`COMPUTER_IDLE_STOP_MS`]; remove
    /// ones idle longer than [`COMPUTER_HIBERNATE_MS`]. `in_use` are the
    /// teammates still using theirs — a session running, or any thread
    /// holding the gate's lease — and their idle clock starts over now: a
    /// teammate still working is not idle. One being brought up is skipped.
    pub async fn sweep(&self, now_ms: i64, in_use: &HashSet<String>) {
        let ids: Vec<(String, i64, PathBuf, Runtime)> = {
            let mut inner = self.lock();
            for id in in_use {
                if let Some(live) = inner.containers.get_mut(id) {
                    live.last_activity_ms = now_ms;
                }
            }
            inner
                .containers
                .iter()
                .filter(|(id, _)| !in_use.contains(id.as_str()))
                .map(|(id, live)| {
                    (
                        id.clone(),
                        live.last_activity_ms,
                        live.cmd.clone(),
                        live.runtime,
                    )
                })
                .collect()
        };
        for (id, last, cmd, runtime) in ids {
            let idle = now_ms.saturating_sub(last);
            let name = container_name(&id);
            let Ok(_turn) = self.turn_of(&id).try_lock_owned() else {
                continue;
            };
            if idle >= COMPUTER_HIBERNATE_MS {
                // A container the runtime would not remove is still ours to
                // try again at the next sweep.
                if run(&cmd, &["rm", "-f", &name], COMMAND_TIMEOUT)
                    .await
                    .is_ok()
                {
                    let removed = self.lock().containers.remove(&id);
                    if let Some(live) = removed {
                        live.attest.forget().await;
                    }
                }
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
            protection: None,
            readable: Vec::new(),
            migration: None,
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
            protection: None,
            readable: Vec::new(),
            migration: None,
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
        protection: None,
        readable: Vec::new(),
        migration: None,
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
/// none. The registry's own port is not a tag, nor a pinned digest
/// (`repo:0.13.0@sha256:…` is 0.13.0; `repo@sha256:…` names no release).
pub fn release_of(image: &str) -> String {
    let image = image.split_once('@').map_or(image, |(named, _)| named);
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

/// Whether a container made on `made_on` was made to split its users.
fn made_to_split(made_on: Option<&str>) -> bool {
    made_on.is_some_and(splits_users)
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
