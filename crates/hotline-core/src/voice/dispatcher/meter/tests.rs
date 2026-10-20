use super::*;
use crate::log::StreamId;
use futures_util::StreamExt;
use rig::agent::MultiTurnStreamItem;
use rig::agent::hook::{ModelTurnAction, ModelTurnFinished};
use rig::completion::{ToolDefinition, Usage};
use rig::message::ToolName;
use rig::streaming::{Item, StreamEvent};
use rig::test_utils::{MockCompletionModel, MockStreamEvent, MockTurn};
use rig::tool::{DynamicTool, ToolOutput};
use serde_json::json;

/// A dollar for each million tokens either way, so a cost reads as tokens.
const PRICE: ModelCost = ModelCost {
    input: 1.0,
    output: 1.0,
    cache_read: None,
    cache_write: None,
};
const OUTPUT_LIMIT: u64 = 512;

/// A desk with a paid provider connected, and its budget.
struct Fixture {
    _root: tempfile::TempDir,
    desk: Arc<crate::desk::Desk>,
    ledger: Arc<Budget>,
    vault: Arc<Vault>,
    provider: String,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::credentials::MemoryStore::default());
        let desk =
            Arc::new(crate::desk::Desk::open_with_store(root.path(), store.clone()).unwrap());
        let vault = Arc::new(Vault::open_with_store(root.path(), desk.log.clone(), store).unwrap());
        let credential = vault
            .save_custom(
                None,
                crate::contract::CustomProviderDraft {
                    name: "Fixture".into(),
                    base_url: "http://127.0.0.1:9/v1".into(),
                    api: crate::contract::OpenAiApi::ChatCompletions,
                    models: vec!["fast-mini".into()],
                    secret: Some("fixture-key".into()),
                },
            )
            .unwrap();
        Fixture {
            ledger: Arc::new(Budget::open(desk.log.clone())),
            _root: root,
            desk,
            vault,
            provider: credential.provider_id,
        }
    }

    fn meter(&self) -> (Meter, Arc<AtomicBool>) {
        Meter::new(
            self.ledger.clone(),
            PRICE,
            OUTPUT_LIMIT,
            self.vault.clone(),
            self.provider.clone(),
        )
    }

    fn spent(&self) -> f64 {
        self.ledger.spent().unwrap().day.dispatcher
    }

    fn chat_day_limit(&self, usd: f64) {
        self.desk
            .log
            .append(
                &StreamId::Room,
                &crate::room::room_event(
                    "setting",
                    json!({"id": "spending", "value": {"chat": {"dayUsd": usd}}}),
                ),
            )
            .unwrap();
    }
}

/// What `input` and `output` tokens cost at [`PRICE`].
fn usd(input: u64, output: u64) -> f64 {
    (input + output) as f64 / 1e6
}

fn usage(input: u64, output: u64) -> Usage {
    Usage::new()
        .input_tokens(input)
        .output_tokens(output)
        .total_tokens(input + output)
}

/// The least any call reserves: its whole output ceiling.
fn reserved_at_least() -> f64 {
    OUTPUT_LIMIT as f64 * PRICE.output / 1e6
}

fn near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-12,
        "{actual} is not {expected}"
    );
}

/// Asks for the turn once more the first time it finishes, as a hook
/// that rejects an answer does.
#[derive(Default)]
struct RetryOnce(AtomicBool);

impl AgentHook for RetryOnce {
    async fn on_model_turn_finished(
        &self,
        _: &HookContext,
        _: ModelTurnFinished<'_>,
    ) -> ModelTurnAction {
        if self.0.swap(true, Ordering::SeqCst) {
            ModelTurnAction::continue_run()
        } else {
            ModelTurnAction::repeat()
        }
    }
}

#[tokio::test]
async fn voice_meter_estimates_a_third_of_the_request_bytes_each_tools_overhead_and_the_whole_output_ceiling()
 {
    let fixture = Fixture::new();
    let (meter, _) = fixture.meter();
    let mut request = CompletionRequest::new("Any schedules?").preamble("Route it.");
    request.tools.push(Arc::new(ToolDefinition::new(
        ToolName::new("desk_command").unwrap(),
        "Read the room.",
        json!({"type": "object"}),
    )));
    let bytes = serde_json::to_vec(&request).unwrap().len();
    let input = (bytes + TOOL_OVERHEAD_BYTES).div_ceil(3) as u64;
    near(meter.estimate(&request), usd(input, OUTPUT_LIMIT));
}

#[tokio::test]
async fn voice_meter_a_provider_error_keeps_its_reservation() {
    let fixture = Fixture::new();
    let (meter, denied) = fixture.meter();
    let model = MockCompletionModel::from_turns([MockTurn::error("upstream refused")]);
    let agent = rig::AgentBuilder::new(model.clone()).build();
    let answer = agent.prompt("Hello").max_turns(1).add_hook(meter).await;
    assert!(answer.is_err());
    assert!(!denied.load(Ordering::SeqCst));
    assert_eq!(model.request_count(), 1);
    assert!(
        fixture.spent() >= reserved_at_least(),
        "{}",
        fixture.spent()
    );
}

