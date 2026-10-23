//! The funnel, driven by a scripted driver.
//!
//! Nothing here talks to a model. What is under test is the part that would
//! be written twice if the session did not own it: what a driver update means
//! on the tape, in what order, and what the room says about the session while
//! it happens.

use super::prompt::said;
use super::*;
use crate::contract::SkillPolicy;
use crate::contract::{
    AttachmentKind, ChapterStatus, DeltaKind, HumanAnswer, McpPolicy, PermissionOption,
    PersonaComputer, PolicyMode, ScheduledJob, SessionCheckpoint,
};
use crate::driver::DriverInfo;
use crate::mcp::server::TeammateTools;
use crate::test_support::until;
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::{Notify, Semaphore, mpsc, watch};

#[cfg(unix)]
mod capture;
mod cards;
mod chapters;
mod chat;
#[cfg(unix)]
mod computer;
mod posting;
mod preamble;
mod reading;
mod runs;
mod scheduled;
mod sent_files;
mod subagents;
mod turns;
mod voice;

/// A driver that says what it was told to say.
///
/// Each prompt replays the next script, one update at a time, waiting for
/// `gate` between updates when the test asked for a pause it can cancel in.
/// The last script stands for every turn after it, so a test that does not
/// care which turn it is in writes one.
pub(crate) struct Scripted {
    turns: Vec<Vec<Update>>,
    /// How many turns have been asked for, which is how the next script is
    /// chosen.
    asked: Arc<Mutex<usize>>,
    /// One permit lets one update out, so a turn can be caught in the middle.
    /// `None` runs the script straight through.
    gate: Option<Arc<Semaphore>>,
    /// What a cancel produces, in place of the rest of the script.
    on_cancel: Vec<Update>,
    cancelled: Arc<Notify>,
    prompts: Arc<Mutex<Vec<String>>>,
    attachments: Arc<Mutex<Vec<Vec<Attachment>>>>,
    reaches: Arc<Mutex<Vec<Reach>>>,
    /// The agent's own id for the conversation, when this script is standing
    /// in for a child that issues one.
    session_id: Arc<Mutex<Option<String>>>,
    /// Whether that child says it reopened the conversation it was handed.
    restored: Arc<Mutex<bool>>,
    /// Permission requests this driver is waiting on, by request id.
    waiting: Arc<Mutex<Vec<String>>>,
    /// How many times the room asked this driver to stop, which is how a
    /// reattach proves the old one was cancelled rather than left running.
    cancels: Arc<Mutex<usize>>,
    info_changes: Option<watch::Sender<DriverInfo>>,
    /// The models the room offers, when this script stands in for Hotline
    /// Agent: its picker is read again from here whenever the room says so.
    room_models: Option<Arc<Mutex<Vec<ConfigChoice>>>>,
    /// Work this agent starts by itself, for the room to take once.
    unprompted: Arc<Mutex<Option<mpsc::UnboundedReceiver<mpsc::Receiver<Update>>>>>,
    /// Accepted steering returned for replay when the activity ends.
    unconsumed: Option<Unconsumed>,
    /// Each line steered into a turn in flight, by its words.
    steered: Arc<Mutex<Vec<String>>>,
    /// Each prompt exactly as handed over, stamp and all.
    heard: Arc<Mutex<Vec<String>>>,
    /// Each retry, by its words, apart from the prompts it also counts as.
    retried: Arc<Mutex<Vec<String>>>,
    /// One permit lets one start finish, so a start can be caught while the
    /// harness is still coming up. `None` starts at once.
    start_gate: Option<Arc<Semaphore>>,
    /// Why every start is refused, for a harness that cannot come up.
    start_error: Option<String>,
}

type Unconsumed = Arc<Mutex<Vec<(String, Vec<Attachment>)>>>;

impl Scripted {
    pub(super) fn new(script: Vec<Update>) -> Self {
        Self::turns(vec![script])
    }

