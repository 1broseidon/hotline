use super::*;

const ID: &str = "0b7e2c55-3f6a-4d8e-9c1a-5e4f3a2b1c0d";

fn data_dir() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("Conta Teste ção");
    let dir = root.join("agent-accounts").join(ID);
    fs::create_dir_all(&dir).unwrap();
    (tmp, root, dir)
}

fn link_to_folder(target: &Path, at: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, at).unwrap();
    #[cfg(windows)]
    {
        let made = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(at)
            .arg(target)
            .output()
            .unwrap();
        assert!(made.status.success(), "mklink /J {at:?} {target:?}");
    }
}

#[test]
fn only_an_item_a_row_opens_and_directly_in_an_account_folder_opens() {
    let (tmp, root, dir) = data_dir();
    let at = |rest: &str| format!("{}/{rest}", dir.display());
    fs::write(dir.join("CLAUDE.md"), "x").unwrap();
    fs::write(dir.join(".credentials.json"), "secret").unwrap();
    fs::write(dir.join("notes.md"), "x").unwrap();
    fs::create_dir(dir.join("projects")).unwrap();
    let other = tmp.path().join("other/agent-accounts").join(ID);
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("CLAUDE.md"), "x").unwrap();
    assert_eq!(
        setup_item(&root, &at("CLAUDE.md")).unwrap(),
        dunce::canonicalize(dir.join("CLAUDE.md")).unwrap()
    );
    for path in [
        at("../../secret.md"),
        at("../../CLAUDE.md"),
        at("rules/CLAUDE.md"),
        at("rules/a.md"),
        at("run.bat"),
        at(".credentials.json"),
        at("notes.md"),
        at("sessions/"),
        at("projects/"),
        root.join("agent-accounts").display().to_string(),
        root.join("agent-accounts/not-a-uuid/CLAUDE.md")
            .display()
            .to_string(),
        other.join("CLAUDE.md").display().to_string(),
    ] {
        assert_eq!(setup_item(&root, &path).unwrap_err(), REFUSED, "{path}");
    }
    assert_eq!(
        fs::read_to_string(dir.join(".credentials.json")).unwrap(),
        "secret"
    );
    for made in ["run.bat", "rules", "sessions"] {
        assert!(!dir.join(made).exists(), "{made}");
    }
}

#[test]
fn a_link_out_of_the_account_folder_or_to_another_item_in_it_is_refused() {
    let (tmp, root, dir) = data_dir();
    let at = |rest: &str| format!("{}/{rest}", dir.display());
    let elsewhere = tmp.path().join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    link_to_folder(&elsewhere, &dir.join("skills"));
    assert_eq!(setup_item(&root, &at("skills/")).unwrap_err(), REFUSED);
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
    #[cfg(unix)]
    {
        fs::write(root.join("secret.json"), "{}").unwrap();
        std::os::unix::fs::symlink(root.join("secret.json"), dir.join("settings.json")).unwrap();
        assert_eq!(
            setup_item(&root, &at("settings.json")).unwrap_err(),
            REFUSED
        );
        fs::write(dir.join(".credentials.json"), "secret").unwrap();
        std::os::unix::fs::symlink(dir.join(".credentials.json"), dir.join("CLAUDE.md")).unwrap();
        assert_eq!(setup_item(&root, &at("CLAUDE.md")).unwrap_err(), REFUSED);
    }
}

#[test]
fn a_missing_item_is_made_before_it_opens_and_an_existing_one_is_never_rewritten() {
    let (_tmp, root, dir) = data_dir();
    let at = |rest: &str| format!("{}/{rest}", dir.display());
    assert_eq!(
        fs::read(setup_item(&root, &at("CLAUDE.md")).unwrap()).unwrap(),
        b""
    );
    assert_eq!(
        fs::read_to_string(setup_item(&root, &at("settings.json")).unwrap()).unwrap(),
        "{}\n"
    );
    assert!(setup_item(&root, &at("skills/")).unwrap().is_dir());
    fs::write(dir.join(".claude.json"), r#"{"mcpServers":{}}"#).unwrap();
    assert_eq!(
        fs::read_to_string(setup_item(&root, &at(".claude.json")).unwrap()).unwrap(),
        r#"{"mcpServers":{}}"#
    );
    fs::write(dir.join("CLAUDE.md"), "keep").unwrap();
    setup_item(&root, &at("CLAUDE.md")).unwrap();
    assert_eq!(fs::read_to_string(dir.join("CLAUDE.md")).unwrap(), "keep");
}

#[test]
fn every_item_a_row_opens_opens_as_a_document_or_a_folder() {
    let (_tmp, root, dir) = data_dir();
    let items = account_setup_items();
    assert!(!items.is_empty());
    for open in items {
        let item = setup_item(&root, &format!("{}/{open}", dir.display())).unwrap();
        assert_eq!(item.is_dir(), open.ends_with('/'), "{open}");
    }
    assert_eq!(kind("CLAUDE.md"), Some(Kind::File));
    assert_eq!(kind("Settings.JSON"), Some(Kind::File));
    assert_eq!(kind("skills/"), Some(Kind::Folder));
    for program in ["run.bat", "setup.sh", "tool.exe", "noextension", "md"] {
        assert_eq!(kind(program), None, "{program}");
    }
}
