//! A teammate's computer: secrets, passkeys, cookies, and its release.

use super::*;

/// The passkey cards on a teammate's tape, oldest first, each as its
/// latest word.
fn passkey_cards(room: &Room, persona_id: &str) -> Vec<Value> {
    tape(room, persona_id)
        .into_iter()
        .filter(|event| event["kind"] == "passkey_ask")
        .collect()
}

/// Grants `names` to ada on the room's record, the way `persona.update`
/// from a client that sends names would, without reattaching anything: a
/// plain variable readable, anything else protected.
fn grant_secrets(room: &Room, names: &[&str]) {
    let mut ada = room.persona("ada").unwrap();
    ada.secrets = Some(names.iter().map(ToString::to_string).collect());
    ada.secret_grants = crate::session::grants::settled(&ada, &room.stored_secrets());
    enrol(&room.log, &ada);
}

/// The secrets a teammate is granted are read from the vault and handed to
/// its computer at start, whole; a name that is not stored any more is said
/// on the tape rather than silently dropped; and the preamble names what
/// the computer has, never a value.
#[tokio::test]
async fn a_computers_granted_secrets_are_handed_to_it_at_start_by_name_and_never_seen() {
    let ComputerRoom {
        room,
        agents,
        taken,
        ..
    } = computer_room("computer-secrets", Some("0.11.0"), TWO_RELEASES, true).await;
    let vault = room.vault.as_ref().unwrap();
    vault
        .set_shared_secret("GITHUB_TOKEN", "ghp_notarealtoken0001")
        .unwrap();
    vault
        .set_shared_secret("NPM_TOKEN", "npm_notarealtoken0002")
        .unwrap();
    grant_secrets(&room, &["GITHUB_TOKEN", "GONE_TOKEN"]);
    room.start("ada").await.unwrap();

    // Only what was granted, and only what is stored: NPM_TOKEN was never
    // ticked, GONE_TOKEN is not there to give.
    let sets = taken.sets();
    assert_eq!(sets.len(), 1, "{sets:?}");
    // A variable travels as its bare value, the form every release with a
    // secrets door takes.
    assert_eq!(
        sets[0],
        std::collections::BTreeMap::from([(
            "GITHUB_TOKEN".to_string(),
            serde_json::json!("ghp_notarealtoken0001")
        )])
    );
    let told = notices(&room, "ada");
    assert!(told.is_empty(), "handing secrets is not narrated: {told:?}");
    // Today's computer, handed only readable variables, is as it was.
    assert_eq!(
        room.computer_status("ada").await.unwrap().protection,
        Some(crate::contract::ComputerProtection::Legacy)
    );

    // The agent hears the names and that it will never see a value; the
    // value itself is on no tape and in no preamble.
    let heard = lock(&agents.preambles)[0].clone();
    assert!(
        heard.contains("by name: GITHUB_TOKEN (a variable), GONE_TOKEN (not stored right now)."),
        "{heard}"
    );
    assert!(heard.contains("The computer redacts values"), "{heard}");
    assert!(!heard.contains("ghp_notarealtoken0001"), "{heard}");
    let on_tape = serde_json::to_string(&tape(&room, "ada")).unwrap();
    assert!(!on_tape.contains("ghp_notarealtoken0001"), "{on_tape}");
    let on_room = std::fs::read_to_string(room.log.root().join("room.jsonl")).unwrap();
    assert!(!on_room.contains("ghp_notarealtoken0001"), "{on_room}");
}

/// A stored value that is replaced or deleted reaches every running
/// computer as a whole new set, so a rotation lands and a revocation
/// leaves nothing behind, without anyone restarting the teammate.
#[tokio::test]
async fn a_changed_secret_is_handed_again_to_every_running_computer() {
    let ComputerRoom { room, taken, .. } = computer_room(
        "computer-secrets-changed",
        Some("0.11.0"),
        TWO_RELEASES,
        true,
    )
    .await;
    let vault = room.vault.as_ref().unwrap();
    vault
        .set_shared_secret("GITHUB_TOKEN", "ghp_notarealtoken0001")
        .unwrap();
    grant_secrets(&room, &["GITHUB_TOKEN"]);
    room.start("ada").await.unwrap();
    assert_eq!(taken.sets().len(), 1);

    vault
        .set_shared_secret("GITHUB_TOKEN", "ghp_rotatedtoken00002")
        .unwrap();
    room.secrets_changed().await;
    let sets = taken.sets();
    assert_eq!(sets.len(), 2, "{sets:?}");
    assert_eq!(sets[1]["GITHUB_TOKEN"], "ghp_rotatedtoken00002");

    vault.delete_shared_secret("GITHUB_TOKEN").unwrap();
    room.secrets_changed().await;
    let sets = taken.sets();
    assert_eq!(sets.len(), 3, "{sets:?}");
    assert!(
        sets[2].is_empty(),
        "a deleted secret is gone from the machine"
    );
    assert_eq!(notices(&room, "ada"), [] as [String; 0]);

    // A stopped computer is not chased: it gets the set at its next start.
    room.stop("ada").unwrap();
    room.computer_stop("ada").await.unwrap();
    vault
        .set_shared_secret("GITHUB_TOKEN", "ghp_afterstop0000003")
        .unwrap();
    room.secrets_changed().await;
    assert_eq!(taken.sets().len(), 3, "nothing handed to a stopped machine");
}

/// An image from before secrets answers 404. Granted nothing or something,
/// the conversation says nothing about it: the teammate still starts.
#[tokio::test]
async fn a_computer_from_before_secrets_leaves_the_conversation_alone() {
    let ComputerRoom { room, .. } =
        computer_room("computer-secrets-old-quiet", None, TWO_RELEASES, true).await;
    room.start("ada").await.unwrap();
    assert!(
        !notices(&room, "ada")
            .iter()
            .any(|text| text.contains("secrets")),
        "{:?}",
        notices(&room, "ada")
    );

    let ComputerRoom { room, .. } =
        computer_room("computer-secrets-old-told", None, TWO_RELEASES, true).await;
    room.vault
        .as_ref()
        .unwrap()
        .set_shared_secret("GITHUB_TOKEN", "ghp_notarealtoken0001")
        .unwrap();
    grant_secrets(&room, &["GITHUB_TOKEN"]);
    room.start("ada").await.unwrap();
    let told = notices(&room, "ada");
    assert!(told.is_empty(), "{told:?}");
    assert!(room.session("ada").is_ok());
}

/// A passkey is made under an arming and nowhere else: the desk arms one
/// teammate's computer for one site, polls, and the poll that finds the
/// credential minted stores it, ticks it for that teammate, hands the
/// computer the set with it, and ends the arming. The private key crosses
/// once, computer to vault, and is on no tape and in no room event. A
/// cancel ends an arming with nothing stored, and a release from before
/// passkeys is told apart and named.
#[tokio::test]
async fn a_passkey_is_made_under_an_arming_stored_and_ticked_for_the_teammate() {
    use crate::computer::guide::fake::KEY_BASE64;
    use crate::contract::{PasskeyRegistrationState, SharedSecretKind};

    let ComputerRoom { room, taken, .. } =
        computer_room("computer-passkey", Some("0.13.0"), TWO_RELEASES, true).await;
    let vault = room.vault.as_ref().unwrap();

    // Nothing armed: idle, and nothing to cancel.
    let idle = room.secrets_passkey_registration("ada").await.unwrap();
    assert_eq!(idle.state, PasskeyRegistrationState::Idle);
    room.secrets_passkey_cancel("ada").await.unwrap();

    // A site that is not one, and a name that is not one, are refused
    // before the computer is touched.
    assert!(
        room.secrets_passkey_register("GITHUB_PASSKEY", "ada", "GitHub.com")
            .await
            .is_err()
    );
    assert!(
        room.secrets_passkey_register("github passkey", "ada", "github.com")
            .await
            .is_err()
    );
    assert_eq!(taken.armed(), None);

    // Arming starts the computer, the same as opening its screen would.
    let armed = room
        .secrets_passkey_register("GITHUB_PASSKEY", "ada", "github.com")
        .await
        .unwrap();
    assert_eq!(armed.state, PasskeyRegistrationState::Armed);
    assert_eq!(armed.name.as_deref(), Some("GITHUB_PASSKEY"));
    assert_eq!(armed.rp_id.as_deref(), Some("github.com"));
    assert!(armed.expires_at.is_some());
    assert_eq!(taken.armed().as_deref(), Some("github.com"));
    assert_eq!(
        room.computer_status("ada").await.unwrap().state,
        crate::contract::ComputerState::Running
    );
    let still = room.secrets_passkey_registration("ada").await.unwrap();
    assert_eq!(still.state, PasskeyRegistrationState::Armed);
    assert!(
        vault.shared_secrets().unwrap().is_empty(),
        "nothing stored yet"
    );

    // The person presses "add a passkey" on the site: the request waits
    // in the browser, and the look that finds it raises the card on ada's
    // tape with what the site asked for. Nothing is stored while it waits.
    let first = taken.ask();
    let asked = registration_until(&room, "ada", PasskeyRegistrationState::Asked).await;
    let ask = asked.ask.expect("the site's request");
    assert_eq!(ask.id, first);
    assert_eq!(ask.user_name.as_deref(), Some("teammate"));
    let cards = passkey_cards(&room, "ada");
    assert_eq!(cards.len(), 1, "{cards:?}");
    assert_eq!(cards[0]["status"], "pending");
    assert_eq!(cards[0]["askId"], first);
    assert_eq!(cards[0]["name"], "GITHUB_PASSKEY");
    assert_eq!(cards[0]["rpId"], "github.com");
    assert_eq!(cards[0]["origin"], "https://github.com");
    assert_eq!(cards[0]["rpName"], "The site");
    assert_eq!(cards[0]["userName"], "teammate");
    assert_eq!(cards[0]["userDisplayName"], "The teammate");
    assert!(
        vault.shared_secrets().unwrap().is_empty(),
        "nothing is stored while the card waits"
    );
    assert_eq!(taken.answered(), None);
    // An answer names the request; a stale card cannot let one through.
    assert!(
        room.secrets_passkey_answer("ada", "ask-9", true)
            .await
            .is_err()
    );
    assert_eq!(taken.answered(), None);
    // Approved on the tape, the browser makes it: the look that finds it
    // made — this poll's, or the room's own — stores it, and the poll is
    // told. One answer to one request.
    let approved = room
        .secrets_passkey_answer("ada", &first, true)
        .await
        .unwrap();
    assert_eq!(approved.state, PasskeyRegistrationState::Approved);
    assert_eq!(taken.answered(), Some(true));
    assert_eq!(passkey_cards(&room, "ada")[0]["status"], "approved");
    assert!(
        room.secrets_passkey_answer("ada", &first, true)
            .await
            .is_err()
    );
    taken.mint();
    let stored = registration_until(&room, "ada", PasskeyRegistrationState::Stored).await;
    let secret = stored.secret.expect("the record, as listed");
    let cards = passkey_cards(&room, "ada");
    assert_eq!(cards.len(), 1, "the card is one card: {cards:?}");
    assert_eq!(cards[0]["status"], "approved");
    assert_eq!(secret.name, "GITHUB_PASSKEY");
    assert_eq!(secret.kind, SharedSecretKind::Passkey);
    assert_eq!(secret.rp_id.as_deref(), Some("github.com"));
    assert_eq!(secret.user_name.as_deref(), Some("teammate"));
    let listed = vault.shared_secrets().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].kind, SharedSecretKind::Passkey);

    // Ticked for ada, handed to the computer whole, and the arming ended.
    let ada = room.persona("ada").unwrap();
    assert_eq!(
        ada.secrets.as_deref(),
        Some(&["GITHUB_PASSKEY".to_string()][..])
    );
    let sets = taken.sets();
    let handed = &sets.last().expect("handed after storing")["GITHUB_PASSKEY"];
    assert_eq!(handed["kind"], "passkey");
    assert_eq!(handed["rpId"], "github.com");
    assert_eq!(handed["privateKey"], KEY_BASE64);
    assert_eq!(taken.armed(), None);
    assert_eq!(
        room.secrets_passkey_registration("ada")
            .await
            .unwrap()
            .state,
        PasskeyRegistrationState::Idle
    );

    // The private key is in the vault and nowhere the model or a log reads.
    let on_tape = serde_json::to_string(&tape(&room, "ada")).unwrap();
    assert!(!on_tape.contains(KEY_BASE64), "{on_tape}");
    let on_room = std::fs::read_to_string(room.log.root().join("room.jsonl")).unwrap();
    assert!(!on_room.contains(KEY_BASE64), "{on_room}");
    assert!(!on_room.contains("AQID"), "{on_room}");

    // A cancel ends an arming with nothing stored, and a card nobody
    // answered expires with it.
    room.secrets_passkey_register("GITLAB_PASSKEY", "ada", "gitlab.com")
        .await
        .unwrap();
    assert_eq!(taken.armed().as_deref(), Some("gitlab.com"));
    taken.ask();
    registration_until(&room, "ada", PasskeyRegistrationState::Asked).await;
    room.secrets_passkey_cancel("ada").await.unwrap();
    assert_eq!(taken.armed(), None);
    assert_eq!(
        room.secrets_passkey_registration("ada")
            .await
            .unwrap()
            .state,
        PasskeyRegistrationState::Idle
    );
    assert_eq!(vault.shared_secrets().unwrap().len(), 1);
    let cards = passkey_cards(&room, "ada");
    assert_eq!(cards.len(), 2, "{cards:?}");
    assert_eq!(cards[1]["name"], "GITLAB_PASSKEY");
    assert_eq!(cards[1]["status"], "expired");

    // A release from before passkeys has no door for one, and is named.
    let ComputerRoom { room, taken, .. } =
        computer_room("computer-passkey-old", Some("0.7.0"), TWO_RELEASES, true).await;
    let refused = room
        .secrets_passkey_register("GITHUB_PASSKEY", "ada", "github.com")
        .await
        .unwrap_err();
    assert!(refused.contains("cannot make passkeys"), "{refused}");
    assert_eq!(taken.armed(), None);
}

