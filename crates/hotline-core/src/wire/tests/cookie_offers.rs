//! The real seat handler, host SQLite fixtures and authenticated computer fake.
use super::*;
use crate::computer::cookies::lending::tests::{RefusingBrowser, candidate_fixture};
use crate::contract::{CookieOffer, CookieOfferStatus};
use crate::session::tests::{cookie_offer_room, cookie_offer_room_with_candidates};
use crate::test_support::until;

fn desk(room: &Arc<Room>) -> Arc<dyn RoomHandle> {
    Arc::new(crate::desk::Desk::with_room(
        room.log().clone(),
        room.clone(),
    ))
}
async fn command(room: &Arc<Room>, seat: Seat, cmd: &str, params: Value) -> Value {
    remote_control_answer(
        seat,
        &desk(room),
        room.log(),
        &Parts::default(),
        json!({"id":1,"cmd":cmd,"params":params}),
    )
    .await
}
async fn answer(room: &Arc<Room>, id: &str, choice: &str) -> Value {
    command(
        room,
        Seat::Phone,
        "computer.cookies.answer",
        json!({"personaId":"ada","id":id,"answer":choice}),
    )
    .await
}

async fn cookie_notices(room: &Arc<Room>, persona: &str) -> Vec<String> {
    super::asks::seat_snapshot(Seat::Desk, &desk(room), room.log(), json!({"tape":persona}))
        .await
        .into_iter()
        .filter(|event| event["kind"] == "notice")
        .filter_map(|event| event["text"].as_str().map(str::to_owned))
        .filter(|text| text.starts_with("Hotline can't read "))
        .collect()
}

