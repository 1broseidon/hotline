//! Serialize reservations across calls and notification narration. The ledger
//! owns persistence; this coordinator owns permission to begin paid work.
//!
//! Each kind of work is spent against its own budget: transcription and
//! speech against Voice, the call assistant against Chat. A reservation is
//! refused when its budget would go over a limit; work that costs nothing is
//! never refused, and never touches the ledger.
use super::{
    ledger::{self, Exhausted, Kind, Ledger, Reservation, Spent},
    settings::VoiceSettings,
};
use crate::contract::{BudgetKind, BudgetLimits};
use crate::log::Log;
use std::sync::{Mutex, PoisonError};

pub const BUDGET_ERROR: &str = "The voice budget cannot authorize this request.";

pub struct Budget {
    ledger: Ledger,
    log: Log,
    gate: Mutex<()>,
}

impl Budget {
    pub fn open(log: Log) -> Self {
        Self {
            ledger: Ledger::open(log.root()),
            log,
            gate: Mutex::new(()),
        }
    }

    fn settings(&self) -> VoiceSettings {
        VoiceSettings::from_log(&self.log)
    }

    /// A budget's limits as the room has them now. Images are not spent here.
    pub fn limits(&self, budget: BudgetKind) -> BudgetLimits {
        limits(&self.settings(), budget)
    }

    /// What voice's kinds have spent today and this month.
    pub fn spent(&self) -> Result<Spent, Exhausted> {
        let _held = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        self.ledger.spent()
    }

    /// Whether paid work of these kinds may begin: each one's budget has
    /// something left. Free work names no kinds, so it is never refused, not
    /// even by a ledger that cannot be read.
    pub fn ready(&self, kinds: &[Kind]) -> Result<(), Exhausted> {
        let _held = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        self.ready_held(kinds)
    }

    fn ready_held(&self, kinds: &[Kind]) -> Result<(), Exhausted> {
        if kinds.is_empty() {
            return Ok(());
        }
        let settings = self.settings();
        let spent = self.ledger.spent()?;
        kinds
            .iter()
            .try_for_each(|kind| left(&settings, &spent, kind.budget()))
    }

    /// Persist a conservative estimate before issuing a request. Settle it
    /// with [`Budget::settle`] once the provider reports what it used. One
    /// that is never settled (the request failed, was interrupted, or came
    /// back without usage) stays charged: it may have been billed upstream.
    /// Speech is priced by what is sent, so its reservation is its cost.
    /// Nothing is never refused.
    pub fn reserve(&self, kind: Kind, usd: f64) -> Result<Reservation, Exhausted> {
        let _held = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        if !usd.is_finite() || usd < 0.0 {
            return Err(Exhausted::Unreadable);
        }
        if usd == 0.0 {
            return Ok(Reservation::free(kind));
        }
        let settings = self.settings();
        let spent = self.ledger.spent()?;
        let budget = kind.budget();
        left(&settings, &spent, budget)?;
        let limits = limits(&settings, budget);
        let over = |spent: f64, limit: Option<f64>| limit.is_some_and(|limit| usd > limit - spent);
        if over(spent.month.budget(budget), limits.month_usd) {
            return Err(Exhausted::Month(budget));
        }
        if over(spent.day.budget(budget), limits.day_usd) {
            return Err(Exhausted::Day(budget));
        }
        // A successful reservation may spend the last cent. That request is
        // paid for; the next reservation will be refused.
        let reservation = self
            .ledger
            .reserve(kind, usd)
            .ok_or(Exhausted::Unreadable)?;
        self.ledger.spent()?;
        Ok(reservation)
    }

    /// Replaces a reservation with what the request cost. A cost above the
    /// estimate was spent whatever the limits say, so it is written down in
    /// full, and `Err` then says its budget will not cover another request.
    pub fn settle(&self, reservation: Reservation, actual_usd: f64) -> Result<(), Exhausted> {
        let _held = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        let over = actual_usd > reservation.usd();
        let kind = reservation.kind();
        self.ledger.settle(reservation, actual_usd);
        if over {
            self.ready_held(&[kind])
        } else {
            Ok(())
        }
    }

    /// Writes down a cost that had no reservation, then says whether its
    /// budget covers another request.
    pub fn spend(&self, kind: Kind, usd: f64) -> Result<(), Exhausted> {
        let _held = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        if usd == 0.0 {
            return Ok(());
        }
        self.ledger.charge(kind, usd);
        self.ready_held(&[kind])
    }

    #[cfg(test)]
    pub(super) fn charge(&self, kind: Kind, usd: f64) {
        self.ledger.charge(kind, usd);
    }
}

fn limits(settings: &VoiceSettings, budget: BudgetKind) -> BudgetLimits {
    match budget {
        BudgetKind::Chat => settings.chat,
        BudgetKind::Voice => settings.voice,
        BudgetKind::Images => BudgetLimits::default(),
    }
}

