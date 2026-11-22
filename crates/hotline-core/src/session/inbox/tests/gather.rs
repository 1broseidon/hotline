//! The fast window is for one quick item and an idle receiver, not a batch or a notice.

use super::*;

async fn quick(room: &Arc<Room>, message: &str) {
    tool(
        room,
        "ada",
        "send",
        json!({"to": ["bob"], "message": message}),
    )
    .await
    .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_lone_quick_item_is_offered_before_the_ordinary_gather() {
    let (room, agents) = room("inbox-short-gather");
    at_rest(&room, "bob").await;
    quick(&room, "Is it ready?").await;
    tokio::time::sleep(heard::gather_delay() / 2).await;
    assert!(
        agents
            .told()
            .iter()
            .any(|line| line.contains("Is it ready?"))
    );
}

#[tokio::test(start_paused = true)]
async fn when_a_second_item_joins_the_gather_it_keeps_the_ordinary_window() {
    let (room, agents) = room("inbox-short-gather-batch");
    at_rest(&room, "bob").await;
    quick(&room, "Is it ready?").await;
    send(
        &room,
        "ada",
        "bob",
        "Check the runner",
        json!({"wants": "work"}),
    )
    .await;
    tokio::time::sleep(heard::gather_delay() / 2).await;
    assert_eq!(agents.told(), [] as [String; 0]);
    until("the two items are in the same wake", || {
        agents
            .told()
            .iter()
            .any(|line| line.contains("NEW 2") && line.contains("Check the runner"))
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_busy_receiver_gets_no_quick_notice_before_the_ordinary_gather() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let (room, agents) = room_on(
        "inbox-short-gather-busy",
        replies().gated(gate.clone()).consuming(),
    );
    at_rest(&room, "bob").await;
    room.prompt("bob", "The person's work", None, None)
        .await
        .unwrap();
    until("Bob started working", || agents.told().len() == 1).await;
    quick(&room, "Is it ready?").await;
    tokio::time::sleep(heard::gather_delay() / 2).await;
    assert_eq!(agents.steered(), [] as [String; 0]);
    until("the usual notice reached Bob", || {
        !agents.steered().is_empty()
    })
    .await;
    gate.add_permits(100);
}
