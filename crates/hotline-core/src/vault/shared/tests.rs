use super::*;
use crate::credentials::MemoryStore;

fn vault(root: &Path) -> Vault {
    Vault::open_with_store(root, Log::open(root), Arc::new(MemoryStore::default())).unwrap()
}

fn names(vault: &Vault) -> Vec<String> {
    vault
        .shared_secrets()
        .unwrap()
        .into_iter()
        .map(|secret| secret.name)
        .collect()
}

fn login(sites: &[&str], totp: Option<&str>) -> StoredSecret {
    StoredSecret::Login {
        r#use: SecretUse::Auto,
        sites: sites.iter().map(|site| (*site).to_owned()).collect(),
        username: "george".to_owned(),
        password: "correct horse battery".to_owned(),
        totp: totp.map(str::to_owned),
    }
}

pub(crate) const KEY_BASE64: &str = "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgeE7T2PDJPCRfPvTUFvVcQI7KFSDmnKbrRfEjRGbtV9WhRANCAATfeasaWHkMKJ4oCcdDzVX9c2xUUkC7Uiuqu8tS0LXtRJ8pCk+gNSvvqWaB3WgFNn4rvQ8wS1bH+dOjfgZoq2gz";

pub(crate) fn passkey(rp_id: &str) -> StoredSecret {
    StoredSecret::Passkey {
        r#use: SecretUse::Auto,
        rp_id: rp_id.to_owned(),
        credential_id: "AQID".to_owned(),
        private_key: KEY_BASE64.to_owned(),
        user_handle: Some("dGVhbW1hdGUtMQ==".to_owned()),
        user_name: Some("teammate".to_owned()),
        user_display_name: None,
    }
}

#[test]
fn a_shared_secret_is_listed_by_name_and_never_by_value() {
    let root = tempfile::tempdir().unwrap();
    let vault = vault(root.path());
    assert!(names(&vault).is_empty(), "nothing stored yet");

    vault
        .set_shared_secret("GITHUB_TOKEN", "ghp_notarealtoken0001")
        .unwrap();
    let listed = vault.shared_secrets().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "GITHUB_TOKEN");
    assert_eq!(listed[0].kind, SharedSecretKind::Variable);
    assert!(listed[0].updated_at > 0);

    // The disk holds a reference; the value is in the store alone.
    let on_disk = fs::read_to_string(root.path().join("vault/shared/GITHUB_TOKEN.json")).unwrap();
    assert!(!on_disk.contains("ghp_notarealtoken0001"), "{on_disk}");
    assert!(on_disk.contains("hotlineCredential"), "{on_disk}");
    assert!(
        !root
            .path()
            .join("vault/shared/GITHUB_TOKEN.about.json")
            .exists()
    );
    let room = fs::read_to_string(root.path().join("room.jsonl")).unwrap_or_default();
    assert!(!room.contains("GITHUB_TOKEN"), "no room event: {room}");

    let (values, missing) = vault
        .shared_secret_values(&["GITHUB_TOKEN".to_string(), "NPM_TOKEN".to_string()])
        .unwrap();
    assert_eq!(
        values["GITHUB_TOKEN"],
        StoredSecret::Variable {
            value: "ghp_notarealtoken0001".to_owned()
        }
    );
    assert_eq!(missing, ["NPM_TOKEN"]);
}

