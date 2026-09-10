//! The wire, driven the way a client drives it: over a real socket.
//!
//! The room behind the door is a stand-in — Phase 0's sessions and vault are
//! being built beside this — so what is proved here is the wire's own half:
//! the token, the framing, the ordering of a subscription, and the roster
//! view the core maintains out of the log.

use super::*;
use crate::contract::{
    ChapterClose, ConfigChoice, Credential, CredentialKind, LoginPrompt, LoginStatus, Persona,
    PersonaDraft, SessionCapabilities, SessionState,
};
use crate::driver::Driver;
use crate::mcp::server::TeammateTools;
use crate::session::{Agents, ProviderAuth, ProviderKeys, Room};
use crate::{paths, room};
use async_trait::async_trait;
use serde_json::{Value, json};
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
            active_input: false,
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
    reattaches: Mutex<Vec<String>>,
    invalidations: Mutex<Vec<String>>,
    policy_updates: Arc<tokio::sync::Mutex<()>>,
}

impl Quiet {
    fn new() -> Self {
        Self {
            infos: broadcast::channel(16).0,
            deltas: broadcast::channel(16).0,
            states: Mutex::new(HashMap::new()),
            reattaches: Mutex::new(Vec::new()),
            invalidations: Mutex::new(Vec::new()),
            policy_updates: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    fn set_info(&self, info: SessionInfo) {
        self.states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(info.persona_id.clone(), info.clone());
        let _ = self.infos.send(info);
    }

    fn reattached(&self) -> Vec<String> {
        self.reattaches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// A real session room behind the WebSocket door, with all desk-only
/// facilities stubbed. This keeps the wire regression independent of a model
/// key while the collaboration card and answer still pass through Room.
struct CoreHandle {
    room: Arc<Room>,
}

struct NoAgents;

#[async_trait]
impl Agents for NoAgents {
    fn agent(
        &self,
        _persona: &Persona,
        _preamble: String,
        _said: Vec<crate::driver::rig::Said>,
        _tools: TeammateTools,
        _extra_mcp: Vec<crate::mcp::McpServer>,
    ) -> Result<Arc<dyn Driver>, String> {
        Err("the denial test must not start a recipient session".to_string())
    }

    async fn complete(
        &self,
        _model_id: &str,
        _system: &str,
        _prompt: &str,
    ) -> Result<String, String> {
        Err("the denial test must not call a model".to_string())
    }
}

struct NoKeys;

impl ProviderKeys for NoKeys {
    fn provider_auth(&self) -> HashMap<String, ProviderAuth> {
        HashMap::new()
    }
}

#[async_trait]
impl RoomHandle for CoreHandle {
    fn policy_update_lock(&self) -> Arc<tokio::sync::Mutex<()>> {
        self.room.policy_update_lock()
    }

    async fn start(&self, persona_id: &str) -> Result<SessionInfo, String> {
        self.room.start(persona_id).await
    }

    fn stop(&self, persona_id: &str) -> Result<(), String> {
        self.room.stop(persona_id)
    }

    fn invalidate(&self, persona_id: &str) -> Result<(), String> {
        self.room.invalidate(persona_id)
    }

    fn invalidate_all(&self) -> Result<(), String> {
        self.room.invalidate_all()
    }

    async fn reattach(&self, persona_id: &str) -> Result<(), String> {
        self.room.reattach(persona_id).await
    }

    async fn reattach_all(&self) -> Result<(), String> {
        self.room.reattach_all().await
    }

    async fn prompt(
        &self,
        persona_id: &str,
        text: &str,
        reply_to: Option<String>,
        attachments: Option<Vec<crate::contract::Attachment>>,
    ) -> Result<(), String> {
        self.room
            .prompt(persona_id, text, reply_to, attachments)
            .await
    }

    fn cancel(&self, persona_id: &str) -> Result<(), String> {
        self.room.cancel(persona_id)
    }

    async fn set_model(&self, persona_id: &str, model_id: &str) -> Result<SessionInfo, String> {
        self.room.set_model(persona_id, model_id).await
    }

    async fn set_mode(&self, persona_id: &str, mode_id: &str) -> Result<SessionInfo, String> {
        self.room.set_mode(persona_id, mode_id).await
    }

    async fn set_config(
        &self,
        persona_id: &str,
        config_id: &str,
        value: &str,
    ) -> Result<SessionInfo, String> {
        self.room.set_config(persona_id, config_id, value).await
    }

    fn models_efforts(&self, model_id: &str) -> Vec<ConfigChoice> {
        crate::models::effort_choices(model_id)
    }

    async fn answer_permission(
        &self,
        persona_id: &str,
        request_id: &str,
        option_id: &str,
    ) -> Result<(), String> {
        self.room
            .answer_permission(persona_id, request_id, option_id)
            .await
    }

    fn answer_human(
        &self,
        persona_id: &str,
        action_id: &str,
        status: crate::contract::HumanAnswer,
        note: Option<String>,
    ) -> Result<(), String> {
        self.room.answer_human(persona_id, action_id, status, note)
    }

    async fn start_fresh_chapter(
        &self,
        persona_id: &str,
    ) -> Result<crate::contract::ChapterSummary, String> {
        self.room
            .start_fresh_chapter(persona_id, ChapterClose::User)
            .await
    }

    async fn resume_chapter(
        &self,
        persona_id: &str,
    ) -> Result<crate::contract::ChapterSummary, String> {
        self.room.resume_chapter(persona_id).await
    }

    fn info(&self, persona_id: &str) -> SessionInfo {
        self.room.info(persona_id)
    }

    fn subscribe_info(&self) -> broadcast::Receiver<SessionInfo> {
        self.room.subscribe_info()
    }

    fn subscribe_deltas(&self) -> broadcast::Receiver<StreamDelta> {
        self.room.subscribe_deltas()
    }

    fn credential_create(
        &self,
        _provider_id: &str,
        _label: &str,
        _secret: &str,
    ) -> Result<Credential, String> {
        Err("credentials are unavailable in this wire test".to_string())
    }

    fn credential_revoke(&self, _id: &str) -> Result<(), String> {
        Err("credentials are unavailable in this wire test".to_string())
    }

    fn credential_delete(&self, _id: &str) -> Result<(), String> {
        Err("credentials are unavailable in this wire test".to_string())
    }

    async fn credential_login(&self, _provider_id: &str) -> Result<LoginPrompt, String> {
        Err("credentials are unavailable in this wire test".to_string())
    }

    fn login_status(&self, _login_id: &str) -> Result<LoginStatus, String> {
        Err("credentials are unavailable in this wire test".to_string())
    }

    fn credentials(&self) -> Vec<Credential> {
        Vec::new()
    }

    async fn backends(&self) -> Vec<crate::contract::BackendChoice> {
        Vec::new()
    }

    fn models(&self) -> Vec<ConfigChoice> {
        self.room.models_for_desk()
    }

    fn models_catalog(
        &self,
        _provider_id: &str,
    ) -> Result<Vec<crate::contract::CatalogModel>, String> {
        Err("catalogues are unavailable in this wire test".to_string())
    }

    fn import(&self, _from: &std::path::Path) -> Result<crate::import::Report, String> {
        Err("import is unavailable in this wire test".to_string())
    }

    fn teammate_tools(&self, persona_id: &str) -> Option<crate::contract::TeammateToolLedger> {
        self.room.teammate_tools(persona_id)
    }

    fn schedule_create(
        &self,
        persona_id: &str,
        kind: crate::contract::ScheduleKind,
        when: Option<i64>,
        every: Option<i64>,
        prompt: &str,
        quiet: bool,
    ) -> Result<crate::contract::ScheduledJob, String> {
        self.room
            .schedule_create(persona_id, kind, when, every, prompt, quiet)
    }

    fn schedule_list(&self) -> Vec<crate::contract::ScheduledJob> {
        self.room.schedule_list()
    }

    fn schedule_cancel(&self, id: &str) -> Result<(), String> {
        self.room.schedule_cancel(id)
    }

    fn schedule_set_quiet(&self, id: &str, quiet: bool) -> Result<(), String> {
        self.room.schedule_set_quiet(id, quiet)
    }

    fn peer_threads(&self, persona_id: &str) -> Vec<crate::contract::PeerThreadSummary> {
        self.room.peer_threads(persona_id)
    }

    fn mark_peer_read(&self, key: &str, event_ids: &[String]) -> usize {
        self.room.mark_peer_read(key, event_ids)
    }

    async fn credential_refresh_models(
        &self,
        _provider_id: &str,
    ) -> Result<Vec<crate::contract::CatalogModel>, String> {
        Err("credentials are unavailable in this wire test".to_string())
    }

    fn forget(&self, persona_id: &str) {
        self.room.forget(persona_id);
    }

    async fn computer_runtimes(&self) -> Vec<crate::contract::RuntimeReport> {
        self.room.computer_runtimes().await
    }

    async fn computer_status(
        &self,
        persona_id: &str,
    ) -> Result<crate::contract::ComputerStatus, String> {
        self.room.computer_status(persona_id).await
    }

    async fn computer_stop(&self, persona_id: &str) -> Result<(), String> {
        self.room.computer_stop(persona_id).await
    }

    async fn computer_remove(&self, persona_id: &str) -> Result<(), String> {
        self.room.computer_remove(persona_id).await
    }
}

#[async_trait::async_trait]
impl RoomHandle for Quiet {
    fn policy_update_lock(&self) -> Arc<tokio::sync::Mutex<()>> {
        self.policy_updates.clone()
    }

    async fn start(&self, persona_id: &str) -> Result<SessionInfo, String> {
        let mut info = idle(persona_id);
        info.current_model_id = Some("anthropic/claude".to_string());
        self.set_info(info.clone());
        Ok(info)
    }

    fn stop(&self, _persona_id: &str) -> Result<(), String> {
        Ok(())
    }

    fn invalidate(&self, persona_id: &str) -> Result<(), String> {
        self.invalidations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(persona_id.to_string());
        Ok(())
    }

    fn invalidate_all(&self) -> Result<(), String> {
        self.invalidate("*")
    }

    async fn reattach(&self, persona_id: &str) -> Result<(), String> {
        self.reattaches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(persona_id.to_string());
        Ok(())
    }

    async fn reattach_all(&self) -> Result<(), String> {
        let ids: Vec<String> = self
            .states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .keys()
            .cloned()
            .collect();
        for id in ids {
            self.reattach(&id).await?;
        }
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

    async fn set_model(&self, persona_id: &str, model_id: &str) -> Result<SessionInfo, String> {
        let mut info = self.info(persona_id);
        info.current_model_id = Some(model_id.to_string());
        self.set_info(info.clone());
        Ok(info)
    }

    async fn set_mode(&self, persona_id: &str, _mode_id: &str) -> Result<SessionInfo, String> {
        Ok(idle(persona_id))
    }

    async fn set_config(
        &self,
        persona_id: &str,
        config_id: &str,
        value: &str,
    ) -> Result<SessionInfo, String> {
        let mut info = self.info(persona_id);
        info.configs = vec![crate::contract::SessionConfig {
            id: config_id.to_string(),
            name: config_id.to_string(),
            category: None,
            current_id: Some(value.to_string()),
            options: Vec::new(),
        }];
        self.set_info(info.clone());
        Ok(info)
    }

    fn models_efforts(&self, model_id: &str) -> Vec<ConfigChoice> {
        crate::models::effort_choices(model_id)
    }

    async fn answer_permission(
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
        _note: Option<String>,
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
            base_url: None,
            custom: None,
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

    async fn credential_login(&self, provider_id: &str) -> Result<LoginPrompt, String> {
        if let Some(message) = crate::models::login_refusal(provider_id) {
            return Err(message);
        }
        Err("Nothing runs in this room, so nothing signs in.".to_string())
    }

    fn login_status(&self, login_id: &str) -> Result<LoginStatus, String> {
        Err(format!("There is no login {login_id}."))
    }

    async fn backends(&self) -> Vec<crate::contract::BackendChoice> {
        Vec::new()
    }

    fn credentials(&self) -> Vec<Credential> {
        Vec::new()
    }

    fn models(&self) -> Vec<ConfigChoice> {
        vec![
            ConfigChoice {
                id: "anthropic/claude".to_string(),
                name: "Claude".to_string(),
                description: None,
                group: None,
            },
            ConfigChoice {
                id: "openai/gpt".to_string(),
                name: "GPT".to_string(),
                description: None,
                group: None,
            },
        ]
    }

    fn models_catalog(
        &self,
        provider_id: &str,
    ) -> Result<Vec<crate::contract::CatalogModel>, String> {
        if crate::models::wiring(provider_id).is_none() {
            return Err(format!(
                "{provider_id} is not a provider Toad Agent can use."
            ));
        }
        Ok(crate::models::catalog_models(
            provider_id,
            &HashMap::new(),
            None,
        ))
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

    async fn credential_refresh_models(
        &self,
        provider_id: &str,
    ) -> Result<Vec<crate::contract::CatalogModel>, String> {
        if let Some(message) = crate::models::login_refusal(provider_id) {
            return Err(message);
        }
        let name = crate::models::catalog()
            .providers
            .get(provider_id)
            .map(|entry| entry.name.as_str())
            .unwrap_or(provider_id);
        Err(format!("There is no sign-in for {name}."))
    }

    fn forget(&self, _persona_id: &str) {}

    async fn computer_runtimes(&self) -> Vec<crate::contract::RuntimeReport> {
        vec![
            crate::contract::RuntimeReport {
                runtime: crate::contract::ComputerRuntime::Podman,
                state: crate::contract::RuntimeState::Ready,
                detail: None,
                rootless: true,
            },
            crate::contract::RuntimeReport {
                runtime: crate::contract::ComputerRuntime::Docker,
                state: crate::contract::RuntimeState::Ready,
                detail: None,
                rootless: false,
            },
        ]
    }

    async fn computer_status(
        &self,
        _persona_id: &str,
    ) -> Result<crate::contract::ComputerStatus, String> {
        Ok(crate::contract::ComputerStatus {
            state: crate::contract::ComputerState::Running,
            url: Some("http://127.0.0.1:18787/mcp".into()),
            viewer: Some("http://127.0.0.1:15800".into()),
        })
    }

    async fn computer_stop(&self, _persona_id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn computer_remove(&self, _persona_id: &str) -> Result<(), String> {
        Ok(())
    }
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

/// A real session room behind the door, with a workspace pair and no model
/// implementation. The denial path must finish before the recipient driver is
/// ever requested, so a driver is unnecessary for this wire proof.
fn core_door(name: &str) -> (PathBuf, Log, u16, Arc<Room>) {
    let root = scratch(name);
    let log = Log::open(&root);
    let persona = |id: &str, name: &str| {
        json!({
            "kind": "persona",
            "id": id,
            "name": name,
            "goal": format!("Role of {name}"),
            "backendId": "pi",
            "cwd": root.to_string_lossy(),
            "mcpPolicy": { "mode": "none", "serverIds": [] },
            "backgroundWork": false,
            "sessionCheckpoints": [],
            "lastSessionId": null,
            "createdAt": 1,
            "updatedAt": 1,
        })
    };
    log.append(&StreamId::Room, &persona("ada", "Ada")).unwrap();
    log.append(&StreamId::Room, &persona("bob", "Bob")).unwrap();
    let room = Room::with_agents(log.clone(), Arc::new(NoKeys), Arc::new(NoAgents));
    let door = Door::bind(
        log.clone(),
        "desk-token".to_string(),
        Arc::new(CoreHandle { room: room.clone() }),
    )
    .unwrap();
    let port = door.port();
    tokio::spawn(door.run());
    (root, log, port, room)
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

/// A command's answer, or a failure saying it never came. A command that
/// takes the read loop down with it is answered never, and a test that waits
/// forever for that answer says nothing about why.
async fn answered(socket: &mut Socket, id: i64) -> Value {
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        heard_where(socket, |frame| frame["id"] == id),
    )
    .await
    .unwrap_or_else(|_| panic!("command {id} was never answered"))
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

#[tokio::test(flavor = "multi_thread")]
async fn the_wire_answers_a_core_owned_collaboration_card_before_peer_start() {
    let (_root, log, port, room) = core_door("collaboration-wire");
    let mut socket = desk(port).await;
    ask(&mut socket, json!({ "id": 1, "sub": { "tape": "ada" } })).await;
    assert_eq!(heard(&mut socket).await, json!({ "id": 1, "ok": true }));
    let snapshot = heard(&mut socket).await;
    assert_eq!(snapshot["snapshot"], json!([]));

    let delivery = {
        let room = room.clone();
        tokio::spawn(async move { room.deliver("ada", "bob", "read Bob's private file").await })
    };
    let card = heard_where(&mut socket, |frame| {
        frame["sub"] == 1 && frame["event"]["kind"] == "permission"
    })
    .await;
    let request_id = card["event"]["requestId"].as_str().unwrap();
    assert!(request_id.starts_with("collab:"), "{card}");
    assert_eq!(
        card["event"]["title"],
        "Allow Ada to ask Bob to work?\n\nBob can use its workspace and enabled tools to fulfill Ada's requests and return results."
    );

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "session.answer_permission",
            "params": { "personaId": "ada", "requestId": request_id, "optionId": "deny" }
        }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    let denied = delivery.await.unwrap().unwrap_err();
    assert!(denied.contains("denied"), "{denied}");
    assert!(
        log.load(&StreamId::Thread("ada~bob".to_string()))
            .is_empty()
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
        json!({ "mode": "none", "serverIds": [] })
    );
    assert_eq!(created["sessionCheckpoints"], json!([]));
    assert_eq!(created["createdAt"], created["updatedAt"]);
    // Absent, not null: the workspace is the wall, and a teammate that never
    // asked for the machine says nothing about reach at all.
    assert!(created.get("reach").is_none(), "{created}");
    assert!(created.get("team").is_none(), "{created}");
    // No room default and no last model used: the driver picks at start.
    assert!(created.get("modelId").is_none(), "{created}");

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
        effort_id: None,
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
            "params": { "personaId": "ada", "actionId": "act-1", "status": "declined", "note": "not now" },
        }),
    )
    .await;
    let declined = heard(&mut socket).await;
    assert_eq!(declined["ok"], true, "{declined}");
}

/// An id off the wire is not a teammate this room has, and one with no
/// characters in it names the transcripts directory rather than a tape in it.
/// It is answered like any other stranger, because a command that panics is a
/// command that is never answered at all — and it takes the socket with it.
#[tokio::test]
async fn a_command_naming_a_teammate_with_no_id_is_answered() {
    let (_root, _log, port) = door("blank-id");
    let mut socket = desk(port).await;

    for (id, cmd, params) in [
        (1, "chapter.list", json!({ "personaId": "" })),
        (2, "peers.list", json!({ "personaId": "" })),
        (
            3,
            "search.thread",
            json!({ "personaId": "", "query": "crane" }),
        ),
        (4, "teammate.tools", json!({ "personaId": "" })),
    ] {
        ask(
            &mut socket,
            json!({ "id": id, "cmd": cmd, "params": params }),
        )
        .await;
        let answer = answered(&mut socket, id).await;
        assert_eq!(answer["ok"], true, "{cmd}: {answer}");
    }

    // The socket is still the one that answered the first of them.
    let created = create(&mut socket, 5, "Ada").await;
    assert_eq!(created["name"], "Ada");
}

/// A subscription to a tape that cannot exist is still a subscription: it is
/// acknowledged, snapshotted empty, and closed by an unsubscribe.
#[tokio::test]
async fn a_tape_subscription_for_a_teammate_with_no_id_snapshots_nothing() {
    let (_root, _log, port) = door("blank-id-sub");
    let mut socket = desk(port).await;

    ask(&mut socket, json!({ "id": 1, "sub": { "tape": "" } })).await;
    let opened = answered(&mut socket, 1).await;
    assert_eq!(opened["ok"], true, "{opened}");
    let snapshot = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        heard_where(&mut socket, |frame| frame["sub"] == 1),
    )
    .await
    .expect("the snapshot never arrived");
    assert_eq!(snapshot["snapshot"], json!([]));
}

#[test]
fn the_desk_seat_may_do_everything_the_room_can_do() {
    assert!(Seat::Desk.permits(&Command::ModelsList {}));
    assert!(Seat::Desk.permits(&Command::ModelsCatalog {
        provider_id: "anthropic".to_string()
    }));
    assert!(Seat::Desk.permits(&Command::PersonaDelete {
        id: "ada".to_string()
    }));
    assert!(Seat::Desk.permits_sub(&Target::Room));
    assert!(Seat::Desk.permits_sub(&Target::View(ViewName::Roster)));
}

#[test]
fn a_hung_up_client_and_an_interrupted_accept_are_waited_out() {
    assert_eq!(
        accept_again(&std::io::Error::from(std::io::ErrorKind::ConnectionAborted)),
        Some(AcceptAgain::Now)
    );
    assert_eq!(
        accept_again(&std::io::Error::from(std::io::ErrorKind::Interrupted)),
        Some(AcceptAgain::Now)
    );
    assert_eq!(
        accept_again(&std::io::Error::from(std::io::ErrorKind::NotConnected)),
        None
    );
}

#[cfg(unix)]
#[test]
fn too_many_open_files_pauses_and_a_gone_listener_does_not() {
    assert_eq!(
        accept_again(&std::io::Error::from_raw_os_error(libc::ECONNABORTED)),
        Some(AcceptAgain::Now)
    );
    assert_eq!(
        accept_again(&std::io::Error::from_raw_os_error(libc::EINTR)),
        Some(AcceptAgain::Now)
    );
    assert_eq!(
        accept_again(&std::io::Error::from_raw_os_error(libc::EMFILE)),
        Some(AcceptAgain::AfterPause)
    );
    assert_eq!(
        accept_again(&std::io::Error::from_raw_os_error(libc::ENFILE)),
        Some(AcceptAgain::AfterPause)
    );
    assert_eq!(
        accept_again(&std::io::Error::from_raw_os_error(libc::EBADF)),
        None
    );
}

#[cfg(windows)]
#[test]
fn winsock_resource_pressure_pauses_without_losing_the_listener() {
    use windows_sys::Win32::Networking::WinSock::{
        WSAECONNRESET, WSAEMFILE, WSAENOBUFS, WSAENOTSOCK,
    };
    for code in [WSAEMFILE, WSAENOBUFS] {
        assert_eq!(
            accept_again(&std::io::Error::from_raw_os_error(code)),
            Some(AcceptAgain::AfterPause)
        );
    }
    assert_eq!(
        accept_again(&std::io::Error::from_raw_os_error(WSAECONNRESET)),
        Some(AcceptAgain::Now)
    );
    assert_eq!(
        accept_again(&std::io::Error::from_raw_os_error(WSAENOTSOCK)),
        None
    );
}

/// A stream subscription that ends without an unsubscribe used to keep its
/// id forever, so the next client that reused the number was told it was
/// already open for a task that would never deliver.
#[tokio::test]
async fn a_subscription_id_is_free_once_its_task_has_ended() {
    let (_root, log, port) = door("sub-ended");
    let mut socket = desk(port).await;

    ask(&mut socket, json!({ "id": 7, "sub": "room" })).await;
    assert_eq!(heard(&mut socket).await, json!({ "id": 7, "ok": true }));
    assert_eq!(
        heard(&mut socket).await,
        json!({ "sub": 7, "snapshot": [] })
    );

    log.close_broadcasts();

    let reused = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            ask(&mut socket, json!({ "id": 7, "sub": "room" })).await;
            let answer = answered(&mut socket, 7).await;
            if answer["ok"] == true {
                return answer;
            }
            let error = answer["error"].as_str().unwrap_or("");
            assert!(
                error.contains("already open"),
                "waiting for the ended task to free the id, got {answer}"
            );
        }
    })
    .await
    .expect("the ended subscription never freed its id");
    assert_eq!(reused["ok"], true, "{reused}");
}

#[tokio::test]
async fn teammate_tools_answers_null_as_a_result_not_a_void() {
    let (_root, _log, port) = door("tools-null");
    let mut socket = desk(port).await;
    let created = create(&mut socket, 1, "Ada").await;
    let persona_id = created["id"].as_str().unwrap();

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "teammate.tools", "params": { "personaId": persona_id } }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    let fields = answer.as_object().expect("an answer is an object");
    assert!(
        fields.contains_key("result"),
        "teammate.tools with no ledger omitted result: {answer}"
    );
    assert_eq!(fields.get("result"), Some(&Value::Null));
}

#[tokio::test]
async fn a_key_provider_cannot_start_a_device_login() {
    let (_root, _log, port) = door("login-key");
    let mut socket = desk(port).await;
    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "credential.login", "params": { "providerId": "anthropic" } }),
    )
    .await;
    let refused = answered(&mut socket, 1).await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(
        refused["error"].as_str(),
        Some("Anthropic takes an API key, not a sign-in.")
    );
}

