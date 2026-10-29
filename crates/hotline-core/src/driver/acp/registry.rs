//! Which external agents Hotline can start on this machine, and how.
//!
//! The list is data, not code. It comes from three places, in this order:
//! a table of agents Hotline has been taught by hand, the ACP registry's
//! published catalogue (fetched once a day and cached, so a session start
//! never waits on the network and an offline machine still gets the table),
//! and a probe of what is actually installed. A day-old catalogue is still
//! the one used until a fetch replaces it: a fetch that fails leaves the last
//! good copy where it was. A locally installed binary
//! always wins over a downloadable one, because the local copy carries the
//! user's login. An agent the catalogue ships only as a prebuilt archive is
//! downloaded the first time a session starts it (see `install`).
//!
//! Hotline Agent is deliberately absent. This module answers "which child
//! process, started how", and the built-in agent has no child process, no
//! binary to find and nothing to download.

use crate::paths;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// The registry's published catalogue: one request for every agent, carrying
/// names, descriptions and launch commands. Preferred over walking the GitHub
/// repository, which costs a request per agent out of an unauthenticated
/// budget shared with everything else on the machine.
const REGISTRY_URL: &str = "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";

/// What this build keeps of each catalogue entry. A cache written by a build
/// that kept less (before binary distributions, say) is refetched rather than
/// trusted for the rest of its day, because the missing fields cannot be
/// told apart from an agent that published none.
const CATALOGUE_FORMAT: u32 = 2;

/// How long a fetched catalogue is used before Hotline asks again.
const CACHE_TTL_MS: i64 = 24 * 60 * 60 * 1000;

/// How long the catalogue fetch may take before it is given up on. The cached
/// copy — or, failing that, the table below — is the answer either way.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// How a backend is started: a command, its arguments and the environment
/// its catalogue entry asks for, as spawned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl Launch {
    /// The command as a person would type it to try it in a terminal. The
    /// environment is left out: it is the catalogue's, and may carry keys.
    pub fn line(&self) -> String {
        std::iter::once(self.command.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Whether this launch has answered on this machine before. One that has not
/// may be `npx` or `uvx` fetching its package, which is a download and not a
/// hang, so its start is given longer (see `StartBounds`).
pub fn started_before(root: &Path, launch: &Launch) -> bool {
    read_started(root).contains(&launch.line())
}

/// Records that this launch answered, or forgets that it did. A launch that
/// stopped answering is forgotten, because its package may have been evicted
/// from the cache and be downloading again; its next start gets the longer
/// bound.
pub fn mark_started(root: &Path, launch: &Launch, started: bool) {
    let line = launch.line();
    let mut lines = read_started(root);
    if lines.contains(&line) == started {
        return;
    }
    if started {
        lines.push(line);
    } else {
        lines.retain(|known| *known != line);
    }
    let path = paths::acp_started_path(root);
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, serde_json::to_vec(&lines).unwrap_or_default()));
    if let Err(error) = written {
        eprintln!("[acp] {} could not be written: {error}", path.display());
    }
}

fn read_started(root: &Path) -> Vec<String> {
    std::fs::read(paths::acp_started_path(root))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// A prebuilt archive the catalogue publishes for this machine, and the
/// folder it unpacks into. The launch command lives inside `dir`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Archive {
    pub url: String,
    pub sha256: Option<String>,
    pub dir: PathBuf,
}

/// One agent a teammate can be run on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Backend {
    pub id: String,
    pub name: String,
    pub description: String,
    /// How to start it, when Hotline knows a way. `None` is an agent the
    /// catalogue lists with nothing published for this machine.
    pub launch: Option<Launch>,
    /// Where `launch` comes from when it is a prebuilt archive: downloaded
    /// before the first start, then reused.
    pub archive: Option<Archive>,
    /// Absent when this backend can be started here; a sentence saying what is
    /// missing when it cannot. A missing login is not missing: that shows up
    /// when the session starts, and is not a reason to grey out the row.
    pub unavailable: Option<String>,
}

/// An agent whose own binary speaks ACP. Finding it on PATH is both the
/// availability check and the launch, because they are the same thing.
struct Native {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    command: &'static str,
    args: &'static [&'static str],
}

/// An agent reached through an adapter: it does not speak ACP itself, so an
/// npm package translates for it.
///
/// The adapter is a translator, not the agent. `npx` will fetch the shim on
/// demand, so the shim is never the question — a shim with nothing to
/// translate for cannot run a turn, and offering it as available sends
/// somebody into a session that dies on the first prompt. So availability is
/// whether `client` is installed. The pinned `package` covers a cold cache
/// with no network; the catalogue's version wins when there is one.
struct Adapted {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    package: &'static str,
    client: &'static str,
}

const NATIVE: &[Native] = &[
    Native {
        id: "cursor",
        name: "Cursor",
        description: "Cursor's agent CLI, signed in with your Cursor account.",
        command: "cursor-agent",
        args: &["acp"],
    },
    Native {
        id: "grok-build",
        name: "Grok Build",
        description: "xAI's Grok CLI, signed in with your Grok account.",
        command: "grok",
        args: &["agent", "stdio"],
    },
    Native {
        id: "opencode",
        name: "opencode",
        description: "The opencode CLI, with the providers you set up in opencode.",
        command: "opencode",
        args: &["acp"],
    },
    Native {
        id: "gemini",
        name: "Gemini CLI",
        description: "Google's Gemini CLI, with its own sign-in.",
        command: "gemini",
        args: &["--acp"],
    },
];

const ADAPTED: &[Adapted] = &[
    Adapted {
        id: "claude-acp",
        name: "Claude Code",
        description: "Anthropic's Claude Code CLI, with your Claude Code login or ANTHROPIC_API_KEY.",
        package: "@agentclientprotocol/claude-agent-acp@0.78.0",
        client: "claude",
    },
    Adapted {
        id: "codex-acp",
        name: "Codex",
        description: "OpenAI's Codex CLI, with your Codex login or OPENAI_API_KEY.",
        package: "@agentclientprotocol/codex-acp@1.12.0",
        client: "codex",
    },
    // BrokkAi's adapter, which drives `muse serve`; the npm `muse-acp`
    // package is an unrelated project.
    Adapted {
        id: "muse-acp",
        name: "Muse Code",
        description: "Meta's Muse Code CLI, with your Muse Code login.",
        package: "@brokkai/muse-acp@0.13.0",
        client: "muse",
    },
];

