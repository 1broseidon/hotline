//! What each agent last offered on this desk — its models, and its modes and
//! effort with the value each opened on — so the picker can list an agent's
//! choices before a teammate runs on it, and a teammate at rest shows the
//! ones its next session will have.
//!
//! A teammate's session records what its agent reports as it opens. When the
//! desk starts, when the picker asks for the agents, and when a teammate's
//! menu or pane opens, each agent people know by name and each that has
//! answered here before gets a quiet look when it is due: no list from the
//! last day, or no modes and no look in the last day (a session that opened
//! before modes were kept left a fresh list with none). It is started in an
//! empty folder of its own, told Hotline offers no files, tools or servers,
//! asked to open a session, and ended. No prompt is ever sent, so the look
//! spends nothing. An agent that answers the session with "sign in first" is
//! remembered as needing it, until a session of its opens.
//!
//! Claude Code's adapter (`claude-agent-acp`, `newSession`) starts the CLI
//! for `session/new` and answers with its modes, models and settings; it
//! asks for a title only on a prompt, and the CLI writes no transcript until
//! a message is sent, so a look leaves no session behind.
//!
//! Beside each offer is the harness's catalogue, when its login's plan read
//! lists its models ([`super::catalog`]): every model with its own effort
//! levels, which fill what the offer does not say.

#[cfg(unix)]
use super::GROUPS;
use super::catalog::Catalog;
use super::session::Disposition;
use super::{end_group, lock, registry};
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
/// What an offer kept holds, raised when it starts to hold more: an offer
/// kept before is looked at again rather than waiting out its day. 1: fast
/// mode; 2: effort levels least first, by the desk's names.
const SHAPE: u32 = 2;
/// How long one look may take from spawn to the session's answer.
const LOOK_BOUND: Duration = Duration::from_secs(60);
/// The first look at an agent may be fetching its adapter package.
const FIRST_LOOK_BOUND: Duration = Duration::from_mins(5);
/// How long an agent looked at is given to leave by itself once its input
/// has ended.
const LEAVE_BOUND: Duration = Duration::from_secs(3);

/// One sweep at a time across the desk, so a sweep is slow and quiet.
static LOOKING: AtomicBool = AtomicBool::new(false);

