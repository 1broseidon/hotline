//! What a teammate set aside in its inbox is offered again once, quietly,
//! when its main conversation goes idle, and never by a turn that offer
//! started (`inbox/pick_up.rs`).

use super::*;
use crate::session::inbox::pick_up::PICK_UP_LINE;
use std::sync::Arc;
use tokio::sync::Semaphore;

/// The pick-ups bob was told: his items are Ada's, and a pick-up of Ada's
/// own, of a reply from him, is not his.
pub(super) fn picked_up(agents: &Fake) -> Vec<String> {
    agents
        .told()
        .into_iter()
        .filter(|told| told.contains(PICK_UP_LINE.0) && told.contains("from Ada"))
        .collect()
}

pub(super) fn kinds_of(room: &Room, persona_id: &str, text: &str) -> Vec<String> {
    room.tape(persona_id)
        .into_iter()
        .filter(|event| event["text"] == text)
        .filter_map(|event| event["kind"].as_str().map(str::to_string))
        .collect()
}

pub(super) async fn idle(session: &Session) {
    until("idle", || !lock(&session.turns).running).await;
}

/// The person's turn, held at its first update by the gate.
pub(super) async fn persons_turn(room: &Arc<Room>, agents: &Fake, text: &str) {
    let before = agents.prompts().len();
    room.prompt("bob", text, None, None).await.unwrap();
    until("the person's turn", || agents.prompts().len() == before + 1).await;
}

/// Bob sorts, as his model would with `update_inbox`.
pub(super) async fn sort(room: &Arc<Room>, entries: Value) -> Value {
    tool(room, "bob", "update_inbox", json!({ "items": entries }))
        .await
        .unwrap()
}

/// Bob, gated, at rest.
pub(super) async fn gated(name: &str) -> (Arc<Room>, Arc<Fake>, Arc<Semaphore>, Arc<Session>) {
    let gate = Arc::new(Semaphore::new(0));
    let (room, agents) = room_on(name, replies().gated(gate.clone()).consuming());
    let session = at_rest(&room, "bob").await;
    (room, agents, gate, session)
}

/// One gated turn says its line and ends: two updates.
pub(super) const A_TURN: usize = 2;

#[test]
fn a_pick_up_is_armed_only_by_a_turn_that_may_and_goes_stale_once_another_turn_begins() {
    let mut pick_up = PickUp::default();
    assert_eq!(pick_up.armed(), None);
    pick_up.ended(true);
    let first = pick_up.armed().expect("armed by the turn that ended");
    pick_up.began();
    assert_eq!(pick_up.armed(), None, "a turn began: it hears the inbox");
    pick_up.ended(true);
    let second = pick_up.armed().unwrap();
    assert_ne!(first, second, "the first turn's pick-up is stale");
    pick_up.ended(false);
    assert_eq!(pick_up.armed(), None, "a pick-up's own turn arms nothing");
    pick_up.ended(true);
    pick_up.disarm();
    assert_eq!(pick_up.armed(), None, "the person's stop");
}

/// Huck's story: busy on the person's work, he is told of three items, reads
/// them, closes two and leaves one for later with why. When the work ends
/// and he is idle, one quiet turn offers him that one, with his reason, and
/// he takes it into work.
#[tokio::test]
async fn an_item_set_aside_while_busy_is_offered_once_quietly_when_idle_and_can_be_taken() {
    let (room, agents, gate, session) = gated("pick-up-huck").await;
    persons_turn(&room, &agents, "the deep research").await;
    let keyboards = send(&room, "ada", "bob", "Keyboards", json!({"wants": "work"})).await;
    let mice = send(&room, "ada", "bob", "Mice", json!({"wants": "nothing"})).await;
    let desks = send(&room, "ada", "bob", "Desks", json!({"wants": "nothing"})).await;
    until("the notice reached the turn", || {
        agents.steered().len() == 1
    })
    .await;
    tool(
        &room,
        "bob",
        "read_inbox",
        json!({"ids": [short(&keyboards), short(&mice), short(&desks)]}),
    )
    .await
    .unwrap();
    sort(
        &room,
        json!([
            {"id": mice, "state": "closed", "outcome": "noted"},
            {"id": desks, "state": "closed", "outcome": "noted"},
            {"id": keyboards, "state": "later", "reason": "Finishing the deep research first; keyboards are a separate task."},
        ]),
    )
    .await;
    tokio::time::sleep(heard::gather_delay() * 2).await;
    assert_eq!(agents.prompts().len(), 1, "nothing while he works");

    gate.add_permits(A_TURN);
    until("one pick-up once he is idle", || {
        picked_up(&agents).len() == 1
    })
    .await;
    let line = &picked_up(&agents)[0];
    assert!(
        line.contains("\"Keyboards\" · from Ada · wants work"),
        "{line}"
    );
    assert!(
        line.contains(": \"Finishing the deep research first; keyboards are a separate task.\""),
        "with the reason he gave: {line}"
    );
    assert!(line.contains("for later"), "{line}");
    assert!(
        !line.contains("Mice") && !line.contains("Desks"),
        "nothing closed: {line}"
    );
    assert!(!line.contains(heard::QUIET_WAKE_LINE), "{line}");
    assert!(room.is_quiet(&ThreadId::dm("bob")), "the pick-up is quiet");

    let taken = sort(&room, json!([{"id": keyboards, "state": "taken"}])).await;
    assert_eq!(taken["workThreads"].as_array().unwrap().len(), 1, "{taken}");
    assert_eq!(item(&room, "bob", &keyboards).state, InboxState::Taken);
    gate.add_permits(100);
    until("the pick-up ended", || {
        kinds_of(&room, "bob", "said-1") == ["thought"]
    })
    .await;
    idle(&session).await;
    tokio::time::sleep(heard::gather_delay() * 3).await;
    assert_eq!(picked_up(&agents).len(), 1, "offered once");
}

