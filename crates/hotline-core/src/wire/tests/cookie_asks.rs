//! A teammate's cookie ask through the real seat handler, host SQLite
//! fixtures and the authenticated computer fake. Nothing is offered on a
//! visit; the teammate asks, and the person answers on one card.
use super::*;
use crate::computer::cookies::lending::tests::candidate_fixture;
use crate::contract::{CookieOffer, CookieOfferStatus};
use crate::session::tests::{cookie_offer_room, cookie_offer_room_with_candidates};

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

/// The teammate's `ask_cookies`, under its live session's lease.
async fn ask(room: &Arc<Room>, site: &str) -> Result<String, String> {
    room.session_tools("ada")
        .call(
            "ask_cookies",
            &json!({"site":site,"reason":"To open the DNS page for the domain."}),
        )
        .await
}

fn asks(room: &Arc<Room>) -> Vec<Value> {
    room.log()
        .load(&StreamId::Tape("ada".into()))
        .into_iter()
        .filter(|e| e["kind"] == "cookie_ask")
        .collect()
}

fn delivered(room: &Arc<Room>) -> Vec<String> {
    room.log()
        .load(&StreamId::Tape("ada".into()))
        .iter()
        .filter(|e| e["kind"] == "delivery")
        .filter_map(|e| e["text"].as_str().map(str::to_owned))
        .collect()
}

async fn heard(room: &Arc<Room>, words: &str) {
    crate::test_support::until(&format!("the teammate hears {words:?}"), || {
        delivered(room).iter().any(|text| text.contains(words))
    })
    .await;
}

