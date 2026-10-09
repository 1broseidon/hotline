//! What voice has cost today and this month, kept on disk so a restart does
//! not forgive it. For a local desk it is a guard against your own bill; for a
//! server desk it is required.
//!
//! It fails closed. A file that cannot be read, or a charge that cannot be
//! written down, ends calls until it can: an unknown balance is not a
//! balance of zero. Every check tries a write that failed again, so a disk
//! that comes back turns voice back on without a restart.
//!
//! Work whose price is known only afterwards is reserved first and settled
//! once the provider reports what it used: [`Ledger::reserve`] writes the
//! estimate down, and [`Ledger::settle`] replaces it with the actual cost. A
//! reservation that is never settled (the request failed, was cancelled, or
//! came back without usage) stays charged, since it may have been billed.

use crate::contract::{BudgetKind, BudgetLimits};
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

const FILE: &str = "voice-ledger.json";

/// What a charge was for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Stt,
    Tts,
    Dispatcher,
}

impl Kind {
    /// The budget it is spent against: hearing and speaking are Voice, the
    /// call assistant is Chat.
    pub fn budget(self) -> BudgetKind {
        match self {
            Kind::Stt | Kind::Tts => BudgetKind::Voice,
            Kind::Dispatcher => BudgetKind::Chat,
        }
    }
}

/// Why paid work may not run. Each has a sentence for a person, naming the
/// budget that stopped it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exhausted {
    /// The budget's daily limit is spent.
    Day(BudgetKind),
    /// The budget's monthly limit is spent.
    Month(BudgetKind),
    /// A limit of the budget is zero, which turns its paid use off.
    Off(BudgetKind),
    /// The ledger cannot be read or written, so the balance is not known.
    Unreadable,
}

impl fmt::Display for Exhausted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Exhausted::Day(budget) => write!(
                f,
                "The {} budget for today is spent. Raise it in Settings › Budgets.",
                budget.name()
            ),
            Exhausted::Month(budget) => write!(
                f,
                "The {} budget for this month is spent. Raise it in Settings › Budgets.",
                budget.name()
            ),
            Exhausted::Off(budget) => write!(
                f,
                "The {} budget is set to zero, so its paid use is off. Raise it in Settings › Budgets.",
                budget.name()
            ),
            Exhausted::Unreadable => write!(
                f,
                "Voice can't check its budget, so paid calls are off. Chat carries on by text."
            ),
        }
    }
}

impl std::error::Error for Exhausted {}

/// Whether a budget that has spent `day` today and `month` this month has
/// anything left under `limits`. No limit is never spent; a zero limit is off.
pub fn left(
    budget: BudgetKind,
    limits: BudgetLimits,
    day: f64,
    month: f64,
) -> Result<(), Exhausted> {
    let spent = |total: f64, limit: Option<f64>| limit.is_some_and(|limit| total >= limit);
    if limits.off() {
        Err(Exhausted::Off(budget))
    } else if spent(month, limits.month_usd) {
        Err(Exhausted::Month(budget))
    } else if spent(day, limits.day_usd) {
        Err(Exhausted::Day(budget))
    } else {
        Ok(())
    }
}

/// What each kind has spent today and this month.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spent {
    pub day: Spend,
    pub month: Spend,
}

/// Spend by kind, for a day or a month.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Spend {
    pub stt: f64,
    pub tts: f64,
    pub dispatcher: f64,
}

impl Spend {
    pub fn total(&self) -> f64 {
        self.stt + self.tts + self.dispatcher
    }

    /// What was spent against one budget.
    pub fn budget(&self, budget: BudgetKind) -> f64 {
        match budget {
            BudgetKind::Voice => self.stt + self.tts,
            BudgetKind::Chat => self.dispatcher,
            BudgetKind::Images => 0.0,
        }
    }

    fn of(&mut self, kind: Kind) -> &mut f64 {
        match kind {
            Kind::Stt => &mut self.stt,
            Kind::Tts => &mut self.tts,
            Kind::Dispatcher => &mut self.dispatcher,
        }
    }

    fn add(&mut self, kind: Kind, usd: f64) {
        *self.of(kind) += usd;
    }

    /// Takes back part of a charge, never below `floor`.
    fn refund(&mut self, kind: Kind, usd: f64, floor: f64) {
        let spent = self.of(kind);
        *spent = (*spent - usd).max(floor).max(0.0);
    }

    fn sane(&self) -> bool {
        [self.stt, self.tts, self.dispatcher]
            .iter()
            .all(|usd| usd.is_finite() && *usd >= 0.0)
    }
}

/// What is on disk: the day and the month the spend is for, and the spend.
/// Dates are `YYYY-MM-DD` and `YYYY-MM`, which sort as dates do.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    day: String,
    month: String,
    day_spend: Spend,
    month_spend: Spend,
}

