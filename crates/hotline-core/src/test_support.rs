//! What the unit tests of more than one module need.

use crate::accounts::resolve::{self, ResolvedAccount};
use crate::contract::AgentAccount;
use crate::driver::acp::registry::Launch;
use crate::log::Log;
use crate::room::{agent_accounts, write_agent_accounts};
use crate::{paths, private_dir};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

/// How long a test waits for something before calling it lost. Generous,
/// because a loaded runner is slow and a passing test never waits it out; the
/// harnesses' `common::ANSWER` is the same minute.
pub(crate) const PATIENCE: Duration = Duration::from_secs(60);

/// Waits until `condition` holds, and fails the test with `what` if it does
/// not within `PATIENCE`. A test waits on the signal itself this way, never
/// on a sleep it hopes is long enough.
pub(crate) async fn until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while !condition() {
        assert!(tokio::time::Instant::now() < deadline, "never: {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// A unit test is not told where its bins are, so the fixture is found one
/// directory up from `deps`, where `cargo build --bin hotline-acp-fixture`
/// left it.
pub(crate) fn acp_fixture_launch(env: &[(&str, &str)]) -> Launch {
    let name = format!("hotline-acp-fixture{}", std::env::consts::EXE_SUFFIX);
    let command = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join(name)))
        .filter(|path| path.exists())
        .expect("hotline-acp-fixture should have been built before these tests");
    Launch {
        command: command.to_string_lossy().into_owned(),
        args: Vec::new(),
        env: env
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect(),
    }
}

pub(crate) fn account_on_disk(
    log: &Log,
    id: &str,
    name: &str,
    login_at: Option<i64>,
) -> ResolvedAccount {
    let mut accounts = agent_accounts(log);
    accounts.push(AgentAccount {
        id: id.to_string(),
        backend_id: "claude-acp".to_string(),
        name: name.to_string(),
        created_at: 1_700_000_000_000,
        login_at,
        setup: None,
    });
    write_agent_accounts(log, &accounts).unwrap();
    private_dir::make_private_directory(&paths::agent_account_dir(log.root(), id).unwrap())
        .unwrap();
    resolve::account(log.root(), log, id).unwrap()
}

/// On Windows a junction, which `mklink /J` makes without the privilege a
/// directory symlink needs.
pub(crate) fn link(target: &Path, at: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, at).unwrap();
    #[cfg(windows)]
    {
        let status = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(at)
            .arg(target)
            .output()
            .unwrap()
            .status;
        assert!(status.success(), "mklink /J {at:?} {target:?}");
    }
}

pub(crate) fn clearing_checkpoints(
    log: &Log,
    persona_id: &str,
) -> (mpsc::Sender<()>, std::thread::JoinHandle<()>) {
    let (held, holding) = mpsc::channel();
    let (go, going) = mpsc::channel::<()>();
    let clearing = std::thread::spawn({
        let (log, persona_id) = (log.clone(), persona_id.to_string());
        move || {
            let _writes = crate::room::persona_write_lock();
            let mut persona = crate::room::roster(&log)
                .into_iter()
                .find(|persona| persona.id == persona_id)
                .unwrap();
            held.send(()).unwrap();
            going.recv().unwrap();
            persona.session_checkpoints.clear();
            crate::room::append_persona(&log, &persona).unwrap();
        }
    });
    holding.recv().unwrap();
    (go, clearing)
}