    pub(crate) fn turns(turns: Vec<Vec<Update>>) -> Self {
        Self {
            turns,
            asked: Arc::new(Mutex::new(0)),
            gate: None,
            on_cancel: Vec::new(),
            cancelled: Arc::new(Notify::new()),
            prompts: Arc::new(Mutex::new(Vec::new())),
            attachments: Arc::new(Mutex::new(Vec::new())),
            reaches: Arc::new(Mutex::new(Vec::new())),
            session_id: Arc::new(Mutex::new(None)),
            restored: Arc::new(Mutex::new(false)),
            waiting: Arc::new(Mutex::new(Vec::new())),
            cancels: Arc::new(Mutex::new(0)),
            info_changes: None,
            room_models: None,
            unprompted: Arc::new(Mutex::new(None)),
            unconsumed: None,
            steered: Arc::new(Mutex::new(Vec::new())),
            heard: Arc::new(Mutex::new(Vec::new())),
            retried: Arc::new(Mutex::new(Vec::new())),
            start_gate: None,
            start_error: None,
        }
    }

    /// An agent whose start-up waits for a permit, as a child's does while its
    /// process spawns and handshakes.
    pub(super) fn slow_to_start(mut self, gate: Arc<Semaphore>) -> Self {
        self.start_gate = Some(gate);
        self
    }

    /// An agent that cannot be started, and says why.
    pub(super) fn failing_to_start(mut self, reason: &str) -> Self {
        self.start_error = Some(reason.to_string());
        self
    }

    /// An agent that can start work by itself: each receiver sent is one such
    /// activity, as the ACP driver hands over what a harness says between turns.
    pub(super) fn with_unprompted(self) -> (Self, mpsc::UnboundedSender<mpsc::Receiver<Update>>) {
        let (sender, receiver) = mpsc::unbounded_channel();
        *lock(&self.unprompted) = Some(receiver);
        (self, sender)
    }

    /// What a cancel produces, in place of the rest of the script.
    pub(super) fn on_cancel(mut self, updates: Vec<Update>) -> Self {
        self.on_cancel = updates;
        self
    }

    pub(super) fn gated(mut self, gate: Arc<Semaphore>) -> Self {
        self.gate = Some(gate);
        self
    }

    /// An agent that takes lines into the turn in flight, as a driver with
    /// active input does.
    pub(super) fn steerable(mut self) -> Self {
        self.unconsumed = Some(Arc::new(Mutex::new(Vec::new())));
        self
    }

    pub(super) fn with_room_models(mut self) -> (Self, Arc<Mutex<Vec<ConfigChoice>>>) {
        let offered = Arc::new(Mutex::new(self.reported().models));
        self.room_models = Some(offered.clone());
        (self, offered)
    }

    fn reported(&self) -> DriverInfo {
        DriverInfo {
            agent_name: "Scripted".to_string(),
            models: vec![ConfigChoice {
                id: "anthropic/claude".to_string(),
                name: "Claude".to_string(),
                description: None,
                group: None,
            }],
            current_model_id: "anthropic/claude".to_string(),
            model_label: Some("Claude".to_string()),
            session_id: lock(&self.session_id).clone(),
            context_restored: *lock(&self.restored),
            ..DriverInfo::default()
        }
    }

    pub(super) fn with_info_changes(mut self) -> (Self, watch::Sender<DriverInfo>) {
        let (sender, _receiver) = watch::channel(DriverInfo::default());
        self.info_changes = Some(sender.clone());
        (self, sender)
    }

    /// The script for the turn now being asked for.
    fn next_script(&self) -> Vec<Update> {
        let mut asked = lock(&self.asked);
        let turn = *asked;
        *asked += 1;
        self.turns[turn.min(self.turns.len() - 1)].clone()
    }
}

#[async_trait]
impl Driver for Scripted {
    async fn start(&self, _persona: &Persona) -> Result<DriverInfo, String> {
        if let Some(gate) = &self.start_gate {
            gate.acquire().await.expect("the gate closed").forget();
        }
        if let Some(reason) = &self.start_error {
            return Err(reason.clone());
        }
        Ok(self.reported())
    }

    fn current_info(&self) -> Option<DriverInfo> {
        let offered = self.room_models.as_ref()?;
        let mut info = self.reported();
        info.models = lock(offered).clone();
        Some(info)
    }

    async fn retry(
        &self,
        text: String,
        attachments: Vec<Attachment>,
        reach: Reach,
    ) -> mpsc::Receiver<Update> {
        lock(&self.retried).push(words_of(&text));
        self.prompt(text, attachments, reach).await
    }

