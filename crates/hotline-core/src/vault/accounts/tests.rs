use super::*;
use crate::credentials::MemoryStore;
use crate::vault::tests::{ids, listed, vault};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct CountingStore {
    secrets: MemoryStore,
    reads: AtomicUsize,
}

impl SecretStore for CountingStore {
    fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.secrets.get(key)
    }
    fn set(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        self.secrets.set(key, bytes)
    }
    fn delete(&self, key: &str) -> io::Result<()> {
        self.secrets.delete(key)
    }
}

fn key(secret: &str) -> CredentialChange {
    CredentialChange {
        label: None,
        secret: Some(secret.into()),
        base_url: None,
    }
}

fn server(base_url: &str, ids: &[&str]) -> CredentialChange {
    CredentialChange {
        label: None,
        secret: None,
        base_url: Some((
            base_url.into(),
            ids.iter().map(ToString::to_string).collect(),
        )),
    }
}

fn label(label: &str) -> CredentialChange {
    CredentialChange {
        label: Some(label.into()),
        secret: None,
        base_url: None,
    }
}

fn chosen(id: &str, provider_id: &str) -> ChosenAccount {
    ChosenAccount {
        id: id.into(),
        provider_id: provider_id.into(),
    }
}

fn signed_out() -> ProviderAuth {
    ProviderAuth::Unavailable(ACCOUNT_SIGNED_OUT.into())
}

fn forget_secret(vault: &Vault, id: &str) {
    let mut secrets = vault.read_secrets().unwrap();
    secrets.remove(id);
    vault.write_secrets(&secrets).unwrap();
}

fn tokens(vault: &Vault, id: &str) -> Option<Vec<u8>> {
    vault.login_tokens(id).read().unwrap()
}

fn signed_in(vault: &Vault, provider_id: &str, tokens: &[u8]) -> (Credential, PathBuf) {
    let (id, dir) = vault.begin_login().unwrap();
    vault.login_tokens(&id).write(tokens).unwrap();
    let credential = vault.finish_login(&id, provider_id, provider_id).unwrap();
    (credential, dir)
}

#[test]
fn the_default_is_the_first_usable_credential_and_skips_a_revoked_or_empty_one() {
    let vault = vault("accounts-default");
    let first = vault.create("anthropic", "First", "key-first").unwrap();
    let second = vault.create("anthropic", "Second", "key-second").unwrap();
    let third = vault.create("anthropic", "Third", "key-third").unwrap();
    let openai = vault.create("openai", "OpenAI", "key-openai").unwrap();
    assert_eq!(vault.default_ids()["anthropic"], first.id);
    assert_eq!(vault.default_ids()["openai"], openai.id);

    vault.revoke(&first.id).unwrap();
    forget_secret(&vault, &second.id);
    assert_eq!(vault.default_ids()["anthropic"], third.id);
    assert_eq!(
        vault.provider_auth()["anthropic"],
        ProviderAuth::ApiKey("key-third".into())
    );

    vault.revoke(&third.id).unwrap();
    assert!(!vault.default_ids().contains_key("anthropic"));
}

#[test]
fn a_chosen_account_replaces_only_its_providers_entry() {
    let vault = vault("accounts-chosen");
    vault.create("anthropic", "A", "key-a").unwrap();
    let b = vault.create("anthropic", "B", "key-b").unwrap();
    vault.create("openai", "C", "key-c").unwrap();

    let auth = vault.provider_auth_for(Some(&chosen(&b.id, "anthropic")));
    assert_eq!(auth["anthropic"], ProviderAuth::ApiKey("key-b".into()));
    assert_eq!(auth["openai"], ProviderAuth::ApiKey("key-c".into()));
    assert_eq!(auth.len(), 2);
    assert_eq!(vault.provider_auth_for(None), vault.provider_auth());
}

