//! What each agent last offered on this desk — its models, and its modes and
//! effort with the value each opened on — so the picker can list an agent's
//! choices before a teammate runs on it, and a teammate at rest shows the
//! ones its next session will have.
//!
//! A teammate's session records what its agent reports as it opens. When the
//! desk starts, and when the picker asks, each agent people know by name and
//! each that has answered here before, with no list from the last day, gets a
//! quiet look: it is started in an empty folder of its own, told Hotline
//! offers no files, tools or servers, asked to open a session, and ended. No
//! prompt is ever sent, so the look spends nothing. An agent that answers the
//! session with "sign in first" is remembered as needing it, until a session
//! of its opens.

use super::session::Disposition;
#[cfg(unix)]
use super::{GROUPS, lock};
use super::{end_group, registry};
use crate::contract::{BackendOffer, ConfigChoice, SessionConfig};
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{InitializeRequest, NewSessionRequest};
use agent_client_protocol::{Agent, ByteStreams, Client, ConnectionTo};
use hotline_support::paths;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::oneshot;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

/// How old a list may be before the agent is looked at again.
const FRESH_MS: i64 = 24 * 60 * 60 * 1000;
/// How long one look may take from spawn to the session's answer.
const LOOK_BOUND: Duration = Duration::from_secs(60);
/// The first look at an agent may be fetching its adapter package.
const FIRST_LOOK_BOUND: Duration = Duration::from_mins(5);

/// One look at a time across the desk, so a sweep is slow and quiet.
static LOOKING: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Seen {
    /// When the list was last read, or a look last tried and failed.
    at: i64,
    #[serde(default)]
    models: Vec<ConfigChoice>,
    /// The agent asked to be signed in before it would open a session.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    sign_in: bool,
    /// The rest of the offer: the model it opened on, its modes and its
    /// effort pickers. Absent in a file from before they were kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    current_model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model_label: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    modes: Vec<ConfigChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    current_mode_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mode_label: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    configs: Vec<SessionConfig>,
}

impl Seen {
    fn offer(&self) -> BackendOffer {
        BackendOffer {
            models: self.models.clone(),
            current_model_id: self.current_model_id.clone(),
            model_label: self.model_label.clone(),
            modes: self.modes.clone(),
            current_mode_id: self.current_mode_id.clone(),
            mode_label: self.mode_label.clone(),
            configs: self.configs.clone(),
        }
    }

    /// Takes a new offer. An empty part is not a part: an agent that names
    /// no models, modes or settings this time keeps the ones it had.
    fn take(&mut self, offer: &BackendOffer) {
        if !offer.models.is_empty() {
            self.models.clone_from(&offer.models);
            if offer.current_model_id.is_some() {
                self.current_model_id.clone_from(&offer.current_model_id);
            }
            if offer.model_label.is_some() {
                self.model_label.clone_from(&offer.model_label);
            }
        }
        if !offer.modes.is_empty() {
            self.modes.clone_from(&offer.modes);
            if offer.current_mode_id.is_some() {
                self.current_mode_id.clone_from(&offer.current_mode_id);
            }
            if offer.mode_label.is_some() {
                self.mode_label.clone_from(&offer.mode_label);
            }
        }
        if !offer.configs.is_empty() {
            self.configs.clone_from(&offer.configs);
        }
    }
}

/// How a look ended without a list.
enum Missed {
    /// The agent wants its own sign-in first.
    SignIn,
    Failed(String),
}

fn read(root: &Path) -> BTreeMap<String, Seen> {
    std::fs::read(paths::acp_models_path(root))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write(root: &Path, seen: &BTreeMap<String, Seen>) {
    let path = paths::acp_models_path(root);
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, serde_json::to_vec(seen).unwrap_or_default()));
    if let Err(error) = written {
        eprintln!("[acp] {} could not be written: {error}", path.display());
    }
}

/// The list each agent last reported, by backend id. An agent that never
/// has is absent.
pub fn cached(root: &Path) -> BTreeMap<String, Vec<ConfigChoice>> {
    read(root)
        .into_iter()
        .filter(|(_, seen)| !seen.models.is_empty())
        .map(|(id, seen)| (id, seen.models))
        .collect()
}