/// A login is stored as one record and handed to the computer as one,
/// kind first, beside a variable's bare value; the preamble names each by
/// what it is, and the password, like the token, is on no tape.
#[tokio::test]
async fn a_login_is_handed_to_the_computer_as_a_record_and_named_by_its_sites() {
    let ComputerRoom {
        room,
        agents,
        taken,
        ..
    } = computer_room("computer-login", Some("0.13.0"), TWO_RELEASES, true).await;
    let vault = room.vault.as_ref().unwrap();
    vault
        .set_shared_secret("GITHUB_TOKEN", "ghp_notarealtoken0001")
        .unwrap();
    vault
        .set_shared(
            "GITHUB_LOGIN",
            crate::vault::StoredSecret::Login {
                r#use: crate::contract::SecretUse::Auto,
                sites: vec!["https://github.com".to_string()],
                username: "george".to_string(),
                password: "correct-horse-battery".to_string(),
                totp: None,
            },
        )
        .unwrap();
    grant_secrets(&room, &["GITHUB_LOGIN", "GITHUB_TOKEN"]);
    room.start("ada").await.unwrap();

    let sets = taken.sets();
    assert_eq!(sets.len(), 1, "{sets:?}");
    assert_eq!(taken.forms(), [2], "a protected computer takes the v2 form");
    assert_eq!(
        sets[0]["GITHUB_TOKEN"],
        serde_json::json!({"kind": "variable", "value": "ghp_notarealtoken0001", "readable": true})
    );
    assert_eq!(
        sets[0]["GITHUB_LOGIN"],
        serde_json::json!({
            "kind": "login", "use": "auto", "sites": ["https://github.com"],
            "username": "george", "password": "correct-horse-battery",
        })
    );

    let heard = lock(&agents.preambles)[0].clone();
    assert!(
        heard.contains(
            "by name: GITHUB_LOGIN (a login for https://github.com), GITHUB_TOKEN (a variable)."
        ),
        "{heard}"
    );
    assert!(
        heard.contains("NAME.username, NAME.password, or NAME.code"),
        "{heard}"
    );
    assert!(!heard.contains("correct-horse-battery"), "{heard}");
    let on_tape = serde_json::to_string(&tape(&room, "ada")).unwrap();
    assert!(!on_tape.contains("correct-horse-battery"), "{on_tape}");
    let on_room = std::fs::read_to_string(room.log.root().join("room.jsonl")).unwrap();
    assert!(!on_room.contains("correct-horse-battery"), "{on_room}");
}

/// What the pane's old import brought over reads as shared cookies, a row
/// a site, and Remove names that site's exact domains to the computer; a
/// release from before the door is named with the pane's Update and the
/// row stays.
#[tokio::test]
async fn an_old_import_is_a_row_a_site_and_remove_takes_its_domains_back() {
    use crate::contract::{CookieImport, CookieSite};
    let ComputerRoom { room, taken, .. } = computer_room(
        "computer-cookies-forget",
        Some("0.11.0"),
        TWO_RELEASES,
        true,
    )
    .await;
    assert_eq!(
        room.cookie_lends("ada"),
        [] as [crate::contract::CookieLend; 0]
    );
    let site = |domain: &str, cookies| CookieSite {
        domain: domain.into(),
        cookies,
    };
    let recorded = vec![CookieImport {
        browser_id: "chrome".to_string(),
        browser_name: "Google Chrome".to_string(),
        profile_id: "Default".to_string(),
        profile_name: "Default".to_string(),
        imported_at: 1,
        sites: vec![
            site("api.github.com", 1),
            site("github.com", 3),
            site("gitlab.com", 2),
        ],
    }];
    room::record_cookie_imports(&room.log, "ada", &recorded).unwrap();
    let rows = room.cookie_lends("ada");
    let sites: Vec<_> = rows.iter().map(|row| row.site.as_str()).collect();
    assert_eq!(sites, ["github.com", "gitlab.com"]);
    assert_eq!(rows[0].browser_name, "Google Chrome");
    assert!(!rows[0].waiting);

    room.start("ada").await.unwrap();
    room.cookie_lend_remove("ada", &rows[0].id).await.unwrap();
    assert_eq!(
        taken.forgotten(),
        vec![(
            "import-chrome-Default".to_string(),
            vec!["api.github.com".to_string(), "github.com".to_string()]
        )]
    );
    let left: Vec<_> = room
        .cookie_lends("ada")
        .into_iter()
        .map(|row| row.site)
        .collect();
    assert_eq!(left, ["gitlab.com"]);
    assert!(room.cookie_lend_remove("ada", &rows[0].id).await.is_err());

    // A release from before the door: refused, and the row stays.
    let ComputerRoom { room, taken, .. } = computer_room(
        "computer-cookies-forget-old",
        Some("0.8.0"),
        TWO_RELEASES,
        true,
    )
    .await;
    room::record_cookie_imports(&room.log, "ada", &recorded).unwrap();
    room.start("ada").await.unwrap();
    let id = room.cookie_lends("ada")[0].id.clone();
    let refused = room.cookie_lend_remove("ada", &id).await.unwrap_err();
    assert!(refused.contains("up to date"), "{refused}");
    assert_eq!(taken.forgotten(), [] as [(String, Vec<String>); 0]);
    assert_eq!(room.cookie_lends("ada").len(), 2, "nothing dropped");
}