#[tokio::test]
async fn an_unknown_login_id_is_an_error() {
    let (_root, _log, port) = door("login-status");
    let mut socket = desk(port).await;
    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "credential.login_status", "params": { "loginId": "nobody" } }),
    )
    .await;
    let refused = answered(&mut socket, 1).await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"].as_str(), Some("There is no login nobody."));
}

#[tokio::test]
async fn credential_refresh_models_needs_a_login_and_refuses_a_key() {
    let (_root, _log, port) = door("refresh-models");
    let mut socket = desk(port).await;

    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "credential.refresh_models", "params": { "providerId": "anthropic" } }),
    )
    .await;
    let key = answered(&mut socket, 1).await;
    assert_eq!(key["ok"], false, "{key}");
    assert_eq!(
        key["error"].as_str(),
        Some("Anthropic takes an API key, not a sign-in.")
    );

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "credential.refresh_models", "params": { "providerId": "github-copilot" } }),
    )
    .await;
    let unsigned = answered(&mut socket, 2).await;
    assert_eq!(unsigned["ok"], false, "{unsigned}");
    assert_eq!(
        unsigned["error"].as_str(),
        Some("There is no sign-in for GitHub Copilot.")
    );
}

#[tokio::test]
async fn models_catalog_lists_a_wired_provider_and_refuses_an_unwired_one() {
    let (_root, _log, port) = door("models-catalog");
    let mut socket = desk(port).await;

    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "models.catalog", "params": { "providerId": "nope" } }),
    )
    .await;
    let refused = answered(&mut socket, 1).await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(
        refused["error"].as_str(),
        Some("nope is not a provider Toad Agent can use.")
    );

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "models.catalog", "params": { "providerId": "anthropic" } }),
    )
    .await;
    let listed = answered(&mut socket, 2).await;
    assert_eq!(listed["ok"], true, "{listed}");
    let models = listed["result"].as_array().expect("a catalogue is a list");
    assert!(!models.is_empty());
    assert!(
        models.iter().all(|model| model["enabled"] == true),
        "{listed}"
    );
    assert!(
        models
            .iter()
            .all(|model| model["id"].as_str().is_some_and(|id| !id.is_empty())),
        "{listed}"
    );
}

