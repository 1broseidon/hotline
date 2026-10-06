//! Which external agents Hotline can start on this machine, and how.
//!
//! The list is data, not code. It comes from three places, in this order:
//! a table of agents Hotline has been taught by hand, the ACP registry's
//! published catalogue (fetched once a day and cached, so a session start
//! never waits on the network and an offline machine still gets the table),
//! and a probe of what is actually installed. A locally installed binary
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
        description: "Cursor's coding agent. Uses your existing Cursor login.",
        command: "cursor-agent",
        args: &["acp"],
    },
    Native {
        id: "grok-build",
        name: "Grok Build",
        description: "xAI's coding agent. Uses your Grok login.",
        command: "grok",
        args: &["agent", "stdio"],
    },
    Native {
        id: "opencode",
        name: "opencode",
        description: "Open-source agent with resume and fork support.",
        command: "opencode",
        args: &["acp"],
    },
    Native {
        id: "gemini",
        name: "Gemini CLI",
        description: "Google's coding agent.",
        command: "gemini",
        args: &["--acp"],
    },
];

const ADAPTED: &[Adapted] = &[
    Adapted {
        id: "claude-acp",
        name: "Claude Code",
        description: "Anthropic's coding agent. Uses your Claude Code login or ANTHROPIC_API_KEY.",
        package: "@agentclientprotocol/claude-agent-acp@0.78.0",
        client: "claude",
    },
    Adapted {
        id: "codex-acp",
        name: "Codex",
        description: "OpenAI's coding agent. Uses your Codex login or OPENAI_API_KEY.",
        package: "@agentclientprotocol/codex-acp@1.12.0",
        client: "codex",
    },
];

/// How long Hotline leaves the catalogue alone after trying for it, whether
/// or not it got it. Without this a machine that is offline pays the whole
/// fetch timeout on every list the picker or the welcome asks for.
const RETRY_AFTER: Duration = Duration::from_secs(10 * 60);

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
    let stale = read_catalogue(root).is_none_or(|catalogue| !fresh(&catalogue, now_ms()));
    if stale && due_for_a_try(root) {
        let root = root.to_path_buf();
        tokio::spawn(async move { refresh_catalogue(&root).await });
    }
    cached_backends(root)
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
    let arch = match std::env::consts::ARCH {
        arch @ ("x86_64" | "aarch64") => arch,
        _ => return None,
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
/// when it cannot. A failure here costs the agents Hotline was not taught by
/// hand, and never the ones it was.
async fn refresh_catalogue(root: &Path) {
    let cached = read_catalogue(root);
    if cached.is_some_and(|catalogue| fresh(&catalogue, now_ms())) {
        return;
    }
    let fetched = reqwest::Client::new()
        .get(REGISTRY_URL)
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
    let path = paths::acp_registry_path(root);
    if let Some(directory) = path.parent()
        && std::fs::create_dir_all(directory).is_ok()
        && let Ok(bytes) = serde_json::to_vec(&catalogue)
    {
        let _ = std::fs::write(path, bytes);
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
                    .map(|a| a.to_string())
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
