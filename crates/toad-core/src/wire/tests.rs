//! The wire, driven the way a client drives it: over a real socket.
//!
//! The room behind the door is a stand-in — Phase 0's sessions and vault are
//! being built beside this — so what is proved here is the wire's own half:
//! the token, the framing, the ordering of a subscription, and the roster
//! view the core maintains out of the log.

use super::*;
use crate::contract::{
    ConfigChoice, Credential, CredentialKind, PersonaDraft, SessionCapabilities, SessionState,
};
use crate::{paths, room};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::{MaybeTlsStream, connect_async};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

fn idle(persona_id: &str) -> SessionInfo {
    SessionInfo {
        persona_id: persona_id.to_string(),
        state: SessionState::Idle,
        session_id: None,
        agent_name: None,
        agent_version: None,
        context_restored: false,
        restore_note: None,
        models: Vec::new(),
        current_model_id: None,
        model_label: None,
        modes: Vec::new(),
        current_mode_id: None,
        mode_label: None,
        configs: Vec::new(),
        slash_commands: Vec::new(),
        capabilities: SessionCapabilities {
            load_session: false,
            resume: false,
            fork: false,
            mcp_http: false,
            image: false,
        },
        error: None,
    }
}

fn thinking(persona_id: &str) -> SessionInfo {
    let mut info = idle(persona_id);
    info.state = SessionState::Thinking;
    info
}

/// A room where nothing is running unless a test says otherwise: every
/// session is idle until `set_info` names one, and the vault answers with
/// what it was handed.
struct Quiet {
    infos: broadcast::Sender<SessionInfo>,
    deltas: broadcast::Sender<StreamDelta>,
    states: Mutex<HashMap<String, SessionInfo>>,
}

impl Quiet {
    fn new() -> Self {
        Self {
            infos: broadcast::channel(16).0,
            deltas: broadcast::channel(16).0,
            states: Mutex::new(HashMap::new()),
        }
    }

    fn set_info(&self, info: SessionInfo) {
        self.states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(info.persona_id.clone(), info.clone());
        let _ = self.infos.send(info);
    }
}

#[async_trait::async_trait]
impl RoomHandle for Quiet {
    async fn start(&self, persona_id: &str) -> Result<SessionInfo, String> {
        Ok(idle(persona_id))
    }