/// Setting a model on an idle Toad Agent teammate writes the choice on the
/// persona and remembers it as the last model used, without needing a live
/// session to hold it.
#[tokio::test]
async fn session_set_model_on_an_idle_toad_agent_writes_the_persona_and_last_used() {
    let (_root, log, port) = door("set-model-idle");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap();
    assert!(ada.get("modelId").is_none(), "{ada}");

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "session.set_model",
            "params": { "personaId": id, "modelId": "openai/gpt" },
        }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["result"]["state"], "idle");
    assert_eq!(answer["result"]["personaId"], id);

    let roster = room::roster(&log);
    assert_eq!(roster[0].model_id.as_deref(), Some("openai/gpt"));
    assert_eq!(room::settings(&log)["lastModelId"], "openai/gpt");
}

#[tokio::test]
async fn session_set_model_refuses_a_toad_agent_id_the_desk_cannot_reach() {
    let (_root, _log, port) = door("set-model-unknown");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap();

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "session.set_model",
            "params": { "personaId": id, "modelId": "nope/nope" },
        }),
    )
    .await;
    let refused = answered(&mut socket, 2).await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(
        refused["error"].as_str(),
        Some("nope/nope is not a model this desk can reach.")
    );
}

/// An ACP harness names its own models. The wire stores what it is given
/// and lets the child refuse an id it does not know, once it is live.
#[tokio::test]
async fn session_set_model_accepts_an_arbitrary_id_on_an_acp_teammate() {
    let (_root, log, port) = door("set-model-acp");
    let mut socket = desk(port).await;
    let draft = PersonaDraft {
        name: "Bob".to_string(),
        goal: None,
        team: None,
        backend_id: Some("cursor".to_string()),
        cwd: None,
        reach: None,
        model_id: None,
        effort_id: None,
        computer: None,
    };
    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "persona.create", "params": { "draft": draft } }),
    )
    .await;
    let created = answered(&mut socket, 1).await["result"].clone();
    let id = created["id"].as_str().unwrap();

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "session.set_model",
            "params": { "personaId": id, "modelId": "cursor-special" },
        }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(
        room::roster(&log)[0].model_id.as_deref(),
        Some("cursor-special")
    );
    assert!(
        room::settings(&log).get("lastModelId").is_none(),
        "an ACP choice is not the room's last Toad Agent model"
    );
}

