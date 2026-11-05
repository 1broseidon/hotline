//! Shared cookies through the real room: the Firefox fixture is the person's
//! browser (porkbun.com and account.porkbun.com signed in, values that are
//! sentinels), a fake desk browser stands for the teammate's own WebKit
//! store, and the scripted computer for its container.
use super::*;
use crate::browser::{Act, Acted, DeskBrowser, Page, Reading, Step};
use crate::session::tests::{
    DeskKeys, Fake, Scripted, cookie_offer_room, desk_browser_cookie_room,
};
use std::sync::Mutex as StdMutex;
use std::sync::atomic::Ordering;

#[derive(Default)]
struct Browser {
    lent: StdMutex<Vec<Vec<Value>>>,
    forgotten: StdMutex<Vec<String>>,
    reloads: StdMutex<usize>,
}

impl Browser {
    fn lent(&self) -> Vec<Vec<Value>> {
        self.lent.lock().unwrap().clone()
    }
    fn forgotten(&self) -> Vec<String> {
        self.forgotten.lock().unwrap().clone()
    }
}

fn page() -> Page {
    Page {
        url: "https://porkbun.com/".into(),
        title: "Porkbun".into(),
        loaded: true,
    }
}

#[async_trait::async_trait]
impl DeskBrowser for Browser {
    async fn open(&self, _persona_id: &str, _url: &url::Url) -> Result<Page, String> {
        Ok(page())
    }
    async fn step(&self, _persona_id: &str, step: Step) -> Result<Page, String> {
        if step == Step::Reload {
            *self.reloads.lock().unwrap() += 1;
        }
        Ok(page())
    }
    async fn read(&self, _persona_id: &str, _reading: Reading) -> Result<String, String> {
        Ok("Porkbun".into())
    }
    async fn act(&self, _persona_id: &str, _act: &Act) -> Result<Acted, String> {
        Ok(Acted {
            page: page(),
            note: None,
        })
    }
    async fn screenshot(&self, _persona_id: &str) -> Result<Vec<u8>, String> {
        Err("No picture here.".into())
    }
    fn current(&self, _persona_id: &str) -> Option<String> {
        Some(page().url)
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
        Ok(())
    }
}

/// The fixture's Firefox cookie store, to change what the person's browser holds.
fn firefox_store(profiles: &tempfile::TempDir) -> rusqlite::Connection {
    rusqlite::Connection::open(profiles.path().join("firefox/Default/cookies.sqlite")).unwrap()
}

fn domains(cookies: &[Value]) -> Vec<&str> {
    cookies
        .iter()
        .map(|cookie| cookie["domain"].as_str().unwrap())
        .collect()
}

fn set_computer(room: &Arc<Room>, enabled: bool) {
    let mut ada = room::roster(room.log())
        .into_iter()
        .find(|p| p.id == "ada")
        .unwrap();
    ada.computer = Some(serde_json::from_value(json!({ "enabled": enabled })).unwrap());
    room::append_persona(room.log(), &ada).unwrap();
}

#[tokio::test]
async fn add_cookies_reads_the_browsers_only_when_it_opens() {
    let browser = Arc::new(Browser::default());
    let (room, _profiles) = desk_browser_cookie_room("lends-read-on-open", browser).await;
    // The pane opening: its rows, and Settings' sources, read no store.
    assert_eq!(
        room.cookie_lends("ada"),
        [] as [crate::contract::CookieLend; 0]
    );
    let sources = room.cookie_sources().await;
    assert_eq!(sources.sources.len(), 1);
    assert_eq!(
        (
            sources.sources[0].browser_name.as_str(),
            sources.sources[0].on
        ),
        ("Firefox", true),
        "every profile found starts switched on"
    );
    assert_eq!(
        room.cookie_holders(),
        [] as [crate::contract::CookieHolder; 0]
    );
    assert_eq!(room.cookie_discovery_calls.load(Ordering::SeqCst), 0);
    // Add Cookies opening: the site names, once.
    let lendable = room.cookie_lendable().await.unwrap();
    assert_eq!(room.cookie_discovery_calls.load(Ordering::SeqCst), 1);
    let sites: Vec<_> = lendable.sites.iter().map(|row| row.site.as_str()).collect();
    assert_eq!(sites, ["porkbun.com", "unrelated.com"]);
    assert_eq!(lendable.sites[0].sources[0].browser_name, "Firefox");
    assert!(
        !serde_json::to_string(&lendable)
            .unwrap()
            .contains("sentinel")
    );
}

