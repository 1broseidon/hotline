use super::*;
use crate::accounts::support::CLAUDE;
use crate::contract::{AccountSetupOrigin, SetupFrom};
use crate::private_dir::make_private_directory;
use serde_json::{Value, json};
use std::io::ErrorKind;
use tempfile::TempDir;

struct Fixture {
    _tmp: TempDir,
    home: PathBuf,
    source: SetupSource,
    account: PathBuf,
    outside: PathBuf,
}

const SETTINGS: &str = r#"{"apiKeyHelper":"/bin/key","awsAuthRefresh":"aws sso login","env":{"ANTHROPIC_API_KEY":"sk-test","KEEP":"1"},"permissions":{"deny":["Bash(rm:*)"]},"enabledPlugins":{"p@m":true},"extraKnownMarketplaces":{"m":{}}}"#;

fn write(path: impl AsRef<Path>, text: &str) {
    let path = path.as_ref();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn read(path: impl AsRef<Path>) -> String {
    std::fs::read_to_string(path).unwrap()
}

fn json(path: impl AsRef<Path>) -> Value {
    serde_json::from_str(&read(path)).unwrap()
}

fn ids(ids: &[&str]) -> Vec<String> {
    ids.iter().map(ToString::to_string).collect()
}

fn sorted(mut keys: Vec<String>) -> Vec<String> {
    keys.sort();
    keys
}

fn is_link(path: impl AsRef<Path>) -> bool {
    std::fs::symlink_metadata(path).unwrap().is_symlink()
}

fn link_to_folder(link: &Path, target: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    assert!(
        std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(is_link(link));
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("Conta Teste ção");
    let home = root.join("home");
    let dir = home.join(CLAUDE.home_dir);
    let outside = root.join("outside");
    let account = root.join("account");

    write(dir.join(".credentials.json"), r#"{"token":"secret"}"#);
    write(dir.join("history.jsonl"), "{}\n");
    write(dir.join("projects/p/s.jsonl"), "{}\n");
    write(dir.join("sessions/1.json"), "{}");
    write(dir.join("todos/t.json"), "{}");
    write(dir.join("CLAUDE.md"), "be kind");
    write(dir.join("rules/style.md"), "short");
    write(dir.join("skills/review/SKILL.md"), "review");
    write(dir.join("agents/critic.md"), "critic");
    write(dir.join("plugins/installed_plugins.json"), "{}");
    write(dir.join("settings.json"), SETTINGS);
    write(
        home.join(CLAUDE.state_file.unwrap()),
        r#"{"mcpServers":{"echo":{"command":"echo"}},"oauthAccount":{"emailAddress":"me@home"},"projects":{"/x":{}}}"#,
    );
    write(outside.join("keep.md"), "keep");
    link_to_folder(&dir.join("skills/linked"), &outside);

    make_private_directory(&account).unwrap();
    write(
        account.join(".claude.json"),
        r#"{"oauthAccount":{"emailAddress":"acct@x"}}"#,
    );
    write(account.join("CLAUDE.md"), "mine");
    write(account.join("skills/stale/SKILL.md"), "stale");

    let source = source_from(&CLAUDE, None, Some(home.clone())).unwrap();
    Fixture {
        _tmp: tmp,
        home,
        source,
        account,
        outside,
    }
}

fn all() -> Vec<String> {
    CLAUDE.setup.iter().map(|c| c.id.to_string()).collect()
}

#[test]
fn copy_brings_the_chosen_categories_and_no_secret() {
    let f = fixture();
    let acct = &f.account;
    let r = copy_setup(&f.source, acct, &CLAUDE, &all()).unwrap();
    assert_eq!(
        (r.copied.clone(), r.links_skipped),
        (
            ids(&["instructions", "skills", "plugins", "mcp", "settings"]),
            1
        )
    );
    for never in [
        ".credentials.json",
        "history.jsonl",
        "projects",
        "sessions",
        "todos",
    ] {
        assert!(!acct.join(never).exists());
    }
    assert_eq!(read(acct.join("CLAUDE.md")), "be kind");
    assert_eq!(read(acct.join("rules/style.md")), "short");
    assert_eq!(read(acct.join("agents/critic.md")), "critic");
    assert!(acct.join("plugins/installed_plugins.json").exists());
    assert!(
        acct.join("skills/review/SKILL.md").exists()
            && !acct.join("skills/stale").exists()
            && !acct.join("skills/linked").exists()
    );
    let s = json(acct.join("settings.json"));
    assert!(s.get("apiKeyHelper").is_none() && s.get("awsAuthRefresh").is_none());
    assert_eq!(
        (s["env"].clone(), s["permissions"]["deny"][0].clone()),
        (json!({"KEEP": "1"}), json!("Bash(rm:*)"))
    );
    assert_eq!(
        sorted(r.removed_keys),
        ["apiKeyHelper", "awsAuthRefresh", "env.ANTHROPIC_API_KEY"]
    );
    let st = json(acct.join(".claude.json"));
    assert_eq!(
        (
            st["mcpServers"].clone(),
            st["oauthAccount"]["emailAddress"].clone()
        ),
        (json!({"echo": {"command": "echo"}}), json!("acct@x"))
    );
    assert!(st.get("projects").is_none());
}

#[test]
fn a_setup_deeper_than_the_windows_path_limit_is_copied_and_copied_again() {
    let f = fixture();
    let deep = Path::new("skills")
        .join("d".repeat(120))
        .join("e".repeat(120))
        .join("SKILL.md");
    write(f.source.dir.join(&deep), "deep");
    copy_setup(&f.source, &f.account, &CLAUDE, &ids(&["skills"])).unwrap();
    assert_eq!(read(f.account.join(&deep)), "deep");
    write(f.source.dir.join(&deep), "deeper");
    copy_setup(&f.source, &f.account, &CLAUDE, &ids(&["skills"])).unwrap();
    assert_eq!(read(f.account.join(&deep)), "deeper");
}

#[test]
fn copy_again_replaces_only_the_marked_categories() {
    let f = fixture();
    let acct = &f.account;
    copy_setup(&f.source, acct, &CLAUDE, &all()).unwrap();
    write(acct.join("skills/mine/SKILL.md"), "mine");
    let mut account_settings = json(acct.join("settings.json"));
    account_settings["permissions"]["allow"] = json!(["Read"]);
    write(acct.join("settings.json"), &account_settings.to_string());
    let source_settings = f.source.dir.join("settings.json");
    let mut changed: Value = json(&source_settings);
    changed["enabledPlugins"] = json!({"q@m": true});
    changed
        .as_object_mut()
        .unwrap()
        .remove("extraKnownMarketplaces");
    write(&source_settings, &changed.to_string());

    copy_setup(&f.source, acct, &CLAUDE, &ids(&["plugins"])).unwrap();

    let s = json(acct.join("settings.json"));
    assert_eq!(s["permissions"]["allow"], json!(["Read"]));
    assert_eq!(s["permissions"]["deny"], json!(["Bash(rm:*)"]));
    assert_eq!(s["enabledPlugins"], json!({"q@m": true}));
    assert!(s.get("extraKnownMarketplaces").is_none() && s.get("apiKeyHelper").is_none());
    assert!(acct.join("skills/mine/SKILL.md").exists());

    std::fs::remove_file(f.source.dir.join("CLAUDE.md")).unwrap();
    copy_setup(&f.source, acct, &CLAUDE, &ids(&["instructions"])).unwrap();
    assert!(!acct.join("CLAUDE.md").exists());
    assert!(acct.join("rules/style.md").exists());
}

#[test]
fn a_refused_copy_leaves_the_account_as_it_was() {
    let f = fixture();
    let acct = &f.account;
    assert_eq!(
        copy_setup(&f.source, acct, &CLAUDE, &ids(&["instructions", "nope"]))
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(read(acct.join("CLAUDE.md")), "mine");
    assert!(!acct.join("rules").exists());

    write(acct.join(".claude.json"), "not json");
    assert_eq!(
        copy_setup(&f.source, acct, &CLAUDE, &ids(&["mcp"]))
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidData
    );
    assert_eq!(read(acct.join(".claude.json")), "not json");

    write(acct.join(".claude.json"), "[]");
    assert_eq!(
        copy_setup(&f.source, acct, &CLAUDE, &ids(&["mcp"]))
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidData
    );
    assert_eq!(read(acct.join(".claude.json")), "[]");
}

#[test]
fn a_source_file_that_is_not_json_refuses_that_category() {
    let f = fixture();
    write(f.source.dir.join("settings.json"), "{ nope");
    let before = read(f.account.join(".claude.json"));
    for id in ["settings", "plugins"] {
        let error = copy_setup(&f.source, &f.account, &CLAUDE, &ids(&[id])).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
    }
    assert!(!f.account.join("settings.json").exists());

    write(f.home.join(".claude.json"), "{ nope");
    let error = copy_setup(&f.source, &f.account, &CLAUDE, &ids(&["mcp"])).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert_eq!(read(f.account.join(".claude.json")), before);
}

#[test]
fn a_link_in_the_account_is_replaced_not_followed() {
    let f = fixture();
    let acct = &f.account;
    std::fs::remove_dir_all(acct.join("skills")).unwrap();
    link_to_folder(&acct.join("skills"), &f.outside);

    copy_setup(&f.source, acct, &CLAUDE, &ids(&["skills"])).unwrap();

    assert!(f.outside.join("keep.md").exists());
    assert!(!f.outside.join("review").exists());
    assert!(!is_link(acct.join("skills")));
    assert!(acct.join("skills/review/SKILL.md").exists());
}

#[test]
fn a_link_inside_a_mirrored_folder_of_the_account_is_unlinked_not_emptied() {
    let f = fixture();
    let acct = &f.account;
    link_to_folder(&acct.join("skills/stale-link"), &f.outside);

    copy_setup(&f.source, acct, &CLAUDE, &ids(&["skills"])).unwrap();

    assert!(f.outside.join("keep.md").exists());
    assert!(!acct.join("skills/stale-link").exists());
}

#[cfg(unix)]
#[test]
fn a_linked_source_file_is_skipped_and_counted() {
    let f = fixture();
    let real = f.outside.join("settings.json");
    write(&real, SETTINGS);
    let linked = f.source.dir.join("settings.json");
    std::fs::remove_file(&linked).unwrap();
    std::os::unix::fs::symlink(&real, &linked).unwrap();
    write(f.account.join("settings.json"), "{}");

    let r = copy_setup(&f.source, &f.account, &CLAUDE, &ids(&["settings"])).unwrap();

    assert_eq!(r.links_skipped, 1);
    assert!(!f.account.join("settings.json").exists());
}

#[test]
fn categories_run_in_row_order_once_each() {
    let f = fixture();
    let r = copy_setup(
        &f.source,
        &f.account,
        &CLAUDE,
        &ids(&["settings", "instructions", "settings"]),
    )
    .unwrap();
    assert_eq!(r.copied, ids(&["instructions", "settings"]));
    assert!(!f.account.join("skills/review").exists());
}

#[test]
fn a_setup_the_source_lacks_is_removed_from_the_account() {
    let f = fixture();
    copy_setup(&f.source, &f.account, &CLAUDE, &all()).unwrap();
    std::fs::remove_file(f.source.dir.join("settings.json")).unwrap();
    std::fs::remove_file(f.home.join(".claude.json")).unwrap();

    let r = copy_setup(&f.source, &f.account, &CLAUDE, &ids(&["settings", "mcp"])).unwrap();

    assert!(r.removed_keys.is_empty());
    assert!(!f.account.join("settings.json").exists());
    let st = json(f.account.join(".claude.json"));
    assert!(st.get("mcpServers").is_none());
    assert_eq!(st["oauthAccount"]["emailAddress"], "acct@x");
}

#[test]
fn merging_nothing_into_a_missing_file_creates_none() {
    let f = fixture();
    std::fs::remove_file(f.account.join(".claude.json")).unwrap();
    std::fs::remove_file(f.home.join(".claude.json")).unwrap();

    copy_setup(&f.source, &f.account, &CLAUDE, &ids(&["mcp"])).unwrap();

    assert!(!f.account.join(".claude.json").exists());
}

#[test]
fn settings_lose_a_credential_whatever_its_case() {
    let f = fixture();
    write(
        f.source.dir.join("settings.json"),
        r#"{"awsCredentialExport":"x","env":{"anthropic_base_url":"https://example.test","CLAUDE_CONFIG_DIR":"/other","KEEP":"1"}}"#,
    );

    let r = copy_setup(&f.source, &f.account, &CLAUDE, &ids(&["settings"])).unwrap();

    assert_eq!(
        sorted(r.removed_keys),
        [
            "awsCredentialExport",
            "env.CLAUDE_CONFIG_DIR",
            "env.anthropic_base_url"
        ]
    );
    assert_eq!(
        json(f.account.join("settings.json")),
        json!({"env": {"KEEP": "1"}})
    );
}

#[test]
fn the_source_is_the_env_folder_else_the_home_default() {
    let f = fixture();
    let home = f.home;
    let env = home.join("elsewhere");
    std::fs::create_dir_all(&env).unwrap();
    assert_eq!(
        source_from(&CLAUDE, Some(env.clone().into()), Some(home.clone())),
        Some(SetupSource {
            dir: env.clone(),
            state_file: Some(env.join(".claude.json"))
        })
    );
    assert_eq!(
        source_from(&CLAUDE, None, Some(home.clone())),
        Some(SetupSource {
            dir: home.join(".claude"),
            state_file: Some(home.join(".claude.json"))
        })
    );
    assert_eq!(
        source_from(&CLAUDE, Some("".into()), Some(home.clone())),
        source_from(&CLAUDE, None, Some(home.clone()))
    );
    assert_eq!(
        source_from(
            &CLAUDE,
            Some(home.join("missing").into()),
            Some(home.clone())
        ),
        None
    );
    assert_eq!(
        source_from(
            &CLAUDE,
            Some(home.join(".claude.json").into()),
            Some(home.clone())
        ),
        None
    );
    assert_eq!(source_from(&CLAUDE, None, Some(env.join("missing"))), None);
    assert_eq!(source_from(&CLAUDE, None, None), None);
    assert_eq!(
        nothing_to_copy("Claude Code"),
        "There is no Claude Code setup on this computer to copy."
    );
}

#[test]
fn setup_view_counts_items_and_names_paths_only_when_asked() {
    let f = fixture();
    let acct = &f.account;
    copy_setup(&f.source, acct, &CLAUDE, &all()).unwrap();
    let origin = AccountSetupOrigin {
        from: SetupFrom::SystemDefault,
        at: 7,
        categories: all(),
    };

    let v = setup_view(acct, &CLAUDE, Some(origin.clone()), false);
    assert_eq!(v.setup, Some(origin));
    assert_eq!(
        v.categories
            .iter()
            .map(|c| (c.id.as_str(), c.count))
            .collect::<Vec<_>>(),
        [
            ("instructions", 2),
            ("skills", 2),
            ("plugins", 1),
            ("mcp", 1),
            ("settings", 1)
        ]
    );
    assert!(v.categories.iter().all(|c| c.path.is_none()));

    let empty = f.account.with_file_name("empty");
    make_private_directory(&empty).unwrap();
    let v = setup_view(&empty, &CLAUDE, None, true);
    assert_eq!(v.setup, None);
    assert!(v.categories.iter().all(|c| c.count == 0));
    assert_eq!(
        v.categories
            .iter()
            .map(|c| c.path.clone().unwrap())
            .collect::<Vec<_>>(),
        CLAUDE
            .setup
            .iter()
            .map(|c| empty.join(c.open).to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    );
    for open in [
        "CLAUDE.md",
        "settings.json",
        "skills",
        "plugins",
        ".claude.json",
    ] {
        assert!(!empty.join(open).exists());
    }
    assert_eq!(std::fs::read_dir(&empty).unwrap().count(), 0);
}

#[test]
fn setup_view_counts_nothing_behind_a_link_or_an_unreadable_file() {
    let f = fixture();
    let acct = &f.account;
    std::fs::remove_dir_all(acct.join("skills")).unwrap();
    link_to_folder(&acct.join("skills"), &f.outside);
    write(acct.join(".claude.json"), "not json");

    let counts = setup_view(acct, &CLAUDE, None, false)
        .categories
        .iter()
        .map(|c| (c.id.clone(), c.count))
        .collect::<Vec<_>>();

    assert_eq!(
        counts,
        [
            ("instructions".to_string(), 1),
            ("skills".to_string(), 0),
            ("plugins".to_string(), 0),
            ("mcp".to_string(), 0),
            ("settings".to_string(), 0)
        ]
    );
}
