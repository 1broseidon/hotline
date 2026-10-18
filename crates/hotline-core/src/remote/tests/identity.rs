use super::*;

/// An OS store that is locked: every call is refused and nothing is lost.
struct LockedStore;
impl SecretStore for LockedStore {
    fn get(&self, _: &str) -> io::Result<Option<Vec<u8>>> {
        Err(locked())
    }
    fn set(&self, _: &str, _: &[u8]) -> io::Result<()> {
        Err(locked())
    }
    fn delete(&self, _: &str) -> io::Result<()> {
        Err(locked())
    }
}
fn locked() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "store locked")
}

/// A room moved over from Toad holds that edition's reference to the
/// listener's certificate, which this build cannot read. Remote comes up on
/// a fresh certificate rather than refusing to turn on, and the phones that
/// pinned the old one need to pair again, but retain their records. A store
/// that is merely locked is the other case: what it holds is reported,
/// never replaced.
#[tokio::test]
async fn an_unreadable_identity_is_replaced_but_a_locked_store_is_reported() {
    let h = Harness::new().await;
    h.remote.configure(false, network::ALL).await.unwrap();
    let identity = h.root.path().join("remote-identity.json");
    let ours = fs::read(&identity).unwrap();
    let path = h.root.path().join("remote.json");
    let mut saved: Saved = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    saved.grants.push(Grant {
        device: RemoteDevice {
            id: "phone".into(),
            name: "Phone".into(),
            paired_at: now(),
            role: DeviceRole::Owner,
            public_key: None,
            current: None,
        },
        push: None,
    });
    atomic_write(&path, &serde_json::to_vec(&saved).unwrap()).unwrap();

    let locked = Remote::open_with_store(
        h.root.path(),
        h.desk.log.clone(),
        h.desk.clone(),
        Arc::new(LockedStore),
    )
    .unwrap();
    let refused = match locked.configure(true, network::ALL).await {
        Ok(_) => panic!("a locked store must not be replaced"),
        Err(refused) => refused,
    };
    assert!(refused.contains("store locked"), "{refused}");
    assert!(!locked.status().enabled);
    assert_eq!(locked.status().devices.len(), 1);
    assert_eq!(fs::read(&identity).unwrap(), ours);

    let earlier = br#"{"toadCredential":1,"generation":"6f1f9e16-6a0b-4a0e-9f3e-3f4b2f4a7a31","chunks":1,"digest":"7d8e4051a3d929fc56a545c5125a8589a4b4bfedd354466455403038d450144f"}"#;
    fs::write(&identity, earlier).unwrap();
    let restored = Remote::open_with_store(
        h.root.path(),
        h.desk.log.clone(),
        h.desk.clone(),
        h.store.clone(),
    )
    .unwrap();
    assert_eq!(restored.status().devices.len(), 1);
    let status = restored.configure(true, network::ALL).await.unwrap();
    assert!(status.enabled);
    assert!(status.error.is_none());
    assert_eq!(status.devices.len(), 1);
    let replaced = fs::read(&identity).unwrap();
    assert!(String::from_utf8_lossy(&replaced).contains("hotlineCredential"));
    assert_ne!(replaced, ours);
    restored.configure(false, network::ALL).await.unwrap();
}