/// How long Hotline leaves the catalogue alone after trying for it, whether
/// or not it got it. Without this a machine that is offline pays the whole
/// fetch timeout on every list the picker or the welcome asks for.
const RETRY_AFTER: Duration = Duration::from_mins(10);

/// When each room last went after its catalogue, for [`RETRY_AFTER`].
static LAST_TRIED: Mutex<Vec<(PathBuf, Instant)>> = Mutex::new(Vec::new());

/// Whether this room may go after the catalogue now, and if so, that it has.
fn due_for_a_try(root: &Path) -> bool {
    let mut tried = LAST_TRIED.lock().unwrap_or_else(PoisonError::into_inner);
    let now = Instant::now();
    match tried.iter_mut().find(|(path, _)| path == root) {
        Some((_, at)) if at.elapsed() < RETRY_AFTER => false,
        Some((_, at)) => {
            *at = now;
            true
        }
        None => {
            tried.push((root.to_path_buf(), now));
            true
        }
    }
}

/// Every agent Hotline can offer, catalogue included.
///
/// This is the listing a picker draws. It answers from the cached catalogue
/// at once and, when that copy is a day old, refreshes it in the background
/// for the next listing, so nobody opening the picker waits on a fetch.
pub async fn backends(root: &Path) -> Vec<Backend> {
    refresh_in_background(root);
    cached_backends(root)
}

/// Fetches the catalogue again in the background when the cached copy is a
/// day old, and returns at once. Both the picker and a session start ask: a
/// desk whose picker is never opened would otherwise start its agents from
/// a catalogue as old as its last look, adapter versions included. The start
/// that asks still runs on the copy it has; the next one gets the new one.
pub fn refresh_in_background(root: &Path) {
    refresh_in_background_from(root, REGISTRY_URL);
}

fn refresh_in_background_from(root: &Path, url: &str) -> Option<tokio::task::JoinHandle<()>> {
    let stale = read_catalogue(root).is_none_or(|catalogue| !fresh(&catalogue, now_ms()));
    if !stale || !due_for_a_try(root) {
        return None;
    }
    let root = root.to_path_buf();
    let url = url.to_string();
    Some(tokio::spawn(
        async move { refresh_catalogue(&root, &url).await },
    ))
}

/// The same listing without touching the network, which is what starting a
/// session reads.
pub fn cached_backends(root: &Path) -> Vec<Backend> {
    let catalogue = read_catalogue(root).unwrap_or_default();
    let machine = Machine::here();
    let mut backends: Vec<Backend> = Vec::new();

    for native in NATIVE {
        let found = which(native.command);
        backends.push(Backend {
            id: native.id.to_string(),
            name: native.name.to_string(),
            description: native.description.to_string(),
            launch: Some(Launch {
                command: found.as_deref().map_or_else(
                    || native.command.to_string(),
                    |found| found.to_string_lossy().into_owned(),
                ),
                args: native.args.iter().map(|arg| (*arg).to_string()).collect(),
                env: Vec::new(),
            }),
            archive: None,
            unavailable: found.is_none().then(|| machine.missing(native.command)),
        });
    }

    for adapted in ADAPTED {
        let published = catalogue
            .agents
            .iter()
            .find(|agent| agent.id == adapted.id)
            .and_then(|agent| match start_for(root, agent)? {
                Start::Run(launch) => Some(launch),
                Start::Fetch(..) => None,
            });
        let launch = published.unwrap_or_else(|| npx(adapted.package, &[]));
        backends.push(Backend {
            id: adapted.id.to_string(),
            name: adapted.name.to_string(),
            description: adapted.description.to_string(),
            unavailable: adapter_missing(&machine, adapted.client, &launch.command),
            launch: Some(launch),
            archive: None,
        });
    }

    let mut listed: Vec<Backend> = catalogue
        .agents
        .iter()
        .filter(|agent| !backends.iter().any(|known| known.id == agent.id))
        .map(|agent| {
            let (launch, archive, unavailable) = match start_for(root, agent) {
                None => (
                    None,
                    None,
                    Some("is not published for this computer".to_string()),
                ),
                Some(Start::Run(launch)) => {
                    let missing = which(&launch.command)
                        .is_none()
                        .then(|| machine.missing(&launch.command));
                    (Some(launch), None, missing)
                }
                // Downloadable is available: the first start fetches it.
                Some(Start::Fetch(launch, archive)) => (Some(launch), Some(archive), None),
            };
            Backend {
                id: agent.id.clone(),
                name: agent.name.clone().unwrap_or_else(|| agent.id.clone()),
                description: agent.description.clone().unwrap_or_default(),
                launch,
                archive,
                unavailable,
            }
        })
        .collect();
    listed.sort_by(|a, b| a.name.cmp(&b.name));
    backends.append(&mut listed);
    backends
}

/// The backend with this id, or `None` when nothing on this machine knows it.
pub fn known(root: &Path, backend_id: &str) -> Option<Backend> {
    cached_backends(root)
        .into_iter()
        .find(|backend| backend.id == backend_id)
}

/// The name a person knows this backend by, without asking the machine
/// whether it is installed: a colleague reading the roster wants to know what
/// a teammate runs, not whether this desk could start it. `None` for an id
/// neither this build nor the cached catalogue knows.
pub fn name_of(root: &Path, backend_id: &str) -> Option<String> {
    if let Some(native) = NATIVE.iter().find(|native| native.id == backend_id) {
        return Some(native.name.to_string());
    }
    if let Some(adapted) = ADAPTED.iter().find(|adapted| adapted.id == backend_id) {
        return Some(adapted.name.to_string());
    }
    let agent = read_catalogue(root)?
        .agents
        .into_iter()
        .find(|agent| agent.id == backend_id)?;
    Some(agent.name.unwrap_or(agent.id))
}

/// An agent people know by name: a native CLI or one Hotline adapts. These
/// are looked at for their models (and their sign-in) before any teammate
/// runs on them; the catalogue's many others only once one has.
pub fn named(backend_id: &str) -> bool {
    NATIVE.iter().any(|native| native.id == backend_id)
        || ADAPTED.iter().any(|adapted| adapted.id == backend_id)
}

