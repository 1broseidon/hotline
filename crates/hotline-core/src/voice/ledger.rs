//! What voice has cost today and this month, kept on disk so a restart does
//! not forgive it. For a local desk it is a guard against your own bill; for a
//! server desk it is required.
//!
//! It fails closed. A file that cannot be read, or a charge that cannot be
//! written down, ends calls until it can: an unknown balance is not a
//! balance of zero.

use crate::voice::settings::VoiceSettings;
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
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
    /// The last write failed, so what is in memory is not what is on disk.
    unsaved: bool,
}

pub struct Ledger {
    path: PathBuf,
    state: Mutex<State>,
}

impl Ledger {
    /// The ledger under a desk's data directory. Nothing is read until it is asked.
    pub fn open(data_root: &Path) -> Ledger {
        Ledger {
            path: data_root.join(FILE),
            state: Mutex::new(State {
                record: None,
                unsaved: false,
            }),
        }
    }

    /// Whether voice may run: `Err` once today's or this month's cap is spent.
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
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.unsaved {
            return Err(Exhausted::Unreadable);
        }
        let record = self.record(&mut state)?;
        record.roll_to(today);
        if record.month_spend.total() >= settings.month_usd {
            Err(Exhausted::Month)
        } else if record.day_spend.total() >= settings.day_usd {
            Err(Exhausted::Day)
        } else {
            Ok(())
        }
    }

    fn charge_on(&self, today: NaiveDate, kind: Kind, usd: f64) {
        let usd = if usd.is_finite() { usd.max(0.0) } else { 0.0 };
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let Ok(record) = self.record(&mut state) else {
            eprintln!(
                "[voice] the ledger could not be read, so ${usd:.4} for {kind:?} was not recorded"
            );
            return;
        };
        record.roll_to(today);
        record.day_spend.add(kind, usd);
        record.month_spend.add(kind, usd);
        let saved = serde_json::to_vec_pretty(record)
            .map_err(io::Error::other)
            .and_then(|bytes| self.write(&bytes));
        if let Err(error) = &saved {
            eprintln!(
                "[voice] the ledger could not be written, so calls are off until it can: {error}"
            );
        }
        state.unsaved = saved.is_err();
    }

    fn budget_on(&self, today: NaiveDate, settings: &VoiceSettings) -> Budget {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let known = if state.unsaved {
            None
        } else {
            self.record(&mut state).ok().map(|record| {
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

    fn record<'a>(&self, state: &'a mut State) -> Result<&'a mut Record, Exhausted> {
        if state.record.is_none() {
            state.record = Some(self.read()?);
        }
        state.record.as_mut().ok_or(Exhausted::Unreadable)
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
        let parent = self.path.parent().unwrap_or(Path::new("."));
        let mut staged = tempfile::NamedTempFile::new_in(parent)?;
        staged.write_all(bytes)?;
        staged.as_file().sync_all()?;
        staged.persist(&self.path).map_err(|error| error.error)?;
        Ok(())
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
];

/// Whisper's price, and a premium voice's, for a provider not in the table.
const UNKNOWN_PRICE: (f64, f64) = (0.006, 0.03);

/// The dispatcher's price when the catalogue has none: dollars per million tokens.
const ESTIMATE_INPUT_PER_MILLION: f64 = 5.0;
const ESTIMATE_OUTPUT_PER_MILLION: f64 = 25.0;

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

/// The cost of speaking this many characters.
pub fn tts_usd(provider_id: &str, characters: usize) -> f64 {
    price(provider_id).1 * characters as f64 / 1000.0
}

/// The cost of one dispatcher call: the model catalogue's price if it has
/// one for exactly this provider and model, else a conservative estimate.
pub fn dispatcher_usd(
    provider_id: &str,
    model_id: &str,
    input_tokens: u64,
    output_tokens: u64,
) -> f64 {
    let (input, output) = crate::models::catalog()
        .providers
        .get(provider_id)
        .and_then(|provider| provider.models.get(model_id))
        .and_then(|model| model.cost.as_ref())
        .map_or(
            (ESTIMATE_INPUT_PER_MILLION, ESTIMATE_OUTPUT_PER_MILLION),
            |cost| (cost.input, cost.output),
        );
    (input * input_tokens as f64 + output * output_tokens as f64) / 1_000_000.0
}

#[cfg(test)]
mod tests {
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
    fn a_zero_cap_is_voice_off() {
        let (_root, ledger) = ledger();
        assert_eq!(
            ledger.check_on(date(2026, 9, 30), &settings(0.0, 20.0)),
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
    fn a_charge_that_cannot_be_written_down_ends_calls() {
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

        // Once it can be written again, the next charge writes all of it.
        fs::create_dir(&gone).unwrap();
        ledger.charge_on(today, Kind::Tts, 0.1);
        assert_eq!(ledger.check_on(today, &settings(2.0, 20.0)), Ok(()));
        let saved: Record = serde_json::from_slice(&fs::read(gone.join(FILE)).unwrap()).unwrap();
        assert!((saved.day_spend.tts - 0.2).abs() < 1e-9);
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
    fn the_dispatcher_costs_what_the_catalogue_says_or_the_estimate() {
        let (provider_id, model_id, cost) = crate::models::catalog()
            .providers
            .iter()
            .find_map(|(provider_id, provider)| {
                provider.models.iter().find_map(|(model_id, model)| {
                    model
                        .cost
                        .as_ref()
                        .filter(|cost| cost.input > 0.0)
                        .map(|cost| (provider_id.clone(), model_id.clone(), cost.clone()))
                })
            })
            .expect("the catalogue prices at least one model");
        let expected = (cost.input * 4000.0 + cost.output * 100.0) / 1e6;
        assert!((dispatcher_usd(&provider_id, &model_id, 4000, 100) - expected).abs() < 1e-12);

        let estimate = (5.0 * 4000.0 + 25.0 * 100.0) / 1e6;
        assert!((dispatcher_usd("custom-1234", "any", 4000, 100) - estimate).abs() < 1e-12);
        assert!(
            (dispatcher_usd(&provider_id, "no-such-model", 4000, 100) - estimate).abs() < 1e-12
        );
    }
}