/// What this agent last offered here, or nothing when it never has.
pub fn offer(root: &Path, backend_id: &str) -> Option<BackendOffer> {
    read(root)
        .get(backend_id)
        .map(Seen::offer)
        .filter(|offer| !offer.is_empty())
}

/// The agents that last asked to be signed in, by backend id.
pub fn needing_sign_in(root: &Path) -> std::collections::BTreeSet<String> {
    read(root)
        .into_iter()
        .filter(|(_, seen)| seen.sign_in)
        .map(|(id, _)| id)
        .collect()
}

/// A session of this agent asked to be signed in first: the picker says so
/// until one opens. The list it had stays.
pub fn mark_sign_in(root: &Path, backend_id: &str) {
    let mut seen = read(root);
    let entry = seen.entry(backend_id.to_string()).or_default();
    if entry.sign_in {
        return;
    }
    entry.sign_in = true;
    write(root, &seen);
}

/// Records what an agent offered as a session opened, which also says it is
/// signed in. An empty part is not a part: an agent that names no models
/// keeps the ones it had, and the same for its modes and settings.
pub fn remember(root: &Path, backend_id: &str, offer: &BackendOffer) {
    let mut seen = read(root);
    let now = now_ms();
    let was = seen.get(backend_id).cloned().unwrap_or_default();
    let mut now_seen = Seen {
        sign_in: false,
        ..was.clone()
    };
    now_seen.take(offer);
    if offer.models.is_empty() {
        // Not a list, so not a reading: the day's look still comes.
        if was.sign_in || now_seen.offer() != was.offer() {
            seen.insert(backend_id.to_string(), now_seen);
            write(root, &seen);
        }
        return;
    }
    if !was.sign_in && was.offer() == now_seen.offer() && now - was.at < FRESH_MS / 2 {
        return;
    }
    now_seen.at = now;
    seen.insert(backend_id.to_string(), now_seen);
    write(root, &seen);
}

/// Looks at each agent people know by name and each that has answered here
/// before, with no list from the last day, one after another, and returns at
/// once. The desk calls it as it starts, so the picker never waits on a list.
pub fn refresh_in_background(root: &Path) {
    if LOOKING.swap(true, Ordering::SeqCst) {
        return;
    }
    let root = root.to_path_buf();
    tokio::spawn(async move {
        let seen = read(&root);
        let now = now_ms();
        for backend in registry::cached_backends(&root) {
            if backend.unavailable.is_some()
                || seen
                    .get(&backend.id)
                    .is_some_and(|was| now - was.at < FRESH_MS)
                || !plain_id(&backend.id)
            {
                continue;
            }
            let Ok(launch) = registry::launch(&root, &backend.id) else {
                continue;
            };
            let before = registry::started_before(&root, &launch);
            if !before && !registry::named(&backend.id) {
                continue;
            }
            let bound = if before { LOOK_BOUND } else { FIRST_LOOK_BOUND };
            let (offer, sign_in) = match look(&root, &backend.id, &launch, bound).await {
                Ok(offer) => (offer, false),
                Err(Missed::SignIn) => (BackendOffer::default(), true),
                Err(Missed::Failed(error)) => {
                    eprintln!("[acp] a look at {}'s models failed: {error}", backend.id);
                    (BackendOffer::default(), false)
                }
            };
            // A failed look waits a day like a good one; the offer it had stays.
            let mut seen = read(&root);
            let entry = seen.entry(backend.id.clone()).or_default();
            entry.at = now_ms();
            entry.sign_in = sign_in;
            entry.take(&offer);
            write(&root, &seen);
        }
        LOOKING.store(false, Ordering::SeqCst);
    });
}

