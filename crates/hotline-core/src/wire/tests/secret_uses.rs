//! Uses cross the real authenticated seat handler and the computer's fake door.
use super::*;
use crate::contract::SecretUse;
use crate::session::tests::secret_use_room;
use crate::test_support::until;

fn cards(log: &Log) -> Vec<Value> {
    log.load(&StreamId::Tape("ada".into()))
        .into_iter()
        .filter(|e| e["kind"] == "secret_use_ask")
        .collect()
}
fn desk(room: &Arc<Room>) -> Arc<dyn RoomHandle> {
    Arc::new(crate::desk::Desk::with_room(
        room.log().clone(),
        room.clone(),
    ))
}
async fn answer(
    desk: &Arc<dyn RoomHandle>,
    log: &Log,
    seat: Seat,
    id: &str,
    choice: &str,
) -> Value {
    remote_control_answer(seat, desk, log, &Parts::default(), json!({"id":1,"cmd":"secrets.use.answer","params":{"personaId":"ada","useId":id,"answer":choice}})).await
}
async fn sees(desk: &Arc<dyn RoomHandle>, log: &Log, status: &str) {
    for seat in [Seat::Desk, Seat::Phone] {
        let snapshot = super::asks::seat_snapshot(seat, desk, log, json!({"tape":"ada"})).await;
        let uses: Vec<_> = snapshot
            .iter()
            .filter(|e| e["kind"] == "secret_use_ask")
            .collect();
        assert_eq!(uses.len(), 1, "{seat:?}: {uses:?}");
        assert_eq!(uses[0]["status"], status);
        let roster = super::asks::seat_snapshot(seat, desk, log, json!({"view":"roster"})).await;
        assert_eq!(roster[0]["waiting"], status == "pending", "{roster:?}");
    }
}

#[tokio::test]
async fn ask_me_use_has_one_card_on_desk_and_phone_and_one_answer_reaches_the_computer() {
    let fixture = secret_use_room("use-two-seats", "0.11.2", SecretUse::Ask).await;
    let room = &fixture.room;
    let desk = desk(room);
    let id = fixture.taken.uses.hold("GH_PASSKEY", "github.com").unwrap();
    // Exercise the room's own two-second watch, without a pane polling it.
    until("the background watch raises a use card", || {
        cards(room.log()).len() == 1
    })
    .await;
    room.poll_secret_uses("ada").await;
    room.poll_secret_uses("ada").await;
    sees(&desk, room.log(), "pending").await;
    let card = &cards(room.log())[0];
    assert_eq!(card["secret"], "GH_PASSKEY");
    assert_eq!(card["site"], "github.com");
    assert_eq!(card["path"], "/login");
    assert_eq!(card["teammate"], "ada");
    let (phone, owner) = tokio::join!(
        answer(&desk, room.log(), Seat::Phone, &id, "approve"),
        answer(&desk, room.log(), Seat::Owner, &id, "approve")
    );
    assert_ne!(phone["ok"], owner["ok"], "{phone} {owner}");
    assert_eq!(
        fixture.taken.uses.answers(),
        vec![json!({"id":id,"answer":"approve"})]
    );
    room.poll_secret_uses("ada").await;
    sees(&desk, room.log(), "approved").await;
}

#[tokio::test]
async fn allow_ten_minutes_is_one_key_one_site_one_teammate_and_lapses() {
    let fixture = secret_use_room("use-allow", "0.11.2", SecretUse::Ask).await;
    let room = &fixture.room;
    let desk = desk(room);
    let id = fixture.taken.uses.hold("GH_PASSKEY", "github.com").unwrap();
    room.poll_secret_uses("ada").await;
    assert_eq!(
        answer(&desk, room.log(), Seat::Phone, &id, "allow").await["ok"],
        true
    );
    assert_eq!(cards(room.log())[0]["status"], "allowed");
    assert!(
        fixture
            .taken
            .uses
            .hold("GH_PASSKEY", "github.com")
            .is_none()
    );
    assert!(fixture.taken.uses.hold("OTHER_KEY", "github.com").is_some());
    assert!(
        fixture
            .taken
            .uses
            .hold("GH_PASSKEY", "another.example")
            .is_some()
    );
    let other = secret_use_room("use-other-teammate", "0.11.2", SecretUse::Ask).await;
    assert!(other.taken.uses.hold("GH_PASSKEY", "github.com").is_some());
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(601)).await;
    assert!(
        fixture
            .taken
            .uses
            .hold("GH_PASSKEY", "github.com")
            .is_some()
    );
    tokio::time::resume();
    room.poll_secret_uses("ada").await;
    assert_eq!(
        cards(room.log())
            .iter()
            .filter(|c| c["secret"] == "GH_PASSKEY"
                && c["site"] == "github.com"
                && c["status"] == "pending")
            .count(),
        1
    );
}

#[tokio::test]
async fn denying_a_secret_use_tells_the_agent_the_person_declined() {
    let fixture = secret_use_room("use-deny", "0.11.2", SecretUse::Ask).await;
    let room = &fixture.room;
    let desk = desk(room);
    let id = fixture
        .taken
        .uses
        .hold("HN_LOGIN", "news.ycombinator.com")
        .unwrap();
    room.poll_secret_uses("ada").await;
    assert_eq!(
        answer(&desk, room.log(), Seat::Phone, &id, "deny").await["ok"],
        true
    );
    sees(&desk, room.log(), "denied").await;
    until("the agent receives the denial", || {
        room.log()
            .load(&StreamId::Tape("ada".into()))
            .iter()
            .any(|e| {
                e["kind"] == "delivery"
                    && e["text"]
                        .as_str()
                        .is_some_and(|text| text.contains("person declined this use of HN_LOGIN"))
            })
    })
    .await;
    assert_eq!(fixture.taken.uses.answers()[0]["answer"], "deny");
}

