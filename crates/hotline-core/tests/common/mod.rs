//! Harness credentials are disposable and never touch the operator's keychain.
use hotline_core::{credentials::SecretStore, desk::Desk, log::Log, vault::Vault};
use std::{
    collections::HashMap,
    io,
    path::Path,
    sync::{Arc, Mutex, OnceLock},
};

#[derive(Default)]
struct MemoryStore(Mutex<HashMap<String, Vec<u8>>>);
impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        self.0.lock().unwrap().insert(key.into(), bytes.into());
        Ok(())
    }
    fn delete(&self, key: &str) -> io::Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

fn store() -> Arc<dyn SecretStore> {
    static STORE: OnceLock<Arc<MemoryStore>> = OnceLock::new();
    STORE.get_or_init(Default::default).clone()
}

pub fn open_desk(root: &Path) -> io::Result<Desk> {
    Desk::open_with_store(root, store())
}

#[allow(dead_code)]
pub fn open_vault(root: &Path, log: Log) -> io::Result<Vault> {
    Vault::open_with_store(root, log, store())
}