impl Record {
    /// Starts a new day or month when `today` is in one. A clock that has
    /// gone backwards keeps counting against the later day, never forgives it.
    fn roll_to(&mut self, today: NaiveDate) {
        let day = today.format("%Y-%m-%d").to_string();
        let month = today.format("%Y-%m").to_string();
        if day > self.day {
            self.day = day;
            self.day_spend = Spend::default();
        }
        if month > self.month {
            self.month = month;
            self.month_spend = Spend::default();
        }
    }

    fn sane(&self) -> bool {
        self.day_spend.sane() && self.month_spend.sane()
    }
}

/// An estimate written down before the work it pays for, and where it was
/// written: the day and month it counts against. Settling it moves the spend
/// to what the work cost; dropping it leaves the estimate charged.
#[derive(Debug)]
pub struct Reservation {
    kind: Kind,
    usd: f64,
    day: String,
    month: String,
}

impl Reservation {
    /// A reservation for work that costs nothing. It is never written down,
    /// so free work runs whatever the ledger or the budgets say.
    pub fn free(kind: Kind) -> Self {
        Reservation {
            kind,
            usd: 0.0,
            day: String::new(),
            month: String::new(),
        }
    }

    pub fn usd(&self) -> f64 {
        self.usd
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }
}

struct State {
    /// `None` until the file has been read; a failed read leaves it `None`
    /// and is tried again on the next call, so mending the file mends the ledger.
    record: Option<Record>,
    /// Goes up with every change to `record`, so a write that arrives late
    /// never puts an older balance over a newer one.
    version: u64,
}

pub struct Ledger {
    path: PathBuf,
    /// The balance in memory. Held for arithmetic and a copy, never for the disk.
    state: Mutex<State>,
    /// The newest version on disk. Held for the write itself, which is what
    /// keeps two writes from interleaving, and never together with `state`:
    /// a check must not wait on somebody's fsync.
    saved: Mutex<u64>,
    /// The last write failed, so what is in memory is not what is on disk.
    unsaved: AtomicBool,
}

impl Ledger {
    /// The ledger under a desk's data directory. Nothing is read until it is asked.
    pub fn open(data_root: &Path) -> Ledger {
        Ledger {
            path: data_root.join(FILE),
            state: Mutex::new(State {
                record: None,
                version: 0,
            }),
            saved: Mutex::new(0),
            unsaved: AtomicBool::new(false),
        }
    }

    /// What has been spent today and this month. `Unreadable` when the file
    /// cannot be read or the last change could not be written down.
    pub fn spent(&self) -> Result<Spent, Exhausted> {
        self.spent_on(today())
    }

    /// Writes down a cost.
    pub fn charge(&self, kind: Kind, usd: f64) {
        self.charge_on(today(), kind, usd);
    }

    /// Writes down an estimate to be settled later. `None` when it could not
    /// be written down, which [`Ledger::spent`] then reports.
    pub fn reserve(&self, kind: Kind, usd: f64) -> Option<Reservation> {
        self.charge_on(today(), kind, usd)
    }

    /// Replaces a reservation with what the work cost.
    pub fn settle(&self, reservation: Reservation, actual_usd: f64) {
        self.settle_on(today(), reservation, actual_usd);
    }

