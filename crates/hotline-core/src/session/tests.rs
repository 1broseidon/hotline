//! The funnel, driven by a scripted driver.
//!
//! Nothing here talks to a model. What is under test is the part that would
//! be written twice if the session did not own it: what a driver update means
//! on the tape, in what order, and what the room says about the session while
//! it happens.

use super::*;
use crate::contract::{
    AttachmentKind, ChapterStatus, HumanAnswer, McpPolicy, PermissionOption, PersonaComputer,
    PolicyMode, ScheduledJob, SessionCheckpoint,
};
use crate::driver::DriverInfo;
use crate::mcp::server::TeammateTools;
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::{Notify, Semaphore, mpsc, watch};

/// A driver that says what it was told to say.
///
/// Each prompt replays the next script, one update at a time, waiting for
/// `gate` between updates when the test asked for a pause it can cancel in.
/// The last script stands for every turn after it, so a test that does not
/// care which turn it is in writes one.
pub(super) struct Scripted {
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
    /// Permission requests this driver is waiting on, by request id.
    waiting: Arc<Mutex<Vec<String>>>,
    /// How many times the room asked this driver to stop, which is how a
    /// reattach proves the old one was cancelled rather than left running.
    cancels: Arc<Mutex<usize>>,
    info_changes: Option<watch::Sender<DriverInfo>>,
}

impl Scripted {
    pub(super) fn new(script: Vec<Update>) -> Self {
        Self::turns(vec![script])
    }

    pub(super) fn turns(turns: Vec<Vec<Update>>) -> Self {
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
            waiting: Arc::new(Mutex::new(Vec::new())),
            cancels: Arc::new(Mutex::new(0)),
            info_changes: None,
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
        Ok(DriverInfo {
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
            ..DriverInfo::default()
        })
    }

