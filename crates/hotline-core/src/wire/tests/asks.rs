//! A `request_human` card a teammate no longer needs goes away: withdrawn by
//! the teammate that raised it, or expired at the end of its lifetime. Each
//! is read back as the desk and the phone read it, through the real handler
//! with each seat, because "needs you" on either is the whole complaint.
use super::*;
use crate::desk::Desk;
use crate::driver::{MessageKind, Update};
use crate::session::now_ms;
use crate::session::tests::{DeskKeys, Fake, Scripted, enrol, persona};
use crate::test_support::until;

/// A desk with Ada and Bob on scripted agents that answer every turn.
fn asking_desk() -> (tempfile::TempDir, Arc<Room>, Arc<dyn RoomHandle>, Log) {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    enrol(&log, &persona("ada"));
    enrol(&log, &persona("bob"));
    let turns = (0..8)
        .map(|i| {
            vec![
                Update::Message {
                    kind: MessageKind::Agent,
                    id: format!("a{i}"),
                    text: "noted".to_string(),
                },
                Update::Turn {
                    stop_reason: "end_turn".into(),
                    usage: None,
                },
            ]
        })
        .collect();
    let room = Room::with_agents(
        log.clone(),
        Arc::new(DeskKeys),
        Fake::new(Scripted::turns(turns)),
    );
    let desk: Arc<dyn RoomHandle> = Arc::new(Desk::with_room(log.clone(), room.clone()));
    (root, room, desk, log)
}

/// The id `request_human` handed back, out of the sentence it returned.
fn asked_id(asked: &str) -> String {
    asked
        .split('`')
        .nth(1)
        .unwrap_or_else(|| panic!("no id in {asked}"))
        .to_string()
}

/// What a seat is shown when it subscribes: the snapshot frame.
async fn seat_snapshot(
    seat: Seat,
    room: &Arc<dyn RoomHandle>,
    log: &Log,
    sub: Value,
) -> Vec<Value> {
    let (tx, mut frames) = mpsc::unbounded_channel();
    let outbox = Outbox {
        call_owner: "desk".into(),
        pairing_cancel: tokio_util::sync::CancellationToken::new(),
        auth_attempts: Arc::default(),
        account_logins: Arc::default(),
        pairing: Arc::default(),
        uploads: Arc::default(),
        threads2: Arc::default(),
        roster_options: Arc::default(),
        lean: Arc::default(),
        sender: Outgoing::Desk(tx),
        cancel: tokio_util::sync::CancellationToken::new(),
        max: usize::MAX,
    };
    let mut subscriptions = HashMap::new();
    let desk = crate::wire::Agents {
        log: log.clone(),
        room: room.clone(),
        parts: Parts::default(),
    };
    answer(
        &json!({ "id": 1, "sub": sub }),
        seat,
        &desk,
        &outbox,
        &mut subscriptions,
        None,
    )
    .await;
    let snapshot = loop {
        let frame: Value = serde_json::from_str(&frames.recv().await.unwrap()).unwrap();
        assert_ne!(frame["ok"], false, "{frame}");
        if let Some(rows) = frame["snapshot"].as_array() {
            break rows.clone();
        }
    };
    for handle in subscriptions.into_values() {
        handle.abort();
    }
    snapshot
}

/// Whether Ada's row says she is waiting on the person, and her card's
/// status on her tape, as this seat is shown them.
async fn seen_by(
    seat: Seat,
    room: &Arc<dyn RoomHandle>,
    log: &Log,
    action_id: &str,
) -> (bool, Value) {
    let roster = seat_snapshot(seat, room, log, json!({ "view": "roster" })).await;
    let ada = roster
        .iter()
        .find(|row| row["persona"]["id"] == "ada")
        .unwrap_or_else(|| panic!("Ada is on the {seat:?} roster: {roster:?}"));
    let tape = seat_snapshot(seat, room, log, json!({ "tape": "ada" })).await;
    let card = tape
        .iter()
        .rev()
        .find(|event| event["id"] == format!("human:{action_id}"))
        .unwrap_or_else(|| panic!("the card is on the {seat:?} tape: {tape:?}"));
    (ada["waiting"] == true, card["status"].clone())
}

fn deliveries(log: &Log) -> usize {
    log.load(&StreamId::Tape("ada".into()))
        .iter()
        .filter(|event| event["kind"] == "delivery")
        .count()
}