    fn stop(&self, _persona_id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn prompt(
        &self,
        _persona_id: &str,
        _text: &str,
        _reply_to: Option<String>,
        _attachments: Option<Vec<crate::contract::Attachment>>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn cancel(&self, _persona_id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn start_fresh_chapter(
        &self,
        _persona_id: &str,
    ) -> Result<crate::contract::ChapterSummary, String> {
        Err("Nothing runs in this room, so nothing has a chapter.".to_string())
    }

    async fn resume_chapter(
        &self,
        _persona_id: &str,
    ) -> Result<crate::contract::ChapterSummary, String> {
        Err("Nothing runs in this room, so nothing has a chapter.".to_string())
    }

    async fn set_model(&self, persona_id: &str, _model_id: &str) -> Result<SessionInfo, String> {
        Ok(idle(persona_id))
    }

    async fn set_mode(&self, persona_id: &str, _mode_id: &str) -> Result<SessionInfo, String> {
        Ok(idle(persona_id))
    }

    fn answer_permission(
        &self,
        _persona_id: &str,
        _request_id: &str,
        _option_id: &str,
    ) -> Result<(), String> {
        Err("Nothing runs in this room, so nothing is waiting.".to_string())
    }

    fn answer_human(
        &self,
        _persona_id: &str,
        _action_id: &str,
        _status: crate::contract::HumanAnswer,
    ) -> Result<(), String> {
        Ok(())
    }

    fn info(&self, persona_id: &str) -> SessionInfo {
        self.states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(persona_id)
            .cloned()
            .unwrap_or_else(|| idle(persona_id))
    }

    fn subscribe_info(&self) -> broadcast::Receiver<SessionInfo> {
        self.infos.subscribe()
    }

    fn subscribe_deltas(&self) -> broadcast::Receiver<StreamDelta> {
        self.deltas.subscribe()
    }

    fn credential_create(
        &self,
        provider_id: &str,
        label: &str,
        _secret: &str,
    ) -> Result<Credential, String> {
        Ok(Credential {
            id: "cred-1".to_string(),
            provider_id: provider_id.to_string(),
            credential_kind: CredentialKind::ApiKey,
            label: label.to_string(),
            revoked: false,
            created_at: 1,
            updated_at: 1,
        })
    }

    fn credential_revoke(&self, _id: &str) -> Result<(), String> {
        Ok(())
    }

    fn credential_delete(&self, _id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn backends(&self) -> Vec<crate::contract::BackendChoice> {
        Vec::new()
    }

    fn credentials(&self) -> Vec<Credential> {
        Vec::new()
    }

    fn models(&self) -> Vec<ConfigChoice> {
        Vec::new()
    }

    fn import(&self, _from: &std::path::Path) -> Result<crate::import::Report, String> {
        Ok(crate::import::Report::default())
    }

    fn teammate_tools(&self, _persona_id: &str) -> Option<crate::contract::TeammateToolLedger> {
        None
    }

    fn schedule_create(
        &self,
        _persona_id: &str,
        _kind: crate::contract::ScheduleKind,
        _when: Option<i64>,
        _every: Option<i64>,
        _prompt: &str,
        _quiet: bool,
    ) -> Result<crate::contract::ScheduledJob, String> {
        Err("Nothing runs in this room, so nothing is scheduled.".to_string())
    }

    fn schedule_list(&self) -> Vec<crate::contract::ScheduledJob> {
        Vec::new()
    }

    fn schedule_cancel(&self, _id: &str) -> Result<(), String> {
        Err("Nothing runs in this room, so nothing is scheduled.".to_string())
    }

    fn schedule_set_quiet(&self, _id: &str, _quiet: bool) -> Result<(), String> {
        Err("Nothing runs in this room, so nothing is scheduled.".to_string())
    }

    fn peer_threads(&self, _persona_id: &str) -> Vec<crate::contract::PeerThreadSummary> {
        Vec::new()
    }

    fn mark_peer_read(&self, _key: &str, _event_ids: &[String]) -> usize {
        0
    }

    fn drop_peer_sessions(&self, _persona_id: &str) {}
}

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("toad-core-wire-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("transcripts")).unwrap();
    root
}

/// A door on a scratch data directory, already serving.
fn door(name: &str) -> (PathBuf, Log, u16) {
    door_with(name, Arc::new(Quiet::new()))
}

fn door_with(name: &str, room: Arc<Quiet>) -> (PathBuf, Log, u16) {
    let root = scratch(name);
    let log = Log::open(&root);
    let door = Door::bind(log.clone(), "desk-token".to_string(), room).unwrap();
    let port = door.port();
    tokio::spawn(door.run());
    (root, log, port)
}

async fn desk(port: u16) -> Socket {
    connect_async(format!("ws://127.0.0.1:{port}/ws?token=desk-token"))
        .await
        .unwrap()
        .0
}

async fn ask(socket: &mut Socket, frame: Value) {
    socket.send(Message::text(frame.to_string())).await.unwrap();
}

async fn heard(socket: &mut Socket) -> Value {
    let message = socket.next().await.unwrap().unwrap();
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}

/// The next frame this test is waiting for. A command's answer and a
/// subscription's frame are queued by different tasks, so a test that wants
/// one of them says which rather than counting.
async fn heard_where(socket: &mut Socket, wanted: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..16 {
        let frame = heard(socket).await;
        if wanted(&frame) {
            return frame;
        }
    }
    panic!("the frame this test was waiting for never arrived");
}

async fn create(socket: &mut Socket, id: i64, name: &str) -> Value {
    ask(
        socket,
        json!({ "id": id, "cmd": "persona.create", "params": { "draft": { "name": name } } }),
    )
    .await;
    let answer = heard_where(socket, |frame| frame["id"] == id).await;
    assert_eq!(answer["ok"], true, "{answer}");
    answer["result"].clone()
}

#[tokio::test]
async fn a_command_is_answered_by_its_id_and_a_command_nobody_has_is_refused() {
    let (_root, _log, port) = door("framing");
    let mut socket = desk(port).await;

    let created = create(&mut socket, 1, "Ada").await;
    assert_eq!(created["name"], "Ada");

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "persona.fly", "params": {} }),
    )
    .await;
    let refused = heard(&mut socket).await;
    assert_eq!(refused["id"], 2);
    assert_eq!(refused["ok"], false);
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("cannot read that command"),
        "{refused}"
    );
}