fn a_model_with_efforts() -> (String, Vec<String>) {
    crate::models::catalog()
        .providers
        .iter()
        .find_map(|(provider, entry)| {
            entry.models.iter().find_map(|(id, model)| {
                (!model.efforts.is_empty())
                    .then(|| (format!("{provider}/{id}"), model.efforts.clone()))
            })
        })
        .expect("the snapshot has a model with an effort list")
}

/// Setting effort on an idle Toad Agent teammate writes it on the persona
/// and answers idle info, without needing a live session to hold it.
#[tokio::test]
async fn session_set_config_on_an_idle_toad_agent_writes_effort_id() {
    let (_root, log, port) = door("set-config-idle");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap();
    assert!(ada.get("effortId").is_none(), "{ada}");

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "session.set_config",
            "params": { "personaId": id, "configId": "effort", "value": "high" },
        }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["result"]["state"], "idle");
    assert_eq!(room::roster(&log)[0].effort_id.as_deref(), Some("high"));

    ask(
        &mut socket,
        json!({
            "id": 3,
            "cmd": "session.set_config",
            "params": { "personaId": id, "configId": "effort", "value": "" },
        }),
    )
    .await;
    let cleared = answered(&mut socket, 3).await;
    assert_eq!(cleared["ok"], true, "{cleared}");
    assert!(
        room::roster(&log)[0].effort_id.is_none(),
        "{:?}",
        room::roster(&log)[0]
    );
}

