//! A cookie ask from a teammate with the desk browser instead of a
//! computer: the same card and answers, with Use Cookies sharing into its
//! own web storage and reloading the page, Sign In giving its browser a
//! login, and Open Browser handing over its pane.
use super::*;
use crate::browser::{Act, Acted, DeskBrowser, Page, Reading, Step};
use crate::session::tests::desk_browser_cookie_room;
use std::sync::Mutex as StdMutex;

#[derive(Default)]
struct Browser {
    page: StdMutex<Option<String>>,
    lent: StdMutex<Vec<Vec<Value>>>,
    forgotten: StdMutex<Vec<String>>,
    reloads: StdMutex<usize>,
}

impl Browser {
    fn visit(&self, url: &str) {
        *self.page.lock().unwrap() = Some(url.into());
    }

    fn here(&self) -> Page {
        Page {
            url: self.page.lock().unwrap().clone().unwrap_or_default(),
            title: "Porkbun".into(),
            loaded: true,
        }
    }
}

#[async_trait::async_trait]
impl DeskBrowser for Browser {
    async fn open(&self, _persona_id: &str, url: &url::Url) -> Result<Page, String> {
        self.visit(url.as_str());
        Ok(self.here())
    }
    async fn step(&self, _persona_id: &str, step: Step) -> Result<Page, String> {
        if step == Step::Reload {
            *self.reloads.lock().unwrap() += 1;
        }
        Ok(self.here())
    }
    async fn read(&self, _persona_id: &str, _reading: Reading) -> Result<String, String> {
        Ok("Porkbun".into())
    }
    async fn act(&self, _persona_id: &str, _act: &Act) -> Result<Acted, String> {
        Ok(Acted {
            page: self.here(),
            note: None,
        })
    }
    async fn screenshot(&self, _persona_id: &str) -> Result<Vec<u8>, String> {
        Err("No picture here.".into())
    }
    fn current(&self, _persona_id: &str) -> Option<String> {
        self.page.lock().unwrap().clone()
    }
    async fn lend_cookies(&self, _persona_id: &str, cookies: &[Value]) -> Result<(), String> {
        self.lent.lock().unwrap().push(cookies.to_vec());
        Ok(())
    }
    async fn forget_cookies(&self, _persona_id: &str, site: &str) -> Result<(), String> {
        self.forgotten.lock().unwrap().push(site.into());
        Ok(())
    }
    async fn close(&self, _persona_id: &str, _wipe: bool) -> Result<(), String> {
        *self.page.lock().unwrap() = None;
        Ok(())
    }
}