#[tokio::test]
async fn the_desk_token_opens_the_door_and_nothing_else_does() {
    let (_root, _log, port) = door("token");

    let wrong = connect_async(format!("ws://127.0.0.1:{port}/ws?token=not-it"))
        .await
        .err()
        .unwrap();
    assert_eq!(status_of(wrong), Some(StatusCode::UNAUTHORIZED));

    let elsewhere = connect_async(format!("ws://127.0.0.1:{port}/pair?token=desk-token"))
        .await
        .err()
        .unwrap();
    assert_eq!(status_of(elsewhere), Some(StatusCode::NOT_FOUND));
}

fn status_of(error: Error) -> Option<StatusCode> {
    match error {
        Error::Http(response) => Some(response.status()),
        _ => None,
    }
}

#[tokio::test]
async fn a_subscription_is_the_fold_and_then_every_event_that_lands_after_it() {
    let (_root, log, port) = door("subscribe");
    let mut socket = desk(port).await;

    let already = json!({ "kind": "setting", "id": "chapterIdleHours", "value": 2 });
    log.append(&StreamId::Room, &already).unwrap();

    ask(&mut socket, json!({ "id": 7, "sub": "room" })).await;
    assert_eq!(heard(&mut socket).await, json!({ "id": 7, "ok": true }));
    assert_eq!(
        heard(&mut socket).await,
        json!({ "sub": 7, "snapshot": [already] })
    );

    let later = json!({ "kind": "setting", "id": "theme", "value": "dark" });
    log.append(&StreamId::Room, &later).unwrap();
    assert_eq!(
        heard(&mut socket).await,
        json!({ "sub": 7, "event": later })
    );

    ask(&mut socket, json!({ "id": 8, "unsub": 7 })).await;
    assert_eq!(heard(&mut socket).await, json!({ "id": 8, "ok": true }));
}

#[tokio::test]
async fn the_roster_is_one_row_per_living_teammate_and_a_tombstone_takes_one_away() {
    let (_root, _log, port) = door("roster");
    let mut socket = desk(port).await;

    let ada = create(&mut socket, 1, "Ada").await;
    let bob = create(&mut socket, 2, "Bob").await;

    ask(&mut socket, json!({ "id": 3, "sub": { "view": "roster" } })).await;
    let snapshot = heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    let rows = snapshot["snapshot"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["persona"], ada);
    assert_eq!(rows[0]["session"]["state"], "idle");
    assert!(rows[0]["preview"].is_null(), "{snapshot}");
    assert_eq!(rows[1]["persona"], bob);

    ask(
        &mut socket,
        json!({ "id": 4, "cmd": "persona.delete", "params": { "id": bob["id"] } }),
    )
    .await;
    let removed = heard_where(&mut socket, |frame| frame["removed"].is_string()).await;
    assert_eq!(removed, json!({ "sub": 3, "removed": bob["id"] }));
}

#[tokio::test]
async fn a_line_in_a_tape_is_the_rosters_preview_of_that_teammate() {
    let (_root, log, port) = door("roster-preview");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let persona_id = ada["id"].as_str().unwrap().to_string();

    ask(&mut socket, json!({ "id": 2, "sub": { "view": "roster" } })).await;
    heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;

    log.append(
        &StreamId::Tape(persona_id.clone()),
        &json!({ "kind": "user", "id": "u1", "ts": 5, "text": "morning" }),
    )
    .unwrap();

    let row = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(
        row["event"]["preview"],
        json!({ "from": "me", "text": "morning", "at": 5 })
    );
    assert_eq!(row["event"]["latest"], 5);
}

#[tokio::test]
async fn the_rosters_latest_is_the_last_message_ts_and_a_tool_does_not_move_it() {
    let (_root, log, port) = door("roster-latest");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let persona_id = ada["id"].as_str().unwrap().to_string();

    ask(&mut socket, json!({ "id": 2, "sub": { "view": "roster" } })).await;
    heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;

    log.append(
        &StreamId::Tape(persona_id.clone()),
        &json!({ "kind": "user", "id": "u1", "ts": 5, "text": "morning" }),
    )
    .unwrap();
    let first = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(first["event"]["latest"], 5);

    log.append(
        &StreamId::Tape(persona_id.clone()),
        &json!({
            "kind": "tool", "id": "t1", "ts": 8, "toolCallId": "c1",
            "title": "Read main.rs", "status": "in_progress",
        }),
    )
    .unwrap();
    let tool = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(tool["event"]["latest"], 5);
    assert!(tool["event"].get("activity").is_none(), "{tool}");

    log.append(
        &StreamId::Tape(persona_id),
        &json!({ "kind": "agent", "id": "a1", "ts": 9, "text": "on it" }),
    )
    .unwrap();
    let second = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(second["event"]["latest"], 9);
}