    async fn prompt(
        &self,
        text: String,
        attachments: Vec<Attachment>,
        reach: Reach,
    ) -> mpsc::Receiver<Update> {
        lock(&self.prompts).push(text);
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
                        _ = &mut stop => {
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
            .map(|changes| changes.subscribe())
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

/// The models this room can reach, all of them scripted.
///
/// One driver for every session the room starts, so a rotation replays the
/// next turn of the same script; the preamble and the seeded conversation are
/// kept because they are what a fresh chapter's context is made of; and the
/// summariser gets whatever answer the test says a model gave.
pub(super) struct Fake {
    driver: Arc<Scripted>,
    preambles: Arc<Mutex<Vec<String>>>,
    seeds: Arc<Mutex<Vec<Vec<Said>>>>,
    answer: Result<String, String>,
}

impl Fake {
    pub(super) fn cancel_count(&self) -> usize {
        *lock(&self.driver.cancels)
    }

    /// A room whose summariser is asked and refused, which is the shape of
    /// every desk with no model set up.
    pub(super) fn new(driver: Scripted) -> Arc<Fake> {
        Fake::answering(driver, Err("no model answered".to_string()))
    }

    pub(super) fn answering(driver: Scripted, answer: Result<String, String>) -> Arc<Fake> {
        Arc::new(Fake {
            driver: Arc::new(driver),
            preambles: Arc::new(Mutex::new(Vec::new())),
            seeds: Arc::new(Mutex::new(Vec::new())),
            answer,
        })
    }
}

#[async_trait]
impl Agents for Fake {
    fn agent(
        &self,
        _persona: &Persona,
        preamble: String,
        said: Vec<Said>,
        _tools: TeammateTools,
        _extra_mcp: Vec<crate::mcp::McpServer>,
    ) -> Result<Arc<dyn Driver>, String> {
        lock(&self.preambles).push(preamble);
        lock(&self.seeds).push(said);
        Ok(self.driver.clone())
    }

    async fn complete(
        &self,
        _model_id: &str,
        _system: &str,
        _prompt: &str,
    ) -> Result<String, String> {
        self.answer.clone()
    }
}

/// One provider key, so the room has a model to name the note's completion
/// with. Nothing here reaches a provider: every agent is a script.
pub(super) struct DeskKeys;

impl ProviderKeys for DeskKeys {
    fn provider_auth(&self) -> HashMap<String, ProviderAuth> {
        HashMap::from([(
            "anthropic".to_string(),
            ProviderAuth::ApiKey("not-a-real-key".to_string()),
        )])
    }
}

/// The JSON a summariser answers with, as a model would write it.
fn note_json(title: &str) -> Result<String, String> {
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

pub(super) fn persona(id: &str) -> Persona {
    Persona {
        node: None,
        id: id.to_string(),
        name: "Ada".to_string(),
        goal: "Keep the harbour running.".to_string(),
        face: None,
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
        skill_policy: Default::default(),
        // Existing scheduler fixtures represent a teammate that has been
        // granted persistent work; revocation tests turn this off explicitly.
        background_work: true,
        allowed_senders: Vec::new(),
        web_search_policy: None,
        computer: None,
        subagents: None,
        session_checkpoints: Vec::new(),
        last_session_id: None,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
    }
}

pub(super) fn enrol(log: &Log, persona: &Persona) {
    let mut event = serde_json::to_value(persona).unwrap();
    event
        .as_object_mut()
        .unwrap()
        .insert("kind".into(), "persona".into());
    log.append(&StreamId::Room, &event).unwrap();
}

/// A room with one teammate enrolled, on scripted agents, no session started.
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

fn tape(room: &Room, persona_id: &str) -> Vec<Value> {
    room.log.load(&StreamId::Tape(persona_id.to_string()))
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
    for _ in 0..200 {
        let events: Vec<Value> = tape(room, persona_id)
            .into_iter()
            .filter(|event| event["kind"] != "chapter")
            .collect();
        if events.len() >= count {
            return events;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the tape never reached {count} events");
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

#[tokio::test]
async fn the_users_line_is_on_the_tape_first_and_every_update_lands_behind_it() {
    let room = room("a-turn", Fake::new(Scripted::new(spoken_turn())));
    let mut infos = room.subscribe_info();
    let mut deltas = room.subscribe_deltas();
    room.start("ada").await.unwrap();

    room.prompt("ada", "what is here?", None, None)
        .await
        .unwrap();
    let events = settled(&room, "ada", 5).await;

    assert_eq!(
        kinds(&events),
        ["user", "thought", "tool", "agent", "turn"],
        "the tool call and its result are one event, superseded by id"
    );
    assert_eq!(events[0]["text"], "what is here?");
    assert_eq!(events[1]["text"], "let me look");
    assert_eq!(events[2]["toolCallId"], "c1");
    assert_eq!(events[2]["status"], "completed");
    assert_eq!(
        events[2]["output"],
        json!([{"type": "text", "text": "AGENTS.md"}])
    );
    assert_eq!(events[3]["text"], "one file");
    assert_eq!(events[4]["stopReason"], "end_turn");

    // The state the roster shows: ready when it started, thinking while the
    // turn ran, ready again when it was over.
    let mut states = Vec::new();
    for _ in 0..3 {
        states.push(infos.recv().await.unwrap().state);
    }
    assert_eq!(
        states,
        [
            SessionState::Ready,
            SessionState::Thinking,
            SessionState::Ready
        ]
    );

    // The deltas went out and were never written down. The answer came
    // after a tool call, so it streamed as thinking and landed whole: a
    // message after the work may turn out to be narration, and the window
    // must never type a bubble that then vanishes.
    let mut streamed = Vec::new();
    while let Ok(delta) = deltas.try_recv() {
        streamed.push(delta);
    }
    assert_eq!(
        streamed,
        [
            StreamDelta::ThoughtDelta {
                persona_id: "ada".to_string(),
                message_id: "m1".to_string(),
                text: "let me look".to_string(),
            },
            StreamDelta::ThoughtDelta {
                persona_id: "ada".to_string(),
                message_id: "m2".to_string(),
                text: "one file".to_string(),
            },
        ]
    );
}

/// The person's line is `sent` the moment it is written and `read` once the
/// agent has produced anything with it in context — not when the driver took
/// it, because a prompt the model never got to would then wear a read tick.
#[tokio::test]
async fn a_line_is_sent_when_written_and_read_once_the_agent_answers_to_it() {
    let gate = Arc::new(Semaphore::new(0));
    let mut driver = Scripted::new(vec![Update::Turn {
        stop_reason: "end_turn".to_string(),
        usage: None,
    }]);
    driver.gate = Some(gate.clone());
    let room = room("receipts", Fake::new(driver));
    room.start("ada").await.unwrap();

    room.prompt("ada", "first", None, None).await.unwrap();
    room.prompt("ada", "second", None, None).await.unwrap();
    let events = settled(&room, "ada", 2).await;
    assert_eq!(kinds(&events), ["user", "user"]);
    assert_eq!(events[0]["receipt"], "sent", "taken, not yet proven read");
    assert_eq!(
        events[1]["receipt"], "sent",
        "still waiting behind the turn"
    );

    gate.add_permits(1);
    let events = settled(&room, "ada", 3).await;
    assert_eq!(kinds(&events), ["user", "user", "turn"]);
    assert_eq!(events[0]["receipt"], "read", "the turn is proof");
    assert_eq!(events[1]["receipt"], "sent", "its own turn has not begun");

    gate.add_permits(1);
    let events = settled(&room, "ada", 4).await;
    assert_eq!(events[1]["receipt"], "read");
    assert_eq!(events[0]["receipt"], "read", "nothing un-reads");
}

/// A cancelled line was taken but never answered; it stays sent.
#[tokio::test]
async fn a_cancelled_turn_leaves_its_line_sent() {
    let gate = Arc::new(Semaphore::new(0));
    let mut driver = Scripted::new(vec![Update::Turn {
        stop_reason: "end_turn".to_string(),
        usage: None,
    }]);
    driver.gate = Some(gate.clone());
    driver.on_cancel = vec![Update::Turn {
        stop_reason: "cancelled".to_string(),
        usage: None,
    }];
    let agents = Fake::new(driver);
    let prompts = agents.driver.prompts.clone();
    let room = room("receipts-cancel", agents);
    room.start("ada").await.unwrap();
    room.prompt("ada", "first", None, None).await.unwrap();
    // The driver has the line and is mid-turn when the person stops it.
    for _ in 0..200 {
        if !lock(&prompts).is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(*lock(&prompts), ["first"]);
    room.cancel("ada").unwrap();
    let events = settled(&room, "ada", 2).await;
    assert_eq!(kinds(&events), ["user", "turn"]);
    assert_eq!(events[0]["receipt"], "sent");
}

/// The agent's reaction lands on the person's last message and nowhere else.
#[tokio::test]
async fn a_reaction_lands_on_the_last_thing_the_person_said() {
    let room = room("react", Fake::new(Scripted::new(spoken_turn())));
    room.start("ada").await.unwrap();
    room.prompt("ada", "ship it", None, None).await.unwrap();
    settled(&room, "ada", 5).await;
    let tools = TeammateTools::new(&room, "ada");
    assert_eq!(
        tools
            .call("react", &json!({ "emoji": "👍" }))
            .await
            .unwrap(),
        "Reacted."
    );
    tools
        .call("react", &json!({ "emoji": "👍" }))
        .await
        .unwrap();
    tools
        .call("react", &json!({ "emoji": "🙏" }))
        .await
        .unwrap();
    assert!(
        tools
            .call("react", &json!({ "emoji": "ok" }))
            .await
            .is_err()
    );
    let events = settled(&room, "ada", 5).await;
    assert_eq!(kinds(&events), ["user", "thought", "tool", "agent", "turn"]);
    assert_eq!(events[0]["reactions"], json!(["👍", "🙏"]));
    assert_eq!(events[0]["receipt"], "read", "a reaction keeps the receipt");
}

#[tokio::test]
async fn a_computer_tool_image_lands_a_frame_on_the_tape() {
    let png = "AAAA";
    let image = crate::driver::ToolImage {
        data: png.into(),
        mime_type: "image/png".into(),
    };
    let room = room(
        "computer-frame",
        Fake::new(Scripted::new(vec![
            Update::ToolCall {
                call_id: "c1".to_string(),
                title: "computer__capture".to_string(),
                kind: "computer__capture".to_string(),
            },
            Update::ToolResult {
                call_id: "c1".to_string(),
                ok: true,
                output: format!("the tree\n{}", image.placeholder()),
                images: vec![image],
            },
            Update::Turn {
                stop_reason: "end_turn".to_string(),
                usage: None,
            },
        ])),
    );
    room.start("ada").await.unwrap();
    room.prompt("ada", "look", None, None).await.unwrap();
    let events = settled(&room, "ada", 4).await;
    assert_eq!(
        kinds(&events),
        ["user", "tool", "computer_frame", "turn"],
        "the frame lands immediately after the completed tool"
    );
    assert_eq!(events[1]["status"], "completed");
    let data_url = events[2]["dataUrl"]
        .as_str()
        .expect("a frame carries a data URL");
    assert!(data_url.starts_with("data:image/png;base64,"), "{data_url}");
    assert!(data_url.ends_with(png), "{data_url}");
}

#[tokio::test]
async fn another_servers_image_is_a_placeholder_not_a_frame() {
    let image = crate::driver::ToolImage {
        data: "AAAA".into(),
        mime_type: "image/png".into(),
    };
    let room = room(
        "other-frame",
        Fake::new(Scripted::new(vec![
            Update::ToolCall {
                call_id: "c1".to_string(),
                title: "echo__shout".to_string(),
                kind: "echo__shout".to_string(),
            },
            Update::ToolResult {
                call_id: "c1".to_string(),
                ok: true,
                output: image.placeholder(),
                images: vec![image],
            },
            Update::Turn {
                stop_reason: "end_turn".to_string(),
                usage: None,
            },
        ])),
    );
    room.start("ada").await.unwrap();
    room.prompt("ada", "look", None, None).await.unwrap();
    let events = settled(&room, "ada", 3).await;
    assert_eq!(kinds(&events), ["user", "tool", "turn"]);
    assert!(
        events[1]["output"][0]["text"]
            .as_str()
            .unwrap()
            .contains("[image image/png"),
        "{:?}",
        events[1]["output"]
    );
}

#[tokio::test]
async fn a_cancelled_turn_fails_the_tool_it_caught_in_flight() {
    let gate = Arc::new(Semaphore::new(0));
    let mut driver = Scripted::new(vec![
        Update::ToolCall {
            call_id: "c1".to_string(),
            title: "shell sleep 600".to_string(),
            kind: "shell".to_string(),
        },
        Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        },
    ]);
    driver.gate = Some(gate.clone());
    driver.on_cancel = vec![Update::Turn {
        stop_reason: "aborted".to_string(),
        usage: None,
    }];
    let room = room("cancel", Fake::new(driver));
    room.start("ada").await.unwrap();

    room.prompt("ada", "run the long thing", None, None)
        .await
        .unwrap();
    gate.add_permits(1);
    let events = settled(&room, "ada", 2).await;
    assert_eq!(events[1]["status"], "in_progress");

    room.cancel("ada").unwrap();
    let events = settled(&room, "ada", 3).await;
    assert_eq!(kinds(&events), ["user", "tool", "turn"]);
    assert_eq!(
        events[1]["status"], "failed",
        "a tool the turn never heard back about did not complete"
    );
    assert_eq!(events[2]["stopReason"], "aborted");
}

#[tokio::test]
async fn a_prompt_during_a_turn_waits_for_the_turn_it_would_have_interrupted() {
    let gate = Arc::new(Semaphore::new(0));
    let mut driver = Scripted::new(vec![Update::Turn {
        stop_reason: "end_turn".to_string(),
        usage: None,
    }]);
    driver.gate = Some(gate.clone());
    let agents = Fake::new(driver);
    let prompts = agents.driver.prompts.clone();
    let reaches = agents.driver.reaches.clone();
    let room = room("queue", agents);
    room.start("ada").await.unwrap();

    room.prompt("ada", "first", None, None).await.unwrap();
    room.prompt("ada", "second", None, None).await.unwrap();

    // Both lines are on the tape at once: what was said is a fact as soon as
    // it was said, whatever the agent is busy with.
    let events = settled(&room, "ada", 2).await;
    assert_eq!(kinds(&events), ["user", "user"]);

    // One permit per turn: the second turn cannot have started until the
    // first one's ended, because the driver only ever ran one script at once.
    gate.add_permits(1);
    let events = settled(&room, "ada", 3).await;
    assert_eq!(kinds(&events), ["user", "user", "turn"]);
    gate.add_permits(1);
    let events = settled(&room, "ada", 4).await;
    assert_eq!(kinds(&events), ["user", "user", "turn", "turn"]);
    assert_eq!(*lock(&prompts), ["first", "second"]);
    // Reach is read from the roster at every prompt, not from the persona the
    // session was started with.
    assert_eq!(*lock(&reaches), [Reach::Machine, Reach::Machine]);
}

/// Every line the room hands the driver is a line the driver hears.
///
/// The claim on the driver and the queue behind it are one fact. Held apart,
/// there was a moment — between the driver finding nothing waiting and letting
/// go of the turn — in which a line was filed behind a turn that was already
/// over. The teammate then sat on it: it took the next thing said to shake it
/// loose, and if nothing else was said, nothing ever did.
///
/// That is why nothing is said after the hunt below: another line would heal
/// exactly the fault being looked for. The moment is nanoseconds wide, so this
/// hunts rather than hopes — each line is said the instant the driver takes
/// the one before it, and the spin after that walks the saying across the tail
/// of the turn. The hunting lines are nudges, because a nudge is the cheapest
/// thing the room can dispatch and a hunt needs thousands of shots; the tape's
/// own line is counted with them.
#[tokio::test(flavor = "multi_thread")]
async fn every_line_the_room_hands_the_driver_is_a_line_the_driver_heard() {
    /// How many shots the hunt takes, and how far past the turn's end the
    /// saying of each one is walked. A spin is tens of nanoseconds and a turn
    /// here is tens of microseconds, so the walk covers the whole of it.
    const SHOTS: usize = 60_000;
    const WALK: usize = 2_000;
    /// How long a line may wait for an idle driver before it is plainly stuck.
    const STUCK: Duration = Duration::from_secs(2);
    const TICK: &str = "tick";

    let agents = Fake::new(Scripted::new(Vec::new()));
    let prompts = agents.driver.prompts.clone();
    let room = room("tight-loop", agents);
    room.start("ada").await.unwrap();
    room.prompt("ada", "first", None, None).await.unwrap();

    let hunting = {
        let room = room.clone();
        let prompts = prompts.clone();
        tokio::task::spawn_blocking(move || {
            let mut said = 0;
            for shot in 0..SHOTS {
                let taken = lock(&prompts).len();
                room.nudge("ada", TICK).unwrap();
                said += 1;
                let waited = std::time::Instant::now();
                while lock(&prompts).len() == taken {
                    if waited.elapsed() > STUCK {
                        return said;
                    }
                    std::hint::spin_loop();
                }
                for _ in 0..(shot % WALK) {
                    std::hint::spin_loop();
                }
            }
            said
        })
    };
    let said = hunting.await.unwrap();

    let heard = lock(&prompts).clone();
    let on_the_tape: Vec<String> = tape(&room, "ada")
        .iter()
        .filter(|event| event["kind"] == "user")
        .map(|event| event["text"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(on_the_tape, ["first"]);
    assert_eq!(
        heard
            .iter()
            .filter(|line| *line != TICK)
            .collect::<Vec<_>>(),
        on_the_tape.iter().collect::<Vec<_>>(),
        "a line on the tape never reached the driver"
    );
    assert_eq!(
        heard.iter().filter(|line| *line == TICK).count(),
        said,
        "the room handed the driver {said} lines and it heard fewer: one of \
         them is waiting on a turn that ended without it"
    );
}

/// Two callers bring a teammate up at the same moment — the wire and a
/// schedule firing are exactly these two — and between them there is one
/// session in one chapter.
///
/// Unserialised, both read a tape with no chapter open and both write a marker
/// onto it, and the teammate is left holding a chapter that will never close.
/// The window is the width of one file read, so a handful of teammates raced
/// in turn is enough to land in it.
#[tokio::test(flavor = "multi_thread")]
async fn two_starts_at_once_leave_one_session_in_one_chapter() {
    const TEAMMATES: usize = 6;
    let log = scratch("start-race");
    for teammate in 0..TEAMMATES {
        enrol(&log, &persona(&format!("ada{teammate}")));
    }
    let agents = Fake::new(Scripted::new(Vec::new()));
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());

    for teammate in 0..TEAMMATES {
        let id = format!("ada{teammate}");
        let together = Arc::new(std::sync::Barrier::new(2));
        let mut starting = Vec::new();
        for _ in 0..2 {
            let room = room.clone();
            let together = together.clone();
            let id = id.clone();
            starting.push(tokio::spawn(async move {
                together.wait();
                room.start(&id).await
            }));
        }
        for start in starting {
            start.await.unwrap().unwrap();
        }
        assert_eq!(
            markers(&room, &id).len(),
            1,
            "one start opened {id}'s chapter and the other joined it"
        );
        assert_eq!(room.info(&id).state, SessionState::Ready);
    }
    assert_eq!(
        lock(&agents.preambles).len(),
        TEAMMATES,
        "a second start on a teammate that was up built a second agent"
    );
}

/// A deleted teammate is forgotten whole: the agent is stopped and the
/// start gate is gone, so nothing is kept for an id that names nobody.
#[tokio::test]
async fn forgetting_a_teammate_stops_it_and_drops_its_gate() {
    let room = room("forget", Fake::new(Scripted::new(Vec::new())));
    room.start("ada").await.unwrap();
    assert!(lock(&room.starts).contains_key("ada"));

    room.forget("ada");

    assert!(
        lock(&room.sessions).get("ada").is_none(),
        "the agent kept running"
    );
    assert!(!lock(&room.starts).contains_key("ada"), "the gate was kept");
}

/// Two messages arriving on a closed chapter open one chapter between them,
/// and both are spoken to the session that chapter belongs to.
///
/// The gate is stop-then-start. Performed twice over, the second stop cancels
/// the session the first had just brought up, and the line behind it is said
/// to an agent that is no longer there. A handful of teammates raced in turn,
/// because the window is the width of one file read.
#[tokio::test(flavor = "multi_thread")]
async fn two_prompts_on_a_closed_chapter_open_one_and_both_are_heard() {
    const TEAMMATES: usize = 5;
    let log = scratch("chapter-gate-race");
    for teammate in 0..TEAMMATES {
        enrol(&log, &persona(&format!("ada{teammate}")));
    }
    let agents = Fake::new(Scripted::new(Vec::new()));
    let prompts = agents.driver.prompts.clone();
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents);

    for teammate in 0..TEAMMATES {
        let id = format!("ada{teammate}");
        room.start(&id).await.unwrap();
        room.start_fresh_chapter(&id, ChapterClose::User)
            .await
            .unwrap();
        lock(&prompts).clear();

        let together = Arc::new(std::sync::Barrier::new(2));
        let mut speaking = Vec::new();
        for line in ["one", "two"] {
            let room = room.clone();
            let together = together.clone();
            let id = id.clone();
            speaking.push(tokio::spawn(async move {
                together.wait();
                room.prompt(&id, line, None, None).await
            }));
        }
        for speaker in speaking {
            speaker.await.unwrap().unwrap();
        }

        let markers = markers(&room, &id);
        assert_eq!(
            markers.len(),
            2,
            "{id}'s closed chapter and the one both lines landed in"
        );
        assert!(markers[1].get("endedAt").is_none());

        for _ in 0..200 {
            if lock(&prompts).len() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let mut heard = lock(&prompts).clone();
        heard.sort();
        assert_eq!(heard, ["one", "two"], "{id} heard both lines");
    }
}

#[tokio::test]
async fn a_teammate_with_no_session_is_idle_and_a_started_one_reports_its_driver() {
    let room = room("info", Fake::new(Scripted::new(Vec::new())));
    assert_eq!(room.info("ada"), idle_info("ada"));
    assert_eq!(room.info("ada").state, SessionState::Idle);

    let info = room.start("ada").await.unwrap();
    assert_eq!(info.agent_name.as_deref(), Some("Scripted"));
    assert_eq!(info.current_model_id.as_deref(), Some("anthropic/claude"));
    assert_eq!(room.info("ada"), info);

    let switched = room.set_model("ada", "anthropic/other").await.unwrap();
    assert_eq!(
        switched.current_model_id.as_deref(),
        Some("anthropic/other")
    );
    assert_eq!(room.info("ada"), switched);

    room.stop("ada").unwrap();
    assert_eq!(room.info("ada").state, SessionState::Idle);
    assert!(
        room.prompt("ada", "anyone there?", None, None)
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn driver_metadata_notifications_refresh_roster_without_tape_events() {
    let (driver, changes) = Scripted::new(Vec::new()).with_info_changes();
    let room = room("driver-info", Fake::new(driver));
    let mut infos = room.subscribe_info();
    room.start("ada").await.unwrap();
    assert_eq!(infos.recv().await.unwrap().state, SessionState::Ready);

    let updated = DriverInfo {
        models: vec![ConfigChoice {
            id: "anthropic/sonnet".to_string(),
            name: "Sonnet".to_string(),
            description: None,
            group: None,
        }],
        current_model_id: "anthropic/sonnet".to_string(),
        model_label: Some("Model".to_string()),
        modes: vec![ConfigChoice {
            id: "build".to_string(),
            name: "Build".to_string(),
            description: None,
            group: None,
        }],
        current_mode_id: Some("build".to_string()),
        mode_label: Some("Runtime mode".to_string()),
        configs: vec![crate::contract::SessionConfig {
            id: "reasoning".to_string(),
            name: "Reasoning".to_string(),
            category: Some(crate::contract::SessionConfigCategory::Effort),
            current_id: Some("high".to_string()),
            options: vec![ConfigChoice {
                id: "high".to_string(),
                name: "High".to_string(),
                description: None,
                group: None,
            }],
        }],
        ..DriverInfo::default()
    };
    changes.send_replace(updated.clone());

    let changed = tokio::time::timeout(Duration::from_secs(2), infos.recv())
        .await
        .expect("the driver metadata update arrived")
        .expect("the info stream stayed open");
    assert_eq!(changed.state, SessionState::Ready);
    assert_eq!(changed.models, updated.models);
    assert_eq!(
        changed.current_model_id,
        Some("anthropic/sonnet".to_string())
    );
    assert_eq!(changed.modes, updated.modes);
    assert_eq!(changed.current_mode_id, Some("build".to_string()));
    assert_eq!(changed.configs, updated.configs);
    assert_eq!(room.info("ada"), changed);
    assert!(
        tape(&room, "ada")
            .into_iter()
            .all(|event| event["kind"] == "chapter")
    );

    // Once the session is stopped, a late notification from its driver cannot
    // update the idle roster or resurrect a picker.
    room.stop("ada").unwrap();
    assert_eq!(room.info("ada").state, SessionState::Idle);
    changes.send_replace(DriverInfo {
        current_model_id: "stale-model".to_string(),
        ..updated
    });
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(room.info("ada").state, SessionState::Idle);
    assert_eq!(room.info("ada").current_model_id, None);
}

/// A teammate's directory is made when it starts, wherever it was pointed.
#[tokio::test(flavor = "multi_thread")]
async fn starting_a_teammate_makes_its_working_directory() {
    let log = scratch("makes-cwd");
    let mut ada = persona("ada");
    ada.cwd = log
        .root()
        .join("workspaces")
        .join("ada")
        .to_string_lossy()
        .into_owned();
    enrol(&log, &ada);
    let room = Room::new(log, Arc::new(DeskKeys));
    assert!(!std::path::Path::new(&ada.cwd).exists());
    room.start("ada").await.unwrap();
    assert!(std::path::Path::new(&ada.cwd).is_dir());
}

/// The preamble is everything the agent would otherwise have to ask for.
#[test]
fn the_preamble_says_who_where_how_far_and_when() {
    let mut ada = persona("ada");
    ada.cwd = "/tmp/harbour".to_string();
    let walled = preamble(&ada, Some(Reach::Workspace), None, &[]);
    assert!(walled.contains("You are Ada."));
    assert!(walled.contains("Keep the harbour running."));
    assert!(walled.contains("Your working directory is /tmp/harbour."));
    assert!(walled.contains("a path that leaves it is refused"));
    assert!(walled.contains(&Local::now().format("%A %-d %B %Y").to_string()));
    // Skills are an index, not a body: the name, when to use it, and the
    // file to read, after the tool sentence and before the house style.
    assert!(walled.contains("\n- hotline-room: "));
    assert!(walled.contains(".agents/skills/hotline-room/SKILL.md"));
    let tools_at = walled.find("`search_thread`").unwrap();
    let skills_at = walled.find("You have skills").unwrap();
    let style_at = walled.find("Hotline shows your reply as chat").unwrap();
    assert!(tools_at < skills_at && skills_at < style_at, "{walled}");
    assert!(
        walled.contains("Hotline shows your reply as chat"),
        "both kinds of agent are told the house style: {walled}"
    );

    let open = preamble(
        &ada,
        Some(Reach::Machine),
        Some("the wake block".to_string()),
        &[],
    );
    assert!(open.contains("reach the whole machine"));
    assert!(open.ends_with("the wake block"));

    // The harness owns its tools' permissions; Hotline owns its file callbacks.
    let child = preamble(&ada, None, None, &[]);
    assert!(child.contains("Your working directory is /tmp/harbour."));
    assert!(!child.contains("reach the whole machine"));
    assert!(!child.contains("a path that leaves it is refused"));
    assert!(child.contains("Your harness manages the permissions of its own tools."));
    assert!(child.contains(
        "File operations delegated to Hotline through ACP stay inside your working directory."
    ));
    assert!(
        child.contains("Hotline shows your reply as chat"),
        "an ACP child hears the same house style in its preamble: {child}"
    );

    // A computer is granted outside the policy, so the preamble is where a
    // teammate learns it has one, and that the person can take it over.
    assert!(!child.contains("You have a computer"));
    ada.computer = Some(PersonaComputer {
        enabled: true,
        image: None,
        memory: None,
        pids: None,
        mounts: None,
        secrets: None,
    });
    let desk = preamble(&ada, Some(Reach::Workspace), None, &[]);
    assert!(desk.contains("You have a computer"));
    assert!(desk.contains("take it over"));
    assert!(desk.contains("action `guide`"));
    assert!(desk.contains("from that running computer"));
    assert!(desk.contains("`request_human`"));
    assert!(
        walled.contains("`request_human`"),
        "the preamble names the tool that asks the person: {walled}"
    );
}

/// A teammate naming a backend no agent on this machine answers to is told
/// so, rather than quietly started on a different agent than the one it names.
#[tokio::test]
async fn a_backend_no_agent_answers_to_is_refused_by_name() {
    let log = scratch("backend");
    let mut stranger = persona("stranger");
    stranger.backend_id = "nonesuch".to_string();
    enrol(&log, &stranger);
    let room = Room::new(log, Arc::new(DeskKeys));

    let refused = room.start("stranger").await.unwrap_err();
    assert!(refused.contains("nonesuch"), "{refused}");
}

/// The history a driver starts back into is what was said in the chapter it
/// is joining, and only that.
#[test]
fn the_conversation_a_driver_is_seeded_with_is_the_words_of_its_own_chapter() {
    let older = [
        json!({"kind": "user", "id": "u1", "ts": 1, "text": "hello"}),
        json!({"kind": "thought", "id": "t1", "ts": 2, "text": "hmm"}),
        json!({"kind": "tool", "id": "tool:c1", "ts": 3, "toolCallId": "c1", "title": "ls", "status": "completed"}),
        json!({"kind": "agent", "id": "a1", "ts": 4, "text": "hi"}),
    ];
    let hello = [
        Said::User("hello".to_string()),
        Said::Agent("hi".to_string()),
    ];

    // A tape nobody has divided reads as one implicit chapter.
    assert_eq!(said(&older), hello);

    // An open chapter is the context: what came before it belongs to a
    // context that has already been summarised and let go of.
    let mut divided = older.to_vec();
    divided.push(json!({"kind": "chapter", "id": "c1", "ts": 5, "backendId": "hotline"}));
    divided.push(json!({"kind": "user", "id": "u2", "ts": 6, "text": "still there?"}));
    assert_eq!(said(&divided), [Said::User("still there?".to_string())]);

    // The last chapter closed, so this session starts on nothing: the wake
    // block is what carries the chapter behind it.
    let mut closed = older.to_vec();
    closed.push(
        json!({"kind": "chapter", "id": "c1", "ts": 5, "backendId": "hotline", "endedAt": 9}),
    );
    assert_eq!(said(&closed), []);
}

fn bubble(n: u32) -> String {
    format!("Paragraph {n} is long enough to stand as its own bubble in the chat.")
}

/// Two paragraphs become two bubbles with the driver's id on the first and
/// `{id}-2` on the second, same timestamp; five become five. The model said
/// one thing either way, so history reads one `Said::Agent`.
#[tokio::test]
async fn a_reply_is_paced_as_chat() {
    let first = bubble(1);
    let second = bubble(2);
    let two = format!("{first}\n\n{second}");
    let five_units: Vec<String> = (1..=5).map(bubble).collect();
    let five = five_units.join("\n\n");

    let room = room(
        "paced-chat",
        Fake::new(Scripted::turns(vec![
            saying("two", &two),
            saying("five", &five),
        ])),
    );
    room.start("ada").await.unwrap();
    room.prompt("ada", "two bubbles", None, None).await.unwrap();
    let chat = settled(&room, "ada", 4).await;
    assert_eq!(
        kinds(&chat),
        ["user", "agent", "agent", "turn"],
        "two paragraphs are two agent events"
    );
    assert_eq!(chat[1]["id"], "m-two");
    assert_eq!(chat[2]["id"], "m-two-2");
    assert_eq!(chat[1]["ts"], chat[2]["ts"]);
    assert_eq!(chat[1]["text"], first);
    assert_eq!(chat[2]["text"], second);
    assert_eq!(
        said(&tape(&room, "ada")),
        [Said::User("two bubbles".to_string()), Said::Agent(two),]
    );

    room.prompt("ada", "five bubbles", None, None)
        .await
        .unwrap();
    let events = settled(&room, "ada", 11).await;
    let agents: Vec<_> = events
        .iter()
        .filter(|event| {
            event["kind"] == "agent" && event["id"].as_str().unwrap().starts_with("m-five")
        })
        .collect();
    assert_eq!(agents.len(), 5, "{}", kinds(&events).join(", "));
    assert_eq!(agents[0]["id"], "m-five");
    assert_eq!(agents[1]["id"], "m-five-2");
    assert_eq!(agents[0]["ts"], agents[4]["ts"]);
    for (i, unit) in five_units.iter().enumerate() {
        assert_eq!(agents[i]["text"], *unit);
    }
    assert_eq!(
        said(&tape(&room, "ada")),
        [
            Said::User("two bubbles".to_string()),
            Said::Agent(format!("{first}\n\n{second}")),
            Said::User("five bubbles".to_string()),
            Said::Agent(five),
        ]
    );
}

/// A teammate that narrates its work is heard once before it and once after.
/// What it said between its tool calls is on the tape as thinking, its
/// deltas after the acknowledgement stream as thinking, and the phone is
/// told about the two lines that were said and nothing else.
#[tokio::test]
async fn what_is_said_between_tool_calls_is_thinking_not_chat() {
    fn says(id: &str, text: &str) -> [Update; 2] {
        [
            Update::Delta {
                kind: MessageKind::Agent,
                message_id: id.to_string(),
                text: text.to_string(),
            },
            Update::Message {
                kind: MessageKind::Agent,
                id: id.to_string(),
                text: text.to_string(),
            },
        ]
    }
    fn calls(id: &str) -> [Update; 2] {
        [
            Update::ToolCall {
                call_id: id.to_string(),
                title: format!("cargo {id}"),
                kind: "bash".to_string(),
            },
            Update::ToolResult {
                call_id: id.to_string(),
                ok: true,
                output: "ok".to_string(),
                images: Vec::new(),
            },
        ]
    }
    let mut turn = Vec::new();
    turn.extend(says("m-ack", "on it"));
    turn.extend(calls("c1"));
    turn.extend(says("m-mid", "Found the failing test, fixing it now."));
    turn.extend(calls("c2"));
    turn.extend(says("m-again", "Running the suite again."));
    turn.extend(calls("c3"));
    turn.extend(says("m-done", "Done, all green."));
    turn.push(Update::Turn {
        stop_reason: "end_turn".to_string(),
        usage: None,
    });

    let room = room("narration", Fake::new(Scripted::turns(vec![turn])));
    room.start("ada").await.unwrap();
    let mut deltas = room.subscribe_deltas();
    room.prompt("ada", "fix the build", None, None)
        .await
        .unwrap();
    let chat = settled(&room, "ada", 9).await;
    assert_eq!(
        kinds(&chat),
        [
            "user", "agent", "tool", "thought", "tool", "thought", "tool", "agent", "turn"
        ],
        "{}",
        kinds(&chat).join(", ")
    );
    assert_eq!(chat[1]["text"], "on it");
    assert_eq!(chat[3]["text"], "Found the failing test, fixing it now.");
    assert_eq!(chat[3]["id"], "m-mid", "the words are demoted, not lost");
    assert_eq!(chat[5]["text"], "Running the suite again.");
    assert_eq!(chat[7]["text"], "Done, all green.");
    assert_eq!(
        said(&tape(&room, "ada")),
        [
            Said::User("fix the build".to_string()),
            Said::Agent("on it\n\nDone, all green.".to_string()),
        ],
        "the model's own history keeps what was said, not what was thought"
    );

    let mut streamed = Vec::new();
    while let Ok(delta) = deltas.try_recv() {
        streamed.push(match delta {
            StreamDelta::AgentDelta { message_id, .. } => format!("agent:{message_id}"),
            StreamDelta::ThoughtDelta { message_id, .. } => format!("thought:{message_id}"),
        });
    }
    assert_eq!(
        streamed,
        [
            "agent:m-ack",
            "thought:m-mid",
            "thought:m-again",
            "thought:m-done"
        ],
        "after the acknowledgement the window shows thinking, never a bubble that vanishes"
    );
}

/// A teammate on a computer, the way one is started in a test: a scripted
/// runtime that plays `docker` and reports the port a fake computer serves
/// on. Answers the room, the fake agents (whose preambles say what the
/// teammate heard) and the workspace.
#[cfg(unix)]
struct ComputerRoom {
    room: Arc<Room>,
    agents: Arc<Fake>,
    root: std::path::PathBuf,
    cwd: std::path::PathBuf,
    /// How often the releases endpoint was asked.
    asked: Arc<std::sync::atomic::AtomicUsize>,
    /// Every set of secrets the fake computer was handed, in order.
    taken: crate::computer::guide::fake::Taken,
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
            Arc::new(crate::credentials::tests::MemoryStore::default()),
        )
        .unwrap(),
    );
    let mut ada = persona("ada");
    ada.cwd = cwd.to_string_lossy().into_owned();
    ada.computer = Some(PersonaComputer {
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

/// The passkey cards on a teammate's tape, oldest first, each as its
/// latest word.
#[cfg(unix)]
fn passkey_cards(room: &Room, persona_id: &str) -> Vec<Value> {
    tape(room, persona_id)
        .into_iter()
        .filter(|event| event["kind"] == "passkey_ask")
        .collect()
}

/// The notices on a teammate's tape, oldest first.
#[cfg(unix)]
fn notices(room: &Room, persona_id: &str) -> Vec<String> {
    tape(room, persona_id)
        .iter()
        .filter(|event| event["kind"] == "notice")
        .map(|event| event["text"].as_str().unwrap().to_string())
        .collect()
}

/// Grants `names` to ada's computer on the room's record, the way
/// `persona.update` would, without reattaching anything.
#[cfg(unix)]
fn grant_secrets(room: &Room, names: &[&str]) {
    let mut ada = room.persona("ada").unwrap();
    let computer = ada.computer.as_mut().unwrap();
    computer.secrets = Some(names.iter().map(|name| name.to_string()).collect());
    enrol(&room.log, &ada);
}

/// The secrets a teammate is granted are read from the vault and handed to
/// its computer at start, whole; a name that is not stored any more is said
/// on the tape rather than silently dropped; and the preamble names what
/// the computer has, never a value.
#[cfg(unix)]
#[tokio::test]
async fn a_computers_granted_secrets_are_handed_to_it_at_start_by_name_and_never_seen() {
    let ComputerRoom {
        room,
        agents,
        taken,
        ..
    } = computer_room("computer-secrets", Some("0.9.1"), TWO_RELEASES, true).await;
    let vault = room.vault.as_ref().unwrap();
    vault
        .set_shared_secret("GITHUB_TOKEN", "ghp_notarealtoken0001")
        .unwrap();
    vault
        .set_shared_secret("NPM_TOKEN", "npm_notarealtoken0002")
        .unwrap();
    grant_secrets(&room, &["GITHUB_TOKEN", "GONE_TOKEN"]);
    room.start("ada").await.unwrap();

    // Only what was granted, and only what is stored: NPM_TOKEN was never
    // ticked, GONE_TOKEN is not there to give.
    let sets = taken.sets();
    assert_eq!(sets.len(), 1, "{sets:?}");
    // A variable travels as its bare value, the form every release with a
    // secrets door takes.
    assert_eq!(
        sets[0],
        std::collections::BTreeMap::from([(
            "GITHUB_TOKEN".to_string(),
            serde_json::json!("ghp_notarealtoken0001")
        )])
    );
    let told = notices(&room, "ada");
    assert!(
        told.iter()
            .any(|text| text.contains("not stored any more: GONE_TOKEN")),
        "{told:?}"
    );
    assert!(
        !told.iter().any(|text| text.contains("cannot take secrets")),
        "{told:?}"
    );

    // The agent hears the names and that it will never see a value; the
    // value itself is on no tape and in no preamble.
    let heard = lock(&agents.preambles)[0].clone();
    assert!(
        heard.contains("by name: GITHUB_TOKEN (a variable), GONE_TOKEN (not stored right now)."),
        "{heard}"
    );
    assert!(heard.contains("You never see a value"), "{heard}");
    assert!(!heard.contains("ghp_notarealtoken0001"), "{heard}");
    let on_tape = serde_json::to_string(&tape(&room, "ada")).unwrap();
    assert!(!on_tape.contains("ghp_notarealtoken0001"), "{on_tape}");
    let on_room = std::fs::read_to_string(room.log.root().join("room.jsonl")).unwrap();
    assert!(!on_room.contains("ghp_notarealtoken0001"), "{on_room}");
}

/// A stored value that is replaced or deleted reaches every running
/// computer as a whole new set, so a rotation lands and a revocation
/// leaves nothing behind, without anyone restarting the teammate.
#[cfg(unix)]
#[tokio::test]
async fn a_changed_secret_is_handed_again_to_every_running_computer() {
    let ComputerRoom { room, taken, .. } = computer_room(
        "computer-secrets-changed",
        Some("0.9.1"),
        TWO_RELEASES,
        true,
    )
    .await;
    let vault = room.vault.as_ref().unwrap();
    vault
        .set_shared_secret("GITHUB_TOKEN", "ghp_notarealtoken0001")
        .unwrap();
    grant_secrets(&room, &["GITHUB_TOKEN"]);
    room.start("ada").await.unwrap();
    assert_eq!(taken.sets().len(), 1);

    vault
        .set_shared_secret("GITHUB_TOKEN", "ghp_rotatedtoken00002")
        .unwrap();
    room.secrets_changed().await;
    let sets = taken.sets();
    assert_eq!(sets.len(), 2, "{sets:?}");
    assert_eq!(sets[1]["GITHUB_TOKEN"], "ghp_rotatedtoken00002");

    vault.delete_shared_secret("GITHUB_TOKEN").unwrap();
    room.secrets_changed().await;
    let sets = taken.sets();
    assert_eq!(sets.len(), 3, "{sets:?}");
    assert!(
        sets[2].is_empty(),
        "a deleted secret is gone from the machine"
    );
    assert!(
        notices(&room, "ada")
            .iter()
            .any(|text| text.contains("not stored any more: GITHUB_TOKEN")),
    );

    // A stopped computer is not chased: it gets the set at its next start.
    room.stop("ada").unwrap();
    room.computer_stop("ada").await.unwrap();
    vault
        .set_shared_secret("GITHUB_TOKEN", "ghp_afterstop0000003")
        .unwrap();
    room.secrets_changed().await;
    assert_eq!(taken.sets().len(), 3, "nothing handed to a stopped machine");
}

/// An image from before secrets answers 404. Granted nothing, that is
/// nothing to say; granted something, the tape says the release cannot
/// take it and points at the update.
#[cfg(unix)]
#[tokio::test]
async fn a_computer_from_before_secrets_is_named_only_when_something_was_granted() {
    let ComputerRoom { room, .. } =
        computer_room("computer-secrets-old-quiet", None, TWO_RELEASES, true).await;
    room.start("ada").await.unwrap();
    assert!(
        !notices(&room, "ada")
            .iter()
            .any(|text| text.contains("secrets")),
        "{:?}",
        notices(&room, "ada")
    );

    let ComputerRoom { room, .. } =
        computer_room("computer-secrets-old-told", None, TWO_RELEASES, true).await;
    room.vault
        .as_ref()
        .unwrap()
        .set_shared_secret("GITHUB_TOKEN", "ghp_notarealtoken0001")
        .unwrap();
    grant_secrets(&room, &["GITHUB_TOKEN"]);
    room.start("ada").await.unwrap();
    let told = notices(&room, "ada");
    assert!(
        told.iter().any(|text| text.contains(
            "cannot take secrets, so GITHUB_TOKEN is not in its shell. Update the computer"
        )),
        "{told:?}"
    );
}

/// A passkey is made under an arming and nowhere else: the desk arms one
/// teammate's computer for one site, polls, and the poll that finds the
/// credential minted stores it, ticks it for that teammate, hands the
/// computer the set with it, and ends the arming. The private key crosses
/// once, computer to vault, and is on no tape and in no room event. A
/// cancel ends an arming with nothing stored, and a release from before
/// passkeys is told apart and named.
#[cfg(unix)]
#[tokio::test]
async fn a_passkey_is_made_under_an_arming_stored_and_ticked_for_the_teammate() {
    use crate::computer::guide::fake::KEY_BASE64;
    use crate::contract::{PasskeyRegistrationState, SharedSecretKind};

    let ComputerRoom { room, taken, .. } =
        computer_room("computer-passkey", Some("0.9.1"), TWO_RELEASES, true).await;
    let vault = room.vault.as_ref().unwrap();

    // Nothing armed: idle, and nothing to cancel.
    let idle = room.secrets_passkey_registration("ada").await.unwrap();
    assert_eq!(idle.state, PasskeyRegistrationState::Idle);
    room.secrets_passkey_cancel("ada").await.unwrap();

    // A site that is not one, and a name that is not one, are refused
    // before the computer is touched.
    assert!(
        room.secrets_passkey_register("GITHUB_PASSKEY", "ada", "GitHub.com")
            .await
            .is_err()
    );
    assert!(
        room.secrets_passkey_register("github passkey", "ada", "github.com")
            .await
            .is_err()
    );
    assert_eq!(taken.armed(), None);

    // Arming starts the computer, the same as opening its screen would.
    let armed = room
        .secrets_passkey_register("GITHUB_PASSKEY", "ada", "github.com")
        .await
        .unwrap();
    assert_eq!(armed.state, PasskeyRegistrationState::Armed);
    assert_eq!(armed.name.as_deref(), Some("GITHUB_PASSKEY"));
    assert_eq!(armed.rp_id.as_deref(), Some("github.com"));
    assert!(armed.expires_at.is_some());
    assert_eq!(taken.armed().as_deref(), Some("github.com"));
    assert_eq!(
        room.computer_status("ada").await.unwrap().state,
        crate::contract::ComputerState::Running
    );
    let still = room.secrets_passkey_registration("ada").await.unwrap();
    assert_eq!(still.state, PasskeyRegistrationState::Armed);
    assert!(
        vault.shared_secrets().unwrap().is_empty(),
        "nothing stored yet"
    );

    // The person presses "add a passkey" on the site: the request waits
    // in the browser, and the look that finds it raises the card on ada's
    // tape with what the site asked for. Nothing is stored while it waits.
    let first = taken.ask();
    let asked = registration_until(&room, "ada", PasskeyRegistrationState::Asked).await;
    let ask = asked.ask.expect("the site's request");
    assert_eq!(ask.id, first);
    assert_eq!(ask.user_name.as_deref(), Some("teammate"));
    let cards = passkey_cards(&room, "ada");
    assert_eq!(cards.len(), 1, "{cards:?}");
    assert_eq!(cards[0]["status"], "pending");
    assert_eq!(cards[0]["askId"], first);
    assert_eq!(cards[0]["name"], "GITHUB_PASSKEY");
    assert_eq!(cards[0]["rpId"], "github.com");
    assert_eq!(cards[0]["origin"], "https://github.com");
    assert_eq!(cards[0]["rpName"], "The site");
    assert_eq!(cards[0]["userName"], "teammate");
    assert_eq!(cards[0]["userDisplayName"], "The teammate");
    assert!(
        vault.shared_secrets().unwrap().is_empty(),
        "nothing is stored while the card waits"
    );
    assert_eq!(taken.answered(), None);
    // An answer names the request; a stale card cannot let one through.
    assert!(
        room.secrets_passkey_answer("ada", "ask-9", true)
            .await
            .is_err()
    );
    assert_eq!(taken.answered(), None);
    // Approved on the tape, the browser makes it: the look that finds it
    // made — this poll's, or the room's own — stores it, and the poll is
    // told. One answer to one request.
    let approved = room
        .secrets_passkey_answer("ada", &first, true)
        .await
        .unwrap();
    assert_eq!(approved.state, PasskeyRegistrationState::Approved);
    assert_eq!(taken.answered(), Some(true));
    assert_eq!(passkey_cards(&room, "ada")[0]["status"], "approved");
    assert!(
        room.secrets_passkey_answer("ada", &first, true)
            .await
            .is_err()
    );
    taken.mint();
    let stored = registration_until(&room, "ada", PasskeyRegistrationState::Stored).await;
    let secret = stored.secret.expect("the record, as listed");
    let cards = passkey_cards(&room, "ada");
    assert_eq!(cards.len(), 1, "the card is one card: {cards:?}");
    assert_eq!(cards[0]["status"], "approved");
    assert_eq!(secret.name, "GITHUB_PASSKEY");
    assert_eq!(secret.kind, SharedSecretKind::Passkey);
    assert_eq!(secret.rp_id.as_deref(), Some("github.com"));
    assert_eq!(secret.user_name.as_deref(), Some("teammate"));
    let listed = vault.shared_secrets().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].kind, SharedSecretKind::Passkey);

    // Ticked for ada, handed to the computer whole, and the arming ended.
    let ada = room.persona("ada").unwrap();
    assert_eq!(
        ada.computer.unwrap().secrets.as_deref(),
        Some(&["GITHUB_PASSKEY".to_string()][..])
    );
    let sets = taken.sets();
    let handed = &sets.last().expect("handed after storing")["GITHUB_PASSKEY"];
    assert_eq!(handed["kind"], "passkey");
    assert_eq!(handed["rpId"], "github.com");
    assert_eq!(handed["privateKey"], KEY_BASE64);
    assert_eq!(taken.armed(), None);
    assert_eq!(
        room.secrets_passkey_registration("ada")
            .await
            .unwrap()
            .state,
        PasskeyRegistrationState::Idle
    );

    // The private key is in the vault and nowhere the model or a log reads.
    let on_tape = serde_json::to_string(&tape(&room, "ada")).unwrap();
    assert!(!on_tape.contains(KEY_BASE64), "{on_tape}");
    let on_room = std::fs::read_to_string(room.log.root().join("room.jsonl")).unwrap();
    assert!(!on_room.contains(KEY_BASE64), "{on_room}");
    assert!(!on_room.contains("AQID"), "{on_room}");

    // A cancel ends an arming with nothing stored, and a card nobody
    // answered expires with it.
    room.secrets_passkey_register("GITLAB_PASSKEY", "ada", "gitlab.com")
        .await
        .unwrap();
    assert_eq!(taken.armed().as_deref(), Some("gitlab.com"));
    taken.ask();
    registration_until(&room, "ada", PasskeyRegistrationState::Asked).await;
    room.secrets_passkey_cancel("ada").await.unwrap();
    assert_eq!(taken.armed(), None);
    assert_eq!(
        room.secrets_passkey_registration("ada")
            .await
            .unwrap()
            .state,
        PasskeyRegistrationState::Idle
    );
    assert_eq!(vault.shared_secrets().unwrap().len(), 1);
    let cards = passkey_cards(&room, "ada");
    assert_eq!(cards.len(), 2, "{cards:?}");
    assert_eq!(cards[1]["name"], "GITLAB_PASSKEY");
    assert_eq!(cards[1]["status"], "expired");

    // A release from before passkeys has no door for one, and is named.
    let ComputerRoom { room, taken, .. } =
        computer_room("computer-passkey-old", Some("0.7.0"), TWO_RELEASES, true).await;
    let refused = room
        .secrets_passkey_register("GITHUB_PASSKEY", "ada", "github.com")
        .await
        .unwrap_err();
    assert!(refused.contains("cannot make passkeys"), "{refused}");
    assert_eq!(taken.armed(), None);
}

/// A login is stored as one record and handed to the computer as one,
/// kind first, beside a variable's bare value; the preamble names each by
/// what it is, and the password, like the token, is on no tape.
#[cfg(unix)]
#[tokio::test]
async fn a_login_is_handed_to_the_computer_as_a_record_and_named_by_its_sites() {
    let ComputerRoom {
        room,
        agents,
        taken,
        ..
    } = computer_room("computer-login", Some("0.9.1"), TWO_RELEASES, true).await;
    let vault = room.vault.as_ref().unwrap();
    vault
        .set_shared_secret("GITHUB_TOKEN", "ghp_notarealtoken0001")
        .unwrap();
    vault
        .set_shared(
            "GITHUB_LOGIN",
            crate::vault::StoredSecret::Login {
                sites: vec!["https://github.com".to_string()],
                username: "george".to_string(),
                password: "correct-horse-battery".to_string(),
                totp: None,
            },
        )
        .unwrap();
    grant_secrets(&room, &["GITHUB_LOGIN", "GITHUB_TOKEN"]);
    room.start("ada").await.unwrap();

    let sets = taken.sets();
    assert_eq!(sets.len(), 1, "{sets:?}");
    assert_eq!(sets[0]["GITHUB_TOKEN"], "ghp_notarealtoken0001");
    assert_eq!(
        sets[0]["GITHUB_LOGIN"],
        serde_json::json!({
            "kind": "login", "sites": ["https://github.com"],
            "username": "george", "password": "correct-horse-battery",
        })
    );

    let heard = lock(&agents.preambles)[0].clone();
    assert!(
        heard.contains(
            "by name: GITHUB_LOGIN (a login for https://github.com), GITHUB_TOKEN (a variable)."
        ),
        "{heard}"
    );
    assert!(
        heard.contains("NAME.username, NAME.password, or NAME.code"),
        "{heard}"
    );
    assert!(!heard.contains("correct-horse-battery"), "{heard}");
    let on_tape = serde_json::to_string(&tape(&room, "ada")).unwrap();
    assert!(!on_tape.contains("correct-horse-battery"), "{on_tape}");
    let on_room = std::fs::read_to_string(room.log.root().join("room.jsonl")).unwrap();
    assert!(!on_room.contains("correct-horse-battery"), "{on_room}");
}

/// What was brought over is listed from the room's record, and taken back
/// by site or whole: the computer is told the exact domains and drops them,
/// the record follows, and an entry with nothing left goes. A site that was
/// never brought over is refused, and a release from before the door is
/// named with the pane's Update.
#[cfg(unix)]
#[tokio::test]
async fn brought_over_cookies_are_listed_and_taken_back_by_site_or_whole() {
    use crate::contract::{CookieImport, CookieSite};
    let ComputerRoom { room, taken, .. } =
        computer_room("computer-cookies-forget", Some("0.9.1"), TWO_RELEASES, true).await;
    assert!(room.computer_cookies_list("ada").is_empty());
    let recorded = vec![CookieImport {
        browser_id: "chrome".to_string(),
        browser_name: "Google Chrome".to_string(),
        profile_id: "Default".to_string(),
        profile_name: "Default".to_string(),
        imported_at: 1,
        sites: vec![
            CookieSite {
                domain: "github.com".to_string(),
                cookies: 3,
            },
            CookieSite {
                domain: "gitlab.com".to_string(),
                cookies: 2,
            },
        ],
    }];
    room::record_cookie_imports(&room.log, "ada", &recorded).unwrap();
    assert_eq!(room.computer_cookies_list("ada"), recorded);

    let refused = room
        .computer_cookies_forget("ada", "chrome", "Default", Some("example.com"))
        .await
        .unwrap_err();
    assert!(refused.contains("not among the sites"), "{refused}");
    assert!(
        room.computer_cookies_forget("ada", "firefox", "default", None)
            .await
            .is_err()
    );
    assert!(
        taken.forgotten().is_empty(),
        "nothing asked of the computer yet"
    );

    // One site: the computer is told that domain, and the record loses it.
    let left = room
        .computer_cookies_forget("ada", "chrome", "Default", Some("github.com"))
        .await
        .unwrap();
    assert_eq!(
        taken.forgotten(),
        vec![(
            "import-chrome-Default".to_string(),
            vec!["github.com".to_string()]
        )]
    );
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].sites.len(), 1);
    assert_eq!(left[0].sites[0].domain, "gitlab.com");
    assert_eq!(room.computer_cookies_list("ada"), left);

    // The rest: every remaining domain is named, and the entry goes.
    let left = room
        .computer_cookies_forget("ada", "chrome", "Default", None)
        .await
        .unwrap();
    assert!(left.is_empty());
    assert_eq!(
        taken.forgotten()[1],
        (
            "import-chrome-Default".to_string(),
            vec!["gitlab.com".to_string()]
        )
    );
    assert!(room.computer_cookies_list("ada").is_empty());

    // A release from before the door: 404, said as the pane's Update.
    let ComputerRoom { room, taken, .. } = computer_room(
        "computer-cookies-forget-old",
        Some("0.8.0"),
        TWO_RELEASES,
        true,
    )
    .await;
    room::record_cookie_imports(&room.log, "ada", &recorded).unwrap();
    let refused = room
        .computer_cookies_forget("ada", "chrome", "Default", None)
        .await
        .unwrap_err();
    assert!(refused.contains("cannot take cookies back"), "{refused}");
    assert!(taken.forgotten().is_empty());
    assert_eq!(
        room.computer_cookies_list("ada"),
        recorded,
        "nothing dropped, nothing forgotten"
    );
}

/// Polls the registration until it answers `state`, since the room's own
/// watch may be storing the passkey at the moment a poll arrives.
#[cfg(unix)]
async fn registration_until(
    room: &Room,
    persona_id: &str,
    state: crate::contract::PasskeyRegistrationState,
) -> crate::contract::PasskeyRegistration {
    for _ in 0..100 {
        let answer = room.secrets_passkey_registration(persona_id).await.unwrap();
        if answer.state == state {
            return answer;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the registration never answered {state:?}");
}

/// The passkey is made from the teammate's screen, or by the teammate, and
/// neither keeps Settings → Secrets open: nothing polls at the moment the
/// browser mints it. The room stores it by itself, ticks it, hands it,
/// ends the arming, says so on the tape, and tells the pane once when it
/// next asks.
#[cfg(unix)]
#[tokio::test]
async fn a_passkey_made_while_no_pane_is_looking_is_stored_by_the_room() {
    use crate::contract::{PasskeyRegistrationState, SharedSecretKind};
    let ComputerRoom { room, taken, .. } = computer_room(
        "computer-passkey-unwatched",
        Some("0.9.1"),
        TWO_RELEASES,
        true,
    )
    .await;
    let vault = room.vault.as_ref().unwrap();
    room.secrets_passkey_register("GITHUB_PASSKEY", "ada", "github.com")
        .await
        .unwrap();
    assert!(vault.shared_secrets().unwrap().is_empty());
    // The site asks: the room's own watch raises the card.
    let ask = taken.ask();
    let mut cards = Vec::new();
    for _ in 0..100 {
        cards = passkey_cards(&room, "ada");
        if !cards.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        cards.len(),
        1,
        "the room raised the card with nothing polling"
    );
    assert_eq!(cards[0]["status"], "pending");
    assert_eq!(cards[0]["askId"], ask);
    // Answered, from whichever seat: the browser makes it, and the watch
    // stores it.
    room.secrets_passkey_answer("ada", &ask, true)
        .await
        .unwrap();
    taken.mint();
    let mut listed = Vec::new();
    for _ in 0..100 {
        listed = vault.shared_secrets().unwrap();
        if !listed.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(listed.len(), 1, "the room stored it with nothing polling");
    assert_eq!(listed[0].name, "GITHUB_PASSKEY");
    assert_eq!(listed[0].kind, SharedSecretKind::Passkey);
    // The pane, opened later, is told once; then nothing is pending.
    let answer = registration_until(&room, "ada", PasskeyRegistrationState::Stored).await;
    assert_eq!(
        answer.secret.map(|secret| secret.name).as_deref(),
        Some("GITHUB_PASSKEY")
    );
    assert_eq!(
        room.secrets_passkey_registration("ada")
            .await
            .unwrap()
            .state,
        PasskeyRegistrationState::Idle
    );
    // Ticked, handed whole, the arming ended, and said on the tape.
    assert_eq!(
        room.persona("ada")
            .unwrap()
            .computer
            .unwrap()
            .secrets
            .as_deref(),
        Some(&["GITHUB_PASSKEY".to_string()][..])
    );
    let sets = taken.sets();
    assert_eq!(
        sets.last().expect("handed after storing")["GITHUB_PASSKEY"]["kind"],
        "passkey"
    );
    assert_eq!(taken.armed(), None);
    let said = notices(&room, "ada");
    assert!(
        said.iter().any(|text| text.contains("GITHUB_PASSKEY")
            && text.contains("github.com")
            && text.contains("ticked")),
        "{said:?}"
    );
}

/// Denied on the tape, the site hears no and the arming is over: nothing
/// is stored, the card says denied, and nothing answers twice. A request
/// that leaves with its page — the person navigated away — expires its
/// card, and the arming goes on for the next request.
#[cfg(unix)]
#[tokio::test]
async fn a_passkey_request_denied_on_the_tape_ends_the_arming_and_a_lost_one_expires() {
    use crate::contract::PasskeyRegistrationState;
    let ComputerRoom { room, taken, .. } =
        computer_room("computer-passkey-denied", Some("0.9.1"), TWO_RELEASES, true).await;
    let vault = room.vault.as_ref().unwrap();
    room.secrets_passkey_register("GITHUB_PASSKEY", "ada", "github.com")
        .await
        .unwrap();
    let first = taken.ask();
    registration_until(&room, "ada", PasskeyRegistrationState::Asked).await;
    let denied = room
        .secrets_passkey_answer("ada", &first, false)
        .await
        .unwrap();
    assert_eq!(denied.state, PasskeyRegistrationState::Idle);
    assert_eq!(taken.armed(), None);
    assert!(vault.shared_secrets().unwrap().is_empty());
    let cards = passkey_cards(&room, "ada");
    assert_eq!(cards.len(), 1, "{cards:?}");
    assert_eq!(cards[0]["status"], "denied");
    let refused = room
        .secrets_passkey_answer("ada", &first, true)
        .await
        .unwrap_err();
    assert!(
        refused.contains("No passkey request is waiting"),
        "{refused}"
    );
    assert_eq!(
        room.secrets_passkey_registration("ada")
            .await
            .unwrap()
            .state,
        PasskeyRegistrationState::Idle
    );
    assert!(
        room.persona("ada")
            .unwrap()
            .computer
            .unwrap()
            .secrets
            .unwrap_or_default()
            .is_empty(),
        "nothing ticked"
    );

    // Lost with its page: the card expires, the arming stays.
    room.secrets_passkey_register("GITHUB_PASSKEY", "ada", "github.com")
        .await
        .unwrap();
    let second = taken.ask();
    registration_until(&room, "ada", PasskeyRegistrationState::Asked).await;
    taken.page_left();
    let armed = registration_until(&room, "ada", PasskeyRegistrationState::Armed).await;
    assert_eq!(armed.ask, None);
    let cards = passkey_cards(&room, "ada");
    assert_eq!(cards.len(), 2, "{cards:?}");
    assert_eq!(cards[1]["askId"], second);
    assert_eq!(cards[1]["status"], "expired");
    let refused = room
        .secrets_passkey_answer("ada", &second, true)
        .await
        .unwrap_err();
    assert!(refused.contains("not waiting any more"), "{refused}");
    // The next request is a new card.
    let third = taken.ask();
    registration_until(&room, "ada", PasskeyRegistrationState::Asked).await;
    let cards = passkey_cards(&room, "ada");
    assert_eq!(cards.len(), 3, "{cards:?}");
    assert_eq!(cards[2]["askId"], third);
    assert_eq!(cards[2]["status"], "pending");
    room.secrets_passkey_cancel("ada").await.unwrap();
    assert_eq!(passkey_cards(&room, "ada")[2]["status"], "expired");
}

#[cfg(unix)]
const TWO_RELEASES: &str = r#"[{"tag_name":"v0.9.2"},{"tag_name":"v0.9.1"}]"#;

/// The command names the scripted runtime was given, in order.
#[cfg(unix)]
fn runtime_commands(root: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(root.join("argv.log"))
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

/// A teammate's computer hands over the guide of the release it is actually
/// running, and that guide is the `hotline-computer` skill in the workspace:
/// on disk under Hotline's marker, in the catalog with its release, and in the
/// preamble as the file to read.
#[cfg(unix)]
#[tokio::test]
async fn a_computers_guide_is_the_hotline_computer_skill_of_the_release_it_runs() {
    let ComputerRoom {
        room, agents, cwd, ..
    } = computer_room("computer-skill", Some("0.9.1"), TWO_RELEASES, true).await;
    room.start("ada").await.unwrap();

    let folder = cwd.join(".agents/skills/hotline-computer");
    assert_eq!(
        std::fs::read_to_string(folder.join("SKILL.md")).unwrap(),
        crate::computer::guide::fake::skill_of("0.9.1")
    );
    let marker = std::fs::read_to_string(folder.join(".managed-by-hotline")).unwrap();
    assert!(marker.starts_with("computer 0.9.1 "), "{marker}");

    let entry = crate::skills::computer_entry(&cwd).expect("listed");
    assert_eq!(entry.source, crate::contract::SkillSource::Computer);
    assert_eq!(entry.name, "hotline-computer");
    assert_eq!(entry.version.as_deref(), Some("0.9.1"));
    assert_eq!(entry.path, ".agents/skills/hotline-computer/SKILL.md");
    assert_eq!(entry.invalid, None);

    let heard = lock(&agents.preambles)[0].clone();
    assert!(heard.contains("your `hotline-computer` skill"), "{heard}");
    assert!(heard.contains("\n- hotline-computer: "), "{heard}");
    assert!(!heard.contains("action `guide`"), "{heard}");

    // A colleague sharing the working directory without a computer of its
    // own is not told about the guide: the line costs context and names a
    // machine it cannot drive.
    let mut bob = persona("bob");
    bob.cwd = cwd.to_string_lossy().into_owned();
    let unheard = preamble(&bob, Some(Reach::Workspace), None, &[]);
    assert!(!unheard.contains("hotline-computer"), "{unheard}");
    assert!(unheard.contains("\n- hotline-room: "), "{unheard}");

    // The pane sees the release running against the one it would be made
    // on now, which is the image tag the teammate asked for.
    let status = room.computer_status("ada").await.unwrap();
    assert_eq!(status.release.as_deref(), Some("0.9.1"));
    assert_eq!(status.available.as_deref(), Some("test"));
}

/// An image too old to serve a guide leaves no skill and no stale one: the
/// teammate is told to ask the computer itself, and the tape says why.
#[cfg(unix)]
#[tokio::test]
async fn a_computer_without_a_guide_leaves_no_skill_and_the_preamble_says_to_ask_it() {
    let ComputerRoom {
        room, agents, cwd, ..
    } = computer_room("computer-no-guide", None, TWO_RELEASES, true).await;
    // A guide from an earlier start, marked as the computer's, must not
    // outlive the computer that served it.
    crate::skills::write_computer(&cwd, "0.1.0", "stale", "stale guide").unwrap();
    room.start("ada").await.unwrap();

    assert!(!cwd.join(".agents/skills/hotline-computer").exists());
    assert!(crate::skills::computer_entry(&cwd).is_none());
    let heard = lock(&agents.preambles)[0].clone();
    assert!(heard.contains("action `guide`"), "{heard}");
    assert!(!heard.contains("- hotline-computer:"), "{heard}");
    let notices: Vec<String> = tape(&room, "ada")
        .iter()
        .filter(|event| event["kind"] == "notice")
        .map(|event| event["text"].as_str().unwrap().to_string())
        .collect();
    assert!(
        notices
            .iter()
            .any(|text| text.contains("did not hand over its guide")),
        "{notices:?}"
    );
    let status = room.computer_status("ada").await.unwrap();
    assert_eq!(status.release, None);
    assert_eq!(status.available, None);
}

/// Updating a computer recreates it on the release it would be created on
/// now and starts the teammate again; the runtime sees a remove and a create.
#[cfg(unix)]
#[tokio::test]
async fn updating_a_computer_recreates_it_and_the_teammate_comes_back() {
    let ComputerRoom { room, root, .. } =
        computer_room("computer-update", Some("0.9.1"), TWO_RELEASES, true).await;
    room.start("ada").await.unwrap();
    room.computer_update("ada").await.unwrap();

    let commands: Vec<String> = runtime_commands(&root)
        .iter()
        .map(|line| {
            line.split_whitespace()
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    let first_create = commands.iter().position(|cmd| cmd == "create").unwrap();
    let removed = commands.iter().position(|cmd| cmd == "rm").unwrap();
    let second_create = commands.iter().rposition(|cmd| cmd == "create").unwrap();
    assert!(
        first_create < removed && removed < second_create,
        "{commands:?}"
    );
    assert_eq!(
        room.info("ada").state,
        SessionState::Ready,
        "the teammate is running again on the new computer"
    );
}

/// A fresh computer is created on the newest published release: the desk
/// asks once before creating, and a computer already on it is offered
/// nothing. A pinned image is used as written and the endpoint never hears
/// about it.
#[cfg(unix)]
#[tokio::test]
async fn a_fresh_computer_is_created_on_the_newest_release_and_a_pin_never_asks() {
    let fresh = computer_room("computer-newest", Some("0.9.2"), TWO_RELEASES, false).await;
    fresh.room.start("ada").await.unwrap();
    let created = runtime_commands(&fresh.root)
        .into_iter()
        .find(|line| line.starts_with("create "))
        .unwrap();
    assert!(
        created.contains("ghcr.io/1broseidon/hotline-computer:0.9.2"),
        "{created}"
    );
    assert_eq!(fresh.asked.load(std::sync::atomic::Ordering::SeqCst), 1);
    let status = fresh.room.computer_status("ada").await.unwrap();
    assert_eq!(status.release.as_deref(), Some("0.9.2"));
    assert_eq!(status.available, None);
    let known = fresh.room.computer_releases();
    assert_eq!(known.floor, crate::computer::COMPUTER_VERSION);
    assert_eq!(known.repository, crate::computer::COMPUTER_REPOSITORY);
    assert_eq!(known.newest.as_deref(), Some("0.9.2"));
    assert_eq!(known.releases, ["0.9.2", "0.9.1"]);
    assert!(known.checked_at.is_some(), "{known:?}");
    assert_eq!(known.error, None);

    let pinned = computer_room("computer-pinned", Some("0.9.2"), TWO_RELEASES, true).await;
    pinned.room.start("ada").await.unwrap();
    let created = runtime_commands(&pinned.root)
        .into_iter()
        .find(|line| line.starts_with("create "))
        .unwrap();
    assert!(created.contains(" hotline-computer:test"), "{created}");
    assert_eq!(
        pinned.asked.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "a pin is exactly what it says"
    );
}

/// Offline, a fresh computer is created on the floor, and nothing is
/// offered until something newer is actually known.
#[cfg(unix)]
#[tokio::test]
async fn offline_a_fresh_computer_is_created_on_the_floor() {
    let offline = computer_room("computer-offline", Some("0.9.1"), "not a list", false).await;
    offline.room.start("ada").await.unwrap();
    let created = runtime_commands(&offline.root)
        .into_iter()
        .find(|line| line.starts_with("create "))
        .unwrap();
    assert!(
        created.contains(&crate::computer::default_image()),
        "{created}"
    );
    assert_eq!(offline.room.computer_releases().newest, None);
    let status = offline.room.computer_status("ada").await.unwrap();
    assert_eq!(
        status.available, None,
        "nothing newer is known, so nothing is offered"
    );
}

/// An image the runtime does not have is pulled before the computer is
/// made, and the pull is one line on the tape that fills in: named when it
/// starts, counted as each layer lands, and left saying done with the
/// time it took. One line, because every report carries the same id.
#[cfg(unix)]
#[tokio::test]
async fn a_pulled_image_is_one_line_on_the_tape_that_fills_in() {
    let desk = computer_room("computer-pull", Some("0.9.1"), TWO_RELEASES, false).await;
    std::fs::write(desk.root.join("state.noimage"), "").unwrap();
    desk.room.start("ada").await.unwrap();
    let pulled = runtime_commands(&desk.root)
        .into_iter()
        .filter(|line| line.starts_with("pull "))
        .count();
    assert_eq!(pulled, 1, "the image was pulled once");
    let lines: Vec<Value> = tape(&desk.room, "ada")
        .into_iter()
        .filter(|event| event["kind"] == "computer_pull")
        .collect();
    assert_eq!(lines.len(), 1, "one line, rewritten in place: {lines:?}");
    let line = &lines[0];
    assert_eq!(line["status"], "done");
    assert_eq!(line["layersDone"], 2);
    assert_eq!(line["layersTotal"], 2);
    assert!(
        line["image"].as_str().unwrap().contains("hotline-computer"),
        "{line}"
    );
    assert!(line["elapsedMs"].is_i64(), "{line}");
    assert!(
        notices(&desk.room, "ada")
            .iter()
            .all(|text| !text.contains("Pulling")),
        "the pull is no longer a notice"
    );
}

/// The Settings button asks the endpoint at once, whatever the clock says,
/// and a refused lookup keeps what was known and says why.
#[cfg(unix)]
#[tokio::test]
async fn a_manual_check_asks_now_and_a_refusal_keeps_what_was_known() {
    use std::sync::atomic::Ordering;
    let desk = computer_room("computer-check-now", Some("0.9.1"), TWO_RELEASES, false).await;
    desk.room.start("ada").await.unwrap();
    assert_eq!(desk.asked.load(Ordering::SeqCst), 1);
    let checked = desk.room.computer_releases_check().await;
    assert_eq!(
        desk.asked.load(Ordering::SeqCst),
        2,
        "the button does not wait six hours"
    );
    assert_eq!(checked.releases, ["0.9.2", "0.9.1"]);
    assert_eq!(checked.error, None);

    let offline = computer_room("computer-check-offline", Some("0.9.1"), "not a list", false).await;
    let refused = offline.room.computer_releases_check().await;
    assert_eq!(refused.newest, None);
    assert!(refused.releases.is_empty());
    assert!(refused.checked_at.is_some());
    assert!(
        refused
            .error
            .as_deref()
            .is_some_and(|why| why.contains("names no release")),
        "{refused:?}"
    );
}

/// A computer on an older release is offered the newest one, and the desk
/// asks the endpoint again only when six hours have passed.
#[cfg(unix)]
#[tokio::test]
async fn an_older_computer_is_offered_the_newest_release_on_the_six_hour_clock() {
    use crate::computer::releases::CHECK_EVERY_MS;
    use std::sync::atomic::Ordering;
    let older = computer_room("computer-older", Some("0.8.0"), TWO_RELEASES, false).await;
    older.room.start("ada").await.unwrap();
    let status = older.room.computer_status("ada").await.unwrap();
    assert_eq!(status.release.as_deref(), Some("0.8.0"));
    assert_eq!(status.available.as_deref(), Some("0.9.2"));

    let asked_at_start = older.asked.load(Ordering::SeqCst);
    let now = now_ms();
    older
        .room
        .computers
        .refresh_releases(now + CHECK_EVERY_MS / 2)
        .await;
    assert_eq!(older.asked.load(Ordering::SeqCst), asked_at_start);
    older
        .room
        .computers
        .refresh_releases(now + CHECK_EVERY_MS)
        .await;
    assert_eq!(older.asked.load(Ordering::SeqCst), asked_at_start + 1);
}

/// The index is written as the tape is, and a search finds the turn without
/// anything having asked for a rebuild.
#[tokio::test]
async fn what_is_appended_is_indexed() {
    let room = room("index", Fake::new(Scripted::new(spoken_turn())));
    room.start("ada").await.unwrap();
    room.prompt("ada", "what is here?", None, None)
        .await
        .unwrap();
    settled(&room, "ada", 5).await;

    let found = crate::store::search::search(room.log.root(), "ada", "here", None);
    assert_eq!(
        found["hits"][0]["excerpt"], "what is here?",
        "the user's line was never indexed: {found}"
    );
}

// ---------------------------------------------------------------------------
// The funnel's own rules: what a line carries, and whose voice is held
// ---------------------------------------------------------------------------

/// A schedule's firing, as the scheduler will hand one over.
fn firing(kind: ScheduleKind, quiet: bool) -> ScheduledRun {
    ScheduledRun {
        job_id: "job-1".to_string(),
        kind,
        name: "Apple order check".to_string(),
        operator_created: false,
        quiet: quiet.then_some(true),
    }
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

fn attachment(name: &str, path: &str) -> Attachment {
    Attachment {
        kind: AttachmentKind::File,
        name: name.to_string(),
        path: path.to_string(),
        mime_type: None,
        size: None,
    }
}

/// Waits for the driver to have been handed `count` lines, so a test never
/// races the task the turn runs on.
async fn heard(prompts: &Arc<Mutex<Vec<String>>>, count: usize) -> Vec<String> {
    for _ in 0..200 {
        let given = lock(prompts).clone();
        if given.len() >= count {
            return given;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the driver was never handed {count} lines");
}

/// The whole point of a quiet run: the agent said something and the chat has
/// nothing in it, and the teammate speaks normally the moment it is over.
#[tokio::test]
async fn a_quiet_runs_words_are_thinking_and_the_next_plain_prompt_speaks() {
    let agents = Fake::new(Scripted::turns(vec![
        saying("quiet", "No change — staying silent per protocol."),
        saying("loud", "It moved."),
    ]));
    let prompts = agents.driver.prompts.clone();
    let room = room("quiet-run", agents);
    let mut deltas = room.subscribe_deltas();
    room.start("ada").await.unwrap();

    room.prompt_scheduled(
        "ada",
        "check the order page",
        firing(ScheduleKind::Loop, true),
    )
    .await
    .unwrap();
    let events = settled(&room, "ada", 3).await;
    assert_eq!(kinds(&events), ["user", "thought", "turn"]);
    assert_eq!(
        events[1]["id"], "m-quiet",
        "the words were demoted, not destroyed"
    );
    assert_eq!(
        events[1]["text"],
        "No change — staying silent per protocol."
    );

    // The tape keeps the bare prompt and the stamp naming the job; the agent
    // heard the framing that says a schedule woke it.
    assert_eq!(events[0]["text"], "check the order page");
    assert_eq!(events[0]["scheduled"]["jobId"], "job-1");
    assert_eq!(events[0]["scheduled"]["quiet"], true);
    assert_eq!(*lock(&prompts), ["loop · check the order page"]);

    room.prompt("ada", "and now?", None, None).await.unwrap();
    let events = settled(&room, "ada", 6).await;
    assert_eq!(
        kinds(&events),
        ["user", "thought", "turn", "user", "agent", "turn"],
        "the run's own turn boundary closed the window"
    );
    assert_eq!(events[4]["text"], "It moved.");

    // The live stream was muted exactly as long as the tape was: an indicator
    // that types and then produces nothing reads as a bug, not as silence.
    let mut streamed = Vec::new();
    while let Ok(delta) = deltas.try_recv() {
        streamed.push(delta);
    }
    assert_eq!(
        streamed,
        [
            StreamDelta::ThoughtDelta {
                persona_id: "ada".to_string(),
                message_id: "m-quiet".to_string(),
                text: "No change — staying silent per protocol.".to_string(),
            },
            StreamDelta::AgentDelta {
                persona_id: "ada".to_string(),
                message_id: "m-loud".to_string(),
                text: "It moved.".to_string(),
            },
        ]
    );
}

/// A person who types during a quiet run is owed an answer they can read; the
/// schedule's silence was never about them.
#[tokio::test]
async fn a_person_typing_during_a_quiet_run_gets_a_bubble_back() {
    let gate = Arc::new(Semaphore::new(0));
    let mut driver = Scripted::turns(vec![
        saying("quiet", "the scheduled run's own words"),
        saying("loud", "It moved."),
    ]);
    driver.gate = Some(gate.clone());
    let room = room("quiet-interrupted", Fake::new(driver));
    room.start("ada").await.unwrap();

    room.prompt_scheduled(
        "ada",
        "check the order page",
        firing(ScheduleKind::Loop, true),
    )
    .await
    .unwrap();
    let events = settled(&room, "ada", 1).await;
    assert_eq!(events[0]["scheduled"]["quiet"], true);

    // The window is open and the run's turn is still gated when a person types.
    room.prompt("ada", "wait, what did you find?", None, None)
        .await
        .unwrap();
    settled(&room, "ada", 2).await;

    // One permit per update: the delta, the message, the turn.
    gate.add_permits(3);
    let events = settled(&room, "ada", 4).await;
    assert_eq!(
        kinds(&events),
        ["user", "user", "agent", "turn"],
        "the human closed the window, so the run's words are a bubble"
    );
}

/// A schedule that is not quiet is stamped and framed all the same; only the
/// voice is left alone.
#[tokio::test]
async fn a_loud_schedule_is_stamped_and_framed_and_keeps_its_voice() {
    let agents = Fake::new(Scripted::new(saying("s", "Standup is at ten.")));
    let prompts = agents.driver.prompts.clone();
    let room = room("scheduled-loud", agents);
    room.start("ada").await.unwrap();

    room.prompt_scheduled(
        "ada",
        "post the standup",
        firing(ScheduleKind::Schedule, false),
    )
    .await
    .unwrap();
    let events = settled(&room, "ada", 3).await;

    assert_eq!(kinds(&events), ["user", "agent", "turn"]);
    assert_eq!(events[0]["text"], "post the standup");
    assert_eq!(events[0]["scheduled"]["name"], "Apple order check");
    assert_eq!(events[0]["scheduled"].get("quiet"), None);
    assert_eq!(*lock(&prompts), ["scheduled · post the standup"]);
}

/// Creating, silencing and cancelling a job go through the room, not around it.
#[tokio::test]
async fn a_job_is_created_silenced_and_cancelled_on_the_room() {
    let room = room("schedule-api", Fake::new(Scripted::new(Vec::new())));
    let when = now_ms() + 60_000;
    let job = room
        .schedule_create(
            "ada",
            ScheduleKind::Schedule,
            Some(when),
            None,
            "check the crane",
            true,
        )
        .unwrap();
    assert_eq!(job.prompt, "check the crane");
    assert_eq!(job.quiet, Some(true));
    assert_eq!(job.when, Some(when));
    assert_eq!(room.schedule_list().len(), 1);

    room.schedule_set_quiet(&job.id, false).unwrap();
    assert_eq!(room.schedule_list()[0].quiet, None);

    room.schedule_cancel(&job.id).unwrap();
    assert!(room.schedule_list().is_empty());
}

/// A job that is already due, planted on the stream before the room opens so
/// the clock's first look fires it — the same path a missed tick takes after
/// Hotline was closed.
fn due_job(
    id: &str,
    kind: ScheduleKind,
    prompt: &str,
    quiet: bool,
    overdue_by: i64,
) -> ScheduledJob {
    let now = now_ms();
    ScheduledJob {
        id: id.to_string(),
        persona_id: "ada".to_string(),
        kind,
        when: (kind == ScheduleKind::Schedule).then_some(now - overdue_by),
        every: (kind == ScheduleKind::Loop).then_some(15_000),
        prompt: prompt.to_string(),
        quiet: quiet.then_some(true),
        operator_created: false,
        next_at: now - overdue_by,
        created_at: now - overdue_by - 60_000,
    }
}

fn room_due(name: &str, agents: Arc<Fake>, job: ScheduledJob) -> Arc<Room> {
    room_due_with_grant(name, agents, job, true)
}

fn room_due_with_grant(
    name: &str,
    agents: Arc<Fake>,
    job: ScheduledJob,
    background_work: bool,
) -> Arc<Room> {
    let log = scratch(name);
    let mut teammate = persona("ada");
    teammate.background_work = background_work;
    enrol(&log, &teammate);
    crate::room::append_schedule(&log, &job).unwrap();
    Room::with_agents(log, Arc::new(DeskKeys), agents)
}

/// A loop that fires writes the bare prompt on the tape and the framed line
/// to the driver, and a fire for a teammate that is not running starts it.
#[tokio::test]
async fn a_loop_job_fires_with_the_bare_prompt_on_the_tape_and_the_framed_text_on_the_wire() {
    let agents = Fake::new(Scripted::new(saying("s", "nothing changed")));
    let prompts = agents.driver.prompts.clone();
    let job = due_job("job-loop", ScheduleKind::Loop, "check the crane", false, 1);
    let room = room_due("schedule-loop-fire", agents, job);

    let events = settled(&room, "ada", 3).await;
    assert_eq!(kinds(&events), ["user", "agent", "turn"]);
    assert_eq!(events[0]["text"], "check the crane");
    assert_eq!(events[0]["scheduled"]["jobId"], "job-loop");
    assert_eq!(events[0]["scheduled"]["kind"], "loop");
    assert_eq!(events[0]["scheduled"].get("quiet"), None);
    assert_eq!(*lock(&prompts), ["loop · check the crane"]);

    let living = crate::room::schedules(&room.log);
    assert_eq!(living.len(), 1);
    assert_eq!(living[0].id, "job-loop");
    assert!(
        living[0].next_at > now_ms(),
        "a loop re-arms into the future, not onto the tick it just fired"
    );
}

/// The whole point of a quiet job, through the clock rather than a direct
/// prompt_scheduled: the agent's words land as thinking.
#[tokio::test]
async fn a_quiet_jobs_reply_lands_as_a_thought() {
    let agents = Fake::new(Scripted::new(saying(
        "quiet",
        "No change — staying silent per protocol.",
    )));
    let job = due_job(
        "job-quiet",
        ScheduleKind::Loop,
        "check the order page",
        true,
        1,
    );
    let room = room_due("schedule-quiet-fire", agents, job);

    let events = settled(&room, "ada", 3).await;
    assert_eq!(kinds(&events), ["user", "thought", "turn"]);
    assert_eq!(events[0]["scheduled"]["quiet"], true);
    assert_eq!(
        events[1]["text"],
        "No change — staying silent per protocol."
    );
}

/// A one-shot fires once and tombstones itself.
#[tokio::test]
async fn a_one_shot_fires_once_and_is_gone() {
    let agents = Fake::new(Scripted::new(saying("s", "the word")));
    let job = due_job("job-once", ScheduleKind::Schedule, "say the word", false, 1);
    let room = room_due("schedule-once-fire", agents, job);

    let events = settled(&room, "ada", 3).await;
    assert_eq!(events[0]["text"], "say the word");
    assert_eq!(events[0]["scheduled"]["kind"], "schedule");
    assert!(
        crate::room::schedules(&room.log)
            .iter()
            .all(|job| job.id != "job-once")
    );
}

/// Jobs missed while Hotline was closed fire once at startup, not once per
/// missed interval. A loop five intervals overdue is still one firing.
#[tokio::test]
async fn a_missed_job_fires_once_at_startup() {
    let agents = Fake::new(Scripted::new(saying("s", "caught up")));
    let job = due_job(
        "job-missed",
        ScheduleKind::Loop,
        "sweep the harbour",
        false,
        5 * 15_000,
    );
    let room = room_due("schedule-missed-fire", agents, job);

    let events = settled(&room, "ada", 3).await;
    let users = events
        .iter()
        .filter(|event| event["kind"] == "user")
        .count();
    assert_eq!(
        users, 1,
        "a missed loop is one firing, not five: {events:?}"
    );

    tokio::time::sleep(Duration::from_millis(200)).await;
    let later = tape(&room, "ada")
        .into_iter()
        .filter(|event| event["kind"] == "user")
        .count();
    assert_eq!(
        later, 1,
        "the clock did not catch up the intervals it slept through"
    );
}

/// An agent-created due job stays on the room stream while its standing grant
/// is off. Regranting it wakes the clock and fires the same job once.
#[tokio::test]
async fn a_due_agent_job_waits_for_a_grant_then_fires_once() {
    let agents = Fake::new(Scripted::new(saying("s", "the word")));
    let prompts = agents.driver.prompts.clone();
    let job = due_job(
        "job-paused",
        ScheduleKind::Schedule,
        "say the word",
        false,
        1,
    );
    let room = room_due_with_grant("schedule-paused", agents, job, false);

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(lock(&prompts).is_empty(), "a paused job was dispatched");
    assert_eq!(crate::room::schedules(&room.log).len(), 1);

    let mut teammate = persona("ada");
    teammate.background_work = true;
    enrol(&room.log, &teammate);
    room.reattach("ada").await.unwrap();

    let events = settled(&room, "ada", 3).await;
    assert_eq!(events[0]["text"], "say the word");
    assert_eq!(events[0]["scheduled"]["operatorCreated"], false);
    assert_eq!(*lock(&prompts), ["scheduled · say the word"]);
    assert!(crate::room::schedules(&room.log).is_empty());
}

/// A job the operator entered from the desk remains runnable when the
/// teammate's autonomous background grant is off.
#[tokio::test]
async fn a_due_operator_job_runs_without_a_background_grant() {
    let agents = Fake::new(Scripted::new(saying("s", "the word")));
    let prompts = agents.driver.prompts.clone();
    let mut job = due_job(
        "job-operator",
        ScheduleKind::Schedule,
        "say the word",
        false,
        1,
    );
    job.operator_created = true;
    let room = room_due_with_grant("schedule-operator", agents, job, false);

    let events = settled(&room, "ada", 3).await;
    assert_eq!(events[0]["text"], "say the word");
    assert_eq!(events[0]["scheduled"]["operatorCreated"], true);
    assert_eq!(*lock(&prompts), ["scheduled · say the word"]);
    assert!(crate::room::schedules(&room.log).is_empty());
}

/// A scheduled line can be queued behind a turn while its one-shot job is
/// already gone from the room stream. The final dispatch check still reads
/// the live grant and drops an agent-created line that lost permission.
#[tokio::test]
async fn a_queued_scheduled_line_is_dropped_when_background_work_is_revoked() {
    let gate = Arc::new(Semaphore::new(0));
    let mut driver = Scripted::turns(vec![
        vec![Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        }],
        vec![Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        }],
    ]);
    driver.gate = Some(gate.clone());
    let agents = Fake::new(driver);
    let prompts = agents.driver.prompts.clone();
    let room = room("scheduled-queue-revoked", agents);
    room.start("ada").await.unwrap();

    room.prompt("ada", "first", None, None).await.unwrap();
    until_state(&room, "ada", SessionState::Thinking).await;
    room.prompt_scheduled(
        "ada",
        "old scheduled work",
        firing(ScheduleKind::Schedule, false),
    )
    .await
    .unwrap();
    let queued = settled(&room, "ada", 2).await;
    assert_eq!(queued[1]["scheduled"]["operatorCreated"], false);

    // This mirrors the live record changing before the old turn gives the
    // queue its next dispatch opportunity. Policy update callers also revoke
    // the session, which clears the queue earlier; this check closes the
    // remaining race where a queued one-shot survives its room tombstone.
    let mut teammate = persona("ada");
    teammate.background_work = false;
    enrol(&room.log, &teammate);
    gate.add_permits(1);
    until_state(&room, "ada", SessionState::Ready).await;

    assert_eq!(*lock(&prompts), ["first"]);
    assert_eq!(
        tape(&room, "ada")
            .into_iter()
            .filter(|event| event["kind"] == "user")
            .count(),
        2,
        "the queued line remains a durable user fact even though no turn ran"
    );
}

/// What a message answers is on the line it is written as, and a mark that has
/// gone stale is claimed by nothing.
#[tokio::test]
async fn a_reply_is_stamped_on_its_own_line_and_only_while_the_mark_is_fresh() {
    let room = room("reply", Fake::new(Scripted::new(Vec::new())));
    room.start("ada").await.unwrap();

    room.prompt("ada", "this one", Some("a1".to_string()), None)
        .await
        .unwrap();
    let events = settled(&room, "ada", 1).await;
    assert_eq!(events[0]["replyTo"], "a1");

    let session = room.session("ada").unwrap();
    *lock(&session.pending_reply) = Some(Mark {
        value: "a1".to_string(),
        until: now_ms() - 1,
    });
    room.prompt("ada", "and this one", None, None)
        .await
        .unwrap();
    let events = settled(&room, "ada", 2).await;
    assert_eq!(
        events[1].get("replyTo"),
        None,
        "an expired mark is not claimed by a later, unrelated message"
    );
}

/// The record keeps what was attached, and the driver is handed the same
/// files beside the words rather than inside them — because how an attachment
/// reaches an agent is the driver's answer and not the room's.
#[tokio::test]
async fn attachments_land_on_the_line_and_beside_what_the_driver_hears() {
    let agents = Fake::new(Scripted::new(Vec::new()));
    let prompts = agents.driver.prompts.clone();
    let handed = agents.driver.attachments.clone();
    let room = room("attachments", agents);
    room.start("ada").await.unwrap();

    room.prompt(
        "ada",
        "read these",
        None,
        Some(vec![
            attachment("note.txt", "/tmp/note.txt"),
            attachment("shot.png", "/tmp/shot.png"),
        ]),
    )
    .await
    .unwrap();

    let events = settled(&room, "ada", 1).await;
    assert_eq!(events[0]["attachments"][0]["name"], "note.txt");
    assert_eq!(events[0]["attachments"][1]["path"], "/tmp/shot.png");
    assert_eq!(heard(&prompts, 1).await, ["read these"]);
    let handed = lock(&handed).clone();
    assert_eq!(
        handed[0]
            .iter()
            .map(|attachment| attachment.path.as_str())
            .collect::<Vec<_>>(),
        ["/tmp/note.txt", "/tmp/shot.png"]
    );
}

/// Hotline's own words to a running teammate: the driver hears them, and the
/// conversation has no line saying anybody spoke.
#[tokio::test]
async fn a_nudge_reaches_the_driver_and_never_the_tape() {
    let agents = Fake::new(Scripted::new(vec![Update::Turn {
        stop_reason: "end_turn".to_string(),
        usage: None,
    }]));
    let prompts = agents.driver.prompts.clone();
    let room = room("nudge", agents);
    room.start("ada").await.unwrap();

    room.nudge("ada", "while you were away the user asked you to hurry")
        .unwrap();
    assert_eq!(
        heard(&prompts, 1).await,
        ["while you were away the user asked you to hurry"]
    );

    let events = settled(&room, "ada", 1).await;
    assert_eq!(
        kinds(&events),
        ["turn"],
        "the turn it ran is on the tape; the words that started it are not"
    );
}

// ---------------------------------------------------------------------------
// Chapters: where one working context ends and the next begins
// ---------------------------------------------------------------------------

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

/// Nothing said is outside a chapter, and a chapter that is still open is the
/// chapter a restarted session rejoins.
#[tokio::test]
async fn a_session_opens_a_chapter_and_a_restart_within_it_opens_no_second_one() {
    let room = room("chapter-open", Fake::new(Scripted::new(Vec::new())));
    room.start("ada").await.unwrap();

    let opened = markers(&room, "ada");
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0]["backendId"], "hotline");
    assert_eq!(
        opened[0].get("endedAt"),
        None,
        "a fresh marker is the open one"
    );
    assert_eq!(
        opened[0].get("sessionId"),
        None,
        "Hotline Agent has no checkpoint"
    );

    room.stop("ada").unwrap();
    room.start("ada").await.unwrap();
    assert_eq!(markers(&room, "ada"), opened);
}

/// An idle teammate has no driver to rebuild; the next start reads the new
/// state anyway.
#[tokio::test]
async fn reattach_on_an_idle_teammate_does_nothing() {
    let agents = Fake::new(Scripted::new(Vec::new()));
    let room = room("reattach-idle", agents.clone());
    room.reattach("ada").await.unwrap();
    assert!(
        lock(&agents.preambles).is_empty(),
        "an idle teammate built a driver"
    );
    assert_eq!(room.info("ada").state, SessionState::Idle);
}

/// Between turns the old driver is cancelled, a new one is built, and the
/// open chapter is the one the new session joins.
#[tokio::test]
async fn reattach_on_a_ready_session_swaps_the_driver_and_keeps_the_chapter() {
    let driver = Scripted::new(Vec::new());
    *lock(&driver.session_id) = Some("s-1".to_string());
    let agents = Fake::new(driver);
    let cancels = agents.driver.cancels.clone();
    let session_id = agents.driver.session_id.clone();
    let room = room("reattach-ready", agents.clone());
    room.start("ada").await.unwrap();
    let opened = markers(&room, "ada");
    assert_eq!(room.info("ada").session_id.as_deref(), Some("s-1"));
    assert_eq!(lock(&agents.preambles).len(), 1);

    // The next start reports a different id, as a new child would.
    *lock(&session_id) = Some("s-2".to_string());
    room.reattach("ada").await.unwrap();

    assert!(*lock(&cancels) > 0, "the old driver saw cancel");
    assert_eq!(lock(&agents.preambles).len(), 2, "a new driver was built");
    assert_eq!(room.info("ada").session_id.as_deref(), Some("s-2"));
    assert_eq!(markers(&room, "ada"), opened, "the chapter stayed open");
}

/// A failed policy write leaves its generation closed. Its retained record
/// must neither claim to be ready nor silently accept work under old grants.
#[tokio::test]
async fn an_unfinished_policy_update_refuses_work_until_reattached() {
    let agents = Fake::new(Scripted::new(Vec::new()));
    let room = room("unfinished-policy", agents.clone());
    room.start("ada").await.unwrap();
    room.invalidate("ada").unwrap();

    assert_eq!(room.info("ada").state, SessionState::Stopped);
    assert!(room.start("ada").await.is_err());
    assert!(
        room.prompt("ada", "must not run", None, None)
            .await
            .is_err()
    );
    assert_eq!(lock(&agents.preambles).len(), 1);

    room.reattach("ada").await.unwrap();
    assert_eq!(room.info("ada").state, SessionState::Ready);
    assert_eq!(lock(&agents.preambles).len(), 2);
}

/// Stop also covers a replacement between detaching its predecessor and
/// entering the driver's startup future. A later explicit start still works.
#[tokio::test]
async fn stop_revokes_a_replacement_before_its_startup_begins() {
    let agents = Fake::new(Scripted::new(Vec::new()));
    let room = room("stop-replacement", agents.clone());
    room.start("ada").await.unwrap();
    let replacement = room.stop_with_capability("ada");
    room.stop("ada").unwrap();

    assert!(room.start_now("ada", replacement).await.is_err());
    assert_eq!(lock(&agents.preambles).len(), 1);
    room.start("ada").await.unwrap();
    assert_eq!(lock(&agents.preambles).len(), 2);
}

/// A routine stop during the wire's invalidate-to-append window keeps the
/// epoch quarantined. A start cannot revive a session with the old policy;
/// reattach is the operation that activates the replacement generation.
#[tokio::test]
async fn stop_during_policy_quarantine_cannot_revive_the_old_generation() {
    let agents = Fake::new(Scripted::new(Vec::new()));
    let room = room("stop-quarantine", agents.clone());
    room.start("ada").await.unwrap();
    let old = lock(&room.sessions).get("ada").cloned().unwrap();

    room.invalidate("ada").unwrap();
    room.stop("ada").unwrap();
    assert!(!old.capability.is_current(), "the old lease was revived");
    assert!(
        room.start("ada").await.is_err(),
        "start reopened an epoch still waiting for the policy append"
    );

    room.reattach("ada").await.unwrap();
    room.start("ada").await.unwrap();
    assert_eq!(lock(&agents.preambles).len(), 2);
}

/// A policy change revokes the turn in flight and drops lines queued behind
/// it. The replacement session is built immediately, so a line already
/// handed to the old session cannot run with its former tools.
#[tokio::test]
async fn reattach_during_a_turn_cancels_the_old_queue_before_rebuilding() {
    let gate = Arc::new(Semaphore::new(0));
    let mut driver = Scripted::turns(vec![
        vec![Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        }],
        vec![Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        }],
    ]);
    driver.gate = Some(gate.clone());
    let agents = Fake::new(driver);
    let prompts = agents.driver.prompts.clone();
    let room = room("reattach-defer", agents.clone());
    room.start("ada").await.unwrap();

    room.prompt("ada", "first", None, None).await.unwrap();
    until_state(&room, "ada", SessionState::Thinking).await;

    room.prompt("ada", "second", None, None).await.unwrap();

    room.reattach("ada").await.unwrap();
    assert_eq!(
        lock(&agents.preambles).len(),
        2,
        "the replacement was built without waiting for the old turn"
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        *lock(&prompts),
        ["first"],
        "the queued line never reached the replacement driver"
    );
}

async fn until_state(room: &Room, persona_id: &str, want: SessionState) {
    for _ in 0..200 {
        if room.info(persona_id).state == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the session never reached {want:?}");
}

/// The gate: a chapter that closed took the agent's context with it, so the
/// next message reaches a session that has never seen it.
#[tokio::test]
async fn a_prompt_after_an_asked_close_starts_a_fresh_session_in_a_new_chapter() {
    let agents = Fake::answering(
        Scripted::turns(vec![
            saying("one", "It jammed."),
            saying("two", "All clear."),
        ]),
        note_json("Crane jam"),
    );
    let room = room("chapter-rotate", agents.clone());
    room.start("ada").await.unwrap();
    room.prompt("ada", "did it jam?", None, None).await.unwrap();
    settled(&room, "ada", 3).await;

    let closed = room
        .start_fresh_chapter("ada", ChapterClose::User)
        .await
        .unwrap();
    assert_eq!(closed.title.as_deref(), Some("Crane jam"));
    assert_eq!(closed.messages, 2);

    room.prompt("ada", "and now?", None, None).await.unwrap();
    settled(&room, "ada", 6).await;

    assert_eq!(
        kinds(&tape(&room, "ada")),
        [
            "chapter", "user", "agent", "turn", "chapter", "user", "agent", "turn"
        ],
        "the new marker opens before the line that lands in it"
    );
    let markers = markers(&room, "ada");
    assert_eq!(markers[0]["closedBy"], "user");
    assert_eq!(markers[1].get("endedAt"), None);

    // Two agents were built, and the second was seeded with nothing: the
    // chapter it joined has no conversation in it yet.
    let seeds = lock(&agents.seeds).clone();
    assert_eq!(seeds.len(), 2);
    assert_eq!(seeds[1], Vec::<Said>::new());
}

/// What the fresh context is told about the conversation it is joining.
#[tokio::test]
async fn the_wake_block_carries_the_previous_chapters_note() {
    let agents = Fake::answering(
        Scripted::turns(vec![
            saying("one", "It jammed."),
            saying("two", "All clear."),
        ]),
        note_json("Crane jam"),
    );
    let room = room("chapter-wake", agents.clone());
    room.start("ada").await.unwrap();
    room.prompt("ada", "did it jam?", None, None).await.unwrap();
    settled(&room, "ada", 3).await;
    room.start_fresh_chapter("ada", ChapterClose::User)
        .await
        .unwrap();
    room.prompt("ada", "and now?", None, None).await.unwrap();
    settled(&room, "ada", 6).await;

    let woken = lock(&agents.preambles)[1].clone();
    assert!(woken.contains("You are Ada."), "{woken}");
    assert!(woken.contains("fresh working context"), "{woken}");
    assert!(
        woken.contains(r#"The previous chapter, "Crane jam", ended"#),
        "{woken}"
    );
    assert!(woken.contains("Goal: Get the crane moving"), "{woken}");
    assert!(woken.contains("Open loops:\n- oil the winch"), "{woken}");
    assert!(
        woken.contains(r#"{"speaker":"user","text":"did it jam?"}"#),
        "{woken}"
    );
}

/// The idle clock: a chapter nobody has said anything in for longer than the
/// room allows closes itself, and one that is still warm is left alone.
#[tokio::test]
async fn the_idle_sweep_closes_a_stale_chapter_and_leaves_a_fresh_one() {
    let log = scratch("chapter-sweep");
    enrol(&log, &persona("ada"));
    enrol(&log, &persona("bob"));
    let stale = now_ms() - 10 * 3_600_000;
    write_tape(
        &log,
        "ada",
        &[
            json!({"kind": "chapter", "id": "c-ada", "ts": stale, "backendId": "hotline"}),
            spoken("user", "u1", stale + 1_000, "did the crane jam?"),
            spoken("agent", "a1", stale + 2_000, "It jammed."),
        ],
    );
    write_tape(
        &log,
        "bob",
        &[
            json!({"kind": "chapter", "id": "c-bob", "ts": now_ms() - 60_000, "backendId": "hotline"}),
            spoken("user", "u2", now_ms() - 30_000, "morning"),
        ],
    );
    let room = Room::with_agents(
        log,
        Arc::new(DeskKeys),
        Fake::answering(Scripted::new(Vec::new()), note_json("Crane jam")),
    );

    room.sweep_chapters(&mut HashMap::new()).await;

    let closed = &markers(&room, "ada")[0];
    assert_eq!(
        closed["endedAt"],
        stale + 2_000,
        "the chapter ended when the conversation stopped, not when the sweep noticed"
    );
    assert_eq!(closed["title"], "Crane jam");
    assert_eq!(
        closed["note"],
        "Goal: Get the crane moving\nOutcome: It moved.\nOpen loops:\n- oil the winch\nFiles: crane.log"
    );
    assert_eq!(closed["status"], "in-progress");
    assert_eq!(closed["tags"], json!(["crane", "harbour"]));
    assert_eq!(closed["closedBy"], "idle");

    assert_eq!(
        markers(&room, "bob")[0].get("endedAt"),
        None,
        "a chapter that heard something a minute ago is not stale"
    );
}

/// A teammate nobody spoke to does not collect empty rules in its drawer.
#[tokio::test]
async fn a_chapter_nobody_spoke_in_closes_without_a_title() {
    let room = room("chapter-empty", Fake::new(Scripted::new(Vec::new())));
    assert!(
        room.start_fresh_chapter("ada", ChapterClose::User)
            .await
            .is_err(),
        "a teammate that never started has no chapter to close"
    );

    room.start("ada").await.unwrap();
    let closed = room
        .start_fresh_chapter("ada", ChapterClose::Agent)
        .await
        .unwrap();

    assert_eq!(closed.title, None);
    assert_eq!(closed.messages, 0);
    let marker = &markers(&room, "ada")[0];
    assert_eq!(marker["closedBy"], "agent");
    assert!(marker["endedAt"].is_i64());
    assert_eq!(marker.get("title"), None);
    assert_eq!(marker.get("status"), None);
}

/// No model, or a model that would not answer: the chapter closes anyway,
/// named after the thing that started it, and the transcript says what is
/// missing rather than leaving the next chapter to start cold in silence.
#[tokio::test]
async fn a_chapter_no_model_would_summarise_closes_titled_from_the_first_message() {
    let agents = Fake::new(Scripted::new(saying("one", "It jammed.")));
    let room = room("chapter-no-note", agents);
    room.start("ada").await.unwrap();
    room.prompt("ada", "did the crane jam?", None, None)
        .await
        .unwrap();
    settled(&room, "ada", 3).await;

    let closed = room
        .start_fresh_chapter("ada", ChapterClose::User)
        .await
        .unwrap();
    assert_eq!(closed.title.as_deref(), Some("did the crane jam?"));
    assert_eq!(closed.note, None);
    assert_eq!(closed.status, Some(ChapterStatus::Done));

    let events = tape(&room, "ada");
    let notice = events.last().expect("something was written");
    assert_eq!(notice["kind"], "notice");
    assert_eq!(notice["level"], "warn");
    assert!(
        notice["text"]
            .as_str()
            .unwrap()
            .contains("without a handoff note"),
        "{notice}"
    );
}

/// The last process's open cards are expired and the tape folded before the
/// room serves anything from it, and the index knows the tape afterwards.
#[tokio::test(flavor = "multi_thread")]
async fn opening_the_room_expires_orphaned_cards_and_indexes_the_tape() {
    let log = scratch("settle");
    let ada = persona("ada");
    enrol(&log, &ada);
    let tape = StreamId::Tape("ada".into());
    log.append(
        &tape,
        &json!({"kind": "user", "id": "u1", "ts": 1, "text": "harbour"}),
    )
    .unwrap();
    log.append(
        &tape,
        &json!({"kind": "permission", "id": "p1", "ts": 2, "requestId": "req", "title": "read a file", "options": []}),
    )
    .unwrap();
    log.append(
        &tape,
        &json!({"kind": "user", "id": "u1", "ts": 1, "text": "harbour again"}),
    )
    .unwrap();

    let _room = Room::new(log.clone(), Arc::new(DeskKeys));

    let events = log.load(&tape);
    assert_eq!(events.len(), 2);
    assert_eq!(events[1]["decision"], "expired");
    // Compacted: the superseded line is gone from the file itself.
    let lines =
        std::fs::read_to_string(crate::paths::transcript_segment_path(log.root(), "ada", 1))
            .unwrap();
    assert_eq!(lines.lines().count(), 2);
    let found = crate::store::search::search(log.root(), "ada", "harbour", None);
    assert_eq!(found["hits"].as_array().unwrap().len(), 1);
}

/// The card a permission writes, and the decision that supersedes it.
///
/// One tape event, written twice by the same id: a fold shows the decided card
/// where the pending one stood, which is what "non-dismissable, and then
/// answered" looks like in an append-only file.
#[tokio::test]
async fn a_permission_is_one_card_that_the_answer_supersedes() {
    let gate = Arc::new(Semaphore::new(0));
    let mut driver = Scripted::new(vec![
        Update::Permission {
            request_id: "r1".to_string(),
            title: "Run rm -rf /".to_string(),
            options: vec![
                PermissionOption {
                    option_id: "once".to_string(),
                    name: "Allow once".to_string(),
                    kind: Some("allow_once".to_string()),
                },
                PermissionOption {
                    option_id: "never".to_string(),
                    name: "Deny".to_string(),
                    kind: None,
                },
            ],
        },
        Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        },
    ]);
    driver.gate = Some(gate.clone());
    // The driver is holding this request open, the way a live agent would.
    lock(&driver.waiting).push("r1".to_string());
    let room = room("permission", Fake::new(driver));
    room.start("ada").await.unwrap();

    room.prompt("ada", "clean up", None, None).await.unwrap();
    gate.add_permits(1);
    let events = settled(&room, "ada", 2).await;
    assert_eq!(kinds(&events), ["user", "permission"]);
    assert_eq!(events[1]["id"], "perm:r1");
    assert_eq!(events[1]["title"], "Run rm -rf /");
    assert_eq!(events[1]["options"][0]["name"], "Allow once");
    assert!(events[1].get("decision").is_none(), "the card is live");

    room.answer_permission("ada", "r1", "once").await.unwrap();
    let events = settled(&room, "ada", 2).await;
    assert_eq!(kinds(&events), ["user", "permission"], "one card, not two");
    assert_eq!(events[1]["decision"], "once");
    assert_eq!(events[1]["decidedOptionName"], "Allow once");

    // Nothing is behind that button now, so a second answer is refused rather
    // than quietly letting an agent through.
    assert!(room.answer_permission("ada", "r1", "never").await.is_err());
}

/// A card the turn ended without an answer to is settled, because the agent
/// has stopped waiting and a button with nothing behind it is a lie.
#[tokio::test]
async fn a_card_the_turn_left_open_is_expired_when_the_turn_ends() {
    let driver = Scripted::new(vec![
        Update::Permission {
            request_id: "r1".to_string(),
            title: "Edit the harbour log".to_string(),
            options: vec![PermissionOption {
                option_id: "once".to_string(),
                name: "Allow once".to_string(),
                kind: None,
            }],
        },
        Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        },
    ]);
    lock(&driver.waiting).push("r1".to_string());
    let room = room("orphaned", Fake::new(driver));
    room.start("ada").await.unwrap();

    room.prompt("ada", "tidy", None, None).await.unwrap();
    let events = settled(&room, "ada", 3).await;
    assert_eq!(kinds(&events), ["user", "permission", "turn"]);
    assert_eq!(events[1]["decision"], "expired");
}

/// The agent's own id for the conversation is remembered on the teammate's
/// record — once, after a turn has completed on it, and one entry per backend.
#[tokio::test]
async fn a_fresh_sessions_id_is_remembered_after_the_turn_that_proves_it() {
    let agents = Fake::new(Scripted::turns(vec![
        vec![Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        }],
        vec![Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        }],
    ]));
    *lock(&agents.driver.session_id) = Some("s-1".to_string());
    let log = scratch("checkpoint");
    let mut ada = persona("ada");
    ada.backend_id = "cursor".to_string();
    // A checkpoint another harness left is not this one's to touch.
    ada.session_checkpoints = vec![SessionCheckpoint {
        backend_id: "opencode".to_string(),
        session_id: "elsewhere".to_string(),
    }];
    enrol(&log, &ada);
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents);
    room.start("ada").await.unwrap();

    // Nothing is written before a turn has run on the session: some agents
    // issue an id they cannot reopen until a prompt has committed.
    assert_eq!(checkpoints(&room, "ada"), ["opencode/elsewhere"]);

    room.prompt("ada", "hello", None, None).await.unwrap();
    settled(&room, "ada", 2).await;
    for _ in 0..200 {
        if checkpoints(&room, "ada").len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        checkpoints(&room, "ada"),
        ["opencode/elsewhere", "cursor/s-1"]
    );
    assert_eq!(
        markers(&room, "ada")[0]["sessionId"],
        "s-1",
        "the marker carries the id a resume will point back at"
    );

    // Closing the chapter withdraws this backend's checkpoint and leaves the
    // other harness's conversation alone.
    room.start_fresh_chapter("ada", ChapterClose::User)
        .await
        .unwrap();
    assert_eq!(checkpoints(&room, "ada"), ["opencode/elsewhere"]);
}

/// Resume reopens only the chapter immediately before: the interim closes as
/// a turning point, the new marker carries the earlier note, Hotline Agent is
/// seeded from that chapter's words, and the user lines said in the meantime
/// arrive as a nudge — never a line of the tape.
#[tokio::test]
async fn resume_reopens_the_previous_chapter_and_nudges_with_what_was_said_since() {
    let agents = Fake::new(Scripted::new(vec![Update::Turn {
        stop_reason: "end_turn".to_string(),
        usage: None,
    }]));
    let prompts = agents.driver.prompts.clone();
    let log = scratch("chapter-resume");
    enrol(&log, &persona("ada"));
    let t = now_ms();
    write_tape(
        &log,
        "ada",
        &[
            json!({
                "kind": "chapter", "id": "c1", "ts": t - 10_000, "backendId": "hotline",
                "sessionId": "s-old", "endedAt": t - 5_000, "title": "Crane jam",
                "note": "Goal: Get the crane moving", "status": "in-progress",
                "closedBy": "idle",
            }),
            spoken("user", "u1", t - 9_000, "did the crane jam?"),
            spoken("agent", "a1", t - 8_000, "It jammed."),
            json!({"kind": "chapter", "id": "c2", "ts": t - 1_000, "backendId": "hotline"}),
            spoken("user", "u2", t - 500, "and now?"),
        ],
    );
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
    room.start("ada").await.unwrap();

    let answered = TeammateTools::new(&room, "ada")
        .call("resume_chapter", &json!({}))
        .await
        .expect("resume_chapter is a teammate tool");
    let resumed: Value = serde_json::from_str(&answered).unwrap();
    assert_eq!(resumed["resumed"], json!(true));
    assert_eq!(resumed["title"], "Crane jam");

    let markers = markers(&room, "ada");
    assert_eq!(markers.len(), 3);
    assert_eq!(markers[1]["title"], "Back to: Crane jam");
    assert_eq!(markers[1]["closedBy"], "resume");
    assert_eq!(markers[1]["status"], "done");
    assert_eq!(markers[2]["resumedFrom"], "c1");
    assert_eq!(markers[2]["note"], "Goal: Get the crane moving");
    assert_eq!(markers[2]["sessionId"], "s-old");
    assert_eq!(
        markers[2].get("endedAt"),
        None,
        "the reopened marker is the open chapter"
    );

    let seeds = lock(&agents.seeds).clone();
    assert_eq!(seeds.len(), 2, "start, then the resume's restart");
    assert_eq!(
        seeds[1],
        vec![
            Said::User("did the crane jam?".to_string()),
            Said::Agent("It jammed.".to_string()),
        ]
    );

    let nudged = heard(&prompts, 1).await;
    assert!(
        nudged[0].contains("and now?"),
        "the nudge carries the interim user line: {}",
        nudged[0]
    );
    assert!(
        nudged[0].contains("<hotline_user_messages>"),
        "{}",
        nudged[0]
    );
    let users: Vec<Value> = tape(&room, "ada")
        .into_iter()
        .filter(|event| event["kind"] == "user")
        .collect();
    assert_eq!(
        users.len(),
        2,
        "the nudge is Hotline's words, never a line of the tape"
    );

    let refused = room.resume_chapter("ada").await.unwrap_err();
    assert!(
        refused.contains("no previous chapter"),
        "a second resume is refused when the chapter immediately before closed by resume: {refused}"
    );
}

/// An ACP resume points the persona's checkpoint back at the previous
/// chapter's session, which is what the child's resume/load path will try.
#[tokio::test]
async fn resume_points_the_checkpoint_back_at_the_previous_chapters_session() {
    // No Turn: a turn would remember the scripted child's new session id
    // and overwrite the checkpoint this test is about.
    let agents = Fake::new(Scripted::new(Vec::new()));
    *lock(&agents.driver.session_id) = Some("s-new".to_string());
    let log = scratch("chapter-resume-checkpoint");
    let mut ada = persona("ada");
    ada.backend_id = "cursor".to_string();
    ada.session_checkpoints = vec![SessionCheckpoint {
        backend_id: "cursor".to_string(),
        session_id: "s-new".to_string(),
    }];
    enrol(&log, &ada);
    let t = now_ms();
    write_tape(
        &log,
        "ada",
        &[
            json!({
                "kind": "chapter", "id": "c1", "ts": t - 10_000, "backendId": "cursor",
                "sessionId": "s-old", "endedAt": t - 5_000, "title": "Crane jam",
                "note": "Goal: Get the crane moving", "closedBy": "idle",
            }),
            spoken("user", "u1", t - 9_000, "did the crane jam?"),
            json!({"kind": "chapter", "id": "c2", "ts": t - 1_000, "backendId": "cursor"}),
            spoken("user", "u2", t - 500, "and now?"),
        ],
    );
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents);
    room.start("ada").await.unwrap();
    room.resume_chapter("ada").await.unwrap();

    assert_eq!(checkpoints(&room, "ada"), ["cursor/s-old"]);
    let notices: Vec<Value> = tape(&room, "ada")
        .into_iter()
        .filter(|event| event["kind"] == "notice")
        .collect();
    assert!(
        notices.iter().any(|notice| notice["text"]
            .as_str()
            .unwrap()
            .contains("could not be reopened")),
        "a scripted child does not restore, so the tape says so: {notices:?}"
    );
}

/// The teammate's checkpoints as `backend/session`, oldest first.
fn checkpoints(room: &Room, persona_id: &str) -> Vec<String> {
    room::roster(&room.log)
        .into_iter()
        .find(|persona| persona.id == persona_id)
        .map(|persona| {
            persona
                .session_checkpoints
                .into_iter()
                .map(|checkpoint| format!("{}/{}", checkpoint.backend_id, checkpoint.session_id))
                .collect()
        })
        .unwrap_or_default()
}

/// A pending `human_action` card's action id, once it has landed.
async fn pending_human(room: &Room, persona_id: &str) -> String {
    for _ in 0..200 {
        if let Some(id) = tape(room, persona_id).into_iter().find_map(|event| {
            (event["kind"] == "human_action" && event["status"] == "pending")
                .then(|| event["actionId"].as_str().map(str::to_string))
                .flatten()
        }) {
            return id;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no pending human_action landed");
}

/// The agent asked the person; the answer flips the same card and unblocks
/// the tool with the sentence the agent reads.
#[tokio::test]
async fn request_human_writes_a_pending_card_and_the_answer_flips_it_to_done() {
    let room = room("human-done", Fake::new(Scripted::new(vec![])));
    let tools = TeammateTools::new(&room, "ada");
    let waiting = {
        let tools = tools.clone();
        tokio::spawn(async move {
            tools
                .call("request_human", &json!({ "reason": "Tap the 2FA prompt" }))
                .await
        })
    };
    let action_id = pending_human(&room, "ada").await;
    room.answer_human("ada", &action_id, HumanAnswer::Done, Some("  ".to_string()))
        .unwrap();
    let text = waiting.await.unwrap().unwrap();
    assert_eq!(text, "The person did it.");

    let card = tape(&room, "ada")
        .into_iter()
        .find(|event| event["kind"] == "human_action")
        .expect("the card is on the tape");
    assert_eq!(card["id"], format!("human:{action_id}"));
    assert_eq!(card["status"], "done");
    assert_eq!(card["reason"], "Tap the 2FA prompt");
    assert!(
        room.answer_human("ada", &action_id, HumanAnswer::Declined, None)
            .is_err()
    );
}

/// The person's note reaches the agent word for word, on the sentence and
/// on the card, so a declined card says why and a card that asked a
/// question gets its answer.
#[tokio::test]
async fn a_declined_human_request_carries_the_note() {
    let room = room("human-declined", Fake::new(Scripted::new(vec![])));
    let tools = TeammateTools::new(&room, "ada");
    let waiting = {
        let tools = tools.clone();
        tokio::spawn(async move {
            tools
                .call(
                    "request_human",
                    &json!({ "reason": "Enter the vault password" }),
                )
                .await
        })
    };
    let action_id = pending_human(&room, "ada").await;
    room.answer_human(
        "ada",
        &action_id,
        HumanAnswer::Declined,
        Some(" It is in the shared vault, use that. ".to_string()),
    )
    .unwrap();
    let text = waiting.await.unwrap().unwrap();
    assert_eq!(
        text,
        "The person declined. They said: It is in the shared vault, use that."
    );
    let card = tape(&room, "ada")
        .into_iter()
        .find(|event| event["kind"] == "human_action")
        .expect("the card is on the tape");
    assert_eq!(card["status"], "dismissed");
    assert_eq!(card["note"], "It is in the shared vault, use that.");
}

/// The deadline is injectable so a test does not sit for ten minutes.
#[tokio::test]
async fn a_human_request_expires_when_nobody_answers() {
    let room = room("human-timeout", Fake::new(Scripted::new(vec![])));
    let text = room
        .request_human("ada", "Tap 2FA", Duration::from_millis(20))
        .await
        .unwrap();
    assert_eq!(text, "Nobody answered in ten minutes.");
    let card = tape(&room, "ada")
        .into_iter()
        .find(|event| event["kind"] == "human_action")
        .expect("the card is on the tape");
    assert_eq!(card["status"], "expired");
}

/// A card left pending when the process died is a button nobody is behind.
#[tokio::test]
async fn a_human_action_left_pending_expires_when_the_room_opens() {
    let log = scratch("human-stale");
    enrol(&log, &persona("ada"));
    log.append(
        &StreamId::Tape("ada".into()),
        &json!({
            "kind": "human_action",
            "id": "human:stale",
            "ts": 1000,
            "actionId": "stale",
            "reason": "log in",
            "status": "pending",
        }),
    )
    .unwrap();
    let room = Room::with_agents(log, Arc::new(DeskKeys), Fake::new(Scripted::new(vec![])));
    let card = tape(&room, "ada")
        .into_iter()
        .find(|event| event["kind"] == "human_action")
        .expect("the card is still on the tape");
    assert_eq!(card["status"], "expired");
    assert_eq!(card["id"], "human:stale");
}

/// A `persona` line with an empty id is a half-written record, not a
/// teammate: nothing could name its tape. The room opens over it rather than
/// taking the whole roster down when the startup fold reaches for that tape.
#[tokio::test]
async fn a_teammate_with_no_id_does_not_stop_the_room_opening() {
    let log = scratch("empty-id-settle");
    let mut nobody = persona("");
    nobody.name = "Nobody".to_string();
    enrol(&log, &nobody);
    enrol(&log, &persona("ada"));

    let room = Room::with_agents(
        log,
        Arc::new(DeskKeys),
        Fake::new(Scripted::new(Vec::new())),
    );
    assert!(room.persona("ada").is_ok());
    assert!(room.persona("").is_err());
}

/// A permission raised inside a peer turn lives on the thread and nowhere
/// else, and no seat is shown one. A card that outlived the process that
/// received it is a button nobody is behind, on a stream nothing else
/// revisits.
#[tokio::test]
async fn a_card_left_open_on_a_thread_expires_when_the_room_opens() {
    let log = scratch("thread-settle");
    enrol(&log, &persona("ada"));
    let key = crate::paths::thread_key("ada", "bob").expect("a key for the pair");
    crate::log::thread::ensure(log.root(), &key).unwrap();
    log.append(
        &StreamId::Thread(key.clone()),
        &json!({
            "kind": "permission",
            "id": "perm:req-1",
            "ts": 1000,
            "requestId": "req-1",
            "title": "read a file",
            "options": [{"optionId": "allow", "name": "Allow", "kind": "allow_once"}],
        }),
    )
    .unwrap();

    let room = Room::with_agents(
        log,
        Arc::new(DeskKeys),
        Fake::new(Scripted::new(Vec::new())),
    );
    let card = room
        .log
        .load(&StreamId::Thread(key))
        .into_iter()
        .find(|event| event["kind"] == "permission")
        .expect("the card is still on the thread");
    assert_eq!(card["decision"], "expired");
    assert_eq!(card["id"], "perm:req-1");
}

/// The tool that asked the person is inside the turn, so a cancelled turn is
/// an agent that has stopped listening. The card goes with it: a live button
/// that writes `done` for nobody is the failure these cards exist to avoid.
#[tokio::test]
async fn cancelling_a_turn_takes_the_card_it_asked_the_person_with() {
    let room = room("human-cancel", Fake::new(Scripted::new(vec![])));
    room.start("ada").await.unwrap();
    let tools = TeammateTools::new(&room, "ada");
    let waiting = {
        let tools = tools.clone();
        tokio::spawn(async move {
            tools
                .call("request_human", &json!({ "reason": "Tap the 2FA prompt" }))
                .await
        })
    };
    let action_id = pending_human(&room, "ada").await;

    room.cancel("ada").unwrap();

    let released = tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .expect("the tool was left parked on a turn that had been cancelled")
        .unwrap()
        .unwrap();
    assert_eq!(released, "Nobody answered in ten minutes.");
    let card = tape(&room, "ada")
        .into_iter()
        .find(|event| event["kind"] == "human_action")
        .expect("the card is on the tape");
    assert_eq!(card["status"], "expired");
    assert!(
        room.answer_human("ada", &action_id, HumanAnswer::Done, None)
            .is_err(),
        "a card nobody is behind still took an answer"
    );
}

#[tokio::test]
async fn updates_wait_for_queued_work_and_release_the_room_after_failure() {
    let semaphore = Arc::new(Semaphore::new(0));
    let mut driver = Scripted::new(vec![Update::Message {
        kind: MessageKind::Agent,
        id: "answer".into(),
        text: "done".into(),
    }]);
    driver.gate = Some(semaphore.clone());
    let room = room("update-lease", Fake::new(driver));
    room.start("ada").await.unwrap();
    room.prompt("ada", "first", None, None).await.unwrap();
    room.prompt("ada", "queued", None, None).await.unwrap();
    assert!(
        room.prepare_restart()
            .unwrap_err()
            .contains("still working")
    );
    semaphore.add_permits(2);
    let held = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(held) = room.prepare_restart() {
                break held;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let before = tape(&room, "ada");
    assert!(
        room.prompt("ada", "must not be lost", None, None)
            .await
            .unwrap_err()
            .contains("restart")
    );
    assert!(room.start("ada").await.unwrap_err().contains("restart"));
    assert!(
        room.nudge("ada", "new work")
            .unwrap_err()
            .contains("restart")
    );
    assert_eq!(before, tape(&room, "ada"));
    // Failed installation/cancellation drops its lease, restoring normal admission.
    drop(held);
    semaphore.add_permits(1);
    room.prompt("ada", "retry", None, None).await.unwrap();
}
