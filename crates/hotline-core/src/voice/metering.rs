//! Serialize reservations across calls and notification narration. The ledger
//! owns persistence; this coordinator owns permission to begin paid work.
use super::{
    ledger::{Budget as Balance, Exhausted, Kind, Ledger, Reservation},
    settings::VoiceSettings,
};
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
    pub fn check(&self) -> Result<(), Exhausted> {
        let _held = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        self.ledger.check(&self.settings())
    }
    pub fn balance(&self) -> Balance {
        let _held = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        self.ledger.budget(&self.settings())
    }
    /// Persist a conservative estimate before issuing a request. Settle it
    /// with [`Budget::settle`] once the provider reports what it used. One
    /// that is never settled (the request failed, was interrupted, or came
    /// back without usage) stays charged: it may have been billed upstream.
    /// Speech is priced by what is sent, so its reservation is its cost.
    pub fn reserve(&self, kind: Kind, usd: f64) -> Result<Reservation, Exhausted> {
        let _held = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        if !usd.is_finite() || usd < 0.0 {
            return Err(Exhausted::Unreadable);
        }
        let settings = self.settings();
        self.ledger.check(&settings)?;
        let balance = self.ledger.budget(&settings);
        if usd > balance.month_usd - balance.spent_month_usd {
            return Err(Exhausted::Month);
        }
        if usd > balance.day_usd - balance.spent_day_usd {
            return Err(Exhausted::Day);
        }
        let reservation = self.ledger.reserve(kind, usd);
        match (self.ledger.check(&settings), reservation) {
            (Err(Exhausted::Unreadable), _) | (_, None) => Err(Exhausted::Unreadable),
            // A successful reservation may spend the last cent. That request
            // is paid for; the next reservation will be refused.
            (_, Some(reservation)) => Ok(reservation),
        }
    }

    /// Replaces a reservation with what the request cost. A cost above the
    /// estimate was spent whatever the caps say, so it is written down in
    /// full, and `Err` then says the budget will not cover another request.
    pub fn settle(&self, reservation: Reservation, actual_usd: f64) -> Result<(), Exhausted> {
        let _held = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        let over = actual_usd > reservation.usd();
        self.ledger.settle(reservation, actual_usd);
        if over {
            self.ledger.check(&self.settings())
        } else {
            Ok(())
        }
    }
    /// Writes down a cost that had no reservation, then says whether the
    /// budget covers another request.
    pub fn spend(&self, kind: Kind, usd: f64) -> Result<(), Exhausted> {
        let _held = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        self.ledger.charge(kind, usd);
        self.ledger.check(&self.settings())
    }
    #[cfg(test)]
    pub(super) fn charge(&self, kind: Kind, usd: f64) {
        self.ledger.charge(kind, usd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    #[test]
    fn concurrent_requests_cannot_reserve_the_same_remaining_budget() {
        let root = tempfile::tempdir().unwrap();
        let budget = Arc::new(Budget::open(Log::open(root.path())));
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
        assert_eq!(budget.balance().spent_day_usd, 1.5);
    }

    #[test]
    fn zero_limits_refuse_paid_voice_and_let_free_voice_through() {
        let root = tempfile::tempdir().unwrap();
        let log = Log::open(root.path());
        log.append(
            &crate::log::StreamId::Room,
            &crate::room::room_event(
                "setting",
                serde_json::json!({"id": "spending", "value": {"dayUsd": 0, "monthUsd": 0}}),
            ),
        )
        .unwrap();
        let budget = Budget::open(log);
        assert!(budget.check().is_ok());
        assert!(budget.reserve(Kind::Tts, 0.0).is_ok());
        assert_eq!(
            budget.reserve(Kind::Tts, 0.01).err(),
            Some(Exhausted::Month)
        );
        assert_eq!(budget.balance().spent_day_usd, 0.0);
    }

    #[test]
    fn a_settled_reservation_frees_the_budget_it_did_not_spend() {
        let root = tempfile::tempdir().unwrap();
        let budget = Budget::open(Log::open(root.path()));
        let reservation = budget.reserve(Kind::Dispatcher, 1.5).unwrap();
        assert!(budget.reserve(Kind::Dispatcher, 1.0).is_err());
        budget.settle(reservation, 0.01).unwrap();
        assert!((budget.balance().spent_day_usd - 0.01).abs() < 1e-12);
        assert!(budget.reserve(Kind::Dispatcher, 1.0).is_ok());
    }

    #[test]
    fn a_cost_above_the_estimate_is_kept_and_can_end_the_budget() {
        let root = tempfile::tempdir().unwrap();
        let budget = Budget::open(Log::open(root.path()));
        let reservation = budget.reserve(Kind::Dispatcher, 0.5).unwrap();
        assert_eq!(budget.settle(reservation, 2.5), Err(Exhausted::Day));
        assert!((budget.balance().spent_day_usd - 2.5).abs() < 1e-12);
    }
}
