//! Reading the account's plan without a turn.
//!
//! Claude Code answers `/usage` itself: its adapter reads the limits with a
//! control request, makes no model call, and says them as one agent message
//! ([`super::plan::usage_markdown`]). So the plan is read by saying `/usage`
//! on the connection the teammate already has, between its turns, with
//! everything the agent says meanwhile diverted into the reading
//! ([`Live::probe`]) rather than onto the tape.
//!
//! It is only ever said with no prompt out: not while a turn runs, not while
//! a line is being steered in, not while the agent works on something it
//! took up by itself. It holds the operation gate a prompt takes, so a
//! prompt that arrives meanwhile waits for it rather than sharing its words.

use super::plan::usage_markdown::{self, USAGE_COMMAND};
use super::*;
use crate::contract::PlanUsage;

/// The backend id of Claude Code, whose adapter answers `/usage` itself.
pub(super) const CLAUDE_CODE: &str = "claude-acp";

/// How long `/usage` has to answer. It is a local read, so a second or two
/// is usual; this bounds how long a prompt can wait behind it.
const PROBE_REPLY: Duration = Duration::from_secs(10);

/// How long a `/usage` that ran out of time has to answer its cancel, so
/// that nothing it says late lands in the next turn.
const PROBE_CANCELLED: Duration = Duration::from_secs(5);

impl ChildAgent {
    /// The account's plan, read now, when this agent can say it without a
    /// turn.
    pub(super) async fn read_plan_now(&self) -> Option<PlanUsage> {
        match self.backend_id.as_str() {
            CLAUDE_CODE => self.ask_usage().await,
            _ => None,
        }
    }

    /// Says `/usage` between turns and reads the answer, or does nothing
    /// when the agent is not idle.
    async fn ask_usage(&self) -> Option<PlanUsage> {
        let _gate = self.operation_gate.lock().await;
        let live = &self.live;
        if self.check_capability().is_err()
            || live.failed.load(Ordering::SeqCst)
            || live.replaying.load(Ordering::SeqCst)
            || lock(&live.auth).as_ref().is_some_and(|a| a.running())
            || lock(&live.startup_failure).is_some()
        {
            return None;
        }
        let connection = lock(&live.connection).clone()?;
        let session_id = lock(&live.session).id.clone()?;
        {
            // A turn or an activity of the agent's own holds `updates`, and a
            // steered line holds the lane: any of them is a prompt out.
            let updates = lock(&live.updates);
            if updates.is_some()
                || lock(&live.unprompted_open).is_some()
                || lock(&live.steering).is_some()
            {
                return None;
            }
            *lock(&live.probe) = Some(String::new());
        }
        let reply = connection
            .send_request(PromptRequest::new(
                session_id.clone(),
                vec![text_block(USAGE_COMMAND)],
            ))
            .block_task();
        tokio::pin!(reply);
        let answered = if let Ok(answered) = tokio::time::timeout(PROBE_REPLY, &mut reply).await {
            answered.is_ok()
        } else {
            let _ = connection.send_notification(CancelNotification::new(session_id));
            let _ = tokio::time::timeout(PROBE_CANCELLED, &mut reply).await;
            false
        };
        let answer = lock(&live.probe).take().unwrap_or_default();
        if !answered {
            return None;
        }
        usage_markdown::parse(&answer, crate::session::now_ms())
    }
}