#[tokio::test]
async fn pressing_a_site_shares_it_and_it_lands_as_a_row_without_a_value_anywhere() {
    let browser = Arc::new(Browser::default());
    let (room, _profiles) = desk_browser_cookie_room("lends-press", browser.clone()).await;
    let row = room.cookie_lend("ada", "porkbun.com").await.unwrap();
    assert_eq!(row.site, "porkbun.com");
    assert_eq!(row.browser_name, "Firefox");
    assert!(!row.waiting && !row.keep_fresh && row.ends_at.is_none());
    let lent = browser.lent();
    assert_eq!(lent.len(), 1);
    assert_eq!(domains(&lent[0]), [".porkbun.com", "account.porkbun.com"]);
    assert_eq!(
        *browser.reloads.lock().unwrap(),
        1,
        "the page reloads signed in"
    );
    assert_eq!(room.cookie_lends("ada"), std::slice::from_ref(&row));
    let again = room.cookie_lend("ada", "porkbun.com").await.unwrap_err();
    assert!(again.contains("already shared"), "{again}");
    assert!(room.cookie_lend("ada", "nowhere.example").await.is_err());
    for stream in [StreamId::Room, StreamId::Tape("ada".into())] {
        let written = serde_json::to_string(&room.log().load(&stream)).unwrap();
        assert!(!written.contains("sentinel"), "{written}");
    }

    room.cookie_lend_remove("ada", &row.id).await.unwrap();
    assert_eq!(browser.forgotten(), ["porkbun.com"]);
    assert_eq!(
        room.cookie_lends("ada"),
        [] as [crate::contract::CookieLend; 0]
    );
}

#[tokio::test]
async fn the_teammate_is_told_the_sites_it_holds_cookies_for_never_the_values() {
    let browser = Arc::new(Browser::default());
    let (room, _profiles) = desk_browser_cookie_room("lends-told", browser).await;
    let told = Mutex::new(None);
    assert_eq!(
        room.with_cookie_news(&told, "ada", "hello".into()),
        "hello",
        "nothing to say before anything is shared"
    );
    let row = room.cookie_lend("ada", "porkbun.com").await.unwrap();
    let line = room.with_cookie_news(&told, "ada", "next".into());
    assert!(
        line.starts_with("(Cookies: the person shared their cookies for porkbun.com"),
        "{line}"
    );
    assert!(line.ends_with("\n\nnext"));
    assert!(!line.contains("sentinel"));
    assert_eq!(
        room.with_cookie_news(&told, "ada", "again".into()),
        "again",
        "told once, until it changes"
    );
    room.cookie_lend_remove("ada", &row.id).await.unwrap();
    let line = room.with_cookie_news(&told, "ada", "after".into());
    assert!(line.contains("no longer shares"), "{line}");
}

#[tokio::test]
async fn keep_fresh_copies_again_when_the_site_changes_in_the_source() {
    let browser = Arc::new(Browser::default());
    let (room, profiles) = desk_browser_cookie_room("lends-fresh", browser.clone()).await;
    let row = room.cookie_lend("ada", "porkbun.com").await.unwrap();
    room.poll_cookie_lends().await;
    assert_eq!(browser.lent().len(), 1, "a plain share is one copy");

    let row = room
        .cookie_lend_set("ada", &row.id, Some(true), None)
        .await
        .unwrap();
    assert!(row.keep_fresh);
    room.poll_cookie_lends().await;
    assert_eq!(
        browser.lent().len(),
        2,
        "switched on, it reads its source once"
    );
    room.poll_cookie_lends().await;
    assert_eq!(browser.lent().len(), 2, "nothing changed, nothing copied");

    firefox_store(&profiles)
        .execute(
            "UPDATE moz_cookies SET value = 'cookie-rotated-sentinel' WHERE name = 'session'",
            [],
        )
        .unwrap();
    room.poll_cookie_lends().await;
    let lent = browser.lent();
    assert_eq!(lent.len(), 3, "the source changed, so it is copied again");
    assert!(
        lent[2]
            .iter()
            .any(|c| c["value"] == "cookie-rotated-sentinel")
    );
    assert_eq!(
        browser.forgotten(),
        ["porkbun.com", "porkbun.com"],
        "the old copy goes before each new one"
    );

    room.cookie_lend_set("ada", &row.id, Some(false), None)
        .await
        .unwrap();
    firefox_store(&profiles)
        .execute(
            "UPDATE moz_cookies SET value = 'again' WHERE name = 'session'",
            [],
        )
        .unwrap();
    room.poll_cookie_lends().await;
    assert_eq!(browser.lent().len(), 3, "off, the source is not read");
}

