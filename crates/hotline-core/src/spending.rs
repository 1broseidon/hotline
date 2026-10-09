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

/// One budget's limits. An absent limit is no limit; zero turns that
/// budget's paid use off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct BudgetLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub month_usd: Option<f64>,
}

impl BudgetLimits {
    pub fn new(day_usd: Option<f64>, month_usd: Option<f64>) -> Self {
        Self { day_usd, month_usd }
    }

    pub fn validate(&self) -> Result<(), String> {
        if [self.day_usd, self.month_usd]
            .into_iter()
            .flatten()
            .all(valid_amount)
        {
            Ok(())
        } else {
            Err("Spending limits must be finite, nonnegative dollar amounts.".into())
        }
    }

    /// Whether either limit is zero, which turns the budget's paid use off.
    pub fn off(&self) -> bool {
        self.day_usd == Some(0.0) || self.month_usd == Some(0.0)
    }
}

/// The room's `spending` setting: a budget each for chat (teammates and the
/// call assistant), voice (transcription and speech) and images, each with
/// optional daily and monthly limits. A room that set none has no limits.
///
/// ```json
/// {"chat": {"dayUsd": 5, "monthUsd": 50}, "voice": {"dayUsd": 10}, "images": {}}
/// ```
///
/// The shared limits earlier versions wrote, `{"dayUsd": 10, "monthUsd": 20}`,
/// still read: as the voice and the images limits, with chat unlimited,
/// since those were the two they covered. Writes are always the new shape.
#[derive(Clone, Debug, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct SpendingSettings {
    pub chat: BudgetLimits,
    pub voice: BudgetLimits,
    pub images: BudgetLimits,
}

impl<'de> Deserialize<'de> for SpendingSettings {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Stored {
            #[serde(default)]
            chat: Option<BudgetLimits>,
            #[serde(default)]
            voice: Option<BudgetLimits>,
            #[serde(default)]
            images: Option<BudgetLimits>,
            #[serde(default)]
            day_usd: Option<f64>,
            #[serde(default)]
            month_usd: Option<f64>,
        }
        let stored = Stored::deserialize(deserializer)?;
        let budgets = stored.chat.is_some() || stored.voice.is_some() || stored.images.is_some();
        let shared = stored.day_usd.is_some() || stored.month_usd.is_some();
        if budgets && shared {
            return Err(serde::de::Error::custom(
                "spending mixes shared limits with per-budget ones",
            ));
        }
        if shared {
            let limits = BudgetLimits::new(stored.day_usd, stored.month_usd);
            return Ok(Self {
                chat: BudgetLimits::default(),
                voice: limits,
                images: limits,
            });
        }
        Ok(Self {
            chat: stored.chat.unwrap_or_default(),
            voice: stored.voice.unwrap_or_default(),
            images: stored.images.unwrap_or_default(),
        })
    }
}

impl SpendingSettings {
    pub fn validate(&self) -> Result<(), String> {
        self.chat.validate()?;
        self.voice.validate()?;
        self.images.validate()
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
    pending: Vec<Settlement>,
}

#[derive(Debug)]
pub struct Reservation {
    ledger: SpendLedger,
    period: Period,
    estimate_usd: f64,
}

#[derive(Debug)]
struct Settlement {
    period: Period,
    estimate_usd: f64,
    actual_usd: f64,
}

impl Settlement {
    fn apply(&self, totals: &mut Totals) {
        if totals.day == self.period.day {
            totals.day_usd = (totals.day_usd - self.estimate_usd).max(0.0) + self.actual_usd;
        }
        if totals.month == self.period.month {
            totals.month_usd = (totals.month_usd - self.estimate_usd).max(0.0) + self.actual_usd;
            totals.month_usd = totals.month_usd.max(totals.day_usd);
        }
    }
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