    fn spent_on(&self, today: NaiveDate) -> Result<Spent, Exhausted> {
        if self.still_unsaved() {
            return Err(Exhausted::Unreadable);
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        self.load(&mut state)?;
        let Some(record) = state.record.as_mut() else {
            return Err(Exhausted::Unreadable);
        };
        record.roll_to(today);
        Ok(Spent {
            day: record.day_spend,
            month: record.month_spend,
        })
    }

    fn charge_on(&self, today: NaiveDate, kind: Kind, usd: f64) -> Option<Reservation> {
        let usd = if usd.is_finite() { usd.max(0.0) } else { 0.0 };
        let changed = self.change_on(today, |record| {
            record.day_spend.add(kind, usd);
            record.month_spend.add(kind, usd);
        });
        let Some((day, month)) = changed else {
            eprintln!(
                "[voice] the ledger could not be read, so ${usd:.4} for {kind:?} was not recorded"
            );
            return None;
        };
        Some(Reservation {
            kind,
            usd,
            day,
            month,
        })
    }

    /// Moves a reservation to the actual cost. More than the estimate is
    /// charged today in full, because it was spent. Less is refunded from the
    /// day and month the estimate was charged to, while they are still the
    /// ledger's; a day that has since rolled over keeps its estimate. A month
    /// never ends up below its day. An unknown cost keeps the reservation.
    fn settle_on(&self, today: NaiveDate, reservation: Reservation, actual_usd: f64) {
        if !actual_usd.is_finite() || actual_usd < 0.0 {
            return;
        }
        let Reservation {
            kind,
            usd,
            day,
            month,
        } = reservation;
        if actual_usd >= usd {
            if actual_usd > usd {
                self.charge_on(today, kind, actual_usd - usd);
            }
            return;
        }
        let refund = usd - actual_usd;
        let changed = self.change_on(today, |record| {
            if record.day == day {
                record.day_spend.refund(kind, refund, 0.0);
            }
            if record.month == month {
                let floor = if record.day.starts_with(month.as_str()) {
                    *record.day_spend.of(kind)
                } else {
                    0.0
                };
                record.month_spend.refund(kind, refund, floor);
            }
        });
        if changed.is_none() {
            eprintln!(
                "[voice] the ledger could not be read, so ${refund:.4} for {kind:?} was not refunded"
            );
        }
    }

    /// Applies a change to the record as of `today` and writes it down. The
    /// day and month the record is for, or `None` when it could not be read.
    fn change_on(
        &self,
        today: NaiveDate,
        change: impl FnOnce(&mut Record),
    ) -> Option<(String, String)> {
        let (version, bytes, period) = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            self.load(&mut state).ok()?;
            let record = state.record.as_mut()?;
            record.roll_to(today);
            change(record);
            let period = (record.day.clone(), record.month.clone());
            let bytes = serde_json::to_vec_pretty(record);
            state.version += 1;
            (state.version, bytes, period)
        };
        match bytes {
            Ok(bytes) => self.persist(version, &bytes),
            Err(error) => {
                eprintln!(
                    "[voice] the ledger could not be written, so calls are off until it can: {error}"
                );
                self.unsaved.store(true, Ordering::SeqCst);
            }
        }
        Some(period)
    }

    fn load(&self, state: &mut State) -> Result<(), Exhausted> {
        if state.record.is_none() {
            state.record = Some(self.read()?);
        }
        Ok(())
    }

    /// Whether the last write is still failing. A failed one is tried again
    /// here, with the whole balance as it is now, so the moment the disk takes
    /// a write voice is back.
    fn still_unsaved(&self) -> bool {
        if !self.unsaved.load(Ordering::SeqCst) {
            return false;
        }
        let now = {
            let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state
                .record
                .as_ref()
                .map(|record| (state.version, serde_json::to_vec_pretty(record)))
        };
        if let Some((version, Ok(bytes))) = now {
            self.persist(version, &bytes);
        }
        self.unsaved.load(Ordering::SeqCst)
    }

    /// Puts a version on disk unless a newer one is already there.
    fn persist(&self, version: u64, bytes: &[u8]) {
        let mut saved = self.saved.lock().unwrap_or_else(PoisonError::into_inner);
        if version <= *saved {
            return;
        }
        match self.write(bytes) {
            Ok(()) => {
                *saved = version;
                self.unsaved.store(false, Ordering::SeqCst);
            }
            Err(error) => {
                eprintln!(
                    "[voice] the ledger could not be written, so calls are off until it can: {error}"
                );
                self.unsaved.store(true, Ordering::SeqCst);
            }
        }
    }

    /// The record on disk. No file is a ledger that has never been charged;
    /// a file that is there and cannot be understood is not.
    fn read(&self) -> Result<Record, Exhausted> {
        match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice::<Record>(&bytes)
                .ok()
                .filter(Record::sane)
                .ok_or(Exhausted::Unreadable),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Record::default()),
            Err(_) => Err(Exhausted::Unreadable),
        }
    }

    fn write(&self, bytes: &[u8]) -> io::Result<()> {
        off_the_runtime(|| {
            let parent = self.path.parent().unwrap_or(Path::new("."));
            let mut staged = tempfile::NamedTempFile::new_in(parent)?;
            staged.write_all(bytes)?;
            staged.as_file().sync_all()?;
            staged.persist(&self.path).map_err(|error| error.error)?;
            Ok(())
        })
    }
}

/// Runs a write that waits on the disk without holding a runtime worker: on
/// a multi-thread runtime the worker's other tasks move elsewhere first. A
/// current-thread runtime, or none, has nowhere to move them and just waits.
fn off_the_runtime<T>(work: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(work)
        }
        _ => work(),
    }
}

fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// What a provider charges, rounded up. These are a guard rail, not an
/// invoice, so a price that is a little high costs a caller a little of the
/// budget and a price that is low costs the owner more than they set.
///
/// (provider id, speech to text per minute, text to speech per 1,000 characters)
const PRICES: &[(&str, f64, f64)] = &[
    ("openai", 0.006, 0.03),
    ("google", 0.002, 0.03),
    ("openrouter", 0.006, 0.03),
    ("groq", 0.001, 0.022),
    ("mistral", 0.006, 0.03),
    ("xai", 0.10 / 60.0, 0.015),
    // The subscription adapter uses the owner's existing plan, never the paid API.
    ("xai-subscription", 0.0, 0.0),
    // The desk's own model runs on the desk and costs nothing.
    ("local", 0.0, 0.0),
];