#[tokio::test]
async fn ends_takes_a_share_back_on_time_even_across_a_restart() {
    let browser = Arc::new(Browser::default());
    let (room, _profiles) = desk_browser_cookie_room("lends-ends", browser.clone()).await;
    let row = room.cookie_lend("ada", "porkbun.com").await.unwrap();
    let row = room
        .cookie_lend_set("ada", &row.id, None, Some(CookieLendEnds::Day))
        .await
        .unwrap();
    let ends = row.ends_at.unwrap();
    assert!(
        (ends - now_ms() - DAY_MS).abs() < 60_000,
        "a day from the press"
    );
    room.poll_cookie_lends().await;
    assert_eq!(room.cookie_lends("ada").len(), 1, "not yet");

    // The day passes while the desk is down.
    let mut record = room
        .lend_records()
        .into_iter()
        .find(|lend| lend.id == row.id)
        .unwrap();
    record.ends_at = Some(now_ms() - 1);
    room.write_lend(&record).unwrap();
    let log = room.log().clone();
    let candidates = room.host_candidates();
    drop(room);
    let room = Room::with_agents_and_computers(
        log,
        Arc::new(DeskKeys),
        Fake::new(Scripted::new(Vec::new())),
        crate::computer::Computer::with_path(std::env::temp_dir().join("no-runtime")),
    );
    *lock(&room.cookie_candidates) = candidates;
    room.install_browser(browser.clone(), &[]);
    assert_eq!(room.cookie_lends("ada").len(), 1);
    room.poll_cookie_lends().await;
    assert_eq!(
        room.cookie_lends("ada"),
        [] as [crate::contract::CookieLend; 0]
    );
    assert_eq!(browser.forgotten(), ["porkbun.com"]);

    room.cookie_lend("ada", "porkbun.com").await.unwrap();
    let id = room.cookie_lends("ada")[0].id.clone();
    room.cookie_lend_set("ada", &id, None, Some(CookieLendEnds::Day))
        .await
        .unwrap();
    let row = room
        .cookie_lend_set("ada", &id, None, Some(CookieLendEnds::Never))
        .await
        .unwrap();
    assert_eq!(row.ends_at, None, "Never takes the end back off");
}

#[tokio::test]
async fn shares_move_into_the_computer_when_it_turns_on_and_back_when_it_turns_off() {
    let (fixture, _profiles) = cookie_offer_room("lends-move", true).await;
    let room = &fixture.room;
    let browser = Arc::new(Browser::default());
    room.install_browser(browser.clone(), &[]);
    let uploads = || fixture.taken.uploads.lock().unwrap().clone();

    // On a computer, a share goes into its saved login.
    let row = room.cookie_lend("ada", "porkbun.com").await.unwrap();
    let up = uploads();
    assert_eq!(up.len(), 1);
    assert_eq!(
        domains(up[0]["cookies"].as_array().unwrap()),
        [".porkbun.com", "account.porkbun.com"]
    );
    let login = up[0]["name"].as_str().unwrap().to_string();
    assert_eq!(browser.lent().len(), 0);

    // Turned off: out of the computer, into the desk browser; the row stays.
    set_computer(room, false);
    room.reattach("ada").await.unwrap();
    assert_eq!(
        fixture.taken.forgotten(),
        vec![(login.clone(), vec!["porkbun.com".to_string()])]
    );
    let lent = browser.lent();
    assert_eq!(lent.len(), 1);
    assert_eq!(domains(&lent[0]), [".porkbun.com", "account.porkbun.com"]);
    let rows = room.cookie_lends("ada");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].id.as_str(), rows[0].site.as_str(), rows[0].waiting),
        (row.id.as_str(), "porkbun.com", false)
    );

    // Turned on again: out of the desk browser's storage, into the computer.
    set_computer(room, true);
    room.reattach("ada").await.unwrap();
    assert_eq!(browser.forgotten(), ["porkbun.com"]);
    room.start("ada").await.unwrap();
    room.poll_cookie_lends().await;
    let up = uploads();
    assert_eq!(
        up.len(),
        2,
        "copied into the computer again from the source"
    );
    assert_eq!(up[1]["name"], login);
    assert!(!room.cookie_lends("ada")[0].waiting);
}

