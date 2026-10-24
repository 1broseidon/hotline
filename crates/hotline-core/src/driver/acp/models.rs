//! The models each agent last reported on this desk, so the picker can list
//! an agent's models before a teammate runs on it.
//!
//! A teammate's session records what its agent reports as it opens. An agent
//! that has answered on this machine before, and has no list from the last
//! day, gets a quiet look: it is started in an empty folder of its own, told
//! Hotline offers no files, tools or servers, asked to open a session, and
//! ended. No prompt is ever sent, so the look spends nothing.

use super::session::Disposition;
#[cfg(unix)]
use super::{GROUPS, lock};
use super::{end_group, registry};
use crate::contract::ConfigChoice;
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

/// One look at a time across the desk, so a sweep is slow and quiet.
static LOOKING: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Seen {
    /// When the list was last read, or a look last tried and failed.
    at: i64,
    #[serde(default)]
    models: Vec<ConfigChoice>,
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

/// Records what an agent reported. An empty list is not a list: an agent
/// that names no models keeps the one it had.
pub fn remember(root: &Path, backend_id: &str, models: &[ConfigChoice]) {
    if models.is_empty() {
        return;
    }
    let mut seen = read(root);
    let now = now_ms();
    if seen
        .get(backend_id)
        .is_some_and(|was| was.models == models && now - was.at < FRESH_MS / 2)
    {
        return;
    }
    seen.insert(
        backend_id.to_string(),
        Seen {
            at: now,
            models: models.to_vec(),
        },
    );
    write(root, &seen);
}

/// Looks at every agent that has answered here before and has no list from
/// the last day, one after another, and returns at once.
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
            if !registry::started_before(&root, &launch) {
                continue;
            }
            let looked = tokio::time::timeout(LOOK_BOUND, look(&root, &backend.id, &launch)).await;
            let models = match looked {
                Ok(Ok(models)) => models,
                Ok(Err(error)) => {
                    eprintln!("[acp] a look at {}'s models failed: {error}", backend.id);
                    Vec::new()
                }
                Err(_) => {
                    eprintln!("[acp] a look at {}'s models ran out of time", backend.id);
                    Vec::new()
                }
            };
            // A failed look waits a day like a good one; the list it had stays.
            let mut seen = read(&root);
            let entry = seen.entry(backend.id.clone()).or_default();
            entry.at = now_ms();
            if !models.is_empty() {
                entry.models = models;
            }
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
/// agent's empty folder with no servers, read the model picker, end.
async fn look(
    root: &Path,
    backend_id: &str,
    launch: &registry::Launch,
) -> Result<Vec<ConfigChoice>, String> {
    let folder = paths::acp_look_dir(root, backend_id);
    std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
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
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    #[cfg(unix)]
    if let Some(id) = child.id() {
        lock(&GROUPS).push(id as libc::pid_t);
    }
    #[cfg(windows)]
    let _job =
        crate::process_windows::Job::attach(child.id()).map_err(|error| error.to_string())?;
    let looked = match (child.stdin.take(), child.stdout.take()) {
        (Some(stdin), Some(stdout)) => {
            ask(
                ByteStreams::new(stdin.compat_write(), stdout.compat()),
                folder,
            )
            .await
        }
        _ => Err("the agent gave no pipes".to_string()),
    };
    end_group(&child);
    let _ = child.start_kill();
    let _ = child.wait().await;
    looked
}

/// The conversation of a look, over any pipe: offer no capabilities, open a
/// session naming no servers, read its model picker.
async fn ask(
    transport: ByteStreams<
        impl futures_util::AsyncWrite + Send + 'static,
        impl futures_util::AsyncRead + Send + 'static,
    >,
    folder: std::path::PathBuf,
) -> Result<Vec<ConfigChoice>, String> {
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
            .map_err(|_| "the connection ended before it opened".to_string())?;
        connection
            .send_request(InitializeRequest::new(ProtocolVersion::V1))
            .block_task()
            .await
            .map_err(|error| error.to_string())?;
        let opened = connection
            .send_request(NewSessionRequest::new(folder))
            .block_task()
            .await
            .map_err(|error| error.to_string())?;
        Ok(Disposition::of(opened.modes.as_ref(), opened.config_options.as_deref()).models())
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

    #[test]
    fn an_agents_reported_models_are_kept_and_an_empty_report_keeps_them() {
        let root = tempfile::tempdir().unwrap();
        assert!(cached(root.path()).is_empty());
        remember(
            root.path(),
            "claude-acp",
            &[choice("opus"), choice("sonnet")],
        );
        remember(root.path(), "claude-acp", &[]);
        let cached = cached(root.path());
        assert_eq!(cached["claude-acp"], vec![choice("opus"), choice("sonnet")]);
        assert!(!cached.contains_key("codex-acp"));
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
        let models = ask(
            ByteStreams::new(client_writer.compat_write(), client_reader.compat()),
            folder.path().to_path_buf(),
        )
        .await
        .unwrap();
        assert_eq!(
            models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["opus", "sonnet"]
        );
        assert_eq!(
            *heard.lock().unwrap(),
            ["initialize fs=false false", "session/new servers=0"]
        );
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
