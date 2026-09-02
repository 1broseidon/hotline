use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use tokio::sync::{Mutex, OwnedMutexGuard, OwnedSemaphorePermit, Semaphore};

const DEFAULT_LEASE_SECONDS: u64 = 300;
const MAX_LEASE_SECONDS: u64 = 600;
const RUN_QUEUE_DEPTH: usize = 4;

#[derive(Clone)]
pub struct MachineAccess {
    action: Arc<Mutex<()>>,
    lease: Arc<Mutex<Option<ControlLease>>>,
    queue: Arc<Semaphore>,
    queue_holder: Arc<Mutex<Option<String>>>,
}

struct ControlLease {
    holder: String,
    expires: Instant,
}

pub struct RunPermit {
    _queue: OwnedSemaphorePermit,
    _action: OwnedMutexGuard<()>,
    queue_holder: Arc<Mutex<Option<String>>>,
    holder: String,
}

impl Drop for RunPermit {
    fn drop(&mut self) {
        let queue_holder = Arc::clone(&self.queue_holder);
        let holder = self.holder.clone();
        tokio::spawn(async move {
            let mut current = queue_holder.lock().await;
            if current.as_deref() == Some(&holder) {
                *current = None;
            }
        });
    }
}

impl MachineAccess {
    pub fn new() -> Self {
        Self {
            action: Arc::new(Mutex::new(())),
            lease: Arc::new(Mutex::new(None)),
            queue: Arc::new(Semaphore::new(RUN_QUEUE_DEPTH)),
            queue_holder: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn mutate(&self, holder: &str) -> Result<OwnedMutexGuard<()>, String> {
        self.check_lease(holder).await?;
        Ok(Arc::clone(&self.action).lock_owned().await)
    }

    pub async fn run(&self, holder: &str) -> Result<RunPermit, String> {
        let queue = Arc::clone(&self.queue)
            .try_acquire_owned()
            .map_err(|_| "run queue full".to_owned())?;
        self.check_lease(holder).await?;
        *self.queue_holder.lock().await = Some(holder.to_owned());
        let action = Arc::clone(&self.action).lock_owned().await;
        Ok(RunPermit {
            _queue: queue,
            _action: action,
            queue_holder: Arc::clone(&self.queue_holder),
            holder: holder.to_owned(),
        })
    }

    async fn check_lease(&self, holder: &str) -> Result<(), String> {
        let mut lease = self.lease.lock().await;
        if lease
            .as_ref()
            .is_some_and(|lease| lease.expires <= Instant::now())
        {
            *lease = None;
        }
        if let Some(lease) = lease.as_ref().filter(|lease| lease.holder != holder) {
            let remaining = lease.expires.saturating_duration_since(Instant::now());
            return Err(format!(
                "machine under {}'s control — lease expires in {}m {}s",
                lease.holder,
                remaining.as_secs() / 60,
                remaining.as_secs() % 60
            ));
        }
        Ok(())
    }

    pub async fn control(
        &self,
        holder: &str,
        seconds: Option<u64>,
    ) -> Result<(u64, SystemTime), String> {
        let seconds = seconds.unwrap_or(DEFAULT_LEASE_SECONDS);
        if seconds > MAX_LEASE_SECONDS {
            return Err(format!("max lease duration is {MAX_LEASE_SECONDS} seconds"));
        }
        let mut active = self.lease.lock().await;
        if active
            .as_ref()
            .is_some_and(|lease| lease.expires > Instant::now())
        {
            let lease = active.as_ref().expect("checked above");
            let remaining = lease.expires.saturating_duration_since(Instant::now());
            return Err(format!(
                "control lease already held by {}, expires in {}s",
                lease.holder,
                remaining.as_secs()
            ));
        }
        *active = Some(ControlLease {
            holder: holder.to_owned(),
            expires: Instant::now() + Duration::from_secs(seconds),
        });
        Ok((seconds, SystemTime::now() + Duration::from_secs(seconds)))
    }

    pub async fn release(&self, holder: &str) -> Result<bool, String> {
        let mut active = self.lease.lock().await;
        let Some(lease) = active.as_ref() else {
            return Ok(false);
        };
        if lease.expires <= Instant::now() {
            *active = None;
            return Ok(false);
        }
        if lease.holder != holder {
            return Err(format!(
                "control lease is held by {} — only the holder can release it",
                lease.holder
            ));
        }
        *active = None;
        Ok(true)
    }
}

impl Default for MachineAccess {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_another_holder_is_refused() {
        let access = MachineAccess::new();
        access.control("alice", Some(60)).await.unwrap();
        assert!(access.mutate("alice").await.is_ok());
        assert!(
            access
                .mutate("mallory")
                .await
                .unwrap_err()
                .contains("alice")
        );
        assert!(
            access
                .release("mallory")
                .await
                .unwrap_err()
                .contains("alice")
        );
        assert!(access.release("alice").await.unwrap());
    }
}
