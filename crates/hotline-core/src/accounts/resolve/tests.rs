use super::*;
use crate::contract::AgentAccount;
use crate::paths::{agent_account_dir, agent_accounts_dir};
use crate::private_dir::make_private_directory;
use crate::room::write_agent_accounts;
use crate::session::tests::persona;
use tempfile::TempDir;

const ID: &str = "89bbac55-e7ac-4409-a709-115f33bb12f0";
/// An account of an agent with no row.
const OTHER: &str = "0b7d1e1c-35f6-4f0c-8a47-6a0e2f1b9c33";
/// A well-formed id that no account has.
const UNKNOWN: &str = "5d0c6a8e-91b4-4c1d-b3a2-7e4f8a2d6c10";

/// A data directory whose name has spaces and a non-ASCII letter, as a profile
/// name can, with two accounts on its desk: the one with a row, and one of an
/// agent that has none.
struct Desk {
    tmp: TempDir,
    root: PathBuf,
    log: Log,
}

fn desk() -> Desk {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("Conta Teste ção");
    let log = Log::open(&root);
    let account = |id: &str, backend_id: &str, name: &str, login_at| AgentAccount {
        id: id.to_string(),
        backend_id: backend_id.to_string(),
        name: name.to_string(),
        created_at: 1_700_000_000_000,
        login_at,
        setup: None,
    };
    write_agent_accounts(
        &log,
        &[
            account(ID, "claude-acp", "Account 1", Some(1_700_000_100_000)),
            account(OTHER, "codex-acp", "Account 2", None),
        ],
    )
    .unwrap();
    Desk { tmp, root, log }
}

/// Ada, set to `account` on `backend_id`.
fn ada(account: Option<&str>, backend_id: &str) -> Persona {
    Persona {
        agent_account_id: account.map(str::to_string),
        backend_id: backend_id.to_string(),
        ..persona("ada")
    }
}

/// Makes `at` another way to reach `target`: a symlink, or on Windows a
/// junction, which `mklink /J` makes without the privilege a directory
/// symlink needs. `at` must not exist yet and its parent must.
fn link(target: &Path, at: &Path) {
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

fn unknown(account_id: &str) -> AccountRefusal {
    AccountRefusal::UnknownAccount {
        account_id: account_id.to_string(),
    }
}

/// What both entry points must answer for the folder of `ID`.
fn unavailable(root: &Path, kind: io::ErrorKind) -> AccountRefusal {
    AccountRefusal::FolderUnavailable {
        account_id: ID.to_string(),
        name: "Account 1".to_string(),
        path: agent_account_dir(root, ID).unwrap(),
        kind,
    }
}

#[test]
fn no_account_or_an_agent_without_accounts_is_system_default() {
    let desk = desk();
    for persona in [ada(None, "claude-acp"), ada(Some(ID), "codex-acp")] {
        assert!(matches!(resolve(&desk.root, &desk.log, &persona), Ok(None)));
    }
}

#[test]
fn an_account_resolves_to_its_canonical_folder() {
    let desk = desk();
    make_private_directory(&agent_account_dir(&desk.root, ID).unwrap()).unwrap();
    let canonical = crate::private_dir::checked_account_dir(&desk.root, ID).unwrap();

    let by_teammate = resolve(&desk.root, &desk.log, &ada(Some(ID), "claude-acp"))
        .unwrap()
        .expect("a teammate on its agent's account");
    let by_id = account(&desk.root, &desk.log, ID).unwrap();
    for resolved in [by_teammate, by_id] {
        assert_eq!(resolved.id, ID);
        assert_eq!(resolved.backend_id, "claude-acp");
        assert_eq!(resolved.name, "Account 1");
        assert_eq!(resolved.login_at, Some(1_700_000_100_000));
        assert!(std::ptr::eq(
            resolved.support,
            account_support("claude-acp").unwrap()
        ));
        assert_eq!(resolved.dir, canonical);
    }
}

#[test]
fn an_account_this_desk_does_not_have_is_refused_in_both_sentences() {
    let desk = desk();
    let on_its_agent = |id: &str| ada(Some(id), "claude-acp");
    assert_eq!(
        resolve(&desk.root, &desk.log, &on_its_agent(UNKNOWN)).unwrap_err(),
        unknown(UNKNOWN)
    );
    assert_eq!(
        account(&desk.root, &desk.log, UNKNOWN).unwrap_err(),
        unknown(UNKNOWN)
    );
    // The account is on the desk, but it belongs to an agent with no row, and
    // a teammate on another agent's account does not get it either.
    assert_eq!(
        resolve(&desk.root, &desk.log, &on_its_agent(OTHER)).unwrap_err(),
        unknown(OTHER)
    );
    assert_eq!(
        account(&desk.root, &desk.log, OTHER).unwrap_err(),
        unknown(OTHER)
    );

    let refusal = unknown(UNKNOWN);
    assert_eq!(
        refusal.private_sentence("Ada", "Claude Code"),
        "Pick a Claude Code account for Ada: the one it was set to is not on this desk."
    );
    assert_eq!(
        refusal.public_sentence("Ada", "Claude Code"),
        "Ada can't start: its Claude Code account needs attention at the desk."
    );
}

#[test]
fn a_missing_or_replaced_folder_is_refused_with_its_kind() {
    let desk = desk();
    let path = agent_account_dir(&desk.root, ID).unwrap();
    let on_its_agent = ada(Some(ID), "claude-acp");

    // No folder at all.
    let missing = unavailable(&desk.root, io::ErrorKind::NotFound);
    assert_eq!(
        resolve(&desk.root, &desk.log, &on_its_agent).unwrap_err(),
        missing
    );
    assert_eq!(account(&desk.root, &desk.log, ID).unwrap_err(), missing);
    assert_eq!(
        missing.private_sentence("Ada", "Claude Code"),
        format!(
            "Can't open Account 1's folder ({}): entity not found. Fix the folder or pick another account.",
            path.display()
        )
    );
    // What a teammate or a phone is told never carries the path or the name.
    let public = missing.public_sentence("Ada", "Claude Code");
    assert_eq!(
        public,
        "Ada can't start: its Claude Code account needs attention at the desk."
    );
    assert!(!public.contains(&*path.to_string_lossy()) && !public.contains("Account 1"));

    // A junction or symlink where the folder should be, to a folder that exists.
    let elsewhere = desk.tmp.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::create_dir_all(agent_accounts_dir(&desk.root)).unwrap();
    link(&elsewhere, &path);
    let replaced = unavailable(&desk.root, io::ErrorKind::PermissionDenied);
    assert_eq!(
        resolve(&desk.root, &desk.log, &on_its_agent).unwrap_err(),
        replaced
    );
    assert_eq!(account(&desk.root, &desk.log, ID).unwrap_err(), replaced);
}
