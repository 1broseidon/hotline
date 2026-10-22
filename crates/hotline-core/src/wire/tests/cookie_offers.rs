//! The real seat handler, host SQLite fixtures and authenticated computer fake.
use super::*;
use crate::contract::CookieOfferStatus;
use crate::session::tests::cookie_offer_room;
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