/// An id that is safe as a folder name: the catalogue's ids are, but it is
/// fetched from the network.
fn plain_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// One look: spawn, `initialize` offering nothing, `session/new` in the
/// agent's empty folder with no servers, read its pickers, end.
async fn look(
    root: &Path,
    backend_id: &str,
    launch: &registry::Launch,
    bound: Duration,
) -> Result<BackendOffer, Missed> {
    let folder = paths::acp_look_dir(root, backend_id);
    std::fs::create_dir_all(&folder).map_err(|error| Missed::Failed(error.to_string()))?;
    let mut command = tokio::process::Command::new(&launch.command);
    command
        .args(&launch.args)
        .envs(launch.env.iter().cloned())
        .current_dir(&folder)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(windows)]
    crate::process_windows::prepare(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| Missed::Failed(error.to_string()))?;
    #[cfg(unix)]
    if let Some(id) = child.id() {
        lock(&GROUPS).push(id as libc::pid_t);
    }
    #[cfg(windows)]
    let _job = crate::process_windows::Job::attach(child.id())
        .map_err(|error| Missed::Failed(error.to_string()))?;
    // The bound is kept here, where the child is owned: a look that runs out
    // of time still ends the agent's whole process group below, launcher
    // children (npx, uvx) and all.
    let looked = match (child.stdin.take(), child.stdout.take()) {
        (Some(stdin), Some(stdout)) => tokio::time::timeout(
            bound,
            ask(
                ByteStreams::new(stdin.compat_write(), stdout.compat()),
                folder,
            ),
        )
        .await
        .unwrap_or_else(|_| Err(Missed::Failed("it ran out of time".to_string()))),
        _ => Err(Missed::Failed("the agent gave no pipes".to_string())),
    };
    end_group(&child);
    let _ = child.start_kill();
    let _ = child.wait().await;
    looked
}

