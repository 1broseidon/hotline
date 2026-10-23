use super::*;
use crate::accounts::resolve::ResolvedAccount;
use crate::accounts::support::{AccountSupport, account_support};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use tempfile::TempDir;

const ID: &str = "89bbac55-e7ac-4409-a709-115f33bb12f0";

fn support() -> &'static AccountSupport {
    account_support("claude-acp").expect("the one row")
}

fn account() -> (TempDir, ResolvedAccount) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("Conta Teste ção").join(ID);
    std::fs::create_dir_all(&dir).unwrap();
    let account = ResolvedAccount {
        id: ID.to_string(),
        backend_id: "claude-acp".to_string(),
        name: "Account 1".to_string(),
        dir: dunce::canonicalize(dir).unwrap(),
        support: support(),
    };
    (tmp, account)
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
        .collect()
}

fn os_pairs(list: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
    list.iter()
        .map(|(name, value)| (OsString::from(*name), OsString::from(*value)))
        .collect()
}

#[test]
fn system_default_removes_nothing_and_keeps_the_legacy_order() {
    let scrubbed = support().scrub[0];
    let catalogue = pairs(&[("A", "1"), (scrubbed, "cat")]);
    let method = pairs(&[("B", "2")]);
    assert_eq!(
        env_plan(&catalogue, &method, None),
        EnvPlan {
            remove: vec![],
            set: os_pairs(&[("A", "1"), (scrubbed, "cat"), ("B", "2")]),
        }
    );
}

#[test]
fn an_account_scrubs_every_listed_name_and_sets_its_folder_last() {
    let (_tmp, account) = account();
    let row = support();
    let plan = env_plan(&pairs(&[("A", "1")]), &pairs(&[("B", "2")]), Some(&account));
    assert_eq!(
        plan.remove,
        row.scrub.iter().map(OsString::from).collect::<Vec<_>>()
    );
    assert_eq!(
        plan.set,
        vec![
            (OsString::from("A"), OsString::from("1")),
            (OsString::from("B"), OsString::from("2")),
            (OsString::from(row.env_var), account.dir.into_os_string()),
        ]
    );
}

#[test]
fn a_catalogue_or_method_that_reintroduces_a_scrubbed_name_loses() {
    let (_tmp, account) = account();
    let row = support();
    let lowercased = row.scrub[6].to_ascii_lowercase();
    assert_ne!(lowercased, row.scrub[6]);
    let plan = env_plan(
        &pairs(&[(row.scrub[0], "cat"), (&lowercased, "x"), ("KEEP", "1")]),
        &pairs(&[(row.env_var, "elsewhere"), ("M", "2")]),
        Some(&account),
    );
    assert_eq!(
        plan.set,
        vec![
            (OsString::from("KEEP"), OsString::from("1")),
            (OsString::from("M"), OsString::from("2")),
            (OsString::from(row.env_var), account.dir.into_os_string()),
        ]
    );
}

#[test]
fn the_command_applier_removes_then_sets() {
    let (_tmp, account) = account();
    let row = support();
    let plan = env_plan(
        &pairs(&[(row.scrub[0], "cat"), ("KEEP", "1")]),
        &[],
        Some(&account),
    );
    let mut command = tokio::process::Command::new("agent");
    command.env(row.scrub[0], "inherited");
    plan.apply(&mut command);
    let envs: BTreeMap<&OsStr, Option<&OsStr>> = command.as_std().get_envs().collect();
    assert_eq!(envs.get(OsStr::new(row.scrub[0])), Some(&None));
    assert_eq!(
        envs.get(OsStr::new(row.env_var)),
        Some(&Some(account.dir.as_os_str()))
    );
    assert_eq!(envs.get(OsStr::new("KEEP")), Some(&Some(OsStr::new("1"))));
}

#[cfg(unix)]
#[test]
fn the_pty_applier_removes_then_sets() {
    let (_tmp, account) = account();
    let row = support();
    let plan = env_plan(&pairs(&[(row.scrub[0], "cat")]), &[], Some(&account));
    let mut builder = portable_pty::CommandBuilder::new("agent");
    builder.env(row.scrub[0], "inherited");
    plan.apply_pty(&mut builder);
    assert_eq!(builder.get_env(row.scrub[0]), None);
    assert_eq!(builder.get_env(row.env_var), Some(account.dir.as_os_str()));
}