/// Whisper's price, and a premium voice's, for a provider not in the table.
const UNKNOWN_PRICE: (f64, f64) = (0.006, 0.03);

fn price(provider_id: &str) -> (f64, f64) {
    PRICES
        .iter()
        .find(|(id, _, _)| *id == provider_id)
        .map_or(UNKNOWN_PRICE, |(_, stt, tts)| (*stt, *tts))
}

/// The cost of transcribing a clip of this many seconds.
/// Whether a provider's voice costs nothing: a subscription's, or the desk's own.
pub fn is_free(provider_id: &str) -> bool {
    price(provider_id) == (0.0, 0.0)
}

pub fn stt_usd(provider_id: &str, seconds: f64) -> f64 {
    price(provider_id).0 * seconds.max(0.0) / 60.0
}

/// The cost of accepting live audio. xAI prices WebSocket transcription
/// separately from batch uploads; all other providers retain their rate.
pub fn stt_live_usd(provider_id: &str, seconds: f64) -> f64 {
    if provider_id == "xai" {
        0.20 * seconds.max(0.0) / 3600.0
    } else {
        stt_usd(provider_id, seconds)
    }
}

/// The cost of speaking this many characters.
pub fn tts_usd(provider_id: &str, characters: usize) -> f64 {
    price(provider_id).1 * characters as f64 / 1000.0
}

#[cfg(test)]
mod tests {
    #[test]
    fn subscription_speech_does_not_estimate_paid_api_spend() {
        assert_eq!(super::stt_usd("xai-subscription", 20.0), 0.0);
        assert_eq!(super::stt_live_usd("xai-subscription", 20.0), 0.0);
        assert_eq!(super::tts_usd("xai-subscription", 8_000), 0.0);
        assert!(super::stt_usd("xai", 20.0) > 0.0);
        assert!(super::tts_usd("xai", 8_000) > 0.0);
    }

    #[test]
    fn the_desks_own_hearing_costs_nothing_batch_or_live() {
        assert_eq!(super::stt_usd("local", 120.0), 0.0);
        assert_eq!(super::stt_live_usd("local", 120.0), 0.0);
    }
    use super::*;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn settings(day_usd: f64, month_usd: f64) -> BudgetLimits {
        BudgetLimits::new(Some(day_usd), Some(month_usd))
    }

    /// What `budget_on` used to report: the spend of every kind, or the
    /// limits when the ledger cannot be read.
    struct Reported {
        spent_day_usd: f64,
        spent_month_usd: f64,
    }

    impl Ledger {
        /// The Voice budget's check, as the call gates make it.
        fn check_on(&self, today: NaiveDate, limits: &BudgetLimits) -> Result<(), Exhausted> {
            let spent = self.spent_on(today)?;
            left(
                BudgetKind::Voice,
                *limits,
                spent.day.budget(BudgetKind::Voice),
                spent.month.budget(BudgetKind::Voice),
            )
        }

        fn budget_on(&self, today: NaiveDate, limits: &BudgetLimits) -> Reported {
            match self.spent_on(today) {
                Ok(spent) => Reported {
                    spent_day_usd: spent.day.total(),
                    spent_month_usd: spent.month.total(),
                },
                Err(_) => Reported {
                    spent_day_usd: limits.day_usd.unwrap_or(0.0),
                    spent_month_usd: limits.month_usd.unwrap_or(0.0),
                },
            }
        }
    }

    fn ledger() -> (tempfile::TempDir, Ledger) {
        let root = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(root.path());
        (root, ledger)
    }

    #[test]
    fn a_fresh_ledger_has_spent_nothing_and_no_limit_is_never_spent() {
        let (_root, ledger) = ledger();
        let today = date(2026, 9, 30);
        assert_eq!(ledger.spent_on(today), Ok(Spent::default()));
        ledger.charge_on(today, Kind::Tts, 1_000.0);
        assert_eq!(ledger.check_on(today, &BudgetLimits::default()), Ok(()));
    }

    #[test]
    fn the_day_cap_ends_voice_the_moment_it_is_spent() {
        let (_root, ledger) = ledger();
        let today = date(2026, 9, 30);
        ledger.charge_on(today, Kind::Stt, 0.5);
        ledger.charge_on(today, Kind::Tts, 1.49);
        assert_eq!(ledger.check_on(today, &settings(2.0, 20.0)), Ok(()));
        ledger.charge_on(today, Kind::Tts, 0.01);
        assert_eq!(
            ledger.check_on(today, &settings(2.0, 20.0)),
            Err(Exhausted::Day(BudgetKind::Voice))
        );
    }