/// The agents being looked at now: one look at an agent at a time, whether
/// the sweep or a teammate's menu asked for it.
static LOOKING_AT: std::sync::Mutex<std::collections::BTreeSet<String>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

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
    /// Every model with its levels, as the harness's own tools last listed
    /// them out of band, and when ([`super::catalog`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    catalog: Option<Catalog>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    catalog_at: Option<i64>,
    /// When the agent was last looked at out of band, so one that offers no
    /// modes is not started again for them more than once a day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    looked_at: Option<i64>,
    /// The [`SHAPE`] of the offer kept: 0 in a file from before it was.
    #[serde(default)]
    shape: u32,
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

    /// The offer with each model's levels, the catalogue filling what no
    /// session reported.
    fn offered(&self) -> BackendOffer {
        let mut offer = self.offer();
        super::catalog::fill(&mut offer, self.catalog.as_ref());
        offer
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

/// The list each agent last reported, or its catalogue listed, by backend
/// id, each model with its levels. An agent that has neither is absent.
pub fn cached(root: &Path) -> BTreeMap<String, Vec<ConfigChoice>> {
    read(root)
        .into_iter()
        .map(|(id, seen)| (id, seen.offered().models))
        .filter(|(_, models)| !models.is_empty())
        .collect()
}

/// What this agent last offered here, each model with its levels, or
/// nothing when it never has and no catalogue of it has been read.
pub fn offer(root: &Path, backend_id: &str) -> Option<BackendOffer> {
    read(root)
        .get(backend_id)
        .map(Seen::offered)
        .filter(|offer| !offer.is_empty())
}

/// Keeps the agent's catalogue, read now; the offer it had stays. A read
/// that listed nothing keeps the catalogue it had and still counts as a
/// read, so a harness that does not list its models is not started again
/// each time the picker opens. Whether the catalogue kept has changed.
pub fn take_catalog(root: &Path, backend_id: &str, catalog: Option<Catalog>) -> bool {
    let mut seen = read(root);
    let entry = seen.entry(backend_id.to_string()).or_default();
    let changed = catalog.is_some() && catalog != entry.catalog;
    if changed {
        entry.catalog = catalog;
    }
    entry.catalog_at = Some(now_ms());
    write(root, &seen);
    changed
}

/// The models a session of this agent opened with, and any its catalogue
/// lists that the session did not ([`super::catalog::merged`]): a Grok
/// session opened while its CLI renews its sign-in lists only two fallback
/// models. A session that lists none has none.
pub fn with_catalogue(
    root: &Path,
    backend_id: &str,
    models: Vec<ConfigChoice>,
) -> Vec<ConfigChoice> {
    match read(root)
        .get(backend_id)
        .and_then(|seen| seen.catalog.as_ref())
    {
        Some(catalog) if !models.is_empty() => super::catalog::merged(models, catalog),
        _ => models,
    }
}

/// When the agent's models were last read out of band, if they ever were.
pub fn catalog_at(root: &Path, backend_id: &str) -> Option<i64> {
    read(root).get(backend_id).and_then(|seen| seen.catalog_at)
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
    now_seen.shape = SHAPE;
    seen.insert(backend_id.to_string(), now_seen);
    write(root, &seen);
}

/// Whether an agent is due a look: it has no list from the last day, or
/// it has never offered modes here and was not looked at in the last day.
fn due(was: Option<&Seen>, now: i64) -> bool {
    was.is_none_or(|was| {
        was.shape < SHAPE
            || now - was.at >= FRESH_MS
            || (was.modes.is_empty() && was.looked_at.is_none_or(|at| now - at >= FRESH_MS))
    })
}

/// Looks at each agent people know by name and each that has answered here
/// before, when it is due, one after another, and returns at once; each
/// agent whose offer changed is handed to `changed`, so the teammates on it
/// that are not running are drawn again. The desk calls it as it starts, so
/// the picker never waits on a list.
pub fn refresh_in_background(root: &Path, changed: impl Fn(&str) + Send + 'static) {
    if LOOKING.swap(true, Ordering::SeqCst) {
        return;
    }
    let root = root.to_path_buf();
    tokio::spawn(async move {
        for backend in registry::cached_backends(&root) {
            if backend.unavailable.is_none() && look_if_due(&root, &backend.id).await {
                changed(&backend.id);
            }
        }
        LOOKING.store(false, Ordering::SeqCst);
    });
}

/// The look at one agent under way: the next one waits for none, it skips.
struct Looking(String);

impl Looking {
    fn begin(backend_id: &str) -> Option<Self> {
        lock(&LOOKING_AT)
            .insert(backend_id.to_string())
            .then(|| Self(backend_id.to_string()))
    }
}

impl Drop for Looking {
    fn drop(&mut self) {
        lock(&LOOKING_AT).remove(&self.0);
    }
}

/// Looks at this agent when it is due ([`due`]) and is one the desk may
/// start for it, and keeps what it offers. Whether what the desk offers for
/// it, or its need to be signed in, changed.
pub async fn look_if_due(root: &Path, backend_id: &str) -> bool {
    if !plain_id(backend_id) || !due(read(root).get(backend_id), now_ms()) {
        return false;
    }
    let Ok(launch) = registry::launch(root, backend_id) else {
        return false;
    };
    let before = registry::started_before(root, &launch);
    if !before && !registry::named(backend_id) {
        return false;
    }
    let Some(_looking) = Looking::begin(backend_id) else {
        return false;
    };
    let bound = if before { LOOK_BOUND } else { FIRST_LOOK_BOUND };
    let looked = look(root, backend_id, &launch, bound).await;
    keep_look(root, backend_id, looked)
}

/// Keeps what a look found. A failed look waits a day like a good one, and
/// the offer it had stays. Whether the offer or the sign-in changed.
fn keep_look(root: &Path, backend_id: &str, looked: Result<BackendOffer, Missed>) -> bool {
    let (offer, sign_in) = match looked {
        Ok(offer) => (offer, false),
        Err(Missed::SignIn) => (BackendOffer::default(), true),
        Err(Missed::Failed(error)) => {
            eprintln!("[acp] a look at {backend_id}'s models failed: {error}");
            (BackendOffer::default(), false)
        }
    };
    let mut seen = read(root);
    let entry = seen.entry(backend_id.to_string()).or_default();
    let was = (entry.offer(), entry.sign_in);
    let now = now_ms();
    entry.at = now;
    entry.looked_at = Some(now);
    entry.sign_in = sign_in;
    entry.take(&offer);
    entry.shape = SHAPE;
    let changed = was != (entry.offer(), entry.sign_in);
    write(root, &seen);
    changed
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
    // The look has closed the agent's input. An agent whose input ends
    // leaves by itself, and Claude Code then clears what it registered for
    // its run (`~/.claude/sessions/<pid>.json`, its socket in
    // `/tmp/cc-socks`), which a kill would leave behind; it is given a
    // moment to. One still running after it is ended with its whole group.
    let _ = tokio::time::timeout(LEAVE_BOUND, child.wait()).await;
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
            efforts: None,
            default_effort: None,
            fast: None,
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
            note: None,
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
        // Offered, the model it opened on has the levels it reported.
        let mut with_levels = offer;
        with_levels.models[0].efforts = Some(vec![choice("low"), choice("high")]);
        with_levels.models[0].default_effort = Some("high".into());
        assert_eq!(offer_of(root.path()), Some(with_levels));
        assert_eq!(super::offer(root.path(), "codex-acp"), None);
    }

    /// A catalogue read out of band is kept on disk beside the offer, so an
    /// agent never run here offers its models with their levels; a read
    /// that lists nothing keeps it and still counts as a read; and a
    /// session's offer, taken later, keeps it too.
    #[test]
    fn a_catalogue_is_kept_beside_the_offer_and_fills_an_agent_never_run_here() {
        let root = tempfile::tempdir().unwrap();
        let mut opus = choice("opus");
        opus.efforts = Some(vec![choice("default"), choice("max")]);
        opus.default_effort = Some("default".into());
        let catalog = Catalog {
            models: vec![opus.clone()],
            default_model: Some("opus".into()),
            effort_id: "effort".into(),
            effort_name: "Effort".into(),
            ..Catalog::default()
        };
        assert_eq!(catalog_at(root.path(), "claude-acp"), None);
        assert!(take_catalog(
            root.path(),
            "claude-acp",
            Some(catalog.clone())
        ));
        assert!(catalog_at(root.path(), "claude-acp").is_some());
        let listed = offer_of(root.path()).expect("the catalogue's offer");
        assert_eq!(listed.models, vec![opus.clone()]);
        assert_eq!(listed.current_model_id.as_deref(), Some("opus"));
        assert_eq!(listed.configs[0].id, "effort");
        assert_eq!(cached(root.path())["claude-acp"], vec![opus.clone()]);
        // The same list again changes nothing; none at all keeps it.
        assert!(!take_catalog(root.path(), "claude-acp", Some(catalog)));
        assert!(!take_catalog(root.path(), "claude-acp", None));
        assert!(!take_catalog(root.path(), "codex-acp", None));
        assert!(catalog_at(root.path(), "codex-acp").is_some());
        assert_eq!(super::offer(root.path(), "codex-acp"), None);
        // A session's report keeps it, and is first for its own model.
        remember(
            root.path(),
            "claude-acp",
            &offered(&[choice("opus"), choice("sonnet")]),
        );
        let listed = offer_of(root.path()).unwrap();
        assert_eq!(listed.models[0], opus);
        assert_eq!(listed.models[1].efforts, None);
    }

    /// A Grok session opened while its CLI renews its sign-in lists only its
    /// two fallback models; the session's list gains the catalogue's others,
    /// so the teammate's own model is still asked for. A session that lists
    /// none, or an agent with no catalogue, is left as it is.
    #[test]
    fn a_sessions_short_list_gains_the_catalogues_models() {
        let root = tempfile::tempdir().unwrap();
        let fallback = vec![choice("grok-4.6"), choice("grok-4.5")];
        assert_eq!(
            with_catalogue(root.path(), "grok-build", fallback.clone()),
            fallback
        );
        take_catalog(
            root.path(),
            "grok-build",
            Some(Catalog {
                models: vec![
                    choice("grok-4.7"),
                    choice("grok-4.7-build-fast"),
                    choice("grok-4.6"),
                    choice("grok-4.5"),
                ],
                effort_id: "reasoning_effort".into(),
                effort_name: "Reasoning Effort".into(),
                ..Catalog::default()
            }),
        );
        let listed: Vec<String> = with_catalogue(root.path(), "grok-build", fallback)
            .into_iter()
            .map(|model| model.id)
            .collect();
        assert_eq!(
            listed,
            ["grok-4.7", "grok-4.7-build-fast", "grok-4.6", "grok-4.5"]
        );
        assert_eq!(with_catalogue(root.path(), "grok-build", Vec::new()), []);
    }

    fn offer_of(root: &Path) -> Option<BackendOffer> {
        super::offer(root, "claude-acp")
    }

    /// The look offers the agent nothing, names no servers, reads the model
    /// picker and the modes its session opens with, and never prompts.
    #[tokio::test]
    async fn a_look_offers_nothing_and_reads_the_models_and_modes_without_a_prompt() {
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
                        let modes = acp::SessionModeState::new(
                            acp::SessionModeId::new("default"),
                            vec![
                                acp::SessionMode::new(
                                    acp::SessionModeId::new("default"),
                                    "Default",
                                ),
                                acp::SessionMode::new(
                                    acp::SessionModeId::new("bypassPermissions"),
                                    "Bypass Permissions",
                                ),
                            ],
                        );
                        responder.respond(
                            acp::NewSessionResponse::new(acp::SessionId::new("look"))
                                .modes(modes)
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
            offer
                .modes
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["default", "bypassPermissions"]
        );
        assert_eq!(offer.current_mode_id.as_deref(), Some("default"));
        // Kept, the agent offers its modes to a teammate at rest on it.
        let root = tempfile::tempdir().unwrap();
        assert!(keep_look(root.path(), "claude-acp", Ok(offer)));
        let kept = super::offer(root.path(), "claude-acp").expect("the offer is kept");
        assert_eq!(kept.modes.len(), 2);
        // Nothing but the two requests reached the agent: no prompt.
        assert_eq!(
            *heard.lock().unwrap(),
            ["initialize fs=false false", "session/new servers=0"]
        );
    }

    /// An agent is looked at when its list is a day old, and also when it
    /// has never offered modes here and was not looked at in the last day —
    /// a session that opened before modes were kept left a fresh list with
    /// none. A look that finds none waits a day like any other.
    #[test]
    fn an_agent_with_no_modes_is_due_a_look_once_a_day() {
        let now = now_ms();
        let hour = 60 * 60 * 1000;
        assert!(due(None, now));
        let listed = Seen {
            at: now - hour,
            models: vec![choice("opus")],
            shape: SHAPE,
            ..Seen::default()
        };
        assert!(due(Some(&listed), now), "a fresh list with no modes");
        let looked = Seen {
            looked_at: Some(now - hour),
            ..listed.clone()
        };
        assert!(!due(Some(&looked), now), "looked at an hour ago");
        let with_modes = Seen {
            modes: vec![choice("default")],
            ..listed
        };
        assert!(!due(Some(&with_modes), now));
        // An offer kept before it held all it holds now is looked at again.
        let older = Seen {
            shape: SHAPE - 1,
            ..with_modes.clone()
        };
        assert!(due(Some(&older), now), "kept before fast mode was");
        let stale = Seen {
            at: now - FRESH_MS,
            ..with_modes
        };
        assert!(due(Some(&stale), now));

        // A look that finds nothing new still counts, and changes nothing.
        let root = tempfile::tempdir().unwrap();
        remember(root.path(), "cursor", &offered(&[choice("auto")]));
        assert!(due(read(root.path()).get("cursor"), now_ms()));
        assert!(!keep_look(
            root.path(),
            "cursor",
            Ok(offered(&[choice("auto")]))
        ));
        assert!(!due(read(root.path()).get("cursor"), now_ms()));
        assert!(!keep_look(
            root.path(),
            "cursor",
            Err(Missed::Failed("gone".into()))
        ));
        assert_eq!(cached(root.path())["cursor"], vec![choice("auto")]);
        assert!(keep_look(root.path(), "cursor", Err(Missed::SignIn)));
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
