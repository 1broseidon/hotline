//! Subscription images consume a durable daily slot before dispatch. Dropping
//! a slot keeps it counted, because cancellation cannot prove no image was made.

use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

const LIMIT: u32 = 20;
const FILE_NAME: &str = "chatgpt-images.json";
const MAX_BYTES: u64 = 4096;
const LIMIT_ERROR: &str = "This room has made 20 ChatGPT images today; the limit resets tomorrow.";
const STORAGE_ERROR: &str =
    "The ChatGPT image counter could not be safely read or saved. Image generation is blocked.";

#[derive(Clone, Debug)]
pub(crate) struct SubscriptionQuota {
    inner: Arc<Counter>,
}

#[derive(Debug)]
struct Counter {
    root: PathBuf,
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    initialized: bool,
    pending_releases: Vec<i64>,
}

#[derive(Debug)]
pub(crate) struct Slot {
    quota: SubscriptionQuota,
    day: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Total {
    version: u8,
    day: i64,
    count: u32,
}

impl Total {
    fn validate(&self) -> Result<(), String> {
        let valid_day = self
            .day
            .checked_mul(86_400)
            .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
            .is_some();
        if self.version != 1 || !valid_day || self.count > LIMIT {
            return Err(STORAGE_ERROR.into());
        }
        Ok(())
    }
}

fn today() -> i64 {
    chrono::Utc::now().timestamp().div_euclid(86_400)
}

impl SubscriptionQuota {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            inner: Arc::new(Counter {
                root,
                state: Mutex::new(State::default()),
            }),
        }
    }

    pub(crate) fn reserve(&self) -> Result<Slot, String> {
        self.reserve_on(today())
    }

    fn reserve_on(&self, day: i64) -> Result<Slot, String> {
        let mut state = self.inner.lock()?;
        let mut total = self.inner.settle_pending(&mut state, day)?;
        // A backward clock retains the later period and its higher count.
        if total.day < day {
            total.day = day;
            total.count = 0;
        }
        if total.count >= LIMIT {
            return Err(LIMIT_ERROR.into());
        }
        total.count += 1;
        self.inner.save(&mut state, &total)?;
        Ok(Slot {
            quota: self.clone(),
            day: total.day,
        })
    }
}

impl Slot {
    pub(crate) fn release(self) -> Result<(), String> {
        let mut state = self.quota.inner.lock()?;
        state.pending_releases.push(self.day);
        self.quota
            .inner
            .settle_pending(&mut state, today())
            .map(|_| ())
    }
}

impl Counter {
    fn lock(&self) -> Result<MutexGuard<'_, State>, String> {
        self.state.lock().map_err(|_| STORAGE_ERROR.to_string())
    }

    fn load(&self, state: &mut State, day: i64) -> Result<Total, String> {
        if !fs::metadata(&self.root)
            .map_err(|_| STORAGE_ERROR)?
            .is_dir()
        {
            return Err(STORAGE_ERROR.into());
        }
        let path = self.root.join(FILE_NAME);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !state.initialized => {
                return Ok(Total {
                    version: 1,
                    day,
                    count: 0,
                });
            }
            Err(_) => return Err(STORAGE_ERROR.into()),
        };
        state.initialized = true;
        if !metadata.is_file() || metadata.len() > MAX_BYTES {
            return Err(STORAGE_ERROR.into());
        }
        let mut bytes = Vec::new();
        File::open(path)
            .map_err(|_| STORAGE_ERROR)?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| STORAGE_ERROR)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(STORAGE_ERROR.into());
        }
        let mut total: Total = serde_json::from_slice(&bytes).map_err(|_| STORAGE_ERROR)?;
        total.validate()?;
        for released_day in &state.pending_releases {
            if total.day == *released_day {
                total.count = total.count.checked_sub(1).ok_or(STORAGE_ERROR)?;
            }
        }
        Ok(total)
    }

    fn settle_pending(&self, state: &mut State, day: i64) -> Result<Total, String> {
        let total = self.load(state, day)?;
        if !state.pending_releases.is_empty() {
            self.save(state, &total)?;
        }
        Ok(total)
    }

    fn save(&self, state: &mut State, total: &Total) -> Result<(), String> {
        total.validate()?;
        let bytes = serde_json::to_vec(total).map_err(|_| STORAGE_ERROR)?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(&self.root).map_err(|_| STORAGE_ERROR)?;
        temporary.write_all(&bytes).map_err(|_| STORAGE_ERROR)?;
        temporary.as_file().sync_all().map_err(|_| STORAGE_ERROR)?;
        temporary
            .persist(self.root.join(FILE_NAME))
            .map_err(|_| STORAGE_ERROR)?;
        // Replacement applied the releases even if the directory sync fails.
        // Retrying must not subtract those same slots a second time.
        state.initialized = true;
        state.pending_releases.clear();
        #[cfg(unix)]
        File::open(&self.root)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| STORAGE_ERROR)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
