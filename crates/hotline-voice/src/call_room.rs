//! The room operations a call uses. The room owns sessions, grants and writes;
//! voice owns the call and its sound. Temporary operations name their removal slice.

use crate::{Calls, Origin, front_tools::FrontReach, speech::Connected};
use async_trait::async_trait;
use hotline_contract::{Command, ConfigChoice, End, ModelCost, Persona, SessionInfo, ThreadId};
use hotline_room::room_handle::RoomHandle;
use hotline_store::log::Log;
use serde_json::{Map, Value};
use std::sync::Arc;

tokio::task_local! { pub static VOICE_COMMAND: (); }
tokio::task_local! { pub static CALL_ORIGIN: Origin; }
tokio::task_local! { pub static PROMPT_CLIENT: hotline_contract::Client; }

pub fn prompt_client() -> Option<hotline_contract::Client> {
    PROMPT_CLIENT.try_with(|client| *client).ok()
}

#[async_trait]
pub trait CallRoom: Send + Sync {
    /// Hold the live room for one operation; calls themselves do not own it.
    fn upgrade(&self) -> Option<Arc<dyn CallRoom>>;
    /// The local engine's directory, without opening provider credentials.
    fn speech_root(&self) -> &std::path::Path;
    fn connected(&self) -> Connected;
    fn call_began(&self, call: &ThreadId, persona: &str, ts: i64);
    fn call_said(&self, call: &ThreadId, persona: &str, line: &Value);
    fn call_ended(&self, call: &ThreadId, persona: &str, end: End, outcome: &str, ts: i64);
    fn validate_call_thread(&self, persona: &str, thread: &ThreadId) -> Result<(), String>;
    async fn call_prompt(&self, persona: &str, thread: &ThreadId, text: &str)
    -> Result<(), String>;
    fn post(&self, persona: &str, thread: &ThreadId, text: &str) -> Result<(), String>;
    /// S6 replaces the live front's DM startup with the thread queue.
    async fn start(&self, persona: &str) -> Result<(), String>;
    /// S6 moves live-front delegations from the DM to the thread queue.
    async fn prompt(&self, persona: &str, text: &str) -> Result<(), String>;
    /// The live front still observes its delegated DM turn until S6.
    fn info(&self, persona: &str) -> Option<SessionInfo>;
    /// Keeps a Legacy reply's spoken version on its own thread.
    fn call_spoken(&self, persona: &str, thread: &ThreadId, event: &str, spoken: &str) -> bool;
    fn voice_spoken(&self, persona: &str, event: &str, spoken: &str) -> bool;
    /// task-203 removes the desk dispatcher tape when desk calls reach the lead.
    fn voice_record(&self, kind: &str, text: &str) -> Result<String, String>;
    /// task-203 removes the dispatcher's out-of-call narration.
    fn voice_push(&self, persona: &str, name: &str, text: &str);
    /// S6 replaces the front's DM reach with the call thread's lease.
    fn front_reach(&self, persona: &Persona) -> Result<FrontReach, String>;
    /// task-203 removes the dispatcher's command table.
    async fn command(
        &self,
        command: Command,
        log: &Log,
        room: &Arc<dyn RoomHandle>,
        calls: Arc<Calls>,
    ) -> Result<Value, String>;
    /// task-203 removes scheduling through the desk dispatcher.
    fn background_work_allowed(&self, log: &Log, persona: &str) -> bool;
    /// task-203 removes the desk dispatcher's model selection.
    fn model_choices(&self, settings: &Map<String, Value>) -> Vec<ConfigChoice>;
    /// task-203 removes the desk dispatcher's default model lookup.
    fn preferred_model(&self, settings: &Map<String, Value>) -> Option<String>;
    /// task-203 removes the desk dispatcher's model pricing lookup.
    fn model_price(&self, model: &str) -> ModelCost;
    /// task-203 removes the desk dispatcher's effort lookup.
    fn model_efforts(&self, model: &str) -> Vec<String>;
    /// S6/task-203 remove delegated and desk-dispatcher model calls.
    async fn completion_builder(
        &self,
        model: &str,
        effort: Option<&str>,
        limit: u64,
    ) -> Result<rig::agent::AgentBuilder, String>;
    /// S6/task-203 remove the call assistant's per-dispatch credential check.
    fn provider_connected(&self, provider: &str) -> bool;
    /// S6/task-203 remove the call assistant's token accounting.
    fn usage_cost(&self, price: &ModelCost, usage: &rig::completion::Usage) -> Option<f64>;
}