#[test]
fn a_login_and_a_passkey_are_listed_by_what_they_are_for_and_never_by_value() {
    let root = tempfile::tempdir().unwrap();
    let vault = vault(root.path());
    vault
        .set_shared(
            "GITHUB",
            StoredSecret::Login {
                r#use: SecretUse::Auto,
                sites: vec![
                    "https://GitHub.com/".to_owned(),
                    " http://localhost:8123 ".to_owned(),
                    String::new(),
                ],
                username: " george ".to_owned(),
                password: "correct horse battery".to_owned(),
                totp: Some("gezd gnbv-gy3t qojq gezd gnbv gy3t qojq".to_owned()),
            },
        )
        .unwrap();
    vault
        .set_shared("GITHUB_PASSKEY", passkey("github.com"))
        .unwrap();
    vault
        .set_shared_secret("GITHUB_TOKEN", "ghp_notarealtoken0001")
        .unwrap();

    let listed = vault.shared_secrets().unwrap();
    let by_name = |name: &str| listed.iter().find(|one| one.name == name).unwrap().clone();
    let github = by_name("GITHUB");
    assert_eq!(github.kind, SharedSecretKind::Login);
    assert_eq!(
        github.sites.as_deref(),
        Some(
            &[
                "https://github.com".to_owned(),
                "http://localhost:8123".to_owned()
            ][..]
        )
    );
    assert_eq!(github.username.as_deref(), Some("george"));
    assert_eq!(github.totp, Some(true));
    let passkey = by_name("GITHUB_PASSKEY");
    assert_eq!(passkey.kind, SharedSecretKind::Passkey);
    assert_eq!(passkey.rp_id.as_deref(), Some("github.com"));
    assert_eq!(passkey.user_name.as_deref(), Some("teammate"));
    assert_eq!(by_name("GITHUB_TOKEN").kind, SharedSecretKind::Variable);
    let listing = serde_json::to_string(&listed).unwrap();
    assert!(!listing.contains("battery"), "{listing}");
    assert!(!listing.contains("GEZDGNBV"), "{listing}");
    assert!(!listing.contains(KEY_BASE64), "{listing}");

    // The sidecar says what the listing says, and nothing more.
    let about = fs::read_to_string(root.path().join("vault/shared/GITHUB.about.json")).unwrap();
    assert!(about.contains("\"username\":\"george\""), "{about}");
    assert!(!about.contains("battery"), "{about}");
    assert!(!about.contains("GEZDGNBV"), "{about}");
    let about =
        fs::read_to_string(root.path().join("vault/shared/GITHUB_PASSKEY.about.json")).unwrap();
    assert!(!about.contains(KEY_BASE64), "{about}");

    // What is handed to a computer is the whole record, in one spelling.
    let (values, missing) = vault
        .shared_secret_values(&["GITHUB".to_string(), "GITHUB_PASSKEY".to_string()])
        .unwrap();
    assert_eq!(missing, [] as [String; 0]);
    assert_eq!(
        values["GITHUB"],
        StoredSecret::Login {
            r#use: SecretUse::Auto,
            sites: vec![
                "https://github.com".to_owned(),
                "http://localhost:8123".to_owned()
            ],
            username: "george".to_owned(),
            password: "correct horse battery".to_owned(),
            totp: Some("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ".to_owned()),
        }
    );
    assert_eq!(
        values["GITHUB_PASSKEY"],
        super::tests::passkey("github.com")
    );

    // Replacing a login with a variable takes its sidecar away.
    vault
        .set_shared_secret("GITHUB", "now-a-variable-value")
        .unwrap();
    assert_eq!(
        by_name("GITHUB").kind,
        SharedSecretKind::Login,
        "the earlier listing"
    );
    assert_eq!(
        vault
            .shared_secrets()
            .unwrap()
            .iter()
            .find(|one| one.name == "GITHUB")
            .unwrap()
            .kind,
        SharedSecretKind::Variable
    );
    assert!(!root.path().join("vault/shared/GITHUB.about.json").exists());
    vault.delete_shared_secret("GITHUB_PASSKEY").unwrap();
    assert!(
        !root
            .path()
            .join("vault/shared/GITHUB_PASSKEY.about.json")
            .exists()
    );
    assert_eq!(names(&vault), ["GITHUB", "GITHUB_TOKEN"]);
}

