//! What voice has cost today and this month, kept on disk so a restart does
//! not forgive it. For a local desk it is a guard against your own bill; for a
//! server desk it is required.
//!
//! It fails closed. A file that cannot be read, or a charge that cannot be
//! written down, ends calls until it can: an unknown balance is not a
//! balance of zero. Every check tries a write that failed again, so a disk
//! that comes back turns voice back on without a restart.

use crate::voice::settings::VoiceSettings;
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

/// Why voice may not run. Each has a sentence for the dispatcher to say
/// before it hangs up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exhausted {
    Day,
    Month,
    /// The ledger cannot be read or written, so the balance is not known.
    Unreadable,
}

impl fmt::Display for Exhausted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Exhausted::Day => write!(f, "Today's voice budget is spent. Chat carries on by text."),
            Exhausted::Month => {
                write!(
                    f,
                    "This month's voice budget is spent. Chat carries on by text."
                )
            }
            Exhausted::Unreadable => write!(
                f,
                "Voice can't check its budget, so calls are off. Chat carries on by text."
            ),
        }
    }
}

impl std::error::Error for Exhausted {}

/// The caps and what is spent against them, as `voice.status` reports.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Budget {
    pub day_usd: f64,
    pub month_usd: f64,
    pub spent_day_usd: f64,
    pub spent_month_usd: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Spend {
    stt: f64,
    tts: f64,
    dispatcher: f64,
}

impl Spend {
    fn total(&self) -> f64 {
        self.stt + self.tts + self.dispatcher
    }