#[tokio::test]
async fn a_tool_in_progress_is_the_rosters_activity_while_the_session_is_thinking() {
    let quiet = Arc::new(Quiet::new());
    let (_root, log, port) = door_with("roster-activity", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let persona_id = ada["id"].as_str().unwrap().to_string();

    ask(&mut socket, json!({ "id": 2, "sub": { "view": "roster" } })).await;
    heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;

    quiet.set_info(thinking(&persona_id));
    let thinking_row = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(thinking_row["event"]["session"]["state"], "thinking");
    assert!(
        thinking_row["event"].get("activity").is_none(),
        "{thinking_row}"
    );

    log.append(
        &StreamId::Tape(persona_id.clone()),
        &json!({
            "kind": "tool", "id": "t1", "ts": 10, "toolCallId": "c1",
            "title": "Read main.rs", "status": "in_progress",
        }),
    )
    .unwrap();
    let running = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(running["event"]["activity"], "Read main.rs");

    log.append(
        &StreamId::Tape(persona_id.clone()),
        &json!({
            "kind": "tool", "id": "t1", "ts": 11, "toolCallId": "c1",
            "title": "Read main.rs", "status": "completed",
        }),
    )
    .unwrap();
    let done = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert!(done["event"].get("activity").is_none(), "{done}");

    log.append(
        &StreamId::Tape(persona_id.clone()),
        &json!({
            "kind": "tool", "id": "t2", "ts": 12, "toolCallId": "c2",
            "title": "Edit lib.rs", "status": "in_progress",
        }),
    )
    .unwrap();
    let next = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(next["event"]["activity"], "Edit lib.rs");

    log.append(
        &StreamId::Tape(persona_id.clone()),
        &json!({ "kind": "turn", "id": "tu1", "ts": 13, "stopReason": "end_turn" }),
    )
    .unwrap();
    let ended = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert!(ended["event"].get("activity").is_none(), "{ended}");

    log.append(
        &StreamId::Tape(persona_id.clone()),
        &json!({
            "kind": "tool", "id": "t3", "ts": 14, "toolCallId": "c3",
            "title": "Run tests", "status": "in_progress",
        }),
    )
    .unwrap();
    let again = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(again["event"]["activity"], "Run tests");

    quiet.set_info(idle(&persona_id));
    let idle_row = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(idle_row["event"]["session"]["state"], "idle");
    assert!(idle_row["event"].get("activity").is_none(), "{idle_row}");
}

#[tokio::test]
async fn a_new_teammate_takes_the_rooms_defaults() {
    let (root, log, port) = door("defaults");
    let mut socket = desk(port).await;

    let created = create(&mut socket, 1, "  Ada  ").await;
    let id = created["id"].as_str().unwrap();
    assert_eq!(created["name"], "Ada");
    assert_eq!(created["goal"], "");
    assert_eq!(created["backendId"], "pi");
    assert_eq!(
        created["cwd"],
        json!(paths::default_workspace(&root, id).to_string_lossy())
    );
    assert_eq!(
        created["mcpPolicy"],
        json!({ "mode": "all", "serverIds": [] })
    );
    assert_eq!(created["sessionCheckpoints"], json!([]));
    assert_eq!(created["createdAt"], created["updatedAt"]);
    // Absent, not null: the workspace is the wall, and a teammate that never
    // asked for the machine says nothing about reach at all.
    assert!(created.get("reach").is_none(), "{created}");
    assert!(created.get("team").is_none(), "{created}");

    // And it is the room's own record, not a value the door made up.
    assert_eq!(json!(room::roster(&log)), json!([created]));
}

#[tokio::test]
async fn a_draft_that_names_things_keeps_them() {
    let (_root, _log, port) = door("draft");
    let mut socket = desk(port).await;

    let draft = PersonaDraft {
        name: "Bob".to_string(),
        goal: Some("  Keep the harbour running.  ".to_string()),
        team: Some("harbour".to_string()),
        backend_id: Some("cursor".to_string()),
        cwd: Some("/tmp/harbour".to_string()),
        reach: Some(crate::contract::Reach::Machine),
        model_id: Some("gpt-5".to_string()),
        computer: None,
    };
    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "persona.create", "params": { "draft": draft } }),
    )
    .await;
    let created = heard(&mut socket).await["result"].clone();
    assert_eq!(created["goal"], "Keep the harbour running.");
    assert_eq!(created["team"], "harbour");
    assert_eq!(created["backendId"], "cursor");
    assert_eq!(created["cwd"], "/tmp/harbour");
    assert_eq!(created["reach"], "machine");
    assert_eq!(created["modelId"], "gpt-5");
}

