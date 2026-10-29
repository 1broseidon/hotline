//! How much of each account's plan is used: one reading per harness login,
//! kept across restarts and shown on every teammate that runs on it.
//!
//! A plan belongs to the login, not to the teammate, and a desk has one
//! login per harness, so a reading is kept by backend id and read by
//! backend, out of band, with no teammate's session in the way
//! ([`crate::driver::acp::plan`]). Every way a reading arrives — the read
//! out of band, Claude Code's report after a reply, Codex's own logs after a
//! turn — is merged into the one kept
//! ([`crate::driver::acp::plan::merged`]), written to the data directory,
//! and put on the `SessionInfo` of every live session on that backend,
//! including one that starts later.
//!
//! A login is read when the desk starts (each one some teammate runs on, a
//! few seconds apart), every ten minutes while it runs, when a teammate's
//! session starts and the kept reading is a quarter of an hour old, and
//! when the picker opens and it is two minutes old. Two reads of one login
//! at once are one: the second leaves it to the first.

use super::*;
use crate::contract::PlanUsage;
use crate::driver::acp::plan::{merged, readable};
use std::io::Write as _;

/// How old a reading may be before opening the picker reads it again.
pub(super) const REFRESH_AFTER_MS: i64 = 2 * 60_000;

/// How old a reading may be before a session starting reads it again.
const START_READ_AFTER_MS: i64 = 15 * 60_000;

/// How often the desk reads every login some teammate runs on, and how old
/// a reading may be before that read takes it again.
const WATCH_EVERY: Duration = Duration::from_mins(10);
const WATCH_READ_AFTER_MS: i64 = 9 * 60_000;

/// How long after the desk starts its logins are first read, and how far
/// apart, so a start does not begin by starting every harness's CLI at once.
const WATCH_FIRST: Duration = if cfg!(test) {
    // A test's room reads its logins when the test says (`read_logins`).
    Duration::from_hours(1)
} else {
    Duration::from_secs(10)
};
const WATCH_STAGGER: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_secs(5)
};

/// How long after a session starts its plan is read, so the line that
/// started it reaches the agent first.
const START_READ_DELAY: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_secs(3)
};

/// The room's readings, and the logins being read now.
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

/// A login being read: one read at a time per login.
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

/// Reads every login some teammate runs on: once soon after the desk
/// starts, then every ten minutes, each only when its reading is older than
/// the round. Ends with the room.
pub(super) fn watch(room: Weak<Room>) {
    tokio::spawn(async move {
        tokio::time::sleep(WATCH_FIRST).await;
        let mut fresh_for = REFRESH_AFTER_MS;
        while read_logins(&room, fresh_for).await {
            fresh_for = WATCH_READ_AFTER_MS;
            tokio::time::sleep(WATCH_EVERY).await;
        }
    });
}

/// One round of [`watch`]: each login some teammate runs on, a few seconds
/// apart, read when its reading is older than `fresh_for`. `false` once the
/// room is gone or closing.
pub(super) async fn read_logins(room: &Weak<Room>, fresh_for: i64) -> bool {
    let backends = match room.upgrade() {
        Some(room) => room.plan_backends(),
        None => return false,
    };
    for (at, backend_id) in backends.into_iter().enumerate() {
        if at > 0 {
            tokio::time::sleep(WATCH_STAGGER).await;
        }
        let Some(room) = room.upgrade() else {
            return false;
        };
        if room.closing() {
            return false;
        }
        room.read_login(&backend_id, fresh_for, PlanRead::Now).await;
    }
    true
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

    /// The picker opened on this teammate: its login is read again when
    /// the reading kept is two minutes old, whether or not the teammate is
    /// running, and otherwise left as it is, already on its sessions.
    pub fn refresh_plan(self: &Arc<Self>, persona_id: &str) -> Result<(), String> {
        let backend_id = self.persona(persona_id)?.backend_id;
        self.read_login_later(backend_id, Duration::ZERO, REFRESH_AFTER_MS, PlanRead::Now);
        Ok(())
    }

    /// A session has started: it shows its login's reading, and the login
    /// is read when it has none or one a quarter of an hour old.
    pub(super) fn read_plan_on_start(self: &Arc<Self>, session: &Arc<Session>) {
        self.read_login_later(
            session.backend_id.clone(),
            START_READ_DELAY,
            START_READ_AFTER_MS,
            PlanRead::Now,
        );
    }

    /// A turn has ended: a login whose harness has just written its plan
    /// down is read from there, for nothing.
    pub(super) fn read_plan_after_turn(self: &Arc<Self>, session: &Arc<Session>) {
        self.read_login_later(
            session.backend_id.clone(),
            Duration::ZERO,
            0,
            PlanRead::AfterTurn,
        );
    }

    /// The logins some teammate runs on whose plan the desk reads.
    fn plan_backends(&self) -> Vec<String> {
        let mut backends: Vec<String> = room::roster(&self.log)
            .into_iter()
            .map(|persona| persona.backend_id)
            .filter(|backend_id| readable(backend_id))
            .collect();
        backends.sort();
        backends.dedup();
        backends
    }

    /// [`Self::read_login`] in the background, after `delay`.
    fn read_login_later(
        self: &Arc<Self>,
        backend_id: String,
        delay: Duration,
        fresh_for: i64,
        how: PlanRead,
    ) {
        if !readable(&backend_id) {
            return;
        }
        let room = Arc::downgrade(self);
        tokio::spawn(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            if let Some(room) = room.upgrade() {
                room.read_login(&backend_id, fresh_for, how).await;
            }
        });
    }

    /// Reads a login, unless the reading kept is younger than `fresh_for`
    /// or the login is being read already, and takes what it gives.
    async fn read_login(&self, backend_id: &str, fresh_for: i64, how: PlanRead) {
        let fresh = self
            .plan_of(backend_id)
            .is_some_and(|plan| now_ms() - plan.at < fresh_for);
        if fresh || self.closing() {
            return;
        }
        let Some(_reading) = Reading::begin(&self.plans, backend_id) else {
            return;
        };
        if let Some(reading) = self.agents.read_plan(backend_id, how).await {
            self.take_plan(backend_id, reading);
        }
    }
}