/// The command that starts this backend, or the sentence saying why nothing
/// can.
pub fn launch(root: &Path, backend_id: &str) -> Result<Launch, String> {
    let Some(backend) = known(root, backend_id) else {
        return Err(format!(
            "Backend \"{backend_id}\" is not an agent this machine knows. Install its CLI, or pick a different agent."
        ));
    };
    match (backend.unavailable, backend.launch) {
        (Some(reason), _) => Err(cannot_start(&backend.name, &reason)),
        (None, None) => Err(format!("{} has no launch command.", backend.name)),
        // Already absolute, inside Hotline's own folder; `install` puts it there.
        (None, Some(launch)) if backend.archive.is_some() => Ok(launch),
        (None, Some(mut launch)) => {
            // Capture the executable once; terminal auth must not resolve a new
            // PATH after initialize (its advertised environment can change PATH).
            let binary = which(&launch.command).ok_or("The harness executable is unavailable.")?;
            launch.command = std::path::absolute(binary)
                .map_err(|_| "Could not resolve the harness executable.")?
                .to_string_lossy()
                .into_owned();
            Ok(launch)
        }
    }
}

/// The launch a look at this agent's models may use: this one, or, for an
/// agent shipped as an archive whose newest release is not fetched yet, the
/// release fetched before it that last answered here. A release is fetched
/// when a teammate starts on it; until then the one on disk still says what
/// the agent offers, so a new release never leaves its models unread.
pub fn for_a_look(root: &Path, backend_id: &str, launch: Launch) -> Launch {
    let Some(archive) = known(root, backend_id).and_then(|backend| backend.archive) else {
        return launch;
    };
    if Path::new(&launch.command).is_file() {
        return launch;
    }
    let (Ok(inner), Some(releases)) = (
        Path::new(&launch.command).strip_prefix(&archive.dir),
        archive.dir.parent(),
    ) else {
        return launch;
    };
    let mut fetched: Vec<(std::time::SystemTime, Launch)> = std::fs::read_dir(releases)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|release| {
            let command = release.path().join(inner);
            let earlier = Launch {
                command: command.to_string_lossy().into_owned(),
                ..launch.clone()
            };
            let answered = command.is_file() && started_before(root, &earlier);
            answered.then_some((release.metadata().ok()?.modified().ok()?, earlier))
        })
        .collect();
    fetched.sort_by_key(|(at, _)| *at);
    fetched.pop().map_or(launch, |(_, earlier)| earlier)
}

/// The error for a start that cannot happen. The reason is a sentence of its
/// own, as `unavailable` is on the wire, so it is not the tail of this one.
fn cannot_start(name: &str, reason: &str) -> String {
    format!("{name} cannot start: {}.", reason.trim_end_matches('.'))
}

/// Downloads and unpacks this backend's archive when it ships as one and is
/// not here yet. Anything else is already as installed as Hotline can make it.
pub async fn install(root: &Path, backend_id: &str) -> Result<(), String> {
    let Some(backend) = known(root, backend_id) else {
        return Ok(());
    };
    let (Some(archive), Some(launch)) = (backend.archive, backend.launch) else {
        return Ok(());
    };
    if Path::new(&launch.command).is_file() {
        return Ok(());
    }
    super::install::fetch(&archive, Path::new(&launch.command))
        .await
        .map_err(|error| format!("Could not install {}: {error}", backend.name))
}

// -- the published catalogue -----------------------------------------------

/// One agent as the ACP registry publishes it. Only the fields Hotline reads are
/// named; the rest of the entry rides along in the cache untouched.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Published {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    distribution: Option<Distribution>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Distribution {
    #[serde(default)]
    npx: Option<Runner>,
    #[serde(default)]
    uvx: Option<Runner>,
    /// Prebuilt archives keyed by platform, `linux-x86_64` and the like.
    #[serde(default)]
    binary: Option<BTreeMap<String, Target>>,
}

/// One platform's archive: where to fetch it, its digest when published,
/// and the command inside it, relative to where it unpacks.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Target {
    archive: String,
    cmd: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Runner {
    package: String,
    #[serde(default)]
    args: Vec<String>,
}

/// The cached catalogue, with the moment it was fetched.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Catalogue {
    #[serde(default)]
    format: u32,
    fetched_at: i64,
    #[serde(default)]
    agents: Vec<Published>,
}

/// How a catalogue entry starts: a command to run as it is, or one inside an
/// archive that has to be fetched first.
enum Start {
    Run(Launch),
    Fetch(Launch, Archive),
}

/// How a catalogue entry is started, or `None` when Hotline cannot start it.
///
/// `npx` and `uvx` both fetch on demand, so they need no install step and are
/// preferred. A binary distribution is an archive for this platform, unpacked
/// into its own folder under the data directory; the folder is named for the
/// archive's URL, so a new release is a new folder and never a half-replaced one.
fn start_for(root: &Path, agent: &Published) -> Option<Start> {
    let distribution = agent.distribution.as_ref()?;
    if let Some(runner) = &distribution.npx {
        return Some(Start::Run(npx(&runner.package, &runner.args)));
    }
    if let Some(runner) = &distribution.uvx {
        let mut args = vec![runner.package.clone()];
        args.extend(runner.args.iter().cloned());
        return Some(Start::Run(Launch {
            command: "uvx".to_string(),
            args,
            env: Vec::new(),
        }));
    }
    let target = distribution.binary.as_ref()?.get(&platform()?)?;
    let command = inside(&target.cmd)?;
    let dir = paths::acp_agents_dir(root)
        .join(inside(&agent.id)?)
        .join(folder_for(&target.archive));
    let launch = Launch {
        command: std::path::absolute(dir.join(command))
            .ok()?
            .to_string_lossy()
            .into_owned(),
        args: target.args.clone(),
        env: target.env.clone().into_iter().collect(),
    };
    Some(Start::Fetch(
        launch,
        Archive {
            url: target.archive.clone(),
            sha256: target.sha256.clone(),
            dir,
        },
    ))
}

/// The catalogue's name for this machine: `linux-x86_64`, `darwin-aarch64`,
/// `windows-x86_64`.
fn platform() -> Option<String> {
    let os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "darwin",
        "windows" => "windows",
        _ => return None,
    };
    let arch @ ("x86_64" | "aarch64") = std::env::consts::ARCH else {
        return None;
    };
    Some(format!("{os}-{arch}"))
}

