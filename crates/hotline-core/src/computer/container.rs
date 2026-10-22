//! The container CLI underneath a computer: which runtime to use, the hardened
//! arguments a computer is created with, pulling its image, reading back what
//! the runtime says about it, and waiting until its tools answer.

use super::*;
use crate::contract::{FolderGrant, RuntimeState};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

pub(super) async fn pick_runtime(
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

pub(super) fn create_args(
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
    let memory = memory_of(persona)?;
    let cpus = cpus_of(persona, runtime)?;
    args.extend([
        "--memory".into(),
        memory.clone(),
        "--shm-size".into(),
        "1g".into(),
    ]);
    if let Some(cpus) = &cpus {
        args.extend(["--cpus".into(), cpus.clone()]);
    }
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
    let folders = folder_mount_args(persona)?;
    for mount in &folders {
        args.extend(["-v".into(), mount.clone()]);
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
        "-e".into(),
        format!("{FOLDERS_ENV}={}", folders.join("\n")),
        "-e".into(),
        format!("{LIMITS_ENV}={}", limits_record(&memory, cpus.as_deref())),
        "--label".into(),
        format!("{RELEASE_LABEL}={}", release_of(image)),
        "-v".into(),
        format!("{cwd}:{WORKSPACE_MOUNT}"),
        image.into(),
    ]);
    Ok(args)
}