#[tokio::test]
async fn voice_meter_missing_or_all_zero_usage_keeps_its_reservation() {
    for reported in [Usage::new(), usage(0, 0)] {
        let fixture = Fixture::new();
        let (meter, denied) = fixture.meter();
        let model =
            MockCompletionModel::from_turns([MockTurn::text("Hello.").with_usage(reported)]);
        let agent = rig::AgentBuilder::new(model).build();
        let answer = agent.prompt("Hello").add_hook(meter).await.unwrap();
        assert_eq!(answer.output(), "Hello.");
        assert!(!denied.load(Ordering::SeqCst));
        assert!(fixture.spent() >= reserved_at_least());
    }
}

#[tokio::test]
async fn voice_meter_a_paid_reply_survives_exhaustion_and_the_next_request_is_refused() {
    let fixture = Fixture::new();
    fixture.chat_day_limit(0.005);
    let model = MockCompletionModel::from_turns([
        MockTurn::text("Paid reply.").with_usage(usage(10_000, 10))
    ]);
    let agent = rig::AgentBuilder::new(model.clone()).build();
    let (meter, denied) = fixture.meter();
    let answer = agent.prompt("Hello").add_hook(meter).await.unwrap();
    assert_eq!(answer.output(), "Paid reply.");
    assert!(!denied.load(Ordering::SeqCst));
    near(fixture.spent(), usd(10_000, 10));

    let (meter, denied) = fixture.meter();
    assert!(agent.prompt("Again").add_hook(meter).await.is_err());
    assert!(denied.load(Ordering::SeqCst));
    assert_eq!(model.request_count(), 1);
    near(fixture.spent(), usd(10_000, 10));
}

#[tokio::test]
async fn voice_meter_a_retried_turn_settles_both_attempts() {
    let fixture = Fixture::new();
    let (meter, _) = fixture.meter();
    let model = MockCompletionModel::from_turns([
        MockTurn::text("First go.").with_usage(usage(1_000, 100)),
        MockTurn::text("Second go.").with_usage(usage(2_000, 200)),
    ]);
    let agent = rig::AgentBuilder::new(model.clone()).build();
    let answer = agent
        .prompt("Hello")
        .max_turns(2)
        .add_hook(meter)
        .add_hook(RetryOnce::default())
        .await
        .unwrap();
    assert_eq!(answer.output(), "Second go.");
    assert_eq!(model.request_count(), 2);
    // Each attempt was settled to what it cost, and neither estimate is left.
    near(fixture.spent(), usd(1_000, 100) + usd(2_000, 200));
}

/// A reply that breaks off, from the provider's side or because the call
/// stopped reading it, was still sent and may have been billed.
#[tokio::test]
async fn voice_meter_a_stream_dropped_mid_reply_stays_charged() {
    for provider_broke_off in [true, false] {
        let fixture = Fixture::new();
        let (meter, _) = fixture.meter();
        let ending = if provider_broke_off {
            MockStreamEvent::error("connection reset")
        } else {
            MockStreamEvent::final_response(usage(10, 10))
        };
        let model = MockCompletionModel::from_stream_turns([vec![
            MockStreamEvent::text("Hello there."),
            ending,
        ]]);
        let agent = rig::AgentBuilder::new(model).build();
        let mut stream = agent.prompt("Hello").max_turns(1).add_hook(meter).stream();
        if provider_broke_off {
            let mut failed = false;
            while let Some(item) = stream.next().await {
                failed |= item.is_err();
            }
            assert!(failed);
        } else {
            while let Some(item) = stream.next().await {
                if matches!(
                    item.unwrap(),
                    MultiTurnStreamItem::StreamAssistantItem(Item::Event(StreamEvent::Text { .. }))
                ) {
                    break;
                }
            }
            drop(stream);
        }
        assert!(
            fixture.spent() >= reserved_at_least(),
            "{provider_broke_off}: {}",
            fixture.spent()
        );
    }
}

/// A cost that spends the budget is written down in full, and the run is
/// refused at its next model call, before it goes out.
#[tokio::test]
async fn voice_meter_a_budget_refusal_lands_on_the_next_dispatch() {
    let fixture = Fixture::new();
    fixture.chat_day_limit(0.005);
    let (meter, denied) = fixture.meter();
    let model = MockCompletionModel::from_turns([
        MockTurn::tool_call("call_1", "look", json!({})).with_usage(usage(10_000, 10)),
        MockTurn::text("Nothing scheduled."),
    ]);
    let look = DynamicTool::new(
        ToolName::new("look").unwrap(),
        "Look.",
        json!({"type": "object"}),
        |_| Box::pin(async { Ok(ToolOutput::text("[]")) }),
    );
    let agent = rig::AgentBuilder::new(model.clone())
        .dynamic_tools(vec![look])
        .build();
    let answer = agent.prompt("Hello").max_turns(2).add_hook(meter).await;
    assert!(answer.is_err());
    assert!(denied.load(Ordering::SeqCst));
    assert_eq!(model.request_count(), 1, "refused before it went out");
    near(fixture.spent(), usd(10_000, 10));
}