#[tokio::test]
async fn session_set_config_refuses_an_effort_the_model_does_not_list() {
    let (model, listed) = a_model_with_efforts();
    let (_root, _log, port) = door("set-config-unknown");
    let mut socket = desk(port).await;
    let draft = PersonaDraft {
        name: "Ada".to_string(),
        goal: None,
        team: None,
        backend_id: None,
        cwd: None,
        reach: None,
        model_id: Some(model.clone()),
        effort_id: None,
        computer: None,
    };
    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "persona.create", "params": { "draft": draft } }),
    )
    .await;
    let created = answered(&mut socket, 1).await["result"].clone();
    let id = created["id"].as_str().unwrap();

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "session.set_config",
            "params": { "personaId": id, "configId": "effort", "value": "nope" },
        }),
    )
    .await;
    let refused = answered(&mut socket, 2).await;
    assert_eq!(refused["ok"], false, "{refused}");
    let error = refused["error"].as_str().unwrap();
    assert!(error.contains("nope is not an effort"), "{error}");
    assert!(
        listed.iter().all(|id| id != "nope"),
        "the fixture must pick a model that does not list nope"
    );
}

#[tokio::test]
async fn session_set_config_on_an_acp_teammate_goes_to_the_room() {
    let (_root, _log, port) = door("set-config-acp");
    let mut socket = desk(port).await;
    let draft = PersonaDraft {
        name: "Bob".to_string(),
        goal: None,
        team: None,
        backend_id: Some("cursor".to_string()),
        cwd: None,
        reach: None,
        model_id: None,
        effort_id: None,
        computer: None,
    };
    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "persona.create", "params": { "draft": draft } }),
    )
    .await;
    let created = answered(&mut socket, 1).await["result"].clone();
    let id = created["id"].as_str().unwrap();

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "session.set_config",
            "params": { "personaId": id, "configId": "effort", "value": "high" },
        }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["result"]["configs"][0]["id"], "effort");
    assert_eq!(answer["result"]["configs"][0]["currentId"], "high");
}