#[test]
fn a_login_needs_a_site_of_its_own_and_a_passkey_a_key() {
    let root = tempfile::tempdir().unwrap();
    let vault = vault(root.path());
    for (what, secret, expect) in [
        ("no site", login(&[], None), "at least one site"),
        (
            "a site with a path",
            login(&["https://github.com/login"], None),
            "nothing after the host",
        ),
        (
            "plain http",
            login(&["http://github.com"], None),
            "plain http",
        ),
        (
            "no scheme",
            login(&["github.com"], None),
            "an origin such as",
        ),
        (
            "a short seed",
            login(&["https://github.com"], Some("GEZDGNBV")),
            "at least 16",
        ),
        (
            "a seed that is not base32",
            login(&["https://github.com"], Some("GEZDGNBVGY3TQOJQ1890")),
            "base32",
        ),
        ("a passkey on no site", passkey(""), "host name"),
        (
            "a passkey on an address",
            passkey("https://github.com"),
            "host name",
        ),
    ] {
        let refused = vault.set_shared("LOGIN", secret).unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::InvalidInput, "{what}");
        assert!(refused.to_string().contains(expect), "{what}: {refused}");
    }
    let short = StoredSecret::Login {
        r#use: SecretUse::Auto,
        sites: vec!["https://github.com".to_owned()],
        username: "george".to_owned(),
        password: "hunter2".to_owned(),
        totp: None,
    };
    let refused = vault.set_shared("LOGIN", short).unwrap_err();
    assert!(
        refused.to_string().contains("LOGIN's password"),
        "{refused}"
    );
    let nobody = StoredSecret::Login {
        r#use: SecretUse::Auto,
        sites: vec!["https://github.com".to_owned()],
        username: "  ".to_owned(),
        password: "correct horse battery".to_owned(),
        totp: None,
    };
    assert!(vault.set_shared("LOGIN", nobody).is_err());
    let StoredSecret::Passkey {
        rp_id,
        credential_id,
        user_handle,
        user_name,
        user_display_name,
        ..
    } = passkey("github.com")
    else {
        unreachable!("passkey() builds a passkey")
    };
    let keyless = StoredSecret::Passkey {
        r#use: SecretUse::Auto,
        rp_id,
        credential_id,
        private_key: "AQID".to_owned(),
        user_handle,
        user_name,
        user_display_name,
    };
    assert!(
        vault
            .set_shared("LOGIN", keyless)
            .unwrap_err()
            .to_string()
            .contains("too short")
    );
    assert!(names(&vault).is_empty(), "nothing refused was stored");
    for site in [
        "https://github.com",
        "https://GitHub.com:443/",
        "https://[::1]:8443",
        "http://localhost:8123",
        "http://app.localhost",
    ] {
        check_site(site).unwrap_or_else(|error| panic!("{site}: {error}"));
    }
    assert_eq!(
        check_site("https://GitHub.com:443/").unwrap(),
        "https://github.com"
    );
    assert_eq!(
        check_site("https://[::1]:8443").unwrap(),
        "https://[::1]:8443"
    );
    for rp in ["github.com", "localhost", "login.example.co.uk"] {
        check_rp_id(rp).unwrap_or_else(|error| panic!("{rp}: {error}"));
    }
    for rp in [
        "",
        "GitHub.com",
        "github.com/",
        "github.com:443",
        ".github.com",
        "a..b",
    ] {
        assert!(check_rp_id(rp).is_err(), "{rp:?}");
    }
}

#[test]
fn a_name_is_an_environment_variable_and_hotlines_own_are_refused() {
    for good in ["GITHUB_TOKEN", "A", "X1_2_3", "NPM_TOKEN_2026"] {
        check_secret_name(good).unwrap_or_else(|error| panic!("{good}: {error}"));
    }
    for bad in [
        "",
        "github_token",
        "Github_Token",
        "1TOKEN",
        "_TOKEN",
        "GITHUB-TOKEN",
        "GITHUB TOKEN",
        "TOKEN=1",
        "../ESCAPE",
        "HOTLINE_COMPUTER_TOKEN",
        "HOTLINE_ANYTHING",
        "PATH",
        "HOME",
        "LD_PRELOAD",
    ] {
        assert!(check_secret_name(bad).is_err(), "{bad:?} is not a name");
    }
    let root = tempfile::tempdir().unwrap();
    let vault = vault(root.path());
    assert!(vault.set_shared_secret("PATH", "/usr/bin:/bin").is_err());
    assert!(vault.delete_shared_secret("../x").is_err());
    assert_eq!(names(&vault), [] as [String; 0]);
}