    fn validate(&self) -> Result<(), String> {
        let date = self
            .day
            .checked_mul(86_400)
            .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
            .ok_or(STORAGE_ERROR)?;
        if self.version != 1
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
        if self.day < period.day {
            self.day = period.day;
            self.day_usd = 0.0;
        }
        if self.month < period.month {
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

    /// Reserves an image's estimated cost against the Images budget's
    /// limits. A zero limit turns paid images off before anything is read.
    pub fn reserve(&self, limits: &BudgetLimits, estimate_usd: f64) -> Result<Reservation, String> {
        limits.validate()?;
        if !valid_amount(estimate_usd) {
            return Err("The estimated cost must be a finite, nonnegative dollar amount.".into());
        }
        if limits.off() {
            return Err("Paid images are disabled because an Images budget limit is zero.".into());
        }
        let mut state = self.inner.lock()?;
        let period = Period::now();
        let mut totals = self.inner.settle_pending(&mut state, period)?;
        totals.advance(period);
        let day_usd = totals.day_usd + estimate_usd;
        let month_usd = totals.month_usd + estimate_usd;
        let over = |total: f64, limit: Option<f64>| limit.is_some_and(|limit| total > limit);
        if !valid_amount(day_usd) || over(day_usd, limits.day_usd) {
            return Err("The Images budget's daily limit would be exceeded.".into());
        }
        if !valid_amount(month_usd) || over(month_usd, limits.month_usd) {
            return Err("The Images budget's monthly limit would be exceeded.".into());
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
            period: Period {
                day: totals.day,
                month: totals.month,
            },
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
        state.pending.push(Settlement {
            period: self.period,
            estimate_usd: self.estimate_usd,
            actual_usd,
        });
        self.ledger
            .inner
            .settle_pending(&mut state, Period::now())
            .map(|_| ())
    }
}

impl Ledger {
    fn lock(&self) -> Result<MutexGuard<'_, LedgerState>, String> {
        self.state.lock().map_err(|_| STORAGE_ERROR.to_string())
    }

    fn load(&self, state: &mut LedgerState, period: Period) -> Result<Totals, String> {
        if !fs::metadata(&self.root)
            .map_err(|_| STORAGE_ERROR)?
            .is_dir()
        {
            return Err(STORAGE_ERROR.into());
        }
        let path = self.root.join("spending.json");
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !state.initialized => {
                return Ok(Totals::new(period));
            }
            Err(_) => return Err(STORAGE_ERROR.into()),
        };
        state.initialized = true;
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
        let mut totals: Totals = serde_json::from_slice(&bytes).map_err(|_| STORAGE_ERROR)?;
        totals.validate()?;
        for settlement in &state.pending {
            settlement.apply(&mut totals);
        }
        totals.validate()?;
        Ok(totals)
    }

    fn settle_pending(&self, state: &mut LedgerState, period: Period) -> Result<Totals, String> {
        let totals = self.load(state, period)?;
        if !state.pending.is_empty() {
            self.save(state, &totals)?;
        }
        Ok(totals)
    }

    fn save(&self, state: &mut LedgerState, totals: &Totals) -> Result<(), String> {
        totals.validate()?;
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
        state.initialized = true;
        state.pending.clear();
        #[cfg(unix)]
        File::open(&self.root)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| STORAGE_ERROR)?;
        Ok(())
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

    /// Limits as the tests write them: a dollar figure each.
    fn limits(day_usd: f64, month_usd: f64) -> BudgetLimits {
        BudgetLimits::new(Some(day_usd), Some(month_usd))
    }

    /// The two and twenty dollars earlier versions defaulted to.
    fn capped() -> BudgetLimits {
        limits(2.0, 20.0)
    }

    #[test]
    fn defaults_are_three_budgets_without_limits() {
        let settings = SpendingSettings::default();
        assert_eq!(settings.chat, BudgetLimits::default());
        assert_eq!(settings.voice, BudgetLimits::default());
        assert_eq!(settings.images, BudgetLimits::default());
        assert!(settings.validate().is_ok());
        assert_eq!(
            serde_json::to_value(&settings).unwrap(),
            serde_json::json!({"chat": {}, "voice": {}, "images": {}})
        );
        assert_eq!(
            serde_json::from_str::<SpendingSettings>("{}").unwrap(),
            settings
        );
        let declaration = SpendingSettings::decl(&ts_rs::Config::default());
        assert!(declaration.contains("chat: BudgetLimits"));
        let declaration = BudgetLimits::decl(&ts_rs::Config::default());
        assert!(declaration.contains("dayUsd?: number"));
    }

    #[test]
    fn each_budget_has_its_own_optional_limits() {
        let value = serde_json::json!({
            "chat": {"dayUsd": 5.0, "monthUsd": 50.0},
            "voice": {"dayUsd": 10.0, "monthUsd": null},
            "images": {}
        });
        let settings: SpendingSettings = serde_json::from_value(value).unwrap();
        assert_eq!(settings.chat, limits(5.0, 50.0));
        assert_eq!(settings.voice, BudgetLimits::new(Some(10.0), None));
        assert_eq!(settings.images, BudgetLimits::default());
        assert_eq!(
            serde_json::to_value(&settings).unwrap(),
            serde_json::json!({
                "chat": {"dayUsd": 5.0, "monthUsd": 50.0},
                "voice": {"dayUsd": 10.0},
                "images": {}
            })
        );
        let missing: SpendingSettings =
            serde_json::from_value(serde_json::json!({"voice": null})).unwrap();
        assert_eq!(missing, SpendingSettings::default());
    }

    #[test]
    fn the_shared_limits_of_earlier_versions_are_voice_and_images_limits() {
        let legacy: SpendingSettings =
            serde_json::from_value(serde_json::json!({"dayUsd": 10, "monthUsd": 20})).unwrap();
        assert_eq!(legacy.chat, BudgetLimits::default());
        assert_eq!(legacy.voice, limits(10.0, 20.0));
        assert_eq!(legacy.images, limits(10.0, 20.0));
        assert_eq!(
            serde_json::to_value(&legacy).unwrap(),
            serde_json::json!({
                "chat": {},
                "voice": {"dayUsd": 10.0, "monthUsd": 20.0},
                "images": {"dayUsd": 10.0, "monthUsd": 20.0}
            })
        );
        let partial: SpendingSettings =
            serde_json::from_value(serde_json::json!({"dayUsd": 0.0})).unwrap();
        assert_eq!(partial.voice, BudgetLimits::new(Some(0.0), None));
        assert!(
            serde_json::from_value::<SpendingSettings>(
                serde_json::json!({"dayUsd": 1, "chat": {"dayUsd": 2}})
            )
            .is_err()
        );
    }

    #[test]
    fn no_limit_is_no_limit_for_images() {
        let root = tempfile::tempdir().unwrap();
        let ledger = SpendLedger::new(root.path().to_path_buf());
        let unlimited = BudgetLimits::default();
        ledger
            .reserve(&unlimited, 50.0)
            .unwrap()
            .charge(50.0)
            .unwrap();
        assert!(ledger.reserve(&unlimited, 500.0).is_ok());
        let day_only = BudgetLimits::new(Some(600.0), None);
        assert!(
            ledger
                .reserve(&day_only, 60.0)
                .unwrap_err()
                .contains("daily")
        );
    }

    #[test]
    fn either_zero_limit_disables_spending_without_io() {
        let root = tempfile::tempdir().unwrap();
        let ledger = SpendLedger::new(root.path().join("absent"));
        for settings in [limits(0.0, 20.0), limits(2.0, 0.0)] {
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
            for settings in [limits(amount, 20.0), limits(2.0, amount)] {
                assert!(settings.validate().is_err());
                assert!(ledger.reserve(&settings, 0.5).is_err());
            }
            assert!(ledger.reserve(&capped(), amount).is_err());
            let reservation = ledger.reserve(&capped(), 0.25).unwrap();
            assert!(reservation.charge(amount).is_err());
        }
        assert_eq!(read_totals(root.path()).day_usd, 1.0);
    }

    #[test]
    fn cancelled_attempts_stay_reserved_and_refusal_is_repeatable() {
        let root = tempfile::tempdir().unwrap();
        let ledger = SpendLedger::new(root.path().to_path_buf());
        let settings = capped();
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
        let settings = limits(10.0, 1.0);
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
        let settings = capped();
        let barrier = Arc::new(std::sync::Barrier::new(16));
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let ledger = ledger.clone();
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
        let reservation = ledger.reserve(&capped(), 1.5).unwrap();
        assert!(ledger.inner.state.try_lock().is_ok());
        drop(reservation);
        drop(ledger);
        let ledger = SpendLedger::new(root.path().to_path_buf());
        assert!(ledger.reserve(&capped(), 0.75).is_err());
        assert_eq!(read_totals(root.path()).month_usd, 1.5);
    }

    #[test]
    fn actual_cost_replaces_the_estimate_and_overage_is_recorded() {
        let root = tempfile::tempdir().unwrap();
        let ledger = SpendLedger::new(root.path().to_path_buf());
        let settings = capped();
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
        let reservation = ledger.reserve(&capped(), 0.75).unwrap();
        drop(ledger.reserve(&capped(), 0.5).unwrap());
        reservation.charge(0.0).unwrap();
        assert_eq!(read_totals(root.path()).day_usd, 0.5);
    }

    #[test]
    fn releasing_concurrent_reservations_cannot_make_roundoff_negative() {
        let root = tempfile::tempdir().unwrap();
        let ledger = SpendLedger::new(root.path().to_path_buf());
        let settings = capped();
        let reservations: Vec<_> = (0..3)
            .map(|_| ledger.reserve(&settings, 0.01).unwrap())
            .collect();
        for reservation in reservations {
            reservation.charge(0.0).unwrap();
        }
        assert_eq!(read_totals(root.path()).day_usd, 0.0);
        assert_eq!(read_totals(root.path()).month_usd, 0.0);
        assert!(ledger.reserve(&settings, settings.day_usd.unwrap()).is_ok());
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
            assert_eq!(ledger.reserve(&capped(), 0.5).unwrap_err(), STORAGE_ERROR);
            assert_eq!(fs::read(root.path().join("spending.json")).unwrap(), bytes);
            assert!(ledger.reserve(&capped(), 0.5).is_err());
            fs::remove_file(root.path().join("spending.json")).unwrap();
            assert!(ledger.reserve(&capped(), 0.5).is_err());
        }
    }