#[tokio::test]
async fn persona_create_fills_model_id_from_the_room_default() {
    let (_root, _log, port) = door("create-default-model");
    let mut socket = desk(port).await;
    ask(
        &mut socket,
        json!({
            "id": 1,
            "cmd": "settings.update",
            "params": { "patch": { "defaultModelId": "anthropic/claude" } },
        }),
    )
    .await;
    assert_eq!(answered(&mut socket, 1).await["ok"], true);

    let created = create(&mut socket, 2, "Ada").await;
    assert_eq!(created["modelId"], "anthropic/claude");
}

#[tokio::test]
async fn persona_create_fills_model_id_from_the_last_used_when_no_default() {
    let (_root, _log, port) = door("create-last-model");
    let mut socket = desk(port).await;
    ask(
        &mut socket,
        json!({
            "id": 1,
            "cmd": "settings.update",
            "params": { "patch": { "lastModelId": "openai/gpt" } },
        }),
    )
    .await;
    assert_eq!(answered(&mut socket, 1).await["ok"], true);

    let created = create(&mut socket, 2, "Ada").await;
    assert_eq!(created["modelId"], "openai/gpt");
}

#[tokio::test]
async fn persona_create_leaves_model_id_absent_when_the_room_has_no_preference() {
    let (_root, _log, port) = door("create-no-model");
    let mut socket = desk(port).await;
    let created = create(&mut socket, 1, "Ada").await;
    assert!(created.get("modelId").is_none(), "{created}");
}