/// Polls the registration until it answers `state`, since the room's own
/// watch may be storing the passkey at the moment a poll arrives.
async fn registration_until(
    room: &Arc<Room>,
    persona_id: &str,
    state: crate::contract::PasskeyRegistrationState,
) -> crate::contract::PasskeyRegistration {
    for _ in 0..100 {
        let answer = room.secrets_passkey_registration(persona_id).await.unwrap();
        if answer.state == state {
            return answer;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the registration never answered {state:?}");
}

/// The passkey is made from the teammate's screen, or by the teammate, and
/// neither keeps Settings → Secrets open: nothing polls at the moment the
/// browser mints it. The room stores it by itself, ticks it, hands it,
/// ends the arming, says so on the tape, and tells the pane once when it
/// next asks.
#[tokio::test]
async fn a_passkey_made_while_no_pane_is_looking_is_stored_by_the_room() {
    use crate::contract::{PasskeyRegistrationState, SharedSecretKind};
    let ComputerRoom { room, taken, .. } = computer_room(
        "computer-passkey-unwatched",
        Some("0.13.0"),
        TWO_RELEASES,
        true,
    )
    .await;
    let vault = room.vault.as_ref().unwrap();
    room.secrets_passkey_register("GITHUB_PASSKEY", "ada", "github.com")
        .await
        .unwrap();
    assert_eq!(vault.shared_secrets().unwrap(), [] as [SharedSecret; 0]);
    // The site asks: the room's own watch raises the card.
    let ask = taken.ask();
    let mut cards = Vec::new();
    for _ in 0..100 {
        cards = passkey_cards(&room, "ada");
        if !cards.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        cards.len(),
        1,
        "the room raised the card with nothing polling"
    );
    assert_eq!(cards[0]["status"], "pending");
    assert_eq!(cards[0]["askId"], ask);
    // Answered, from whichever seat: the browser makes it, and the watch
    // stores it.
    room.secrets_passkey_answer("ada", &ask, true)
        .await
        .unwrap();
    taken.mint();
    let mut listed = Vec::new();
    for _ in 0..100 {
        listed = vault.shared_secrets().unwrap();
        if !listed.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(listed.len(), 1, "the room stored it with nothing polling");
    assert_eq!(listed[0].name, "GITHUB_PASSKEY");
    assert_eq!(listed[0].kind, SharedSecretKind::Passkey);
    // The pane, opened later, is told once; then nothing is pending.
    let answer = registration_until(&room, "ada", PasskeyRegistrationState::Stored).await;
    assert_eq!(
        answer.secret.map(|secret| secret.name).as_deref(),
        Some("GITHUB_PASSKEY")
    );
    assert_eq!(
        room.secrets_passkey_registration("ada")
            .await
            .unwrap()
            .state,
        PasskeyRegistrationState::Idle
    );
    // Ticked, handed whole, the arming ended, and said on the tape.
    assert_eq!(
        room.persona("ada").unwrap().secrets.as_deref(),
        Some(&["GITHUB_PASSKEY".to_string()][..])
    );
    let sets = taken.sets();
    assert_eq!(
        sets.last().expect("handed after storing")["GITHUB_PASSKEY"]["kind"],
        "passkey"
    );
    assert_eq!(taken.armed(), None);
    let said = notices(&room, "ada");
    assert!(
        said.iter().any(|text| text.contains("GITHUB_PASSKEY")
            && text.contains("github.com")
            && text.contains("ticked")),
        "{said:?}"
    );
}

/// Denied on the tape, the site hears no and the arming is over: nothing
/// is stored, the card says denied, and nothing answers twice. A request
/// that leaves with its page — the person navigated away — expires its
/// card, and the arming goes on for the next request.
#[tokio::test]
async fn a_passkey_request_denied_on_the_tape_ends_the_arming_and_a_lost_one_expires() {
    use crate::contract::PasskeyRegistrationState;
    let ComputerRoom { room, taken, .. } = computer_room(
        "computer-passkey-denied",
        Some("0.11.0"),
        TWO_RELEASES,
        true,
    )
    .await;
    let vault = room.vault.as_ref().unwrap();
    room.secrets_passkey_register("GITHUB_PASSKEY", "ada", "github.com")
        .await
        .unwrap();
    let first = taken.ask();
    registration_until(&room, "ada", PasskeyRegistrationState::Asked).await;
    let denied = room
        .secrets_passkey_answer("ada", &first, false)
        .await
        .unwrap();
    assert_eq!(denied.state, PasskeyRegistrationState::Idle);
    assert_eq!(taken.armed(), None);
    assert_eq!(vault.shared_secrets().unwrap(), [] as [SharedSecret; 0]);
    let cards = passkey_cards(&room, "ada");
    assert_eq!(cards.len(), 1, "{cards:?}");
    assert_eq!(cards[0]["status"], "denied");
    let refused = room
        .secrets_passkey_answer("ada", &first, true)
        .await
        .unwrap_err();
    assert!(
        refused.contains("No passkey request is waiting"),
        "{refused}"
    );
    assert_eq!(
        room.secrets_passkey_registration("ada")
            .await
            .unwrap()
            .state,
        PasskeyRegistrationState::Idle
    );
    assert!(
        room.persona("ada")
            .unwrap()
            .secrets
            .unwrap_or_default()
            .is_empty(),
        "nothing ticked"
    );

    // Lost with its page: the card expires, the arming stays.
    room.secrets_passkey_register("GITHUB_PASSKEY", "ada", "github.com")
        .await
        .unwrap();
    let second = taken.ask();
    registration_until(&room, "ada", PasskeyRegistrationState::Asked).await;
    taken.page_left();
    let armed = registration_until(&room, "ada", PasskeyRegistrationState::Armed).await;
    assert_eq!(armed.ask, None);
    let cards = passkey_cards(&room, "ada");
    assert_eq!(cards.len(), 2, "{cards:?}");
    assert_eq!(cards[1]["askId"], second);
    assert_eq!(cards[1]["status"], "expired");
    let refused = room
        .secrets_passkey_answer("ada", &second, true)
        .await
        .unwrap_err();
    assert!(refused.contains("not waiting any more"), "{refused}");
    // The next request is a new card.
    let third = taken.ask();
    registration_until(&room, "ada", PasskeyRegistrationState::Asked).await;
    let cards = passkey_cards(&room, "ada");
    assert_eq!(cards.len(), 3, "{cards:?}");
    assert_eq!(cards[2]["askId"], third);
    assert_eq!(cards[2]["status"], "pending");
    room.secrets_passkey_cancel("ada").await.unwrap();
    assert_eq!(passkey_cards(&room, "ada")[2]["status"], "expired");
}

/// Looks at a pane-armed passkey until the desk has let the arming go:
/// refused, or idle because the room's own watch refused it first.
async fn until_let_go(room: &Arc<Room>, persona_id: &str) {
    for _ in 0..100 {
        match room.secrets_passkey_registration(persona_id).await {
            Err(_) => return,
            Ok(answer) if answer.state == crate::contract::PasskeyRegistrationState::Idle => {
                return;
            }
            Ok(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    panic!("the arming was never let go");
}

/// Nothing of a pane-armed `GITHUB_PASSKEY` was kept: none stored, none
/// ticked, none handed back, and the computer's arming ended.
fn assert_nothing_kept(room: &Room, taken: &crate::computer::guide::fake::Taken) {
    assert_eq!(
        room.vault.as_ref().unwrap().shared_secrets().unwrap(),
        [] as [SharedSecret; 0]
    );
    assert!(
        room.persona("ada")
            .unwrap()
            .secrets
            .unwrap_or_default()
            .is_empty(),
        "nothing ticked"
    );
    assert!(
        !taken
            .sets()
            .iter()
            .any(|set| set.contains_key("GITHUB_PASSKEY")),
        "nothing handed back"
    );
    assert_eq!(taken.armed(), None, "the arming is ended");
}

/// A passkey armed from the pane is kept only past the card the person
/// answered here, and only for the arming the desk made. Whatever holds the
/// computer's bearer can answer the site's request on the computer itself,
/// or re-arm it for another site, and the computer then reports a passkey
/// made: none of that is stored, ticked or handed back, the arming is
/// ended, its card expires rather than reading approved, and the tape says
/// why. A release that mints without asking is not kept either.
#[tokio::test]
async fn a_pane_armed_passkey_without_approval_or_with_another_rp_is_not_stored() {
    // Approved on the computer, not on the tape.
    let ComputerRoom { room, taken, .. } = computer_room(
        "computer-passkey-approved-there",
        Some("0.11.0"),
        TWO_RELEASES,
        true,
    )
    .await;
    room.secrets_passkey_register("GITHUB_PASSKEY", "ada", "github.com")
        .await
        .unwrap();
    let ask = taken.ask();
    registration_until(
        &room,
        "ada",
        crate::contract::PasskeyRegistrationState::Asked,
    )
    .await;
    taken.approve_on_the_computer();
    taken.mint();
    until_let_go(&room, "ada").await;
    assert_nothing_kept(&room, &taken);
    let cards = passkey_cards(&room, "ada");
    assert_eq!(cards.len(), 1, "{cards:?}");
    assert_eq!(cards[0]["askId"], ask);
    assert_eq!(
        cards[0]["status"], "expired",
        "the card never read approved"
    );
    assert!(
        notices(&room, "ada")
            .iter()
            .any(|text| text.contains("not approved here")),
        "{:?}",
        notices(&room, "ada")
    );
    // The card cannot be answered after.
    assert!(
        room.secrets_passkey_answer("ada", &ask, true)
            .await
            .is_err()
    );

    // Re-armed on the computer for another site, asked and approved there.
    let ComputerRoom { room, taken, .. } = computer_room(
        "computer-passkey-rearmed",
        Some("0.11.0"),
        TWO_RELEASES,
        true,
    )
    .await;
    room.secrets_passkey_register("GITHUB_PASSKEY", "ada", "github.com")
        .await
        .unwrap();
    taken.rearm("evil.example");
    taken.ask();
    taken.approve_on_the_computer();
    taken.mint();
    until_let_go(&room, "ada").await;
    assert_nothing_kept(&room, &taken);
    assert!(
        notices(&room, "ada")
            .iter()
            .any(|text| text.contains("another site")),
        "{:?}",
        notices(&room, "ada")
    );

    // The person approves the card on the tape, then the computer is
    // re-armed for another site before the passkey is made.
    let ComputerRoom { room, taken, .. } = computer_room(
        "computer-passkey-rearmed-after",
        Some("0.11.0"),
        TWO_RELEASES,
        true,
    )
    .await;
    room.secrets_passkey_register("GITHUB_PASSKEY", "ada", "github.com")
        .await
        .unwrap();
    let ask = taken.ask();
    registration_until(
        &room,
        "ada",
        crate::contract::PasskeyRegistrationState::Asked,
    )
    .await;
    let _ = room.secrets_passkey_answer("ada", &ask, true).await;
    taken.rearm("evil.example");
    taken.ask();
    taken.approve_on_the_computer();
    taken.mint();
    until_let_go(&room, "ada").await;
    assert_nothing_kept(&room, &taken);

    // A release from before the card mints without asking.
    let ComputerRoom { room, taken, .. } = computer_room(
        "computer-passkey-unasked",
        Some("0.8.1"),
        TWO_RELEASES,
        true,
    )
    .await;
    room.secrets_passkey_register("GITHUB_PASSKEY", "ada", "github.com")
        .await
        .unwrap();
    taken.mint();
    until_let_go(&room, "ada").await;
    assert_nothing_kept(&room, &taken);
    assert!(
        notices(&room, "ada")
            .iter()
            .any(|text| text.contains("without asking") && text.contains("update it")),
        "{:?}",
        notices(&room, "ada")
    );
}

/// The person's answer is reserved while it is on its way: a look meanwhile
/// changes nothing, a second answer is refused, and an answer the computer
/// refuses leaves nothing approved here. So a passkey something on the
/// computer approved and had made while the answer was out is not stored,
/// the card never reads approved, and it expires when the arming ends.
#[tokio::test]
async fn an_answer_the_computer_refuses_while_a_look_is_out_approves_and_stores_nothing() {
    use crate::contract::PasskeyRegistrationState;
    let ComputerRoom { room, taken, .. } = computer_room(
        "computer-passkey-answer-refused",
        Some("0.11.0"),
        TWO_RELEASES,
        true,
    )
    .await;
    room.secrets_passkey_register("GITHUB_PASSKEY", "ada", "github.com")
        .await
        .unwrap();
    let ask = taken.ask();
    registration_until(&room, "ada", PasskeyRegistrationState::Asked).await;
    let held = taken.hold_answers();
    taken.refuse_answers();
    let answering = {
        let room = room.clone();
        let ask = ask.clone();
        tokio::spawn(async move { room.secrets_passkey_answer("ada", &ask, true).await })
    };
    for _ in 0..200 {
        if taken.answers_seen() > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(taken.answers_seen(), 1, "the answer is on its way");
    // Meanwhile something on the computer approves the request itself and
    // the browser makes the passkey.
    taken.approve_on_the_computer();
    taken.mint();
    let look = room.secrets_passkey_registration("ada").await.unwrap();
    assert_ne!(look.state, PasskeyRegistrationState::Stored);
    assert_eq!(passkey_cards(&room, "ada")[0]["status"], "pending");
    assert_eq!(
        room.vault.as_ref().unwrap().shared_secrets().unwrap(),
        [] as [SharedSecret; 0]
    );
    let again = room
        .secrets_passkey_answer("ada", &ask, true)
        .await
        .unwrap_err();
    assert!(again.contains("already on its way"), "{again}");
    drop(held);
    assert!(answering.await.unwrap().is_err(), "the computer refused it");
    until_let_go(&room, "ada").await;
    assert_nothing_kept(&room, &taken);
    let cards = passkey_cards(&room, "ada");
    assert_eq!(cards.len(), 1, "{cards:?}");
    assert_eq!(
        cards[0]["status"], "expired",
        "the card never read approved"
    );
}

/// A look that was out when the person cancelled and armed again answers
/// about the arming that is gone, and changes nothing about the one that
/// took its place: the new arming stands, on the desk and the computer,
/// and makes its passkey as any arming does.
#[tokio::test]
async fn a_look_begun_under_a_cancelled_arming_leaves_its_replacement_armed() {
    use crate::contract::PasskeyRegistrationState;
    let ComputerRoom { room, taken, .. } = computer_room(
        "computer-passkey-replaced",
        Some("0.11.0"),
        TWO_RELEASES,
        true,
    )
    .await;
    room.secrets_passkey_register("GITHUB_PASSKEY", "ada", "github.com")
        .await
        .unwrap();
    let held = taken.hold_polls();
    let before = taken.polls_seen();
    let looking = {
        let room = room.clone();
        tokio::spawn(async move { room.secrets_passkey_registration("ada").await })
    };
    for _ in 0..200 {
        if taken.polls_seen() > before {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(taken.polls_seen() > before, "the look is out");
    room.secrets_passkey_cancel("ada").await.unwrap();
    room.secrets_passkey_register("GITLAB_PASSKEY", "ada", "gitlab.com")
        .await
        .unwrap();
    drop(held);
    let _ = looking.await.unwrap();
    // The room's own watch looks again, and finds the new arming alone.
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    assert_eq!(taken.armed().as_deref(), Some("gitlab.com"));
    let now = room.secrets_passkey_registration("ada").await.unwrap();
    assert_eq!(now.state, PasskeyRegistrationState::Armed);
    assert_eq!(now.rp_id.as_deref(), Some("gitlab.com"));
    assert!(
        !notices(&room, "ada")
            .iter()
            .any(|text| text.contains("not approved here")),
        "{:?}",
        notices(&room, "ada")
    );
    let ask = taken.ask();
    registration_until(&room, "ada", PasskeyRegistrationState::Asked).await;
    room.secrets_passkey_answer("ada", &ask, true)
        .await
        .unwrap();
    taken.mint();
    let stored = registration_until(&room, "ada", PasskeyRegistrationState::Stored).await;
    assert_eq!(
        stored.secret.map(|secret| secret.name).as_deref(),
        Some("GITLAB_PASSKEY")
    );
}

/// The passkey kept is the one the card showed. Which request reaches the
/// card follows the site's family (the armed site, a name under it, or one
/// it is under), but once the person approved a request for one name, a
/// passkey made for another name under the same site is not kept, while one
/// for the name the card showed is.
#[tokio::test]
async fn a_passkey_for_another_name_under_the_site_than_the_card_showed_is_not_stored() {
    use crate::contract::PasskeyRegistrationState;
    let ComputerRoom { room, taken, .. } = computer_room(
        "computer-passkey-other-name",
        Some("0.11.0"),
        TWO_RELEASES,
        true,
    )
    .await;
    room.secrets_passkey_register("GITHUB_PASSKEY", "ada", "github.com")
        .await
        .unwrap();
    let ask = taken.ask_for("login.github.com");
    registration_until(&room, "ada", PasskeyRegistrationState::Asked).await;
    assert_eq!(passkey_cards(&room, "ada")[0]["rpId"], "login.github.com");
    room.secrets_passkey_answer("ada", &ask, true)
        .await
        .unwrap();
    taken.mint_for("gist.github.com");
    until_let_go(&room, "ada").await;
    assert_nothing_kept(&room, &taken);
    assert!(
        notices(&room, "ada")
            .iter()
            .any(|text| text.contains("not approved here")),
        "{:?}",
        notices(&room, "ada")
    );

    let ComputerRoom { room, taken, .. } = computer_room(
        "computer-passkey-same-name",
        Some("0.11.0"),
        TWO_RELEASES,
        true,
    )
    .await;
    room.secrets_passkey_register("GITHUB_PASSKEY", "ada", "github.com")
        .await
        .unwrap();
    let ask = taken.ask_for("login.github.com");
    registration_until(&room, "ada", PasskeyRegistrationState::Asked).await;
    room.secrets_passkey_answer("ada", &ask, true)
        .await
        .unwrap();
    taken.mint_for("login.github.com");
    let stored = registration_until(&room, "ada", PasskeyRegistrationState::Stored).await;
    assert_eq!(
        stored.secret.and_then(|secret| secret.rp_id).as_deref(),
        Some("login.github.com")
    );
}

/// The arming's ten minutes are the desk's, counted before the computer is
/// asked, whatever end the computer answers: a computer that says its
/// arming ends an hour later cannot have a passkey stored or granted after
/// the desk's ten minutes, while one made in time is stored as ever.
#[tokio::test]
async fn a_computer_that_says_its_arming_ends_later_cannot_have_a_passkey_kept_after_the_desks_ten_minutes()
 {
    use crate::contract::PasskeyRegistrationState;
    let ComputerRoom { room, taken, .. } = computer_room(
        "computer-passkey-desk-deadline",
        Some("0.11.0"),
        TWO_RELEASES,
        true,
    )
    .await;
    taken.arm_later(60 * 60_000);
    let before = room.passkey_now();
    let armed = room
        .secrets_passkey_register("GITHUB_PASSKEY", "ada", "github.com")
        .await
        .unwrap();
    let ends = armed.expires_at.expect("an end");
    assert!(
        ends >= before + 10 * 60_000 && ends < before + 11 * 60_000,
        "the desk's ten minutes, not the computer's hour: {}",
        ends - before
    );
    let ask = taken.ask();
    registration_until(&room, "ada", PasskeyRegistrationState::Asked).await;
    room.secrets_passkey_answer("ada", &ask, true)
        .await
        .unwrap();
    room.advance_passkey_clock(10 * 60_000 + 1_000);
    taken.mint();
    until_let_go(&room, "ada").await;
    assert_nothing_kept(&room, &taken);

    // In time, the same computer's passkey is stored.
    let ComputerRoom { room, taken, .. } = computer_room(
        "computer-passkey-desk-deadline-in-time",
        Some("0.11.0"),
        TWO_RELEASES,
        true,
    )
    .await;
    taken.arm_later(60 * 60_000);
    room.secrets_passkey_register("GITHUB_PASSKEY", "ada", "github.com")
        .await
        .unwrap();
    let ask = taken.ask();
    registration_until(&room, "ada", PasskeyRegistrationState::Asked).await;
    room.secrets_passkey_answer("ada", &ask, true)
        .await
        .unwrap();
    room.advance_passkey_clock(9 * 60_000);
    taken.mint();
    let stored = registration_until(&room, "ada", PasskeyRegistrationState::Stored).await;
    assert_eq!(
        stored.secret.map(|secret| secret.name).as_deref(),
        Some("GITHUB_PASSKEY")
    );
    assert_eq!(
        room.persona("ada").unwrap().secrets.as_deref(),
        Some(&["GITHUB_PASSKEY".to_string()][..])
    );
}

/// The command names the scripted runtime was given, in order.
fn runtime_commands(root: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(root.join("argv.log"))
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

/// A teammate's computer hands over the guide of the release it is actually
/// running, and that guide is the `hotline-computer` skill in the workspace:
/// on disk under Hotline's marker, in the catalog with its release, and in the
/// preamble as the file to read.
#[tokio::test]
async fn a_computers_guide_is_the_hotline_computer_skill_of_the_release_it_runs() {
    let ComputerRoom {
        room, agents, cwd, ..
    } = computer_room("computer-skill", Some("0.11.0"), TWO_RELEASES, true).await;
    room.start("ada").await.unwrap();

    let folder = cwd.join(".agents/skills/hotline-computer");
    assert_eq!(
        std::fs::read_to_string(folder.join("SKILL.md")).unwrap(),
        crate::computer::guide::fake::skill_of("0.11.0")
    );
    let marker = std::fs::read_to_string(folder.join(".managed-by-hotline")).unwrap();
    assert!(marker.starts_with("computer 0.11.0 "), "{marker}");

    let entry = crate::skills::computer_entry(&cwd, None).expect("listed");
    assert_eq!(entry.source, crate::contract::SkillSource::Computer);
    assert_eq!(entry.name, "hotline-computer");
    assert_eq!(entry.version.as_deref(), Some("0.11.0"));
    assert_eq!(entry.path, ".agents/skills/hotline-computer/SKILL.md");
    assert_eq!(entry.invalid, None);

    let heard = lock(&agents.preambles)[0].clone();
    assert!(heard.contains("your `hotline-computer` skill"), "{heard}");
    assert!(heard.contains("\n- hotline-computer: "), "{heard}");
    assert!(!heard.contains("action `guide`"), "{heard}");

    // A colleague sharing the working directory without a computer of its
    // own is not told about the guide: the line costs context and names a
    // machine it cannot drive.
    let mut bob = persona("bob");
    bob.cwd = cwd.to_string_lossy().into_owned();
    let unheard = preamble(&bob, Some(Reach::Workspace), None, &[], false);
    assert!(!unheard.contains("hotline-computer"), "{unheard}");
    assert!(unheard.contains("\n- hotline-room: "), "{unheard}");

    // The pane sees the release running. A computer made on the very image
    // it would be made on now has nothing to update to, though its guide
    // names another version, as a release candidate's does (0.13.0 for
    // 0.13.0-rc.1): an Update would only make it again on the same image.
    let status = room.computer_status("ada").await.unwrap();
    assert_eq!(status.release.as_deref(), Some("0.11.0"));
    assert_eq!(status.available, None);
    // Pinned to another image, it is offered that one.
    let mut ada = room.persona("ada").unwrap();
    ada.computer.as_mut().unwrap().image = Some("hotline-computer:0.13.0-rc.2".into());
    enrol(&room.log, &ada);
    let status = room.computer_status("ada").await.unwrap();
    assert_eq!(status.available.as_deref(), Some("0.13.0-rc.2"));
}

/// An image too old to serve a guide leaves no stale skill. The teammate
/// asks the computer directly, while status still knows the release.
#[tokio::test]
async fn a_held_computer_without_a_guide_reports_its_label_and_leaves_no_stale_skill() {
    let ComputerRoom {
        room, agents, cwd, ..
    } = computer_room("computer-no-guide", None, TWO_RELEASES, true).await;
    // A guide from an earlier start, marked as the computer's, must not
    // outlive the computer that served it.
    crate::skills::write_computer(&cwd, None, "0.1.0", "stale", "stale guide").unwrap();
    room.start("ada").await.unwrap();

    assert!(!cwd.join(".agents/skills/hotline-computer").exists());
    assert!(crate::skills::computer_entry(&cwd, None).is_none());
    let heard = lock(&agents.preambles)[0].clone();
    assert!(heard.contains("action `guide`"), "{heard}");
    assert!(!heard.contains("- hotline-computer:"), "{heard}");
    let notices: Vec<String> = tape(&room, "ada")
        .iter()
        .filter(|event| event["kind"] == "notice")
        .map(|event| event["text"].as_str().unwrap().to_string())
        .collect();
    assert!(
        notices.is_empty(),
        "a missing guide is not the conversation's: {notices:?}"
    );
    let status = room.computer_status("ada").await.unwrap();
    assert_eq!(status.release.as_deref(), Some("test"));
    assert_eq!(status.available, None);
}

/// Updating a computer recreates it on the release it would be created on
/// now, without touching the teammate's work: the command answers at once,
/// the new release downloads while the old computer keeps serving the turn
/// in flight, and the swap happens once that turn ends. The runtime sees a
/// remove and a create only then.
#[tokio::test]
async fn updating_a_computer_waits_for_the_turn_and_the_teammate_carries_on() {
    let ComputerRoom { room, root, .. } =
        computer_room("computer-update", Some("0.11.0"), TWO_RELEASES, true).await;
    room.start("ada").await.unwrap();
    let session = room.session("ada").unwrap();
    assert!(session.computer);
    lock(&session.turns).running = true;
    std::fs::write(root.join("state.noimage"), "").unwrap();
    std::fs::write(root.join("state.pullgate"), "").unwrap();

    tokio::time::timeout(Duration::from_secs(5), room.computer_update("ada"))
        .await
        .expect("the update does not wait for the download")
        .unwrap();
    let removes = |root: &std::path::Path| {
        runtime_commands(root)
            .iter()
            .filter(|line| line.starts_with("rm "))
            .count()
    };
    assert_eq!(removes(&root), 0, "the old computer keeps working");

    std::fs::remove_file(root.join("state.pullgate")).unwrap();
    let downloaded = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if room.computer_links.swap_waiting("ada") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(downloaded.is_ok(), "the new release downloads");
    let still = room.session("ada").unwrap();
    assert!(
        Arc::ptr_eq(&still, &session),
        "the turn in flight is not cut short"
    );
    assert_eq!(removes(&root), 0, "nothing is swapped mid-turn");

    // Idle for a moment, and a message is admitted in the gap between the
    // swap seeing that and doing it: behind the start gate the swap finds the
    // turn claimed and leaves the session alone.
    lock(&session.turns).running = false;
    let gate = room.start_gate("ada");
    let held = gate.lock().await;
    room.swap_computer_when_idle("ada");
    lock(&session.turns).running = true;
    drop(held);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(Arc::ptr_eq(&room.session("ada").unwrap(), &session));
    assert_eq!(removes(&root), 0, "the admitted turn keeps its computer");
    assert!(
        room.computer_links.swap_waiting("ada"),
        "the swap waits for that turn instead"
    );

    // The turn ends; what the turn loop does next swaps the computer in.
    lock(&session.turns).running = false;
    room.swap_computer_when_idle("ada");
    let swapped = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let replaced = room
                .session("ada")
                .is_ok_and(|now| !Arc::ptr_eq(&now, &session) && now.computer);
            if replaced && room.info("ada").state == SessionState::Ready {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        swapped.is_ok(),
        "the teammate carries on with the new computer"
    );

    let commands: Vec<String> = runtime_commands(&root)
        .iter()
        .map(|line| {
            line.split_whitespace()
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    let first_create = commands.iter().position(|cmd| cmd == "create").unwrap();
    let removed = commands.iter().position(|cmd| cmd == "rm").unwrap();
    let second_create = commands.iter().rposition(|cmd| cmd == "create").unwrap();
    assert!(
        first_create < removed && removed < second_create,
        "{commands:?}"
    );
    assert!(
        notices(&room, "ada").is_empty(),
        "the swap is seamless: nothing on the tape"
    );
}

/// An update the person pressed that cannot download says why in the pane
/// that offered it, never in the conversation, and pressing it again
/// clears the old reason.
#[tokio::test]
async fn a_failed_update_is_told_in_the_pane_not_the_conversation() {
    let ComputerRoom { room, root, .. } =
        computer_room("computer-update-fails", Some("0.11.0"), TWO_RELEASES, true).await;
    room.start("ada").await.unwrap();
    std::fs::write(root.join("state.noimage"), "").unwrap();
    std::fs::write(root.join("state.pullfail"), "").unwrap();

    room.computer_update("ada").await.unwrap();
    let failed = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(reason) = room.computer_status("ada").await.unwrap().update_failed {
                break reason;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the pane is told");
    assert_ne!(failed, "");
    assert!(notices(&room, "ada").is_empty(), "the conversation is not");

    std::fs::write(root.join("state.pullgate"), "").unwrap();
    room.computer_update("ada").await.unwrap();
    assert_eq!(
        room.computer_status("ada").await.unwrap().update_failed,
        None,
        "trying again clears the old reason"
    );
    std::fs::remove_file(root.join("state.pullgate")).unwrap();
}

/// With no session running there is nothing to wait for: the container
/// goes at once and the next start makes the new one.
#[tokio::test]
async fn updating_a_resting_teammates_computer_removes_it_at_once() {
    let ComputerRoom { room, root, .. } =
        computer_room("computer-update-idle", Some("0.11.0"), TWO_RELEASES, true).await;
    room.start("ada").await.unwrap();
    room.stop("ada").unwrap();
    room.computer_update("ada").await.unwrap();
    assert!(
        runtime_commands(&root)
            .iter()
            .any(|line| line.starts_with("rm ")),
        "the container is gone"
    );
    assert!(room.session("ada").is_err(), "nothing was started");
}

/// Memory and CPUs are taken when a computer is made, so the pane is told
/// when the running one has other limits than the teammate's settings, and
/// a restart pressed while the teammate is working makes it again between
/// turns, with the new limits, and clears the offer.
#[tokio::test]
async fn a_restart_makes_the_computer_again_with_the_limits_set_since() {
    let ComputerRoom { room, root, .. } =
        computer_room("computer-restart", Some("0.11.0"), TWO_RELEASES, true).await;
    room.start("ada").await.unwrap();
    let session = room.session("ada").unwrap();
    assert!(!room.computer_status("ada").await.unwrap().limits_changed);

    let mut ada = room.persona("ada").unwrap();
    let computer = ada.computer.as_mut().unwrap();
    computer.memory = Some("2048m".into());
    computer.cpus = Some(1.5);
    enrol(&room.log, &ada);
    assert!(
        room.computer_status("ada").await.unwrap().limits_changed,
        "the running computer still has the old limits"
    );

    std::fs::write(root.join("argv.log"), "").unwrap();
    room.computer_restart("ada").await.unwrap();
    let swapped = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let replaced = room
                .session("ada")
                .is_ok_and(|now| !Arc::ptr_eq(&now, &session) && now.computer);
            if replaced && room.info("ada").state == SessionState::Ready {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        swapped.is_ok(),
        "the teammate carries on with the new computer"
    );
    let commands = runtime_commands(&root);
    let create = commands
        .iter()
        .find(|line| line.starts_with("create "))
        .unwrap_or_else(|| panic!("{commands:?}"));
    assert!(create.contains("--memory 2048m"), "{create}");
    assert!(create.contains("--cpus 1.5"), "{create}");
    assert!(!room.computer_status("ada").await.unwrap().limits_changed);
}

/// With no session running a restart only stops the computer: its next
/// start finds the limits differ and makes it again.
#[tokio::test]
async fn restarting_a_resting_teammates_computer_stops_it_for_the_next_start() {
    let ComputerRoom { room, root, .. } =
        computer_room("computer-restart-idle", Some("0.11.0"), TWO_RELEASES, true).await;
    room.start("ada").await.unwrap();
    room.stop("ada").unwrap();
    std::fs::write(root.join("argv.log"), "").unwrap();
    room.computer_restart("ada").await.unwrap();
    let commands = runtime_commands(&root);
    assert!(
        commands.iter().any(|line| line.starts_with("stop ")),
        "{commands:?}"
    );
    assert!(
        !commands
            .iter()
            .any(|line| line.starts_with("rm ") || line.starts_with("create ")),
        "nothing is made before it is needed: {commands:?}"
    );
    assert!(room.session("ada").is_err(), "nothing was started");
}

/// Play on the pane's desktop row: a teammate at rest starts with its
/// computer, and a running one whose computer was stopped from the pane has
/// it started again by the session attaching to it.
#[tokio::test]
async fn starting_a_computer_from_the_pane_brings_it_up_with_its_session() {
    let ComputerRoom { room, root, .. } =
        computer_room("computer-start", Some("0.11.0"), TWO_RELEASES, true).await;
    room.computer_start("ada").await.unwrap();
    let session = room.session("ada").expect("the teammate started");
    assert!(session.computer, "with its computer");

    room.computer_stop("ada").await.unwrap();
    std::fs::write(root.join("argv.log"), "").unwrap();
    room.computer_start("ada").await.unwrap();
    let commands = runtime_commands(&root);
    assert!(
        commands.iter().any(|line| line.starts_with("start ")),
        "the stopped computer is started again: {commands:?}"
    );
    let again = room.session("ada").expect("still running");
    assert!(
        !Arc::ptr_eq(&again, &session),
        "the session attached afresh"
    );
    assert!(again.computer);
}

/// A computer that cannot come up is not a silent Start: the session goes
/// ahead without it, as any start does, and the key says why.
#[tokio::test]
async fn starting_a_computer_that_cannot_come_up_says_why() {
    let ComputerRoom { room, root, .. } =
        computer_room("computer-start-fails", Some("0.11.0"), TWO_RELEASES, true).await;
    std::fs::write(root.join("state.startfail"), "").unwrap();
    let said = room.computer_start("ada").await.expect_err("it says why");
    assert!(said.contains("computer could not start"), "{said}");
    assert!(
        room.session("ada").is_ok_and(|session| !session.computer),
        "the teammate answers without it"
    );
}

/// A fresh computer is created on the newest published release: the desk
/// asks once before creating, and a computer already on it is offered
/// nothing. A pinned image is used as written and the endpoint never hears
/// about it.
#[tokio::test]
async fn a_fresh_computer_is_created_on_the_newest_release_and_a_pin_never_asks() {
    let fresh = computer_room("computer-newest", Some("0.11.1"), TWO_RELEASES, false).await;
    fresh.room.start("ada").await.unwrap();
    let created = runtime_commands(&fresh.root)
        .into_iter()
        .find(|line| line.starts_with("create "))
        .unwrap();
    assert!(
        created.contains("ghcr.io/1broseidon/hotline-computer:0.11.1"),
        "{created}"
    );
    assert_eq!(fresh.asked.load(std::sync::atomic::Ordering::SeqCst), 1);
    let status = fresh.room.computer_status("ada").await.unwrap();
    assert_eq!(status.release.as_deref(), Some("0.11.1"));
    assert_eq!(status.available, None);
    let known = fresh.room.computer_releases();
    assert_eq!(known.floor, crate::computer::COMPUTER_VERSION);
    assert_eq!(known.repository, crate::computer::COMPUTER_REPOSITORY);
    assert_eq!(known.newest.as_deref(), Some("0.11.1"));
    assert_eq!(known.releases, ["0.11.1", "0.11.0"]);
    assert!(known.checked_at.is_some(), "{known:?}");
    assert_eq!(known.error, None);

    let pinned = computer_room("computer-pinned", Some("0.11.1"), TWO_RELEASES, true).await;
    pinned.room.start("ada").await.unwrap();
    let created = runtime_commands(&pinned.root)
        .into_iter()
        .find(|line| line.starts_with("create "))
        .unwrap();
    assert!(created.contains(" hotline-computer:test"), "{created}");
    assert_eq!(
        pinned.asked.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "a pin is exactly what it says"
    );
}

/// Offline, a fresh computer is created on the floor, and nothing is
/// offered until something newer is actually known.
#[tokio::test]
async fn offline_a_fresh_computer_is_created_on_the_floor() {
    let offline = computer_room("computer-offline", Some("0.11.0"), "not a list", false).await;
    offline.room.start("ada").await.unwrap();
    let created = runtime_commands(&offline.root)
        .into_iter()
        .find(|line| line.starts_with("create "))
        .unwrap();
    assert!(
        created.contains(&crate::computer::default_image()),
        "{created}"
    );
    assert_eq!(offline.room.computer_releases().newest, None);
    let status = offline.room.computer_status("ada").await.unwrap();
    assert_eq!(
        status.available, None,
        "nothing newer is known, so nothing is offered"
    );
}

/// An image the runtime does not have is pulled before the computer is
/// made, and the pull shows only as live progress for the computer's
/// button: counted as each layer lands, then done. Nothing about it is
/// written on the tape.
#[tokio::test]
async fn a_pulled_image_is_live_progress_and_never_on_the_tape() {
    let desk = computer_room("computer-pull", Some("0.11.0"), TWO_RELEASES, false).await;
    std::fs::write(desk.root.join("state.noimage"), "").unwrap();
    let mut deltas = desk.room.subscribe_deltas();
    desk.room.start("ada").await.unwrap();
    // The start no longer waits for a download; it finishes behind it.
    let mut seen = Vec::new();
    let finished = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(StreamDelta::ComputerPull {
                persona_id,
                layers_done,
                layers_total,
                status,
            }) = deltas.recv().await
            {
                assert_eq!(persona_id, "ada");
                seen.push((layers_done, layers_total, status));
                if status == crate::contract::PullStatus::Done {
                    break;
                }
            }
        }
    })
    .await;
    assert!(finished.is_ok(), "the pull finishes behind the start");
    assert_eq!(seen.last().map(|seen| (seen.0, seen.1)), Some((2, 2)));
    assert!(seen.len() > 1, "it is counted as it lands: {seen:?}");
    let pulled = runtime_commands(&desk.root)
        .into_iter()
        .filter(|line| line.starts_with("pull "))
        .count();
    assert_eq!(pulled, 1, "the image was pulled once");
    assert!(
        tape(&desk.room, "ada")
            .iter()
            .all(|event| event["kind"] != "computer_pull"),
        "the pull is not on the tape"
    );
    assert_eq!(notices(&desk.room, "ada"), [] as [String; 0]);
}

/// A download never keeps a teammate from answering. The start goes ahead
/// the moment the image starts coming down, and the agent is told its
/// computer is on the way and how to ask after it. When the image lands
/// during a turn, the turn is left alone; the computer joins between turns,
/// by a restart the conversation survives.
#[tokio::test]
async fn a_download_starts_the_teammate_at_once_and_the_computer_joins_after_the_turn() {
    let desk = computer_room(
        "computer-download-behind",
        Some("0.11.0"),
        TWO_RELEASES,
        false,
    )
    .await;
    std::fs::write(desk.root.join("state.noimage"), "").unwrap();
    std::fs::write(desk.root.join("state.pullgate"), "").unwrap();

    let info = tokio::time::timeout(Duration::from_secs(10), desk.room.start("ada"))
        .await
        .expect("the start does not wait for the download")
        .unwrap();
    assert_eq!(info.state, SessionState::Ready);
    let first = lock(&desk.agents.preambles).last().cloned().unwrap();
    assert!(!first.contains("You have a computer"), "{first}");
    assert!(first.contains("still downloading"), "{first}");
    let status = desk.room.computer_setup_status("ada", Duration::ZERO).await;
    assert_eq!(status["state"], "downloading", "{status}");
    // The layers are counted as the runtime names them.
    let counted = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if desk.room.computer_setup_status("ada", Duration::ZERO).await["layersTotal"] == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(counted.is_ok(), "the status counts the layers");

    // A turn is in flight when the image lands.
    let session = desk.room.session("ada").unwrap();
    lock(&session.turns).running = true;
    std::fs::remove_file(desk.root.join("state.pullgate")).unwrap();
    let status = desk
        .room
        .computer_setup_status("ada", Duration::from_secs(10))
        .await;
    assert_eq!(
        status["state"], "ready",
        "waiting returns once it lands: {status}"
    );
    let still = desk.room.session("ada").unwrap();
    assert!(
        Arc::ptr_eq(&still, &session),
        "the turn in flight is not cut short"
    );
    assert!(!still.computer);

    // The turn ends; what the turn loop does next brings the computer in.
    lock(&session.turns).running = false;
    desk.room.attach_computer_when_idle("ada");
    let attached = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if desk.room.computer_setup_status("ada", Duration::ZERO).await["state"] == "attached" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(attached.is_ok(), "the computer joins between turns");
    let last = lock(&desk.agents.preambles).last().cloned().unwrap();
    assert!(last.contains("You have a computer"), "{last}");
    assert!(
        notices(&desk.room, "ada").is_empty(),
        "the computer joins without a word on the tape"
    );
    let pulls = runtime_commands(&desk.root)
        .into_iter()
        .filter(|line| line.starts_with("pull "))
        .count();
    assert_eq!(
        pulls, 1,
        "the restart finds the image and does not pull again"
    );
}

/// The Settings button asks the endpoint at once, whatever the clock says,
/// and a refused lookup keeps what was known and says why.
#[tokio::test]
async fn a_manual_check_asks_now_and_a_refusal_keeps_what_was_known() {
    use std::sync::atomic::Ordering;
    let desk = computer_room("computer-check-now", Some("0.11.0"), TWO_RELEASES, false).await;
    desk.room.start("ada").await.unwrap();
    assert_eq!(desk.asked.load(Ordering::SeqCst), 1);
    let checked = desk.room.computer_releases_check().await;
    assert_eq!(
        desk.asked.load(Ordering::SeqCst),
        2,
        "the button does not wait six hours"
    );
    assert_eq!(checked.releases, ["0.11.1", "0.11.0"]);
    assert_eq!(checked.error, None);

    let offline = computer_room(
        "computer-check-offline",
        Some("0.11.0"),
        "not a list",
        false,
    )
    .await;
    let refused = offline.room.computer_releases_check().await;
    assert_eq!(refused.newest, None);
    assert_eq!(refused.releases, [] as [String; 0]);
    assert!(refused.checked_at.is_some());
    assert!(
        refused
            .error
            .as_deref()
            .is_some_and(|why| why.contains("names no release")),
        "{refused:?}"
    );
}

/// A computer on an older release is offered the newest one, and the desk
/// asks the endpoint again only when six hours have passed.
#[tokio::test]
async fn an_older_computer_is_offered_the_newest_release_on_the_six_hour_clock() {
    use crate::computer::releases::CHECK_EVERY_MS;
    use std::sync::atomic::Ordering;
    let older = computer_room("computer-older", Some("0.8.0"), TWO_RELEASES, false).await;
    // Made on 0.8.0 by an older desk's choice, now the desk's to choose.
    let mut ada = older.room.persona("ada").unwrap();
    ada.computer.as_mut().unwrap().image = Some(crate::computer::image_at("0.8.0"));
    enrol(&older.room.log, &ada);
    older.room.start("ada").await.unwrap();
    ada.computer.as_mut().unwrap().image = None;
    enrol(&older.room.log, &ada);
    older.room.computers.refresh_releases(now_ms()).await;
    let status = older.room.computer_status("ada").await.unwrap();
    assert_eq!(status.release.as_deref(), Some("0.8.0"));
    assert_eq!(status.available.as_deref(), Some("0.11.1"));

    let asked_at_start = older.asked.load(Ordering::SeqCst);
    let now = now_ms();
    older
        .room
        .computers
        .refresh_releases(now + CHECK_EVERY_MS / 2)
        .await;
    assert_eq!(older.asked.load(Ordering::SeqCst), asked_at_start);
    older
        .room
        .computers
        .refresh_releases(now + CHECK_EVERY_MS)
        .await;
    assert_eq!(older.asked.load(Ordering::SeqCst), asked_at_start + 1);
}

#[tokio::test]
async fn laptop_cookies_reach_the_computer_but_never_the_room_or_tape() {
    let ComputerRoom { room, taken, .. } =
        computer_room("laptop-cookies", Some("0.11.0"), TWO_RELEASES, true).await;
    let transfer = crate::contract::CookieTransfer {
        source_id: "laptop".into(),
        browser_id: "firefox".into(),
        profile_id: "default".into(),
        domains: vec!["example.com".into()],
        cookies: vec![
            json!({"domain":".example.com","name":"session","value":"laptop-secret-value","path":"/"}),
        ],
    };
    let mut invalid = transfer.clone();
    invalid.domains = vec!["unselected.com".into()];
    assert!(room.computer_cookies_push("ada", invalid).await.is_err());
    assert!(taken.uploads.lock().unwrap().is_empty());
    let sites = room.computer_cookies_push("ada", transfer).await.unwrap();
    assert_eq!(sites[0].domain, "example.com");
    assert_eq!(
        taken.uploads.lock().unwrap()[0]["cookies"][0]["value"],
        "laptop-secret-value"
    );
    assert_eq!(room.cookie_lends("ada")[0].browser_name, "firefox (laptop)");
    assert!(room.cookie_lends("ada")[0].from_laptop);
    for stream in [StreamId::Room, StreamId::Tape("ada".into())] {
        assert!(
            !serde_json::to_string(&room.log.load(&stream))
                .unwrap()
                .contains("laptop-secret-value")
        );
    }
}

#[tokio::test]
async fn a_slow_secret_use_poll_does_not_delay_another_computers_next_card() {
    let fixture = secret_use_room(
        "use-independent-polls",
        "0.13.0",
        crate::contract::SecretUse::Ask,
    )
    .await;
    let room = &fixture.room;
    // An in-flight poll or answer owns this lock while its computer is slow.
    let busy = room.use_watch("another-computer");
    let _held = busy.lock().await;
    let first = fixture.taken.uses.hold("GH_PASSKEY", "github.com").unwrap();
    until("the first use is offered", || {
        tape(room, "ada").iter().any(|event| {
            event["kind"] == "secret_use_ask"
                && event["useId"] == first
                && event["status"] == "pending"
        })
    })
    .await;
    fixture.taken.uses.expire(&first);
    let second = fixture.taken.uses.hold("GH_PASSKEY", "github.com").unwrap();
    until("the next round still reaches this computer", || {
        tape(room, "ada").iter().any(|event| {
            event["kind"] == "secret_use_ask"
                && event["useId"] == second
                && event["status"] == "pending"
        })
    })
    .await;
    assert!(
        tape(room, "ada")
            .iter()
            .any(|event| event["useId"] == first && event["status"] == "expired")
    );
}

fn commands_starting(root: &std::path::Path, word: &str) -> usize {
    runtime_commands(root)
        .iter()
        .filter(|line| line.starts_with(word))
        .count()
}

/// A side thread or a job holding the computer's lease is using it with no
/// main session behind it. The sweep leaves its computer running, and once
/// the lease is let go the computer idles like any other.
#[tokio::test]
async fn a_computer_held_by_a_side_thread_survives_the_sweep() {
    let ComputerRoom { room, root, .. } =
        computer_room("computer-lease-sweep", Some("0.11.0"), TWO_RELEASES, true).await;
    room.start("ada").await.unwrap();
    room.stop("ada").unwrap();
    let holder = crate::computer::gate::Holder::new("side-1", "a side thread");
    let call = room
        .computer_leases
        .take("ada", &holder, &|| true)
        .await
        .unwrap();
    drop(call);

    room.computers.mark_idle("ada", 0);
    let before = commands_starting(&root, "stop") + commands_starting(&root, "rm");
    room.sweep_computers().await;
    assert_eq!(
        commands_starting(&root, "stop") + commands_starting(&root, "rm"),
        before,
        "a held lease is not idle"
    );

    room.let_go_of_computer("ada", &holder);
    room.computers.mark_idle("ada", 0);
    room.sweep_computers().await;
    assert_eq!(commands_starting(&root, "rm"), 1, "let go, it is swept");
}

/// A thread running jobs on the computer with calls that need no lease is
/// using it too, holding no lease: the sweep leaves it running.
#[tokio::test]
async fn a_computer_a_thread_runs_jobs_on_without_the_lease_survives_the_sweep() {
    let ComputerRoom { room, root, .. } =
        computer_room("computer-shared-sweep", Some("0.11.0"), TWO_RELEASES, true).await;
    room.start("ada").await.unwrap();
    room.stop("ada").unwrap();
    let before = commands_starting(&root, "stop") + commands_starting(&root, "rm");
    let call = room.computer_leases.share("ada");
    room.computers.mark_idle("ada", 0);
    room.sweep_computers().await;
    drop(call);
    room.computers.mark_idle("ada", 0);
    room.sweep_computers().await;
    assert_eq!(
        commands_starting(&root, "stop") + commands_starting(&root, "rm"),
        before,
        "a call in flight, or one just ended, is not idle"
    );
}

/// An update's swap is claimed under the start gate, not before it: while it
/// waits there, the teammate's phase still says a swap is under way, so a
/// second update does not download another.
#[tokio::test]
async fn a_swap_waiting_at_the_start_gate_still_holds_the_teammates_update() {
    let ComputerRoom { room, root, .. } =
        computer_room("computer-swap-gate", Some("0.11.0"), TWO_RELEASES, true).await;
    room.start("ada").await.unwrap();
    let session = room.session("ada").unwrap();
    lock(&session.turns).running = true;
    std::fs::write(root.join("state.noimage"), "").unwrap();
    room.computer_update("ada").await.unwrap();
    while !room.computer_links.swap_waiting("ada") {
        tokio::task::yield_now().await;
    }
    lock(&session.turns).running = false;
    let pulls = commands_starting(&root, "pull");

    let held = room.start_gate("ada").lock_owned().await;
    room.swap_computer_when_idle("ada");
    assert!(
        room.computer_links.swap_waiting("ada"),
        "the swap is not claimed before it has the gate"
    );
    room.computer_update("ada").await.unwrap();
    assert_eq!(
        commands_starting(&root, "pull"),
        pulls,
        "no second download"
    );
    drop(held);

    // The swap queued first, so this is behind it.
    drop(room.start_gate("ada").lock_owned().await);
    assert!(!Arc::ptr_eq(&room.session("ada").unwrap(), &session));
    assert_eq!(
        room.computer_links.phase("ada"),
        crate::session::computer::ComputerPhase::None
    );
}

/// A room that is closing starts nothing: the swap leaves the old computer
/// where it is and waits.
#[tokio::test]
async fn a_swap_in_a_closing_room_leaves_the_old_computer_and_waits() {
    let ComputerRoom { room, root, .. } =
        computer_room("computer-swap-closing", Some("0.11.0"), TWO_RELEASES, true).await;
    room.start("ada").await.unwrap();
    room.computer_restart("ada").await.unwrap();
    while !matches!(
        room.computer_links.phase("ada"),
        crate::session::computer::ComputerPhase::None
    ) {
        tokio::task::yield_now().await;
    }
    // That restart landed. Ask for another with the room closing.
    let removes = commands_starting(&root, "rm");
    let held = room.start_gate("ada").lock_owned().await;
    room.computer_links.restart("ada");
    room.swap_computer_when_idle("ada");
    room.closing
        .store(true, std::sync::atomic::Ordering::SeqCst);
    drop(held);
    drop(room.start_gate("ada").lock_owned().await);
    assert_eq!(commands_starting(&root, "rm"), removes, "nothing removed");
    assert!(room.computer_links.swap_waiting("ada"), "the swap waits");
}

/// A key Used with GitHub, granted protected; a plain variable, granted
/// readable as every one was at upgrade; and a login; to a Hotline Agent
/// teammate confined to its workspace, the one a key can be protected for.
fn grant_a_key_a_variable_and_a_login(room: &Room) {
    let mut ada = room.persona("ada").unwrap();
    ada.reach = Some(crate::contract::Reach::Workspace);
    enrol(&room.log, &ada);
    let vault = room.vault.as_ref().unwrap();
    vault
        .set_shared(
            "GITHUB_TOKEN",
            crate::vault::StoredSecret::Variable {
                value: "ghp_notarealtoken00000000000000000000001".into(),
                r#use: Some(
                    crate::vault::resolve_used_with(&crate::contract::UsedWith {
                        service: "github".into(),
                        host: None,
                    })
                    .unwrap(),
                ),
            },
        )
        .unwrap();
    vault
        .set_shared_secret("NPM_TOKEN", "npm_notarealtoken0002")
        .unwrap();
    vault
        .set_shared(
            "GITHUB_LOGIN",
            crate::vault::StoredSecret::Login {
                r#use: crate::contract::SecretUse::Auto,
                sites: vec!["https://github.com".into()],
                username: "george".into(),
                password: "correct-horse-battery".into(),
                totp: None,
            },
        )
        .unwrap();
    grant_secrets(room, &["GITHUB_LOGIN", "GITHUB_TOKEN", "NPM_TOKEN"]);
}

async fn protection(room: &Room) -> Option<crate::contract::ComputerProtection> {
    room.computer_status("ada").await.unwrap().protection
}

/// Today's pinned computer is handed what it was always handed, in the
/// form it takes: the readable variable and the login; the protected key
/// waits on the desk, and the pane is told so once.
#[tokio::test]
async fn a_protected_key_never_reaches_an_old_computer() {
    let ComputerRoom { room, taken, .. } = computer_room(
        "protected-old",
        Some(crate::computer::COMPUTER_VERSION),
        TWO_RELEASES,
        true,
    )
    .await;
    grant_a_key_a_variable_and_a_login(&room);
    room.start("ada").await.unwrap();
    let sets = taken.sets();
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0]["NPM_TOKEN"], json!("npm_notarealtoken0002"));
    assert_eq!(sets[0]["GITHUB_LOGIN"]["kind"], "login");
    assert!(!sets[0].contains_key("GITHUB_TOKEN"));
    assert_eq!(taken.forms(), [1]);
    assert_eq!(
        protection(&room).await,
        Some(crate::contract::ComputerProtection::NeedsUpdate)
    );
    let everywhere = serde_json::to_string(&taken.sets()).unwrap();
    assert!(!everywhere.contains("ghp_notarealtoken"), "{everywhere}");
}

#[tokio::test]
async fn a_login_reaches_a_legacy_computer_as_before_and_is_marked_unprotected() {
    let ComputerRoom { room, taken, .. } =
        computer_room("protected-legacy", Some("0.13.0"), TWO_RELEASES, true).await;
    taken
        .legacy
        .store(true, std::sync::atomic::Ordering::SeqCst);
    grant_a_key_a_variable_and_a_login(&room);
    room.start("ada").await.unwrap();
    assert_eq!(taken.forms(), [1]);
    assert_eq!(
        taken.sets()[0].keys().collect::<Vec<_>>(),
        ["GITHUB_LOGIN", "NPM_TOKEN"],
        "the login and the variable as 0.49 handed them; the protected key waits"
    );
    assert_eq!(
        taken.sets()[0]["GITHUB_LOGIN"]["password"],
        "correct-horse-battery"
    );
    assert_eq!(
        protection(&room).await,
        Some(crate::contract::ComputerProtection::NeedsUpdate)
    );
    grant_secrets(&room, &["GITHUB_LOGIN"]);
    room.secrets_changed().await;
    assert_eq!(taken.sets()[1].keys().collect::<Vec<_>>(), ["GITHUB_LOGIN"]);
    assert_eq!(
        protection(&room).await,
        Some(crate::contract::ComputerProtection::Legacy),
        "not protected, though nothing waits"
    );
}

/// A teammate that runs as the person is never handed a key it was given
/// protected, even by a computer that proves itself.
#[tokio::test]
async fn a_whole_machine_teammates_protected_key_is_held_back() {
    let ComputerRoom { room, taken, .. } =
        computer_room("protected-machine", Some("0.13.0"), TWO_RELEASES, true).await;
    grant_a_key_a_variable_and_a_login(&room);
    let mut ada = room.persona("ada").unwrap();
    ada.reach = Some(crate::contract::Reach::Machine);
    enrol(&room.log, &ada);
    room.start("ada").await.unwrap();
    let set = &taken.sets()[0];
    assert!(!set.contains_key("GITHUB_TOKEN"), "{set:?}");
    assert_eq!(set["GITHUB_LOGIN"]["kind"], "login");
}

#[tokio::test]
async fn a_split_computer_that_proves_itself_takes_every_grant_and_a_forged_one_none() {
    let ComputerRoom {
        room,
        taken,
        agents,
        ..
    } = computer_room("protected-split", Some("0.13.0"), TWO_RELEASES, true).await;
    grant_a_key_a_variable_and_a_login(&room);
    room.start("ada").await.unwrap();
    assert_eq!(taken.forms(), [2]);
    let set = &taken.sets()[0];
    assert_eq!(set["NPM_TOKEN"]["readable"], true);
    assert_eq!(set["GITHUB_TOKEN"]["use"]["standIn"]["prefix"], "ghp_");
    assert!(set["GITHUB_TOKEN"].get("readable").is_none());
    assert_eq!(set["GITHUB_LOGIN"]["kind"], "login");
    assert_eq!(
        protection(&room).await,
        Some(crate::contract::ComputerProtection::Protected)
    );
    // The agent is told the key is used with its service, and what that
    // means for it, never the value.
    let heard = lock(&agents.preambles)[0].clone();
    assert!(
        heard.contains(
            "GITHUB_TOKEN (a key used with api.github.com, github.com, uploads.github.com)"
        ),
        "{heard}"
    );
    assert!(heard.contains("holds a stand-in"), "{heard}");
    assert!(heard.contains("NPM_TOKEN (a variable)"), "{heard}");
    assert!(!heard.contains("ghp_notarealtoken"), "{heard}");

    taken
        .forged
        .store(true, std::sync::atomic::Ordering::SeqCst);
    room.secrets_changed().await;
    assert_eq!(taken.sets().len(), 1, "a forged attest is handed nothing");
    assert_eq!(
        protection(&room).await,
        Some(crate::contract::ComputerProtection::NeedsUpdate),
        "and is no longer said to be protected"
    );
}

/// A computer that attests as 0.13.0-rc.2 did, v1 under the bearer, is
/// handed nothing, however good its attest: the desk's own asking carried
/// the bearer that attest is made with to whatever answered, so it proves
/// nothing about what that is. It is never said to be protected, and the
/// pane says it needs an update.
#[tokio::test]
async fn a_v1_claim_of_split_is_handed_nothing_whatever_it_attests() {
    use std::sync::atomic::Ordering::SeqCst;
    let ComputerRoom { room, taken, .. } =
        computer_room("v1-claim", Some("0.13.0-rc.2"), TWO_RELEASES, true).await;
    taken.v1.store(true, SeqCst);
    grant_a_key_a_variable_and_a_login(&room);
    room.start("ada").await.unwrap();
    assert!(taken.sets().is_empty(), "{:?}", taken.sets());
    assert_eq!(
        protection(&room).await,
        Some(crate::contract::ComputerProtection::NeedsUpdate)
    );
    let said = lock(&room.secret_notices).clone();
    assert!(
        said.iter()
            .any(|text| text.contains(crate::computer::secrets::UNPROVEN)),
        "{said:?}"
    );
}

/// Every release a container is made to split on serves `/capabilities`,
/// split or legacy, so one that answers 404 there is not that computer, and
/// is handed nothing, with no key held to tell: not even what a release
/// from before 0.13 is handed.
#[tokio::test]
async fn a_computer_made_to_split_without_a_capabilities_door_is_handed_nothing() {
    // What answers the port is a release from before `/capabilities`; the
    // container was made on 0.13.0, as its label says.
    let ComputerRoom { room, taken, .. } = computer_room(
        "split-no-door",
        Some(crate::computer::COMPUTER_VERSION),
        TWO_RELEASES,
        true,
    )
    .await;
    let mut ada = room.persona("ada").unwrap();
    ada.computer.as_mut().unwrap().image = Some("hotline-computer:0.13.0".into());
    enrol(&room.log, &ada);
    grant_a_key_a_variable_and_a_login(&room);
    room.start("ada").await.unwrap();
    assert!(taken.sets().is_empty(), "{:?}", taken.sets());
    assert_eq!(
        protection(&room).await,
        Some(crate::contract::ComputerProtection::NeedsUpdate)
    );
}

/// A container made to split answers v2, split or legacy, every time. One
/// that says plain legacy under the older proof, with no key this desk
/// could take, is not that computer: it is handed nothing, not even what a
/// legacy computer is, and needs an update.
#[tokio::test]
async fn a_computer_made_to_split_that_says_plain_legacy_with_no_key_is_handed_nothing() {
    use std::sync::atomic::Ordering::SeqCst;
    let ComputerRoom { room, taken, .. } =
        computer_room("split-plain-legacy", Some("0.13.0"), TWO_RELEASES, true).await;
    taken.v1.store(true, SeqCst);
    taken.legacy.store(true, SeqCst);
    grant_a_key_a_variable_and_a_login(&room);
    room.start("ada").await.unwrap();
    assert!(taken.sets().is_empty(), "{:?}", taken.sets());
    assert_eq!(
        protection(&room).await,
        Some(crate::contract::ComputerProtection::NeedsUpdate)
    );
}

/// A computer from 0.13.0-rc.3 proves itself with the key its boot hands
/// once: the desk takes it as it starts the computer, hands every grant, a
/// login for the computer itself readable so it is still typed there, and
/// the pane hears what the computer says it cannot keep from its programs.
/// One whose key something else took first is not protected: the key Used
/// with a service waits, and the pane says it needs an update.
#[tokio::test]
async fn a_v2_computer_proves_itself_with_its_boots_key_and_one_taken_first_does_not() {
    use std::sync::atomic::Ordering::SeqCst;
    let local_login = |room: &Room| {
        room.vault
            .as_ref()
            .unwrap()
            .set_shared(
                "LOCAL_LOGIN",
                crate::vault::StoredSecret::Login {
                    r#use: crate::contract::SecretUse::Auto,
                    sites: vec!["http://localhost:8600".into()],
                    username: "rc-user".into(),
                    password: "not-a-real-password".into(),
                    totp: None,
                },
            )
            .unwrap();
        grant_secrets(
            room,
            &["GITHUB_LOGIN", "GITHUB_TOKEN", "LOCAL_LOGIN", "NPM_TOKEN"],
        );
    };

    let ComputerRoom { room, taken, .. } =
        computer_room("protected-v2", Some("0.13.0"), TWO_RELEASES, true).await;
    grant_a_key_a_variable_and_a_login(&room);
    local_login(&room);
    room.start("ada").await.unwrap();
    assert!(taken.key_taken.load(SeqCst), "taken as the desk started it");
    assert_eq!(taken.forms(), [2]);
    let set = &taken.sets()[0];
    assert_eq!(set["GITHUB_TOKEN"]["use"]["standIn"]["prefix"], "ghp_");
    assert_eq!(set["LOCAL_LOGIN"]["readable"], true);
    assert!(set["GITHUB_LOGIN"].get("readable").is_none());
    let status = room.computer_status("ada").await.unwrap();
    assert_eq!(
        status.protection,
        Some(crate::contract::ComputerProtection::Protected)
    );
    assert_eq!(status.readable, ["LOCAL_LOGIN"]);

    let ComputerRoom { room, taken, .. } =
        computer_room("protected-v2-taken", Some("0.13.0"), TWO_RELEASES, true).await;
    taken.key_taken.store(true, SeqCst);
    grant_a_key_a_variable_and_a_login(&room);
    local_login(&room);
    room.start("ada").await.unwrap();
    assert_eq!(taken.forms(), [1], "what a legacy computer is handed");
    let set = &taken.sets()[0];
    assert!(!set.contains_key("GITHUB_TOKEN"), "the key waits");
    assert_eq!(set["NPM_TOKEN"], json!("npm_notarealtoken0002"));
    let status = room.computer_status("ada").await.unwrap();
    assert_eq!(
        status.protection,
        Some(crate::contract::ComputerProtection::NeedsUpdate)
    );
    assert_eq!(status.readable, Vec::<String>::new());
    // No update helps it: only a new boot, whose key the desk takes.
    let said = lock(&room.secret_notices).clone();
    assert!(
        said.iter()
            .any(|text| text.contains("GITHUB_TOKEN waits on this desk")
                && text.contains("Restart the computer")),
        "{said:?}"
    );
    assert!(!said.iter().any(|text| text.contains("until it is updated")));
}

/// A protected key goes with its service's refuse list as this desk
/// defines it now, never the shorter one its record kept when it was
/// stored, and never left for the computer's own shorter list.
#[tokio::test]
async fn a_protected_key_is_handed_its_services_refuse_list_as_it_is_now() {
    let ComputerRoom { room, taken, .. } =
        computer_room("protected-refuse", Some("0.13.0"), TWO_RELEASES, true).await;
    grant_a_key_a_variable_and_a_login(&room);
    let github = crate::vault::resolve_used_with(&crate::contract::UsedWith {
        service: "github".into(),
        host: None,
    })
    .unwrap();
    room.vault
        .as_ref()
        .unwrap()
        .set_secret_used_with(
            "GITHUB_TOKEN",
            Some(crate::contract::VariableUse {
                service: None,
                refuse: vec!["/authorizations".into()],
                ..github.clone()
            }),
        )
        .unwrap();
    room.start("ada").await.unwrap();
    let refuse = &taken.sets()[0]["GITHUB_TOKEN"]["use"]["refuse"];
    assert_eq!(*refuse, json!(github.refuse));
    for route in ["/user/keys", "/repos/*/*/keys", "/authorizations"] {
        assert!(
            refuse.as_array().unwrap().contains(&json!(route)),
            "{route}"
        );
    }
}

/// Use Cookies goes into a protected computer's signed-in browser, never
/// as a file in the agent's home; today's computer still takes the file.
#[tokio::test]
async fn shared_cookies_go_to_a_protected_computers_sessions_and_never_to_a_file() {
    let transfer = || crate::contract::CookieTransfer {
        source_id: "laptop".into(),
        browser_id: "firefox".into(),
        profile_id: "default".into(),
        domains: vec!["example.com".into()],
        cookies: vec![
            json!({"domain":".example.com","name":"session","value":"laptop-secret-value","path":"/"}),
        ],
    };
    let ComputerRoom { room, taken, .. } =
        computer_room("sessions-protected", Some("0.13.0"), TWO_RELEASES, true).await;
    room.computer_cookies_push("ada", transfer()).await.unwrap();
    assert!(taken.uploads.lock().unwrap().is_empty(), "no file");
    let sessions = taken.sessions.lock().unwrap().clone();
    assert_eq!(sessions.len(), 1, "{sessions:?}");
    assert_eq!(sessions[0].0, "PUT");
    assert_eq!(sessions[0].2["sites"], json!(["example.com"]));
    assert_eq!(sessions[0].2["cookies"][0]["value"], "laptop-secret-value");
    assert_eq!(
        sessions[0].2["storage"],
        json!({}),
        "storage by origin, as the signed-in browser takes it"
    );
    let id = room.cookie_lends("ada")[0].id.clone();
    room.cookie_lend_remove("ada", &id).await.unwrap();
    let sessions = taken.sessions.lock().unwrap().clone();
    assert_eq!(sessions.len(), 2, "{sessions:?}");
    assert_eq!(
        (sessions[1].0.as_str(), sessions[1].1.as_str()),
        ("DELETE", sessions[0].1.as_str())
    );
    assert!(sessions[1].2["domains"].is_array(), "{sessions:?}");
    // The computer says what a removal cleared, and a share it does not
    // have (a computer made again starts without its shares) is nothing
    // left to take back rather than a failure.
    let ready = room.computers.running("ada").await.unwrap();
    let name = sessions[0].1.clone();
    let gone = crate::computer::login::forget(&ready, &name, &[]).await;
    assert_eq!(gone.map(|gone| gone.forgotten), Ok(0));
    room.computer_cookies_push("ada", transfer()).await.unwrap();
    let gone = crate::computer::login::forget(&ready, &name, &[]).await;
    assert_eq!(gone.map(|gone| gone.forgotten), Ok(1));

    let ComputerRoom { room, taken, .. } = computer_room(
        "sessions-today",
        Some(crate::computer::COMPUTER_VERSION),
        TWO_RELEASES,
        true,
    )
    .await;
    room.computer_cookies_push("ada", transfer()).await.unwrap();
    assert_eq!(taken.uploads.lock().unwrap().len(), 1);
    assert!(taken.sessions.lock().unwrap().is_empty());
}

/// After a computer's first split boot, the snapshots from before it wait
/// for the person's answer in the pane's one notice; the answer reaches the
/// computer and the notice asks no more. A legacy computer asks nothing.
#[tokio::test]
async fn the_snapshots_from_before_split_mode_wait_for_the_persons_answer() {
    let ComputerRoom { room, taken, .. } =
        computer_room("migration-pending", Some("0.13.0"), TWO_RELEASES, true).await;
    *taken.migration.lock().unwrap() = json!({
        "state": "pending", "at": 1, "snapshots": [{"name": "before-update", "bytes": 1024}],
        "loginsDropped": 2, "sitesCleared": ["github.com"], "answer": null,
    });
    grant_a_key_a_variable_and_a_login(&room);
    room.start("ada").await.unwrap();
    let asked = room
        .computer_status("ada")
        .await
        .unwrap()
        .migration
        .unwrap();
    assert_eq!(asked.snapshots[0].name, "before-update");
    assert_eq!(asked.snapshots[0].bytes, 1024);
    assert!(
        taken.migration_answers.lock().unwrap().is_empty(),
        "nothing erased unasked"
    );

    room.computer_migration_answer("ada", crate::contract::SnapshotAnswer::Clean)
        .await
        .unwrap();
    assert_eq!(
        *taken.migration_answers.lock().unwrap(),
        [json!({"snapshots": "clean"})]
    );
    assert_eq!(room.computer_status("ada").await.unwrap().migration, None);

    // Keep leaves the snapshots quarantined, so a real 0.13 computer still
    // says pending, with the answer beside it: the person answered, and the
    // notice asks no more.
    let ComputerRoom { room, taken, .. } =
        computer_room("migration-keep", Some("0.13.0"), TWO_RELEASES, true).await;
    *taken.migration.lock().unwrap() = json!({
        "state": "pending", "at": 1, "snapshots": [{"name": "before-update", "bytes": 1024}],
        "loginsDropped": 0, "sitesCleared": [], "answer": null,
    });
    grant_a_key_a_variable_and_a_login(&room);
    room.start("ada").await.unwrap();
    assert!(
        room.computer_status("ada")
            .await
            .unwrap()
            .migration
            .is_some()
    );
    room.computer_migration_answer("ada", crate::contract::SnapshotAnswer::Keep)
        .await
        .unwrap();
    assert_eq!(taken.migration.lock().unwrap()["state"], "pending");
    assert_eq!(room.computer_status("ada").await.unwrap().migration, None);
    room.secrets_changed().await;
    assert_eq!(
        room.computer_status("ada").await.unwrap().migration,
        None,
        "nor at the next handing over"
    );

    let ComputerRoom { room, taken, .. } =
        computer_room("migration-legacy", Some("0.13.0"), TWO_RELEASES, true).await;
    taken
        .legacy
        .store(true, std::sync::atomic::Ordering::SeqCst);
    room.start("ada").await.unwrap();
    assert_eq!(room.computer_status("ada").await.unwrap().migration, None);
}

/// A sign-in the protected computer could not finish is one card for the
/// person, whatever the login's policy: it opens the site's screen, takes
/// no answer, and folds away, telling the agent, once the computer stops
/// listing it.
#[tokio::test]
async fn a_sign_in_to_finish_is_one_card_until_the_computer_clears_it() {
    let ComputerRoom { room, taken, .. } =
        computer_room("finish-sign-in", Some("0.13.0"), TWO_RELEASES, true).await;
    grant_a_key_a_variable_and_a_login(&room);
    room.start("ada").await.unwrap();
    taken
        .uses
        .finish("GITHUB_LOGIN", "https://github.com", "george");
    room.poll_secret_uses("ada").await;
    room.poll_secret_uses("ada").await;
    let cards: Vec<Value> = tape(&room, "ada")
        .into_iter()
        .filter(|event| event["kind"] == "secret_use_ask")
        .collect();
    assert_eq!(cards.len(), 1, "one card, polled twice: {cards:?}");
    assert_eq!(cards[0]["operation"]["kind"], "finish");
    assert_eq!(cards[0]["operation"]["account"], "george");
    assert_eq!(cards[0]["site"], "github.com");
    assert_eq!(cards[0]["status"], "pending");
    let use_id = cards[0]["useId"].as_str().unwrap().to_owned();
    let refused = room
        .secrets_use_answer("ada", &use_id, crate::contract::SecretUseAnswer::Approve)
        .await
        .unwrap_err();
    assert!(refused.contains("nothing to answer"), "{refused}");
    assert_eq!(taken.uses.answers(), [] as [Value; 0]);

    taken.uses.finished("GITHUB_LOGIN");
    room.poll_secret_uses("ada").await;
    let last = tape(&room, "ada")
        .into_iter()
        .rfind(|event| event["kind"] == "secret_use_ask")
        .unwrap();
    assert_eq!(last["status"], "expired");
}

/// What the room's store keeps of ada's computer's attest key.
fn kept_attest(store: &crate::credentials::MemoryStore) -> Option<Value> {
    use crate::credentials::SecretStore;
    store
        .get("computer-attest/ada")
        .unwrap()
        .map(|bytes| serde_json::from_slice(&bytes).unwrap())
}

fn bearer_hash(bearer: &str) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(bearer.as_bytes()))
}

/// Every file under `root`, for a test that looks for what must not be in
/// any of them.
fn files_under(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => todo.push(path),
                Ok(kind) if kind.is_file() => found.push(path),
                _ => {}
            }
        }
    }
    found
}

/// A v2 computer keeps running across a desk restart, and the key its boot
/// handed the first desk is read back from the store by the second: it is
/// handed the v2 set, keys and all, its door is not asked again, nothing
/// waits, and a value changed after the restart reaches it.
#[tokio::test]
async fn a_desk_restart_keeps_a_v2_computer_protected_and_its_key_is_used_without_waiting() {
    use base64::Engine;
    use std::sync::atomic::Ordering::SeqCst;
    let mut fixture = kept_computer_room("restart-v2", Some("0.13.0")).await;
    let taken = fixture.taken.clone();
    grant_a_key_a_variable_and_a_login(&fixture.room);
    fixture.room.start("ada").await.unwrap();
    assert_eq!(
        protection(&fixture.room).await,
        Some(crate::contract::ComputerProtection::Protected)
    );
    let bearer = fixture.room.computers.running("ada").await.unwrap().token;
    let entry = kept_attest(&fixture.store).expect("kept as it was taken");
    assert_eq!(entry["bearer"], bearer_hash(&bearer));
    assert_eq!(taken.key_asked.load(SeqCst), 1);

    // The desk restarts: the same data root, store and runtime, the same
    // container still running with the same bearer, its door now saying
    // 409 to anyone who asks for the key.
    fixture.room = reopened(&fixture);
    let room = fixture.room.clone();
    room.start("ada").await.unwrap();
    assert_eq!(
        room.computers.running("ada").await.unwrap().token,
        bearer,
        "the same container"
    );
    assert_eq!(taken.forms(), [2, 2], "the v2 form after the restart too");
    let set = &taken.sets()[1];
    assert_eq!(set["GITHUB_TOKEN"]["use"]["standIn"]["prefix"], "ghp_");
    assert_eq!(
        set["GITHUB_TOKEN"]["value"],
        "ghp_notarealtoken00000000000000000000001"
    );
    let status = room.computer_status("ada").await.unwrap();
    assert_eq!(
        status.protection,
        Some(crate::contract::ComputerProtection::Protected)
    );
    let said = lock(&room.secret_notices).clone();
    assert!(
        !said.iter().any(|text| text.contains("on this desk")),
        "nothing waits: {said:?}"
    );
    assert_eq!(
        taken.key_asked.load(SeqCst),
        1,
        "the key door was asked once across both desks"
    );

    // A use after the restart runs with nothing waiting: a new value is
    // handed over at once, as a protected key.
    let github = crate::vault::resolve_used_with(&crate::contract::UsedWith {
        service: "github".into(),
        host: None,
    })
    .unwrap();
    room.vault
        .as_ref()
        .unwrap()
        .set_shared(
            "GITHUB_TOKEN",
            crate::vault::StoredSecret::Variable {
                value: "ghp_notarealtoken00000000000000000000002".into(),
                r#use: Some(github),
            },
        )
        .unwrap();
    room.secrets_changed().await;
    assert_eq!(taken.forms(), [2, 2, 2]);
    let set = &taken.sets()[2];
    assert_eq!(
        set["GITHUB_TOKEN"]["value"],
        "ghp_notarealtoken00000000000000000000002"
    );
    assert!(set["GITHUB_TOKEN"]["use"].is_object());
    assert_eq!(taken.key_asked.load(SeqCst), 1);

    // The key is in the store alone: no file of the desk's or the
    // container's settings holds it, in any spelling. Only a desk served
    // with `--store file` keeps its store's entries as files, under
    // `<data>/secrets`, and only then are those passed over.
    let key = crate::computer::guide::fake::ATTEST_KEY;
    let spellings = [
        base64::engine::general_purpose::STANDARD.encode(key),
        hex::encode(key),
    ];
    let file_store = std::fs::read_to_string(room.log.root().join("store.json"))
        .is_ok_and(|marker| marker.contains(r#""file""#));
    let store_files = room.log.root().join("secrets");
    for path in files_under(room.log.root())
        .into_iter()
        .filter(|path| !(file_store && path.starts_with(&store_files)))
        .chain(files_under(&fixture.root))
    {
        let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).to_lowercase();
        for spelling in &spellings {
            assert!(
                !text.contains(&spelling.to_lowercase()),
                "{} holds the key",
                path.display()
            );
        }
    }
}

/// A computer a desk that knew only v1 left running never had its key
/// asked for: the desk now running takes it at its first asking, keeps it,
/// and the computer is protected without starting again.
#[tokio::test]
async fn a_computer_left_by_a_desk_that_never_took_its_key_is_protected_at_the_first_asking() {
    use std::sync::atomic::Ordering::SeqCst;
    let fixture = kept_computer_room("left-v1-desk", Some("0.13.0")).await;
    let taken = fixture.taken.clone();
    // The container the earlier desk made, running, its bearer in its
    // settings, its release on its label.
    let state = fixture.root.join("state");
    std::fs::write(&state, "running").unwrap();
    std::fs::write(fixture.root.join("state.token"), "").unwrap();
    std::fs::write(fixture.root.join("state.release"), "0.13.0").unwrap();
    assert_eq!(kept_attest(&fixture.store), None);
    let room = fixture.room.clone();
    grant_a_key_a_variable_and_a_login(&room);
    room.computers
        .adopt(&["ada".to_owned()], None, crate::session::now_ms())
        .await;
    room.start("ada").await.unwrap();
    let commands = runtime_commands(&fixture.root);
    assert!(
        !commands
            .iter()
            .any(|line| line.starts_with("start ") || line.starts_with("create ")),
        "not started again: {commands:?}"
    );
    assert_eq!(taken.forms(), [2]);
    assert!(taken.sets()[0]["GITHUB_TOKEN"]["use"].is_object());
    assert_eq!(
        protection(&room).await,
        Some(crate::contract::ComputerProtection::Protected)
    );
    let entry = kept_attest(&fixture.store).expect("written as it was taken");
    assert_eq!(entry["bearer"], bearer_hash("fixture-persisted-token"));
    assert_eq!(entry["boot"], "fake-boot");
    assert_eq!(taken.key_asked.load(SeqCst), 1);
}

/// A computer made again has a bearer of its own, and the key kept for the
/// one before goes with it: the entry is written again for the new bearer.
#[tokio::test]
async fn a_computer_made_again_forgets_the_kept_key() {
    use std::sync::atomic::Ordering::SeqCst;
    let fixture = kept_computer_room("made-again-v2", Some("0.13.0")).await;
    let taken = fixture.taken.clone();
    let room = fixture.room.clone();
    room.start("ada").await.unwrap();
    let before = room.computers.running("ada").await.unwrap().token;
    assert_eq!(
        kept_attest(&fixture.store).unwrap()["bearer"],
        bearer_hash(&before)
    );
    let session = room.session("ada").unwrap();
    // The container made again boots with a key nobody has taken.
    taken.key_taken.store(false, SeqCst);
    room.computer_restart("ada").await.unwrap();
    let swapped = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let replaced = room
                .session("ada")
                .is_ok_and(|now| !Arc::ptr_eq(&now, &session) && now.computer);
            if replaced && room.info("ada").state == SessionState::Ready {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        swapped.is_ok(),
        "the teammate carries on with a new computer"
    );
    let after = room.computers.running("ada").await.unwrap().token;
    assert_ne!(after, before, "made again, with a bearer of its own");
    let entry = kept_attest(&fixture.store).expect("the new boot's key is kept");
    assert_eq!(entry["bearer"], bearer_hash(&after));
    assert_ne!(entry["bearer"], bearer_hash(&before));
    assert_eq!(taken.key_asked.load(SeqCst), 2);

    // Removed, it is gone from the store.
    room.computer_remove("ada").await.unwrap();
    assert_eq!(kept_attest(&fixture.store), None);
}

/// A computer made on a release before 0.13 runs everything as one user,
/// so a job there can hold the bearer, take the port and answer as a split
/// 0.13.0-rc.3, key door and all. It is handed nothing, needs an update,
/// and the pane is told why.
#[tokio::test]
async fn an_old_computer_that_claims_to_split_is_handed_nothing() {
    use std::sync::atomic::Ordering::SeqCst;
    // What answers the port says it is 0.13 (the fake's own claim); the
    // container was made on 0.12.0, as its label says.
    let ComputerRoom { room, taken, .. } =
        computer_room("old-claims-split", Some("0.13.0"), TWO_RELEASES, true).await;
    let mut ada = room.persona("ada").unwrap();
    ada.computer.as_mut().unwrap().image = Some("hotline-computer:0.12.0".into());
    enrol(&room.log, &ada);
    grant_a_key_a_variable_and_a_login(&room);
    room.start("ada").await.unwrap();
    assert!(taken.sets().is_empty(), "no set taken: {:?}", taken.sets());
    assert_eq!(taken.key_asked.load(SeqCst), 0, "its key door never asked");
    assert_eq!(
        protection(&room).await,
        Some(crate::contract::ComputerProtection::NeedsUpdate)
    );
    let said = lock(&room.secret_notices).clone();
    assert!(
        said.iter()
            .any(|text| text.contains(crate::computer::secrets::UNPROVEN)),
        "{said:?}"
    );
}

/// Whether no update, restart or swap of the teammate's computer is under
/// way: the swap has finished, whatever it came to (a start that failed
/// leaves the reason for the pane).
fn settled(room: &Room) -> bool {
    use crate::session::computer::ComputerPhase;
    !matches!(
        room.computer_links.phase("ada"),
        ComputerPhase::Preparing { .. }
            | ComputerPhase::Swap { .. }
            | ComputerPhase::Swapping { .. }
    )
}

/// Waits for the teammate to carry on with a session other than `before`,
/// on a computer, as a swap between turns leaves it, and for the swap to
/// have finished.
async fn carried_on(room: &Arc<Room>, before: &Arc<Session>) {
    let swapped = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let replaced = room
                .session("ada")
                .is_ok_and(|now| !Arc::ptr_eq(&now, before) && now.computer);
            if replaced && room.info("ada").state == SessionState::Ready && settled(room) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        swapped.is_ok(),
        "the teammate carries on with a new computer"
    );
}

/// The bearer the scripted runtime made the container there now with.
fn made_with(fixture: &ComputerRoom) -> String {
    std::fs::read_to_string(fixture.root.join("state.envtoken")).unwrap_or_default()
}

/// What the desk did with the computer `bearer` opens, once it is made:
/// every request since carried that bearer or met no container at all, the
/// desk presents it and keeps the boot's key under it, the computer proved
/// itself under the key its own boot handed, and a protected key reached
/// it. Each bearer before it is refused by it, and a `Ready` still holding
/// one hands it nothing.
/// `seen` is how many requests the fake had noted when the last such look
/// ended, and is moved past this one's own.
async fn only_the_new_bearer(
    fixture: &ComputerRoom,
    before: &[String],
    seen: &mut usize,
) -> String {
    let (room, taken) = (&fixture.room, &fixture.taken);
    let bearer = made_with(fixture);
    assert_ne!(bearer, "");
    assert!(!before.contains(&bearer), "a bearer of its own: {before:?}");
    let ready = room.computers.running("ada").await.unwrap();
    assert_eq!(ready.token, bearer, "the desk presents the new bearer");
    assert_eq!(
        kept_attest(&fixture.store).unwrap()["bearer"],
        bearer_hash(&bearer),
        "the new boot's key, under the new bearer"
    );
    assert_eq!(
        protection(room).await,
        Some(crate::contract::ComputerProtection::Protected)
    );
    let sets = taken.sets().len();
    room.secrets_changed().await;
    assert_eq!(taken.sets().len(), sets + 1);
    assert!(taken.sets().last().unwrap()["GITHUB_TOKEN"]["use"].is_object());
    let reused: Vec<_> = taken.bearers()[*seen..]
        .iter()
        .filter(|(presented, made)| !made.is_empty() && presented != made)
        .cloned()
        .collect();
    assert!(reused.is_empty(), "an old bearer was presented: {reused:?}");

    for old in before {
        let refused = reqwest::Client::new()
            .get(format!(
                "{}/capabilities?nonce=old",
                ready.url.strip_suffix("/mcp").unwrap()
            ))
            .bearer_auth(old)
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(refused, reqwest::StatusCode::UNAUTHORIZED);
        let stale = crate::computer::Ready {
            made_to_split: true,
            ..crate::computer::Ready::new(ready.url.clone(), old.clone())
        };
        assert!(
            crate::computer::secrets::deliver(&stale, &std::collections::BTreeMap::new())
                .await
                .is_err()
        );
    }
    assert_eq!(
        taken.sets().len(),
        sets + 1,
        "nothing taken under an old one"
    );
    *seen = taken.bearers().len();
    bearer
}

/// Every way the desk makes a computer again gives it a bearer of its own:
/// Update, a restart for new limits between turns or at the next start,
/// and a removal the runtime refused followed by the next one that worked. The computer made opens only to
/// its own bearer, the desk presents only that one from then on (never one
/// a computer before it had, whose programs may have read it), the boot's
/// v2 key is taken and kept under it, and a protected key goes to it once
/// it proved itself with that key.
#[tokio::test]
async fn every_way_a_computer_is_made_again_gives_it_a_bearer_the_old_ones_do_not_open() {
    let fixture = kept_computer_room("remade-bearers", Some("0.13.0")).await;
    let (room, taken) = (fixture.room.clone(), fixture.taken.clone());
    taken.opens_only_to(fixture.root.join("state.envtoken"));
    grant_a_key_a_variable_and_a_login(&room);
    room.start("ada").await.unwrap();
    let mut seen = 0;
    let mut bearers = vec![only_the_new_bearer(&fixture, &[], &mut seen).await];

    // Update, with the teammate running between turns.
    let session = room.session("ada").unwrap();
    room.computer_update("ada").await.unwrap();
    carried_on(&room, &session).await;
    bearers.push(only_the_new_bearer(&fixture, &bearers, &mut seen).await);

    // Made again for its limits, between turns.
    let session = room.session("ada").unwrap();
    room.computer_restart("ada").await.unwrap();
    carried_on(&room, &session).await;
    bearers.push(only_the_new_bearer(&fixture, &bearers, &mut seen).await);

    // Made again for its limits at the next start, with the desk still
    // holding the stopped computer's bearer.
    room.stop("ada").unwrap();
    room.computer_restart("ada").await.unwrap();
    let mut ada = room.persona("ada").unwrap();
    ada.computer.as_mut().unwrap().memory = Some("2048m".into());
    enrol(&room.log, &ada);
    let removes = commands_starting(&fixture.root, "rm ");
    room.start("ada").await.unwrap();
    assert_eq!(commands_starting(&fixture.root, "rm "), removes + 1);
    bearers.push(only_the_new_bearer(&fixture, &bearers, &mut seen).await);

    // The runtime will not remove it: the swap waits, the desk lets go of
    // the bearer, and nothing is handed to the computer meanwhile.
    std::fs::write(fixture.root.join("state.rmfail"), "").unwrap();
    let session = room.session("ada").unwrap();
    let removes = commands_starting(&fixture.root, "rm ");
    let stops = commands_starting(&fixture.root, "stop ");
    room.computer_restart("ada").await.unwrap();
    // The swap has given up once the refused removal stopped the computer
    // and the swap waits again: the desk lets go of the bearer before it
    // asks the runtime, so that alone says nothing of where the swap is.
    tokio::time::timeout(Duration::from_secs(10), async {
        while commands_starting(&fixture.root, "stop ") == stops
            || !room.computer_links.swap_waiting("ada")
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the swap gives up and waits");
    assert_eq!(commands_starting(&fixture.root, "rm "), removes + 1);
    assert!(
        room.computers.running("ada").await.is_none(),
        "the bearer is let go"
    );
    assert_ne!(
        protection(&room).await,
        Some(crate::contract::ComputerProtection::Protected)
    );
    let sets = taken.sets().len();
    room.secrets_changed().await;
    assert_eq!(taken.sets().len(), sets, "nothing handed to it");

    // The next try removes it, and the computer made then has a new one.
    std::fs::remove_file(fixture.root.join("state.rmfail")).unwrap();
    room.swap_computer_when_idle("ada");
    carried_on(&room, &session).await;
    only_the_new_bearer(&fixture, &bearers, &mut seen).await;
}

/// A computer that proved itself with the v2 key its boot handed this desk
/// cannot be stepped down to an older proof: answering as rc.2 does (v1,
/// under the bearer the desk's asking carried), split or legacy, or with a
/// v2 proof another key made, it is no longer said to be protected, and it
/// is handed nothing — not the protected key, and not what a legacy
/// computer gets — until it proves itself on v2 again. Saying legacy under
/// its own key, as a split computer whose key proxy did not start does, it
/// is handed what a legacy computer is, the key waiting. A computer that
/// never offered v2 is told by its own answer: one too old for
/// `/capabilities` (`a_protected_key_never_reaches_an_old_computer`) or
/// legacy (`a_login_reaches_a_legacy_computer_as_before_and_is_marked_unprotected`)
/// is handed what 0.49 handed it, never a protected key, and labelled so;
/// a v1 claim of split is handed nothing
/// (`a_v1_claim_of_split_is_handed_nothing_whatever_it_attests`).
#[tokio::test]
async fn a_computer_stepped_down_from_v2_is_not_protected_and_is_handed_nothing() {
    use std::sync::atomic::Ordering::SeqCst;
    let fixture = kept_computer_room("stepped-down", Some("0.13.0")).await;
    let (room, taken) = (fixture.room.clone(), fixture.taken.clone());
    grant_a_key_a_variable_and_a_login(&room);
    room.start("ada").await.unwrap();
    assert_eq!(taken.forms(), [2]);
    assert_eq!(
        protection(&room).await,
        Some(crate::contract::ComputerProtection::Protected)
    );

    for (v1, legacy, forged) in [
        (true, false, false),
        (true, true, false),
        (false, false, true),
        (false, true, true),
    ] {
        let what = format!("v1 {v1}, legacy {legacy}, forged {forged}");
        taken.v1.store(v1, SeqCst);
        taken.legacy.store(legacy, SeqCst);
        taken.forged.store(forged, SeqCst);
        room.secrets_changed().await;
        assert_eq!(taken.forms(), [2], "nothing handed: {what}");
        assert_eq!(
            protection(&room).await,
            Some(crate::contract::ComputerProtection::NeedsUpdate),
            "{what}"
        );
        assert!(
            lock(&room.secret_notices)
                .last()
                .is_some_and(|text| text.contains(crate::computer::secrets::UNPROVEN)),
            "{what}"
        );
    }

    // Its key proxy did not start: legacy, under its own key.
    taken.v1.store(false, SeqCst);
    taken.forged.store(false, SeqCst);
    room.secrets_changed().await;
    assert_eq!(taken.forms(), [2, 1]);
    assert!(!taken.sets()[1].contains_key("GITHUB_TOKEN"));
    assert_eq!(taken.sets()[1]["GITHUB_LOGIN"]["kind"], "login");
    assert_eq!(
        protection(&room).await,
        Some(crate::contract::ComputerProtection::NeedsUpdate)
    );

    // The computer that handed the key answers split again: protected.
    taken.legacy.store(false, SeqCst);
    room.secrets_changed().await;
    assert_eq!(taken.forms(), [2, 1, 2]);
    assert_eq!(
        protection(&room).await,
        Some(crate::contract::ComputerProtection::Protected)
    );
}

/// An update that fails part way never leaves a protected label on a
/// computer that has not proved itself, nor hands one a protected key: a
/// download that fails keeps the computer it has, still protected; a new
/// computer that will not start leaves no computer and no label; one whose
/// boot's key something else took first is handed what a legacy one is,
/// the key waiting; one that forges its proof is handed nothing. Each is
/// said to need an update or a restart, and the next that proves itself
/// takes the key.
#[tokio::test]
async fn an_update_that_fails_part_way_keeps_the_label_right_and_the_key_back() {
    use std::sync::atomic::Ordering::SeqCst;
    let fixture = kept_computer_room("update-part-way", Some("0.13.0")).await;
    let (room, taken, root) = (
        fixture.room.clone(),
        fixture.taken.clone(),
        fixture.root.clone(),
    );
    taken.opens_only_to(root.join("state.envtoken"));
    grant_a_key_a_variable_and_a_login(&room);
    room.start("ada").await.unwrap();
    let first = made_with(&fixture);
    let protected = Some(crate::contract::ComputerProtection::Protected);
    let needs_update = Some(crate::contract::ComputerProtection::NeedsUpdate);
    assert_eq!(protection(&room).await, protected);
    let keys_handed = |taken: &crate::computer::guide::fake::Taken| {
        taken
            .sets()
            .iter()
            .filter(|set| set.contains_key("GITHUB_TOKEN"))
            .count()
    };
    assert_eq!(keys_handed(&taken), 1);

    // The download fails: the computer it has stays, proved and protected.
    std::fs::write(root.join("state.noimage"), "").unwrap();
    std::fs::write(root.join("state.pullfail"), "").unwrap();
    room.computer_update("ada").await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while room
            .computer_status("ada")
            .await
            .unwrap()
            .update_failed
            .is_none()
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the pane is told");
    assert_eq!(made_with(&fixture), first, "the same computer");
    assert_eq!(protection(&room).await, protected);
    room.secrets_changed().await;
    assert_eq!(taken.forms(), [2, 2], "it proved itself again first");
    std::fs::remove_file(root.join("state.pullfail")).unwrap();
    std::fs::remove_file(root.join("state.noimage")).unwrap();

    // The old one goes and the new one will not start: the teammate goes
    // on without a computer, and there is no label, no key kept, nothing
    // handed. The rebuild takes the session out before it starts the new
    // computer, so the session going is not the start having failed: the
    // swap ending, with a session and no computer, is.
    std::fs::write(root.join("state.startfail"), "").unwrap();
    room.computer_update("ada").await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !settled(&room) || room.session("ada").is_ok_and(|session| session.computer) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the teammate goes on without a computer");
    assert!(room.computers.running("ada").await.is_none());
    assert_eq!(room.computer_status("ada").await.unwrap().protection, None);
    assert_eq!(
        kept_attest(&fixture.store),
        None,
        "forgotten with the old one"
    );
    room.secrets_changed().await;
    assert_eq!(taken.forms(), [2, 2]);
    std::fs::remove_file(root.join("state.startfail")).unwrap();

    // It starts, but its boot's key was taken before the desk asked: it
    // cannot be checked, so the key waits and it is not protected.
    taken.keys_taken_first.store(true, SeqCst);
    room.computer_start("ada").await.unwrap();
    assert_ne!(made_with(&fixture), first);
    assert_eq!(protection(&room).await, needs_update);
    assert_eq!(taken.forms(), [2, 2, 1]);
    assert_eq!(keys_handed(&taken), 2, "the key stayed on the desk");
    assert!(
        lock(&room.secret_notices)
            .last()
            .is_some_and(|text| text.contains("cannot be checked until it starts again"))
    );

    // Made again on a computer that forges its proof: nothing at all.
    taken.keys_taken_first.store(false, SeqCst);
    taken.forged.store(true, SeqCst);
    let session = room.session("ada").unwrap();
    room.computer_restart("ada").await.unwrap();
    carried_on(&room, &session).await;
    assert_eq!(taken.forms(), [2, 2, 1], "nothing handed");
    assert_eq!(protection(&room).await, needs_update);

    // The next computer proves itself with its own boot's key: protected,
    // and the key goes to it.
    taken.forged.store(false, SeqCst);
    let session = room.session("ada").unwrap();
    room.computer_restart("ada").await.unwrap();
    carried_on(&room, &session).await;
    assert_eq!(protection(&room).await, protected);
    assert_eq!(taken.forms(), [2, 2, 1, 2]);
    assert_eq!(keys_handed(&taken), 3);
}
