//! Which external agents Toad can start on this machine, and how.
//!
//! The list is data, not code. It comes from three places, in this order:
//! a table of agents Toad has been taught by hand, the ACP registry's
//! published catalogue (fetched once a day and cached, so a session start
//! never waits on the network and an offline machine still gets the table),
//! and a probe of what is actually installed. A locally installed binary
//! always wins over a downloadable one, because the local copy carries the
//! user's login.
//!
//! Toad Agent is deliberately absent. This module answers "which child
//! process, started how", and the built-in agent has no child process, no
//! binary to find and nothing to download.

use crate::paths;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The registry's published catalogue: one request for every agent, carrying
/// names, descriptions and launch commands. Preferred over walking the GitHub
/// repository, which costs a request per agent out of an unauthenticated
/// budget shared with everything else on the machine.
const REGISTRY_URL: &str = "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";

/// How long a fetched catalogue is used before Toad asks again.
const CACHE_TTL_MS: i64 = 24 * 60 * 60 * 1000;

/// How long the catalogue fetch may take before it is given up on. The cached
/// copy — or, failing that, the table below — is the answer either way.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// How a backend is started: a command and its arguments, as spawned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub command: String,
    pub args: Vec<String>,
}

/// One agent a teammate can be run on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Backend {
    pub id: String,
    pub name: String,
    pub description: String,
    /// How to start it, when Toad knows a way. `None` is an agent the
    /// catalogue lists but distributes as a prebuilt archive, which Toad does
    /// not download.
    pub launch: Option<Launch>,
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
        package: "@agentclientprotocol/claude-agent-acp@0.69.0",
        client: "claude",
    },
    Adapted {
        id: "codex-acp",
        name: "Codex",
        description: "OpenAI's coding agent. Uses your Codex login or OPENAI_API_KEY.",
        package: "@agentclientprotocol/codex-acp@1.4.0",
        client: "codex",
    },
];

/// Every agent Toad can offer, catalogue included.
///
/// This is the listing a picker draws. It refreshes the cached catalogue
/// first, so opening the picker is where the day's fetch happens rather than
/// somewhere a person is waiting on an agent to start.
pub async fn backends(root: &Path) -> Vec<Backend> {
    refresh_catalogue(root).await;
    cached_backends(root)
}

/// The same listing without touching the network, which is what starting a
/// session reads.
pub fn cached_backends(root: &Path) -> Vec<Backend> {
    let catalogue = read_catalogue(root).unwrap_or_default();
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
            }),
            unavailable: found.is_none().then(|| "Not installed".to_string()),
        });
    }

    for adapted in ADAPTED {
        let published = catalogue
            .agents
            .iter()
            .find(|agent| agent.id == adapted.id)
            .and_then(launch_for);
        let launch = published.unwrap_or_else(|| npx(adapted.package, &[]));
        backends.push(Backend {
            id: adapted.id.to_string(),
            name: adapted.name.to_string(),
            description: adapted.description.to_string(),
            unavailable: adapter_missing(adapted.client, &launch.command),
            launch: Some(launch),
        });
    }

    let mut listed: Vec<Backend> = catalogue
        .agents
        .iter()
        .filter(|agent| !backends.iter().any(|known| known.id == agent.id))
        .map(|agent| {
            let launch = launch_for(agent);
            let unavailable = match &launch {
                None => Some(
                    "distributed as a prebuilt binary, which Toad cannot install yet".to_string(),
                ),
                Some(launch) => which(&launch.command)
                    .is_none()
                    .then(|| "Not installed".to_string()),
            };
            Backend {
                id: agent.id.clone(),
                name: agent.name.clone().unwrap_or_else(|| agent.id.clone()),
                description: agent.description.clone().unwrap_or_default(),
                launch,
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
        (Some(reason), _) => Err(format!("{} cannot start: it {reason}.", backend.name)),
        (None, None) => Err(format!("{} has no launch command.", backend.name)),
        (None, Some(launch)) => Ok(launch),
    }
}

// -- the published catalogue -----------------------------------------------

/// One agent as the ACP registry publishes it. Only the fields Toad reads are
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
    fetched_at: i64,
    #[serde(default)]
    agents: Vec<Published>,
}