#[tokio::test]
async fn a_patch_is_folded_over_the_teammate_and_the_whole_record_is_written_again() {
    let (_root, log, port) = door("update");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap().to_string();

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "persona.update", "params": { "id": id, "patch": { "name": "Ada Lovelace" } } }),
    )
    .await;
    let updated = heard(&mut socket).await["result"].clone();
    assert_eq!(updated["name"], "Ada Lovelace");
    assert_eq!(updated["goal"], ada["goal"]);
    assert_eq!(updated["createdAt"], ada["createdAt"]);
    assert_eq!(json!(room::roster(&log)), json!([updated]));

    ask(
        &mut socket,
        json!({ "id": 3, "cmd": "persona.update", "params": { "id": "nobody", "patch": {} } }),
    )
    .await;
    let refused = heard(&mut socket).await;
    assert_eq!(refused["ok"], false);
    assert_eq!(refused["error"], "There is no teammate nobody.");
}

#[tokio::test]
async fn a_setting_is_one_event_per_key_and_null_puts_the_default_back() {
    let (_root, log, port) = door("settings");
    let mut socket = desk(port).await;

    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "settings.update", "params": { "patch": { "theme": "dark", "chapterIdleHours": 2 } } }),
    )
    .await;
    let settings = heard(&mut socket).await["result"].clone();
    assert_eq!(settings["theme"], "dark");
    assert_eq!(settings["chapterIdleHours"], 2);

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "settings.update", "params": { "patch": { "chapterIdleHours": null } } }),
    )
    .await;
    let cleared = heard(&mut socket).await["result"].clone();
    assert_eq!(cleared["chapterIdleHours"], 8);
    assert_eq!(json!(room::settings(&log)), cleared);
}

#[tokio::test]
async fn a_tape_carries_the_deltas_nobody_writes_down() {
    let quiet = Arc::new(Quiet::new());
    let deltas = quiet.deltas.clone();
    let (_root, _log, port) = door_with("ephemeral", quiet);

    let mut socket = desk(port).await;
    ask(&mut socket, json!({ "id": 1, "sub": { "tape": "ada" } })).await;
    assert_eq!(heard(&mut socket).await, json!({ "id": 1, "ok": true }));
    assert_eq!(
        heard(&mut socket).await,
        json!({ "sub": 1, "snapshot": [] })
    );

    // A delta for somebody else's tape is not this subscription's business.
    let _ = deltas.send(StreamDelta::AgentDelta {
        persona_id: "bob".to_string(),
        message_id: "m1".to_string(),
        text: "not here".to_string(),
    });
    let _ = deltas.send(StreamDelta::AgentDelta {
        persona_id: "ada".to_string(),
        message_id: "m2".to_string(),
        text: "hel".to_string(),
    });
    assert_eq!(
        heard(&mut socket).await,
        json!({
            "sub": 1,
            "ephemeral": { "type": "agent_delta", "personaId": "ada", "messageId": "m2", "text": "hel" }
        })
    );
}

#[tokio::test]
async fn human_answer_is_a_command_the_wire_can_read() {
    let (_root, _log, port) = door("human-answer");
    let mut socket = desk(port).await;

    ask(
        &mut socket,
        json!({
            "id": 1,
            "cmd": "human.answer",
            "params": { "personaId": "ada", "actionId": "act-1", "status": "done" },
        }),
    )
    .await;
    let answered = heard(&mut socket).await;
    assert_eq!(answered["id"], 1);
    assert_eq!(answered["ok"], true, "{answered}");

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "human.answer",
            "params": { "personaId": "ada", "actionId": "act-1", "status": "declined" },
        }),
    )
    .await;
    let declined = heard(&mut socket).await;
    assert_eq!(declined["ok"], true, "{declined}");
}

#[test]
fn the_desk_seat_may_do_everything_the_room_can_do() {
    assert!(Seat::Desk.permits(&Command::ModelsList {}));
    assert!(Seat::Desk.permits(&Command::PersonaDelete {
        id: "ada".to_string()
    }));
    assert!(Seat::Desk.permits_sub(&Target::Room));
    assert!(Seat::Desk.permits_sub(&Target::View(ViewName::Roster)));
}
