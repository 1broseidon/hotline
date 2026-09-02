//! The funnel, driven by a scripted driver.
//!
//! Nothing here talks to a model. What is under test is the part that would
//! be written twice if the session did not own it: what a driver update means
//! on the tape, in what order, and what the room says about the session while
//! it happens.

use super::*;
use crate::contract::{AttachmentKind, McpPolicy, PolicyMode};
use crate::driver::DriverInfo;
use async_trait::async_trait;
use serde_json::json;
use std::time::Duration;
use tokio::sync::{Notify, Semaphore, mpsc};

/// A driver that says what it was told to say.
///
/// Each prompt replays the next script, one update at a time, waiting for
/// `gate` between updates when the test asked for a pause it can cancel in.
/// The last script stands for every turn after it, so a test that does not
/// care which turn it is in writes one.
struct Scripted {
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
    reaches: Arc<Mutex<Vec<Reach>>>,
}

impl Scripted {
    fn new(script: Vec<Update>) -> Self {
        Self::turns(vec![script])
    }

    fn turns(turns: Vec<Vec<Update>>) -> Self {
        Self {
            turns,
            asked: Arc::new(Mutex::new(0)),
            gate: None,
            on_cancel: Vec::new(),
            cancelled: Arc::new(Notify::new()),
            prompts: Arc::new(Mutex::new(Vec::new())),
            reaches: Arc::new(Mutex::new(Vec::new())),
        }
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
        })
    }

    async fn prompt(&self, text: String, reach: Reach) -> mpsc::Receiver<Update> {
        lock(&self.prompts).push(text);
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
        self.cancelled.notify_one();
    }

    async fn set_model(&self, model_id: &str) -> Result<DriverInfo, String> {
        let mut info = self.start(&persona("x")).await?;
        info.current_model_id = model_id.to_string();
        info.model_label = None;
        Ok(info)
    }
}

/// No key anywhere: nothing in these tests reaches a provider.
struct NoKeys;

impl ProviderKeys for NoKeys {
    fn provider_keys(&self) -> HashMap<String, String> {
        HashMap::new()
    }
}

