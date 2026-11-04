//! Passkeys in the desk browser: a page's `navigator.credentials` call as
//! the app hands it over, with the frame's origin from WebKit, answered by
//! the room from the vault.

use super::*;
use crate::browser::authenticator::{self, Request};
use crate::contract::PasskeyRegistrationState;
use crate::vault::{StoredSecret, Vault};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// A room whose vault holds GITHUB_PASSKEY, a passkey for github.com,
/// granted to Ada; `ask` stores it as an ask-me one. Answers the passkey's
/// public key as a site would keep it.
fn with_passkey(name: &str, ask: bool) -> (Arc<Room>, Arc<Vault>, Vec<u8>) {
    let log = Log::open(scratch(name));
    let mut ada = ada();
    ada.secrets = Some(vec!["GITHUB_PASSKEY".into()]);
    write_persona(&log, ada);
    write_persona(&log, bob());
    let vault = Arc::new(
        Vault::open_with_store(
            log.root(),
            log.clone(),
            Arc::new(crate::credentials::MemoryStore::default()),
        )
        .unwrap(),
    );
    let made =
        authenticator::creation("https://github.com", &creation_options("github.com")).unwrap();
    let (record, response) = authenticator::mint(&made).unwrap();
    vault.set_shared("GITHUB_PASSKEY", record).unwrap();
    if ask {
        vault
            .set_secret_use("GITHUB_PASSKEY", crate::contract::SecretUse::Ask)
            .unwrap();
    }
    let room = Room::new_with_vault(log, Arc::new(NoKeys), vault.clone());
    room.install_browser(Arc::new(Fake::default()), &[]);
    (room, vault, public_key(&response))
}

fn creation_options(rp_id: &str) -> Value {
    json!({
        "challenge": "Y2hhbGxlbmdl",
        "rp": { "id": rp_id, "name": "GitHub" },
        "user": { "id": "dXNlci0x", "name": "ada", "displayName": "Ada" },
        "pubKeyCredParams": [{ "type": "public-key", "alg": -7 }],
    })
}

fn public_key(created: &Value) -> Vec<u8> {
    let spki = URL_SAFE_NO_PAD
        .decode(created["response"]["publicKey"].as_str().unwrap())
        .unwrap();
    // The P-256 point after its 26-byte SubjectPublicKeyInfo prefix.
    spki[26..].to_vec()
}

fn sign_in(origin: &str, options: Value) -> Request {
    Request {
        create: false,
        origin: origin.into(),
        top_origin: origin.into(),
        options,
    }
}

