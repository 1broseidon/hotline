//! Read-only tools bound to the call's thread and the teammate's current
//! capability lease. Work and writes run through the call's own agent,
//! except the desk lead's bounded named handoff.

use super::front::Tool;
use crate::call_room::CallRoom;
use futures_util::future::BoxFuture;
use hotline_contract::ThreadId;
use hotline_tools::capability::CapabilityLease;
use serde_json::{Map, Value, json};
use std::sync::Arc;
use std::time::Duration;

/// The most a tool's output may be before the model hears it. A voice cannot
/// use more, and the model's context is spent on what it reads.
pub const OUTPUT_CHARS: usize = 4_000;
/// A tool that has not answered by now is given up on, so the front can say so.
const CALL_LIMIT: Duration = Duration::from_secs(30);
pub(crate) struct FrontTools {
    room: Arc<dyn CallRoom>,
    thread: ThreadId,
    persona_id: String,
    teammate: DeskTools,
    capability: CapabilityLease,
}

impl FrontTools {
    /// Binds the permitted reads to this call.
    pub(crate) fn open(
        room: Arc<dyn CallRoom>,
        thread: &ThreadId,
        persona_id: &str,
        reach: FrontReach,
    ) -> Self {
        let FrontReach { tools, capability } = reach;
        Self {
            room,
            thread: thread.clone(),
            persona_id: persona_id.into(),
            teammate: tools,
            capability,
        }
    }

    /// Every function the front is given besides its own two.
    pub fn definitions(&self) -> Vec<Tool> {
        let built_in =
            self.teammate
                .definitions()
                .into_iter()
                .map(|(name, description, parameters)| Tool {
                    name,
                    description,
                    parameters,
                });
        built_in.collect()
    }

    pub fn check(&self) -> Result<(), String> {
        self.capability.check()
    }

    /// Whether `name` is one of these.
    pub fn has(&self, name: &str) -> bool {
        self.teammate
            .definitions()
            .iter()
            .any(|(known, _, _)| known == name)
    }

    /// Record admission before another speech event can be written.
    pub fn begin(&self, name: &str, arguments: &str) -> Value {
        let id = uuid::Uuid::new_v4().to_string();
        let line = json!({"kind":"tool", "id":id,
            "ts":chrono::Local::now().timestamp_millis(), "toolCallId":id, "title":name,
            "output":[{"type":"text", "text":arguments}],
            "status":"in_progress"});
        self.room
            .call_said(&self.thread, Some(&self.persona_id), &line);
        line
    }

    /// Run beside speech, completing the same transcript line even on hang-up.
    pub async fn call(
        &self,
        name: &str,
        arguments: &str,
        mut line: Value,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> String {
        let answer = tokio::select! {
            () = cancel.cancelled() => "The call ended before this lookup finished.".into(),
            answer = self.run(name, arguments) => answer,
        };
        line["status"] = json!("completed");
        line["output"] = json!([{"type":"text", "text":format!("{arguments}\n{answer}")}]);
        self.room
            .call_said(&self.thread, Some(&self.persona_id), &line);
        answer
    }

    async fn run(&self, name: &str, arguments: &str) -> String {
        let arguments: Value = if arguments.trim().is_empty() {
            Value::Object(Map::default())
        } else {
            match serde_json::from_str(arguments) {
                Ok(arguments) => arguments,
                Err(_) => return format!("The arguments to {name} were not JSON."),
            }
        };
        if let Err(refusal) = self.capability.check() {
            return refusal;
        }
        let run = (self.teammate.call)(name.to_string(), arguments);
        let answer = match tokio::time::timeout(CALL_LIMIT, run).await {
            Ok(Ok(output)) => output,
            Ok(Err(error)) => error,
            Err(_) => format!("{name} did not answer in time."),
        };
        hotline_room::text::clip(&answer, OUTPUT_CHARS)
    }
}

/// The built-in handlers remain bound to the core's teammate and lease.
pub type DeskToolCall =
    dyn Fn(String, Value) -> BoxFuture<'static, Result<String, String>> + Send + Sync;

pub struct DeskTools {
    pub definitions: Vec<(String, String, Value)>,
    pub call: Arc<DeskToolCall>,
}
impl DeskTools {
    fn definitions(&self) -> Vec<(String, String, Value)> {
        self.definitions.clone()
    }
}
/// The call thread's tools under the teammate's authority.
pub struct FrontReach {
    pub tools: DeskTools,
    pub capability: CapabilityLease,
}