#[test]
fn a_value_is_at_least_eight_characters() {
    let root = tempfile::tempdir().unwrap();
    let vault = vault(root.path());
    let refused = vault.set_shared_secret("PIN", "1234").unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::InvalidInput);
    assert!(refused.to_string().contains("at least 8"), "{refused}");
    assert!(vault.set_shared_secret("NUL", "abc\0defgh").is_err());
    assert!(
        vault
            .set_shared_secret("KEY", "-----BEGIN KEY-----\nline\n-----END KEY-----\n")
            .is_ok(),
        "a multi-line value, such as a private key, is a value"
    );
    // A variable that happens to look like a record is still a variable.
    vault
        .set_shared_secret(
            "JSON",
            r#"{"kind":"login","sites":[],"username":"","password":""}"#,
        )
        .unwrap();
    let (values, _) = vault.shared_secret_values(&["JSON".to_string()]).unwrap();
    assert!(
        matches!(&values["JSON"], StoredSecret::Login { .. }),
        "a value that parses as a record is taken as one; the desk never stores such a variable through the window, which types it"
    );
}

#[test]
fn replacing_keeps_one_record_and_deleting_takes_it() {
    let root = tempfile::tempdir().unwrap();
    let vault = vault(root.path());
    vault
        .set_shared_secret("NPM_TOKEN", "npm_first_value")
        .unwrap();
    vault
        .set_shared_secret("NPM_TOKEN", "npm_second_value")
        .unwrap();
    assert_eq!(names(&vault), ["NPM_TOKEN"]);
    let (values, _) = vault
        .shared_secret_values(&["NPM_TOKEN".to_string()])
        .unwrap();
    assert_eq!(
        values["NPM_TOKEN"],
        StoredSecret::Variable {
            value: "npm_second_value".to_owned()
        }
    );

    vault.delete_shared_secret("NPM_TOKEN").unwrap();
    assert_eq!(names(&vault), [] as [String; 0]);
    assert!(!root.path().join("vault/shared/NPM_TOKEN.json").exists());
    let (values, missing) = vault
        .shared_secret_values(&["NPM_TOKEN".to_string()])
        .unwrap();
    assert!(values.is_empty());
    assert_eq!(missing, ["NPM_TOKEN"]);
    assert_eq!(
        vault.delete_shared_secret("NPM_TOKEN").unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
}

#[cfg(unix)]
#[test]
fn the_shared_directory_and_its_records_are_private_and_a_planted_link_is_not_a_secret() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let vault = vault(root.path());
    vault
        .set_shared_secret("GITHUB_TOKEN", "ghp_notarealtoken0001")
        .unwrap();
    vault
        .set_shared("GITHUB", login(&["https://github.com"], None))
        .unwrap();
    let mode = |path: PathBuf| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(vault.shared_dir()), 0o700);
    assert_eq!(mode(vault.shared_dir().join("GITHUB_TOKEN.json")), 0o600);
    assert_eq!(mode(vault.shared_dir().join("GITHUB.about.json")), 0o600);

    // A link where a record should be is refused rather than followed,
    // and it is not listed as a secret either.
    let elsewhere = root.path().join("elsewhere.json");
    fs::write(&elsewhere, "{}").unwrap();
    std::os::unix::fs::symlink(&elsewhere, vault.shared_dir().join("PLANTED.json")).unwrap();
    std::os::unix::fs::symlink(
        &elsewhere,
        vault.shared_dir().join("GITHUB_TOKEN.about.json"),
    )
    .unwrap();
    assert_eq!(names(&vault), ["GITHUB", "GITHUB_TOKEN"]);
    assert_eq!(
        vault.shared_secrets().unwrap()[1].kind,
        SharedSecretKind::Variable,
        "a planted sidecar is not read"
    );
    assert!(
        vault
            .shared_secret_values(&["PLANTED".to_string()])
            .is_err(),
        "a planted link is refused, not read"
    );
}
#[test]
fn secret_use_changes_survive_restart_without_rewriting_values() {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::default());
    let vault = Vault::open_with_store(root.path(), Log::open(root.path()), store.clone()).unwrap();
    vault
        .set_shared("LOGIN", login(&["https://github.com"], None))
        .unwrap();
    vault.set_shared("PASSKEY", passkey("github.com")).unwrap();
    for name in ["LOGIN", "PASSKEY"] {
        let bytes = fs::read(vault.shared_path(name)).unwrap();
        vault.set_secret_use(name, SecretUse::Ask).unwrap();
        assert_eq!(fs::read(vault.shared_path(name)).unwrap(), bytes);
    }
    drop(vault);
    let reopened = Vault::open_with_store(root.path(), Log::open(root.path()), store).unwrap();
    assert!(
        reopened
            .shared_secrets()
            .unwrap()
            .iter()
            .all(|s| s.r#use == SecretUse::Ask)
    );
    let (values, _) = reopened
        .shared_secret_values(&["LOGIN".into(), "PASSKEY".into()])
        .unwrap();
    let wire = crate::computer::secrets::wire_form(&values);
    assert_eq!(wire["LOGIN"]["use"], "ask");
    assert_eq!(wire["PASSKEY"]["use"], "ask");
    reopened.set_secret_use("PASSKEY", SecretUse::Auto).unwrap();
    assert_eq!(reopened.shared_secrets().unwrap()[1].r#use, SecretUse::Auto);
    reopened
        .set_shared_secret("TOKEN", "long-enough-token")
        .unwrap();
    assert!(reopened.set_secret_use("TOKEN", SecretUse::Ask).is_err());
}

#[test]
fn keys_stored_before_use_policy_are_automatic() {
    let root = tempfile::tempdir().unwrap();
    let vault = vault(root.path());
    for (name, secret) in [
        ("LOGIN", login(&["https://github.com"], None)),
        ("PASSKEY", passkey("github.com")),
    ] {
        vault.set_shared(name, secret.clone()).unwrap();
        let mut about: serde_json::Value =
            serde_json::from_slice(&fs::read(vault.about_path(name)).unwrap()).unwrap();
        about.as_object_mut().unwrap().remove("use");
        crate::credentials::atomic_write(
            &vault.about_path(name),
            &serde_json::to_vec(&about).unwrap(),
        )
        .unwrap();
        let mut value = serde_json::to_value(secret).unwrap();
        value.as_object_mut().unwrap().remove("use");
        vault
            .files
            .file(vault.shared_path(name))
            .write(&serde_json::to_vec(&value).unwrap())
            .unwrap();
    }
    vault
        .set_shared_secret("TOKEN", "long-enough-token")
        .unwrap();
    assert!(
        vault
            .shared_secrets()
            .unwrap()
            .iter()
            .all(|s| s.r#use == SecretUse::Auto)
    );
    let (values, _) = vault
        .shared_secret_values(&["LOGIN".into(), "PASSKEY".into(), "TOKEN".into()])
        .unwrap();
    let wire = crate::computer::secrets::wire_form(&values);
    assert_eq!(wire["LOGIN"]["use"], "auto");
    assert_eq!(wire["PASSKEY"]["use"], "auto");
    assert_eq!(wire["TOKEN"], "long-enough-token");
}
