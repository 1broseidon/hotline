//! An item is taken into one work thread only: a second take, in the same
//! turn or a later one, in a thread or not, is refused with the thread
//! that has it, and of two takes at once only one opens a thread.

use super::*;

/// The number of work requests the two teammates' pair carries.
fn requests(room: &Room) -> usize {
    pair(room)
        .and_then(|pair| pair["requests"].as_array().map(Vec::len))
        .unwrap_or(0)
}

#[tokio::test]
async fn a_second_take_of_an_item_in_a_work_thread_is_refused_with_the_thread_that_has_it() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let (room, _) = room_on("inbox-taken-once", replies().gated(gate.clone()));
    let id = send(
        &room,
        "ada",
        "bob",
        "Fix the crane",
        json!({"wants": "work"}),
    )
    .await;
    // Taken into a thread and, in the same call, refused a second time.
    let twice = tool(
        &room,
        "bob",
        "update_inbox",
        json!({"items": [{"id": id, "state": "taken"}, {"id": id, "state": "taken", "in_thread": false}]}),
    )
    .await
    .unwrap_err();
    assert!(twice.contains("listed twice"), "{twice}");
    tool(
        &room,
        "bob",
        "update_inbox",
        json!({"items": [{"id": id, "state": "taken"}]}),
    )
    .await
    .unwrap();
    let request = item(&room, "bob", &id).request.expect("taken into work");
    until("its work thread opened", || {
        item(&room, "bob", &id).work.is_some()
    })
    .await;
    let work = item(&room, "bob", &id).work.unwrap();
    for again in [
        json!({"in_thread": false}),
        json!({}),
        json!({"in_thread": true}),
    ] {
        let mut entry = json!({"id": id, "state": "taken"});
        if let (Some(entry), Some(again)) = (entry.as_object_mut(), again.as_object()) {
            entry.extend(again.clone());
        }
        let refused = tool(&room, "bob", "update_inbox", json!({"items": [entry]}))
            .await
            .unwrap_err();
        assert!(
            refused.contains("already taken into the work thread")
                && refused.contains(&crate::session::index::short(&work)),
            "{refused}"
        );
        assert_eq!(
            item(&room, "bob", &id).request.as_deref(),
            Some(request.as_str()),
            "the record of its thread stands"
        );
    }
    assert_eq!(requests(&room), 1, "one request");
    assert_eq!(room.sides("bob").len(), 1, "one work thread");
    gate.add_permits(100);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn of_two_takes_at_once_only_one_opens_a_work_thread() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let (room, agents) = room_on("inbox-taken-race", replies().gated(gate.clone()));
    room.start("bob").await.unwrap();
    let id = send(
        &room,
        "ada",
        "bob",
        "Fix the crane",
        json!({"wants": "work"}),
    )
    .await;
    until("bob woke for it", || agents.told().len() == 1).await;
    // The exchange lock is held where a take queues its work, so the take
    // that gets there first is held mid-take while the other runs.
    let (release, released) = std::sync::mpsc::channel::<()>();
    let (holding, held) = std::sync::mpsc::channel::<()>();
    let holder = {
        let room = room.clone();
        std::thread::spawn(move || {
            let _guard = lock(&room.exchange_lock);
            holding.send(()).expect("the test waits for the lock");
            released.recv().ok();
        })
    };
    held.recv().expect("the lock is held");
    // Each take runs on a thread of its own: the one held mid-take blocks
    // its thread, and a task queued behind it on a runtime worker would wait
    // with it.
    let runtime = tokio::runtime::Handle::current();
    let takes: Vec<_> = (0..2)
        .map(|_| {
            let (room, id, runtime) = (room.clone(), id.clone(), runtime.clone());
            tokio::task::spawn_blocking(move || {
                runtime.block_on(tool(
                    &room,
                    "bob",
                    "update_inbox",
                    json!({"items": [{"id": id, "state": "taken"}]}),
                ))
            })
        })
        .collect();
    until("one take claimed the item", || {
        item(&room, "bob", &id).request.is_some()
    })
    .await;
    until("the other take is answered while the first is held", || {
        takes.iter().any(tokio::task::JoinHandle::is_finished)
    })
    .await;
    release.send(()).expect("the holder waits");
    holder.join().expect("the holder lets go");
    let mut taken = 0;
    for take in takes {
        match take.await.expect("the take ran") {
            Ok(_) => taken += 1,
            Err(refused) => assert!(refused.contains("already taken"), "{refused}"),
        }
    }
    assert_eq!(taken, 1, "taken once");
    assert_eq!(requests(&room), 1, "one request");
    gate.add_permits(100);
}