#[tokio::test]
async fn refused_browsers_raise_one_notice_each_per_desk_run_and_no_offer() {
    let profiles = tempfile::tempdir().unwrap();
    let brave = RefusingBrowser::new(profiles.path(), false);
    let firefox = RefusingBrowser::new(profiles.path(), true);
    let fixture = cookie_offer_room_with_candidates(
        "offers-refused-once",
        vec![brave.candidate.clone(), firefox.candidate.clone()],
    )
    .await;
    let room = &fixture.room;
    room.poll_secret_uses("ada").await;
    let notices = cookie_notices(room, "ada").await;
    assert_eq!(notices.len(), 2);
    assert!(notices[0].starts_with(
        "Hotline can't read Brave's cookies, so it can't offer your session for porkbun.com. "
    ));
    assert!(notices[1].starts_with(
        "Hotline can't read Firefox's cookies, so it can't offer your session for porkbun.com. "
    ));
    assert_eq!(room.computer_cookie_offers("ada"), [] as [CookieOffer; 0]);

    *fixture.taken.pages.lock().unwrap() = vec![json!({"origin":"https://another.example"})];
    room.poll_secret_uses("ada").await;
    room.stop("ada").unwrap();
    room.start("ada").await.unwrap();
    room.poll_secret_uses("ada").await;
    assert_eq!(cookie_notices(room, "ada").await, notices);
    let mut other = crate::room::roster(room.log())
        .into_iter()
        .find(|p| p.id == "ada")
        .unwrap();
    other.id = "bob".into();
    crate::room::append_persona(room.log(), &other).unwrap();
    room.start("bob").await.unwrap();
    room.poll_secret_uses("bob").await;
    assert_eq!(cookie_notices(room, "bob").await, [] as [String; 0]);
    assert_eq!(room.computer_cookie_offers("bob"), [] as [CookieOffer; 0]);
    assert!(fixture.taken.uploads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_refused_site_is_offered_after_access_appears_without_reopening_the_page() {
    let profiles = tempfile::tempdir().unwrap();
    let browser = RefusingBrowser::new(profiles.path(), false);
    let fixture = cookie_offer_room_with_candidates(
        "offers-access-restored",
        vec![browser.candidate.clone()],
    )
    .await;
    let room = &fixture.room;
    room.poll_secret_uses("ada").await;
    assert_eq!(room.computer_cookie_offers("ada"), [] as [CookieOffer; 0]);
    assert_eq!(cookie_notices(room, "ada").await.len(), 1);
    browser.grant();
    // Only the host permission changes: same session, computer and /pages.
    room.poll_secret_uses("ada").await;
    room.poll_secret_uses("ada").await;
    let cards = room.computer_cookie_offers("ada");
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].site, "porkbun.com");
    assert_eq!(cards[0].browser_name, "Brave");
    assert_eq!(cards[0].status, CookieOfferStatus::Pending);
    assert_eq!(cookie_notices(room, "ada").await.len(), 1);
    assert!(fixture.taken.uploads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_granted_browser_offers_once_even_when_another_browser_is_refused() {
    let profiles = tempfile::tempdir().unwrap();
    let brave = RefusingBrowser::new(profiles.path(), false);
    let firefox = candidate_fixture(profiles.path(), true);
    let fixture = cookie_offer_room_with_candidates(
        "offers-partly-readable",
        vec![brave.candidate.clone(), firefox],
    )
    .await;
    let room = &fixture.room;
    room.poll_secret_uses("ada").await;
    room.poll_secret_uses("ada").await;
    let cards = room.computer_cookie_offers("ada");
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].browser_name, "Firefox");
    assert_eq!(cards[0].status, CookieOfferStatus::Pending);
    assert_eq!(cookie_notices(room, "ada").await.len(), 1);
    assert!(fixture.taken.uploads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_refused_browser_does_not_raise_a_notice_for_a_never_site() {
    let profiles = tempfile::tempdir().unwrap();
    let browser = RefusingBrowser::new(profiles.path(), false);
    browser.grant();
    let fixture =
        cookie_offer_room_with_candidates("offers-never-refused", vec![browser.candidate.clone()])
            .await;
    let room = &fixture.room;
    room.poll_secret_uses("ada").await;
    let id = room.computer_cookie_offers("ada")[0].id.clone();
    assert_eq!(answer(room, &id, "never").await["ok"], true);
    *fixture.taken.pages.lock().unwrap() = vec![json!({"origin":"https://porkbun.com"})];
    browser.refuse();
    room.stop("ada").unwrap();
    room.start("ada").await.unwrap();
    room.poll_secret_uses("ada").await;
    assert_eq!(cookie_notices(room, "ada").await, [] as [String; 0]);
    assert_eq!(room.computer_cookie_offers("ada").len(), 1);
}

#[tokio::test]
async fn cookie_offer_is_one_card_on_desk_and_phone_only_for_sites_in_chromium_or_firefox() {
    for firefox in [false, true] {
        let (fixture, _profiles) = cookie_offer_room(
            if firefox {
                "offer-firefox"
            } else {
                "offer-chromium"
            },
            firefox,
        )
        .await;
        let room = &fixture.room;
        until("the two-second watch publishes a cookie offer", || {
            !room.computer_cookie_offers("ada").is_empty()
        })
        .await;
        room.poll_secret_uses("ada").await;
        for seat in [Seat::Desk, Seat::Phone] {
            let snapshot =
                super::asks::seat_snapshot(seat, &desk(room), room.log(), json!({"tape":"ada"}))
                    .await;
            let cards: Vec<_> = snapshot
                .iter()
                .filter(|e| e["kind"] == "cookie_offer")
                .collect();
            assert_eq!(cards.len(), 1);
            assert_eq!(cards[0]["site"], "porkbun.com");
            assert_eq!(cards[0]["status"], "pending");
            let roster =
                super::asks::seat_snapshot(seat, &desk(room), room.log(), json!({"view":"roster"}))
                    .await;
            assert_eq!(roster[0]["waiting"], true);
        }
        assert!(fixture.taken.uploads.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn use_lends_only_the_site_and_subdomains_and_remove_takes_it_back_and_tells_the_agent() {
    let (fixture, _profiles) = cookie_offer_room("offer-use-remove", true).await;
    let room = &fixture.room;
    room.poll_secret_uses("ada").await;
    let offer = room.computer_cookie_offers("ada")[0].clone();
    let (phone, owner) = tokio::join!(
        answer(room, &offer.id, "use"),
        command(
            room,
            Seat::Owner,
            "computer.cookies.answer",
            json!({"personaId":"ada","id":offer.id,"answer":"use"})
        )
    );
    assert_ne!(phone["ok"], owner["ok"], "{phone} {owner}");
    let uploads = fixture.taken.uploads.lock().unwrap().clone();
    assert_eq!(uploads.len(), 1);
    let domains: Vec<_> = uploads[0]["cookies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["domain"].as_str().unwrap())
        .collect();
    assert_eq!(domains, [".porkbun.com", "account.porkbun.com"]);
    assert!(
        uploads[0]["name"]
            .as_str()
            .unwrap()
            .starts_with("lent-firefox-Default-")
    );
    assert_eq!(
        room.computer_cookie_offers("ada")[0].status,
        CookieOfferStatus::Lent
    );
    assert!(
        room.log()
            .load(&StreamId::Tape("ada".into()))
            .iter()
            .any(|e| e["kind"] == "delivery"
                && e["text"].as_str().is_some_and(|s| s.contains("may reload")))
    );
    assert_eq!(
        command(
            room,
            Seat::Phone,
            "computer.cookies.remove",
            json!({"personaId":"ada","id":offer.id})
        )
        .await["ok"],
        true
    );
    assert_eq!(
        fixture.taken.forgotten(),
        vec![(offer.login, vec!["porkbun.com".into()])]
    );
    assert_eq!(
        room.computer_cookie_offers("ada")[0].status,
        CookieOfferStatus::Removed
    );
    assert!(
        room.log()
            .load(&StreamId::Tape("ada".into()))
            .iter()
            .any(|e| e["kind"] == "delivery"
                && e["text"]
                    .as_str()
                    .is_some_and(|s| s.contains("took back the session")))
    );
}

#[tokio::test]
async fn not_now_asks_again_next_session_but_never_waits_until_settings_undo() {
    let (fixture, _profiles) = cookie_offer_room("offer-choices", true).await;
    let room = &fixture.room;
    room.poll_secret_uses("ada").await;
    let id = room.computer_cookie_offers("ada")[0].id.clone();
    assert_eq!(answer(room, &id, "not_now").await["ok"], true);
    room.poll_secret_uses("ada").await;
    assert_eq!(room.computer_cookie_offers("ada").len(), 1);
    room.stop("ada").unwrap();
    room.start("ada").await.unwrap();
    room.poll_secret_uses("ada").await;
    let cards = room.computer_cookie_offers("ada");
    assert_eq!(cards.len(), 2);
    assert_eq!(answer(room, &cards[1].id, "never").await["ok"], true);
    room.stop("ada").unwrap();
    room.start("ada").await.unwrap();
    room.poll_secret_uses("ada").await;
    assert_eq!(room.computer_cookie_offers("ada").len(), 2);
    let blocked = command(room, Seat::Desk, "computer.cookies.blocked", json!({})).await;
    assert_eq!(blocked["result"][0]["site"], "porkbun.com");
    assert_eq!(blocked["result"][0]["teammate"], "ada");
    assert_eq!(
        command(
            room,
            Seat::Phone,
            "computer.cookies.reset",
            json!({"personaId":"ada","site":"porkbun.com"})
        )
        .await["ok"],
        false
    );
    assert_eq!(
        command(
            room,
            Seat::Desk,
            "computer.cookies.reset",
            json!({"personaId":"ada","site":"porkbun.com"})
        )
        .await["ok"],
        true
    );
    room.poll_secret_uses("ada").await;
    assert_eq!(room.computer_cookie_offers("ada").len(), 3);
}

#[tokio::test]
async fn cookie_offer_refuses_unknown_wrong_teammate_and_revoked_cards_before_reading_values() {
    let (fixture, _profiles) = cookie_offer_room("offer-authority", true).await;
    let room = &fixture.room;
    room.poll_secret_uses("ada").await;
    let id = room.computer_cookie_offers("ada")[0].id.clone();
    assert_eq!(answer(room, "missing", "use").await["ok"], false);
    assert_eq!(
        command(
            room,
            Seat::Phone,
            "computer.cookies.answer",
            json!({"personaId":"other","id":id,"answer":"use"})
        )
        .await["ok"],
        false
    );
    assert_eq!(
        command(
            room,
            Seat::Link,
            "computer.cookies.answer",
            json!({"personaId":"ada","id":id,"answer":"use"})
        )
        .await["ok"],
        false
    );
    room.stop("ada").unwrap();
    assert_eq!(answer(room, &id, "use").await["ok"], false);
    assert!(fixture.taken.uploads.lock().unwrap().is_empty());
    assert_eq!(
        room.computer_cookie_offers("ada")[0].status,
        CookieOfferStatus::Expired
    );
}

#[tokio::test]
async fn cookie_values_never_reach_tape_room_agent_or_wire_sinks() {
    let (fixture, _profiles) = cookie_offer_room("offer-sinks", true).await;
    let room = &fixture.room;
    room.poll_secret_uses("ada").await;
    let id = room.computer_cookie_offers("ada")[0].id.clone();
    let response = answer(room, &id, "use").await;
    assert_eq!(response["ok"], true, "{response}");
    let listing = command(
        room,
        Seat::Phone,
        "computer.cookies.offers",
        json!({"personaId":"ada"}),
    )
    .await;
    for sink in [
        response.to_string(),
        listing.to_string(),
        serde_json::to_string(&room.log().load(&StreamId::Room)).unwrap(),
        serde_json::to_string(&room.log().load(&StreamId::Tape("ada".into()))).unwrap(),
        fixture.agents.cookie_heard(),
    ] {
        assert!(!sink.contains("sentinel-273"), "cookie leaked into sink");
    }
}

#[tokio::test]
async fn failed_delivery_keeps_a_removal_handle_and_never_echoes_a_cookie_in_errors() {
    let (fixture, _profiles) = cookie_offer_room("offer-failed-delivery", true).await;
    let room = &fixture.room;
    room.poll_secret_uses("ada").await;
    let id = room.computer_cookie_offers("ada")[0].id.clone();
    *fixture.taken.login_error.lock().unwrap() = Some("cookie-sentinel-273".into());
    fixture
        .taken
        .forget_error
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let response = answer(room, &id, "use").await;
    assert_eq!(response["ok"], false);
    assert!(!response.to_string().contains("sentinel-273"));
    assert_eq!(
        room.computer_cookie_offers("ada")[0].status,
        CookieOfferStatus::Unconfirmed
    );
    let response = command(
        room,
        Seat::Phone,
        "computer.cookies.remove",
        json!({"personaId":"ada","id":id}),
    )
    .await;
    assert_eq!(response["ok"], false);
    assert!(!response.to_string().contains("sentinel-273"));
    fixture
        .taken
        .forget_error
        .store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        command(
            room,
            Seat::Phone,
            "computer.cookies.remove",
            json!({"personaId":"ada","id":id})
        )
        .await["ok"],
        true
    );
    assert_eq!(
        room.computer_cookie_offers("ada")[0].status,
        CookieOfferStatus::Removed
    );
}

async fn set_offers(room: &Arc<Room>, settings: Value) {
    let reply = command(
        room,
        Seat::Desk,
        "settings.update",
        json!({"patch":{"cookieOffers":settings}}),
    )
    .await;
    assert_eq!(reply["ok"], true, "{reply}");
}

#[tokio::test]
async fn cookie_offers_off_never_discovers_and_expires_pending_cards_with_an_explanation() {
    use std::sync::atomic::Ordering;
    let (fixture, _profiles) = cookie_offer_room("offers-disabled", false).await;
    let room = &fixture.room;
    set_offers(room, json!({"enabled":false})).await;
    let calls = room.cookie_discovery_calls.load(Ordering::SeqCst);
    room.poll_secret_uses("ada").await;
    assert_eq!(room.cookie_discovery_calls.load(Ordering::SeqCst), calls);
    assert_eq!(room.computer_cookie_offers("ada"), [] as [CookieOffer; 0]);

    set_offers(room, json!({"enabled":true})).await;
    room.poll_secret_uses("ada").await;
    let offer = room.computer_cookie_offers("ada")[0].clone();
    assert_eq!(offer.status, CookieOfferStatus::Pending);
    let calls = room.cookie_discovery_calls.load(Ordering::SeqCst);
    assert!(calls > 0, "the fixture records actual discovery calls");
    set_offers(room, json!({"enabled":false})).await;
    // The update settles the card before acknowledging, without another poll.
    assert_eq!(
        room.computer_cookie_offers("ada")[0].status,
        CookieOfferStatus::Expired
    );
    assert!(
        room.log()
            .load(&StreamId::Tape("ada".into()))
            .iter()
            .any(|e| e["kind"] == "notice"
                && e["text"] == "Browser cookie offers are off. Pending offers have expired.")
    );
    assert_eq!(answer(room, &offer.id, "use").await["ok"], false);
    *fixture.taken.pages.lock().unwrap() = vec![json!({"origin":"https://unrelated.com"})];
    room.poll_secret_uses("ada").await;
    assert_eq!(room.cookie_discovery_calls.load(Ordering::SeqCst), calls);
    assert!(fixture.taken.uploads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn cookie_offers_selected_sites_only_offer_matches_and_expire_deselected_cards() {
    let (fixture, _profiles) = cookie_offer_room("offers-selected", false).await;
    let room = &fixture.room;
    set_offers(room, json!({"enabled":true,"sites":["unrelated.com"]})).await;
    *fixture.taken.pages.lock().unwrap() = vec![
        json!({"origin":"https://porkbun.com"}),
        json!({"origin":"https://www.unrelated.com"}),
    ];
    room.poll_secret_uses("ada").await;
    let offers = room.computer_cookie_offers("ada");
    assert_eq!(offers.len(), 1);
    assert_eq!(offers[0].site, "unrelated.com");
    set_offers(room, json!({"enabled":true,"sites":["porkbun.com"]})).await;
    assert_eq!(
        room.computer_cookie_offers("ada")[0].status,
        CookieOfferStatus::Expired
    );
    assert_eq!(answer(room, &offers[0].id, "use").await["ok"], false);
    room.poll_secret_uses("ada").await;
    let offers = room.computer_cookie_offers("ada");
    assert_eq!(offers.len(), 2);
    assert_eq!(offers[1].site, "porkbun.com");
    assert_eq!(offers[1].status, CookieOfferStatus::Pending);
}

#[tokio::test]
async fn cookie_offers_all_sites_offer_any_discovered_site_and_an_empty_list_offers_none() {
    use std::sync::atomic::Ordering;
    let (fixture, _profiles) = cookie_offer_room("offers-all", false).await;
    let room = &fixture.room;
    set_offers(room, json!({"enabled":true,"sites":[]})).await;
    *fixture.taken.pages.lock().unwrap() = vec![
        json!({"origin":"https://porkbun.com"}),
        json!({"origin":"https://unrelated.com"}),
    ];
    let calls = room.cookie_discovery_calls.load(Ordering::SeqCst);
    room.poll_secret_uses("ada").await;
    assert_eq!(room.computer_cookie_offers("ada"), [] as [CookieOffer; 0]);
    assert_eq!(room.cookie_discovery_calls.load(Ordering::SeqCst), calls);
    // Clearing the override restores the old-room default: enabled, all sites.
    set_offers(room, Value::Null).await;
    room.poll_secret_uses("ada").await;
    let offers = room.computer_cookie_offers("ada");
    assert_eq!(
        offers.iter().map(|c| c.site.as_str()).collect::<Vec<_>>(),
        ["porkbun.com", "unrelated.com"]
    );
}

#[tokio::test]
async fn cookie_offer_settings_are_owner_written_phone_read_and_site_discovery_is_explicit() {
    use std::sync::atomic::Ordering;
    let profiles = tempfile::tempdir().unwrap();
    let fixture = cookie_offer_room_with_candidates(
        "offers-settings-seats",
        vec![
            candidate_fixture(profiles.path(), true),
            candidate_fixture(profiles.path(), false),
        ],
    )
    .await;
    let room = &fixture.room;
    let settings = json!({"enabled":false,"sites":["porkbun.com"]});
    for seat in [Seat::Desk, Seat::Owner] {
        assert_eq!(
            command(
                room,
                seat,
                "settings.update",
                json!({"patch":{"cookieOffers":settings}})
            )
            .await["ok"],
            true
        );
    }
    let calls = room.cookie_discovery_calls.load(Ordering::SeqCst);
    for seat in [Seat::Desk, Seat::Owner, Seat::Phone] {
        assert_eq!(
            command(room, seat, "computer.cookies.settings", json!({})).await["result"],
            settings
        );
    }
    for seat in [Seat::Phone, Seat::Link] {
        assert_eq!(
            command(
                room,
                seat,
                "settings.update",
                json!({"patch":{"cookieOffers":{"enabled":true}}})
            )
            .await["ok"],
            false
        );
        assert_eq!(
            command(room, seat, "computer.cookies.sites", json!({})).await["ok"],
            false
        );
    }
    assert_eq!(room.cookie_discovery_calls.load(Ordering::SeqCst), calls);
    for seat in [Seat::Desk, Seat::Owner] {
        let response = command(room, seat, "computer.cookies.sites", json!({})).await;
        assert_eq!(response["ok"], true);
        assert_eq!(
            response["result"]["sites"],
            json!([
                {"site":"porkbun.com","sources":[{"browserName":"Brave","profileName":"Default"},{"browserName":"Firefox","profileName":"Default"}]},
                {"site":"unrelated.com","sources":[{"browserName":"Brave","profileName":"Default"},{"browserName":"Firefox","profileName":"Default"}]}
            ])
        );
        assert!(!response.to_string().contains("sentinel-273"));
    }
    assert_eq!(
        room.cookie_discovery_calls.load(Ordering::SeqCst),
        calls + 2
    );
    assert!(fixture.taken.uploads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn cookie_offer_settings_refuse_invalid_domains_and_shapes_without_changing_the_setting() {
    let (fixture, _profiles) = cookie_offer_room("offers-validation", false).await;
    let room = &fixture.room;
    set_offers(room, json!({"enabled":false})).await;
    for bad in [
        json!({"enabled":"yes"}),
        json!({"sites":"porkbun.com"}),
        json!({"sites":["account.porkbun.com"]}),
        json!({"sites":["https://porkbun.com"]}),
        json!({"sites":["com"]}),
        json!({"sites":["127.0.0.1"]}),
        json!([]),
    ] {
        let response = command(
            room,
            Seat::Owner,
            "settings.update",
            json!({"patch":{"cookieOffers":bad}}),
        )
        .await;
        assert_eq!(response["ok"], false, "{bad}: {response}");
    }
    assert_eq!(
        command(room, Seat::Phone, "computer.cookies.settings", json!({})).await["result"],
        json!({"enabled":false})
    );
}