    #[test]
    fn unreadable_or_missing_existing_ledger_blocks_spending() {
        let root = tempfile::tempdir().unwrap();
        let ledger = SpendLedger::new(root.path().to_path_buf());
        drop(ledger.reserve(&capped(), 0.5).unwrap());
        let saved = fs::read(root.path().join("spending.json")).unwrap();
        fs::remove_file(root.path().join("spending.json")).unwrap();
        assert!(ledger.reserve(&capped(), 0.5).is_err());
        fs::create_dir(root.path().join("spending.json")).unwrap();
        let restarted = SpendLedger::new(root.path().to_path_buf());
        assert!(restarted.reserve(&capped(), 0.5).is_err());
        assert!(ledger.reserve(&capped(), 0.5).is_err());
        fs::remove_dir(root.path().join("spending.json")).unwrap();
        fs::write(root.path().join("spending.json"), saved).unwrap();
        ledger
            .reserve(&capped(), 0.5)
            .unwrap()
            .charge(0.25)
            .unwrap();
        assert_eq!(read_totals(root.path()).day_usd, 0.75);
    }

    #[test]
    fn a_failed_save_can_retry_after_storage_recovers() {
        let root = tempfile::tempdir().unwrap();
        let ledger = SpendLedger::new(root.path().to_path_buf());
        let reservation = ledger.reserve(&capped(), 0.5).unwrap();
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
        ledger.reserve(&capped(), 0.5).unwrap().charge(0.5).unwrap();
        reservation.charge(0.25).unwrap();
        assert_eq!(read_totals(root.path()).day_usd, 0.75);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_settlements_are_recovered_before_more_spending_is_admitted() {
        let root = tempfile::tempdir().unwrap();
        let ledger = SpendLedger::new(root.path().to_path_buf());
        let settings = capped();
        let reservation = ledger.reserve(&settings, 0.5).unwrap();
        let saved = fs::read(root.path().join("spending.json")).unwrap();
        fs::remove_file(root.path().join("spending.json")).unwrap();
        assert_eq!(reservation.charge(3.0).unwrap_err(), STORAGE_ERROR);
        assert!(ledger.reserve(&settings, 0.1).is_err());
        fs::write(root.path().join("spending.json"), saved).unwrap();
        assert_eq!(ledger.summary().unwrap().day_usd, 3.0);
        assert_eq!(read_totals(root.path()).day_usd, 0.5);
        assert_eq!(ledger.inner.lock().unwrap().pending.len(), 1);
        assert!(ledger.reserve(&settings, 0.1).is_err());
        assert_eq!(read_totals(root.path()).day_usd, 3.0);
        assert_eq!(read_totals(root.path()).month_usd, 3.0);
        assert_eq!(ledger.summary().unwrap().day_usd, 3.0);
        assert!(ledger.inner.lock().unwrap().pending.is_empty());
        assert!(ledger.reserve(&settings, 0.1).is_err());
        assert_eq!(read_totals(root.path()).day_usd, 3.0);
        let higher_limit = BudgetLimits {
            day_usd: Some(4.0),
            ..settings
        };
        ledger
            .reserve(&higher_limit, 0.5)
            .unwrap()
            .charge(0.25)
            .unwrap();
        assert_eq!(read_totals(root.path()).day_usd, 3.25);
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
            drop(ledger.reserve(&capped(), 0.5).unwrap());
            reservation.charge(0.25).unwrap();
            let current = read_totals(root.path());
            assert_eq!(current.day, Period::now().day);
            assert_eq!(current.month, Period::now().month);
            assert_eq!(current.day_usd, 0.5);
            assert_eq!(current.month_usd, if same_month { 0.75 } else { 0.5 });
        }
    }