/// A teammate that answers "later" every time settles after one pick-up:
/// the turn the pick-up started arms none, and the person's next turn
/// offers it nothing either, since the item has had its pick-up and waits
/// for its own clock.
#[tokio::test]
async fn a_pick_up_turn_is_followed_by_no_other_however_much_it_leaves_open() {
    let (room, agents, gate, session) = gated("pick-up-no-loop").await;
    persons_turn(&room, &agents, "the person's work").await;
    let id = send(&room, "ada", "bob", "Keyboards", json!({})).await;
    sort(
        &room,
        json!([{"id": id, "state": "later", "reason": "not now"}]),
    )
    .await;
    gate.add_permits(A_TURN);
    until("the pick-up", || picked_up(&agents).len() == 1).await;
    sort(
        &room,
        json!([{"id": id, "state": "later", "reason": "still not now"}]),
    )
    .await;
    gate.add_permits(A_TURN);
    idle(&session).await;
    assert_eq!(
        lock(&session.pick_up).armed(),
        None,
        "the pick-up's turn armed nothing"
    );
    tokio::time::sleep(heard::gather_delay() * 3).await;
    assert_eq!(agents.told().len(), 2, "settled: {:?}", agents.told());
    assert_eq!(item(&room, "bob", &id).state, InboxState::Later);

    persons_turn(&room, &agents, "and now").await;
    assert!(
        agents.told()[2].contains("INBOX · 1 for later"),
        "the person's turn has it in its index: {}",
        agents.told()[2]
    );
    gate.add_permits(A_TURN);
    idle(&session).await;
    tokio::time::sleep(heard::gather_delay() * 3).await;
    assert_eq!(picked_up(&agents).len(), 1, "{:?}", agents.told());
    assert_eq!(agents.told().len(), 3, "its clock's: {:?}", agents.told());
}

/// However many turns the person starts, an item set aside is offered by a
/// turn's end once: the first time the conversation goes idle after it was
/// set aside. Every later offer is its own clock's.
#[tokio::test]
async fn ten_turns_of_the_persons_bring_one_pick_up_of_an_item_set_aside() {
    let (room, agents, gate, session) = gated("pick-up-ten-turns").await;
    persons_turn(&room, &agents, "turn 0").await;
    let id = send(&room, "ada", "bob", "Keyboards", json!({})).await;
    sort(
        &room,
        json!([{"id": id, "state": "later", "reason": "after the research"}]),
    )
    .await;
    gate.add_permits(A_TURN);
    until("the pick-up", || picked_up(&agents).len() == 1).await;
    gate.add_permits(A_TURN);
    idle(&session).await;
    for turn in 1..10 {
        persons_turn(&room, &agents, &format!("turn {turn}")).await;
        gate.add_permits(A_TURN);
        idle(&session).await;
        tokio::time::sleep(heard::gather_delay() * 3).await;
    }
    assert_eq!(picked_up(&agents).len(), 1, "{:?}", agents.told());
    assert_eq!(agents.told().len(), 11, "ten turns and one pick-up");
    assert_eq!(item(&room, "bob", &id).state, InboxState::Later);
}