/// A path from the catalogue as one that stays inside the folder it is
/// joined to: `./bin/agent` is `bin/agent`, and anything with `..`, a root or
/// a drive in it is refused rather than followed out of Hotline's directory.
fn inside(path: &str) -> Option<PathBuf> {
    let mut relative = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!relative.as_os_str().is_empty()).then_some(relative)
}

fn folder_for(url: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(&Sha256::digest(url.as_bytes())[..8])
}

/// What a hand-taught adapter row is missing, or nothing when it can start.
///
/// Two things have to be here: the harness's own CLI, which is what signs
/// itself in, and whatever starts its ACP adapter — usually `npx`, which is a
/// different program and may well not be installed. A row that probed only
/// the first offered a start that fails at the spawn, with `unavailable`
/// saying nothing.
fn adapter_missing(machine: &Machine, client: &str, launcher: &str) -> Option<String> {
    if which(client).is_none() {
        return Some(machine.missing(client));
    }
    which(launcher).is_none().then(|| machine.missing(launcher))
}

fn npx(package: &str, extra: &[String]) -> Launch {
    let mut args = vec!["-y".to_string(), package.to_string()];
    args.extend(extra.iter().cloned());
    Launch {
        command: "npx".to_string(),
        args,
        env: Vec::new(),
    }
}

/// Whether a cached catalogue is still the day's.
///
/// A stamp from the future is not fresh, and neither is one this machine
/// cannot subtract from. The cache is a file, so `fetchedAt` is whatever is in
/// it: a clock that was ahead when the fetch happened would otherwise pin that
/// catalogue until real time caught up, and a hand-edited number would take
/// the picker down with an overflow.
fn still_the_days(fetched_at: i64, now: i64) -> bool {
    (0..CACHE_TTL_MS).contains(&now.saturating_sub(fetched_at))
}

/// Whether a cached catalogue can stand in for a fetch: written by this
/// build's format, and still the day's.
fn fresh(catalogue: &Catalogue, now: i64) -> bool {
    catalogue.format == CATALOGUE_FORMAT && still_the_days(catalogue.fetched_at, now)
}

fn read_catalogue(root: &Path) -> Option<Catalogue> {
    let bytes = std::fs::read(paths::acp_registry_path(root)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Fetches the catalogue when the cached copy is a day old, and says nothing
/// when it cannot. A fetch that fails, or answers with anything but a list of
/// agents, writes nothing, so the last good catalogue stays in use however
/// old it is; with none ever fetched, the cost is the agents Hotline was not
/// taught by hand, and never the ones it was.
async fn refresh_catalogue(root: &Path, url: &str) {
    let cached = read_catalogue(root);
    if cached.is_some_and(|catalogue| fresh(&catalogue, now_ms())) {
        return;
    }
    let fetched = reqwest::Client::new()
        .get(url)
        .timeout(FETCH_TIMEOUT)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status);
    let body = match fetched {
        Ok(response) => response.json::<serde_json::Value>().await.ok(),
        Err(_) => None,
    };
    let agents: Vec<Published> = body
        .as_ref()
        .and_then(|body| body.get("agents"))
        .and_then(|agents| serde_json::from_value(agents.clone()).ok())
        .unwrap_or_default();
    if agents.is_empty() {
        return;
    }
    let catalogue = Catalogue {
        format: CATALOGUE_FORMAT,
        fetched_at: now_ms(),
        agents,
    };
    // Written beside the cache and renamed over it, so a desk stopped
    // halfway through the write still has the last good copy, not half of
    // the new one.
    let path = paths::acp_registry_path(root);
    let partial = path.with_extension(format!("{}.partial", uuid::Uuid::new_v4()));
    let Ok(bytes) = serde_json::to_vec(&catalogue) else {
        return;
    };
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&partial, bytes))
        .and_then(|()| std::fs::rename(&partial, &path));
    if written.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
}

// -- why a command is missing ------------------------------------------------

/// Where a person's own installs of a CLI go, under their home: the folder
/// the Claude and Codex installers use, npm's per-user prefix, and Claude's
/// older local install.
const PER_USER_BINS: &[&str] = &[".local/bin", ".npm-global/bin", ".claude/local"];

/// Where a system-wide install goes.
#[cfg(unix)]
const SYSTEM_BINS: &[&str] = &["/usr/local/bin", "/usr/bin"];

/// A login account on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Account {
    name: String,
    home: PathBuf,
}

/// What can be said about a command that is not on the desk's PATH: who runs
/// the desk, and who else has an account here. A served desk runs as a
/// service account with its own HOME and PATH, so a CLI installed and signed
/// in for the person who set it up is invisible to it, and "Not installed"
/// sends them to install what they already have.
///
/// Only whether an executable is there is ever asked, by its metadata. Nobody's
/// files are opened, and a folder this account may not enter says nothing.
#[derive(Default)]
struct Machine {
    /// The account the desk runs as.
    own_name: String,
    /// The desk's own HOME, which a service sets and passwd may not agree with.
    own_home: Option<PathBuf>,
    /// The people's accounts, other than the desk's own, by name.
    others: Vec<Account>,
    /// Where a system-wide install would be.
    system_bins: Vec<PathBuf>,
}

impl Machine {
    #[cfg(unix)]
    fn here() -> Machine {
        // SAFETY: geteuid takes no arguments and cannot fail.
        let uid = unsafe { libc::geteuid() };
        let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
        let (own_name, others) = read_accounts(&passwd, uid);
        Machine {
            own_name: own_name
                .or_else(|| std::env::var("USER").ok())
                .unwrap_or_else(|| format!("uid {uid}")),
            own_home: std::env::var_os("HOME").map(PathBuf::from),
            others,
            system_bins: SYSTEM_BINS.iter().map(PathBuf::from).collect(),
        }
    }

    /// Windows has no other accounts to look through, and its PATH rules are
    /// the one `which` already follows.
    #[cfg(not(unix))]
    fn here() -> Machine {
        Machine::default()
    }

