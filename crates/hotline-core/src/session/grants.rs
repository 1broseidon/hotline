//! A teammate's secret grants: each name it is given, readable or
//! protected, and where.
//!
//! `secrets` (the names) and `secretGrants` (how each is given) are written
//! together. A client from before grants had options writes `secrets`
//! alone; what it ticks is today's grant, so a plain variable it adds is
//! readable. A client that knows them writes `secretGrants`, and `secrets`
//! follows. At upgrade every granted variable becomes `readable: true`, so
//! it keeps working where it did and the pane says what it was all along.
//!
//! A teammate that runs as the person — whole-machine reach, or an ACP
//! harness — can read the desk's data and reach the container runtime, its
//! own computer included, so nothing is protected from it: every key it is
//! granted is readable, and says so.

use super::*;
use crate::contract::{GrantPlace, SecretGrant, SharedSecretKind};

/// A variable with no "Used with" can only be read: there is nowhere a
/// computer could add it for its programs without handing it over.
fn plain_variable(name: &str, stored: &[SharedSecret]) -> bool {
    stored.iter().any(|secret| {
        secret.name == name
            && secret.kind == SharedSecretKind::Variable
            && secret.used_with.is_none()
    })
}

/// Whether the teammate's programs run as the person: Hotline Agent with
/// the whole machine, or any ACP harness.
pub(crate) fn runs_as_the_person(persona: &Persona) -> bool {
    persona.backend_id != crate::driver::HOTLINE_BACKEND_ID
        || persona.reach == Some(crate::contract::Reach::Machine)
}

fn variable(name: &str, stored: &[SharedSecret]) -> bool {
    stored
        .iter()
        .any(|secret| secret.name == name && secret.kind == SharedSecretKind::Variable)
}

/// The grant a name gets when nobody said how: readable for a plain
/// variable, and for any variable to a teammate that runs as the person;
/// protected and on its computer for anything else.
fn default_grant(persona: &Persona, name: &str, stored: &[SharedSecret]) -> SecretGrant {
    SecretGrant {
        name: name.to_owned(),
        readable: plain_variable(name, stored)
            || (runs_as_the_person(persona) && variable(name, stored)),
        on: None,
    }
}

/// The grants for the names in `secrets`: the entry each already has, or
/// its default, in name order. A name taken out of `secrets` loses its
/// entry with it.
pub(crate) fn settled(persona: &Persona, stored: &[SharedSecret]) -> Option<Vec<SecretGrant>> {
    let names = persona.secrets.as_ref()?;
    let kept = persona.secret_grants.as_deref().unwrap_or_default();
    let mut grants: Vec<SecretGrant> = names
        .iter()
        .map(|name| {
            kept.iter()
                .find(|grant| grant.name == *name)
                .cloned()
                .unwrap_or_else(|| default_grant(persona, name, stored))
        })
        .collect();
    grants.sort_by(|left, right| left.name.cmp(&right.name));
    grants.dedup_by(|left, right| left.name == right.name);
    Some(grants)
}

/// Grants as a client sent them, checked, with `secrets` written to match.
/// A plain variable is given only as readable, and so is every variable to
/// a teammate that runs as the person; only a variable can be: a login or a
/// passkey is used, never read, wherever it can be protected.
pub(crate) fn apply(
    persona: &mut Persona,
    grants: Vec<SecretGrant>,
    stored: &[SharedSecret],
) -> Result<(), String> {
    let mut grants = grants;
    grants.sort_by(|left, right| left.name.cmp(&right.name));
    for pair in grants.windows(2) {
        if pair[0].name == pair[1].name {
            return Err(format!("{} is granted twice.", pair[0].name));
        }
    }
    for grant in &mut grants {
        if let Some(on) = &mut grant.on {
            on.sort_by_key(|place| *place == GrantPlace::Desk);
            on.dedup();
            if on.is_empty() {
                return Err(format!(
                    "{} is granted nowhere; untick it instead.",
                    grant.name
                ));
            }
        }
        let kind = stored
            .iter()
            .find(|secret| secret.name == grant.name)
            .map(|secret| secret.kind);
        if !grant.readable && plain_variable(&grant.name, stored) {
            return Err(format!(
                "{} has no Used with, so it can only be given as readable by this teammate's programs.",
                grant.name
            ));
        }
        if !grant.readable && runs_as_the_person(persona) && variable(&grant.name, stored) {
            return Err(format!(
                "{} runs as you, so {} can only be given as readable by its programs.",
                persona.name, grant.name
            ));
        }
        if grant.readable && kind.is_some_and(|kind| kind != SharedSecretKind::Variable) {
            return Err(format!(
                "{} is a login or passkey: it is used for the teammate, never readable by it.",
                grant.name
            ));
        }
    }
    if grants.is_empty() {
        persona.secrets = None;
        persona.secret_grants = None;
    } else {
        persona.secrets = Some(grants.iter().map(|grant| grant.name.clone()).collect());
        persona.secret_grants = Some(grants);
    }
    Ok(())
}

