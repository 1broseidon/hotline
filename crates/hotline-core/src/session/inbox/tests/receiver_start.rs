//! Cold DM startup overlaps gathering without admitting a turn early.

use super::*;
use crate::contract::SessionState;
use tokio::sync::Semaphore;
use tokio::time::{Instant, advance};

async fn land(room: &Arc<Room>, title: &str) -> String {
    send(
        room,
        "ada",
        "bob",
        title,
        json!({"message": "Full handoff details. ".repeat(20)}),
    )
    .await
}

async fn booting(room: &Room, agents: &Fake, deadline: Instant) {
    until("the fake was built", || agents.views().len() == 1).await;
    assert!(Instant::now() < deadline, "startup did not overlap gather");
    assert_eq!(room.info("bob").state, SessionState::Starting);
    assert_eq!(agents.prompts(), [] as [String; 0]);
}

fn gathered(room: &Room) -> bool {
    lock(&room.inbox_gathers).is_empty()
}

#[tokio::test(start_paused = true)]
async fn cold_start_overlaps_gather_but_the_turn_waits_for_the_whole_batch() {
    let gate = Arc::new(Semaphore::new(0));
    let (room, agents) = room_on("inbox-cold-overlap", replies().slow_to_start(gate.clone()));
    let deadline = Instant::now() + heard::gather_delay();
    let first = land(&room, "First").await;
    booting(&room, &agents, deadline).await;
    let second = land(&room, "Second").await;
    gate.add_permits(1);
    until("Bob is ready", || room.session("bob").is_ok()).await;
    assert!(Instant::now() < deadline);
    assert!(item(&room, "bob", &first).offered.is_none());
    assert!(item(&room, "bob", &second).offered.is_none());
    assert_eq!(
        agents.prompts(),
        [] as [String; 0],
        "the timer still owns the turn"
    );
    advance(heard::gather_delay()).await;
    until("the inbox turn", || !agents.told().is_empty()).await;
    let told = agents.told();
    assert_eq!(told.len(), 1);
    assert!(told[0].contains("First") && told[0].contains("Second"));
    assert_eq!(agents.views().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_failed_early_start_is_not_retried_at_the_deadline() {
    let (room, agents) = room_on("inbox-cold-failure", replies().failing_to_start("offline"));
    let deadline = Instant::now() + heard::gather_delay();
    let id = land(&room, "Wait here").await;
    until("startup was attempted", || agents.views().len() == 1).await;
    assert!(Instant::now() < deadline);
    advance(heard::gather_delay() * 2).await;
    until("the gather retired", || gathered(&room)).await;
    assert_eq!(agents.views().len(), 1);
    assert_eq!(agents.prompts(), [] as [String; 0]);
    let saved = item(&room, "bob", &id);
    assert_eq!(saved.state, InboxState::New);
    assert!(saved.offered.is_none());
}

#[tokio::test(start_paused = true)]
async fn stop_retires_a_slow_start_without_stranding_the_next_arrival() {
    let gate = Arc::new(Semaphore::new(0));
    let (room, agents) = room_on("inbox-cold-stop", replies().slow_to_start(gate.clone()));
    let deadline = Instant::now() + heard::gather_delay();
    let old = land(&room, "Before Stop").await;
    booting(&room, &agents, deadline).await;
    room.stop("bob").unwrap();
    assert!(room.session("bob").is_err());

    // The old task still owns the start gate and has not removed its gather.
    // The new arrival needs its own window in the current authority.
    let new = land(&room, "After Stop").await;
    gate.add_permits(1);
    advance(heard::gather_delay() * 2).await;
    until("a new attempt for the new arrival", || {
        agents.views().len() == 2
    })
    .await;
    assert!(
        room.session("bob").is_err(),
        "the revoked start did not publish"
    );
    assert_eq!(agents.prompts(), [] as [String; 0]);
    assert!(item(&room, "bob", &old).offered.is_none());
    gate.add_permits(1);
    until("the new arrival was offered", || {
        item(&room, "bob", &new).offered.is_some()
    })
    .await;
    assert_eq!(agents.views().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn stopping_an_already_warmed_session_does_not_wake_a_manual_restart() {
    let (room, agents) = room("inbox-cold-stop-after-ready");
    let deadline = Instant::now() + heard::gather_delay();
    let id = land(&room, "Only before Stop").await;
    until("the early start published", || room.session("bob").is_ok()).await;
    assert!(Instant::now() < deadline);
    room.stop("bob").unwrap();
    room.start("bob").await.unwrap();
    advance(heard::gather_delay() * 2).await;
    until("the old gather retired", || gathered(&room)).await;
    assert_eq!(
        agents.views().len(),
        2,
        "only inbox warm and explicit Start"
    );
    assert_eq!(agents.prompts(), [] as [String; 0]);
    assert!(item(&room, "bob", &id).offered.is_none());
}

#[tokio::test(start_paused = true)]
async fn quiet_and_pair_holds_skip_overlap_without_changing_the_delayed_route() {
    for quiet in [true, false] {
        let (room, agents) = room(if quiet {
            "inbox-cold-quiet-held"
        } else {
            "inbox-cold-pair-held"
        });
        let hold = if quiet {
            json!({"kind": "quiet_limit", "id": "quiet-limit:bob", "teammate": "bob", "held": "stopped"})
        } else {
            // This send is admitted, but its count reaches the pair's brake.
            json!({"kind": "exchange_pair", "id": "ada~bob", "a": "ada", "b": "bob",
                "exchanges": 11, "paused": false, "requests": []})
        };
        room.log.append(&StreamId::Room, &hold).unwrap();
        let id = land(&room, "At the hold").await;
        tokio::time::sleep(heard::gather_delay() / 2).await;
        assert!(agents.views().is_empty(), "no overlap for a held receiver");
        assert!(item(&room, "bob", &id).offered.is_none());
        until("the legacy delayed start", || room.session("bob").is_ok()).await;
        if quiet {
            assert_eq!(agents.prompts(), [] as [String; 0]);
            assert!(item(&room, "bob", &id).offered.is_none());
        } else {
            assert_eq!(pair(&room).unwrap()["paused"], true);
            until("the admitted twelfth item", || {
                item(&room, "bob", &id).offered.is_some()
            })
            .await;
        }
    }
}