fn left(settings: &VoiceSettings, spent: &Spent, budget: BudgetKind) -> Result<(), Exhausted> {
    ledger::left(
        budget,
        limits(settings, budget),
        spent.day.budget(budget),
        spent.month.budget(budget),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::sync::Arc;

    fn budget_with(spending: Value) -> (tempfile::TempDir, Budget) {
        let root = tempfile::tempdir().unwrap();
        let log = Log::open(root.path());
        log.append(
            &crate::log::StreamId::Room,
            &crate::room::room_event("setting", json!({"id": "spending", "value": spending})),
        )
        .unwrap();
        (root, Budget::open(log))
    }

    #[test]
    fn concurrent_requests_cannot_reserve_the_same_remaining_budget() {
        let (_root, budget) = budget_with(json!({"chat": {"dayUsd": 2, "monthUsd": 20}}));
        let budget = Arc::new(budget);
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let budget = budget.clone();
                std::thread::spawn(move || budget.reserve(Kind::Dispatcher, 0.75).is_ok())
            })
            .collect();
        let allowed = handles
            .into_iter()
            .map(|thread| usize::from(thread.join().unwrap()))
            .sum::<usize>();
        assert_eq!(allowed, 2);
        assert_eq!(budget.spent().unwrap().day.dispatcher, 1.5);
    }

    #[test]
    fn zero_limits_refuse_paid_work_and_let_free_work_through() {
        let (_root, budget) = budget_with(json!({"chat": {"dayUsd": 0}, "voice": {"monthUsd": 0}}));
        assert!(budget.ready(&[]).is_ok());
        assert!(budget.reserve(Kind::Tts, 0.0).is_ok());
        assert!(budget.reserve(Kind::Dispatcher, 0.0).is_ok());
        assert_eq!(
            budget.reserve(Kind::Tts, 0.01).err(),
            Some(Exhausted::Off(BudgetKind::Voice))
        );
        assert_eq!(
            budget.ready(&[Kind::Dispatcher]),
            Err(Exhausted::Off(BudgetKind::Chat))
        );
        assert_eq!(budget.spent().unwrap(), Spent::default());
    }

    #[test]
    fn with_no_limits_nothing_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let budget = Budget::open(Log::open(root.path()));
        for kind in [Kind::Stt, Kind::Tts, Kind::Dispatcher] {
            budget.reserve(kind, 400.0).unwrap();
        }
        assert!(
            budget
                .ready(&[Kind::Stt, Kind::Tts, Kind::Dispatcher])
                .is_ok()
        );
        assert_eq!(budget.spent().unwrap().day.total(), 1200.0);
    }

    #[test]
    fn each_budget_is_refused_on_its_own_limits() {
        let (_root, budget) = budget_with(json!({
            "chat": {"dayUsd": 1},
            "voice": {"dayUsd": 5, "monthUsd": 6},
            "images": {"dayUsd": 0}
        }));
        budget.reserve(Kind::Dispatcher, 1.0).unwrap();
        // Chat is spent for today; voice is not, and images are not voice's.
        assert_eq!(
            budget.reserve(Kind::Dispatcher, 0.01).err(),
            Some(Exhausted::Day(BudgetKind::Chat))
        );
        assert_eq!(
            budget.ready(&[Kind::Tts, Kind::Dispatcher]),
            Err(Exhausted::Day(BudgetKind::Chat))
        );
        assert!(budget.ready(&[Kind::Stt, Kind::Tts]).is_ok());
        budget.reserve(Kind::Stt, 2.0).unwrap();
        budget.reserve(Kind::Tts, 3.0).unwrap();
        assert_eq!(
            budget.reserve(Kind::Tts, 0.5).err(),
            Some(Exhausted::Day(BudgetKind::Voice))
        );
        assert_eq!(
            budget.ready(&[Kind::Stt]),
            Err(Exhausted::Day(BudgetKind::Voice))
        );
        assert_eq!(
            Exhausted::Day(BudgetKind::Voice).to_string(),
            "The Voice budget for today is spent. Raise it in Settings › Budgets."
        );
    }

    #[test]
    fn a_settled_reservation_frees_the_budget_it_did_not_spend() {
        let (_root, budget) = budget_with(json!({"chat": {"dayUsd": 2, "monthUsd": 20}}));
        let reservation = budget.reserve(Kind::Dispatcher, 1.5).unwrap();
        assert!(budget.reserve(Kind::Dispatcher, 1.0).is_err());
        budget.settle(reservation, 0.01).unwrap();
        assert!((budget.spent().unwrap().day.dispatcher - 0.01).abs() < 1e-12);
        assert!(budget.reserve(Kind::Dispatcher, 1.0).is_ok());
    }

    #[test]
    fn a_cost_above_the_estimate_is_kept_and_can_end_the_budget() {
        let (_root, budget) = budget_with(json!({"chat": {"dayUsd": 2, "monthUsd": 20}}));
        let reservation = budget.reserve(Kind::Dispatcher, 0.5).unwrap();
        assert_eq!(
            budget.settle(reservation, 2.5),
            Err(Exhausted::Day(BudgetKind::Chat))
        );
        assert!((budget.spent().unwrap().day.dispatcher - 2.5).abs() < 1e-12);
    }

    #[test]
    fn free_work_never_reads_a_ledger_that_cannot_be_read() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("voice-ledger.json"), "not json").unwrap();
        let budget = Budget::open(Log::open(root.path()));
        assert!(budget.ready(&[]).is_ok());
        assert!(budget.reserve(Kind::Tts, 0.0).is_ok());
        assert_eq!(budget.ready(&[Kind::Tts]), Err(Exhausted::Unreadable));
        assert_eq!(
            budget.reserve(Kind::Tts, 0.1).err(),
            Some(Exhausted::Unreadable)
        );
    }
}