    #[test]
    fn invalid_totals_fail_closed() {
        let period = Period::now();
        let mut totals = Totals::new(period);
        totals.day_usd = 0.5;
        totals.month_usd = 1.0;
        for (field, value) in [
            ("version", serde_json::json!(2)),
            ("day", serde_json::json!(i64::MAX)),
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
            assert!(ledger.reserve(&capped(), 0.5).is_err());
        }
    }

    #[test]
    fn clock_rollback_keeps_future_totals_and_settles_in_the_reserved_period() {
        for days_ahead in [1, 40] {
            let root = tempfile::tempdir().unwrap();
            let ledger = SpendLedger::new(root.path().to_path_buf());
            let future = Utc::now() + chrono::Duration::days(days_ahead);
            let period = Period {
                day: future.timestamp().div_euclid(86_400),
                month: future.year() * 12 + future.month0() as i32,
            };
            let mut totals = Totals::new(period);
            totals.day_usd = 0.75;
            totals.month_usd = 1.5;
            fs::write(
                root.path().join("spending.json"),
                serde_json::to_vec(&totals).unwrap(),
            )
            .unwrap();
            let summary = ledger.summary().unwrap();
            assert_eq!(summary.day_usd, 0.75);
            assert_eq!(summary.month_usd, 1.5);
            assert!(ledger.reserve(&capped(), 1.5).is_err());
            let reservation = ledger.reserve(&capped(), 0.25).unwrap();
            assert_eq!(reservation.period.day, period.day);
            assert_eq!(reservation.period.month, period.month);
            reservation.charge(0.125).unwrap();
            let saved = read_totals(root.path());
            assert_eq!(saved.day, period.day);
            assert_eq!(saved.month, period.month);
            assert_eq!(saved.day_usd, 0.875);
            assert_eq!(saved.month_usd, 1.625);
            assert_eq!(ledger.summary().unwrap().day_usd, 0.875);
        }
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
        drop(ledger.reserve(&capped(), 0.5).unwrap());
        ledger
            .reserve(&capped(), 0.75)
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