/// Ada asks, then withdraws: the desk and the phone stop showing her as
/// needing the person, the card reads withdrawn, and nothing comes back to
/// her. Bob cannot withdraw her card, and the phone cannot answer it after.
#[tokio::test]
async fn a_withdrawn_card_is_gone_for_the_desk_and_the_phone() {
    let (_root, room, desk, log) = asking_desk();
    let asked = TeammateTools::new(&room, "ada")
        .call("request_human", &json!({ "reason": "Sign in to ChatGPT" }))
        .await
        .unwrap();
    assert!(asked.contains("stays up for 2 hours"), "{asked}");
    let action_id = asked_id(&asked);
    for seat in [Seat::Desk, Seat::Phone] {
        let (waiting, status) = seen_by(seat, &desk, &log, &action_id).await;
        assert!(waiting, "{seat:?} shows Ada waiting while the card is up");
        assert_eq!(status, "pending");
    }

    let theirs = TeammateTools::new(&room, "bob")
        .call("withdraw_request", &json!({ "id": action_id }))
        .await;
    assert!(theirs.is_err(), "{theirs:?}");

    let withdrawn = TeammateTools::new(&room, "ada")
        .call("withdraw_request", &json!({ "id": action_id }))
        .await
        .unwrap();
    assert!(withdrawn.starts_with("Withdrawn."), "{withdrawn}");
    for seat in [Seat::Desk, Seat::Phone] {
        let (waiting, status) = seen_by(seat, &desk, &log, &action_id).await;
        assert!(!waiting, "{seat:?} no longer shows Ada waiting");
        assert_eq!(status, "withdrawn");
    }

    let again = TeammateTools::new(&room, "ada")
        .call("withdraw_request", &json!({ "id": action_id }))
        .await;
    assert!(again.is_err(), "{again:?}");
    let answer = remote_control_answer(
        Seat::Phone,
        &desk,
        &log,
        &Parts::default(),
        json!({ "id": 9, "cmd": "human.answer", "params": {
            "personaId": "ada", "actionId": action_id, "status": "done",
        }}),
    )
    .await;
    assert_eq!(answer["ok"], false, "{answer}");
    room.expire_stale_asks(now_ms() + 25 * 60 * 60_000);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        deliveries(&log),
        0,
        "a withdrawn card tells the teammate nothing"
    );
}

/// A card past its two hours expires for both seats, and the teammate is
/// told once however often the sweep runs.
#[tokio::test]
async fn a_card_past_its_lifetime_expires_and_tells_the_teammate_once() {
    let (_root, room, desk, log) = asking_desk();
    let asked = TeammateTools::new(&room, "ada")
        .call("request_human", &json!({ "reason": "Sign in to xAI" }))
        .await
        .unwrap();
    let action_id = asked_id(&asked);

    room.expire_stale_asks(now_ms() + 2 * 60 * 60_000 - 60_000);
    let (waiting, status) = seen_by(Seat::Phone, &desk, &log, &action_id).await;
    assert!(waiting);
    assert_eq!(status, "pending", "a card inside its two hours stays up");

    let later = now_ms() + 2 * 60 * 60_000 + 1;
    room.expire_stale_asks(later);
    room.expire_stale_asks(later);
    until("Ada is told her request expired", || deliveries(&log) == 1).await;
    for seat in [Seat::Desk, Seat::Phone] {
        let (waiting, status) = seen_by(seat, &desk, &log, &action_id).await;
        assert!(!waiting, "{seat:?} no longer shows Ada waiting");
        assert_eq!(status, "expired");
    }
    let delivery = log
        .load(&StreamId::Tape("ada".into()))
        .into_iter()
        .find(|event| event["kind"] == "delivery")
        .unwrap();
    assert_eq!(delivery["cause"]["status"], "expired");
    assert_eq!(delivery["cause"]["actionId"], action_id.as_str());
    room.expire_stale_asks(later + 60_000);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(deliveries(&log), 1, "the teammate is told once");
}

/// `wait_hours` keeps a card up past the default, and is held to a day.
#[tokio::test]
async fn a_longer_wait_is_honoured_up_to_a_day() {
    let (_root, room, desk, log) = asking_desk();
    let ada = TeammateTools::new(&room, "ada");
    let six = asked_id(
        &ada.call(
            "request_human",
            &json!({ "reason": "Approve the release when you are back", "wait_hours": 6 }),
        )
        .await
        .unwrap(),
    );
    let capped = ada
        .call(
            "request_human",
            &json!({ "reason": "Review the contract next week", "wait_hours": 500 }),
        )
        .await
        .unwrap();
    assert!(capped.contains("stays up for 24 hours"), "{capped}");
    let capped = asked_id(&capped);
    let hour = 60 * 60_000;

    room.expire_stale_asks(now_ms() + 5 * hour);
    assert_eq!(seen_by(Seat::Phone, &desk, &log, &six).await.1, "pending");
    room.expire_stale_asks(now_ms() + 6 * hour + 1);
    assert_eq!(seen_by(Seat::Phone, &desk, &log, &six).await.1, "expired");

    room.expire_stale_asks(now_ms() + 23 * hour);
    assert_eq!(seen_by(Seat::Desk, &desk, &log, &capped).await.1, "pending");
    room.expire_stale_asks(now_ms() + 24 * hour + 1);
    assert_eq!(seen_by(Seat::Desk, &desk, &log, &capped).await.1, "expired");
    until("Ada is told of both", || deliveries(&log) == 2).await;
}