/// The zone the computer's clock shows: the host's, so the bar reads the
/// same as the person's own clock. `TZ` in the desk's environment wins;
/// a host whose zone cannot be read gets UTC rather than an empty `TZ`.
pub(super) fn host_time_zone() -> String {
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

/// The limits a container is made with, as it records them: what the
/// runtime was told, so a start can tell whether the teammate's settings
/// have moved since.
fn limits_record(memory: &str, cpus: Option<&str>) -> String {
    format!("memory={memory} cpus={}", cpus.unwrap_or("all"))
}

/// The record a container made now with the teammate's settings would
/// carry, or why those settings cannot make one.
pub(super) fn limits_of(persona: &Persona, runtime: Runtime) -> Result<String, String> {
    Ok(limits_record(
        &memory_of(persona)?,
        cpus_of(persona, runtime)?.as_deref(),
    ))
}

/// CPU limits, like memory, only apply when the container is created, so a
/// start remakes one whose recorded limits differ (see [`limits_of`]).
/// Apple assigns whole virtual CPUs; Docker/Podman accept fractional quotas.
fn cpus_of(persona: &Persona, runtime: Runtime) -> Result<Option<String>, String> {
    let Some(cpus) = persona.computer.as_ref().and_then(|computer| computer.cpus) else {
        return Ok(None);
    };
    if !cpus.is_finite() || cpus <= 0.0 {
        return Err("The computer's CPU limit must be a finite number greater than zero.".into());
    }
    Ok(Some(if runtime == Runtime::AppleContainer {
        cpus.ceil().max(1.0).to_string()
    } else {
        cpus.to_string()
    }))
}

/// The teammate's memory limit as the runtime spells it, else 4g. A size
/// is digits and at most one unit letter; anything else is refused before
/// the runtime can misread it.
pub(super) fn memory_of(persona: &Persona) -> Result<String, String> {
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
pub(super) fn pids_of(persona: &Persona) -> String {
    match persona.computer.as_ref().and_then(|computer| computer.pids) {
        None => DEFAULT_PIDS.to_string(),
        Some(0) => "-1".to_string(),
        Some(pids) => pids.to_string(),
    }
}

/// The teammate's extra bind mounts as `host:path[:ro]`. A host folder
/// must exist now, because the runtime would otherwise create it as root;
/// a container path may not shadow or sit inside one of Hotline's own.
pub(super) fn mount_args(persona: &Persona) -> Result<Vec<String>, String> {
    let Some(computer) = persona.computer.as_ref() else {
        return Ok(Vec::new());
    };
    let reserved = [WORKSPACE_MOUNT, SCRATCH_MOUNT, NIX_MOUNT, FOLDERS_MOUNT];
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

/// Each granted folder that is still a folder here, with the path the
/// computer shows it at: `/home/agent/folders/<its name>`, the name made
/// safe for a path and numbered when two folders share one. Every teammate
/// with a computer gets them, whatever its reach: a container only ever
/// sees what is bound into it.
pub(crate) fn folder_mounts(persona: &Persona) -> Vec<(&FolderGrant, String)> {
    let mut taken: Vec<String> = Vec::new();
    persona
        .folders
        .iter()
        .flatten()
        .filter(|folder| Path::new(&folder.path).is_dir())
        .map(|folder| {
            let base: String = Path::new(&folder.path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                        c
                    } else {
                        '-'
                    }
                })
                .collect();
            let base = if base.trim_matches('.').is_empty() {
                "folder".to_string()
            } else {
                base
            };
            let mut name = base.clone();
            let mut n = 2;
            while taken.contains(&name) {
                name = format!("{base}-{n}");
                n += 1;
            }
            taken.push(name.clone());
            (folder, format!("{FOLDERS_MOUNT}/{name}"))
        })
        .collect()
}

/// The granted folders as `host:path[:ro]`, read-only unless the grant
/// says the teammate may change it. A folder gone from this machine is left
/// out, because the runtime would otherwise make it, as root.
pub(super) fn folder_mount_args(persona: &Persona) -> Result<Vec<String>, String> {
    folder_mounts(persona)
        .into_iter()
        .map(|(folder, path)| {
            // `-v` separates its fields with `:`, so a host path holding one
            // would be read as other fields.
            if folder.path.contains(':') {
                return Err(format!(
                    "The folder {} has a colon in its path, which a computer cannot mount.",
                    folder.path
                ));
            }
            Ok(if folder.writable {
                format!("{}:{path}", folder.path)
            } else {
                format!("{}:{path}:ro", folder.path)
            })
        })
        .collect()
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

pub(super) async fn image_present(cmd: &Path, _runtime: Runtime, image: &str) -> bool {
    run(cmd, &["image", "inspect", image], COMMAND_TIMEOUT)
        .await
        .is_ok()
}

/// Pulls the image, reporting each layer the runtime names as it lands.
///
/// The runtime's stdout and stderr are read line by line while it runs —
/// Docker writes its progress to stdout, Podman to stderr — and every line
/// that changes the tally is reported at once. The first report goes out
/// before the first line, so the tape shows the pull the moment it starts,
/// and the last says done or failed with the same counts.
pub(super) async fn pull(
    cmd: &Path,
    runtime: Runtime,
    image: &str,
    mut report: impl FnMut(PullReport),
) -> Result<(), String> {
    let args: &[&str] = match runtime {
        Runtime::AppleContainer => &["image", "pull", image],
        _ => &["pull", image],
    };
    let started = std::time::Instant::now();
    let mut tally = PullTally::new(runtime);
    let progress = |tally: &PullTally, outcome: PullOutcome| PullReport {
        image: image.to_string(),
        layers_done: tally.done(),
        layers_total: tally.total(),
        outcome,
    };
    report(progress(&tally, PullOutcome::Pulling));

    let mut command = Command::new(cmd);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    crate::process_windows::quiet(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("{} could not be started: {error}", cmd.display()))?;
    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let (lines, mut heard) = tokio::sync::mpsc::unbounded_channel::<String>();
    let readers = tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let out = lines.clone();
        let from_stdout = async move {
            let mut lines_of = tokio::io::BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines_of.next_line().await {
                if out.send(line).is_err() {
                    break;
                }
            }
        };
        let from_stderr = async move {
            let mut lines_of = tokio::io::BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines_of.next_line().await {
                if lines.send(line).is_err() {
                    break;
                }
            }
        };
        tokio::join!(from_stdout, from_stderr);
    });
    let mut tail: Vec<String> = Vec::new();
    let outcome = tokio::time::timeout(PULL_TIMEOUT, async {
        // The channel closes when both pipes do, which is when the runtime
        // has said everything; only then is its exit status read.
        while let Some(line) = heard.recv().await {
            if tail.len() == 8 {
                tail.remove(0);
            }
            tail.push(line.clone());
            if tally.take(&line) {
                report(progress(&tally, PullOutcome::Pulling));
            }
        }
        child.wait().await
    })
    .await;
    readers.abort();
    match outcome {
        Ok(Ok(status)) if status.success() => {
            let elapsed_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
            report(progress(&tally, PullOutcome::Done { elapsed_ms }));
            Ok(())
        }
        Ok(Ok(status)) => {
            report(progress(&tally, PullOutcome::Failed));
            Err(format!(
                "{} pull failed ({}): {}",
                cmd.display(),
                status.code().unwrap_or(-1),
                tail.join("\n").trim()
            ))
        }
        Ok(Err(error)) => {
            report(progress(&tally, PullOutcome::Failed));
            Err(format!(
                "{} pull could not be waited for: {error}",
                cmd.display()
            ))
        }
        Err(_) => {
            report(progress(&tally, PullOutcome::Failed));
            Err(format!("{} pull timed out", cmd.display()))
        }
    }
}

/// The layers a pull has named and finished, read off the runtime's lines.
///
/// Docker, to a pipe, writes `<id>: Pulling fs layer` when it starts one,
/// `<id>: Pull complete` when it finishes, and `<id>: Already exists` for
/// one it had. Podman writes `Copying blob <digest>` when it starts one and
/// `Copying blob <digest> done` when it finishes, or `… skipped: already
/// exists`. Both are counted by layer id, so a line repeated is not a layer
/// counted twice. Apple's `container` prints nothing the desk reads yet.
pub(super) struct PullTally {
    runtime: Runtime,
    named: std::collections::HashSet<String>,
    finished: std::collections::HashSet<String>,
}

