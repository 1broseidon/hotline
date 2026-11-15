//! The person's two acts on a work thread from its menu (decision 5): Loop
//! in…, which brings a colleague in for one answer (`inbox/tests.rs` follows
//! it end to end), and End thread, which ends a taken item's thread and its
//! handoff.

use super::*;
use crate::contract::{End, SideEnd};
use crate::thread::ThreadState;

#[tokio::test]
async fn loop_in_is_refused_for_the_threads_own_teammate_and_an_ended_thread() {
    let (room, _agents) = setup("loop-in-refused");
    let opened = room.start_side("ada", "Ship it").await.unwrap();
    let own = room.loop_in(&opened.side_id, "ada").await.unwrap_err();
    assert!(own.contains("own teammate"), "{own}");
    room.archive_side(&opened.side_id, SideEnd::Person, None)
        .unwrap();
    let ended = room.loop_in(&opened.side_id, "bob").await.unwrap_err();
    assert!(ended.contains("has ended"), "{ended}");
    assert_eq!(
        crate::store::inbox::of(&room.log, "bob"),
        [] as [crate::contract::InboxItem; 0]
    );
}

/// Bob's work on Ada's item waits on the person, who ends its thread: it
/// closes as theirs, the handoff is released and stopped with no result, and
/// Ada's item closes as dropped, which she is told at her next turn, with
/// nothing in her chat and no turn.
#[tokio::test]
async fn the_persons_end_thread_releases_the_handoff_and_drops_its_item_once() {
    let (room, id, _action) = suspended_handoff("end-thread").await;
    let thread = work_thread(&room, &id);
    let ada_before = room.tape("ada").len();

    room.archive_side(&thread, SideEnd::Person, None).unwrap();

    let link = room.link_of(&ThreadId::side(&thread)).unwrap();
    assert_eq!(link.state, ThreadState::Closed(End::Person));
    assert!(link.handoff_ended, "the bar says the handoff has ended");
    let request = room
        .exchange_pair("ada~bob")
        .unwrap()
        .requests
        .into_iter()
        .find(|r| r.id == id)
        .unwrap();
    assert!(request.released);
    assert_eq!(request.phase(), Phase::Stopped);
    assert_ne!(request.owed(), Owed::Thread);

    let item = crate::store::inbox::of(&room.log, "bob")
        .into_iter()
        .find(|item| item.request.as_deref() == Some(id.as_str()))
        .unwrap();
    assert_eq!(item.state, crate::contract::InboxState::Closed);
    assert_eq!(item.outcome, Some(crate::contract::InboxOutcome::Dropped));
    assert!(
        item.reason
            .as_deref()
            .unwrap()
            .contains("ended this thread"),
        "{:?}",
        item.reason
    );
    assert!(answer_to(&room, &id).is_none(), "no result and no failure");
    assert_eq!(room.tape("ada").len(), ada_before, "nothing in Ada's chat");
    assert!(!room.mid_turn("ada"), "Ada is not woken");

    // Ending it again moves nothing twice.
    let receipts = item.receipts.len();
    room.archive_side(&thread, SideEnd::Person, None).unwrap();
    let again = crate::store::inbox::of(&room.log, "bob")
        .into_iter()
        .find(|kept| kept.id == item.id)
        .unwrap();
    assert_eq!(again.receipts.len(), receipts);
}
