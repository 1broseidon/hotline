//! Sessions, vault reads and real tool handlers behind the call-room seam.
use crate::{
    contract::{ConfigChoice, End, ModelCost, Persona, SessionInfo, ThreadId},
    log::Log,
    session::Room,
    vault::Vault,
};
use async_trait::async_trait;
use hotline_voice::{
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
    fn card_notice(
        &self,
        card: &hotline_voice::cards::Card,
        first_person: bool,
    ) -> Option<hotline_voice::cards::Notice> {
        self.room().ok()?.call_card_notice(card, first_person)
    }
    async fn answer_card(
        &self,
        card: &hotline_voice::cards::Card,
        call: &str,
        yes: bool,
    ) -> Result<(), String> {
        self.room()?.answer_call_card(card, call, yes).await
    }
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
    fn call_began(&self, call: &ThreadId, persona: Option<&str>, ts: i64) {
        if let Some(persona) = persona
            && let Ok(room) = self.room()
        {
            room.call_began(call, persona, ts);
        }
    }
    fn call_said(&self, call: &ThreadId, persona: Option<&str>, line: &Value) {
        if let Ok(room) = self.room() {
            if let Some(persona) = persona {
                room.call_said(call, persona, line);
            } else if let Some(stream) = crate::log::StreamId::of_thread(call)
                && let Err(error) = room.log().append(&stream, line)
            {
                eprintln!("[voice] could not save desk call: {error}");
            }
        }
    }
    fn call_detached(&self, call: &ThreadId) {
        if let Ok(room) = self.room() {
            room.call_detached(call);
        }
    }
    fn call_ended(&self, call: &ThreadId, persona: Option<&str>, end: End, outcome: &str, ts: i64) {
        if let Some(persona) = persona
            && let Ok(room) = self.room()
        {
            room.call_ended(call, persona, end, outcome, ts);
        }
    }
    fn validate_call_thread(&self, persona: &str, thread: &ThreadId) -> Result<(), String> {
        self.room()?.validate_call_thread(persona, thread)
    }
    async fn call_prompt(
        &self,
        persona: &str,
        thread: &ThreadId,
        text: &str,
    ) -> Result<(), String> {
        let room = self.room()?;
        room.validate_call_thread(persona, thread)?;
        room.prompt_call_work(persona, thread, text).await
    }
    fn cancel_work(&self, persona: &str, thread: &ThreadId) -> Result<(), String> {
        let room = self.room()?;
        room.validate_call_thread(persona, thread)?;
        room.cancel_side(&thread.key)
    }
    fn info(&self, persona: &str) -> Option<SessionInfo> {
        self.room.upgrade().map(|room| room.info(persona))
    }
    fn call_spoken(&self, persona: &str, thread: &ThreadId, event: &str, spoken: &str) -> bool {
        self.room()
            .is_ok_and(|room| room.call_spoken(persona, thread, event, spoken))
    }
    fn voice_spoken(&self, persona: &str, event: &str, spoken: &str) -> bool {
        self.room
            .upgrade()
            .is_some_and(|room| room.voice_spoken(persona, event, spoken))
    }
    fn voice_push(&self, persona: &str, name: &str, text: &str) {
        if let Ok(room) = self.room() {
            room.voice_push(persona, name, text);
        }
    }
    fn front_reach(&self, persona: &Persona, thread: &ThreadId) -> Result<FrontReach, String> {
        let reach = self.room()?.front_reach(persona, thread)?;
        let definitions = crate::mcp::server::TeammateTools::front_descriptors(
            self.room()?.lead_call(&persona.id, thread),
        );
        let tools = Arc::new(reach.tools);
        let capability = reach.capability;
        Ok(FrontReach {
            tools: DeskTools {
                definitions,
                call: Arc::new(move |name, arguments| {
                    let tools = tools.clone();
                    Box::pin(async move { tools.call(&name, &arguments).await })
                }),
            },
            capability,
        })
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
        // Billed as the account the id names, else the provider's default.
        let (base, account) = crate::models::split_account(model);
        let kind = match account {
            Some(account) => self
                .vault
                .list()
                .into_iter()
                .find(|credential| credential.id == account)
                .map(|credential| credential.credential_kind),
            None => base
                .split_once('/')
                .and_then(|(provider, _)| self.vault.connection(provider))
                .map(|(credential, _)| credential.credential_kind),
        };
        crate::pricing::billed(
            kind,
            &base,
            crate::pricing::listed_price(&base, &self.vault.model_metadata()),
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
        // A call on a teammate's model runs on the teammate's account.
        let (base, account) = crate::models::split_account(model);
        let mut keys = self.vault.provider_auth();
        if let Some(account) = account {
            let provider = base.split('/').next().unwrap_or_default().to_string();
            match self.vault.account_auth(account) {
                Some((owner, held)) if owner == provider => {
                    keys.insert(owner, held);
                }
                _ => return Err(crate::models::account_gone(&provider)),
            }
        }
        crate::driver::rig::completion_builder_with_effort(&keys, &base, effort, Some(limit)).await
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