/// The fake room's start reports a model, which is enough for the wire to
/// remember it as the last one used.
#[tokio::test]
async fn session_start_writes_last_model_id_from_the_sessions_info() {
    let (_root, log, port) = door("start-last-model");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap();

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "session.start", "params": { "personaId": id } }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(room::settings(&log)["lastModelId"], "anthropic/claude");
}

/// A harness picks its own model and only says so once running, so what a
/// start reports is written to the teammate: the band can name it before
/// the child is started again. The room's last model is Toad Agent's alone.
#[tokio::test]
async fn session_start_writes_the_reported_model_on_an_acp_teammate() {
    let (_root, log, port) = door("start-acp-model");
    let mut socket = desk(port).await;
    let draft = PersonaDraft {
        name: "Bob".to_string(),
        goal: None,
        team: None,
        backend_id: Some("cursor".to_string()),
        cwd: None,
        reach: None,
        model_id: None,
        effort_id: None,
        computer: None,
    };
    ask(
        &mut socket,
        json!({ "id": 1, "cmd": "persona.create", "params": { "draft": draft } }),
    )
    .await;
    let id = answered(&mut socket, 1).await["result"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "session.start", "params": { "personaId": id } }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    let bob = room::roster(&log)
        .into_iter()
        .find(|persona| persona.id == id)
        .unwrap();
    assert_eq!(bob.model_id.as_deref(), Some("anthropic/claude"));
    assert!(
        room::settings(&log).get("lastModelId").is_none(),
        "a harness's model is the teammate's, not the room's last"
    );
}

/// A patch that changes what a teammate can use restarts it; a patch of
/// only the name does not, because the driver is not built from the name.
#[tokio::test]
async fn persona_update_of_reach_reattaches_and_a_name_patch_does_not() {
    let quiet = Arc::new(Quiet::new());
    let (_root, _log, port) = door_with("reattach-persona", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap().to_string();

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "persona.update",
            "params": { "id": id, "patch": { "reach": "machine" } },
        }),
    )
    .await;
    assert_eq!(answered(&mut socket, 2).await["ok"], true);
    assert_eq!(quiet.reattached(), vec![id.clone()]);
    assert_eq!(*quiet.invalidations.lock().unwrap(), vec![id.clone()]);

    ask(
        &mut socket,
        json!({
            "id": 3,
            "cmd": "persona.update",
            "params": { "id": id, "patch": { "name": "Ada Lovelace" } },
        }),
    )
    .await;
    assert_eq!(answered(&mut socket, 3).await["ok"], true);
    assert_eq!(
        quiet.reattached(),
        vec![id.clone()],
        "a name patch reattached a second time"
    );
    assert_eq!(*quiet.invalidations.lock().unwrap(), vec![id.clone()]);

    ask(
        &mut socket,
        json!({
            "id": 4,
            "cmd": "persona.update",
            "params": { "id": id, "patch": { "reach": "invalid" } },
        }),
    )
    .await;
    assert_eq!(answered(&mut socket, 4).await["ok"], false);
    assert_eq!(
        *quiet.invalidations.lock().unwrap(),
        vec![id],
        "an invalid patch must not revoke a working session"
    );
}

