//! The scripted ACP agent this package builds (`hotline-acp-fixture`), put in
//! a desk's catalogue so a teammate can run on it as a real child.
//!
//! It reaches the core the way a catalogue agent shipped as an archive does:
//! the catalogue is written before the core starts, with the command already
//! where the archive would have unpacked it, so nothing is downloaded.

use serde_json::json;
use sha2::Digest;
use std::path::{Path, PathBuf};

pub const BACKEND: &str = "acp-fixture";
const ARCHIVE: &str = "https://example.invalid/acp-fixture.tar.gz";

/// The fixture's binary. Cargo sets its path at run time on this toolchain;
/// when it does not, the bin is one directory up from this test's own `deps`.
pub fn built() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_hotline-acp-fixture") {
        return PathBuf::from(path);
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| {
            let name = format!("hotline-acp-fixture{}", std::env::consts::EXE_SUFFIX);
            Some(exe.parent()?.parent()?.join(name))
        })
        .filter(|path| path.exists())
        .expect("hotline-acp-fixture should have been built with this test")
}

/// Puts the fixture in `root`'s catalogue, its command on disk.
pub fn install(root: &Path) {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    let platform = format!("{os}-{}", std::env::consts::ARCH);
    let command = format!("hotline-acp-fixture{}", std::env::consts::EXE_SUFFIX);
    let catalogue = json!({
        "format": 2,
        "fetchedAt": chrono::Utc::now().timestamp_millis(),
        "agents": [{
            "id": BACKEND,
            "name": "ACP Fixture",
            "distribution": {"binary": {platform: {
                "archive": ARCHIVE,
                "cmd": format!("./{command}"),
            }}},
        }],
    });
    let cache = root.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("acp-registry.json"), catalogue.to_string()).unwrap();

    let folder = hex::encode(&sha2::Sha256::digest(ARCHIVE.as_bytes())[..8]);
    let dir = root.join("acp-agents").join(BACKEND).join(folder);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(built(), dir.join(command)).unwrap();
}
