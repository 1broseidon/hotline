//! Updates to the agent harnesses this desk runs.
//!
//! A harness is one thing to the person and up to three to Hotline: the CLI
//! that signs in and does the work, the npm adapter that speaks ACP for it,
//! and a release Hotline fetched from the ACP registry. Each part is checked
//! the way it was installed — read off where its command really lives, never
//! guessed — and the parts fold into one state per harness, which is all the
//! Updates screen shows.
//!
//! - A CLI under an npm prefix (`…/lib/node_modules/<package>`) is checked
//!   with that prefix's own `npm view` and updated with its `npm install -g`.
//!   One under a Homebrew `Cellar` or `Caskroom` is checked with `brew info`
//!   and updated with `brew upgrade`. A mise shim is followed to the real
//!   command first. Anything else is updated by the CLI's own updater when it
//!   has one (`claude update`, `cursor-agent update`, `opencode upgrade`),
//!   and compared with the npm package it is published as when there is one.
//! - An adapter is started by `npx` at the version the registry pins, so it
//!   is behind when today's catalogue pins a newer one than the last start
//!   ran. The next start fetches it whatever happens; an update fetches it
//!   into npm's cache now, so that start does not wait on the download.
//! - A fetched release is behind when the catalogue names one not on disk
//!   while an older one is; an update downloads it ([`registry::install`]).
//!
//! A running session keeps the version it started with. Its teammate gets
//! the new one at its next start, and the screen says so.

use super::registry;
use crate::contract::{HarnessUpdate, HarnessUpdateState, HarnessUpdates};
use async_trait::async_trait;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// How often the harnesses are looked at on their own.
pub(crate) const CHECK_EVERY_MS: i64 = 6 * 60 * 60_000;

/// A version query answers in well under this; a hung one is not waited on.
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// An install can be large and the line slow, but not this slow.
const UPDATE_TIMEOUT: Duration = Duration::from_mins(15);

/// CLIs that update themselves, and the words that do it.
const OWN_UPDATERS: &[(&str, &[&str])] = &[
    ("claude", &["update"]),
    ("cursor-agent", &["update"]),
    ("opencode", &["upgrade"]),
];

/// The npm package a CLI is published as, to read its newest version from
/// when it was installed some other way.
const PUBLISHED_AS: &[(&str, &str)] = &[
    ("claude", "@anthropic-ai/claude-code"),
    ("codex", "@openai/codex"),
    ("gemini", "@google/gemini-cli"),
    ("opencode", "opencode-ai"),
];

/// What a check and an update need of the machine: one seam, so tests run
/// on a scripted machine with no network and no package manager.
#[async_trait]
pub(crate) trait World: Send + Sync {
    /// Where `command` is found on the PATH Hotline launches harnesses with.
    fn which(&self, command: &str) -> Option<PathBuf>;
    /// `path` with every link followed.
    fn real_path(&self, path: &Path) -> PathBuf;
    fn exists(&self, path: &Path) -> bool;
    /// Runs `program` and answers its standard output, or a sentence saying
    /// why it failed.
    async fn run(
        &self,
        program: &Path,
        args: &[String],
        timeout: Duration,
    ) -> Result<String, String>;
    async fn refresh_catalogue(&self, root: &Path);
    async fn fetch_release(&self, root: &Path, backend_id: &str) -> Result<(), String>;
}

/// This machine.
pub(crate) struct System;

#[async_trait]
impl World for System {
    fn which(&self, command: &str) -> Option<PathBuf> {
        registry::which(command)
    }

