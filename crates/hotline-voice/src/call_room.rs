//! The room operations a call uses. The room owns sessions, grants and writes;
//! voice owns the call and its sound. Temporary operations name their removal slice.

use crate::{front_tools::FrontReach, speech::Connected};
use async_trait::async_trait;
use hotline_contract::{ConfigChoice, End, ModelCost, Persona, SessionInfo, ThreadId};
use hotline_store::log::Log;
use serde_json::{Map, Value};
use std::sync::Arc;

tokio::task_local! { pub static VOICE_COMMAND: (); }
tokio::task_local! { pub static PROMPT_CLIENT: hotline_contract::Client; }

pub fn prompt_client() -> Option<hotline_contract::Client> {
    PROMPT_CLIENT.try_with(|client| *client).ok()
}

#[async_trait]
pub trait CallRoom: Send + Sync {
    /// Hold the live room for one operation; calls themselves do not own it.
    /// `first_person` when the voice on the call is the card's own teammate.
    fn card_notice(
        &self,
        card: &crate::cards::Card,
        first_person: bool,
    ) -> Option<crate::cards::Notice>;
    async fn answer_card(
        &self,
        card: &crate::cards::Card,
        call: &str,
        yes: bool,
    ) -> Result<(), String>;
    fn upgrade(&self) -> Option<Arc<dyn CallRoom>>;
    /// The local engine's directory, without opening provider credentials.
    fn speech_root(&self) -> &std::path::Path;
    fn connected(&self) -> Connected;
    fn call_began(&self, call: &ThreadId, persona: Option<&str>, ts: i64);
    fn call_said(&self, call: &ThreadId, persona: Option<&str>, line: &Value);
    /// Mark the current turn synchronously; recording the end may lag behind it.
    fn call_detached(&self, call: &ThreadId);
    fn call_ended(&self, call: &ThreadId, persona: Option<&str>, end: End, outcome: &str, ts: i64);
    fn validate_call_thread(&self, persona: &str, thread: &ThreadId) -> Result<(), String>;
    async fn call_prompt(&self, persona: &str, thread: &ThreadId, text: &str)
    -> Result<(), String>;
    fn cancel_work(&self, persona: &str, thread: &ThreadId) -> Result<(), String>;
    /// Model information for reply diagnostics.
    fn info(&self, persona: &str) -> Option<SessionInfo>;
    /// Keeps a Turn by turn reply's spoken version on its own thread.
    fn call_spoken(&self, persona: &str, thread: &ThreadId, event: &str, spoken: &str) -> bool;
    fn voice_spoken(&self, persona: &str, event: &str, spoken: &str) -> bool;
    /// The no-lead fallback may narrate a completed handoff after hang-up.
    fn voice_push(&self, persona: &str, name: &str, text: &str);
    /// Tools bound to the call and the teammate's authority.
    fn front_reach(&self, persona: &Persona, thread: &ThreadId) -> Result<FrontReach, String>;
    /// The no-lead fallback retains the ordinary scheduling grant.
    fn background_work_allowed(&self, log: &Log, persona: &str) -> bool;
    /// The no-lead fallback's model selection.
    fn model_choices(&self, settings: &Map<String, Value>) -> Vec<ConfigChoice>;
    /// The no-lead fallback's default model lookup.
    fn preferred_model(&self, settings: &Map<String, Value>) -> Option<String>;
    /// The no-lead fallback's model pricing lookup.
    fn model_price(&self, model: &str) -> ModelCost;
    /// The no-lead fallback's effort lookup.
    fn model_efforts(&self, model: &str) -> Vec<String>;
    /// Builds the no-lead assistant and Turn by turn speech rewrites.
    async fn completion_builder(
        &self,
        model: &str,
        effort: Option<&str>,
        limit: u64,
    ) -> Result<rig::agent::AgentBuilder, String>;
    /// The fallback call assistant's per-dispatch credential check.
    fn provider_connected(&self, provider: &str) -> bool;
    /// The fallback call assistant's token accounting.
    fn usage_cost(&self, price: &ModelCost, usage: &rig::completion::Usage) -> Option<f64>;
}