#[test]
fn a_chosen_accounts_turn_reads_the_keychain_once() {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(CountingStore::default());
    let vault = Vault::open_with_store(root.path(), Log::open(root.path()), store.clone()).unwrap();
    vault.create("anthropic", "A", "key-a").unwrap();
    let b = vault.create("anthropic", "B", "key-b").unwrap();
    vault.create("openai", "C", "key-c").unwrap();

    store.reads.store(0, Ordering::SeqCst);
    let auth = vault.provider_auth_for(Some(&chosen(&b.id, "anthropic")));
    assert_eq!(auth["anthropic"], ProviderAuth::ApiKey("key-b".into()));
    assert_eq!(store.reads.load(Ordering::SeqCst), 1);
}

#[test]
fn the_default_account_says_whether_its_provider_has_another_live_one() {
    let vault = vault("accounts-default-account");
    let a = vault.create("anthropic", "A", "key-a").unwrap();
    let b = vault.create("anthropic", "B", "key-b").unwrap();
    let c = vault.create("openai", "C", "key-c").unwrap();
    let d = vault.create("openai", "D", "key-d").unwrap();
    vault.revoke(&d.id).unwrap();

    assert_eq!(
        vault.default_account("anthropic"),
        Some((a.id.clone(), true))
    );
    assert_eq!(vault.default_account("openai"), Some((c.id, false)));
    assert_eq!(vault.default_account("xai"), None);
    vault.revoke(&a.id).unwrap();
    assert_eq!(vault.default_account("anthropic"), Some((b.id, false)));
}

#[test]
fn a_chosen_account_that_is_revoked_or_gone_never_becomes_another() {
    let vault = vault("accounts-signed-out");
    vault.create("anthropic", "A", "key-a").unwrap();
    let revoked = vault.create("anthropic", "B", "key-b").unwrap();
    let empty = vault.create("anthropic", "C", "key-c").unwrap();
    let openai = vault.create("openai", "D", "key-d").unwrap();
    vault.revoke(&revoked.id).unwrap();
    forget_secret(&vault, &empty.id);

    for account in [
        chosen(&revoked.id, "anthropic"),
        chosen(&empty.id, "anthropic"),
        chosen("no-such-credential", "anthropic"),
        chosen(&openai.id, "anthropic"),
    ] {
        let auth = vault.provider_auth_for(Some(&account));
        assert_eq!(auth["anthropic"], signed_out(), "{}", account.id);
        assert_eq!(auth["openai"], ProviderAuth::ApiKey("key-d".into()));
    }
    assert_eq!(
        vault.provider_auth()["anthropic"],
        ProviderAuth::ApiKey("key-a".into())
    );

    // A store that cannot be read says why, as it does for the default.
    fs::write(vault.secrets_path(), "not a reference").unwrap();
    let ProviderAuth::Unavailable(why) =
        &vault.provider_auth_for(Some(&chosen(&revoked.id, "anthropic")))["anthropic"]
    else {
        panic!("a revoked account is never a key");
    };
    assert_eq!(why, ACCOUNT_SIGNED_OUT);
    let auth = vault.provider_auth_for(Some(&chosen(&empty.id, "anthropic")));
    let ProviderAuth::Unavailable(why) = &auth["anthropic"] else {
        panic!("an unreadable store is never a key");
    };
    assert_ne!(why, ACCOUNT_SIGNED_OUT);
    assert_eq!(auth["anthropic"], vault.provider_auth()["anthropic"]);
}