/// The room's server list is what every policy of "all" includes, so every
/// live session restarts. A setting that does not name servers leaves them.
#[tokio::test]
async fn settings_update_of_mcp_servers_reattaches_every_live_session() {
    let quiet = Arc::new(Quiet::new());
    let (_root, _log, port) = door_with("reattach-servers", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let bob = create(&mut socket, 2, "Bob").await;
    let ada_id = ada["id"].as_str().unwrap().to_string();
    let bob_id = bob["id"].as_str().unwrap().to_string();

    ask(
        &mut socket,
        json!({ "id": 3, "cmd": "session.start", "params": { "personaId": ada_id } }),
    )
    .await;
    assert_eq!(answered(&mut socket, 3).await["ok"], true);
    ask(
        &mut socket,
        json!({ "id": 4, "cmd": "session.start", "params": { "personaId": bob_id } }),
    )
    .await;
    assert_eq!(answered(&mut socket, 4).await["ok"], true);

    ask(
        &mut socket,
        json!({
            "id": 5,
            "cmd": "settings.update",
            "params": { "patch": { "mcpServers": [] } },
        }),
    )
    .await;
    assert_eq!(answered(&mut socket, 5).await["ok"], true);
    let mut got = quiet.reattached();
    got.sort();
    let mut want = vec![ada_id, bob_id];
    want.sort();
    assert_eq!(got, want);
    assert_eq!(*quiet.invalidations.lock().unwrap(), vec!["*"]);

    ask(
        &mut socket,
        json!({
            "id": 6,
            "cmd": "settings.update",
            "params": { "patch": { "chapterIdleHours": 2 } },
        }),
    )
    .await;
    assert_eq!(answered(&mut socket, 6).await["ok"], true);
    let mut after = quiet.reattached();
    after.sort();
    assert_eq!(after, want, "a chapterIdleHours patch reattached again");
    assert_eq!(*quiet.invalidations.lock().unwrap(), vec!["*"]);
}

#[tokio::test]
async fn computer_commands_and_the_runtime_setting() {
    let quiet = Arc::new(Quiet::new());
    let (_root, _log, port) = door_with("computer-wire", quiet.clone());
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap().to_string();

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "computer.runtimes", "params": {} }),
    )
    .await;
    let runtimes = answered(&mut socket, 2).await;
    assert_eq!(runtimes["ok"], true, "{runtimes}");
    assert_eq!(runtimes["result"][0]["runtime"], "podman");
    assert_eq!(runtimes["result"][0]["state"], "ready");
    assert_eq!(runtimes["result"][0]["rootless"], true);
    assert_eq!(runtimes["result"][1]["runtime"], "docker");

    ask(
        &mut socket,
        json!({ "id": 3, "cmd": "computer.status", "params": { "personaId": id } }),
    )
    .await;
    let status = answered(&mut socket, 3).await;
    assert_eq!(status["ok"], true, "{status}");
    assert_eq!(status["result"]["state"], "running");
    assert_eq!(status["result"]["url"], "http://127.0.0.1:18787/mcp");
    assert_eq!(status["result"]["viewer"], "http://127.0.0.1:15800");

    ask(
        &mut socket,
        json!({ "id": 4, "cmd": "computer.stop", "params": { "personaId": id } }),
    )
    .await;
    let stopped = answered(&mut socket, 4).await;
    assert_eq!(stopped["ok"], true, "{stopped}");
    assert!(stopped.get("result").is_none(), "{stopped}");

    ask(
        &mut socket,
        json!({ "id": 5, "cmd": "computer.remove", "params": { "personaId": id } }),
    )
    .await;
    let removed = answered(&mut socket, 5).await;
    assert_eq!(removed["ok"], true, "{removed}");
    assert!(removed.get("result").is_none(), "{removed}");

    ask(
        &mut socket,
        json!({
            "id": 6,
            "cmd": "settings.update",
            "params": { "patch": { "computerRuntime": "podman" } },
        }),
    )
    .await;
    let settings = answered(&mut socket, 6).await;
    assert_eq!(settings["ok"], true, "{settings}");
    assert_eq!(settings["result"]["computerRuntime"], "podman");
    assert!(
        quiet.reattached().is_empty(),
        "picking a runtime does not reattach: {:?}",
        quiet.reattached()
    );

    ask(
        &mut socket,
        json!({
            "id": 7,
            "cmd": "computer.status",
            "params": { "personaId": "nobody" },
        }),
    )
    .await;
    let missing = answered(&mut socket, 7).await;
    assert_eq!(missing["ok"], false, "{missing}");
    assert!(
        missing["error"]
            .as_str()
            .unwrap()
            .contains("There is no teammate nobody"),
        "{missing}"
    );
}
