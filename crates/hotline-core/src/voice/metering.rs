//! Serialize reservations across calls and notification narration. The ledger
//! owns persistence; this coordinator owns permission to begin paid work.
use super::{
    ledger::{Budget as Balance, Exhausted, Kind, Ledger},
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
        VoiceSettings::from_room(&crate::room::settings(&self.log))
    }
    pub fn check(&self) -> Result<(), Exhausted> {
        let _held = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        self.ledger.check(&self.settings())
    }
    pub fn balance(&self) -> Balance {
        let _held = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        self.ledger.budget(&self.settings())
    }
    /// Persist a conservative estimate before issuing a request. An interrupted
    /// or lost request keeps its reservation: it may have been billed upstream.
    pub fn reserve(&self, kind: Kind, usd: f64) -> Result<(), Exhausted> {
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
        self.ledger.charge(kind, usd);
        match self.ledger.check(&settings) {
            Err(Exhausted::Unreadable) => Err(Exhausted::Unreadable),
            // A successful reservation may spend the last cent. That request
            // is paid for; the next reservation will be refused.
            _ => Ok(()),
        }
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
}
