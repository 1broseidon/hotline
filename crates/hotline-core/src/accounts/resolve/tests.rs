use super::*;
use crate::contract::AgentAccount;
use crate::paths::{agent_account_dir, agent_accounts_dir};
use crate::private_dir::make_private_directory;
use crate::room::write_agent_accounts;
use crate::session::tests::persona;
use crate::test_support::link;
use tempfile::TempDir;

const ID: &str = "89bbac55-e7ac-4409-a709-115f33bb12f0";
const OTHER: &str = "0b7d1e1c-35f6-4f0c-8a47-6a0e2f1b9c33";
const UNKNOWN: &str = "5d0c6a8e-91b4-4c1d-b3a2-7e4f8a2d6c10";

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

fn ada(account: Option<&str>, backend_id: &str) -> Persona {
    Persona {
        account_id: account.map(str::to_string),
        backend_id: backend_id.to_string(),
        ..persona("ada")
    }
}

fn unknown(account_id: &str) -> AccountRefusal {
    AccountRefusal::UnknownAccount {
        account_id: account_id.to_string(),
    }
}

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
fn an_account_of_another_backend_or_a_credential_is_ignored_and_an_unknown_one_refused() {
    let desk = desk();
    let vault = crate::vault::Vault::open_with_store(
        &desk.root,
        desk.log.clone(),
        std::sync::Arc::new(crate::credentials::MemoryStore::default()),
    )
    .unwrap();
    let credential = vault.create("anthropic", "Work", "not-a-real-key").unwrap();
    let revoked = vault.create("openai", "Old", "not-a-real-key").unwrap();
    vault.revoke(&revoked.id).unwrap();
    for account_id in [OTHER, &credential.id, &revoked.id] {
        assert!(
            matches!(
                resolve(&desk.root, &desk.log, &ada(Some(account_id), "claude-acp")),
                Ok(None)
            ),
            "{account_id}"
        );
    }
    assert_eq!(
        resolve(&desk.root, &desk.log, &ada(Some(UNKNOWN), "claude-acp")).unwrap_err(),
        unknown(UNKNOWN)
    );
}

#[test]
fn a_teammate_runs_only_on_an_account_its_agent_can_use() {
    const ON_HOTLINE: &str = "6f1c2a3b-4d5e-4f60-8a71-b2c3d4e5f607";
    let desk = desk();
    let mut rows = room::agent_accounts(&desk.log);
    rows.push(AgentAccount {
        id: ON_HOTLINE.to_string(),
        backend_id: "hotline".to_string(),
        ..rows[0].clone()
    });
    write_agent_accounts(&desk.log, &rows).unwrap();
    let vault = crate::vault::Vault::open_with_store(
        &desk.root,
        desk.log.clone(),
        std::sync::Arc::new(crate::credentials::MemoryStore::default()),
    )
    .unwrap();
    let credential = vault
        .create("anthropic", "Work", "not-a-real-key")
        .unwrap()
        .id;
    let events = desk.log.load(&StreamId::Room);
    for (backend_id, its_own, another) in [
        ("claude-acp", ID, OTHER),
        ("codex-acp", OTHER, ID),
        ("hotline", ON_HOTLINE, ID),
    ] {
        for (kind, account_id, runs) in [
            ("none", None, false),
            ("its agent's", Some(its_own), backend_id == "claude-acp"),
            ("another agent's", Some(another), false),
            (
                "credential",
                Some(credential.as_str()),
                backend_id == "hotline",
            ),
            ("unknown", Some(UNKNOWN), false),
        ] {
            assert_eq!(
                crate::accounts::account_of_from(&events, &ada(account_id, backend_id)),
                account_id.filter(|_| runs).map(str::to_string),
                "{backend_id}, {kind}"
            );
        }
    }
}

#[test]
fn a_missing_or_replaced_folder_is_refused_with_its_kind() {
    let desk = desk();
    let path = agent_account_dir(&desk.root, ID).unwrap();
    let on_its_agent = ada(Some(ID), "claude-acp");

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
    let public = missing.public_sentence("Ada", "Claude Code");
    assert_eq!(
        public,
        "Ada can't start: its Claude Code account needs attention at the desk."
    );
    assert!(!public.contains(&*path.to_string_lossy()) && !public.contains("Account 1"));

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
