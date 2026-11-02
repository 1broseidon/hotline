//! A main session's run state, and what a restart does with the lines behind
//! it: one answer to "is it running", whichever way it is asked, and no
//! line lost between the spill to `pending.json` and the process ending.

use super::*;

/// The last state the room pushed about `persona_id`.
fn last_pushed(infos: &mut broadcast::Receiver<SessionInfo>, persona_id: &str) -> SessionInfo {
    let mut last = None;
    while let Ok(info) = infos.try_recv() {
        if info.persona_id == persona_id {
            last = Some(info);
        }
    }
    last.expect("the room pushed a state")
}

/// A start under way reads as starting on a poll and on the push alike; it
/// ends as ready.
#[tokio::test]
async fn a_start_under_way_is_starting_until_it_is_ready() {
    let permits = Arc::new(Semaphore::new(0));
    let agents = Fake::new(Scripted::new(Vec::new()).slow_to_start(permits.clone()));
    let room = room("run-state-starting", agents);
    let mut infos = room.subscribe_info();
    let starting = tokio::spawn({
        let room = room.clone();
        async move { room.start("ada").await }
    });
    until_state(&room, "ada", SessionState::Starting).await;
    assert_eq!(
        last_pushed(&mut infos, "ada").state,
        SessionState::Starting,
        "the push said something else"
    );
    permits.add_permits(1);
    starting.await.unwrap().unwrap();
    assert_eq!(room.info("ada").state, SessionState::Ready);
    assert_eq!(last_pushed(&mut infos, "ada").state, SessionState::Ready);
}

/// A start that fails leaves the teammate stopped with the reason, and a
/// poll after the push says the same thing: the error is not erased.
#[tokio::test]
async fn a_failed_start_keeps_its_error_on_a_poll() {
    let agents = Fake::new(Scripted::new(Vec::new()).failing_to_start("no key"));
    let room = room("run-state-failed", agents);
    let mut infos = room.subscribe_info();
    assert!(room.start("ada").await.is_err());
    let pushed = last_pushed(&mut infos, "ada");
    let polled = room.info("ada");
    assert_eq!(pushed.state, SessionState::Stopped);
    assert_eq!(polled.state, pushed.state);
    assert!(polled.error.as_deref().unwrap().contains("no key"));
    assert_eq!(polled.error, pushed.error);
}

/// A stop is the same word on the poll as on the push, and a start after it
/// clears the record.
#[tokio::test]
async fn a_stopped_teammate_polls_as_the_push_said() {
    let room = room("run-state-stop", Fake::new(Scripted::new(Vec::new())));
    room.start("ada").await.unwrap();
    let mut infos = room.subscribe_info();
    room.stop("ada").unwrap();
    assert_eq!(room.info("ada").state, last_pushed(&mut infos, "ada").state);
    assert_eq!(room.info("ada").state, SessionState::Stopped);
    room.start("ada").await.unwrap();
    assert_eq!(room.info("ada").state, SessionState::Ready);
    assert_eq!(room.info("ada").error, None);
}

/// A rebuild that finds a turn already claimed leaves the session alone,
/// whoever asked: the claim is rechecked where the session is swapped out.
#[tokio::test]
async fn a_rebuild_never_cuts_a_turn_a_message_has_claimed() {
    let agents = Fake::new(Scripted::new(Vec::new()));
    let room = room("run-state-claimed", agents.clone());
    room.start("ada").await.unwrap();
    let session = room.session("ada").unwrap();
    // A message admitted in the gap: it holds the gate and has claimed.
    lock(&session.turns).running = true;

    room.rebuild_held("ada").await.unwrap();
    assert!(room.current_session(&session), "the session was replaced");
    assert_eq!(lock(&agents.preambles).len(), 1, "a second agent was built");
    assert_eq!(*lock(&agents.driver.cancels), 0, "the turn was cut");

    // Once the turn is over the same call rebuilds.
    lock(&session.turns).running = false;
    room.rebuild_held("ada").await.unwrap();
    assert!(!room.current_session(&session));
    assert_eq!(lock(&agents.preambles).len(), 2);
}

