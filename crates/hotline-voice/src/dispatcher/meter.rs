//! Meters the call assistant's model calls on the Chat budget. Rig gives each
//! model call it dispatches an id, and reports that call's outcome exactly
//! once, streamed or not, accepted, retried or abandoned. So each dispatch
//! keeps its own reservation: `on_dispatch` reserves an estimate, or denies
//! the call when the budget will not cover it, and `on_outcome` settles that
//! dispatch's reservation to what it cost. Two attempts at one turn are two
//! calls the provider bills, and are settled apart.
//!
//! A call that fails, is abandoned, or comes back without usage keeps its
//! reservation, because the provider may have billed it. A call whose stream
//! is dropped before its outcome keeps it too. An outcome must not discard
//! a paid reply, so a cost that spends the budget is refused at the next
//! dispatch, and the answer it paid for is still given.

use crate::call_room::CallRoom;
use crate::{
    ledger::{Kind, Reservation},
    lock,
    metering::{BUDGET_ERROR, Budget},
};
use hotline_contract::ModelCost;
use rig::agent::hook::{
    AgentHook, DispatchAction, DispatchEvent, HookContext, OutcomeAction, OutcomeEvent,
};
use rig::completion::CompletionRequest;
use rig::effect::EffectId;
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

/// What a provider adds to a request for each tool it offers, beyond the
/// tool's definition, in bytes: its own tool-use instructions (Anthropic's
/// are about 300 tokens).
pub(crate) const TOOL_OVERHEAD_BYTES: usize = 1024;

/// The voice budget's hook on one request's model calls.
#[derive(Clone)]
pub struct Meter {
    ledger: Arc<Budget>,
    price: ModelCost,
    /// The output ceiling the request was built with.
    output_limit: u64,
    /// Each model call's reservation, by the id Rig dispatched it under,
    /// until its outcome settles it.
    reservations: Arc<Mutex<HashMap<EffectId, Reservation>>>,
    /// Set when a model call was refused for the budget, so the run's error
    /// can be said as the budget's.
    denied: Arc<AtomicBool>,
    room: Arc<dyn CallRoom>,
    provider: String,
}

impl Meter {
    /// A meter for a request to `provider`'s model at `price`, built with
    /// `output_limit`, and the flag it raises when the budget refuses a call.
    pub fn new(
        ledger: Arc<Budget>,
        price: ModelCost,
        output_limit: u64,
        room: Arc<dyn CallRoom>,
        provider: String,
    ) -> (Self, Arc<AtomicBool>) {
        let denied = Arc::new(AtomicBool::new(false));
        let meter = Meter {
            ledger,
            price,
            output_limit,
            reservations: Arc::default(),
            denied: denied.clone(),
            room,
            provider,
        };
        (meter, denied)
    }

    /// A conservative estimate for a model call: about three bytes of the
    /// request as it goes out (preamble, history and tool definitions) to an
    /// input token, the provider's own instructions for each tool, and the
    /// whole output ceiling.
    pub fn estimate(&self, request: &CompletionRequest) -> f64 {
        let bytes = serde_json::to_vec(request).map_or(usize::MAX, |bytes| bytes.len());
        let input = bytes
            .saturating_add(request.tools.len().saturating_mul(TOOL_OVERHEAD_BYTES))
            .div_ceil(3);
        (input as f64 * self.price.input + self.output_limit as f64 * self.price.output)
            / 1_000_000.0
    }
}

impl AgentHook for Meter {
    async fn on_dispatch(&self, _: &HookContext, event: DispatchEvent<'_>) -> DispatchAction {
        let Some(request) = event.completion_request() else {
            return DispatchAction::Proceed;
        };
        if !self.room.provider_connected(&self.provider) {
            return DispatchAction::stop("The dispatcher provider has been disconnected.");
        }
        if let Ok(reservation) = self
            .ledger
            .reserve(Kind::Dispatcher, self.estimate(request))
        {
            lock(&self.reservations).insert(event.id, reservation);
            DispatchAction::Proceed
        } else {
            self.denied.store(true, Ordering::SeqCst);
            DispatchAction::stop(BUDGET_ERROR)
        }
    }

    async fn on_outcome(&self, _: &HookContext, event: OutcomeEvent<'_>) -> OutcomeAction {
        let Some(reservation) = lock(&self.reservations).remove(&event.id) else {
            return OutcomeAction::Proceed;
        };
        if let Some(cost) = event
            .completion()
            .and_then(|response| self.room.usage_cost(&self.price, &response.usage))
        {
            // Over the budget, the cost is still written down in full; the
            // next dispatch is the one refused.
            self.ledger.settle(reservation, cost).ok();
        }
        OutcomeAction::Proceed
    }
}
