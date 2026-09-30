use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::{Datelike, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

const MAX_LEDGER_BYTES: u64 = 4096;
const STORAGE_ERROR: &str =
    "The spending ledger could not be safely read or saved. Spending is blocked.";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct SpendingSettings {
    pub day_usd: f64,
    pub month_usd: f64,
}

impl Default for SpendingSettings {
    fn default() -> Self {
        Self {
            day_usd: 2.0,
            month_usd: 20.0,
        }
    }
}

impl SpendingSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !valid_amount(self.day_usd) || !valid_amount(self.month_usd) {
            return Err("Spending limits must be finite, nonnegative dollar amounts.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct SpendingSummary {
    pub day_usd: f64,
    pub month_usd: f64,
}

#[derive(Clone, Debug)]
pub struct SpendLedger {
    inner: Arc<Ledger>,
}

#[derive(Debug)]
struct Ledger {
    root: PathBuf,
    state: Mutex<LedgerState>,
}

#[derive(Debug, Default)]
struct LedgerState {
    initialized: bool,
    failed: bool,
}

#[derive(Debug)]
pub struct Reservation {
    ledger: SpendLedger,
    period: Period,
    estimate_usd: f64,
}

#[derive(Clone, Copy, Debug)]
struct Period {
    day: i64,
    month: i32,
}

impl Period {
    fn now() -> Self {
        let now = Utc::now();
        Self {
            day: now.timestamp().div_euclid(86_400),
            month: now.year() * 12 + now.month0() as i32,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Totals {
    version: u8,
    day: i64,
    month: i32,
    day_usd: f64,
    month_usd: f64,
}

impl Totals {
    fn new(period: Period) -> Self {
        Self {
            version: 1,
            day: period.day,
            month: period.month,
            day_usd: 0.0,
            month_usd: 0.0,
        }
    }

    fn validate(&self, now: Period) -> Result<(), String> {
        let date = self
            .day
            .checked_mul(86_400)
            .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
            .ok_or(STORAGE_ERROR)?;
        if self.version != 1
            || self.day > now.day
            || self.month != date.year() * 12 + date.month0() as i32
            || !valid_amount(self.day_usd)
            || !valid_amount(self.month_usd)
            || self.day_usd > self.month_usd
        {
            return Err(STORAGE_ERROR.into());
        }
        Ok(())
    }

    fn advance(&mut self, period: Period) {
        if self.day != period.day {
            self.day = period.day;
            self.day_usd = 0.0;
        }
        if self.month != period.month {
            self.month = period.month;
            self.month_usd = 0.0;
        }
    }
}

impl SpendLedger {
    pub fn new(root: PathBuf) -> Self {
        Self {
            inner: Arc::new(Ledger {
                root,
                state: Mutex::new(LedgerState::default()),
            }),
        }
    }

    pub fn reserve(
        &self,
        settings: &SpendingSettings,
        estimate_usd: f64,
    ) -> Result<Reservation, String> {
        settings.validate()?;
        if !valid_amount(estimate_usd) {
            return Err("The estimated cost must be a finite, nonnegative dollar amount.".into());
        }
        if settings.day_usd == 0.0 || settings.month_usd == 0.0 {
            return Err("Spending is disabled because a spending limit is zero.".into());
        }
        let mut state = self.inner.lock()?;
        let period = Period::now();
        let mut totals = self.inner.load(&mut state, period)?;
        totals.advance(period);
        let day_usd = totals.day_usd + estimate_usd;
        let month_usd = totals.month_usd + estimate_usd;
        if !valid_amount(day_usd) || day_usd > settings.day_usd {
            return Err("The daily spending limit would be exceeded.".into());
        }
        if !valid_amount(month_usd) || month_usd > settings.month_usd {
            return Err("The monthly spending limit would be exceeded.".into());
        }
        if estimate_usd > 0.0 && (day_usd == totals.day_usd || month_usd == totals.month_usd) {
            return Err(
                "The estimated cost is too small to safely add to the spending totals.".into(),
            );
        }
        totals.day_usd = day_usd;
        totals.month_usd = month_usd;
        self.inner.save(&mut state, &totals)?;
        Ok(Reservation {
            ledger: self.clone(),
            period,
            estimate_usd,
        })
    }

    pub fn summary(&self) -> Result<SpendingSummary, String> {
        let mut state = self.inner.lock()?;
        let period = Period::now();
        let mut totals = self.inner.load(&mut state, period)?;
        totals.advance(period);
        Ok(SpendingSummary {
            day_usd: totals.day_usd,
            month_usd: totals.month_usd,
        })
    }
}

impl Reservation {
    pub fn charge(self, actual_usd: f64) -> Result<(), String> {
        if !valid_amount(actual_usd) {
            return Err("The actual cost must be a finite, nonnegative dollar amount.".into());
        }
        let mut state = self.ledger.inner.lock()?;
        let mut totals = self.ledger.inner.load(&mut state, Period::now())?;
        if totals.day == self.period.day {
            totals.day_usd = totals.day_usd - self.estimate_usd + actual_usd;
        }
        if totals.month == self.period.month {
            totals.month_usd = totals.month_usd - self.estimate_usd + actual_usd;
            totals.month_usd = totals.month_usd.max(totals.day_usd);
        }
        self.ledger.inner.save(&mut state, &totals)
    }
}

impl Ledger {
    fn lock(&self) -> Result<MutexGuard<'_, LedgerState>, String> {
        let state = self.state.lock().map_err(|_| STORAGE_ERROR.to_string())?;
        if state.failed {
            return Err(STORAGE_ERROR.into());
        }
        Ok(state)
    }

    fn load(&self, state: &mut LedgerState, period: Period) -> Result<Totals, String> {
        let result = (|| {
            if !fs::metadata(&self.root)
                .map_err(|_| STORAGE_ERROR)?
                .is_dir()
            {
                return Err(STORAGE_ERROR.into());
            }
            let path = self.root.join("spending.json");
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound && !state.initialized =>
                {
                    return Ok(Totals::new(period));
                }
                Err(_) => return Err(STORAGE_ERROR.into()),
            };
            if !metadata.is_file() || metadata.len() > MAX_LEDGER_BYTES {
                return Err(STORAGE_ERROR.into());
            }
            let mut bytes = Vec::new();
            File::open(path)
                .map_err(|_| STORAGE_ERROR)?
                .take(MAX_LEDGER_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| STORAGE_ERROR)?;
            if bytes.len() as u64 > MAX_LEDGER_BYTES {
                return Err(STORAGE_ERROR.into());
            }
            let totals: Totals = serde_json::from_slice(&bytes).map_err(|_| STORAGE_ERROR)?;
            totals.validate(period)?;
            state.initialized = true;
            Ok(totals)
        })();
        if result.is_err() {
            state.failed = true;
        }
        result
    }

    fn save(&self, state: &mut LedgerState, totals: &Totals) -> Result<(), String> {
        let result = (|| {
            totals.validate(Period::now())?;
            let bytes = serde_json::to_vec(totals).map_err(|_| STORAGE_ERROR)?;
            if bytes.len() as u64 > MAX_LEDGER_BYTES {
                return Err(STORAGE_ERROR.into());
            }
            let mut temporary =
                tempfile::NamedTempFile::new_in(&self.root).map_err(|_| STORAGE_ERROR)?;
            temporary.write_all(&bytes).map_err(|_| STORAGE_ERROR)?;
            temporary.as_file().sync_all().map_err(|_| STORAGE_ERROR)?;
            temporary
                .persist(self.root.join("spending.json"))
                .map_err(|_| STORAGE_ERROR)?;
            #[cfg(unix)]
            File::open(&self.root)
                .and_then(|directory| directory.sync_all())
                .map_err(|_| STORAGE_ERROR)?;
            Ok(())
        })();
        if result.is_err() {
            state.failed = true;
        } else {
            state.initialized = true;
        }
        result
    }
}

fn valid_amount(amount: f64) -> bool {
    amount.is_finite() && amount >= 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_totals(root: &std::path::Path) -> Totals {
        serde_json::from_slice(&fs::read(root.join("spending.json")).unwrap()).unwrap()
    }

    #[test]
    fn defaults_and_contract_use_dollar_limits() {
        let settings = SpendingSettings::default();
        assert_eq!(settings.day_usd, 2.0);
        assert_eq!(settings.month_usd, 20.0);
        assert!(settings.validate().is_ok());
        assert_eq!(
            serde_json::from_str::<SpendingSettings>("{}").unwrap(),
            settings
        );
        let partial: SpendingSettings =
            serde_json::from_value(serde_json::json!({"dayUsd": 0.0})).unwrap();
        assert_eq!(partial.day_usd, 0.0);
        assert_eq!(partial.month_usd, 20.0);
        let value = serde_json::json!({"dayUsd": 2.0, "monthUsd": 20.0});
        assert_eq!(serde_json::to_value(&settings).unwrap(), value);
        assert_eq!(
            serde_json::from_value::<SpendingSettings>(value).unwrap(),
            settings
        );
        let declaration = SpendingSettings::decl(&ts_rs::Config::default());
        assert!(declaration.contains("dayUsd: number"));
        assert!(declaration.contains("monthUsd: number"));
    }

    #[test]
    fn either_zero_limit_disables_spending_without_io() {
        let root = tempfile::tempdir().unwrap();
        let ledger = SpendLedger::new(root.path().join("absent"));
        for settings in [
            SpendingSettings {
                day_usd: 0.0,
                month_usd: 20.0,
            },
            SpendingSettings {
                day_usd: 2.0,
                month_usd: 0.0,
            },
        ] {
            assert!(settings.validate().is_ok());
            assert!(
                ledger
                    .reserve(&settings, 0.0)
                    .unwrap_err()
                    .contains("disabled")
            );
        }
        assert!(!root.path().join("absent").exists());
    }

    #[test]
    fn invalid_limits_and_costs_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let ledger = SpendLedger::new(root.path().to_path_buf());
        for amount in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            for settings in [
                SpendingSettings {
                    day_usd: amount,
                    month_usd: 20.0,
                },
                SpendingSettings {
                    day_usd: 2.0,
                    month_usd: amount,
                },
            ] {
                assert!(settings.validate().is_err());
                assert!(ledger.reserve(&settings, 0.5).is_err());
            }
            assert!(
                ledger
                    .reserve(&SpendingSettings::default(), amount)
                    .is_err()
            );
            let reservation = ledger.reserve(&SpendingSettings::default(), 0.25).unwrap();
            assert!(reservation.charge(amount).is_err());
        }
        assert_eq!(read_totals(root.path()).day_usd, 1.0);
    }

    #[test]
    fn cancelled_attempts_stay_reserved_and_refusal_is_repeatable() {
        let root = tempfile::tempdir().unwrap();
        let ledger = SpendLedger::new(root.path().to_path_buf());
        let settings = SpendingSettings::default();
        drop(ledger.reserve(&settings, 1.5).unwrap());
        let saved = fs::read(root.path().join("spending.json")).unwrap();
        for _ in 0..3 {
            assert!(
                ledger
                    .reserve(&settings, 0.75)
                    .unwrap_err()
                    .contains("daily")
            );
            assert_eq!(fs::read(root.path().join("spending.json")).unwrap(), saved);
        }
        drop(ledger.reserve(&settings, 0.5).unwrap());
        assert!(ledger.reserve(&settings, 0.01).is_err());
    }

    #[test]
    fn monthly_limit_is_independent_of_daily_limit() {
        let root = tempfile::tempdir().unwrap();
        let ledger = SpendLedger::new(root.path().to_path_buf());
        let settings = SpendingSettings {
            day_usd: 10.0,
            month_usd: 1.0,
        };
        ledger
            .reserve(&settings, 0.75)
            .unwrap()
            .charge(0.75)
            .unwrap();
        for _ in 0..3 {
            assert!(
                ledger
                    .reserve(&settings, 0.5)
                    .unwrap_err()
                    .contains("monthly")
            );
        }
    }

    #[test]
    fn concurrent_reservations_and_charges_share_one_gate() {
        let root = tempfile::tempdir().unwrap();
        let ledger = SpendLedger::new(root.path().to_path_buf());
        let settings = SpendingSettings::default();
        let barrier = Arc::new(std::sync::Barrier::new(16));
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let ledger = ledger.clone();
                let settings = settings.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    match ledger.reserve(&settings, 0.5) {
                        Ok(reservation) => {
                            reservation.charge(0.5).unwrap();
                            true
                        }
                        Err(_) => false,
                    }
                })
            })
            .collect();
        let admitted = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(|admitted| *admitted)
            .count();
        assert_eq!(admitted, 4);
        assert_eq!(read_totals(root.path()).day_usd, 2.0);
    }

    #[test]
    fn reservations_survive_restart_and_hold_no_lock() {
        let root = tempfile::tempdir().unwrap();
        let ledger = SpendLedger::new(root.path().to_path_buf());
        let reservation = ledger.reserve(&SpendingSettings::default(), 1.5).unwrap();
        assert!(ledger.inner.state.try_lock().is_ok());
        drop(reservation);
        drop(ledger);
        let ledger = SpendLedger::new(root.path().to_path_buf());
        assert!(ledger.reserve(&SpendingSettings::default(), 0.75).is_err());
        assert_eq!(read_totals(root.path()).month_usd, 1.5);
    }

    #[test]
    fn actual_cost_replaces_the_estimate_and_overage_is_recorded() {
        let root = tempfile::tempdir().unwrap();
        let ledger = SpendLedger::new(root.path().to_path_buf());
        let settings = SpendingSettings::default();
        ledger.reserve(&settings, 1.5).unwrap().charge(0.5).unwrap();
        assert_eq!(read_totals(root.path()).day_usd, 0.5);
        ledger.reserve(&settings, 1.5).unwrap().charge(3.0).unwrap();
        assert_eq!(read_totals(root.path()).day_usd, 3.5);
        assert_eq!(read_totals(root.path()).month_usd, 3.5);
        assert!(ledger.reserve(&settings, 0.0).is_err());
        let restarted = SpendLedger::new(root.path().to_path_buf());
        assert!(restarted.reserve(&settings, 0.01).is_err());
    }

    #[test]
    fn zero_actual_cost_releases_only_its_own_estimate() {
        let root = tempfile::tempdir().unwrap();
        let ledger = SpendLedger::new(root.path().to_path_buf());
        let reservation = ledger.reserve(&SpendingSettings::default(), 0.75).unwrap();
        drop(ledger.reserve(&SpendingSettings::default(), 0.5).unwrap());
        reservation.charge(0.0).unwrap();
        assert_eq!(read_totals(root.path()).day_usd, 0.5);
    }

    #[test]
    fn corruption_and_oversized_files_never_become_empty_ledgers() {
        for bytes in [
            b"not json with a private value".to_vec(),
            b"{}".to_vec(),
            vec![b' '; MAX_LEDGER_BYTES as usize + 1],
        ] {
            let root = tempfile::tempdir().unwrap();
            fs::write(root.path().join("spending.json"), &bytes).unwrap();
            let ledger = SpendLedger::new(root.path().to_path_buf());
            assert_eq!(
                ledger
                    .reserve(&SpendingSettings::default(), 0.5)
                    .unwrap_err(),
                STORAGE_ERROR
            );
            assert_eq!(fs::read(root.path().join("spending.json")).unwrap(), bytes);
            fs::remove_file(root.path().join("spending.json")).unwrap();
            assert!(ledger.reserve(&SpendingSettings::default(), 0.5).is_err());
        }
    }

    #[test]
    fn unreadable_or_missing_existing_ledger_blocks_spending() {
        let root = tempfile::tempdir().unwrap();
        let ledger = SpendLedger::new(root.path().to_path_buf());
        drop(ledger.reserve(&SpendingSettings::default(), 0.5).unwrap());
        fs::remove_file(root.path().join("spending.json")).unwrap();
        assert!(ledger.reserve(&SpendingSettings::default(), 0.5).is_err());
        fs::create_dir(root.path().join("spending.json")).unwrap();
        let restarted = SpendLedger::new(root.path().to_path_buf());
        assert!(
            restarted
                .reserve(&SpendingSettings::default(), 0.5)
                .is_err()
        );
    }

    #[test]
    fn write_failure_blocks_later_spending_even_if_storage_recovers() {
        let root = tempfile::tempdir().unwrap();
        let ledger = SpendLedger::new(root.path().to_path_buf());
        let reservation = ledger.reserve(&SpendingSettings::default(), 0.5).unwrap();
        let totals = read_totals(root.path());
        fs::remove_file(root.path().join("spending.json")).unwrap();
        fs::create_dir(root.path().join("spending.json")).unwrap();
        let mut state = ledger.inner.lock().unwrap();
        assert_eq!(
            ledger.inner.save(&mut state, &totals).unwrap_err(),
            STORAGE_ERROR
        );
        drop(state);
        fs::remove_dir(root.path().join("spending.json")).unwrap();
        fs::write(
            root.path().join("spending.json"),
            serde_json::to_vec(&totals).unwrap(),
        )
        .unwrap();
        assert!(reservation.charge(3.0).is_err());
        assert!(ledger.reserve(&SpendingSettings::default(), 0.5).is_err());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn late_charge_does_not_subtract_from_a_new_day_or_month() {
        for days_ago in [1, 40] {
            let root = tempfile::tempdir().unwrap();
            let ledger = SpendLedger::new(root.path().to_path_buf());
            let past = Utc::now() - chrono::Duration::days(days_ago);
            let period = Period {
                day: past.timestamp().div_euclid(86_400),
                month: past.year() * 12 + past.month0() as i32,
            };
            let mut totals = Totals::new(period);
            totals.day_usd = 0.75;
            totals.month_usd = 0.75;
            fs::write(
                root.path().join("spending.json"),
                serde_json::to_vec(&totals).unwrap(),
            )
            .unwrap();
            let reservation = Reservation {
                ledger: ledger.clone(),
                period,
                estimate_usd: 0.75,
            };
            let same_month = period.month == Period::now().month;
            assert_eq!(ledger.summary().unwrap().day_usd, 0.0);
            assert_eq!(read_totals(root.path()).day, period.day);
            drop(ledger.reserve(&SpendingSettings::default(), 0.5).unwrap());
            reservation.charge(0.25).unwrap();
            let current = read_totals(root.path());
            assert_eq!(current.day, Period::now().day);
            assert_eq!(current.month, Period::now().month);
            assert_eq!(current.day_usd, 0.5);
            assert_eq!(current.month_usd, if same_month { 0.75 } else { 0.5 });
        }
    }

    #[test]
    fn invalid_totals_and_a_clock_rollback_fail_closed() {
        let period = Period::now();
        let mut totals = Totals::new(period);
        totals.day_usd = 0.5;
        totals.month_usd = 1.0;
        for (field, value) in [
            ("version", serde_json::json!(2)),
            ("day", serde_json::json!(period.day + 1)),
            ("month", serde_json::json!(period.month + 1)),
            ("dayUsd", serde_json::json!(-1.0)),
            ("monthUsd", serde_json::json!(0.0)),
        ] {
            let root = tempfile::tempdir().unwrap();
            let mut value_totals = serde_json::to_value(&totals).unwrap();
            value_totals[field] = value;
            fs::write(
                root.path().join("spending.json"),
                serde_json::to_vec(&value_totals).unwrap(),
            )
            .unwrap();
            let ledger = SpendLedger::new(root.path().to_path_buf());
            assert_eq!(ledger.summary().unwrap_err(), STORAGE_ERROR);
            assert!(ledger.reserve(&SpendingSettings::default(), 0.5).is_err());
        }
        assert!(
            totals
                .validate(Period {
                    day: period.day - 1,
                    month: period.month,
                })
                .is_err()
        );
    }

    #[test]
    fn summary_counts_reservations_and_actuals_without_creating_a_file() {
        let root = tempfile::tempdir().unwrap();
        let ledger = SpendLedger::new(root.path().to_path_buf());
        assert_eq!(
            ledger.summary().unwrap(),
            SpendingSummary {
                day_usd: 0.0,
                month_usd: 0.0
            }
        );
        assert!(!root.path().join("spending.json").exists());
        drop(ledger.reserve(&SpendingSettings::default(), 0.5).unwrap());
        ledger
            .reserve(&SpendingSettings::default(), 0.75)
            .unwrap()
            .charge(0.25)
            .unwrap();
        assert_eq!(
            ledger.summary().unwrap(),
            SpendingSummary {
                day_usd: 0.75,
                month_usd: 0.75
            }
        );
    }
}