    fn real_path(&self, path: &Path) -> PathBuf {
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    async fn run(
        &self,
        program: &Path,
        args: &[String],
        timeout: Duration,
    ) -> Result<String, String> {
        let child = tokio::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            // A run that times out is dropped, and dropping it ends the one
            // process it started, by that process's handle.
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| format!("{} could not start: {error}", shown(program)))?;
        let output = tokio::time::timeout(timeout, child.wait_with_output())
            .await
            .map_err(|_| format!("{} did not finish in time", shown(program)))?
            .map_err(|error| error.to_string())?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        let said = String::from_utf8_lossy(&output.stderr);
        let last = said.lines().rev().find(|line| !line.trim().is_empty());
        Err(match last {
            Some(line) => line.trim().to_string(),
            None => format!("{} exited with {}", shown(program), output.status),
        })
    }

    async fn refresh_catalogue(&self, root: &Path) {
        registry::refresh_now(root).await;
    }

    async fn fetch_release(&self, root: &Path, backend_id: &str) -> Result<(), String> {
        registry::install(root, backend_id).await
    }
}

fn shown(program: &Path) -> String {
    program.file_name().map_or_else(
        || program.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

// -- what a harness is made of ----------------------------------------------

/// One harness, as a check sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Harness {
    pub id: String,
    pub name: String,
    /// The CLI it is installed as, for one Hotline knows by name.
    pub cli: Option<String>,
    /// The package `npx` or `uvx` starts it, or its adapter, as.
    pub package: Option<Package>,
    /// A release fetched from the registry.
    pub release: Option<Release>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Package {
    /// `name@version`, as the launch names it.
    pub spec: String,
    /// The command that starts it: `npx` or `uvx`, as found.
    pub runner: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Release {
    /// The command inside the release the catalogue names today.
    pub command: PathBuf,
    /// The folder that release unpacks into; older releases sit beside it.
    pub dir: PathBuf,
}

/// The harnesses this desk runs: the ones Hotline knows by name that are
/// installed here, and any other a teammate is set to.
pub(crate) fn harnesses(root: &Path, used: &[String]) -> Vec<Harness> {
    registry::cached_backends(root)
        .into_iter()
        .filter(|backend| backend.unavailable.is_none())
        .filter(|backend| registry::named(&backend.id) || used.contains(&backend.id))
        .map(|backend| {
            let package = backend.launch.as_ref().and_then(package_of);
            let release =
                backend
                    .archive
                    .as_ref()
                    .zip(backend.launch.as_ref())
                    .map(|(archive, launch)| Release {
                        command: PathBuf::from(&launch.command),
                        dir: archive.dir.clone(),
                    });
            Harness {
                cli: registry::cli_of(&backend.id).map(str::to_string),
                id: backend.id,
                name: backend.name,
                package,
                release,
            }
        })
        .collect()
}

/// The package a launch runs through `npx` or `uvx`, when it does.
fn package_of(launch: &registry::Launch) -> Option<Package> {
    let runner = Path::new(&launch.command).file_stem()?.to_str()?;
    if runner != "npx" && runner != "uvx" {
        return None;
    }
    let spec = launch.args.iter().find(|arg| !arg.starts_with('-'))?;
    Some(Package {
        spec: spec.clone(),
        runner: launch.command.clone(),
    })
}

/// `@scope/name@1.2.3` as its name and version; a bare name has none.
fn split_spec(spec: &str) -> (&str, Option<&str>) {
    let at = spec
        .char_indices()
        .skip(1)
        .find(|(_, c)| *c == '@')
        .map(|(i, _)| i);
    match at {
        Some(at) => (&spec[..at], Some(&spec[at + 1..])),
        None => (spec, None),
    }
}

// -- versions ---------------------------------------------------------------

/// The first version in what a command said: `2.1.3 (Claude Code)`,
/// `codex-cli 0.5.0`, `v1.4.0`.
pub(crate) fn version_in(said: &str) -> Option<String> {
    said.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | ',' | '"'))
        .map(|word| word.trim_start_matches('v'))
        .find(|word| numbers(word).is_some_and(|parts| parts.len() >= 2))
        .map(str::to_string)
}

/// A version's numbers, up to any pre-release or build tag.
fn numbers(version: &str) -> Option<Vec<u64>> {
    let core = version.split(['-', '+']).next()?;
    core.split('.').map(|part| part.parse().ok()).collect()
}

/// Whether `latest` is a newer version than `installed`.
pub(crate) fn newer(latest: &str, installed: &str) -> bool {
    match (numbers(latest), numbers(installed)) {
        (Some(latest), Some(installed)) => latest > installed,
        _ => false,
    }
}

// -- parts --------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Cli,
    Adapter,
    Release,
}

