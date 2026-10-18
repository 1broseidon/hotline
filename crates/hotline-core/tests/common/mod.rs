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

#[allow(dead_code)]
pub fn store() -> Arc<dyn SecretStore> {
    static STORE: OnceLock<Arc<MemoryStore>> = OnceLock::new();
    STORE.get_or_init(Default::default).clone()
}

#[allow(dead_code)]
pub fn open_desk(root: &Path) -> io::Result<Desk> {
    Desk::open_with_store(root, store())
}

#[allow(dead_code)]
pub fn open_vault(root: &Path, log: Log) -> io::Result<Vault> {
    Vault::open_with_store(root, log, store())
}

#[cfg(unix)]
pub mod stalled;

/// Fails the test by name when `tool` cannot be started here. A harness whose
/// command runs an interpreter would otherwise time out waiting on what the
/// command was meant to do, and say nothing about the interpreter.
#[allow(dead_code)]
pub fn require(tool: &str) {
    let ran = std::process::Command::new(tool).arg("--version").output();
    assert!(
        ran.is_ok_and(|output| output.status.success()),
        "needs {tool}: it is not on PATH or does not run"
    );
}

/// Stands in for the system's link opener, so a teammate's `open_link` in a
/// harness opens no tab on the machine running it. The opener the core runs
/// (`open`, or `xdg-open`) is found on `PATH`; a script by that name goes
/// first on it and writes each link it is handed to the returned file.
#[cfg(unix)]
#[allow(dead_code)]
pub fn browser() -> &'static Path {
    static OPENED: OnceLock<std::path::PathBuf> = OnceLock::new();
    OPENED.get_or_init(|| {
        use std::os::unix::fs::PermissionsExt;
        let bin = tempfile::tempdir().unwrap().keep();
        let opened = bin.join("opened");
        let script = format!("#!/bin/sh\necho \"$1\" >> '{}'\n", opened.display());
        for name in ["open", "xdg-open"] {
            let path = bin.join(name);
            std::fs::write(&path, &script).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = std::env::var("PATH").unwrap_or_default();
        // Safety: std takes its environment lock for this and for every read
        // it makes, Command's PATH lookup included, and nothing in these
        // harnesses reads the environment through libc directly.
        unsafe { std::env::set_var("PATH", format!("{}:{path}", bin.display())) };
        opened
    })
}
