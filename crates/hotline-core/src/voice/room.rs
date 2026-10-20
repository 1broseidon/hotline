//! Sessions, vault reads and real tool handlers behind the call-room seam.
use crate::{
    contract::{Command, ConfigChoice, End, ModelCost, Persona, SessionInfo, ThreadId},
    log::Log,
    session::{Post, Room},
    vault::Vault,
    wire::RoomHandle,
};
use async_trait::async_trait;
use hotline_voice::{
    Calls,
    call_room::CallRoom,
    front_tools::{DeskTools, FrontReach},
};
use serde_json::{Map, Value};
use std::sync::{Arc, Weak};

pub struct RoomSide {
    room: Weak<Room>,
    vault: Arc<Vault>,
    // A call holds only the weak room. An upgrade keeps it alive across a
    // whole operation, including the awaits between startup and prompting.
    _keep_alive: Option<Arc<Room>>,
}
impl RoomSide {
    pub fn new(room: Weak<Room>, vault: Arc<Vault>) -> Arc<Self> {
        Arc::new(Self {
            room,
            vault,
            _keep_alive: None,
        })
    }
    fn room(&self) -> Result<Arc<Room>, String> {
        self.room
            .upgrade()
            .ok_or_else(|| "The desk has closed.".into())
    }
}
#[async_trait]
impl CallRoom for RoomSide {
    fn upgrade(&self) -> Option<Arc<dyn CallRoom>> {
        let room = self.room.upgrade()?;
        Some(Arc::new(Self {
            room: self.room.clone(),
            vault: self.vault.clone(),
            _keep_alive: Some(room),
        }))
    }
    fn speech_root(&self) -> &std::path::Path {
        self.vault.root()
    }
    fn connected(&self) -> hotline_voice::speech::Connected {
        super::connected::connected(&self.vault)
    }
    fn call_began(&self, call: &str, persona: &str, ts: i64) {
        if let Ok(room) = self.room() {
            room.call_began(call, persona, ts);
        }
    }
    fn call_said(&self, call: &str, persona: &str, line: &Value) {
        if let Ok(room) = self.room() {
            room.call_said(call, persona, line);
        }
    }
    fn call_ended(&self, call: &str, persona: &str, end: End, outcome: &str, ts: i64) {
        if let Ok(room) = self.room() {
            room.call_ended(call, persona, end, outcome, ts);
        }
    }
    fn post(&self, persona: &str, thread: &ThreadId, text: &str) -> Result<(), String> {
        self.room()?.post(persona, thread, Post::words(text))
    }
    async fn start(&self, persona: &str) -> Result<(), String> {
        self.room()?.start(persona).await.map(|_| ())
    }
    async fn prompt(&self, persona: &str, text: &str) -> Result<(), String> {
        self.room()?.prompt(persona, text, None, None).await
    }
    fn info(&self, persona: &str) -> Option<SessionInfo> {
        self.room.upgrade().map(|room| room.info(persona))
    }
    fn voice_spoken(&self, persona: &str, event: &str, spoken: &str) -> bool {
        self.room
            .upgrade()
            .is_some_and(|room| room.voice_spoken(persona, event, spoken))
    }
    fn voice_record(&self, kind: &str, text: &str) -> Result<String, String> {
        self.room()?.voice_record(kind, text)
    }
    fn voice_push(&self, persona: &str, name: &str, text: &str) {
        if let Ok(room) = self.room() {
            room.voice_push(persona, name, text);
        }
    }
    fn front_reach(&self, persona: &Persona) -> Result<FrontReach, String> {
        let reach = self.room()?.front_reach(persona);
        let definitions = crate::mcp::server::TeammateTools::front_descriptors();
        let tools = Arc::new(reach.tools);
        let capability = reach.capability;
        let connect = if reach.servers.is_empty() {
            None
        } else {
            let persona = persona.id.clone();
            let capability = capability.clone();
            Some(Box::pin(async move {
                crate::mcp::connect_with_capability_and_vault(
                    &persona,
                    &reach.servers,
                    Some(capability),
                    reach.vault,
                )
                .await
            })
                as futures_util::future::BoxFuture<
                    'static,
                    crate::mcp::Connections,
                >)
        };
        Ok(FrontReach {
            tools: DeskTools {
                definitions,
                call: Arc::new(move |name, arguments| {
                    let tools = tools.clone();
                    Box::pin(async move { tools.call(&name, &arguments).await })
                }),
            },
            capability,
            connect,
        })
    }
    async fn command(
        &self,
        command: Command,
        log: &Log,
        room: &Arc<dyn RoomHandle>,
        calls: Arc<Calls>,
    ) -> Result<Value, String> {
        crate::wire::commands::run(
            command,
            log,
            room,
            &crate::wire::Parts {
                voice: Some(calls),
                ..Default::default()
            },
        )
        .await
    }
    fn background_work_allowed(&self, log: &Log, persona: &str) -> bool {
        crate::session::schedule::background_work_allowed(log, persona)
    }
    fn model_choices(&self, settings: &Map<String, Value>) -> Vec<ConfigChoice> {
        crate::models::choices(
            &self.vault.provider_auth(),
            &crate::models::enabled_models(settings),
            &self.vault.account_models(),
            &self.vault.model_metadata(),
        )
    }
    fn preferred_model(&self, settings: &Map<String, Value>) -> Option<String> {
        crate::models::preferred_model(settings)
    }
    fn model_price(&self, model: &str) -> ModelCost {
        crate::pricing::billed(
            model
                .split_once('/')
                .and_then(|(provider, _)| self.vault.connection(provider))
                .map(|(credential, _)| credential.credential_kind),
            model,
            crate::pricing::listed_price(model, &self.vault.model_metadata()),
        )
    }
    fn model_efforts(&self, model: &str) -> Vec<String> {
        crate::models::efforts(model)
    }
    async fn completion_builder(
        &self,
        model: &str,
        effort: Option<&str>,
        limit: u64,
    ) -> Result<rig::agent::AgentBuilder, String> {
        crate::driver::rig::completion_builder_with_effort(
            &self.vault.provider_auth(),
            model,
            effort,
            Some(limit),
        )
        .await
    }
    fn provider_connected(&self, provider: &str) -> bool {
        self.vault.provider_auth().contains_key(provider)
    }
    fn usage_cost(&self, price: &ModelCost, usage: &rig::completion::Usage) -> Option<f64> {
        let tokens = crate::pricing::Tokens::of(usage);
        tokens
            .reported
            .then(|| crate::pricing::cost(price, &tokens))
    }
}

pub mod dispatcher;
#[cfg(test)]
mod meter;
#[cfg(test)]
pub(crate) mod tests;