/// The sign-in's mark stays until the rebuild is done: a restart lease that
/// refuses it for now does not spend the mark.
#[tokio::test]
async fn a_sign_in_rebuild_refused_for_now_is_kept_for_later() {
    let agents = Fake::new(Scripted::new(Vec::new()));
    let room = room("run-state-sign-in", agents.clone());
    room.start("ada").await.unwrap();
    let session = room.session("ada").unwrap();
    session
        .rebuild_for_sign_in
        .store(true, std::sync::atomic::Ordering::SeqCst);

    let preparing = room.prepare_restart().unwrap();
    room.rebuild_for_sign_in_now(&session).await;
    assert!(
        session
            .rebuild_for_sign_in
            .load(std::sync::atomic::Ordering::SeqCst),
        "the mark was spent without a rebuild"
    );
    assert_eq!(lock(&agents.preambles).len(), 1);

    drop(preparing);
    room.rebuild_for_sign_in_now(&session).await;
    assert_eq!(lock(&agents.preambles).len(), 2, "the rebuild never came");
    assert!(
        !session
            .rebuild_for_sign_in
            .load(std::sync::atomic::Ordering::SeqCst)
    );
}

fn pending(room: &Room) -> Vec<Value> {
    std::fs::read(room.log.root().join(stopping::PENDING_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// A message already past the door when the desk begins to stop, waiting
/// behind the chapter's gate, is dispatched after the spill. It is kept for
/// the next start, not run as a turn that extends the drain or dropped.
#[tokio::test]
async fn a_line_admitted_before_the_stop_and_dispatched_after_the_spill_is_kept() {
    let agents = Fake::new(Scripted::new(spoken_turn()));
    let prompts = agents.driver.prompts.clone();
    let room = room("run-state-late-line", agents);
    room.start("ada").await.unwrap();

    let gate = room.start_gate("ada").lock_owned().await;
    let message = tokio::spawn({
        let room = room.clone();
        async move { room.prompt("ada", "late", None, None).await }
    });
    until("the message holds a work lease", || {
        room.activity.clone().try_write_owned().is_err()
    })
    .await;
    let stopping = tokio::spawn({
        let room = room.clone();
        async move { room.stop_for_restart(Duration::from_millis(100)).await }
    });
    until("the room is closing", || room.closing()).await;
    drop(gate);
    message.await.unwrap().unwrap();
    let stopped = stopping.await.unwrap();

    let kept = pending(&room);
    assert_eq!(kept.len(), 1, "the line was lost: {kept:?}");
    assert!(
        kept[0]["text"].as_str().unwrap().ends_with("late"),
        "{kept:?}"
    );
    assert!(lock(&prompts).is_empty(), "the line ran during the drain");
    drop(stopped);
}

/// Lines the driver had taken into the turn and hands back when it is cut
/// are kept too, rather than run on or dropped.
#[tokio::test]
async fn steered_lines_handed_back_by_a_cut_turn_are_kept_for_the_next_start() {
    let gate = Arc::new(Semaphore::new(0));
    let driver = Scripted::new(spoken_turn()).gated(gate).steerable();
    let agents = Fake::new(driver);
    let prompts = agents.driver.prompts.clone();
    let room = room("run-state-steered", agents);
    room.start("ada").await.unwrap();
    room.prompt("ada", "first", None, None).await.unwrap();
    until_state(&room, "ada", SessionState::Thinking).await;
    room.prompt("ada", "second", None, None).await.unwrap();
    settled(&room, "ada", 2).await;

    let stopped = room.stop_for_restart(Duration::from_millis(100)).await;
    assert_eq!(stopped.interrupted, ["ada"]);
    let kept: Vec<String> = pending(&room)
        .iter()
        .map(|line| line["text"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert!(kept[0].contains("second"), "{kept:?}");
    assert_eq!(lock(&prompts).len(), 1, "the handed-back line ran");
    drop(stopped);
}