fn scratch(name: &str) -> Log {
    let root = std::env::temp_dir().join(format!(
        "toad-core-session-{name}-{}-{}",
        std::process::id(),
        now_ms()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    Log::open(root)
}

fn persona(id: &str) -> Persona {
    Persona {
        node: None,
        id: id.to_string(),
        name: "Ada".to_string(),
        goal: "Keep the harbour running.".to_string(),
        face: None,
        team: None,
        backend_id: "pi".to_string(),
        cwd: std::env::temp_dir().to_string_lossy().to_string(),
        reach: Some(Reach::Machine),
        model_id: None,
        mode_id: None,
        harness_override: None,
        hop_notice: None,
        mcp_policy: McpPolicy {
            mode: PolicyMode::All,
            server_ids: Vec::new(),
        },
        web_search_policy: None,
        computer: None,
        subagents: None,
        session_checkpoints: Vec::new(),
        last_session_id: None,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
    }
}

fn enrol(log: &Log, persona: &Persona) {
    let mut event = serde_json::to_value(persona).unwrap();
    event
        .as_object_mut()
        .unwrap()
        .insert("kind".into(), "persona".into());
    log.append(&StreamId::Room, &event).unwrap();
}

/// A room with one teammate enrolled and no session started.
fn room(name: &str) -> (Arc<Room>, Persona) {
    let log = scratch(name);
    let ada = persona("ada");
    enrol(&log, &ada);
    (Room::new(log, Arc::new(NoKeys)), ada)
}

fn tape(room: &Room, persona_id: &str) -> Vec<Value> {
    room.log.load(&StreamId::Tape(persona_id.to_string()))
}

/// The event kinds on the tape, in the order they were written.
fn kinds(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .map(|event| event["kind"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// Waits for the tape to hold `count` events, so a test never races the task
/// the turn runs on.
async fn settled(room: &Room, persona_id: &str, count: usize) -> Vec<Value> {
    for _ in 0..200 {
        let events = tape(room, persona_id);
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
    let (room, ada) = room("a-turn");
    let mut infos = room.subscribe_info();
    let mut deltas = room.subscribe_deltas();
    room.start_on(&ada, Arc::new(Scripted::new(spoken_turn())))
        .await
        .unwrap();

    room.prompt("ada", "what is here?", None, None).unwrap();
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

    // The deltas went out and were never written down.
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
            StreamDelta::AgentDelta {
                persona_id: "ada".to_string(),
                message_id: "m2".to_string(),
                text: "one file".to_string(),
            },
        ]
    );
}

#[tokio::test]
async fn a_cancelled_turn_fails_the_tool_it_caught_in_flight() {
    let (room, ada) = room("cancel");
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
    room.start_on(&ada, Arc::new(driver)).await.unwrap();

    room.prompt("ada", "run the long thing", None, None)
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
    let (room, ada) = room("queue");
    let gate = Arc::new(Semaphore::new(0));
    let mut driver = Scripted::new(vec![Update::Turn {
        stop_reason: "end_turn".to_string(),
        usage: None,
    }]);
    driver.gate = Some(gate.clone());
    let prompts = driver.prompts.clone();
    let reaches = driver.reaches.clone();
    room.start_on(&ada, Arc::new(driver)).await.unwrap();

    room.prompt("ada", "first", None, None).unwrap();
    room.prompt("ada", "second", None, None).unwrap();

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

#[tokio::test]
async fn a_teammate_with_no_session_is_idle_and_a_started_one_reports_its_driver() {
    let (room, ada) = room("info");
    assert_eq!(room.info("ada"), idle_info("ada"));
    assert_eq!(room.info("ada").state, SessionState::Idle);

    let info = room
        .start_on(&ada, Arc::new(Scripted::new(Vec::new())))
        .await
        .unwrap();
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
    assert!(room.prompt("ada", "anyone there?", None, None).is_err());
}

/// A teammate's directory is made when it starts, wherever it was pointed.
#[tokio::test(flavor = "multi_thread")]
async fn starting_a_teammate_makes_its_working_directory() {
    struct OneKey;
    impl ProviderKeys for OneKey {
        fn provider_keys(&self) -> HashMap<String, String> {
            HashMap::from([("anthropic".to_string(), "not-a-real-key".to_string())])
        }
    }
    let log = scratch("makes-cwd");
    let mut ada = persona("ada");
    ada.cwd = log
        .root()
        .join("workspaces")
        .join("ada")
        .to_string_lossy()
        .into_owned();
    enrol(&log, &ada);
    let room = Room::new(log, Arc::new(OneKey));
    assert!(!std::path::Path::new(&ada.cwd).exists());
    room.start("ada").await.unwrap();
    assert!(std::path::Path::new(&ada.cwd).is_dir());
}

/// The preamble is everything the agent would otherwise have to ask for.
#[test]
fn the_preamble_says_who_where_how_far_and_when() {
    let mut ada = persona("ada");
    ada.cwd = "/tmp/harbour".to_string();
    let walled = preamble(&ada, Reach::Workspace);
    assert!(walled.contains("You are Ada."));
    assert!(walled.contains("Keep the harbour running."));
    assert!(walled.contains("Your working directory is /tmp/harbour."));
    assert!(walled.contains("a path that leaves it is refused"));
    assert!(walled.contains(&Local::now().format("%A %-d %B %Y").to_string()));

    let open = preamble(&ada, Reach::Machine);
    assert!(open.contains("reach the whole machine"));
}

/// A teammate whose backend has no driver in this build is told so, rather
/// than quietly started on a different agent than the one it names.
#[tokio::test]
async fn a_backend_with_no_driver_is_refused_by_name() {
    let log = scratch("backend");
    let mut cursor = persona("cursor-teammate");
    cursor.backend_id = "cursor".to_string();
    enrol(&log, &cursor);
    let room = Room::new(log, Arc::new(NoKeys));

    let refused = room.start("cursor-teammate").await.unwrap_err();
    assert!(refused.contains("cursor"), "{refused}");
}

/// The history a driver starts back into is what was said, and only that.
#[test]
fn the_conversation_a_driver_is_seeded_with_is_the_words_on_the_tape() {
    let log = scratch("said");
    let ada = StreamId::Tape("ada".to_string());
    for event in [
        json!({"kind": "user", "id": "u1", "ts": 1, "text": "hello"}),
        json!({"kind": "thought", "id": "t1", "ts": 2, "text": "hmm"}),
        json!({"kind": "tool", "id": "tool:c1", "ts": 3, "toolCallId": "c1", "title": "ls", "status": "completed"}),
        json!({"kind": "agent", "id": "a1", "ts": 4, "text": "hi"}),
    ] {
        log.append(&ada, &event).unwrap();
    }
    let room = Room::new(log, Arc::new(NoKeys));

    assert_eq!(
        room.said("ada"),
        [
            Said::User("hello".to_string()),
            Said::Agent("hi".to_string())
        ]
    );
}

/// The index is written as the tape is, and a search finds the turn without
/// anything having asked for a rebuild.
#[tokio::test]
async fn what_is_appended_is_indexed() {
    let (room, ada) = room("index");
    room.start_on(&ada, Arc::new(Scripted::new(spoken_turn())))
        .await
        .unwrap();
    room.prompt("ada", "what is here?", None, None).unwrap();
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
        quiet: quiet.then_some(true),
    }
}

/// A turn in which the agent says one thing. `tag` keeps the message id of one
/// turn apart from the next, because a tape folds by id.
fn said(tag: &str, text: &str) -> Vec<Update> {
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
    let (room, ada) = room("quiet-run");
    let mut deltas = room.subscribe_deltas();
    let driver = Scripted::turns(vec![
        said("quiet", "No change — staying silent per protocol."),
        said("loud", "It moved."),
    ]);
    let prompts = driver.prompts.clone();
    room.start_on(&ada, Arc::new(driver)).await.unwrap();

    room.prompt_scheduled(
        "ada",
        "check the order page",
        firing(ScheduleKind::Loop, true),
    )
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

    room.prompt("ada", "and now?", None, None).unwrap();
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
    let (room, ada) = room("quiet-interrupted");
    let gate = Arc::new(Semaphore::new(0));
    let mut driver = Scripted::turns(vec![
        said("quiet", "the scheduled run's own words"),
        said("loud", "It moved."),
    ]);
    driver.gate = Some(gate.clone());
    room.start_on(&ada, Arc::new(driver)).await.unwrap();

    room.prompt_scheduled(
        "ada",
        "check the order page",
        firing(ScheduleKind::Loop, true),
    )
    .unwrap();
    let events = settled(&room, "ada", 1).await;
    assert_eq!(events[0]["scheduled"]["quiet"], true);

    // The window is open and the run's turn is still gated when a person types.
    room.prompt("ada", "wait, what did you find?", None, None)
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
    let (room, ada) = room("scheduled-loud");
    let driver = Scripted::new(said("s", "Standup is at ten."));
    let prompts = driver.prompts.clone();
    room.start_on(&ada, Arc::new(driver)).await.unwrap();

    room.prompt_scheduled(
        "ada",
        "post the standup",
        firing(ScheduleKind::Schedule, false),
    )
    .unwrap();
    let events = settled(&room, "ada", 3).await;

    assert_eq!(kinds(&events), ["user", "agent", "turn"]);
    assert_eq!(events[0]["text"], "post the standup");
    assert_eq!(events[0]["scheduled"]["name"], "Apple order check");
    assert_eq!(events[0]["scheduled"].get("quiet"), None);
    assert_eq!(*lock(&prompts), ["scheduled · post the standup"]);
}

/// What a message answers is on the line it is written as, and a mark that has
/// gone stale is claimed by nothing.
#[tokio::test]
async fn a_reply_is_stamped_on_its_own_line_and_only_while_the_mark_is_fresh() {
    let (room, ada) = room("reply");
    room.start_on(&ada, Arc::new(Scripted::new(Vec::new())))
        .await
        .unwrap();

    room.prompt("ada", "this one", Some("a1".to_string()), None)
        .unwrap();
    let events = settled(&room, "ada", 1).await;
    assert_eq!(events[0]["replyTo"], "a1");

    let session = room.session("ada").unwrap();
    *lock(&session.pending_reply) = Some(Mark {
        value: "a1".to_string(),
        until: now_ms() - 1,
    });
    room.prompt("ada", "and this one", None, None).unwrap();
    let events = settled(&room, "ada", 2).await;
    assert_eq!(
        events[1].get("replyTo"),
        None,
        "an expired mark is not claimed by a later, unrelated message"
    );
}

/// The record keeps what was attached; the agent is given the paths, because
/// it opens a file with its read tool.
#[tokio::test]
async fn attachments_land_on_the_line_and_their_paths_in_what_the_agent_hears() {
    let (room, ada) = room("attachments");
    let driver = Scripted::new(Vec::new());
    let prompts = driver.prompts.clone();
    room.start_on(&ada, Arc::new(driver)).await.unwrap();

    room.prompt(
        "ada",
        "read these",
        None,
        Some(vec![
            attachment("note.txt", "/tmp/note.txt"),
            attachment("shot.png", "/tmp/shot.png"),
        ]),
    )
    .unwrap();

    let events = settled(&room, "ada", 1).await;
    assert_eq!(events[0]["attachments"][0]["name"], "note.txt");
    assert_eq!(events[0]["attachments"][1]["path"], "/tmp/shot.png");
    assert_eq!(
        heard(&prompts, 1).await,
        ["read these\n\nAttached files:\n/tmp/note.txt\n/tmp/shot.png"]
    );
}

/// Toad's own words to a running teammate: the driver hears them, and the
/// conversation has no line saying anybody spoke.
#[tokio::test]
async fn a_nudge_reaches_the_driver_and_never_the_tape() {
    let (room, ada) = room("nudge");
    let driver = Scripted::new(vec![Update::Turn {
        stop_reason: "end_turn".to_string(),
        usage: None,
    }]);
    let prompts = driver.prompts.clone();
    room.start_on(&ada, Arc::new(driver)).await.unwrap();

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