    async fn prompt(
        &self,
        text: String,
        attachments: Vec<Attachment>,
        reach: Reach,
    ) -> mpsc::Receiver<Update> {
        lock(&self.heard).push(text.clone());
        lock(&self.prompts).push(words_of(&text));
        lock(&self.attachments).push(attachments);
        lock(&self.reaches).push(reach);
        let (sender, receiver) = mpsc::channel(64);
        let script = self.next_script();
        let on_cancel = self.on_cancel.clone();
        let gate = self.gate.clone();
        let cancelled = self.cancelled.clone();
        tokio::spawn(async move {
            for update in script {
                if let Some(gate) = &gate {
                    let stop = cancelled.notified();
                    tokio::pin!(stop);
                    tokio::select! {
                        permit = gate.acquire() => { permit.expect("the gate closed").forget(); }
                        () = &mut stop => {
                            for update in on_cancel {
                                let _ = sender.send(update).await;
                            }
                            return;
                        }
                    }
                }
                if sender.send(update).await.is_err() {
                    return;
                }
            }
        });
        receiver
    }

    fn steer(&self, text: String, attachments: Vec<Attachment>) -> bool {
        let Some(unconsumed) = &self.unconsumed else {
            return false;
        };
        lock(&self.steered).push(words_of(&text));
        lock(unconsumed).push((text, attachments));
        true
    }

    fn take_unconsumed(&self) -> Vec<(String, Vec<Attachment>)> {
        self.unconsumed
            .as_ref()
            .map(|inputs| std::mem::take(&mut *lock(inputs)))
            .unwrap_or_default()
    }

    fn cancel(&self) {
        // A permit, not a wake: the test cancels between updates, and a wake
        // nobody is waiting for yet is a wake that never happened.
        *lock(&self.cancels) += 1;
        self.cancelled.notify_one();
    }

    async fn set_model(&self, model_id: &str) -> Result<DriverInfo, String> {
        let mut info = self.start(&persona("x")).await?;
        info.current_model_id = model_id.to_string();
        info.model_label = None;
        Ok(info)
    }

    fn subscribe_info(&self) -> Option<watch::Receiver<DriverInfo>> {
        self.info_changes
            .as_ref()
            .map(tokio::sync::watch::Sender::subscribe)
    }

    fn subscribe_unprompted(&self) -> Option<mpsc::UnboundedReceiver<mpsc::Receiver<Update>>> {
        lock(&self.unprompted).take()
    }

    /// A card is answerable exactly once, the way a live request is.
    fn answer_permission(&self, request_id: &str, _option_id: &str) -> bool {
        let mut waiting = lock(&self.waiting);
        let Some(at) = waiting.iter().position(|id| id == request_id) else {
            return false;
        };
        waiting.remove(at);
        true
    }
}

/// The words of a line a driver heard, once it is checked for the day and
/// time it was said: every line reaches a driver timed, so every test that
/// reads what a driver heard checks that, and reads the words.
pub(super) fn words_of(heard: &str) -> String {
    let (stamp, words) = heard
        .strip_prefix('[')
        .and_then(|rest| rest.split_once("] "))
        .unwrap_or_else(|| panic!("a line reached the driver untimed: {heard}"));
    let stamp = stamp
        .strip_suffix(" · desktop")
        .or_else(|| stamp.strip_suffix(" · phone"))
        .unwrap_or(stamp);
    assert!(
        chrono::NaiveDateTime::parse_from_str(stamp, "%a %d %b %Y, %H:%M").is_ok(),
        "not a local day and time: [{stamp}]"
    );
    words.to_string()
}

/// A conversation as a driver is seeded with it, each of the person's lines
/// checked for its time and read as its words.
pub(super) fn words(said: Vec<Said>) -> Vec<Said> {
    said.into_iter()
        .map(|line| match line {
            Said::User(heard) => Said::User(words_of(&heard)),
            agent @ Said::Agent(_) => agent,
        })
        .collect()
}