/// Nothing open, nothing offered: what the teammate closed, answered or is
/// working on is not set aside.
#[tokio::test]
async fn a_teammate_with_nothing_open_gets_no_pick_up() {
    let (room, agents, gate, session) = gated("pick-up-nothing-open").await;
    persons_turn(&room, &agents, "the person's work").await;
    let closed = send(&room, "ada", "bob", "Closed one", json!({})).await;
    let answered = send(&room, "ada", "bob", "Answered one", json!({})).await;
    let working = send(&room, "ada", "bob", "Worked here", json!({"wants": "work"})).await;
    sort(
        &room,
        json!([
            {"id": closed, "state": "closed", "outcome": "dropped"},
            {"id": answered, "answer": "Yes."},
            {"id": working, "state": "taken", "in_thread": false},
        ]),
    )
    .await;
    assert_eq!(room.set_aside("bob"), [] as [InboxItem; 0]);
    gate.add_permits(A_TURN);
    idle(&session).await;
    tokio::time::sleep(heard::gather_delay() * 3).await;
    // Ada, woken by the answer, takes turns of her own; bob takes none.
    assert!(picked_up(&agents).is_empty(), "{:?}", agents.told());
    assert_eq!(
        agents
            .told()
            .iter()
            .filter(|told| told.contains("from Ada"))
            .count(),
        0,
        "{:?}",
        agents.told()
    );
}

/// The person's line in the gather delay starts its turn, which has the
/// items in its index; the pick-up the turn before armed is stale and does
/// nothing, and the one that follows is armed by the person's turn.
#[tokio::test]
async fn the_persons_line_in_the_gather_delay_wins_and_the_pick_up_follows_its_turn_instead() {
    let (room, agents, gate, session) = gated("pick-up-person-wins").await;
    // The delay outlasts the test: it fires each pick-up itself.
    *lock(&room.pick_up_delay) = Some(Duration::from_secs(3_600));
    persons_turn(&room, &agents, "the person's work").await;
    let id = send(&room, "ada", "bob", "Keyboards", json!({})).await;
    sort(
        &room,
        json!([{"id": id, "state": "later", "reason": "after this"}]),
    )
    .await;
    gate.add_permits(A_TURN);
    idle(&session).await;
    let first = lock(&session.pick_up).armed().expect("armed");

    persons_turn(&room, &agents, "one more thing").await;
    assert_eq!(lock(&session.pick_up).armed(), None, "the turn began");
    room.pick_up("bob", first).await;
    assert_eq!(
        agents.told().len(),
        2,
        "nothing while the person's turn runs"
    );
    assert!(
        agents.told()[1].contains("INBOX · 1 for later"),
        "{}",
        agents.told()[1]
    );
    gate.add_permits(A_TURN);
    idle(&session).await;
    let second = lock(&session.pick_up).armed().expect("armed again");
    assert_ne!(first, second);
    room.pick_up("bob", first).await;
    assert!(!lock(&session.turns).running, "the first is stale");
    assert_eq!(picked_up(&agents), [] as [String; 0]);

    room.pick_up("bob", second).await;
    until("the person's turn's pick-up", || {
        picked_up(&agents).len() == 1
    })
    .await;
    gate.add_permits(A_TURN);
    idle(&session).await;
}

/// A turn the person stopped is followed by nothing, and their Stop while a
/// pick-up waits disarms it.
#[tokio::test]
async fn a_stopped_turn_is_followed_by_no_pick_up() {
    let gate = Arc::new(Semaphore::new(0));
    let (room, agents) = room_on(
        "pick-up-stopped",
        replies()
            .gated(gate.clone())
            .consuming()
            .on_cancel(vec![Update::Turn {
                stop_reason: "cancelled".into(),
                usage: None,
            }]),
    );
    let session = at_rest(&room, "bob").await;
    persons_turn(&room, &agents, "the person's work").await;
    let id = send(&room, "ada", "bob", "Keyboards", json!({})).await;
    sort(
        &room,
        json!([{"id": id, "state": "later", "reason": "after this"}]),
    )
    .await;
    room.cancel("bob").unwrap();
    idle(&session).await;
    assert_eq!(lock(&session.pick_up).armed(), None);
    tokio::time::sleep(heard::gather_delay() * 3).await;
    assert_eq!(agents.told().len(), 1, "{:?}", agents.told());

    // Stopped while one waits.
    *lock(&room.pick_up_delay) = Some(Duration::from_secs(3_600));
    persons_turn(&room, &agents, "go on").await;
    gate.add_permits(A_TURN);
    idle(&session).await;
    let armed = lock(&session.pick_up).armed().expect("armed");
    room.cancel("bob").unwrap();
    room.pick_up("bob", armed).await;
    assert!(!lock(&session.turns).running);
    assert!(picked_up(&agents).is_empty(), "{:?}", agents.told());
}