#[test]
fn changing_a_key_or_a_server_keeps_its_id_its_place_and_the_default() {
    let vault = vault("accounts-update");
    let a = vault.create("anthropic", "A", "key-old").unwrap();
    let b = vault.create("anthropic", "B", "key-b").unwrap();
    let order = ids(&vault);

    let changed = vault.update(&a.id, key("key-new")).unwrap();
    assert_eq!(
        (&changed.id, changed.created_at, changed.revoked),
        (&a.id, a.created_at, false)
    );
    assert!(changed.updated_at >= a.updated_at);
    assert_eq!(vault.default_ids()["anthropic"], a.id);
    assert_eq!(ids(&vault), order);
    assert_eq!(
        vault.provider_auth()["anthropic"],
        ProviderAuth::ApiKey("key-new".into())
    );

    let renamed = vault.update(&b.id, label("Work")).unwrap();
    assert_eq!(
        (renamed.id.as_str(), renamed.label.as_str()),
        (b.id.as_str(), "Work")
    );
    assert_eq!(vault.find(&b.id).unwrap().label, "Work");
    assert_eq!(ids(&vault), order);

    let local = vault
        .connect_local(
            "http://127.0.0.1:11434",
            &["llama-old".into()],
            "Ollama Local",
        )
        .unwrap();
    vault
        .cache_discovery(&local.id, &[listed("from-the-old-server")])
        .unwrap();
    assert_eq!(vault.account_models()["ollama"], ["from-the-old-server"]);
    let moved = vault
        .update(&local.id, server("http://10.0.0.2:11434", &["qwen-new"]))
        .unwrap();
    assert_eq!(moved.id, local.id);
    assert_eq!(moved.base_url.as_deref(), Some("http://10.0.0.2:11434"));
    assert_eq!(
        vault.provider_auth()["ollama"],
        ProviderAuth::Local {
            base_url: "http://10.0.0.2:11434".into()
        }
    );
    assert_eq!(vault.account_models()["ollama"], ["qwen-new"]);
}

#[test]
fn a_change_that_does_not_fit_the_account_is_refused() {
    let vault = vault("accounts-update-refused");
    let api_key = vault.create("anthropic", "A", "key-a").unwrap();
    let local = vault
        .connect_local("http://127.0.0.1:11434", &["llama".into()], "Ollama Local")
        .unwrap();
    let (login, _) = signed_in(&vault, "openai-codex", b"tokens");
    let custom = vault
        .save_custom(
            None,
            crate::contract::CustomProviderDraft {
                name: "Box".into(),
                base_url: "http://127.0.0.1:9000/v1".into(),
                api: crate::contract::OpenAiApi::ChatCompletions,
                models: vec!["box-model".into()],
                secret: None,
            },
        )
        .unwrap();
    let before = vault.list();

    for (id, change) in [
        (&api_key.id, server("http://10.0.0.2:11434", &["m"])),
        (&local.id, key("key-x")),
        (&login.id, key("key-x")),
        (&login.id, server("http://10.0.0.2:11434", &["m"])),
        (&custom.id, server("http://10.0.0.2:9000/v1", &["m"])),
        (&custom.id, key("key-x")),
        (
            &api_key.id,
            CredentialChange {
                label: None,
                secret: None,
                base_url: None,
            },
        ),
    ] {
        let refused = vault.update(id, change).unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::InvalidInput, "{id}");
    }
    assert_eq!(vault.list(), before);
    assert_eq!(
        vault.provider_auth()["anthropic"],
        ProviderAuth::ApiKey("key-a".into())
    );

    let both = vault
        .update(
            &api_key.id,
            CredentialChange {
                label: Some("Work".into()),
                secret: Some("key-b".into()),
                base_url: None,
            },
        )
        .unwrap();
    assert_eq!(both.label, "Work");
    assert_eq!(
        vault.provider_auth()["anthropic"],
        ProviderAuth::ApiKey("key-b".into())
    );
}

