//! One Linux desktop per teammate, as a container Toad starts and stops.
//!
//! The image is a contract, not a binary: anything that serves streamable
//! HTTP MCP at `http://127.0.0.1:<port>/mcp` with bearer `TOAD_COMPUTER_TOKEN`,
//! `/health` open, and the viewer page at `/` on the same port is a valid
//! computer. This module is the Toad-side lifecycle — wake, idle stop,
//! hibernate — and the grant a session is handed. The agent and the image
//! are toad.computer, a repository of their own.

pub mod runtime;

use crate::contract::{ComputerState, ComputerStatus, Persona, RuntimeReport, RuntimeState};
use crate::mcp::{HttpAuth, McpServer, McpTransport};
use runtime::{BinSearch, Runtime};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tokio::process::Command;

/// The toad.computer release this desktop is built against. Bumped here
/// deliberately when the desktop is ready for a new image — never derived
/// from the desktop version, and never `latest`.
pub const COMPUTER_VERSION: &str = "0.3.0";

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
const NIX_VOLUME: &str = "toad-nix";
const DEFAULT_MEMORY: &str = "2g";
const DEFAULT_PIDS: u32 = 512;
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

pub fn default_image() -> String {
    format!("ghcr.io/1broseidon/toad-computer:{COMPUTER_VERSION}")
}

pub fn container_name(persona_id: &str) -> String {
    format!("toad-computer-{persona_id}")
}

/// The MCP server a session is handed for this computer. Not part of
/// `mcpPolicy`: the computer is a per-teammate capability Toad manages.
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

/// Toad names the computer's tools `{slug}__{remote}` from the server's
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
/// already exists from a previous run of Toad has a token we no longer know
/// — it was process state, never a setting — so it is removed and recreated
/// rather than started with a bearer nobody can present.
#[derive(Clone)]
pub struct Computer {
    inner: Arc<Mutex<Inner>>,
    bins: BinSearch,
}

struct Inner {
    containers: HashMap<String, Live>,
}

#[derive(Clone)]
struct Live {
    runtime: Runtime,
    cmd: PathBuf,
    token: String,
    mcp_port: u16,
    last_activity_ms: i64,
}