impl PullTally {
    pub(super) fn new(runtime: Runtime) -> Self {
        Self {
            runtime,
            named: HashSet::default(),
            finished: HashSet::default(),
        }
    }

    pub(super) fn total(&self) -> u32 {
        u32::try_from(self.named.len()).unwrap_or(u32::MAX)
    }

    pub(super) fn done(&self) -> u32 {
        u32::try_from(self.finished.len()).unwrap_or(u32::MAX)
    }

    /// Reads one line; true when the tally changed.
    pub(super) fn take(&mut self, line: &str) -> bool {
        let line = line.trim();
        let (id, finished) = match self.runtime {
            Runtime::Docker => {
                let Some((id, status)) = line.split_once(": ") else {
                    return false;
                };
                if id.contains(' ') || id.is_empty() {
                    return false;
                }
                match status.trim() {
                    "Pulling fs layer" => (id, false),
                    "Pull complete" | "Already exists" => (id, true),
                    _ => return false,
                }
            }
            Runtime::Podman => {
                let Some(rest) = line.strip_prefix("Copying blob ") else {
                    return false;
                };
                let mut words = rest.split_whitespace();
                let Some(id) = words.next() else {
                    return false;
                };
                let rest = rest[id.len()..].trim();
                let finished = rest.starts_with("done") || rest.starts_with("skipped");
                (id, finished)
            }
            Runtime::AppleContainer => return false,
        };
        let id = id.to_string();
        let mut changed = self.named.insert(id.clone());
        if finished {
            changed |= self.finished.insert(id);
        }
        changed
    }
}

pub(super) async fn inspect(
    cmd: &Path,
    runtime: Runtime,
    name: &str,
) -> Result<Inspection, String> {
    inspect_within(cmd, runtime, name, COMMAND_TIMEOUT).await
}

pub(super) async fn inspect_within(
    cmd: &Path,
    runtime: Runtime,
    name: &str,
    timeout: Duration,
) -> Result<Inspection, String> {
    match run(cmd, &["inspect", name], timeout).await {
        Ok(stdout) => parse_inspect(runtime, &stdout),
        Err(error) if is_not_found(&error) => Ok(Inspection {
            exists: false,
            running: false,
            mcp_port: None,
            token: None,
            folders: String::new(),
            limits: None,
            release: None,
        }),
        Err(error) => Err(error),
    }
}

pub(super) fn parse_inspect(runtime: Runtime, stdout: &str) -> Result<Inspection, String> {
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
        folders: from_environment(object.pointer("/Config/Env"), FOLDERS_ENV).unwrap_or_default(),
        limits: from_environment(object.pointer("/Config/Env"), LIMITS_ENV),
        release: inspected_release(
            object
                .pointer("/Config/Labels")
                .and_then(|labels| labels.get(RELEASE_LABEL)),
            object
                .pointer("/Config/Image")
                .or_else(|| object.get("ImageName")),
        ),
    })
}

/// Older containers have no Hotline label, but the runtime still knows
/// their image reference. The top-level Docker/Podman `Image` is an ID,
/// not that reference, and must never become a release.
fn inspected_release(label: Option<&Value>, image: Option<&Value>) -> Option<String> {
    label
        .and_then(Value::as_str)
        .filter(|release| !release.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            image
                .and_then(Value::as_str)
                .filter(|image| !image.is_empty())
                .map(release_of)
        })
}

pub(super) fn token_from_environment(environment: Option<&Value>) -> Option<String> {
    from_environment(environment, "HOTLINE_COMPUTER_TOKEN").filter(|token| !token.is_empty())
}

fn from_environment(environment: Option<&Value>, name: &str) -> Option<String> {
    environment?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .find_map(|entry| {
            entry
                .strip_prefix(name)
                .and_then(|rest| rest.strip_prefix('='))
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

pub(super) fn parse_apple(object: &Value) -> Result<Inspection, String> {
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
        folders: from_environment(
            object.pointer("/configuration/initProcess/environment"),
            FOLDERS_ENV,
        )
        .unwrap_or_default(),
        limits: from_environment(
            object.pointer("/configuration/initProcess/environment"),
            LIMITS_ENV,
        ),
        release: inspected_release(
            object
                .pointer("/configuration/labels")
                .and_then(|labels| labels.get(RELEASE_LABEL)),
            object.pointer("/configuration/image/reference"),
        ),
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

pub(super) async fn run(cmd: &Path, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut command = Command::new(cmd);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    crate::process_windows::quiet(&mut command);
    let child = command
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

pub(super) async fn wait_healthy(port: u16) -> Result<(), String> {
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