impl Room {
    /// At start, every teammate's grants are settled against what the vault
    /// lists: a record from before grants had options gets one per name,
    /// readable for each plain variable, so it is handed over as it was. A
    /// vault that cannot be listed changes nothing; a variable must not lose
    /// its grant to a keychain that did not answer.
    pub(super) fn settle_secret_grants(&self) {
        let Some(stored) = self
            .vault
            .as_ref()
            .and_then(|vault| vault.shared_secrets().ok())
        else {
            return;
        };
        for mut persona in room::roster(&self.log) {
            let grants = settled(&persona, &stored);
            if grants == persona.secret_grants {
                continue;
            }
            persona.secret_grants = grants;
            if let Err(error) = room::append_persona(&self.log, &persona) {
                eprintln!(
                    "{}'s secret grants could not be settled: {error}",
                    persona.name
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{SecretUse, VariableUse};

    fn stored(name: &str, kind: SharedSecretKind, bound: bool) -> SharedSecret {
        SharedSecret {
            name: name.into(),
            updated_at: 1,
            kind,
            r#use: SecretUse::Auto,
            sites: None,
            username: None,
            totp: None,
            rp_id: None,
            user_name: None,
            used_with: bound.then(|| VariableUse {
                service: Some("github".into()),
                origins: vec!["https://api.github.com".into()],
                header: "Authorization".into(),
                schemes: vec!["Bearer".into()],
                refuse: Vec::new(),
            }),
        }
    }

    fn vault_list() -> Vec<SharedSecret> {
        vec![
            stored("GITHUB", SharedSecretKind::Login, false),
            stored("GITHUB_TOKEN", SharedSecretKind::Variable, true),
            stored("NPM_TOKEN", SharedSecretKind::Variable, false),
        ]
    }

    fn grant(name: &str, readable: bool) -> SecretGrant {
        SecretGrant {
            name: name.into(),
            readable,
            on: None,
        }
    }

    #[tokio::test]
    async fn existing_variable_grants_become_readable_at_upgrade() {
        let log = crate::session::tests::scratch("grants-upgrade");
        let vault = Arc::new(
            Vault::open_with_store(
                log.root(),
                log.clone(),
                Arc::new(crate::credentials::MemoryStore::default()),
            )
            .unwrap(),
        );
        vault
            .set_shared_secret("NPM_TOKEN", "npm_notarealtoken01")
            .unwrap();
        vault
            .set_shared(
                "GITHUB",
                crate::vault::StoredSecret::Login {
                    r#use: SecretUse::Auto,
                    sites: vec!["https://github.com".into()],
                    username: "george".into(),
                    password: "correct horse battery".into(),
                    totp: None,
                },
            )
            .unwrap();
        // A record as 0.49 wrote it: names only.
        let mut ada = crate::session::tests::persona("ada");
        ada.secrets = Some(vec!["GITHUB".into(), "NPM_TOKEN".into()]);
        crate::session::tests::enrol(&log, &ada);

        let room = Room::with_agents_computers_and_vault(
            log.clone(),
            Arc::new(crate::session::tests::DeskKeys),
            crate::session::tests::Fake::new(crate::session::tests::Scripted::new(Vec::new())),
            crate::computer::Computer::with_path(log.root().join("no-runtime").as_os_str()),
            Some(vault),
        );
        let ada = room.persona("ada").unwrap();
        assert_eq!(
            ada.secret_grants,
            Some(vec![grant("GITHUB", false), grant("NPM_TOKEN", true)]),
            "the variable is readable, as it always was; the login is not readable"
        );
        assert_eq!(ada.secrets, Some(vec!["GITHUB".into(), "NPM_TOKEN".into()]));
    }

    /// A Hotline Agent teammate confined to its workspace.
    fn confined() -> Persona {
        let mut ada = crate::session::tests::persona("ada");
        ada.reach = Some(crate::contract::Reach::Workspace);
        ada
    }

    #[test]
    fn a_name_ticked_by_an_older_client_gets_todays_grant_and_keeps_the_rest() {
        let mut ada = confined();
        ada.secrets = Some(vec!["GITHUB_TOKEN".into(), "NPM_TOKEN".into()]);
        ada.secret_grants = Some(vec![SecretGrant {
            name: "GITHUB_TOKEN".into(),
            readable: false,
            on: Some(vec![GrantPlace::Computer]),
        }]);
        let grants = settled(&ada, &vault_list()).unwrap();
        assert_eq!(grants[0].on, Some(vec![GrantPlace::Computer]), "kept");
        assert_eq!(
            grants[1],
            grant("NPM_TOKEN", true),
            "a plain variable reads"
        );
        ada.secrets = Some(vec!["NPM_TOKEN".into()]);
        assert_eq!(
            settled(&ada, &vault_list()).unwrap(),
            [grant("NPM_TOKEN", true)]
        );
    }

    #[test]
    fn a_whole_machine_teammate_never_gets_a_protected_key() {
        let mut ada = crate::session::tests::persona("ada");
        ada.reach = Some(crate::contract::Reach::Machine);
        let refused =
            apply(&mut ada, vec![grant("GITHUB_TOKEN", false)], &vault_list()).unwrap_err();
        assert!(refused.contains("readable"), "{refused}");
        apply(&mut ada, vec![grant("GITHUB_TOKEN", true)], &vault_list()).unwrap();
        assert_eq!(ada.secret_grants, Some(vec![grant("GITHUB_TOKEN", true)]));
        // An older client's tick of a key Used with a service is readable too.
        ada.secrets = Some(vec!["GITHUB_TOKEN".into(), "GITHUB".into()]);
        ada.secret_grants = None;
        assert_eq!(
            settled(&ada, &vault_list()).unwrap(),
            [grant("GITHUB", false), grant("GITHUB_TOKEN", true)],
            "a login is still used for it, as before"
        );
        // A harness runs as the person whatever its reach.
        let mut harness = crate::session::tests::persona("bob");
        harness.backend_id = "claude-code".into();
        assert!(
            apply(
                &mut harness,
                vec![grant("GITHUB_TOKEN", false)],
                &vault_list()
            )
            .is_err()
        );
        assert!(!runs_as_the_person(&confined()));
    }

    #[test]
    fn a_plain_variable_is_granted_only_as_readable_and_a_login_never_is() {
        let mut ada = confined();
        let refused = apply(&mut ada, vec![grant("NPM_TOKEN", false)], &vault_list()).unwrap_err();
        assert!(refused.contains("readable"), "{refused}");
        assert!(apply(&mut ada, vec![grant("GITHUB", true)], &vault_list()).is_err());
        let nowhere = SecretGrant {
            on: Some(Vec::new()),
            ..grant("GITHUB_TOKEN", false)
        };
        assert!(apply(&mut ada, vec![nowhere], &vault_list()).is_err());
        assert_eq!(ada.secrets, None, "a refusal changes nothing");

        apply(
            &mut ada,
            vec![
                grant("NPM_TOKEN", true),
                grant("GITHUB_TOKEN", false),
                grant("GITHUB", false),
            ],
            &vault_list(),
        )
        .unwrap();
        assert_eq!(
            ada.secrets,
            Some(vec![
                "GITHUB".into(),
                "GITHUB_TOKEN".into(),
                "NPM_TOKEN".into()
            ]),
            "the names follow the grants, for an older client"
        );
        apply(&mut ada, Vec::new(), &vault_list()).unwrap();
        assert_eq!((ada.secrets, ada.secret_grants), (None, None));
    }
}