/// The models this room can reach, all of them scripted.
///
/// One driver for every session the room starts, so a rotation replays the
/// next turn of the same script; the preamble and the seeded conversation are
/// kept because they are what a fresh chapter's context is made of; and the
/// summariser gets whatever answer the test says a model gave.
pub(crate) struct Fake {
    driver: Arc<Scripted>,
    pub(super) preambles: Arc<Mutex<Vec<String>>>,
    pub(super) seeds: Arc<Mutex<Vec<Vec<Said>>>>,
    /// Each agent's view of its teammate and the tools it was handed, in the
    /// order the room asked for them.
    views: Arc<Mutex<Vec<Persona>>>,
    tools: Arc<Mutex<Vec<TeammateTools>>>,
    answer: Result<String, String>,
    /// When set, the summariser does not answer until this is notified.
    hold_note: Arc<tokio::sync::Notify>,
    holding: std::sync::atomic::AtomicBool,
}

impl Fake {
    #[cfg(unix)]
    pub(crate) fn cookie_heard(&self) -> String {
        format!(
            "{:?} {:?} {:?}",
            lock(&self.preambles),
            lock(&self.seeds),
            lock(&self.driver.prompts)
        )
    }

    pub(super) fn cancel_count(&self) -> usize {
        *lock(&self.driver.cancels)
    }

    /// Makes the agents stand in for a child that issues this session id, and
    /// says whether it reopens a conversation it is asked to.
    pub(super) fn reporting(&self, session_id: &str, restored: bool) {
        *lock(&self.driver.session_id) = Some(session_id.to_string());
        *lock(&self.driver.restored) = restored;
    }

    /// The tools each agent was handed, in order.
    pub(super) fn tools(&self) -> Vec<TeammateTools> {
        lock(&self.tools).clone()
    }

    /// Each teammate view an agent was built for, in order.
    pub(super) fn views(&self) -> Vec<Persona> {
        lock(&self.views).clone()
    }

    /// Makes the driver treat this permission request as one it is waiting on.
    pub(super) fn awaiting(&self, request_id: &str) {
        lock(&self.driver.waiting).push(request_id.to_string());
    }

    /// The permission requests the driver is still waiting on.
    pub(super) fn waiting(&self) -> Vec<String> {
        lock(&self.driver.waiting).clone()
    }

    /// Every line any driver in this room has been handed, in order.
    pub(super) fn prompts(&self) -> Vec<String> {
        lock(&self.driver.prompts).clone()
    }

    /// Every line steered into a turn in flight, in order.
    pub(super) fn steered(&self) -> Vec<String> {
        lock(&self.driver.steered).clone()
    }

    /// A room whose summariser is asked and refused, which is the shape of
    /// every desk with no model set up.
    pub(crate) fn new(driver: Scripted) -> Arc<Fake> {
        Fake::answering(driver, Err("no model answered".to_string()))
    }

    pub(super) fn answering(driver: Scripted, answer: Result<String, String>) -> Arc<Fake> {
        Arc::new(Fake {
            driver: Arc::new(driver),
            preambles: Arc::new(Mutex::new(Vec::new())),
            seeds: Arc::new(Mutex::new(Vec::new())),
            views: Arc::new(Mutex::new(Vec::new())),
            tools: Arc::new(Mutex::new(Vec::new())),
            answer,
            hold_note: Arc::new(tokio::sync::Notify::new()),
            holding: std::sync::atomic::AtomicBool::new(false),
        })
    }
}

#[async_trait]
impl Agents for Fake {
    fn agent(
        &self,
        persona: &Persona,
        preamble: String,
        said: Vec<Said>,
        tools: TeammateTools,
        _extra_mcp: Vec<crate::mcp::McpServer>,
    ) -> Result<Arc<dyn Driver>, String> {
        lock(&self.preambles).push(preamble);
        lock(&self.seeds).push(said);
        lock(&self.views).push(persona.clone());
        lock(&self.tools).push(tools);
        Ok(self.driver.clone())
    }

    async fn complete(
        &self,
        _model_id: &str,
        _system: &str,
        _prompt: &str,
    ) -> Result<String, String> {
        if self.holding.load(std::sync::atomic::Ordering::SeqCst) {
            self.hold_note.notified().await;
        }
        self.answer.clone()
    }
}

/// One provider key, so the room has a model to name the note's completion
/// with. Nothing here reaches a provider: every agent is a script.
pub(crate) struct DeskKeys;