#[tokio::test]
async fn a_computer_on_a_signed_in_site_raises_no_card_and_reads_no_browser() {
    use std::sync::atomic::Ordering;
    let (fixture, _profiles) = cookie_offer_room("ask-no-offer", true).await;
    let room = &fixture.room;
    // The computer's browser is on porkbun.com, where the person's Firefox
    // is signed in: a visit is not an ask.
    for _ in 0..3 {
        room.poll_secret_uses("ada").await;
    }
    assert_eq!(asks(room), [] as [Value; 0]);
    assert_eq!(room.computer_cookie_offers("ada"), [] as [CookieOffer; 0]);
    assert_eq!(room.cookie_discovery_calls.load(Ordering::SeqCst), 0);
    assert!(fixture.taken.uploads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_ask_is_one_card_on_desk_and_phone_and_a_second_for_its_site_is_refused() {
    let (fixture, _profiles) = cookie_offer_room("ask-one-card", true).await;
    let room = &fixture.room;
    let said = ask(room, "porkbun.com").await.unwrap();
    assert!(said.starts_with("Asked: cookie-ask:"), "{said}");
    let again = ask(room, "https://account.porkbun.com/login")
        .await
        .unwrap_err();
    assert!(again.contains("already up"), "{again}");
    for seat in [Seat::Desk, Seat::Phone] {
        let snapshot =
            super::asks::seat_snapshot(seat, &desk(room), room.log(), json!({"tape":"ada"})).await;
        let cards: Vec<_> = snapshot
            .iter()
            .filter(|e| e["kind"] == "cookie_ask")
            .collect();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0]["site"], "porkbun.com");
        assert_eq!(cards[0]["origin"], "https://porkbun.com");
        assert_eq!(cards[0]["reason"], "To open the DNS page for the domain.");
        assert_eq!(cards[0]["status"], "pending");
        assert_eq!(
            cards[0]["sources"],
            json!([{"browserName":"Firefox","profileName":"Default"}])
        );
        assert!(cards[0].get("deskBrowser").is_none());
        let roster =
            super::asks::seat_snapshot(seat, &desk(room), room.log(), json!({"view":"roster"}))
                .await;
        assert_eq!(roster[0]["waiting"], true);
    }
    // Another site is its own ask.
    assert!(ask(room, "unrelated.com").await.is_ok());
    assert!(fixture.taken.uploads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn use_cookies_shares_only_the_site_from_the_chosen_browser_and_says_so_without_values() {
    let profiles = tempfile::tempdir().unwrap();
    let fixture = cookie_offer_room_with_candidates(
        "ask-use-chosen",
        vec![
            candidate_fixture(profiles.path(), false),
            candidate_fixture(profiles.path(), true),
        ],
    )
    .await;
    let room = &fixture.room;
    ask(room, "porkbun.com").await.unwrap();
    let card = asks(room).remove(0);
    let id = card["id"].as_str().unwrap().to_string();
    assert_eq!(
        card["sources"],
        json!([
            {"browserName":"Brave","profileName":"Default"},
            {"browserName":"Firefox","profileName":"Default"}
        ])
    );
    let firefox = json!({"browserName":"Firefox","profileName":"Default"});
    let (phone, owner) = tokio::join!(
        command(
            room,
            Seat::Phone,
            "computer.cookies.answer",
            json!({"personaId":"ada","id":id,"answer":"use","source":firefox})
        ),
        command(
            room,
            Seat::Owner,
            "computer.cookies.answer",
            json!({"personaId":"ada","id":id,"answer":"use","source":firefox})
        )
    );
    assert_ne!(phone["ok"], owner["ok"], "{phone} {owner}");
    let uploads = fixture.taken.uploads.lock().unwrap().clone();
    assert_eq!(uploads.len(), 1);
    assert!(
        uploads[0]["name"]
            .as_str()
            .unwrap()
            .starts_with("lent-firefox-Default-")
    );
    let domains: Vec<_> = uploads[0]["cookies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["domain"].as_str().unwrap())
        .collect();
    assert_eq!(domains, [".porkbun.com", "account.porkbun.com"]);
    let settled = asks(room).remove(0);
    assert_eq!(settled["status"], "cookies");
    assert_eq!(settled["browserName"], "Firefox");
    let lends = room.computer_cookie_offers("ada");
    assert_eq!(lends.len(), 1);
    assert_eq!(lends[0].status, CookieOfferStatus::Lent);
    assert_eq!(lends[0].browser_name, "Firefox");
    heard(room, "shared their Firefox cookies for porkbun.com").await;

    // The browser shared from last is the one the next ask picks first.
    assert!(ask(room, "unrelated.com").await.is_ok());
    let next = asks(room)
        .into_iter()
        .find(|e| e["site"] == "unrelated.com")
        .unwrap();
    assert_eq!(next["sources"][0]["browserName"], "Firefox");

    for sink in [
        phone.to_string(),
        owner.to_string(),
        serde_json::to_string(&room.log().load(&StreamId::Room)).unwrap(),
        serde_json::to_string(&room.log().load(&StreamId::Tape("ada".into()))).unwrap(),
        fixture.agents.cookie_heard(),
    ] {
        assert!(
            !sink.contains("sentinel-273"),
            "a cookie leaked into {sink}"
        );
    }

    // The share is removed the way every share is.
    let lend = lends[0].id.clone();
    assert_eq!(
        command(
            room,
            Seat::Phone,
            "computer.cookies.remove",
            json!({"personaId":"ada","id":lend})
        )
        .await["ok"],
        true
    );
    assert_eq!(
        fixture.taken.forgotten(),
        vec![(lends[0].login.clone(), vec!["porkbun.com".into()])]
    );
}

#[tokio::test]
async fn closing_the_card_blocks_its_site_until_the_person_next_writes() {
    let (fixture, _profiles) = cookie_offer_room("ask-not-now", true).await;
    let room = &fixture.room;
    ask(room, "porkbun.com").await.unwrap();
    let id = asks(room)[0]["id"].as_str().unwrap().to_string();
    assert_eq!(answer(room, &id, "not_now").await["ok"], true);
    assert_eq!(asks(room)[0]["status"], "not_now");
    heard(
        room,
        "Do not ask for porkbun.com again until they next write to you",
    )
    .await;
    assert_eq!(answer(room, &id, "use").await["ok"], false, "settled once");

    let refused = ask(room, "porkbun.com").await.unwrap_err();
    assert!(refused.contains("said no"), "{refused}");
    // A stop and start is not the person writing.
    room.stop("ada").unwrap();
    room.start("ada").await.unwrap();
    assert!(ask(room, "porkbun.com").await.is_err());
    assert!(ask(room, "unrelated.com").await.is_ok(), "only that site");

    room.log()
        .append(
            &StreamId::Tape("ada".into()),
            &json!({"kind":"user","id":"said-again","ts":crate::session::now_ms() + 1,"text":"Try again."}),
        )
        .unwrap();
    assert!(ask(room, "porkbun.com").await.is_ok());
    assert!(fixture.taken.uploads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn open_browser_hands_over_the_computers_screen_and_tells_the_teammate() {
    let (fixture, _profiles) = cookie_offer_room("ask-open", true).await;
    let room = &fixture.room;
    ask(room, "porkbun.com").await.unwrap();
    let id = asks(room)[0]["id"].as_str().unwrap().to_string();
    assert_eq!(answer(room, &id, "open_browser").await["ok"], true);
    assert_eq!(asks(room)[0]["status"], "opened");
    heard(
        room,
        "opened your computer's screen to sign in to porkbun.com",
    )
    .await;
    assert!(fixture.taken.uploads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_ask_refuses_a_wrong_teammate_or_seat_and_expires_with_its_session() {
    let (fixture, _profiles) = cookie_offer_room("ask-authority", true).await;
    let room = &fixture.room;
    ask(room, "porkbun.com").await.unwrap();
    let id = asks(room)[0]["id"].as_str().unwrap().to_string();
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
    assert_eq!(
        command(
            room,
            Seat::Phone,
            "computer.cookies.answer",
            json!({"personaId":"ada","id":id,"answer":"use","source":{"browserName":"Safari","profileName":"Default"}})
        )
        .await["ok"],
        false,
        "a browser the card does not name"
    );
    room.stop("ada").unwrap();
    assert_eq!(answer(room, &id, "use").await["ok"], false);
    assert!(fixture.taken.uploads.lock().unwrap().is_empty());
    assert_eq!(asks(room)[0]["status"], "expired");
}

#[tokio::test]
async fn withdraw_request_takes_an_ask_down() {
    let (fixture, _profiles) = cookie_offer_room("ask-withdraw", true).await;
    let room = &fixture.room;
    ask(room, "porkbun.com").await.unwrap();
    let id = asks(room)[0]["id"].as_str().unwrap().to_string();
    room.session_tools("ada")
        .call("withdraw_request", &json!({"id":id}))
        .await
        .unwrap();
    assert_eq!(asks(room)[0]["status"], "withdrawn");
    assert_eq!(answer(room, &id, "use").await["ok"], false);
}

#[tokio::test]
async fn a_failed_share_keeps_a_removal_handle_and_never_echoes_a_cookie() {
    let (fixture, _profiles) = cookie_offer_room("ask-failed-share", true).await;
    let room = &fixture.room;
    ask(room, "porkbun.com").await.unwrap();
    let id = asks(room)[0]["id"].as_str().unwrap().to_string();
    *fixture.taken.login_error.lock().unwrap() = Some("cookie-sentinel-273".into());
    fixture
        .taken
        .forget_error
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let response = answer(room, &id, "use").await;
    assert_eq!(response["ok"], false);
    assert!(!response.to_string().contains("sentinel-273"));
    assert_eq!(asks(room)[0]["status"], "pending", "the card stays up");
    let lend = room.computer_cookie_offers("ada")[0].clone();
    assert_eq!(lend.status, CookieOfferStatus::Unconfirmed);
    fixture
        .taken
        .forget_error
        .store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        command(
            room,
            Seat::Phone,
            "computer.cookies.remove",
            json!({"personaId":"ada","id":lend.id})
        )
        .await["ok"],
        true
    );
    assert_eq!(
        room.computer_cookie_offers("ada")[0].status,
        CookieOfferStatus::Removed
    );
}

#[tokio::test]
async fn the_retired_offer_settings_and_commands_are_gone_and_site_discovery_stays_explicit() {
    let (fixture, _profiles) = cookie_offer_room("ask-retired", true).await;
    let room = &fixture.room;
    for retired in [
        "computer.cookies.settings",
        "computer.cookies.blocked",
        "computer.cookies.reset",
    ] {
        assert_eq!(
            command(room, Seat::Desk, retired, json!({})).await["ok"],
            false,
            "{retired}"
        );
    }
    assert_eq!(
        command(room, Seat::Phone, "computer.cookies.sites", json!({})).await["ok"],
        false
    );
    let sites = command(room, Seat::Desk, "computer.cookies.sites", json!({})).await;
    assert_eq!(sites["result"]["sites"][0]["site"], "porkbun.com");
    assert!(!sites.to_string().contains("sentinel-273"));
}

mod desk_browser;