#[tokio::test]
async fn a_switched_off_source_is_never_read_by_any_path() {
    let browser = Arc::new(Browser::default());
    let (room, profiles) = desk_browser_cookie_room("lends-gate", browser.clone()).await;
    let row = room.cookie_lend("ada", "porkbun.com").await.unwrap();
    room.cookie_lend_set("ada", &row.id, Some(true), None)
        .await
        .unwrap();
    room.poll_cookie_lends().await;
    assert_eq!(browser.lent().len(), 2);
    // A source discovered while it was on, as a card holds one.
    let held = lending::discover(room.host_candidates(), &room.cookie_lending.gate)
        .sources
        .remove(0);

    let sources = room
        .cookie_source_set("firefox", "Default", false)
        .await
        .unwrap();
    assert!(!sources.sources[0].on);
    // Take the store away: a read that tried would fail differently.
    std::fs::remove_file(profiles.path().join("firefox/Default/cookies.sqlite")).unwrap();
    assert_eq!(held.sites().unwrap_err(), lending::SWITCHED_OFF);
    assert_eq!(
        held.fingerprint("porkbun.com").unwrap_err(),
        lending::SWITCHED_OFF
    );
    assert_eq!(
        held.read("porkbun.com").await.unwrap_err(),
        lending::SWITCHED_OFF
    );

    // Add Cookies and the ask card's discovery list nothing, and say nothing failed.
    let lendable = room.cookie_lendable().await.unwrap();
    assert!(lendable.sites.is_empty() && lendable.unavailable.is_empty());
    assert!(room.computer_cookie_sites().await.unwrap().sites.is_empty());
    assert!(room.cookie_inventory().await.unwrap().sources.is_empty());
    // A share cannot be made from it.
    room.cookie_lend_remove("ada", &row.id).await.unwrap();
    let refused = room.cookie_lend("ada", "porkbun.com").await.unwrap_err();
    assert!(refused.contains("switched on"), "{refused}");
    assert_eq!(browser.lent().len(), 2, "keep fresh read nothing more");

    // A share that must be copied again (its teammate's browser moved) waits.
    let id = "made-before".to_string();
    room.write_lend(&Lend {
        id: id.clone(),
        teammate: "ada".into(),
        site: "porkbun.com".into(),
        browser_id: Some("firefox".into()),
        browser_name: "Firefox".into(),
        profile_id: Some("Default".into()),
        profile_name: "Default".into(),
        login: "lent-firefox-Default-made-before".into(),
        lent_at: now_ms(),
        keep_fresh: true,
        ends_at: None,
        domains: Vec::new(),
        desk_browser: true,
        copied: false,
        in_computer: false,
        removed: false,
        card: false,
        from_laptop: false,
    })
    .unwrap();
    room.poll_cookie_lends().await;
    assert_eq!(browser.lent().len(), 2);
    assert!(room.cookie_lends("ada")[0].waiting);
}

#[tokio::test]
async fn in_use_lists_every_teammate_holding_cookies_with_its_sites() {
    let browser = Arc::new(Browser::default());
    let (room, _profiles) = desk_browser_cookie_room("lends-in-use", browser).await;
    let mut bob = room::roster(room.log())
        .into_iter()
        .find(|p| p.id == "ada")
        .unwrap();
    bob.id = "bob".into();
    bob.name = "Bob".into();
    room::append_persona(room.log(), &bob).unwrap();
    assert_eq!(
        room.cookie_holders(),
        [] as [crate::contract::CookieHolder; 0]
    );
    let ada = room.cookie_lend("ada", "porkbun.com").await.unwrap();
    room.cookie_lend("ada", "unrelated.com").await.unwrap();
    room.cookie_lend("bob", "porkbun.com").await.unwrap();
    let holders = room.cookie_holders();
    assert_eq!(holders.len(), 2);
    assert_eq!(holders[0].teammate, "ada");
    assert_eq!(holders[0].sites, ["porkbun.com", "unrelated.com"]);
    assert_eq!(
        (holders[1].name.as_str(), holders[1].sites.as_slice()),
        ("Bob", ["porkbun.com".to_string()].as_slice())
    );
    room.cookie_lend_remove("ada", &ada.id).await.unwrap();
    assert_eq!(room.cookie_holders()[0].sites, ["unrelated.com"]);
}