impl ProviderKeys for DeskKeys {
    fn provider_auth(&self) -> HashMap<String, ProviderAuth> {
        HashMap::from([(
            "anthropic".to_string(),
            ProviderAuth::ApiKey("not-a-real-key".to_string()),
        )])
    }
}

/// The JSON a summariser answers with, as a model would write it.
pub(super) fn note_json(title: &str) -> Result<String, String> {
    Ok(format!(
        r#"{{"title": "{title}", "goal": "Get the crane moving", "outcome": "It moved.",
            "open_loops": ["oil the winch"], "decisions": [], "files": ["crane.log"],
            "tags": ["Crane", "harbour"], "status": "in-progress"}}"#
    ))
}

pub(super) fn scratch(name: &str) -> Log {
    let root = std::env::temp_dir().join(format!(
        "hotline-core-session-{name}-{}-{}",
        std::process::id(),
        now_ms()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    Log::open(root)
}

pub(crate) fn persona(id: &str) -> Persona {
    Persona {
        node: None,
        id: id.to_string(),
        name: "Ada".to_string(),
        goal: "Keep the harbour running.".to_string(),
        avatar: None,
        team: None,
        backend_id: "hotline".to_string(),
        cwd: std::env::temp_dir().to_string_lossy().to_string(),
        reach: Some(Reach::Machine),
        model_id: None,
        mode_id: None,
        effort_id: None,
        harness_override: None,
        hop_notice: None,
        mcp_policy: McpPolicy {
            mode: PolicyMode::All,
            server_ids: Vec::new(),
        },
        skill_policy: SkillPolicy::default(),
        // Existing scheduler fixtures represent a teammate that has been
        // granted persistent work; revocation tests turn this off explicitly.
        background_work: true,
        allowed_senders: Vec::new(),
        web_search_policy: None,
        folders: None,
        computer: None,
        voice: None,
        session_checkpoints: Vec::new(),
        last_session_id: None,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
    }
}

pub(crate) fn enrol(log: &Log, persona: &Persona) {
    let mut event = serde_json::to_value(persona).unwrap();
    event
        .as_object_mut()
        .unwrap()
        .insert("kind".into(), "persona".into());
    log.append(&StreamId::Room, &event).unwrap();
}

/// A room with one teammate enrolled, on scripted agents, no session started.
/// A phone paired with the desk, so a push has somewhere to go.
pub(crate) fn pair_a_phone(room: &Room) {
    std::fs::write(
        room.log.root().join("remote.json"),
        json!({
            "desktopId": "desk-1",
            "host": "desk.local",
            "enabled": true,
            "grants": [{
                "device": {"id": "phone-1", "name": "Phone", "pairedAt": 0, "publicKey": "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE"},
                "push": {"token": "ExponentPushToken[phone]", "platform": "ios"}
            }]
        })
        .to_string(),
    )
    .unwrap();
}

fn room(name: &str, agents: Arc<Fake>) -> Arc<Room> {
    let log = scratch(name);
    enrol(&log, &persona("ada"));
    // No runtime and no releases endpoint: nothing here reaches the network.
    Room::with_agents_and_computers(
        log,
        Arc::new(DeskKeys),
        agents,
        crate::computer::Computer::with_path(std::env::temp_dir().join("no-runtime")),
    )
}

/// The tape as a client is sent it: a thread's link is the marker its kind has
/// always had. What is stored is checked where it is the point.
fn tape(room: &Room, persona_id: &str) -> Vec<Value> {
    room.log
        .load(&StreamId::Tape(persona_id.to_string()))
        .into_iter()
        .map(crate::thread::Link::wire)
        .collect()
}

/// The chapter markers on the tape, oldest first.
fn markers(room: &Room, persona_id: &str) -> Vec<Value> {
    tape(room, persona_id)
        .into_iter()
        .filter(|event| event["kind"] == "chapter")
        .collect()
}

/// The event kinds on the tape, in the order they were written.
fn kinds(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .map(|event| event["kind"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// Waits for the conversation to hold `count` events, so a test never races
/// the task the turn runs on.
///
/// Chapter markers are left out: every session opens one, and a test about
/// what a turn writes should not have to count the room's bookkeeping. The
/// tests that are about chapters read [`markers`] instead.
async fn settled(room: &Room, persona_id: &str, count: usize) -> Vec<Value> {
    let events = || -> Vec<Value> {
        tape(room, persona_id)
            .into_iter()
            .filter(|event| event["kind"] != "chapter")
            .collect()
    };
    until(&format!("the tape reaches {count} events"), || {
        events().len() >= count
    })
    .await;
    events()
}

fn spoken_turn() -> Vec<Update> {
    vec![
        Update::Delta {
            kind: MessageKind::Thought,
            message_id: "m1".to_string(),
            text: "let me look".to_string(),
        },
        Update::Message {
            kind: MessageKind::Thought,
            id: "m1".to_string(),
            text: "let me look".to_string(),
        },
        Update::ToolCall {
            call_id: "c1".to_string(),
            title: "ls .".to_string(),
            kind: "ls".to_string(),
        },
        Update::ToolResult {
            call_id: "c1".to_string(),
            ok: true,
            output: "AGENTS.md".to_string(),
            images: Vec::new(),
        },
        Update::Delta {
            kind: MessageKind::Agent,
            message_id: "m2".to_string(),
            text: "one file".to_string(),
        },
        Update::Message {
            kind: MessageKind::Agent,
            id: "m2".to_string(),
            text: "one file".to_string(),
        },
        Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        },
    ]
}

/// A turn in which the agent says one thing. `tag` keeps the message id of one
/// turn apart from the next, because a tape folds by id.
fn saying(tag: &str, text: &str) -> Vec<Update> {
    vec![
        Update::Delta {
            kind: MessageKind::Agent,
            message_id: format!("m-{tag}"),
            text: text.to_string(),
        },
        Update::Message {
            kind: MessageKind::Agent,
            id: format!("m-{tag}"),
            text: text.to_string(),
        },
        Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        },
    ]
}

/// Waits for the driver to have been handed `count` lines, so a test never
/// races the task the turn runs on.
async fn heard(prompts: &Arc<Mutex<Vec<String>>>, count: usize) -> Vec<String> {
    until(&format!("the driver is handed {count} lines"), || {
        lock(prompts).len() >= count
    })
    .await;
    lock(prompts).clone()
}

/// A tape event, for a test that needs a conversation older than this run.
fn spoken(kind: &str, id: &str, ts: i64, text: &str) -> Value {
    json!({"kind": kind, "id": id, "ts": ts, "text": text})
}

fn write_tape(log: &Log, persona_id: &str, events: &[Value]) {
    for event in events {
        log.append(&StreamId::Tape(persona_id.to_string()), event)
            .unwrap();
    }
}

async fn until_state(room: &Room, persona_id: &str, want: SessionState) {
    until(&format!("the session reaches {want:?}"), || {
        room.info(persona_id).state == want
    })
    .await;
}

/// A teammate on a computer, the way one is started in a test: a scripted
/// runtime that plays `docker` and reports the port a fake computer serves
/// on. Answers the room, the fake agents (whose preambles say what the
/// teammate heard) and the workspace.
#[cfg(unix)]
pub(crate) struct ComputerRoom {
    pub(crate) room: Arc<Room>,
    pub(crate) agents: Arc<Fake>,
    root: std::path::PathBuf,
    cwd: std::path::PathBuf,
    /// How often the releases endpoint was asked.
    asked: Arc<std::sync::atomic::AtomicUsize>,
    /// Every set of secrets the fake computer was handed, in order.
    pub(crate) taken: crate::computer::guide::fake::Taken,
}

/// `release` is what the fake computer claims to be (`None`: too old to
/// say); `releases` is the body the releases endpoint answers; `pinned`
/// gives the teammate its own image instead of the desk's choice.
#[cfg(unix)]
async fn computer_room(
    name: &str,
    release: Option<&str>,
    releases: &'static str,
    pinned: bool,
) -> ComputerRoom {
    use crate::computer::{Computer, fixtures, guide, releases as published};
    let root = fixtures::scratch(name);
    let cwd = root.join("work");
    std::fs::create_dir_all(&cwd).unwrap();
    let (port, taken) = guide::fake::serve_taking(release).await;
    fixtures::fake_runtime(&root, port);
    std::fs::write(root.join("state"), "absent").unwrap();
    let (url, asked) = published::fake::serve(releases).await;
    let log = scratch(name);
    // A vault of the test's own, so a granted secret has somewhere to be
    // read from on its way to the machine.
    let vault = Arc::new(
        Vault::open_with_store(
            log.root(),
            log.clone(),
            Arc::new(crate::credentials::MemoryStore::default()),
        )
        .unwrap(),
    );
    let mut ada = persona("ada");
    ada.cwd = cwd.to_string_lossy().into_owned();
    ada.computer = Some(PersonaComputer {
        cpus: None,
        enabled: true,
        image: pinned.then(|| "hotline-computer:test".to_string()),
        memory: None,
        pids: None,
        mounts: None,
        secrets: None,
    });
    enrol(&log, &ada);
    let agents = Fake::new(Scripted::new(Vec::new()));
    let room = Room::with_agents_computers_and_vault(
        log,
        Arc::new(DeskKeys),
        agents.clone(),
        Computer::with_path(root.as_os_str()).with_releases(&url),
        Some(vault),
    );
    ComputerRoom {
        room,
        agents,
        root,
        cwd,
        asked,
        taken,
    }
}

#[cfg(unix)]
const TWO_RELEASES: &str = r#"[{"tag_name":"v0.10.4"},{"tag_name":"v0.10.3"}]"#;

/// The notices on a teammate's tape, oldest first.
fn notices(room: &Room, persona_id: &str) -> Vec<String> {
    tape(room, persona_id)
        .iter()
        .filter(|event| event["kind"] == "notice")
        .map(|event| event["text"].as_str().unwrap().to_string())
        .collect()
}

#[cfg(unix)]
pub(crate) async fn secret_use_room(
    name: &str,
    release: &str,
    usage: crate::contract::SecretUse,
) -> ComputerRoom {
    let fixture = computer_room(name, Some(release), TWO_RELEASES, true).await;
    let vault = fixture.room.vault.as_ref().unwrap();
    for name in ["GH_PASSKEY", "OTHER_KEY"] {
        vault
            .set_shared(
                name,
                crate::vault::StoredSecret::Passkey {
                    r#use: usage,
                    rp_id: "github.com".into(),
                    credential_id: "aGVsbG8=".into(),
                    private_key: crate::computer::guide::fake::KEY_BASE64.into(),
                    user_handle: None,
                    user_name: None,
                    user_display_name: None,
                },
            )
            .unwrap();
    }
    vault
        .set_shared(
            "HN_LOGIN",
            crate::vault::StoredSecret::Login {
                r#use: usage,
                sites: vec!["https://news.ycombinator.com".into()],
                username: "person".into(),
                password: "private-password".into(),
                totp: None,
            },
        )
        .unwrap();
    let mut persona = fixture.room.persona("ada").unwrap();
    persona.computer.as_mut().unwrap().secrets = Some(vec![
        "GH_PASSKEY".into(),
        "OTHER_KEY".into(),
        "HN_LOGIN".into(),
    ]);
    enrol(&fixture.room.log, &persona);
    fixture.room.start("ada").await.unwrap();
    fixture
}

#[cfg(unix)]
pub(crate) async fn cookie_offer_room(
    name: &str,
    firefox: bool,
) -> (ComputerRoom, tempfile::TempDir) {
    let profiles = tempfile::tempdir().unwrap();
    let candidate =
        crate::computer::cookies::lending::tests::candidate_fixture(profiles.path(), firefox);
    let fixture = cookie_offer_room_with_candidates(name, vec![candidate]).await;
    (fixture, profiles)
}

#[cfg(unix)]
pub(crate) async fn cookie_offer_room_with_candidates(
    name: &str,
    candidates: Vec<crate::computer::cookies::Candidate>,
) -> ComputerRoom {
    let fixture = computer_room(name, Some("0.11.2"), TWO_RELEASES, true).await;
    *lock(&fixture.room.cookie_candidates) = candidates;
    fixture.room.start("ada").await.unwrap();
    *fixture.taken.pages.lock().unwrap() = vec![
        json!({"origin":"https://porkbun.com"}),
        json!({"origin":"https://account.porkbun.com"}),
        json!({"origin":"https://no-cookies.example"}),
    ];
    fixture
}