#[test]
fn a_sign_in_adopted_in_place_keeps_its_id_and_a_failed_one_leaves_the_old_tokens() {
    let vault = vault("accounts-adopt");
    let (into, into_dir) = signed_in(&vault, "openai-codex", b"old");
    write_beside_login(&into_dir, "endpoints.json", b"{}\n").unwrap();
    vault
        .cache_discovery(&into.id, &[listed("old-account-model")])
        .unwrap();

    // A sign-in that is cancelled is abandoned, never adopted.
    let (cancelled, _) = vault.begin_login().unwrap();
    vault.login_tokens(&cancelled).write(b"new").unwrap();
    vault.abandon_login(&cancelled).unwrap();
    assert_eq!(tokens(&vault, &into.id).as_deref(), Some(&b"old"[..]));

    let (from, from_dir) = vault.begin_login().unwrap();
    vault.login_tokens(&from).write(b"new").unwrap();
    write_account_models(&from_dir, &["new-account-model".into()]).unwrap();
    let adopted = vault.adopt_login(&from, &into.id).unwrap();

    assert_eq!(
        (&adopted.id, adopted.created_at, &adopted.label),
        (&into.id, into.created_at, &into.label)
    );
    assert!(adopted.updated_at >= into.updated_at);
    assert_eq!(tokens(&vault, &into.id).as_deref(), Some(&b"new"[..]));
    assert!(!from_dir.exists());
    assert_eq!(ids(&vault), [into.id]);
    assert!(!into_dir.join("endpoints.json").exists());
    assert!(!into_dir.join("discovery.json").exists());
    assert_eq!(
        vault.account_models()["openai-codex"],
        ["new-account-model"]
    );
    assert_eq!(
        vault.provider_auth()["openai-codex"],
        ProviderAuth::Login {
            token_dir: into_dir
        }
    );
}

#[test]
fn a_copilot_sign_in_adopted_in_place_leaves_exactly_its_own_records() {
    let vault = vault("accounts-adopt-copilot");
    let (into, into_dir) = signed_in(&vault, "github-copilot", b"stray");
    let into_copilot = vault.files.file(into_dir.join(COPILOT_RECORD));
    into_copilot.write(b"old-copilot").unwrap();
    write_beside_login(&into_dir, "endpoints.json", b"old-endpoints").unwrap();

    let (from, from_dir) = vault.begin_login().unwrap();
    vault
        .files
        .file(from_dir.join(COPILOT_RECORD))
        .write(b"new-copilot")
        .unwrap();
    write_beside_login(&from_dir, "endpoints.json", b"new-endpoints").unwrap();
    vault.adopt_login(&from, &into.id).unwrap();

    assert_eq!(
        into_copilot.read().unwrap().as_deref(),
        Some(&b"new-copilot"[..])
    );
    assert_eq!(tokens(&vault, &into.id), None);
    assert!(!into_dir.join(rig_login::CHATGPT_RECORD).exists());
    assert_eq!(
        fs::read(into_dir.join("endpoints.json")).unwrap(),
        b"new-endpoints"
    );
    assert!(!from_dir.exists());
}

#[test]
fn a_sign_in_that_holds_no_record_is_refused_and_leaves_the_tokens() {
    let vault = vault("accounts-adopt-refused");
    let (into, _) = signed_in(&vault, "openai-codex", b"old");
    let (copilot, _) = signed_in(&vault, "github-copilot", b"unused");
    let (revoked, _) = signed_in(&vault, "openai-codex", b"revoked");
    vault.revoke(&revoked.id).unwrap();
    let api_key = vault.create("openai-codex", "Key", "key-a").unwrap();

    let (empty, empty_dir) = vault.begin_login().unwrap();
    let (fresh, fresh_dir) = vault.begin_login().unwrap();
    vault.login_tokens(&fresh).write(b"new").unwrap();

    for (from, target) in [
        (&empty, &into.id),
        (&fresh, &copilot.id),
        (&fresh, &revoked.id),
        (&fresh, &api_key.id),
        (&copilot.id, &into.id),
        (&into.id, &into.id),
    ] {
        let refused = vault.adopt_login(from, target).unwrap_err();
        assert_eq!(
            refused.kind(),
            io::ErrorKind::InvalidInput,
            "{from} into {target}"
        );
    }
    assert_eq!(tokens(&vault, &into.id).as_deref(), Some(&b"old"[..]));
    assert!(empty_dir.is_dir() && fresh_dir.is_dir());
    assert!(vault.login_dir(&copilot.id).is_dir());
}