/// An item `from` sent `to`, kept as `state` without waking anyone: what a
/// desk that stopped left behind.
pub(super) fn left_behind(
    room: &Room,
    from: &str,
    to: &str,
    title: &str,
    later: Option<&str>,
) -> String {
    let sender = room.persona(from).unwrap();
    let mut kept = crate::session::inbox::item(
        &sender,
        &ThreadId::dm(from),
        to,
        title.into(),
        String::new(),
        InboxWants::Answer,
    );
    moved(&mut kept, InboxState::New, None, None, None);
    if let Some(reason) = later {
        kept.offered = Some(now_ms());
        moved(
            &mut kept,
            InboxState::Later,
            Some(reason.into()),
            None,
            None,
        );
    }
    room.keep_item(&kept).unwrap();
    kept.id
}

/// The pick-ups told to the teammate whose items `from` sent.
pub(super) fn picked_up_from(agents: &Fake, from: &str) -> Vec<String> {
    agents
        .told()
        .into_iter()
        .filter(|told| told.contains(PICK_UP_LINE.0) && told.contains(&format!("from {from}")))
        .collect()
}

/// The desk opens on the data a stopped one left.
pub(super) fn reopened(room: Arc<Room>) -> (Arc<Room>, Arc<Fake>) {
    let log = room.log.clone();
    drop(room);
    let agents = Fake::new(replies());
    (
        Room::with_agents(log, Arc::new(DeskKeys), agents.clone()),
        agents,
    )
}

/// A desk that opens offers each teammate what it left for later, once, in a
/// quiet turn it starts the teammate for, without waiting for another turn;
/// the turn arms nothing, so an idle stretch after it offers nothing more,
/// and a second look at start offers nothing again.
#[tokio::test]
async fn a_desk_that_opens_offers_what_was_set_aside_once_quietly_and_nothing_more() {
    let (room, _) = room("pick-up-restart");
    left_behind(&room, "ada", "bob", "Keyboards", Some("after the research"));
    left_behind(&room, "bob", "ada", "Monitors", Some("after the release"));
    let (room, agents) = reopened(room);
    room.recover_exchanges().await;
    until("each teammate is offered its own", || {
        picked_up_from(&agents, "Ada").len() == 1 && picked_up_from(&agents, "Bob").len() == 1
    })
    .await;
    let bobs = &picked_up_from(&agents, "Ada")[0];
    assert!(
        bobs.contains("\"Keyboards\" · from Ada") && bobs.contains(": \"after the research\""),
        "{bobs}"
    );
    let bob = room.session("bob").unwrap();
    let ada = room.session("ada").unwrap();
    idle(&bob).await;
    idle(&ada).await;
    assert!(
        room.tape("bob")
            .iter()
            .chain(room.tape("ada").iter())
            .all(|event| event["kind"] != "agent"),
        "both quiet"
    );
    assert_eq!(lock(&bob.pick_up).armed(), None, "its turn armed nothing");

    room.recover_exchanges().await;
    tokio::time::sleep(heard::gather_delay() * 4).await;
    assert_eq!(agents.told().len(), 2, "{:?}", agents.told());
    assert_eq!(room.set_aside("bob").len(), 1, "still for later");
}

/// An item no turn was offered still wakes its receiver at start by its own
/// wake, which wins over the pick-up; what is set aside is offered once, by
/// the pick-up that wake's turn arms.
#[tokio::test]
async fn at_start_an_item_never_offered_wakes_by_its_own_wake_and_the_pick_up_follows_that_turn() {
    let (room, _) = room("pick-up-restart-unoffered");
    left_behind(&room, "ada", "bob", "Keyboards", Some("after the research"));
    left_behind(&room, "ada", "bob", "Crane permit", None);
    let (room, agents) = reopened(room);
    room.recover_exchanges().await;
    until("the new item's wake, then one pick-up", || {
        agents.told().len() == 2
    })
    .await;
    assert!(
        agents.told()[0].contains(heard::QUIET_WAKE_LINE),
        "{}",
        agents.told()[0]
    );
    assert!(
        agents.told()[1].contains(PICK_UP_LINE.0),
        "{}",
        agents.told()[1]
    );
    idle(&room.session("bob").unwrap()).await;
    tokio::time::sleep(heard::gather_delay() * 4).await;
    assert_eq!(agents.told().len(), 2, "{:?}", agents.told());
}
