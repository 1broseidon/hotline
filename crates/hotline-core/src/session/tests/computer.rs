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

/// Grants `names` to ada's computer on the room's record, the way
/// `persona.update` would, without reattaching anything.
fn grant_secrets(room: &Room, names: &[&str]) {
    let mut ada = room.persona("ada").unwrap();
    let computer = ada.computer.as_mut().unwrap();
    computer.secrets = Some(names.iter().map(ToString::to_string).collect());
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

    // The agent hears the names and that it will never see a value; the
    // value itself is on no tape and in no preamble.
    let heard = lock(&agents.preambles)[0].clone();
    assert!(
        heard.contains("by name: GITHUB_TOKEN (a variable), GONE_TOKEN (not stored right now)."),
        "{heard}"
    );
    assert!(heard.contains("You never see a value"), "{heard}");
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
        computer_room("computer-passkey", Some("0.11.0"), TWO_RELEASES, true).await;
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
        ada.computer.unwrap().secrets.as_deref(),
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
    } = computer_room("computer-login", Some("0.11.0"), TWO_RELEASES, true).await;
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
    assert_eq!(sets[0]["GITHUB_TOKEN"], "ghp_notarealtoken0001");
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

/// What was brought over is listed from the room's record, and taken back
/// by site or whole: the computer is told the exact domains and drops them,
/// the record follows, and an entry with nothing left goes. A site that was
/// never brought over is refused, and a release from before the door is
/// named with the pane's Update.
#[tokio::test]
async fn brought_over_cookies_are_listed_and_taken_back_by_site_or_whole() {
    use crate::contract::{CookieImport, CookieSite};
    let ComputerRoom { room, taken, .. } = computer_room(
        "computer-cookies-forget",
        Some("0.11.0"),
        TWO_RELEASES,
        true,
    )
    .await;
    assert_eq!(room.computer_cookies_list("ada"), [] as [CookieImport; 0]);
    let recorded = vec![CookieImport {
        browser_id: "chrome".to_string(),
        browser_name: "Google Chrome".to_string(),
        profile_id: "Default".to_string(),
        profile_name: "Default".to_string(),
        imported_at: 1,
        sites: vec![
            CookieSite {
                domain: "github.com".to_string(),
                cookies: 3,
            },
            CookieSite {
                domain: "gitlab.com".to_string(),
                cookies: 2,
            },
        ],
    }];
    room::record_cookie_imports(&room.log, "ada", &recorded).unwrap();
    assert_eq!(room.computer_cookies_list("ada"), recorded);

    let refused = room
        .computer_cookies_forget("ada", "chrome", "Default", Some("example.com"))
        .await
        .unwrap_err();
    assert!(refused.contains("not among the sites"), "{refused}");
    assert!(
        room.computer_cookies_forget("ada", "firefox", "default", None)
            .await
            .is_err()
    );
    assert!(
        taken.forgotten().is_empty(),
        "nothing asked of the computer yet"
    );

    // One site: the computer is told that domain, and the record loses it.
    let left = room
        .computer_cookies_forget("ada", "chrome", "Default", Some("github.com"))
        .await
        .unwrap();
    assert_eq!(
        taken.forgotten(),
        vec![(
            "import-chrome-Default".to_string(),
            vec!["github.com".to_string()]
        )]
    );
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].sites.len(), 1);
    assert_eq!(left[0].sites[0].domain, "gitlab.com");
    assert_eq!(room.computer_cookies_list("ada"), left);

    // The rest: every remaining domain is named, and the entry goes.
    let left = room
        .computer_cookies_forget("ada", "chrome", "Default", None)
        .await
        .unwrap();
    assert_eq!(left, [] as [CookieImport; 0]);
    assert_eq!(
        taken.forgotten()[1],
        (
            "import-chrome-Default".to_string(),
            vec!["gitlab.com".to_string()]
        )
    );
    assert_eq!(room.computer_cookies_list("ada"), [] as [CookieImport; 0]);

    // A release from before the door: 404, said as the pane's Update.
    let ComputerRoom { room, taken, .. } = computer_room(
        "computer-cookies-forget-old",
        Some("0.8.0"),
        TWO_RELEASES,
        true,
    )
    .await;
    room::record_cookie_imports(&room.log, "ada", &recorded).unwrap();
    let refused = room
        .computer_cookies_forget("ada", "chrome", "Default", None)
        .await
        .unwrap_err();
    assert!(refused.contains("cannot take cookies back"), "{refused}");
    assert_eq!(taken.forgotten(), [] as [(String, Vec<String>); 0]);
    assert_eq!(
        room.computer_cookies_list("ada"),
        recorded,
        "nothing dropped, nothing forgotten"
    );
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
        Some("0.11.0"),
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
        room.persona("ada")
            .unwrap()
            .computer
            .unwrap()
            .secrets
            .as_deref(),
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
            .computer
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

    let entry = crate::skills::computer_entry(&cwd).expect("listed");
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
    let unheard = preamble(&bob, Some(Reach::Workspace), None, &[]);
    assert!(!unheard.contains("hotline-computer"), "{unheard}");
    assert!(unheard.contains("\n- hotline-room: "), "{unheard}");

    // The pane sees the release running against the one it would be made
    // on now, which is the image tag the teammate asked for.
    let status = room.computer_status("ada").await.unwrap();
    assert_eq!(status.release.as_deref(), Some("0.11.0"));
    assert_eq!(status.available.as_deref(), Some("test"));
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
    crate::skills::write_computer(&cwd, "0.1.0", "stale", "stale guide").unwrap();
    room.start("ada").await.unwrap();

    assert!(!cwd.join(".agents/skills/hotline-computer").exists());
    assert!(crate::skills::computer_entry(&cwd).is_none());
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
            if lock(&room.computer_swaps)
                .get("ada")
                .is_some_and(|swap| swap.ready)
            {
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
        lock(&room.computer_swaps)
            .get("ada")
            .is_some_and(|swap| swap.ready),
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
    older.room.start("ada").await.unwrap();
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
    assert_eq!(
        room.computer_cookies_list("ada")[0].browser_name,
        "firefox (laptop)"
    );
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
        "0.11.2",
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