/// One part of a harness, as last checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Part {
    pub kind: Kind,
    pub installed: Option<String>,
    pub latest: Option<String>,
    /// A newer version is published than the one here.
    pub behind: bool,
    /// What brings it up to date, when Hotline can.
    pub step: Option<Step>,
}

impl Part {
    /// Whether this part's standing is known: both versions read, or a
    /// release, which is known by being on disk or not.
    fn known(&self) -> bool {
        self.kind == Kind::Release || (self.installed.is_some() && self.latest.is_some())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Run { program: PathBuf, args: Vec<String> },
    FetchRelease,
}

/// How a CLI was installed, read off where it really lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Method {
    Npm {
        npm: PathBuf,
        package: String,
    },
    Brew {
        brew: PathBuf,
        name: String,
        cask: bool,
    },
    Own {
        command: PathBuf,
        args: Vec<String>,
    },
    Unknown,
}

/// The install method of the CLI found at `found`, whose links lead to
/// `real`.
pub(crate) fn method_for(world: &dyn World, command: &str, found: &Path, real: &Path) -> Method {
    let parts: Vec<String> = real
        .components()
        .filter_map(|part| match part {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    let base = |upto: usize| -> PathBuf {
        let mut path: PathBuf = real
            .ancestors()
            .last()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        for part in &parts[..upto] {
            path.push(part);
        }
        path
    };
    if let Some(at) = parts.iter().position(|part| part == "node_modules")
        && let Some(first) = parts.get(at + 1)
    {
        let package = if first.starts_with('@') {
            parts.get(at + 2).map(|second| format!("{first}/{second}"))
        } else {
            Some(first.clone())
        };
        // `<prefix>/lib/node_modules` with npm in `<prefix>/bin`; on Windows
        // `<prefix>/node_modules` with `npm.cmd` beside it.
        let prefix = if at > 0 && parts[at - 1] == "lib" {
            base(at - 1)
        } else {
            base(at)
        };
        let npm = [prefix.join("bin").join("npm"), prefix.join("npm.cmd")]
            .into_iter()
            .find(|npm| world.exists(npm));
        if let (Some(package), Some(npm)) = (package, npm) {
            return Method::Npm { npm, package };
        }
    }
    for (folder, cask) in [("Cellar", false), ("Caskroom", true)] {
        if let Some(at) = parts.iter().position(|part| part == folder)
            && let Some(name) = parts.get(at + 1)
        {
            let brew = base(at).join("bin").join("brew");
            if world.exists(&brew) {
                return Method::Brew {
                    brew,
                    name: name.clone(),
                    cask,
                };
            }
        }
    }
    OWN_UPDATERS
        .iter()
        .find(|(name, _)| *name == command)
        .map_or(Method::Unknown, |(_, args)| Method::Own {
            command: found.to_path_buf(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
        })
}

/// Whether `path` is a mise shim, which is not the command but mise itself.
fn is_shim(found: &Path, real: &Path) -> bool {
    found.components().any(|part| part.as_os_str() == "shims")
        && found.to_string_lossy().contains("mise")
        || real.file_stem().is_some_and(|stem| stem == "mise")
}

fn args(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_string()).collect()
}

/// The CLI part of a harness.
pub(crate) async fn cli_part(world: &dyn World, command: &str) -> Part {
    let mut part = Part {
        kind: Kind::Cli,
        installed: None,
        latest: None,
        behind: false,
        step: None,
    };
    let Some(found) = world.which(command) else {
        return part;
    };
    let mut real = world.real_path(&found);
    if is_shim(&found, &real)
        && let Some(mise) = world.which("mise")
        && let Ok(said) = world
            .run(&mise, &args(&["which", command]), CHECK_TIMEOUT)
            .await
    {
        real = world.real_path(Path::new(said.trim()));
    }
    part.installed = world
        .run(&found, &args(&["--version"]), CHECK_TIMEOUT)
        .await
        .ok()
        .and_then(|said| version_in(&said));
    let method = method_for(world, command, &found, &real);
    part.latest = match &method {
        Method::Npm { npm, package } => npm_latest(world, npm, package).await,
        Method::Brew { brew, name, cask } => {
            let mut words = vec!["info".to_string(), "--json=v2".to_string()];
            if *cask {
                words.push("--cask".to_string());
            }
            words.push(name.clone());
            world
                .run(brew, &words, CHECK_TIMEOUT)
                .await
                .ok()
                .and_then(|said| brew_version(&said, *cask))
        }
        Method::Own { .. } | Method::Unknown => match (
            PUBLISHED_AS.iter().find(|(name, _)| *name == command),
            world.which("npm"),
        ) {
            (Some((_, package)), Some(npm)) => npm_latest(world, &npm, package).await,
            _ => None,
        },
    };
    part.behind = matches!((&part.latest, &part.installed), (Some(latest), Some(installed)) if newer(latest, installed));
    part.step = match method {
        Method::Npm { npm, package } => Some(Step::Run {
            program: npm,
            args: vec!["install".into(), "-g".into(), format!("{package}@latest")],
        }),
        Method::Brew { brew, name, cask } => {
            let mut words = vec!["upgrade".to_string()];
            if cask {
                words.push("--cask".to_string());
            }
            words.push(name);
            Some(Step::Run {
                program: brew,
                args: words,
            })
        }
        Method::Own { command, args } => Some(Step::Run {
            program: command,
            args,
        }),
        Method::Unknown => None,
    };
    part
}

async fn npm_latest(world: &dyn World, npm: &Path, package: &str) -> Option<String> {
    world
        .run(npm, &args(&["view", package, "version"]), CHECK_TIMEOUT)
        .await
        .ok()
        .and_then(|said| version_in(&said))
}

/// The newest version `brew info --json=v2` says a formula or cask has.
fn brew_version(said: &str, cask: bool) -> Option<String> {
    let info: serde_json::Value = serde_json::from_str(said).ok()?;
    let version = if cask {
        info["casks"][0]["version"].as_str()
    } else {
        info["formulae"][0]["versions"]["stable"].as_str()
    }?;
    version_in(version).or_else(|| Some(version.to_string()))
}

/// The adapter or runner package part: the version the last start ran
/// against the version today's catalogue pins.
pub(crate) fn package_part(world: &dyn World, root: &Path, package: &Package) -> Part {
    let (name, pinned) = split_spec(&package.spec);
    let ran = registry::started_lines(root).iter().rev().find_map(|line| {
        line.split_whitespace()
            .find_map(|word| match split_spec(word) {
                (word, Some(version)) if word == name => Some(version.to_string()),
                _ => None,
            })
    });
    let latest = pinned.map(str::to_string);
    let installed = ran.or_else(|| latest.clone());
    let behind = matches!((&latest, &installed), (Some(latest), Some(installed)) if newer(latest, installed));
    // npm's cache is what `npx` fetches into; uvx fetches on its own at the
    // next start, and needs nothing now.
    let npx = Path::new(&package.runner)
        .file_stem()
        .is_some_and(|stem| stem == "npx");
    let npm = npx
        .then(|| {
            world
                .which(&package.runner)
                .and_then(|npx| npx.parent().map(|dir| dir.join("npm")))
                .filter(|npm| world.exists(npm))
                .or_else(|| world.which("npm"))
        })
        .flatten();
    Part {
        kind: Kind::Adapter,
        installed,
        latest,
        behind,
        step: npm.filter(|_| behind).map(|npm| Step::Run {
            program: npm,
            args: vec!["cache".into(), "add".into(), package.spec.clone()],
        }),
    }
}

/// The fetched-release part: behind when today's release is not on disk
/// and an older one is.
pub(crate) fn release_part(world: &dyn World, release: &Release) -> Part {
    let fetched = world.exists(&release.command);
    let older = release.dir.parent().is_some_and(|releases| {
        std::fs::read_dir(releases)
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| entry.path() != release.dir && entry.path().is_dir())
    });
    let behind = !fetched && older;
    Part {
        kind: Kind::Release,
        installed: None,
        latest: None,
        behind,
        step: behind.then_some(Step::FetchRelease),
    }
}

/// Every part of `harness`, checked.
pub(crate) async fn parts_of(world: &dyn World, root: &Path, harness: &Harness) -> Vec<Part> {
    let mut parts = Vec::new();
    if let Some(cli) = &harness.cli {
        parts.push(cli_part(world, cli).await);
    }
    if let Some(package) = &harness.package {
        parts.push(package_part(world, root, package));
    }
    if let Some(release) = &harness.release {
        parts.push(release_part(world, release));
    }
    parts
}

// -- folding ------------------------------------------------------------------

/// What the screen shows of a harness's parts: one state, the version it is
/// known by, the newer version of that same part, and whether Update runs
/// anything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Folded {
    pub state: HarnessUpdateState,
    pub version: Option<String>,
    pub latest: Option<String>,
    pub can_update: bool,
}