#[tokio::test]
async fn a_desk_browser_on_a_signed_in_site_raises_no_card_and_reads_no_browser() {
    use std::sync::atomic::Ordering;
    let browser = Arc::new(Browser::default());
    let (room, _profiles) = desk_browser_cookie_room("desk-ask-no-offer", browser.clone()).await;
    browser.visit("https://account.porkbun.com/login");
    for _ in 0..3 {
        room.poll_secret_uses("ada").await;
    }
    assert_eq!(asks(&room), [] as [Value; 0]);
    assert_eq!(room.computer_cookie_offers("ada"), [] as [CookieOffer; 0]);
    assert_eq!(room.cookie_discovery_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn use_cookies_shares_into_the_browser_and_reloads_and_remove_takes_it_back() {
    let browser = Arc::new(Browser::default());
    let (room, _profiles) = desk_browser_cookie_room("desk-ask-use", browser.clone()).await;
    browser.visit("https://porkbun.com/");
    ask(&room, "porkbun.com").await.unwrap();
    let card = asks(&room).remove(0);
    assert_eq!(card["deskBrowser"], true);
    let id = card["id"].as_str().unwrap().to_string();
    assert_eq!(answer(&room, &id, "use").await["ok"], true);
    let lent = browser.lent.lock().unwrap().clone();
    assert_eq!(lent.len(), 1);
    let domains: Vec<_> = lent[0]
        .iter()
        .map(|c| c["domain"].as_str().unwrap())
        .collect();
    assert_eq!(domains, [".porkbun.com", "account.porkbun.com"]);
    assert_eq!(
        *browser.reloads.lock().unwrap(),
        1,
        "the page reloads signed in"
    );
    let settled = asks(&room).remove(0);
    assert_eq!(settled["status"], "cookies");
    assert_eq!(settled["browserName"], "Firefox");
    let lend = room.computer_cookie_offers("ada")[0].clone();
    assert_eq!(lend.status, CookieOfferStatus::Lent);
    assert!(lend.desk_browser);
    heard(&room, "Your browser is signed in as them").await;

    assert_eq!(
        command(
            &room,
            Seat::Phone,
            "computer.cookies.remove",
            json!({"personaId":"ada","id":lend.id})
        )
        .await["ok"],
        true
    );
    assert_eq!(*browser.forgotten.lock().unwrap(), ["porkbun.com"]);
    assert_eq!(*browser.reloads.lock().unwrap(), 2);
}

#[tokio::test]
async fn open_browser_hands_over_the_browser_pane_and_tells_the_teammate() {
    let browser = Arc::new(Browser::default());
    let (room, _profiles) = desk_browser_cookie_room("desk-ask-open", browser.clone()).await;
    ask(&room, "porkbun.com").await.unwrap();
    let id = asks(&room)[0]["id"].as_str().unwrap().to_string();
    assert_eq!(answer(&room, &id, "open_browser").await["ok"], true);
    assert_eq!(asks(&room)[0]["status"], "opened");
    heard(
        &room,
        "opened your browser to sign in to porkbun.com themselves",
    )
    .await;
    assert!(browser.lent.lock().unwrap().is_empty());
}

async fn sign_in(room: &Arc<Room>, id: &str, save: bool) -> Value {
    command(
        room,
        Seat::Phone,
        "secrets.capture.answer",
        json!({"actionId":id,"name":"PORKBUN_COM","values":{"username":"person@example.com","password":"only-for-the-vault"},"ask":false,"save":save}),
    )
    .await
}

#[tokio::test]
async fn sign_in_stores_the_login_and_gives_it_to_the_browser_without_a_value_anywhere() {
    let browser = Arc::new(Browser::default());
    let (room, _profiles) = desk_browser_cookie_room("desk-ask-sign-in", browser.clone()).await;
    ask(&room, "https://porkbun.com/account/login")
        .await
        .unwrap();
    let card = asks(&room).remove(0);
    assert_eq!(card["name"], "PORKBUN_COM");
    let id = card["id"].as_str().unwrap().to_string();
    let reply = sign_in(&room, &id, true).await;
    assert_eq!(reply["ok"], true, "{reply}");
    let settled = asks(&room).remove(0);
    assert_eq!(settled["status"], "signed_in");
    assert_eq!(settled["storedAs"], "PORKBUN_COM");
    assert!(settled.get("keptUntil").is_none());
    let vault = room.vault_for_tests();
    let stored = vault.shared_secrets().unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(
        stored[0].sites.as_deref(),
        Some(&["https://porkbun.com".to_string()][..])
    );
    assert_eq!(granted(&room), Some(vec!["PORKBUN_COM".to_string()]));
    heard(&room, "stored as PORKBUN_COM").await;
    assert!(
        sign_in(&room, &id, true).await["ok"] == false,
        "settled once"
    );
    for sink in [
        reply.to_string(),
        serde_json::to_string(&room.log().load(&StreamId::Room)).unwrap(),
        serde_json::to_string(&room.log().load(&StreamId::Tape("ada".into()))).unwrap(),
    ] {
        assert!(!sink.contains("only-for-the-vault"));
        assert!(!sink.contains("person@example.com"));
    }
}

#[tokio::test]
async fn a_login_not_saved_to_secrets_leaves_the_vault_and_the_grant_after_its_hour() {
    let browser = Arc::new(Browser::default());
    let (room, _profiles) = desk_browser_cookie_room("desk-ask-unsaved", browser.clone()).await;
    ask(&room, "porkbun.com").await.unwrap();
    let id = asks(&room)[0]["id"].as_str().unwrap().to_string();
    assert_eq!(sign_in(&room, &id, false).await["ok"], true);
    let settled = asks(&room).remove(0);
    let until = settled["keptUntil"].as_i64().unwrap();
    heard(&room, "not saved to the person's Secrets").await;
    room.forget_unsaved_logins(until - 1).await;
    assert_eq!(room.vault_for_tests().shared_secrets().unwrap().len(), 1);
    room.forget_unsaved_logins(until).await;
    assert_eq!(room.vault_for_tests().shared_secrets().unwrap(), []);
    assert_eq!(granted(&room), Some(vec![]));
    let after = asks(&room).remove(0);
    assert_eq!(after["status"], "signed_in", "the line still says how");
    assert!(after.get("keptUntil").is_none());
}

fn granted(room: &Arc<Room>) -> Option<Vec<String>> {
    crate::room::roster(room.log())
        .into_iter()
        .find(|p| p.id == "ada")
        .unwrap()
        .secrets
}

#[tokio::test]
async fn a_server_desk_asks_for_a_login_or_its_browser_and_never_shares_cookies() {
    let browser = Arc::new(Browser::default());
    let (room, _profiles) = desk_browser_cookie_room("desk-ask-served", browser.clone()).await;
    // The person's Firefox here is signed in to porkbun.com, and a server
    // reads it all the same for nothing.
    room.share_no_cookies();
    browser.visit("https://porkbun.com/");
    let said = ask(&room, "porkbun.com").await.unwrap();
    assert!(
        said.contains("may give you a login or open your browser") && !said.contains("cookies"),
        "{said}"
    );
    let card = asks(&room).remove(0);
    assert_eq!(
        card["sources"],
        json!([]),
        "no Use Cookies on desk, linked desktop or phone"
    );
    assert_eq!(card["deskBrowser"], true);
    let id = card["id"].as_str().unwrap().to_string();
    let used = answer(&room, &id, "use").await;
    assert_ne!(used["ok"], true, "{used}");
    assert!(browser.lent.lock().unwrap().is_empty());

    // Settings has no Cookie Sources here, and none can be switched.
    let sources = command(&room, Seat::Desk, "computer.cookies.sources", json!({})).await;
    assert_eq!(
        sources["result"],
        json!({"sources": [], "blocked": [], "off": true})
    );
    let switched = command(
        &room,
        Seat::Desk,
        "computer.cookies.source.set",
        json!({"browserId": "firefox", "profileId": "Default", "on": false}),
    )
    .await;
    assert_ne!(switched["ok"], true, "{switched}");

    // Open Browser still hands the person the teammate's own browser.
    assert_eq!(answer(&room, &id, "open_browser").await["ok"], true);
    assert_eq!(asks(&room).remove(0)["status"], "opened");
}