#[tokio::test]
async fn a_page_that_stops_waiting_expires_both_seats_and_tells_the_agent_it_lapsed() {
    let fixture = secret_use_room("use-expired", "0.11.2", SecretUse::Ask).await;
    let room = &fixture.room;
    let desk = desk(room);
    let id = fixture.taken.uses.hold("GH_PASSKEY", "github.com").unwrap();
    room.poll_secret_uses("ada").await;
    fixture.taken.uses.expire(&id);
    room.poll_secret_uses("ada").await;
    sees(&desk, room.log(), "expired").await;
    assert_eq!(
        answer(&desk, room.log(), Seat::Owner, &id, "approve").await["ok"],
        false
    );
    until("the agent receives the lapse", || {
        room.log()
            .load(&StreamId::Tape("ada".into()))
            .iter()
            .any(|e| {
                e["kind"] == "delivery"
                    && e["text"]
                        .as_str()
                        .is_some_and(|text| text.contains("lapsed before it was answered"))
            })
    })
    .await;
    assert_eq!(fixture.taken.uses.answers(), [] as [Value; 0]);
}

#[tokio::test]
async fn automatic_keys_are_not_held_and_policy_changes_reach_the_next_use_without_restart() {
    let fixture = secret_use_room("use-change", "0.11.2", SecretUse::Auto).await;
    let room = &fixture.room;
    let desk = desk(room);
    assert!(
        fixture
            .taken
            .uses
            .hold("HN_LOGIN", "news.ycombinator.com")
            .is_none()
    );
    room.poll_secret_uses("ada").await;
    assert_eq!(cards(room.log()), [] as [Value; 0]);
    assert_eq!(fixture.taken.uses.polls(), 0);
    let frame = json!({"id":1,"cmd":"secrets.use.set","params":{"name":"HN_LOGIN","use":"ask"}});
    assert_eq!(
        remote_control_answer(
            Seat::Phone,
            &desk,
            room.log(),
            &Parts::default(),
            frame.clone()
        )
        .await["ok"],
        false
    );
    let updated =
        remote_control_answer(Seat::Desk, &desk, room.log(), &Parts::default(), frame).await;
    assert_eq!(updated["ok"], true, "{updated}");

    assert_eq!(
        fixture.taken.sets().last().unwrap()["HN_LOGIN"]["use"],
        "ask"
    );
    assert!(
        fixture
            .taken
            .uses
            .hold("HN_LOGIN", "news.ycombinator.com")
            .is_some()
    );
    room.poll_secret_uses("ada").await;
    sees(&desk, room.log(), "pending").await;
    desk.secrets_use_set("HN_LOGIN", SecretUse::Auto)
        .await
        .unwrap();
    room.poll_secret_uses("ada").await;
    assert!(
        fixture
            .taken
            .uses
            .hold("HN_LOGIN", "news.ycombinator.com")
            .is_none()
    );
    sees(&desk, room.log(), "expired").await;
}

#[tokio::test]
async fn use_answers_refuse_unknown_wrong_teammate_revoked_and_page_expired_requests() {
    let fixture = secret_use_room("use-refusals", "0.11.2", SecretUse::Ask).await;
    let room = &fixture.room;
    let desk = desk(room);
    assert_eq!(
        answer(&desk, room.log(), Seat::Phone, "unknown", "approve").await["ok"],
        false
    );
    let id = fixture.taken.uses.hold("GH_PASSKEY", "github.com").unwrap();
    room.poll_secret_uses("ada").await;
    assert!(
        desk.secrets_use_answer(
            "someone-else",
            &id,
            crate::contract::SecretUseAnswer::Approve
        )
        .await
        .is_err()
    );
    fixture.taken.uses.expire(&id);
    // 409 settles the card even before the next poll.
    assert_eq!(
        answer(&desk, room.log(), Seat::Owner, &id, "approve").await["ok"],
        false
    );
    assert_eq!(cards(room.log())[0]["status"], "expired");
    let id = fixture.taken.uses.hold("GH_PASSKEY", "github.com").unwrap();
    room.poll_secret_uses("ada").await;
    room.stop("ada").unwrap();
    assert_eq!(
        answer(&desk, room.log(), Seat::Owner, &id, "allow").await["ok"],
        false
    );
    assert_eq!(fixture.taken.uses.answers(), [] as [Value; 0]);
}

#[tokio::test]
async fn an_older_computer_gets_automatic_keys_without_changing_the_vault_choice() {
    let fixture = secret_use_room("use-old-computer", "0.10.3", SecretUse::Ask).await;
    let room = &fixture.room;
    let desk = desk(room);
    room.poll_secret_uses("ada").await;
    assert_eq!(
        fixture.taken.sets().last().unwrap()["GH_PASSKEY"]["use"],
        "auto"
    );
    assert!(
        fixture
            .taken
            .uses
            .hold("GH_PASSKEY", "github.com")
            .is_none()
    );
    let delivered = fixture.taken.sets().len();
    room.poll_secret_uses("ada").await;
    room.poll_secret_uses("ada").await;
    assert_eq!(
        fixture.taken.sets().len(),
        delivered,
        "the 404 fallback runs once per running computer"
    );
    assert_eq!(cards(room.log()), [] as [Value; 0]);
    assert_eq!(desk.secrets_list().unwrap()[0].r#use, SecretUse::Ask);
    desk.secrets_use_set("HN_LOGIN", SecretUse::Ask)
        .await
        .unwrap();
    assert_eq!(
        fixture.taken.sets().last().unwrap()["HN_LOGIN"]["use"],
        "auto"
    );
}