pub(crate) fn fold(parts: &[Part]) -> Folded {
    let shown = parts
        .iter()
        .find(|part| part.kind == Kind::Cli && part.installed.is_some())
        .or_else(|| parts.iter().find(|part| part.installed.is_some()));
    let version = shown.and_then(|part| part.installed.clone());
    let latest = shown
        .filter(|part| part.behind)
        .and_then(|part| part.latest.clone());
    let behind = parts.iter().any(|part| part.behind);
    let state = if behind {
        HarnessUpdateState::Available
    } else if parts.iter().all(Part::known) {
        HarnessUpdateState::UpToDate
    } else {
        HarnessUpdateState::Unknown
    };
    // A part that is behind is updated by its step, or by the next start
    // when it is an adapter `uvx` fetches. A part whose standing is unknown
    // can still run a CLI's own updater, which knows.
    let can_update = match state {
        HarnessUpdateState::Available => true,
        HarnessUpdateState::Unknown => parts
            .iter()
            .any(|part| !part.known() && part.step.is_some()),
        _ => false,
    };
    Folded {
        state,
        version,
        latest,
        can_update,
    }
}

// -- the harnesses' state -----------------------------------------------------

/// Where one harness stands: one state, moved only by [`Updates`].
#[derive(Clone, Debug, PartialEq, Eq)]
enum Entry {
    Checking,
    Checked(Vec<Part>),
    Updating(Vec<Part>),
    Updated { parts: Vec<Part>, note: String },
    Failed { parts: Vec<Part>, reason: String },
}