    /// Why `command` cannot be started: where it is, if it is somewhere the
    /// desk cannot see, else the plain fact.
    fn missing(&self, command: &str) -> String {
        // In the desk's own home or a system folder, but not on its PATH.
        let own = self.own_home.iter().flat_map(|home| {
            PER_USER_BINS
                .iter()
                .map(move |folder| home.join(folder).join(command))
        });
        let system = self.system_bins.iter().map(|folder| folder.join(command));
        if let Some(found) = own.chain(system).find(|path| executable(path)) {
            let folder = found
                .parent()
                .map_or_else(String::new, |folder| folder.display().to_string());
            return format!(
                "{command} is at {}, which is not on the desk's PATH. Add {folder} to PATH in the service unit (see docs/serve.md).",
                found.display()
            );
        }
        // Installed for somebody else, who is not who the desk runs as.
        let owners: Vec<&str> = self
            .others
            .iter()
            .filter(|account| {
                PER_USER_BINS
                    .iter()
                    .any(|folder| executable(&account.home.join(folder).join(command)))
            })
            .map(|account| account.name.as_str())
            .collect();
        let own = &self.own_name;
        match owners.as_slice() {
            [] => "Not installed".to_string(),
            [only] => format!(
                "{command} is installed for {only}, but the desk runs as {own}. Run the desk as {only} (see docs/serve.md), or install {command} for {own}."
            ),
            [rest @ .., last] => format!(
                "{command} is installed for {} and {last}, but the desk runs as {own}. Run the desk as one of them (see docs/serve.md), or install {command} for {own}.",
                rest.join(", ")
            ),
        }
    }
}

/// Whether there is a program here, by its metadata alone.
fn executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

/// The name of the account with this uid, and the people's other accounts, from
/// the text of a passwd file. A person's account is one a login can use: a uid
/// in the range a Linux distribution gives people, a shell that is not
/// `nologin` or `false`, and a home that is a path.
#[cfg(unix)]
fn read_accounts(passwd: &str, uid: u32) -> (Option<String>, Vec<Account>) {
    let mut own = None;
    let mut others = Vec::new();
    for line in passwd.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        let [name, _, id, _, _, home, shell] = fields[..] else {
            continue;
        };
        let Ok(id) = id.parse::<u32>() else {
            continue;
        };
        if id == uid {
            own = Some(name.to_string());
        } else if (1000..60000).contains(&id)
            && Path::new(home).is_absolute()
            && !shell.ends_with("/nologin")
            && !shell.ends_with("/false")
        {
            others.push(Account {
                name: name.to_string(),
                home: PathBuf::from(home),
            });
        }
    }
    others.sort_by(|a, b| a.name.cmp(&b.name));
    (own, others)
}

// -- PATH -------------------------------------------------------------------

/// The absolute path of a command on PATH, so spawning does not depend on
/// what PATH the child happens to inherit.
///
/// A name with a separator in it is a path already and is answered as itself
/// when it exists. On Windows a bare name is tried with each `PATHEXT`
/// suffix, which is where `npx` actually lives there.
pub(crate) fn which(command: &str) -> Option<PathBuf> {
    if command.contains(['/', '\\']) {
        let path = PathBuf::from(command);
        return path.is_file().then_some(path);
    }
    let extensions: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
            .split(';')
            .map(str::to_string)
            .collect()
    } else {
        vec![String::new()]
    };
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .flat_map(|directory| {
            extensions
                .iter()
                .map(move |extension| directory.join(format!("{command}{extension}")))
        })
        .find(|candidate| candidate.is_file())
}