impl Computer {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                containers: HashMap::new(),
            })),
            bins: BinSearch::from_env(),
        }
    }

    #[cfg(test)]
    pub fn with_path(path: impl Into<std::ffi::OsString>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                containers: HashMap::new(),
            })),
            bins: BinSearch::only(path),
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
        let image = image_of(persona, room_image);
        let cwd = abs_cwd(workspace_cwd);
        let known_token = self
            .lock()
            .containers
            .get(&persona.id)
            .map(|live| live.token.clone());

        let mut inspection = inspect(&cmd, runtime, &name).await?;
        if inspection.exists && known_token.is_none() {
            // A container that exists without a token in this process came
            // from a previous run of Toad: the bearer was never written
            // down, so starting it would hand the agent a machine it cannot
            // authenticate to. Remove and recreate.
            run(&cmd, &["rm", "-f", &name], COMMAND_TIMEOUT).await?;
            inspection.exists = false;
            inspection.running = false;
        }

        let token = known_token.clone().unwrap_or_else(new_token);
        if !inspection.exists {
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
            },
        );
        wait_healthy(mcp_port).await?;
        Ok(Ready {
            url: mcp_url(mcp_port),
            token,
        })
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
                    });
                }
            };
            return Ok(status_of(inspection, Some((live.mcp_port, &live.token))));
        }
        let Ok((runtime, cmd)) = pick_runtime(prefer, &self.bins).await else {
            return Ok(ComputerStatus {
                state: ComputerState::Absent,
                url: None,
                viewer: None,
            });
        };
        let inspection = match inspect(&cmd, runtime, &name).await {
            Ok(seen) => seen,
            Err(_) => {
                return Ok(ComputerStatus {
                    state: ComputerState::Absent,
                    url: None,
                    viewer: None,
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
}

/// The viewer needs the bearer, which only this process knows, so a
/// container this process did not wake reports a URL and no viewer.
fn status_of(inspection: Inspection, known: Option<(u16, &str)>) -> ComputerStatus {
    if !inspection.exists {
        return ComputerStatus {
            state: ComputerState::Absent,
            url: None,
            viewer: None,
        };
    }
    if !inspection.running {
        return ComputerStatus {
            state: ComputerState::Stopped,
            url: None,
            viewer: None,
        };
    }
    let mcp = inspection.mcp_port.or(known.map(|known| known.0));
    ComputerStatus {
        state: ComputerState::Running,
        url: mcp.map(mcp_url),
        viewer: match (mcp, known) {
            (Some(port), Some((_, token))) => Some(viewer_url(port, token)),
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
fn image_of(persona: &Persona, room_image: Option<&str>) -> String {
    persona
        .computer
        .as_ref()
        .and_then(|computer| computer.image.as_deref())
        .map(str::trim)
        .filter(|image| !image.is_empty())
        .or(room_image)
        .map(str::to_string)
        .unwrap_or_else(default_image)
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
            .ok_or_else(|| format!("{} is not a runtime Toad looks for.", want.name()))?;
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
        format!("TOAD_COMPUTER_TOKEN={token}"),
        "-v".into(),
        format!("{cwd}:{WORKSPACE_MOUNT}"),
        image.into(),
    ]);
    Ok(args)
}

fn scratch_volume(persona_id: &str) -> String {
    format!("toad-src-{persona_id}")
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
/// a container path may not shadow or sit inside one of Toad's own.
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
                "The computer mount path {path:?} overlaps a path Toad reserves: {}.",
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

// These fixtures execute POSIX shell scripts; native process jobs have Windows coverage.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::contract::{ComputerMount, McpPolicy, PersonaComputer, PolicyMode};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "toad-core-computer-{name}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_millis()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_script(dir: &Path, name: &str, body: &str) {
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// The fake runtime belongs to one test: each writes it into its own
    /// directory with the log it appends to, the state file it reads and the
    /// port it reports baked in. Nothing about it is process-wide, so these
    /// tests run beside each other instead of queueing behind a lock on the
    /// environment. The paths are single quoted and a scratch path holds no
    /// quote to close them with.
    fn fake_runtime(dir: &Path, mcp_port: u16) {
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
    printf '[{"State":{"Running":%s},"NetworkSettings":{"Ports":{"8787/tcp":[{"HostPort":"%s"}]}}}]\n' "$running" "$MCP"
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

    fn persona(id: &str, cwd: &str) -> Persona {
        Persona {
            node: None,
            id: id.to_string(),
            name: "Ada".to_string(),
            goal: String::new(),
            face: None,
            team: None,
            backend_id: "toad".to_string(),
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
            background_work: false,
            allowed_senders: Vec::new(),
            web_search_policy: None,
            computer: Some(PersonaComputer {
                enabled: true,
                image: Some("toad-computer:test".into()),
                memory: None,
                pids: None,
                mounts: None,
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
                "512",
                "--memory",
                "2g",
                "--shm-size",
                "1g",
                "-v",
                "toad-nix:/nix",
                "-v",
                "toad-src-ada:/home/agent/src",
                "-p",
                "127.0.0.1:0:8787",
                "-e",
                "TOAD_COMPUTER_TOKEN=",
                "-v",
                &format!(
                    "{}:{WORKSPACE_MOUNT}",
                    cwd.canonicalize().unwrap().display()
                ),
                "toad-computer:test",
            ],
        );
        assert!(create.contains("--name toad-computer-ada"), "{create}");
        assert!(recorded.contains("start toad-computer-ada"), "{recorded}");
    }

    #[test]
    fn limits_are_the_teammates_else_the_defaults() {
        let mut ada = persona("ada", "/tmp");
        assert_eq!(memory_of(&ada).unwrap(), "2g");
        assert_eq!(pids_of(&ada), "512");
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
    fn mounts_are_existing_folders_outside_toads_own_paths() {
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
            mount("/home/agent/toad-next", true),
            mount("/mnt/data/", false),
        ]);
        let canonical = checkout.canonicalize().unwrap().display().to_string();
        assert_eq!(
            mount_args(&ada).unwrap(),
            vec![
                format!("{canonical}:/home/agent/toad-next:ro"),
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
                .any(|line| line == "stop toad-computer-ada"),
            "{recorded}"
        );
        assert!(
            recorded
                .lines()
                .any(|line| line == "rm -f toad-computer-ada"),
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
                .any(|line| line == "stop toad-computer-ada"),
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
                .any(|line| line == "rm -f toad-computer-ada"),
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
        });
        assert_eq!(image_of(&ada, None), default_image());
        assert_eq!(image_of(&ada, Some("room/image:1")), "room/image:1");
        ada.computer = Some(PersonaComputer {
            enabled: true,
            image: Some("  own/image:2  ".into()),
            memory: None,
            pids: None,
            mounts: None,
        });
        assert_eq!(image_of(&ada, Some("room/image:1")), "own/image:2");
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
