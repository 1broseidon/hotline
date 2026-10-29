//! How much of each account's plan is used: one reading per harness login,
//! kept across restarts and shown on every teammate that runs on it.
//!
//! A plan belongs to the login, not to the teammate, and a desk has one
//! login per harness, so a reading is kept by backend id. Every way a
//! reading arrives — Claude Code's report after a reply, `/usage` said
//! between turns, Codex's own logs — is merged into the one kept
//! ([`crate::driver::acp::plan::merged`]), written to the data directory,
//! and put on the `SessionInfo` of every live session on that backend,
//! including one that starts later.
//!
//! A reading is taken when a teammate's session starts and the kept one is
//! missing or a quarter of an hour old, when the picker opens and it is a
//! few minutes old, and, where it costs nothing, after every turn.

use super::*;
use crate::contract::PlanUsage;
use crate::driver::acp::plan::merged;
use std::io::Write as _;

/// How old a reading may be before opening the picker reads it again.
const REFRESH_AFTER_MS: i64 = 3 * 60_000;

/// How old a reading may be before a session starting reads it again.
const START_READ_AFTER_MS: i64 = 15 * 60_000;

/// How long after a session starts its plan is read, so the line that
/// started it reaches the agent first.
const START_READ_DELAY: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_secs(3)
};

/// The room's readings, and the accounts being read now.
#[derive(Default)]
pub(super) struct Plans {
    readings: Mutex<HashMap<String, PlanUsage>>,
    reading: Mutex<HashSet<String>>,
}

impl Plans {
    /// The readings kept when the desk last ran.
    pub(super) fn load(root: &Path) -> Self {
        let readings = std::fs::read(crate::paths::plan_usage_path(root))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            readings: Mutex::new(readings),
            reading: Mutex::default(),
        }
    }
}

/// An account being read: one read at a time per account.
struct Reading<'a> {
    plans: &'a Plans,
    backend_id: String,
}

impl<'a> Reading<'a> {
    fn begin(plans: &'a Plans, backend_id: &str) -> Option<Self> {
        lock(&plans.reading)
            .insert(backend_id.to_string())
            .then(|| Self {
                plans,
                backend_id: backend_id.to_string(),
            })
    }
}

impl Drop for Reading<'_> {
    fn drop(&mut self) {
        lock(&self.plans.reading).remove(&self.backend_id);
    }
}

/// Writes the readings whole, through a file beside the one it replaces.
fn save(root: &Path, readings: &HashMap<String, PlanUsage>) -> std::io::Result<()> {
    let path = crate::paths::plan_usage_path(root);
    let folder = path.parent().unwrap_or(root);
    std::fs::create_dir_all(folder)?;
    let mut temporary = tempfile::NamedTempFile::new_in(folder)?;
    temporary.write_all(&serde_json::to_vec(readings)?)?;
    temporary.as_file().sync_all()?;
    temporary.persist(&path).map_err(|error| error.error)?;
    Ok(())
}

impl Room {
    /// The account's reading as the room keeps it.
    pub(super) fn plan_of(&self, backend_id: &str) -> Option<PlanUsage> {
        lock(&self.plans.readings).get(backend_id).cloned()
    }

    /// A reading of an account, from any teammate on it: merged into the one
    /// kept, written down, and shown on every session of that account.
    pub(super) fn take_plan(&self, backend_id: &str, reading: PlanUsage) {
        let plan = {
            let mut readings = lock(&self.plans.readings);
            let was = readings.get(backend_id);
            let plan = merged(was, reading);
            if was == Some(&plan) {
                return;
            }
            readings.insert(backend_id.to_string(), plan.clone());
            // Under the lock, so two readings land on disk in the order they
            // were kept.
            if let Err(error) = save(self.log.root(), &readings) {
                eprintln!("the plan reading for {backend_id} was not kept: {error}");
            }
            plan
        };
        self.show_plan(backend_id, &plan);
    }

    /// Puts the account's reading on every live session that runs on it.
    fn show_plan(&self, backend_id: &str, plan: &PlanUsage) {
        let sessions: Vec<Arc<Session>> = lock(&self.sessions)
            .values()
            .filter(|session| session.backend_id == backend_id)
            .cloned()
            .collect();
        for session in sessions {
            let _lifecycle = lock(&self.lifecycle);
            if !session.capability.is_current() || !self.current_session(&session) {
                continue;
            }
            let info = {
                let mut info = lock(&session.info);
                if info.plan_usage.as_ref() == Some(plan) {
                    continue;
                }
                info.plan_usage = Some(plan.clone());
                info.clone()
            };
            let _ = self.info_changes.send(info);
        }
    }

    /// The picker opened on this teammate: its account is read again when
    /// the reading kept is a few minutes old and the teammate is idle, and
    /// otherwise left as it is, already on the teammate's session. A
    /// teammate that is not running has nothing to read with.
    pub fn refresh_plan(self: &Arc<Self>, persona_id: &str) -> Result<(), String> {
        let session = lock(&self.sessions).get(persona_id).cloned();
        if let Some(session) = session {
            self.read_plan(&session, Duration::ZERO, REFRESH_AFTER_MS, true);
        }
        Ok(())
    }

    /// A session has started: it shows its account's reading, and the
    /// account is read when it has none or one a quarter of an hour old.
    pub(super) fn read_plan_on_start(self: &Arc<Self>, session: &Arc<Session>) {
        self.read_plan(session, START_READ_DELAY, START_READ_AFTER_MS, true);
    }

    /// A turn has ended: an account whose reading costs nothing is read
    /// again, the turn having just used some of it.
    pub(super) fn read_plan_after_turn(self: &Arc<Self>, session: &Arc<Session>) {
        if session.driver.plan_after_turn() {
            self.read_plan(session, Duration::ZERO, 0, false);
        }
    }

    /// Reads the account through this session's agent, in the background,
    /// one read at a time per account, unless the reading kept is younger
    /// than `fresh_for` or, when the read asks the agent, the teammate is
    /// busy.
    fn read_plan(
        self: &Arc<Self>,
        session: &Arc<Session>,
        delay: Duration,
        fresh_for: i64,
        when_idle: bool,
    ) {
        let room = Arc::downgrade(self);
        let session = Arc::downgrade(session);
        tokio::spawn(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            let (Some(room), Some(session)) = (room.upgrade(), session.upgrade()) else {
                return;
            };
            let backend_id = session.backend_id.clone();
            let fresh = room
                .plan_of(&backend_id)
                .is_some_and(|plan| now_ms() - plan.at < fresh_for);
            if fresh || room.closing() || (when_idle && !room.plan_idle(&session)) {
                return;
            }
            let Some(_reading) = Reading::begin(&room.plans, &backend_id) else {
                return;
            };
            if let Some(reading) = session.driver.read_plan().await {
                room.take_plan(&backend_id, reading);
            }
        });
    }

    /// Whether the teammate is between turns with nothing queued, so asking
    /// its agent takes nothing from anybody.
    fn plan_idle(&self, session: &Arc<Session>) -> bool {
        session.capability.is_current()
            && self.current_session(session)
            && !lock(&session.turns).running
            && lock(&session.info).state == SessionState::Ready
    }
}
