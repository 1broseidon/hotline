//! Cookie offers for a teammate with the desk browser instead of a
//! computer: the site its browser is on raises the same card, Not now
//! quiets it the same day, Use lends into its own web storage and reloads
//! the page, and Remove takes the site's cookies back out.
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

fn delivered(room: &Arc<Room>, words: &str) -> bool {
    room.log()
        .load(&StreamId::Tape("ada".into()))
        .iter()
        .any(|e| e["kind"] == "delivery" && e["text"].as_str().is_some_and(|s| s.contains(words)))
}

#[tokio::test]
async fn the_site_the_desk_browser_is_on_raises_one_offer_and_not_now_quiets_it_for_a_day() {
    let browser = Arc::new(Browser::default());
    let (room, _profiles) = desk_browser_cookie_room("desk-offer-once", browser.clone()).await;
    room.poll_secret_uses("ada").await;
    assert_eq!(
        room.computer_cookie_offers("ada"),
        [] as [CookieOffer; 0],
        "a browser on no page is offered nothing"
    );
    browser.visit("https://no-cookies.example/");
    room.poll_secret_uses("ada").await;
    assert_eq!(room.computer_cookie_offers("ada"), [] as [CookieOffer; 0]);
    browser.visit("https://account.porkbun.com/login");
    room.poll_secret_uses("ada").await;
    room.poll_secret_uses("ada").await;
    let cards = room.computer_cookie_offers("ada");
    assert_eq!(cards.len(), 1, "one card, however often it is looked at");
    assert_eq!(cards[0].site, "porkbun.com");
    assert_eq!(cards[0].status, CookieOfferStatus::Pending);
    assert!(cards[0].desk_browser);

    assert_eq!(answer(&room, &cards[0].id, "not_now").await["ok"], true);
    room.stop("ada").unwrap();
    room.start("ada").await.unwrap();
    room.poll_secret_uses("ada").await;
    assert_eq!(
        room.computer_cookie_offers("ada").len(),
        1,
        "Not now holds through a restart"
    );
    let mut aged = serde_json::to_value(&room.computer_cookie_offers("ada")[0]).unwrap();
    aged["kind"] = json!("cookie_offer");
    aged["ts"] = json!(crate::session::now_ms() - crate::session::NOT_NOW_QUIET_MS - 60_000);
    room.log()
        .append(&StreamId::Tape("ada".into()), &aged)
        .unwrap();
    room.stop("ada").unwrap();
    room.start("ada").await.unwrap();
    room.poll_secret_uses("ada").await;
    assert_eq!(
        room.computer_cookie_offers("ada").len(),
        2,
        "a day on, the site is offered again"
    );
    assert!(browser.lent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn use_lends_the_site_into_the_browser_and_reloads_and_remove_takes_it_back() {
    let browser = Arc::new(Browser::default());
    let (room, _profiles) = desk_browser_cookie_room("desk-offer-use", browser.clone()).await;
    browser.visit("https://porkbun.com/");
    room.poll_secret_uses("ada").await;
    let offer = room.computer_cookie_offers("ada")[0].clone();
    assert_eq!(answer(&room, &offer.id, "use").await["ok"], true);
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
    let card = room.computer_cookie_offers("ada")[0].clone();
    assert_eq!(card.status, CookieOfferStatus::Lent);
    assert!(card.desk_browser);
    assert!(delivered(&room, "Your browser is signed in as them"));
    room.poll_secret_uses("ada").await;
    assert_eq!(
        room.computer_cookie_offers("ada").len(),
        1,
        "a lent site is not offered again"
    );

    assert_eq!(
        command(
            &room,
            Seat::Phone,
            "computer.cookies.remove",
            json!({"personaId":"ada","id":offer.id})
        )
        .await["ok"],
        true
    );
    assert_eq!(*browser.forgotten.lock().unwrap(), ["porkbun.com"]);
    assert_eq!(
        room.computer_cookie_offers("ada")[0].status,
        CookieOfferStatus::Removed
    );
    assert_eq!(*browser.reloads.lock().unwrap(), 2);
    assert!(delivered(&room, "removed from your browser"));
}

#[tokio::test]
async fn an_offer_for_the_browser_expires_once_the_teammate_has_a_computer_instead() {
    let browser = Arc::new(Browser::default());
    let (room, _profiles) = desk_browser_cookie_room("desk-offer-moved", browser.clone()).await;
    browser.visit("https://porkbun.com/");
    room.poll_secret_uses("ada").await;
    let id = room.computer_cookie_offers("ada")[0].id.clone();
    let mut ada = crate::room::roster(room.log())
        .into_iter()
        .find(|p| p.id == "ada")
        .unwrap();
    ada.computer = Some(serde_json::from_value(json!({ "enabled": true })).unwrap());
    crate::room::append_persona(room.log(), &ada).unwrap();
    assert_eq!(answer(&room, &id, "use").await["ok"], false);
    assert_eq!(
        room.computer_cookie_offers("ada")[0].status,
        CookieOfferStatus::Expired
    );
    assert!(browser.lent.lock().unwrap().is_empty());
}