impl Entry {
    fn parts(&self) -> &[Part] {
        match self {
            Entry::Checking => &[],
            Entry::Checked(parts) | Entry::Updating(parts) => parts,
            Entry::Updated { parts, .. } | Entry::Failed { parts, .. } => parts,
        }
    }

    fn busy(&self) -> bool {
        matches!(self, Entry::Checking | Entry::Updating(_))
    }
}

#[derive(Default)]
struct State {
    harnesses: Vec<(Harness, Entry)>,
    checked_at: Option<i64>,
    checking: bool,
}

/// The desk's harnesses and where each stands on updates.
pub(crate) struct Updates {
    world: Mutex<Arc<dyn World>>,
    state: Mutex<State>,
}

impl Default for Updates {
    fn default() -> Self {
        Self::new(Arc::new(System))
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Updates {
    pub(crate) fn new(world: Arc<dyn World>) -> Self {
        Self {
            world: Mutex::new(world),
            state: Mutex::default(),
        }
    }

    /// Checks and updates on a scripted machine from now on.
    #[cfg(test)]
    pub(crate) fn use_world(&self, world: Arc<dyn World>) {
        *lock(&self.world) = world;
    }

    fn world(&self) -> Arc<dyn World> {
        lock(&self.world).clone()
    }

    /// Where every harness stands, as last checked.
    pub(crate) fn view(&self) -> HarnessUpdates {
        let state = lock(&self.state);
        HarnessUpdates {
            harnesses: state
                .harnesses
                .iter()
                .map(|(harness, entry)| row(harness, entry))
                .collect(),
            checked_at: state.checked_at,
        }
    }

    /// Whether the last check is old enough, or missing, to look again.
    pub(crate) fn due(&self, now: i64) -> bool {
        let state = lock(&self.state);
        !state.checking
            && state
                .checked_at
                .is_none_or(|at| now.saturating_sub(at) >= CHECK_EVERY_MS)
    }

    /// Looks at every harness now. A check already under way is not run
    /// twice: this answers what is known. A harness mid-update is left to
    /// its update.
    pub(crate) async fn check(&self, root: &Path, used: &[String], now: i64) -> HarnessUpdates {
        {
            let mut state = lock(&self.state);
            if state.checking {
                drop(state);
                return self.view();
            }
            state.checking = true;
        }
        let world = self.world();
        world.refresh_catalogue(root).await;
        let found = harnesses(root, used);
        {
            let mut state = lock(&self.state);
            let mut next = Vec::new();
            for harness in found {
                let entry = match state
                    .harnesses
                    .iter()
                    .find(|(known, _)| known.id == harness.id)
                {
                    Some((_, entry @ Entry::Updating(_))) => entry.clone(),
                    _ => Entry::Checking,
                };
                next.push((harness, entry));
            }
            state.harnesses = next;
        }
        let to_check: Vec<Harness> = lock(&self.state)
            .harnesses
            .iter()
            .filter(|(_, entry)| *entry == Entry::Checking)
            .map(|(harness, _)| harness.clone())
            .collect();
        for harness in to_check {
            let parts = parts_of(world.as_ref(), root, &harness).await;
            let mut state = lock(&self.state);
            if let Some((_, entry)) = state
                .harnesses
                .iter_mut()
                .find(|(known, entry)| known.id == harness.id && *entry == Entry::Checking)
            {
                *entry = Entry::Checked(parts);
            }
        }
        {
            let mut state = lock(&self.state);
            state.checking = false;
            state.checked_at = Some(now);
        }
        self.view()
    }

    /// Brings every part of one harness that is behind up to date, then
    /// checks it again. Refused while it is being checked or updated, and
    /// for a harness with nothing to run.
    pub(crate) async fn update(
        &self,
        root: &Path,
        backend_id: &str,
    ) -> Result<HarnessUpdate, String> {
        let (harness, steps) = {
            let mut state = lock(&self.state);
            let Some((harness, entry)) = state
                .harnesses
                .iter_mut()
                .find(|(harness, _)| harness.id == backend_id)
            else {
                return Err("That harness is not one this desk runs. Check again.".into());
            };
            if entry.busy() {
                return Err(format!(
                    "{} is already being {}.",
                    harness.name,
                    if *entry == Entry::Checking {
                        "checked"
                    } else {
                        "updated"
                    }
                ));
            }
            let parts = entry.parts().to_vec();
            if !fold(&parts).can_update {
                return Err(format!("{} has nothing to update.", harness.name));
            }
            let steps: Vec<Step> = parts
                .iter()
                .filter(|part| part.behind || !part.known())
                .filter_map(|part| part.step.clone())
                .collect();
            *entry = Entry::Updating(parts);
            (harness.clone(), steps)
        };
        let world = self.world();
        let mut failure = None;
        for step in steps {
            let ran = match &step {
                Step::Run { program, args } => {
                    world.run(program, args, UPDATE_TIMEOUT).await.map(|_| ())
                }
                Step::FetchRelease => world.fetch_release(root, &harness.id).await,
            };
            if let Err(error) = ran {
                failure = Some(error);
                break;
            }
        }
        let parts = parts_of(world.as_ref(), root, &harness).await;
        let entry = if let Some(error) = failure {
            Entry::Failed {
                parts,
                reason: format!(
                    "Updating {} failed: {}.",
                    harness.name,
                    error.trim_end_matches('.')
                ),
            }
        } else {
            let note = match fold(&parts).version {
                Some(version) => format!(
                    "{} is {version}. A teammate on it gets it at its next start.",
                    harness.name
                ),
                None => format!(
                    "{} is updated. A teammate on it gets it at its next start.",
                    harness.name
                ),
            };
            Entry::Updated { parts, note }
        };
        let mut state = lock(&self.state);
        let Some((harness, slot)) = state
            .harnesses
            .iter_mut()
            .find(|(harness, _)| harness.id == backend_id)
        else {
            return Err("That harness is not one this desk runs any more.".into());
        };
        *slot = entry;
        Ok(row(harness, slot))
    }
}

/// One harness as the screen shows it.
fn row(harness: &Harness, entry: &Entry) -> HarnessUpdate {
    let folded = fold(entry.parts());
    let (state, detail) = match entry {
        Entry::Checking => (HarnessUpdateState::Checking, None),
        Entry::Checked(_) => (folded.state, None),
        Entry::Updating(_) => (HarnessUpdateState::Updating, None),
        Entry::Updated { note, .. } => (HarnessUpdateState::Updated, Some(note.clone())),
        Entry::Failed { reason, .. } => (HarnessUpdateState::Failed, Some(reason.clone())),
    };
    let can_update = match entry {
        Entry::Checked(_) | Entry::Failed { .. } | Entry::Updated { .. } => folded.can_update,
        Entry::Checking | Entry::Updating(_) => false,
    };
    HarnessUpdate {
        backend_id: harness.id.clone(),
        name: harness.name.clone(),
        state,
        version: folded.version,
        latest: folded.latest,
        can_update,
        detail,
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use tests::Scripted;

#[cfg(test)]
pub(crate) fn harness_for_tests(id: &str, cli: Option<&str>, package: Option<&str>) -> Harness {
    Harness {
        id: id.into(),
        name: id.into(),
        cli: cli.map(str::to_string),
        package: package.map(|spec| Package {
            spec: spec.into(),
            runner: "npx".into(),
        }),
        release: None,
    }
}

#[cfg(test)]
impl Updates {
    /// Sets the harnesses a test's check looks at, standing for the
    /// registry's listing.
    pub(crate) fn seed(&self, harnesses: Vec<Harness>) {
        lock(&self.state).harnesses = harnesses
            .into_iter()
            .map(|harness| (harness, Entry::Checking))
            .collect();
    }

    /// Checks the seeded harnesses, without the registry's listing.
    pub(crate) async fn check_seeded(&self, root: &Path, now: i64) -> HarnessUpdates {
        let world = self.world();
        let seeded: Vec<Harness> = lock(&self.state)
            .harnesses
            .iter()
            .map(|(harness, _)| harness.clone())
            .collect();
        for harness in seeded {
            let parts = parts_of(world.as_ref(), root, &harness).await;
            let mut state = lock(&self.state);
            if let Some((_, entry)) = state
                .harnesses
                .iter_mut()
                .find(|(known, _)| known.id == harness.id)
            {
                *entry = Entry::Checked(parts);
            }
        }
        lock(&self.state).checked_at = Some(now);
        self.view()
    }
}
