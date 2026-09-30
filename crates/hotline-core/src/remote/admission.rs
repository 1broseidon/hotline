//! Anonymous sockets rotate through a bounded pool; only a proven device may
//! hold a session slot. A proxy's TCP address is not a device identity.
use std::{
    collections::VecDeque,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const PENDING_MAX: usize = 16;
const PENDING_PER_IP: usize = 4;
const AUTHENTICATED_MAX: usize = 16;
const AUTHENTICATED_PER_DEVICE: usize = 4;
const PRE_AUTH_LIFETIME: Duration = Duration::from_secs(5);
// Rotation must not let a burst evict a newly accepted handshake before it
// can run. This also bounds allocation churn when the pending pool is full.
const PRE_AUTH_GRACE: Duration = Duration::from_secs(1);

#[derive(Default)]
pub(super) struct Admission {
    connections: Mutex<VecDeque<Connection>>,
}

struct Connection {
    id: Uuid,
    ip: IpAddr,
    device: Option<String>,
    replace_after: Instant,
    evicted: CancellationToken,
    released: CancellationToken,
}

pub(super) struct Permit {
    admission: Arc<Admission>,
    id: Uuid,
    deadline: Instant,
    evicted: CancellationToken,
    authenticated: CancellationToken,
    released: CancellationToken,
}

impl Admission {
    /// The listener waits here, holding at most one extra accepted socket.
    /// Wait for the victim's last owner to drop before allocating replacement
    /// TLS/HTTP state, including when its WebSocket upgrade has already spawned.
    pub async fn accept(self: &Arc<Self>, ip: IpAddr, accepted: Instant) -> Arc<Permit> {
        let ip = match ip {
            IpAddr::V6(ip) => ip
                .to_ipv4_mapped()
                .map(IpAddr::V4)
                .unwrap_or(IpAddr::V6(ip)),
            ip => ip,
        };
        loop {
            let (released, ready) = {
                let mut connections = self.connections.lock().unwrap();
                let pending = || connections.iter().filter(|c| c.device.is_none());
                let victim = if pending().filter(|c| c.ip == ip).count() >= PENDING_PER_IP {
                    pending().find(|c| c.ip == ip)
                } else if pending().count() >= PENDING_MAX {
                    pending().next()
                } else {
                    None
                };
                if let Some(victim) = victim {
                    if Instant::now() < victim.replace_after {
                        (victim.released.clone(), Some(victim.replace_after))
                    } else {
                        victim.evicted.cancel();
                        (victim.released.clone(), None)
                    }
                } else {
                    let permit = Arc::new(Permit {
                        admission: self.clone(),
                        id: Uuid::new_v4(),
                        deadline: accepted + PRE_AUTH_LIFETIME,
                        evicted: CancellationToken::new(),
                        authenticated: CancellationToken::new(),
                        released: CancellationToken::new(),
                    });
                    connections.push_back(Connection {
                        id: permit.id,
                        ip,
                        device: None,
                        replace_after: Instant::now() + PRE_AUTH_GRACE,
                        evicted: permit.evicted.clone(),
                        released: permit.released.clone(),
                    });
                    return permit;
                }
            };
            if let Some(ready) = ready {
                tokio::select! {
                    _ = released.cancelled() => {},
                    _ = tokio::time::sleep_until(ready) => {},
                }
            } else {
                released.cancelled().await;
            }
        }
    }

    #[cfg(test)]
    pub fn counts(&self) -> (usize, usize) {
        let connections = self.connections.lock().unwrap();
        let pending = connections.iter().filter(|c| c.device.is_none()).count();
        (pending, connections.len() - pending)
    }
}

impl Permit {
    /// Called only after bearer validation or the Noise handshake. Promotion
    /// and eviction share a lock, so an evicted handshake cannot take a seat.
    pub fn authenticate(&self, device: &str) -> bool {
        let mut connections = self.admission.connections.lock().unwrap();
        if self.evicted.is_cancelled() || Instant::now() >= self.deadline {
            return false;
        }
        let authenticated = || connections.iter().filter(|c| c.device.is_some());
        if authenticated().count() >= AUTHENTICATED_MAX
            || authenticated()
                .filter(|c| c.device.as_deref() == Some(device))
                .count()
                >= AUTHENTICATED_PER_DEVICE
        {
            return false;
        }
        let connection = connections.iter_mut().find(|c| c.id == self.id).unwrap();
        if connection.device.is_some() {
            return false;
        }
        connection.device = Some(device.to_owned());
        self.authenticated.cancel();
        true
    }

    /// One deadline spans TLS, HTTP keepalive, upgrade and Noise. Once a
    /// device is authenticated, only normal session/revocation lifetimes apply.
    pub async fn expired(&self) {
        tokio::select! {
            biased;
            _ = self.authenticated.cancelled() => std::future::pending::<()>().await,
            _ = self.evicted.cancelled() => {},
            _ = tokio::time::sleep_until(self.deadline) => {},
        }
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.admission
            .connections
            .lock()
            .unwrap()
            .retain(|c| c.id != self.id);
        self.released.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn eviction_waits_for_every_owner_and_cannot_promote_its_victim() {
        let admission = Arc::new(Admission::default());
        let ip = "127.0.0.1".parse().unwrap();
        let started = Instant::now();
        let mut held = Vec::new();
        for _ in 0..PENDING_PER_IP {
            held.push(admission.accept(ip, Instant::now()).await);
        }
        let extra_owner = held[0].clone();
        let replacement = admission.accept("::ffff:127.0.0.1".parse().unwrap(), Instant::now());
        tokio::pin!(replacement);
        tokio::select! {
            _ = &mut replacement => panic!("replacement allocated before victim release"),
            _ = extra_owner.evicted.cancelled() => {},
        }
        assert!(started.elapsed() >= PRE_AUTH_GRACE);
        assert!(!extra_owner.authenticate("device"));
        held.remove(0);
        assert_eq!(admission.counts(), (4, 0));
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut replacement)
                .await
                .is_err()
        );
        drop(extra_owner);
        let replacement = tokio::time::timeout(Duration::from_secs(1), replacement)
            .await
            .unwrap();
        assert_eq!(admission.counts(), (4, 0));
        assert!(!held.iter().any(|p| p.evicted.is_cancelled()));
        drop(replacement);
        drop(held);
        assert_eq!(admission.counts(), (0, 0));
    }

    #[tokio::test]
    async fn global_pending_rotation_leaves_authenticated_sessions_and_device_budgets_alone() {
        let admission = Arc::new(Admission::default());
        let ip = "127.0.0.1".parse().unwrap();
        let mut sessions = Vec::new();
        for device in 0..4 {
            for _ in 0..AUTHENTICATED_PER_DEVICE {
                let permit = admission.accept(ip, Instant::now()).await;
                assert!(permit.authenticate(&device.to_string()));
                sessions.push(permit);
            }
            let fifth = admission.accept(ip, Instant::now()).await;
            assert!(!fifth.authenticate(&device.to_string()));
        }
        let overflow = admission.accept(ip, Instant::now()).await;
        assert!(!overflow.authenticate("another-device"));
        drop(overflow);
        assert_eq!(admission.counts(), (0, 16));
        let mut pending = Vec::new();
        for peer in 1..=PENDING_MAX {
            pending.push(
                admission
                    .accept(format!("192.0.2.{peer}").parse().unwrap(), Instant::now())
                    .await,
            );
        }
        assert_eq!(admission.counts(), (16, 16));
        let replacement = admission.accept("192.0.2.99".parse().unwrap(), Instant::now());
        tokio::pin!(replacement);
        tokio::select! {
            _ = &mut replacement => panic!("global pending bound exceeded"),
            _ = pending[0].evicted.cancelled() => {},
        }
        assert!(!sessions.iter().any(|p| p.evicted.is_cancelled()));
        pending.remove(0);
        let replacement = replacement.await;
        assert_eq!(admission.counts(), (16, 16));
        drop(sessions);
        assert!(replacement.authenticate("another-device"));
        assert_eq!(admission.counts(), (15, 1));
        drop(replacement);
        drop(pending);
        assert_eq!(admission.counts(), (0, 0));
    }

    #[tokio::test]
    async fn promotion_cannot_restart_an_expired_accept_deadline() {
        let admission = Arc::new(Admission::default());
        let permit = admission
            .accept(
                "127.0.0.1".parse().unwrap(),
                Instant::now() - PRE_AUTH_LIFETIME,
            )
            .await;
        tokio::time::timeout(Duration::from_secs(1), permit.expired())
            .await
            .unwrap();
        assert!(!permit.authenticate("device"));
        assert_eq!(admission.counts(), (1, 0));
        drop(permit);
        assert_eq!(admission.counts(), (0, 0));
    }
}