    fn add(&mut self, kind: Kind, usd: f64) {
        match kind {
            Kind::Stt => self.stt += usd,
            Kind::Tts => self.tts += usd,
            Kind::Dispatcher => self.dispatcher += usd,
        }
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

    /// Whether voice may run: `Err` once today's or this month's cap is spent.
    /// A cap of zero turns paid voice off, not voice: nothing spent is not
    /// a spent cap, so a subscription's free voice still runs, and
    /// [`super::metering::Budget::reserve`] refuses anything that costs.
    pub fn check(&self, settings: &VoiceSettings) -> Result<(), Exhausted> {
        self.check_on(today(), settings)
    }

    /// Writes down a cost.
    pub fn charge(&self, kind: Kind, usd: f64) {
        self.charge_on(today(), kind, usd);
    }

    /// The caps and the spend, for `voice.status`. A ledger that cannot be
    /// read reports everything spent.
    pub fn budget(&self, settings: &VoiceSettings) -> Budget {
        self.budget_on(today(), settings)
    }

    fn check_on(&self, today: NaiveDate, settings: &VoiceSettings) -> Result<(), Exhausted> {
        if self.still_unsaved() {
            return Err(Exhausted::Unreadable);
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        self.load(&mut state)?;
        let Some(record) = state.record.as_mut() else {
            return Err(Exhausted::Unreadable);
        };
        record.roll_to(today);
        let spent = |total: f64, cap: f64| total > 0.0 && total >= cap;
        if spent(record.month_spend.total(), settings.month_usd) {
            Err(Exhausted::Month)
        } else if spent(record.day_spend.total(), settings.day_usd) {
            Err(Exhausted::Day)
        } else {
            Ok(())
        }
    }

    fn charge_on(&self, today: NaiveDate, kind: Kind, usd: f64) {
        let usd = if usd.is_finite() { usd.max(0.0) } else { 0.0 };
        let (version, bytes) = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if self.load(&mut state).is_err() {
                eprintln!(
                    "[voice] the ledger could not be read, so ${usd:.4} for {kind:?} was not recorded"
                );
                return;
            }
            let Some(record) = state.record.as_mut() else {
                return;
            };
            record.roll_to(today);
            record.day_spend.add(kind, usd);
            record.month_spend.add(kind, usd);
            let bytes = serde_json::to_vec_pretty(record);
            state.version += 1;
            (state.version, bytes)
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
    }

    fn budget_on(&self, today: NaiveDate, settings: &VoiceSettings) -> Budget {
        let known = if self.still_unsaved() {
            None
        } else {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            self.load(&mut state).ok();
            state.record.as_mut().map(|record| {
                record.roll_to(today);
                (record.day_spend.total(), record.month_spend.total())
            })
        };
        let (spent_day_usd, spent_month_usd) =
            known.unwrap_or((settings.day_usd, settings.month_usd));
        Budget {
            day_usd: settings.day_usd,
            month_usd: settings.month_usd,
            spent_day_usd,
            spent_month_usd,
        }
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

    fn settings(day_usd: f64, month_usd: f64) -> VoiceSettings {
        VoiceSettings {
            day_usd,
            month_usd,
            ..VoiceSettings::default()
        }
    }

    fn ledger() -> (tempfile::TempDir, Ledger) {
        let root = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(root.path());
        (root, ledger)
    }

    #[test]
    fn a_fresh_ledger_allows_voice_and_the_defaults_are_two_and_twenty() {
        let (_root, ledger) = ledger();
        let defaults = VoiceSettings::default();
        assert_eq!((defaults.day_usd, defaults.month_usd), (2.0, 20.0));
        assert_eq!(ledger.check_on(date(2026, 9, 30), &defaults), Ok(()));
    }

    #[test]
    fn the_day_cap_ends_voice_the_moment_it_is_spent() {
        let (_root, ledger) = ledger();
        let today = date(2026, 9, 30);
        ledger.charge_on(today, Kind::Stt, 0.5);
        ledger.charge_on(today, Kind::Tts, 1.49);
        assert_eq!(ledger.check_on(today, &settings(2.0, 20.0)), Ok(()));
        ledger.charge_on(today, Kind::Dispatcher, 0.01);
        assert_eq!(
            ledger.check_on(today, &settings(2.0, 20.0)),
            Err(Exhausted::Day)
        );
    }

    #[test]
    fn a_new_day_forgives_the_day_but_not_the_month() {
        let (_root, ledger) = ledger();
        let limits = settings(2.0, 3.0);
        ledger.charge_on(date(2026, 9, 28), Kind::Tts, 2.0);
        assert_eq!(
            ledger.check_on(date(2026, 9, 28), &limits),
            Err(Exhausted::Day)
        );
        assert_eq!(ledger.check_on(date(2026, 9, 29), &limits), Ok(()));
        ledger.charge_on(date(2026, 9, 29), Kind::Tts, 1.0);
        // Three dollars this month, and the month is the tighter statement.
        assert_eq!(
            ledger.check_on(date(2026, 9, 29), &limits),
            Err(Exhausted::Month)
        );
        assert_eq!(
            ledger.check_on(date(2026, 9, 30), &limits),
            Err(Exhausted::Month)
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
            Err(Exhausted::Month)
        );
        assert_eq!(ledger.check_on(date(2026, 10, 1), &limits), Ok(()));
        assert_eq!(
            ledger.budget_on(date(2026, 10, 1), &limits),
            Budget {
                day_usd: 2.0,
                month_usd: 3.0,
                spent_day_usd: 0.0,
                spent_month_usd: 0.0
            }
        );
    }

    #[test]
    fn the_year_turning_is_a_new_month_too() {
        let (_root, ledger) = ledger();
        ledger.charge_on(date(2026, 12, 31), Kind::Tts, 5.0);
        assert_eq!(
            ledger.check_on(date(2026, 12, 31), &settings(100.0, 5.0)),
            Err(Exhausted::Month)
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
            Err(Exhausted::Day)
        );
    }

    #[test]
    fn a_zero_cap_turns_paid_voice_off_not_free_voice() {
        let (_root, ledger) = ledger();
        let today = date(2026, 9, 30);
        // Nothing spent: a subscription's free voice still runs.
        assert_eq!(ledger.check_on(today, &settings(0.0, 20.0)), Ok(()));
        assert_eq!(ledger.check_on(today, &settings(0.0, 0.0)), Ok(()));
        // Anything spent against a zero cap has spent it.
        ledger.charge_on(today, Kind::Tts, 0.01);
        assert_eq!(
            ledger.check_on(today, &settings(0.0, 20.0)),
            Err(Exhausted::Day)
        );
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
            Err(Exhausted::Day)
        );
        assert_eq!(
            Ledger::open(root.path()).check_on(today, &settings(2.0, 20.0)),
            Err(Exhausted::Day)
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

    #[test]
    fn the_status_budget_serialises_as_the_wire_names_it() {
        let (_root, ledger) = ledger();
        let today = date(2026, 9, 30);
        ledger.charge_on(today, Kind::Tts, 0.25);
        assert_eq!(
            serde_json::to_value(ledger.budget_on(today, &settings(2.0, 20.0))).unwrap(),
            serde_json::json!({"dayUsd": 2.0, "monthUsd": 20.0, "spentDayUsd": 0.25, "spentMonthUsd": 0.25})
        );
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