    #[test]
    fn a_new_day_forgives_the_day_but_not_the_month() {
        let (_root, ledger) = ledger();
        let limits = settings(2.0, 3.0);
        ledger.charge_on(date(2026, 9, 28), Kind::Tts, 2.0);
        assert_eq!(
            ledger.check_on(date(2026, 9, 28), &limits),
            Err(Exhausted::Day(BudgetKind::Voice))
        );
        assert_eq!(ledger.check_on(date(2026, 9, 29), &limits), Ok(()));
        ledger.charge_on(date(2026, 9, 29), Kind::Tts, 1.0);
        // Three dollars this month, and the month is the tighter statement.
        assert_eq!(
            ledger.check_on(date(2026, 9, 29), &limits),
            Err(Exhausted::Month(BudgetKind::Voice))
        );
        assert_eq!(
            ledger.check_on(date(2026, 9, 30), &limits),
            Err(Exhausted::Month(BudgetKind::Voice))
        );
    }

    #[test]
    fn a_new_month_forgives_the_month() {
        let (_root, ledger) = ledger();
        let limits = settings(2.0, 3.0);
        ledger.charge_on(date(2026, 9, 29), Kind::Tts, 1.5);
        ledger.charge_on(date(2026, 9, 30), Kind::Tts, 1.5);
        assert_eq!(
            ledger.check_on(date(2026, 9, 30), &limits),
            Err(Exhausted::Month(BudgetKind::Voice))
        );
        assert_eq!(ledger.check_on(date(2026, 10, 1), &limits), Ok(()));
        assert_eq!(ledger.spent_on(date(2026, 10, 1)), Ok(Spent::default()));
    }

    #[test]
    fn the_year_turning_is_a_new_month_too() {
        let (_root, ledger) = ledger();
        ledger.charge_on(date(2026, 12, 31), Kind::Tts, 5.0);
        assert_eq!(
            ledger.check_on(date(2026, 12, 31), &settings(100.0, 5.0)),
            Err(Exhausted::Month(BudgetKind::Voice))
        );
        assert_eq!(
            ledger.check_on(date(2027, 1, 1), &settings(100.0, 5.0)),
            Ok(())
        );
    }

    #[test]
    fn a_clock_that_goes_backwards_does_not_forgive_the_spend() {
        let (_root, ledger) = ledger();
        ledger.charge_on(date(2026, 9, 30), Kind::Tts, 2.0);
        assert_eq!(
            ledger.check_on(date(2026, 9, 1), &settings(2.0, 20.0)),
            Err(Exhausted::Day(BudgetKind::Voice))
        );
    }

    #[test]
    fn a_zero_cap_turns_the_budget_off_and_names_it() {
        let (_root, ledger) = ledger();
        let today = date(2026, 9, 30);
        assert_eq!(
            ledger.check_on(today, &BudgetLimits::new(Some(0.0), None)),
            Err(Exhausted::Off(BudgetKind::Voice))
        );
        // The call assistant's spend is Chat's, not Voice's.
        ledger.charge_on(today, Kind::Dispatcher, 5.0);
        assert_eq!(ledger.check_on(today, &settings(2.0, 20.0)), Ok(()));
        assert_eq!(Kind::Dispatcher.budget(), BudgetKind::Chat);
        assert_eq!(Kind::Stt.budget(), BudgetKind::Voice);
    }

    #[test]
    fn the_spend_survives_a_restart() {
        let root = tempfile::tempdir().unwrap();
        let today = date(2026, 9, 30);
        Ledger::open(root.path()).charge_on(today, Kind::Tts, 1.25);
        let reopened = Ledger::open(root.path());
        assert_eq!(
            reopened
                .budget_on(today, &settings(2.0, 20.0))
                .spent_day_usd,
            1.25
        );
        reopened.charge_on(today, Kind::Stt, 0.75);
        assert_eq!(
            reopened.check_on(today, &settings(2.0, 20.0)),
            Err(Exhausted::Day(BudgetKind::Voice))
        );
        assert_eq!(
            Ledger::open(root.path()).check_on(today, &settings(2.0, 20.0)),
            Err(Exhausted::Day(BudgetKind::Voice))
        );
    }