fn now_ms() -> i64 {
    chrono::Local::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "hotline-core-registry-{name}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// A catalogue that cannot be fetched is not asked for again on every
    /// listing: the second ask inside the window is told to wait.
    #[test]
    fn a_failed_catalogue_fetch_is_not_retried_straight_away() {
        let root = scratch("backoff");
        assert!(due_for_a_try(&root));
        assert!(!due_for_a_try(&root));
        assert!(due_for_a_try(&scratch("backoff-elsewhere")));
    }

    fn write_catalogue(root: &Path, agents: serde_json::Value) {
        let path = paths::acp_registry_path(root);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            serde_json::to_vec(&serde_json::json!({
                "fetchedAt": now_ms(),
                "agents": agents,
            }))
            .unwrap(),
        )
        .unwrap();
    }

    /// A catalogue this build wrote two days ago, listing one agent Hotline
    /// was not taught by hand.
    fn write_days_old_catalogue(root: &Path, agent_id: &str) {
        let path = paths::acp_registry_path(root);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            serde_json::to_vec(&serde_json::json!({
                "format": CATALOGUE_FORMAT,
                "fetchedAt": now_ms() - 2 * CACHE_TTL_MS,
                "agents": [{"id": agent_id, "distribution": {"npx": {"package": agent_id}}}],
            }))
            .unwrap(),
        )
        .unwrap();
    }

    fn lists(root: &Path, agent_id: &str) -> bool {
        cached_backends(root)
            .iter()
            .any(|backend| backend.id == agent_id)
    }

    /// A catalogue endpoint on this machine that answers one request with
    /// `status` and `body`, once `hold` lets it when there is one.
    async fn endpoint(
        status: u16,
        body: &str,
        hold: Option<tokio::sync::oneshot::Receiver<()>>,
    ) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/registry.json", listener.local_addr().unwrap());
        let body = body.to_string();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4096];
            let _ = socket.read(&mut request).await;
            if let Some(hold) = hold {
                let _ = hold.await;
            }
            let reply = format!(
                "HTTP/1.1 {status} Answer\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(reply.as_bytes()).await;
        });
        url
    }

    /// A day-old catalogue is the last good one. A fetch that fails — no
    /// answer, an error status, a body that is not a list of agents, an empty
    /// list — leaves it in use; only a fetch that brings agents replaces it.
    #[tokio::test]
    async fn a_failed_fetch_keeps_the_last_good_list() {
        let root = scratch("keeps-last-good");
        write_days_old_catalogue(&root, "amp");
        assert!(lists(&root, "amp"), "a day-old catalogue is still listed");

        let nobody = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}/registry.json", listener.local_addr().unwrap())
        };
        refresh_catalogue(&root, &nobody).await;
        for (status, body) in [
            (500, r#"{"agents": [{"id": "from-an-error"}]}"#),
            (200, "<html>maintenance</html>"),
            (200, r#"{"agents": "not a list"}"#),
            (200, r#"{"agents": []}"#),
        ] {
            refresh_catalogue(&root, &endpoint(status, body, None).await).await;
            assert!(lists(&root, "amp"), "{status} {body}");
            assert!(!lists(&root, "from-an-error"));
        }
        assert!(!fresh(&read_catalogue(&root).unwrap(), now_ms()));

        let good = r#"{"agents": [{"id": "zed", "distribution": {"npx": {"package": "zed"}}}]}"#;
        refresh_catalogue(&root, &endpoint(200, good, None).await).await;
        assert!(lists(&root, "zed"));
        assert!(!lists(&root, "amp"));
        assert!(fresh(&read_catalogue(&root).unwrap(), now_ms()));
        let leftovers: Vec<_> =
            std::fs::read_dir(paths::acp_registry_path(&root).parent().unwrap())
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .filter(|name| name.to_string_lossy().contains("partial"))
                .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    /// A session start on a day-old catalogue asks for a new one and does not
    /// wait for it: the refresh is under way while the endpoint is still
    /// holding its answer, the start's own listing is the old copy, and the
    /// listing after the answer is the new one. A fresh catalogue is not
    /// fetched at all, and a stale one is not asked for twice in a row.
    #[tokio::test]
    async fn a_stale_catalogue_is_refreshed_behind_a_session_start_without_holding_it_up() {
        let root = scratch("background-refresh");
        write_days_old_catalogue(&root, "amp");
        let (answer, hold) = tokio::sync::oneshot::channel();
        let good = r#"{"agents": [{"id": "zed", "distribution": {"npx": {"package": "zed"}}}]}"#;
        let url = endpoint(200, good, Some(hold)).await;

        let asked = Instant::now();
        let refreshing =
            refresh_in_background_from(&root, &url).expect("a day-old catalogue is fetched");
        assert!(
            asked.elapsed() < Duration::from_secs(1),
            "{:?}",
            asked.elapsed()
        );
        assert!(lists(&root, "amp"), "the start runs on the copy it has");
        assert!(
            refresh_in_background_from(&root, &url).is_none(),
            "asked once"
        );

        answer.send(()).unwrap();
        refreshing.await.unwrap();
        assert!(lists(&root, "zed"), "the next start gets the new one");

        let fresh_root = scratch("background-refresh-fresh");
        std::fs::create_dir_all(paths::acp_registry_path(&fresh_root).parent().unwrap()).unwrap();
        std::fs::write(
            paths::acp_registry_path(&fresh_root),
            serde_json::to_vec(&Catalogue {
                format: CATALOGUE_FORMAT,
                fetched_at: now_ms(),
                agents: Vec::new(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(refresh_in_background_from(&fresh_root, &url).is_none());
    }

    /// The table Hotline was taught is answered with no catalogue and no network,
    /// which is the state of a machine that has never been online.
    #[test]
    fn the_hand_taught_table_stands_without_a_catalogue() {
        let root = scratch("bare");
        let backends = cached_backends(&root);
        for native in NATIVE {
            let found = backends
                .iter()
                .find(|backend| backend.id == native.id)
                .unwrap_or_else(|| panic!("{} is missing", native.id));
            assert_eq!(
                found.launch.as_ref().unwrap().args,
                native
                    .args
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            );
        }
        let claude = backends
            .iter()
            .find(|backend| backend.id == "claude-acp")
            .unwrap();
        assert_eq!(claude.launch.as_ref().unwrap().command, "npx");
        assert!(
            claude.launch.as_ref().unwrap().args[1]
                .starts_with("@agentclientprotocol/claude-agent-acp@")
        );
    }

    /// A catalogue entry Hotline was not taught is listed, and the version it
    /// publishes for an adapter Hotline WAS taught replaces the pinned fallback.
    #[test]
    fn the_catalogue_adds_agents_and_updates_the_adapters_version() {
        let root = scratch("catalogue");
        write_catalogue(
            &root,
            serde_json::json!([
                {
                    "id": "claude-acp",
                    "distribution": {"npx": {"package": "@agentclientprotocol/claude-agent-acp@9.9.9"}}
                },
                {
                    "id": "amp",
                    "name": "Amp",
                    "description": "Sourcegraph's agent.",
                    "distribution": {"npx": {"package": "@sourcegraph/amp", "args": ["--acp"]}}
                },
                {"id": "archived", "name": "Archived", "distribution": {}}
            ]),
        );
        let backends = cached_backends(&root);

        let claude = backends.iter().find(|b| b.id == "claude-acp").unwrap();
        assert_eq!(
            claude.launch.as_ref().unwrap().args,
            ["-y", "@agentclientprotocol/claude-agent-acp@9.9.9"]
        );

        let amp = backends.iter().find(|b| b.id == "amp").unwrap();
        assert_eq!(amp.name, "Amp");
        assert_eq!(
            amp.launch.as_ref().unwrap().args,
            ["-y", "@sourcegraph/amp", "--acp"]
        );

        // Nothing to run and nothing to fetch is not an agent Hotline can offer.
        let archived = backends.iter().find(|b| b.id == "archived").unwrap();
        assert_eq!(archived.launch, None);
        assert!(archived.unavailable.is_some());
        assert!(launch(&root, "archived").is_err());
    }

    /// An agent shipped only as a prebuilt archive is offered when there is
    /// one for this machine, and starts from its own folder under the data
    /// directory with the catalogue's arguments and environment.
    #[test]
    fn a_prebuilt_archive_for_this_machine_is_offered_and_started_from_hotlines_folder() {
        let root = scratch("binary");
        let here = platform().unwrap();
        write_catalogue(
            &root,
            serde_json::json!([
                {
                    "id": "antigravity-acp",
                    "name": "Google Antigravity",
                    "distribution": {"binary": {(here.as_str()): {
                        "archive": "https://example.com/agy-acp-server.zip",
                        "cmd": "./agy_acp_server.par",
                        "args": ["--uid="],
                        "env": {"AGY_ACP": "1"}
                    }}}
                },
                {
                    "id": "elsewhere",
                    "distribution": {"binary": {"plan9-mips": {
                        "archive": "https://example.com/x.tar.gz", "cmd": "./x"
                    }}}
                },
                {
                    "id": "escapes",
                    "distribution": {"binary": {(here.as_str()): {
                        "archive": "https://example.com/x.tar.gz", "cmd": "../../bin/sh"
                    }}}
                }
            ]),
        );
        let backends = cached_backends(&root);

        let agy = backends.iter().find(|b| b.id == "antigravity-acp").unwrap();
        assert_eq!(agy.unavailable, None);
        let archive = agy.archive.as_ref().unwrap();
        assert!(
            archive
                .dir
                .starts_with(paths::acp_agents_dir(&root).join("antigravity-acp"))
        );
        let started = launch(&root, "antigravity-acp").unwrap();
        assert_eq!(
            Path::new(&started.command),
            std::path::absolute(archive.dir.join("agy_acp_server.par")).unwrap()
        );
        assert_eq!(started.args, ["--uid="]);
        assert_eq!(started.env, [("AGY_ACP".to_string(), "1".to_string())]);

        // Nothing for this machine, or a command that climbs out of its folder,
        // is not something Hotline can offer.
        for id in ["elsewhere", "escapes"] {
            let backend = backends.iter().find(|b| b.id == id).unwrap();
            assert!(backend.unavailable.is_some(), "{id}");
            assert!(launch(&root, id).is_err(), "{id}");
        }
    }

    /// A new release of an archived agent is fetched only when a teammate
    /// starts on it. Until then a look at its models uses the release on
    /// disk that last answered here; with none, the new one.
    #[test]
    fn a_look_uses_the_fetched_release_until_the_new_one_is() {
        let root = scratch("release");
        let here = platform().unwrap();
        let release = |version: &str| {
            write_catalogue(
                &root,
                serde_json::json!([{
                    "id": "antigravity-acp",
                    "distribution": {"binary": {(here.as_str()): {
                        "archive": format!("https://example.com/agy-{version}.zip"),
                        "cmd": "./agy_acp_server.par"
                    }}}
                }]),
            );
            launch(&root, "antigravity-acp").unwrap()
        };
        let old = release("1.4.3");
        // Nothing fetched yet: the look is the new release's own.
        assert_eq!(for_a_look(&root, "antigravity-acp", old.clone()), old);
        let path = Path::new(&old.command);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
        mark_started(&root, &old, true);

        let new = release("1.4.4");
        assert_ne!(new, old);
        assert_eq!(for_a_look(&root, "antigravity-acp", new.clone()), old);
        // Fetched: the new release is the look's.
        let path = Path::new(&new.command);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
        assert_eq!(for_a_look(&root, "antigravity-acp", new.clone()), new);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_catalogue_path_never_leaves_its_folder() {
        assert_eq!(inside("./bin/agent"), Some(PathBuf::from("bin/agent")));
        assert_eq!(inside("agent"), Some(PathBuf::from("agent")));
        assert_eq!(inside("../agent"), None);
        assert_eq!(inside("bin/../../agent"), None);
        assert_eq!(inside("/usr/bin/agent"), None);
        assert_eq!(inside("."), None);
    }

    #[test]
    fn a_backend_nobody_has_heard_of_is_refused_by_name() {
        let root = scratch("unknown");
        assert!(known(&root, "nonesuch").is_none());
        let error = launch(&root, "nonesuch").unwrap_err();
        assert!(error.contains("nonesuch"), "{error}");
    }

    #[test]
    fn a_command_is_found_on_path_and_a_missing_one_is_not() {
        assert!(which("sh").is_some() || cfg!(windows));
        assert!(which("this-command-does-not-exist-anywhere").is_none());
    }

    /// The cache is a file, and `fetchedAt` is whatever is in it.
    #[test]
    fn a_cached_catalogue_is_the_days_only_while_it_is_behind_us() {
        let now = 1_700_000_000_000;
        assert!(still_the_days(now, now));
        assert!(still_the_days(now - CACHE_TTL_MS + 1, now));
        assert!(!still_the_days(now - CACHE_TTL_MS, now));
        // A clock that was ahead when the fetch happened would otherwise pin
        // this catalogue until real time caught up with the stamp.
        assert!(!still_the_days(now + 3_600_000, now));
        // And a number nothing can be subtracted from is stale, not a panic.
        assert!(!still_the_days(i64::MIN, now));
        assert!(!still_the_days(i64::MAX, now));
    }

    /// A cache an older build wrote dropped fields this one reads, so it is
    /// refetched however young it is.
    #[test]
    fn a_catalogue_an_older_build_cached_is_fetched_again() {
        let now = 1_700_000_000_000;
        let older: Catalogue =
            serde_json::from_value(serde_json::json!({"fetchedAt": now, "agents": []})).unwrap();
        assert!(!fresh(&older, now));
        let current = Catalogue {
            format: CATALOGUE_FORMAT,
            fetched_at: now,
            agents: Vec::new(),
        };
        assert!(fresh(&current, now));
    }

    /// An adapter is two programs: the harness's own CLI and whatever starts
    /// its ACP adapter. A row that probed only the first was offered as
    /// startable and failed at the spawn.
    #[test]
    fn an_adapter_needs_both_its_harness_and_whatever_starts_it() {
        const NOWHERE: &str = "this-command-does-not-exist-anywhere";
        if cfg!(windows) {
            return;
        }
        let nobody_else = Machine::default();
        assert_eq!(adapter_missing(&nobody_else, "sh", "sh"), None);
        assert_eq!(
            adapter_missing(&nobody_else, "sh", NOWHERE),
            Some("Not installed".to_string())
        );
        assert_eq!(
            adapter_missing(&nobody_else, NOWHERE, "sh"),
            Some("Not installed".to_string())
        );
    }

    #[cfg(unix)]
    fn program(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    fn somebody(name: &str, home: &Path) -> Account {
        Account {
            name: name.to_string(),
            home: home.to_path_buf(),
        }
    }

    fn desk_as(own: &str, others: Vec<Account>) -> Machine {
        Machine {
            own_name: own.to_string(),
            own_home: None,
            others,
            system_bins: Vec::new(),
        }
    }

    #[cfg(unix)]
    const PASSWD: &str = "\
root:x:0:0:root:/root:/bin/bash
daemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin
hotline:x:998:998::/var/lib/hotline:/usr/sbin/nologin
agent:x:1001:1001:Agent:/home/agent:/bin/bash
bob:x:1002:1002:Bob:/home/bob:/bin/zsh
service:x:1003:1003::/srv/service:/usr/sbin/nologin
locked:x:1004:1004::/home/locked:/bin/false
nobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin
a line that is not passwd
broken:x:not-a-number:1:::/bin/sh
";

    /// A person's account is one a login can use; the desk's own is whatever
    /// account has its uid. Unix only, like the lookup: `/home/agent` is not
    /// an absolute path on Windows.
    #[cfg(unix)]
    #[test]
    fn people_are_the_accounts_a_login_could_use() {
        let (own, others) = read_accounts(PASSWD, 998);
        assert_eq!(own.as_deref(), Some("hotline"));
        assert_eq!(
            others,
            [
                somebody("agent", Path::new("/home/agent")),
                somebody("bob", Path::new("/home/bob")),
            ]
        );
        // The desk running as one of them leaves the other, and not itself.
        let (own, others) = read_accounts(PASSWD, 1001);
        assert_eq!(own.as_deref(), Some("agent"));
        assert_eq!(others, [somebody("bob", Path::new("/home/bob"))]);
        assert_eq!(read_accounts("", 5), (None, Vec::new()));
    }

    #[cfg(unix)]
    #[test]
    fn a_cli_installed_for_another_user_says_so_and_what_to_do() {
        let root = scratch("other-user");
        let home = root.join("home/agent");
        program(&home.join(".local/bin/claude"));
        let machine = desk_as("hotline", vec![somebody("agent", &home)]);
        assert_eq!(
            machine.missing("claude"),
            "claude is installed for agent, but the desk runs as hotline. Run the desk as agent (see docs/serve.md), or install claude for hotline."
        );
        // Not everything they have is a CLI the desk was asked about.
        assert_eq!(machine.missing("codex"), "Not installed");
    }

    #[cfg(unix)]
    #[test]
    fn each_place_a_person_installs_a_cli_is_looked_in() {
        let root = scratch("places");
        let home = root.join("home/agent");
        for folder in PER_USER_BINS {
            let command = format!("cli-in-{}", folder.replace('/', "-"));
            program(&home.join(folder).join(&command));
            let machine = desk_as("hotline", vec![somebody("agent", &home)]);
            assert!(
                machine.missing(&command).contains("installed for agent"),
                "{folder}"
            );
        }
        // Somewhere they do not install is not looked in.
        program(&home.join("elsewhere/bin/claude"));
        let machine = desk_as("hotline", vec![somebody("agent", &home)]);
        assert_eq!(machine.missing("claude"), "Not installed");
    }

    #[cfg(unix)]
    #[test]
    fn several_people_are_named_in_order() {
        let root = scratch("several");
        let accounts: Vec<Account> = ["agent", "bob", "carol"]
            .iter()
            .map(|name| {
                let home = root.join("home").join(name);
                program(&home.join(".local/bin/claude"));
                somebody(name, &home)
            })
            .collect();
        let both = desk_as("hotline", accounts[..2].to_vec());
        assert_eq!(
            both.missing("claude"),
            "claude is installed for agent and bob, but the desk runs as hotline. Run the desk as one of them (see docs/serve.md), or install claude for hotline."
        );
        let three = desk_as("hotline", accounts);
        assert!(
            three
                .missing("claude")
                .starts_with("claude is installed for agent, bob and carol, but"),
            "{}",
            three.missing("claude")
        );
    }

    #[cfg(unix)]
    #[test]
    fn only_a_program_counts_as_installed() {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch("not-a-program");
        let home = root.join("home/agent");
        let bin = home.join(".local/bin");
        std::fs::create_dir_all(bin.join("a-folder")).unwrap();
        std::fs::write(bin.join("not-executable"), "text").unwrap();
        std::fs::set_permissions(
            bin.join("not-executable"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        let machine = desk_as("hotline", vec![somebody("agent", &home)]);
        assert_eq!(machine.missing("a-folder"), "Not installed");
        assert_eq!(machine.missing("not-executable"), "Not installed");
    }

    /// The desk cannot tell what is in a home it may not enter, and does not
    /// guess: that is today's sentence, never a claim about somebody else.
    #[cfg(unix)]
    #[test]
    fn a_home_the_desk_may_not_enter_says_nothing() {
        use std::os::unix::fs::PermissionsExt;
        // SAFETY: geteuid takes no arguments and cannot fail.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let root = scratch("private-home");
        let home = root.join("home/agent");
        program(&home.join(".local/bin/claude"));
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o000)).unwrap();
        let machine = desk_as("hotline", vec![somebody("agent", &home)]);
        let said = machine.missing("claude");
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(said, "Not installed");
    }

    #[cfg(unix)]
    #[test]
    fn a_cli_in_the_desks_own_home_or_a_system_folder_is_a_path_problem_not_a_user_one() {
        let root = scratch("off-path");
        let own = root.join("var-lib-hotline");
        program(&own.join(".local/bin/claude"));
        let system = root.join("usr-local-bin");
        program(&system.join("codex"));
        let other_home = root.join("home/agent");
        program(&other_home.join(".local/bin/claude"));
        let machine = Machine {
            own_name: "hotline".to_string(),
            own_home: Some(own.clone()),
            others: vec![somebody("agent", &other_home)],
            system_bins: vec![system.clone()],
        };
        // The desk's own copy is the nearer explanation, and the cheaper fix.
        assert_eq!(
            machine.missing("claude"),
            format!(
                "claude is at {0}/.local/bin/claude, which is not on the desk's PATH. Add {0}/.local/bin to PATH in the service unit (see docs/serve.md).",
                own.display()
            )
        );
        assert_eq!(
            machine.missing("codex"),
            format!(
                "codex is at {0}/codex, which is not on the desk's PATH. Add {0} to PATH in the service unit (see docs/serve.md).",
                system.display()
            )
        );
    }

    #[test]
    fn a_command_nobody_has_is_still_not_installed() {
        assert_eq!(
            desk_as("hotline", Vec::new()).missing("claude"),
            "Not installed"
        );
        assert_eq!(Machine::default().missing("claude"), "Not installed");
    }

    /// The reason is a sentence of its own, so the error that carries it does
    /// not run it on to "it".
    #[test]
    fn a_start_that_cannot_happen_reads_as_a_sentence() {
        assert_eq!(
            cannot_start("Claude Code", "Not installed"),
            "Claude Code cannot start: Not installed."
        );
        assert_eq!(
            cannot_start(
                "Claude Code",
                "claude is installed for agent, but the desk runs as hotline. Run the desk as agent (see docs/serve.md), or install claude for hotline."
            ),
            "Claude Code cannot start: claude is installed for agent, but the desk runs as hotline. Run the desk as agent (see docs/serve.md), or install claude for hotline."
        );
    }
}