/// The conversation of a look, over any pipe: offer no capabilities, open a
/// session naming no servers, read its pickers.
async fn ask(
    transport: ByteStreams<
        impl futures_util::AsyncWrite + Send + 'static,
        impl futures_util::AsyncRead + Send + 'static,
    >,
    folder: std::path::PathBuf,
) -> Result<BackendOffer, Missed> {
    let (ready, started) = oneshot::channel();
    let (done, finished) = oneshot::channel::<()>();
    let task = tokio::spawn(Client.builder().name("Hotline").connect_with(
        transport,
        async move |cx: ConnectionTo<Agent>| {
            let _ = ready.send(cx.clone());
            let _ = finished.await;
            Ok(())
        },
    ));
    let asked = async {
        let connection = started
            .await
            .map_err(|_| Missed::Failed("the connection ended before it opened".to_string()))?;
        connection
            .send_request(InitializeRequest::new(ProtocolVersion::V1))
            .block_task()
            .await
            .map_err(|error| Missed::Failed(error.to_string()))?;
        let opened = connection
            .send_request(NewSessionRequest::new(folder))
            .block_task()
            .await
            .map_err(|error| {
                // The same reading a teammate's start gives the same refusal.
                let said = format!("The agent would not open a session: {error}");
                let failure = crate::driver::failure::Failure::classify(&said, None, "acp_start");
                if failure.kind == crate::driver::failure::Kind::AgentAuth {
                    Missed::SignIn
                } else {
                    Missed::Failed(error.to_string())
                }
            })?;
        Ok(Disposition::of(opened.modes.as_ref(), opened.config_options.as_deref()).offer())
    }
    .await;
    let _ = done.send(());
    task.abort();
    asked
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(id: &str) -> ConfigChoice {
        ConfigChoice {
            id: id.into(),
            name: id.into(),
            description: None,
            group: None,
            account: None,
        }
    }

    fn offered(models: &[ConfigChoice]) -> BackendOffer {
        BackendOffer {
            models: models.to_vec(),
            ..BackendOffer::default()
        }
    }

    #[test]
    fn an_agents_reported_models_are_kept_and_an_empty_report_keeps_them() {
        let root = tempfile::tempdir().unwrap();
        assert!(cached(root.path()).is_empty());
        remember(
            root.path(),
            "claude-acp",
            &offered(&[choice("opus"), choice("sonnet")]),
        );
        remember(root.path(), "claude-acp", &offered(&[]));
        let cached = cached(root.path());
        assert_eq!(cached["claude-acp"], vec![choice("opus"), choice("sonnet")]);
        assert!(!cached.contains_key("codex-acp"));
    }

    /// The whole offer is kept, effort and modes with the value each opened
    /// on, read back from the file as the desk reads it after a restart; a
    /// later session that names no settings keeps them.
    #[test]
    fn an_agents_whole_offer_is_kept_on_disk_and_a_bare_report_keeps_its_parts() {
        let root = tempfile::tempdir().unwrap();
        let effort = SessionConfig {
            id: "reasoning".into(),
            name: "Effort".into(),
            category: Some(crate::contract::SessionConfigCategory::Effort),
            current_id: Some("high".into()),
            options: vec![choice("low"), choice("high")],
        };
        let offer = BackendOffer {
            models: vec![choice("opus"), choice("sonnet")],
            current_model_id: Some("opus".into()),
            model_label: Some("Model".into()),
            modes: vec![choice("default"), choice("plan")],
            current_mode_id: Some("default".into()),
            mode_label: Some("Mode".into()),
            configs: vec![effort],
        };
        assert_eq!(offer_of(root.path()), None);
        remember(root.path(), "claude-acp", &offer);
        remember(
            root.path(),
            "claude-acp",
            &offered(&[choice("opus"), choice("sonnet")]),
        );
        let written = std::fs::read(paths::acp_models_path(root.path())).unwrap();
        let read_back: BTreeMap<String, Seen> = serde_json::from_slice(&written).unwrap();
        assert_eq!(read_back["claude-acp"].offer(), offer);
        assert_eq!(offer_of(root.path()), Some(offer));
        assert_eq!(super::offer(root.path(), "codex-acp"), None);
    }

    fn offer_of(root: &Path) -> Option<BackendOffer> {
        super::offer(root, "claude-acp")
    }

    /// The look offers the agent nothing, names no servers, reads the model
    /// picker its session opens with, and never prompts.
    #[tokio::test]
    async fn a_look_offers_nothing_and_reads_the_model_picker_without_a_prompt() {
        use agent_client_protocol::schema::v1 as acp;
        use std::sync::{Arc, Mutex};
        let (client_writer, agent_reader) = tokio::io::duplex(64 * 1024);
        let (agent_writer, client_reader) = tokio::io::duplex(64 * 1024);
        let heard = Arc::new(Mutex::new(Vec::<String>::new()));
        let (on_init, on_new, on_prompt) = (heard.clone(), heard.clone(), heard.clone());
        tokio::spawn(
            Agent
                .builder()
                .name("scripted")
                .on_receive_request(
                    async move |request: acp::InitializeRequest, responder, _cx| {
                        let fs = request.client_capabilities.fs;
                        on_init.lock().unwrap().push(format!(
                            "initialize fs={} {}",
                            fs.read_text_file, fs.write_text_file
                        ));
                        responder.respond(acp::InitializeResponse::new(request.protocol_version))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |request: acp::NewSessionRequest, responder, _cx| {
                        on_new
                            .lock()
                            .unwrap()
                            .push(format!("session/new servers={}", request.mcp_servers.len()));
                        let model = acp::SessionConfigOption::select(
                            "model".to_string(),
                            "Model".to_string(),
                            "opus".to_string(),
                            vec![
                                acp::SessionConfigSelectOption::new(
                                    "opus".to_string(),
                                    "Opus".to_string(),
                                ),
                                acp::SessionConfigSelectOption::new(
                                    "sonnet".to_string(),
                                    "Sonnet".to_string(),
                                ),
                            ],
                        )
                        .category(Some(acp::SessionConfigOptionCategory::Model));
                        responder.respond(
                            acp::NewSessionResponse::new(acp::SessionId::new("look"))
                                .config_options(vec![model]),
                        )
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |_request: acp::PromptRequest,
                                responder: agent_client_protocol::Responder<
                        acp::PromptResponse,
                    >,
                                _cx| {
                        on_prompt.lock().unwrap().push("prompt".to_string());
                        responder.respond(acp::PromptResponse::new(acp::StopReason::EndTurn))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_to(ByteStreams::new(
                    agent_writer.compat_write(),
                    agent_reader.compat(),
                )),
        );
        let folder = tempfile::tempdir().unwrap();
        let offer = ask(
            ByteStreams::new(client_writer.compat_write(), client_reader.compat()),
            folder.path().to_path_buf(),
        )
        .await
        .unwrap_or_else(|_| panic!("the look failed"));
        assert_eq!(
            offer
                .models
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["opus", "sonnet"]
        );
        assert_eq!(offer.current_model_id.as_deref(), Some("opus"));
        assert_eq!(
            *heard.lock().unwrap(),
            ["initialize fs=false false", "session/new servers=0"]
        );
    }

    /// An agent that refuses the session until it is signed in is marked so
    /// the picker can offer its sign-in, and a session of it opening clears it.
    #[tokio::test]
    async fn a_look_that_meets_sign_in_marks_it_until_a_session_opens() {
        use agent_client_protocol::schema::v1 as acp;
        let (client_writer, agent_reader) = tokio::io::duplex(64 * 1024);
        let (agent_writer, client_reader) = tokio::io::duplex(64 * 1024);
        tokio::spawn(
            Agent
                .builder()
                .name("scripted")
                .on_receive_request(
                    async move |request: acp::InitializeRequest, responder, _cx| {
                        responder.respond(acp::InitializeResponse::new(request.protocol_version))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |_request: acp::NewSessionRequest,
                                responder: agent_client_protocol::Responder<
                        acp::NewSessionResponse,
                    >,
                                _cx| {
                        responder.respond_with_error(agent_client_protocol::Error::new(
                            -32000,
                            "Authentication required",
                        ))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_to(ByteStreams::new(
                    agent_writer.compat_write(),
                    agent_reader.compat(),
                )),
        );
        let folder = tempfile::tempdir().unwrap();
        let looked = ask(
            ByteStreams::new(client_writer.compat_write(), client_reader.compat()),
            folder.path().to_path_buf(),
        )
        .await;
        assert!(matches!(looked, Err(Missed::SignIn)));

        let root = tempfile::tempdir().unwrap();
        remember(root.path(), "cursor", &offered(&[choice("auto")]));
        mark_sign_in(root.path(), "cursor");
        assert!(needing_sign_in(root.path()).contains("cursor"));
        // Re-signed in: the list it had stays the whole time.
        assert_eq!(cached(root.path())["cursor"], vec![choice("auto")]);
        remember(root.path(), "cursor", &offered(&[choice("auto")]));
        assert!(needing_sign_in(root.path()).is_empty());
        // A session that names no models still says it signed in.
        mark_sign_in(root.path(), "grok-build");
        remember(root.path(), "grok-build", &offered(&[]));
        assert!(needing_sign_in(root.path()).is_empty());
    }

    /// An agent that never answers is ended with everything it started when
    /// the look runs out of time, not left running until the desk quits.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_look_that_runs_out_of_time_ends_the_agents_whole_process_group() {
        let root = tempfile::tempdir().unwrap();
        let pidfile = root.path().join("grandchild.pid");
        let launch = registry::Launch {
            command: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                format!("sleep 600 & echo $! > {}; sleep 600", pidfile.display()),
            ],
            env: Vec::new(),
        };
        let looked = look(root.path(), "hangs", &launch, Duration::from_millis(400)).await;
        assert!(matches!(looked, Err(Missed::Failed(_))));
        let pid: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // The launcher's own child went with the group (reaped by init, so give it a moment).
        let gone = (0..50).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(20));
            // Safety: signal 0 only asks whether the pid exists.
            (unsafe { libc::kill(pid, 0) }) != 0
        });
        assert!(gone, "the hung agent's child {pid} is still running");
    }

    #[test]
    fn only_a_plain_id_names_a_look_folder() {
        assert!(plain_id("claude-acp"));
        assert!(plain_id("gemini_cli.v2"));
        assert!(!plain_id("../vault"));
        assert!(!plain_id("a/b"));
        assert!(!plain_id(".."));
        assert!(!plain_id(""));
    }
}