    #[test]
    fn a_file_that_cannot_be_read_fails_closed_until_it_can() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(FILE);
        let today = date(2026, 9, 30);
        for corrupt in [
            &b"not json"[..],
            b"",
            br#"{"day":"2026-09-30"}"#,
            br#"{"day":"2026-09-30","month":"2026-09","daySpend":{"stt":-1,"tts":0,"dispatcher":0},"monthSpend":{"stt":0,"tts":0,"dispatcher":0}}"#,
        ] {
            fs::write(&path, corrupt).unwrap();
            let ledger = Ledger::open(root.path());
            assert_eq!(ledger.check_on(today, &settings(2.0, 20.0)), Err(Exhausted::Unreadable));
            // A charge it cannot add to is not written over what is there.
            ledger.charge_on(today, Kind::Tts, 0.1);
            assert_eq!(fs::read(&path).unwrap(), corrupt);
            let budget = ledger.budget_on(today, &settings(2.0, 20.0));
            assert_eq!((budget.spent_day_usd, budget.spent_month_usd), (2.0, 20.0));
        }

        // Mending the file mends the ledger, without a restart.
        let ledger = Ledger::open(root.path());
        assert!(ledger.check_on(today, &settings(2.0, 20.0)).is_err());
        fs::remove_file(&path).unwrap();
        assert_eq!(ledger.check_on(today, &settings(2.0, 20.0)), Ok(()));
    }

    #[test]
    fn a_directory_where_the_file_should_be_is_unreadable_too() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join(FILE)).unwrap();
        assert_eq!(
            Ledger::open(root.path()).check_on(date(2026, 9, 30), &settings(2.0, 20.0)),
            Err(Exhausted::Unreadable)
        );
    }

    #[test]
    fn a_charge_that_cannot_be_written_down_ends_calls_until_it_can() {
        let root = tempfile::tempdir().unwrap();
        let gone = root.path().join("gone");
        let ledger = Ledger::open(&gone);
        let today = date(2026, 9, 30);
        assert_eq!(ledger.check_on(today, &settings(2.0, 20.0)), Ok(()));
        ledger.charge_on(today, Kind::Tts, 0.1);
        assert_eq!(
            ledger.check_on(today, &settings(2.0, 20.0)),
            Err(Exhausted::Unreadable)
        );
        assert_eq!(
            ledger.budget_on(today, &settings(2.0, 20.0)).spent_day_usd,
            2.0,
            "a balance that is not on disk is reported spent"
        );

        // The disk comes back and nobody charges anything: the next check
        // writes what it was holding, and voice is on again, with that spend.
        fs::create_dir(&gone).unwrap();
        assert_eq!(ledger.check_on(today, &settings(2.0, 20.0)), Ok(()));
        let saved: Record = serde_json::from_slice(&fs::read(gone.join(FILE)).unwrap()).unwrap();
        assert!((saved.day_spend.tts - 0.1).abs() < 1e-9);
        assert_eq!(
            Ledger::open(&gone)
                .budget_on(today, &settings(2.0, 20.0))
                .spent_day_usd,
            0.1
        );
    }

    #[test]
    fn a_status_read_also_tries_a_failed_write_again() {
        let root = tempfile::tempdir().unwrap();
        let gone = root.path().join("gone");
        let ledger = Ledger::open(&gone);
        let today = date(2026, 9, 30);
        ledger.charge_on(today, Kind::Stt, 0.25);
        assert_eq!(
            ledger.budget_on(today, &settings(2.0, 20.0)).spent_day_usd,
            2.0
        );
        fs::create_dir(&gone).unwrap();
        assert_eq!(
            ledger.budget_on(today, &settings(2.0, 20.0)).spent_day_usd,
            0.25
        );
    }

    #[test]
    fn a_disk_that_keeps_failing_keeps_voice_off() {
        let root = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(&root.path().join("gone"));
        let today = date(2026, 9, 30);
        ledger.charge_on(today, Kind::Tts, 0.1);
        for _ in 0..3 {
            assert_eq!(
                ledger.check_on(today, &settings(2.0, 20.0)),
                Err(Exhausted::Unreadable)
            );
        }
    }

    /// Charges land from many threads at once, and the file ends up with
    /// all of them: a write that arrives late never puts an older balance
    /// over a newer one.
    #[test]
    fn concurrent_charges_are_all_on_disk_at_the_end() {
        let root = tempfile::tempdir().unwrap();
        let ledger = std::sync::Arc::new(Ledger::open(root.path()));
        let today = date(2026, 9, 30);
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let ledger = ledger.clone();
                std::thread::spawn(move || {
                    for _ in 0..25 {
                        ledger.charge_on(today, Kind::Tts, 0.001);
                        let _ = ledger.check_on(today, &settings(100.0, 100.0));
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let saved: Record =
            serde_json::from_slice(&fs::read(root.path().join(FILE)).unwrap()).unwrap();
        assert!(
            (saved.day_spend.tts - 0.2).abs() < 1e-9,
            "{}",
            saved.day_spend.tts
        );
        assert!((saved.month_spend.tts - 0.2).abs() < 1e-9);
    }

    /// A check waits for arithmetic and never for somebody's fsync.
    #[test]
    fn a_check_does_not_wait_for_a_write_in_progress() {
        let root = tempfile::tempdir().unwrap();
        let ledger = std::sync::Arc::new(Ledger::open(root.path()));
        let today = date(2026, 9, 30);
        ledger.charge_on(today, Kind::Tts, 0.1);
        let held = ledger.saved.lock().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let asking = ledger.clone();
        std::thread::spawn(move || {
            let checked = asking.check_on(today, &settings(2.0, 20.0));
            let budget = asking.budget_on(today, &settings(2.0, 20.0));
            sender.send((checked, budget.spent_day_usd)).unwrap();
        });
        let (checked, spent) = receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("a check waited for the disk");
        drop(held);
        assert_eq!(checked, Ok(()));
        assert!((spent - 0.1).abs() < 1e-9);
    }

    /// A write is made off the runtime's worker where there is one to spare,
    /// and just made where there is not; neither panics.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_charge_from_a_runtime_worker_is_written() {
        let root = tempfile::tempdir().unwrap();
        let ledger = std::sync::Arc::new(Ledger::open(root.path()));
        let today = date(2026, 9, 30);
        let charging = ledger.clone();
        tokio::spawn(async move { charging.charge_on(today, Kind::Tts, 0.3) })
            .await
            .unwrap();
        assert_eq!(
            ledger.budget_on(today, &settings(2.0, 20.0)).spent_day_usd,
            0.3
        );
        assert!(root.path().join(FILE).exists());
    }

    #[tokio::test]
    async fn a_charge_from_a_current_thread_runtime_is_written() {
        let root = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(root.path());
        ledger.charge_on(date(2026, 9, 30), Kind::Stt, 0.3);
        assert!(root.path().join(FILE).exists());
    }

    #[test]
    fn a_charge_is_never_negative_or_not_a_number() {
        let (_root, ledger) = ledger();
        let today = date(2026, 9, 30);
        ledger.charge_on(today, Kind::Tts, -5.0);
        ledger.charge_on(today, Kind::Stt, f64::NAN);
        ledger.charge_on(today, Kind::Dispatcher, f64::INFINITY);
        assert_eq!(
            ledger.budget_on(today, &settings(2.0, 20.0)).spent_day_usd,
            0.0
        );
    }

    fn spent(ledger: &Ledger, today: NaiveDate) -> (f64, f64) {
        let budget = ledger.budget_on(today, &settings(100.0, 100.0));
        (budget.spent_day_usd, budget.spent_month_usd)
    }

    fn close(actual: (f64, f64), expected: (f64, f64)) {
        assert!(
            (actual.0 - expected.0).abs() < 1e-12 && (actual.1 - expected.1).abs() < 1e-12,
            "{actual:?} is not {expected:?}"
        );
    }

    #[test]
    fn settling_moves_a_reservation_to_the_actual_cost_both_ways() {
        let (_root, ledger) = ledger();
        let today = date(2026, 9, 30);
        ledger.charge_on(today, Kind::Tts, 0.25);
        let reservation = ledger.charge_on(today, Kind::Dispatcher, 1.0).unwrap();
        ledger.settle_on(today, reservation, 0.01);
        close(spent(&ledger, today), (0.26, 0.26));

        let reservation = ledger.charge_on(today, Kind::Dispatcher, 0.1).unwrap();
        ledger.settle_on(today, reservation, 0.3);
        close(spent(&ledger, today), (0.56, 0.56));

        // Settled from disk too, not just in memory.
        close(spent(&Ledger::open(_root.path()), today), (0.56, 0.56));
    }

    #[test]
    fn an_unknown_cost_keeps_the_reservation() {
        let (_root, ledger) = ledger();
        let today = date(2026, 9, 30);
        for unknown in [f64::NAN, f64::INFINITY, -1.0] {
            let reservation = ledger.charge_on(today, Kind::Dispatcher, 0.5).unwrap();
            ledger.settle_on(today, reservation, unknown);
        }
        // A dropped reservation is kept as well.
        drop(ledger.charge_on(today, Kind::Dispatcher, 0.5));
        close(spent(&ledger, today), (2.0, 2.0));
    }

    #[test]
    fn a_refund_never_takes_another_kind_or_the_totals_below_zero() {
        let (root, ledger) = ledger();
        let today = date(2026, 9, 30);
        ledger.charge_on(today, Kind::Tts, 0.4);
        let reservation = ledger.charge_on(today, Kind::Dispatcher, 0.5).unwrap();
        // Something else (a mended file) took the dispatcher's spend away.
        {
            let mut state = ledger.state.lock().unwrap();
            let record = state.record.as_mut().unwrap();
            record.day_spend.dispatcher = 0.1;
            record.month_spend.dispatcher = 0.1;
        }
        ledger.settle_on(today, reservation, 0.0);
        close(spent(&ledger, today), (0.4, 0.4));
        let saved: Record =
            serde_json::from_slice(&fs::read(root.path().join(FILE)).unwrap()).unwrap();
        assert_eq!(saved.day_spend.dispatcher, 0.0);
        assert!((saved.day_spend.tts - 0.4).abs() < 1e-12);
    }

    #[test]
    fn a_day_that_rolled_over_before_settling_keeps_its_estimate_and_the_month_is_refunded() {
        let (_root, ledger) = ledger();
        let (monday, tuesday) = (date(2026, 9, 28), date(2026, 9, 29));
        ledger.charge_on(monday, Kind::Tts, 0.2);
        let reservation = ledger.charge_on(monday, Kind::Dispatcher, 1.0).unwrap();
        ledger.charge_on(tuesday, Kind::Dispatcher, 0.3);
        ledger.settle_on(tuesday, reservation, 0.1);
        // Tuesday has only its own spend; the month holds Monday's actual cost.
        close(spent(&ledger, tuesday), (0.3, 0.6));

        // A cost above the estimate is charged to the day it became known.
        let reservation = ledger.charge_on(tuesday, Kind::Dispatcher, 0.1).unwrap();
        ledger.settle_on(date(2026, 9, 30), reservation, 0.4);
        close(spent(&ledger, date(2026, 9, 30)), (0.3, 1.0));
    }

    #[test]
    fn a_month_that_rolled_over_before_settling_is_not_refunded_into_the_new_one() {
        let (_root, ledger) = ledger();
        let reservation = ledger
            .charge_on(date(2026, 9, 30), Kind::Dispatcher, 1.0)
            .unwrap();
        ledger.charge_on(date(2026, 10, 1), Kind::Dispatcher, 0.25);
        ledger.settle_on(date(2026, 10, 1), reservation, 0.0);
        close(spent(&ledger, date(2026, 10, 1)), (0.25, 0.25));
    }

    #[test]
    fn a_month_refund_never_leaves_the_month_below_the_day() {
        let (_root, ledger) = ledger();
        let today = date(2026, 9, 30);
        let reservation = ledger.charge_on(today, Kind::Dispatcher, 1.0).unwrap();
        // The day says more than the month (an edited file); the refund must
        // not make that worse.
        {
            let mut state = ledger.state.lock().unwrap();
            let record = state.record.as_mut().unwrap();
            record.day_spend.dispatcher = 1.0;
            record.month_spend.dispatcher = 1.0;
        }
        let yesterdays = Reservation {
            day: "2026-09-29".into(),
            ..reservation
        };
        ledger.settle_on(today, yesterdays, 0.0);
        let (day, month) = spent(&ledger, today);
        assert!(month >= day, "{month} < {day}");
        close((day, month), (1.0, 1.0));
    }

    #[test]
    fn a_clock_that_went_backwards_still_settles_against_the_later_day() {
        let (_root, ledger) = ledger();
        let reservation = ledger
            .charge_on(date(2026, 9, 30), Kind::Dispatcher, 1.0)
            .unwrap();
        ledger.settle_on(date(2026, 9, 29), reservation, 0.2);
        close(spent(&ledger, date(2026, 9, 30)), (0.2, 0.2));
    }

    #[test]
    fn speech_is_priced_by_provider_and_a_stranger_pays_the_high_price() {
        assert!((stt_usd("groq", 30.0) - 0.0005).abs() < 1e-9);
        assert!((stt_usd("openai", 60.0) - 0.006).abs() < 1e-9);
        assert!((tts_usd("groq", 500) - 0.011).abs() < 1e-9);
        assert!((tts_usd("custom-1234", 1000) - 0.03).abs() < 1e-9);
        assert!((stt_usd("custom-1234", 60.0) - 0.006).abs() < 1e-9);
        assert_eq!(stt_usd("openai", -3.0), 0.0);
    }

    #[test]
    fn native_xai_streaming_and_batch_prices_are_distinct_and_other_rates_stay_the_same() {
        assert!((stt_usd("xai", 3600.0) - 0.10).abs() < 1e-9);
        assert!((stt_live_usd("xai", 3600.0) - 0.20).abs() < 1e-9);
        assert!((tts_usd("xai", 1_000_000) - 15.0).abs() < 1e-9);
        for provider in [
            "openai",
            "google",
            "openrouter",
            "groq",
            "mistral",
            "custom-1234",
        ] {
            assert_eq!(stt_live_usd(provider, 10.0), stt_usd(provider, 10.0));
        }
        assert_eq!(stt_live_usd("xai", -3.0), 0.0);
    }
}