#[test]
fn a_blank_name_takes_the_next_free_one() {
    let vault = vault("accounts-free-label");
    vault.create("anthropic", "Anthropic", "key-a").unwrap();
    vault.create("anthropic", "Anthropic", "key-b").unwrap();
    assert_eq!(vault.free_label("anthropic", "Anthropic"), "Anthropic 2");

    vault.create("anthropic", "anthropic 2", "key-c").unwrap();
    assert_eq!(vault.free_label("anthropic", "Anthropic"), "Anthropic 3");
    assert_eq!(vault.free_label("openai", "OpenAI"), "OpenAI");

    let spare = vault.create("anthropic", "Spare", "key-d").unwrap();
    vault.revoke(&spare.id).unwrap();
    assert_eq!(vault.free_label("anthropic", "Spare"), "Spare");
}

#[test]
fn a_name_is_unique_per_provider_and_section_and_counted_in_characters() {
    let vault = vault("accounts-check-label");
    let work = vault.create("anthropic", "Work", "key-a").unwrap();
    assert_eq!(
        vault.check_label("anthropic", AccountSection::ApiKey, "work", None),
        Err("Anthropic already has an account named work.".into())
    );
    assert_eq!(
        vault.check_label(
            "anthropic",
            AccountSection::ApiKey,
            "  Work  ",
            Some(&work.id)
        ),
        Ok("Work".into())
    );
    assert_eq!(
        vault.check_label("openai", AccountSection::ApiKey, "work", None),
        Ok("work".into())
    );

    let local = vault
        .connect_local("http://127.0.0.1:11434", &["llama".into()], "Ollama Local")
        .unwrap();
    vault.update(&local.id, label("Work")).unwrap();
    assert_eq!(
        vault.check_label("ollama", AccountSection::ApiKey, "work", None),
        Ok("work".into())
    );
    assert_eq!(
        vault.check_label("ollama", AccountSection::Server, "work", None),
        Err("Ollama Local already has an account named work.".into())
    );

    let (subscription, _) = signed_in(&vault, "xai", b"tokens");
    vault.update(&subscription.id, label("Work")).unwrap();
    assert_eq!(
        vault.check_label("xai", AccountSection::ApiKey, "Work", None),
        Ok("Work".into())
    );
    assert!(
        vault
            .check_label("xai", AccountSection::Subscription, "Work", None)
            .is_err()
    );
    vault.create("xai", "Work", "key-x").unwrap();
    assert!(
        vault
            .check_label("xai", AccountSection::ApiKey, "work", None)
            .is_err()
    );

    vault.revoke(&work.id).unwrap();
    assert_eq!(
        vault.check_label("anthropic", AccountSection::ApiKey, "work", None),
        Ok("work".into())
    );

    let too_long = "Give the account a name of 1 to 64 characters.";
    for (name, accepted) in [
        ("🙂".repeat(64), true),
        ("🙂".repeat(65), false),
        ("   ".into(), false),
        (String::new(), false),
    ] {
        let checked = vault.check_label("anthropic", AccountSection::ApiKey, &name, None);
        assert_eq!(
            checked.is_ok(),
            accepted,
            "{} characters",
            name.chars().count()
        );
        if !accepted {
            assert_eq!(checked, Err(too_long.into()));
        }
    }
}

#[test]
fn sections_follow_what_the_account_bills() {
    for (provider_id, kind, section) in [
        ("ollama", CredentialKind::Local, AccountSection::Server),
        ("openrouter", CredentialKind::Oauth, AccountSection::ApiKey),
        (
            "openai-codex",
            CredentialKind::Oauth,
            AccountSection::Subscription,
        ),
        ("xai", CredentialKind::Oauth, AccountSection::Subscription),
        ("anthropic", CredentialKind::ApiKey, AccountSection::ApiKey),
    ] {
        assert_eq!(
            Vault::section_of(provider_id, kind),
            section,
            "{provider_id}"
        );
    }
}