/// How a catalogue entry is started, or `None` when Toad cannot start it.
///
/// `npx` and `uvx` both fetch on demand, so they need no install step. A
/// binary distribution is an archive to download, verify and unpack, which
/// Toad does not do — those are reported unavailable rather than offered and
/// then failing at the moment somebody tries to use them.
fn launch_for(agent: &Published) -> Option<Launch> {
    let distribution = agent.distribution.as_ref()?;
    if let Some(runner) = &distribution.npx {
        return Some(npx(&runner.package, &runner.args));
    }
    let runner = distribution.uvx.as_ref()?;
    let mut args = vec![runner.package.clone()];
    args.extend(runner.args.iter().cloned());
    Some(Launch {
        command: "uvx".to_string(),
        args,
    })
}

/// What a hand-taught adapter row is missing, or nothing when it can start.
///
/// Two things have to be here: the harness's own CLI, which is what signs
/// itself in, and whatever starts its ACP adapter — usually `npx`, which is a
/// different program and may well not be installed. A row that probed only
/// the first offered a start that fails at the spawn, with `unavailable`
/// saying nothing.
fn adapter_missing(client: &str, launcher: &str) -> Option<String> {
    if which(client).is_none() {
        return Some("Not installed".to_string());
    }
    which(launcher)
        .is_none()
        .then(|| "Not installed".to_string())
}

fn npx(package: &str, extra: &[String]) -> Launch {
    let mut args = vec!["-y".to_string(), package.to_string()];
    args.extend(extra.iter().cloned());
    Launch {
        command: "npx".to_string(),
        args,
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

fn read_catalogue(root: &Path) -> Option<Catalogue> {
    let bytes = std::fs::read(paths::acp_registry_path(root)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Fetches the catalogue when the cached copy is a day old, and says nothing
/// when it cannot. A failure here costs the agents Toad was not taught by
/// hand, and never the ones it was.
async fn refresh_catalogue(root: &Path) {
    let cached = read_catalogue(root);
    if cached.is_some_and(|catalogue| still_the_days(catalogue.fetched_at, now_ms())) {
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

// -- PATH -------------------------------------------------------------------

/// The absolute path of a command on PATH, so spawning does not depend on
/// what PATH the child happens to inherit.
///
/// A name with a separator in it is a path already and is answered as itself
/// when it exists. On Windows a bare name is tried with each `PATHEXT`
/// suffix, which is where `npx` actually lives there.
fn which(command: &str) -> Option<PathBuf> {
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
            "toad-core-registry-{name}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
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

    /// The table Toad was taught is answered with no catalogue and no network,
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

    /// A catalogue entry Toad was not taught is listed, and the version it
    /// publishes for an adapter Toad WAS taught replaces the pinned fallback.
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

        // Nothing to run and nothing to fetch is not an agent Toad can offer.
        let archived = backends.iter().find(|b| b.id == "archived").unwrap();
        assert_eq!(archived.launch, None);
        assert!(archived.unavailable.is_some());
        assert!(launch(&root, "archived").is_err());
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

    /// An adapter is two programs: the harness's own CLI and whatever starts
    /// its ACP adapter. A row that probed only the first was offered as
    /// startable and failed at the spawn.
    #[test]
    fn an_adapter_needs_both_its_harness_and_whatever_starts_it() {
        const NOWHERE: &str = "this-command-does-not-exist-anywhere";
        if cfg!(windows) {
            return;
        }
        assert_eq!(adapter_missing("sh", "sh"), None);
        assert_eq!(
            adapter_missing("sh", NOWHERE),
            Some("Not installed".to_string())
        );
        assert_eq!(
            adapter_missing(NOWHERE, "sh"),
            Some("Not installed".to_string())
        );
    }
}