/// What the signed client data says, after checking the signature against
/// the passkey's public key, as the site would.
fn verified(got: &Value, public_key: &[u8]) -> Value {
    use ring::signature::{ECDSA_P256_SHA256_ASN1, UnparsedPublicKey};
    use sha2::{Digest, Sha256};
    let field = |name: &str| {
        URL_SAFE_NO_PAD
            .decode(got["response"][name].as_str().unwrap())
            .unwrap()
    };
    let client = field("clientDataJSON");
    let mut signed = field("authenticatorData");
    signed.extend_from_slice(&Sha256::digest(&client));
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, public_key)
        .verify(&signed, &field("signature"))
        .expect("the site verifies the signature");
    serde_json::from_slice(&client).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_passkey_signs_for_the_origin_webkit_reports_never_the_one_the_page_claims() {
    let (room, _, public) = with_passkey("browser-passkey-origin", false);
    let asked = json!({ "challenge": "Y2hhbGxlbmdl", "rpId": "github.com" });
    let got = room
        .browser_passkey("ada", &sign_in("https://github.com", asked.clone()))
        .await
        .unwrap();
    let client = verified(&got, &public);
    assert_eq!(client["type"], "webauthn.get");
    assert_eq!(client["origin"], "https://github.com");
    assert_eq!(client["challenge"], "Y2hhbGxlbmdl");

    // A page elsewhere that writes github.com as its origin is still the
    // page elsewhere: the origin is the one WebKit reports for its frame.
    let mut claimed = asked.clone();
    claimed["origin"] = json!("https://github.com");
    let refused = room
        .browser_passkey("ada", &sign_in("https://github.com.evil.dev", claimed))
        .await
        .unwrap_err();
    assert_eq!(refused.name, "SecurityError", "{refused:?}");

    // A frame of another site inside github.com gets nothing either.
    let framed = Request {
        top_origin: "https://evil.dev".into(),
        ..sign_in("https://github.com", asked.clone())
    };
    let refused = room.browser_passkey("ada", &framed).await.unwrap_err();
    assert_eq!(refused.name, "NotAllowedError", "{refused:?}");

    // Not over plain HTTP, and not for a teammate not granted it.
    let refused = room
        .browser_passkey("ada", &sign_in("http://github.com", asked.clone()))
        .await
        .unwrap_err();
    assert_eq!(refused.name, "SecurityError", "{refused:?}");
    let refused = room
        .browser_passkey("bob", &sign_in("https://github.com", asked))
        .await
        .unwrap_err();
    assert_eq!(refused.name, "NotAllowedError", "{refused:?}");
    assert!(refused.message.contains("no passkey for github.com"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ask_me_passkey_holds_on_the_card_until_the_person_answers() {
    let (room, _, public) = with_passkey("browser-passkey-ask", true);
    let asked = json!({ "challenge": "Y2hhbGxlbmdl", "rpId": "github.com" });
    let request = sign_in("https://github.com", asked);
    let page = {
        let room = room.clone();
        let request = request.clone();
        tokio::spawn(async move { room.browser_passkey("ada", &request).await })
    };
    let card = guard_card(&room, "ada").await;
    let title = card["title"].as_str().unwrap();
    assert!(
        title.starts_with("Ada wants to use GITHUB_PASSKEY on https://github.com."),
        "{title}"
    );
    assert!(title.contains("sign in to github.com"), "{title}");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!page.is_finished(), "the page waits for the answer");
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), "deny")
        .await
        .unwrap();
    let refused = page.await.unwrap().unwrap_err();
    assert_eq!(refused.name, "NotAllowedError");

    let page = {
        let room = room.clone();
        let request = request.clone();
        tokio::spawn(async move { room.browser_passkey("ada", &request).await })
    };
    let card = loop {
        let card = guard_card(&room, "ada").await;
        if card["decision"].is_null() {
            break card;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), "approve")
        .await
        .unwrap();
    let got = page.await.unwrap().unwrap();
    assert_eq!(verified(&got, &public)["origin"], "https://github.com");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_passkey_is_made_only_under_an_arming_for_its_site_and_lands_in_the_vault() {
    let (room, vault, _) = with_passkey("browser-passkey-make", false);
    let make = |origin: &str, rp_id: &str| Request {
        create: true,
        origin: origin.into(),
        top_origin: origin.into(),
        options: creation_options(rp_id),
    };
    let refused = room
        .browser_passkey("ada", &make("https://gitlab.com", "gitlab.com"))
        .await
        .unwrap_err();
    assert_eq!(refused.name, "NotAllowedError");
    assert!(refused.message.contains("not armed"), "{refused:?}");

    let armed = room
        .secrets_passkey_register("GITLAB_PASSKEY", "ada", "gitlab.com")
        .await
        .expect("a teammate with a desk browser and no computer is armed");
    assert_eq!(armed.state, PasskeyRegistrationState::Armed);
    let refused = room
        .browser_passkey("ada", &make("https://github.com", "github.com"))
        .await
        .unwrap_err();
    assert!(
        refused.message.contains("not armed"),
        "an arming is for its one site: {refused:?}"
    );

    let page = {
        let room = room.clone();
        let request = make("https://gitlab.com", "gitlab.com");
        tokio::spawn(async move { room.browser_passkey("ada", &request).await })
    };
    let asked = loop {
        let now = room.secrets_passkey_registration("ada").await.unwrap();
        if now.state == PasskeyRegistrationState::Asked {
            break now.ask.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(asked.origin, "https://gitlab.com");
    assert_eq!(asked.user_name.as_deref(), Some("ada"));
    assert!(
        tape(&room, "ada")
            .iter()
            .any(|event| event["kind"] == "passkey_ask" && event["status"] == "pending"),
        "the card is on the tape"
    );
    assert!(!page.is_finished(), "nothing is made before the answer");
    room.secrets_passkey_answer("ada", &asked.id, true)
        .await
        .unwrap();
    let created = page.await.unwrap().unwrap();
    assert!(created["response"]["attestationObject"].is_string());

    let (values, _) = vault
        .shared_secret_values(&["GITLAB_PASSKEY".to_string()])
        .unwrap();
    let Some(StoredSecret::Passkey {
        rp_id, private_key, ..
    }) = values.get("GITLAB_PASSKEY")
    else {
        panic!("the minted passkey is in the vault: {:?}", values.keys());
    };
    assert_eq!(rp_id, "gitlab.com");
    assert!(
        !created.to_string().contains(private_key.as_str()),
        "the page gets the public key only"
    );
    assert!(
        crate::room::roster(room.log())
            .iter()
            .find(|persona| persona.id == "ada")
            .and_then(|persona| persona.secrets.clone())
            .unwrap()
            .contains(&"GITLAB_PASSKEY".to_string()),
        "ticked for the teammate"
    );
    let stored = room.secrets_passkey_registration("ada").await.unwrap();
    assert_eq!(stored.state, PasskeyRegistrationState::Stored);

    // Made once: the arming is spent, and the new passkey signs in.
    let refused = room
        .browser_passkey("ada", &make("https://gitlab.com", "gitlab.com"))
        .await
        .unwrap_err();
    assert!(refused.message.contains("not armed"), "{refused:?}");
    let got = room
        .browser_passkey(
            "ada",
            &sign_in(
                "https://gitlab.com",
                json!({ "challenge": "YWdhaW4", "rpId": "gitlab.com" }),
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        verified(&got, &public_key(&created))["origin"],
        "https://gitlab.com"
    );

    // A denial ends the arming with nothing made.
    room.secrets_passkey_register("GITLAB_TWO", "ada", "gitlab.com")
        .await
        .unwrap();
    let page = {
        let room = room.clone();
        let request = make("https://gitlab.com", "gitlab.com");
        tokio::spawn(async move { room.browser_passkey("ada", &request).await })
    };
    let asked = loop {
        let now = room.secrets_passkey_registration("ada").await.unwrap();
        if let Some(ask) = now.ask {
            break ask;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    room.secrets_passkey_answer("ada", &asked.id, false)
        .await
        .unwrap();
    assert_eq!(page.await.unwrap().unwrap_err().name, "NotAllowedError");
    assert!(
        vault
            .shared_secret_values(&["GITLAB_TWO".to_string()])
            .unwrap()
            .0
            .is_empty()
    );
}
