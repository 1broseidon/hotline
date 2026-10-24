//! A request between two teammates, and the delegation a handoff carries,
//! end only by an explicit end: Stop exchange, a revocation (a policy
//! change, either teammate stopped or removed), or the end of the link to the
//! other desk. A chapter turning over or reopened, a parked thread and a
//! job's firing ending leave it standing, and so does a restart: the result,
//! the reports and the questions back keep flowing.
use super::ask_back::{ask_back, in_workspace, open_card, question_in};
use super::gates::{idle, request, turn};
use super::progress::{
    handed_over_and_done, heard_with, is_released, planted_job, room_with, says_ended,
    talks_without_a_report,
};
use super::questions::requested;
use super::*;
use crate::contract::ChapterClose;
use crate::thread::ThreadState;
use tokio::sync::Semaphore;

/// Turns `whose` main conversation over to its next chapter the way the
/// person's line after a closed chapter does: the old session stops, and a
/// new one hears the line.
async fn next_chapter(room: &Arc<Room>, agents: &Fake, whose: &str) {
    room.start(whose).await.unwrap();
    let before = format!("{whose}, before");
    room.prompt(whose, &before, None, None).await.unwrap();
    until("the line is heard", || {
        heard_with(agents, &before).is_some() && !room.mid_turn(whose)
    })
    .await;
    let old = room.session(whose).unwrap();
    room.start_fresh_chapter(whose, ChapterClose::User)
        .await
        .unwrap();
    let after = format!("{whose}, after");
    room.prompt(whose, &after, None, None).await.unwrap();
    until("the next chapter's session hears the line", || {
        heard_with(agents, &after).is_some() && !room.mid_turn(whose)
    })
    .await;
    assert!(
        !Arc::ptr_eq(&old, &room.session(whose).unwrap()),
        "{whose}'s session was replaced"
    );
}

/// The delegator's chapter and its colleague's turn over, and the
/// delegator's previous one is reopened, while the person goes on in the
/// handed-over thread: it still reports to the delegator, its link still
/// says the delegation stands, and its question back still needs no card.
#[tokio::test]
async fn a_delegation_outlives_either_teammates_chapter_turning_over_or_reopening() {
    let (room, agents, handoff, thread) = handed_over_and_done("lasting-chapters").await;
    in_workspace(&room, "bob");
    for whose in ["ada", "bob"] {
        next_chapter(&room, &agents, whose).await;
    }
    room.resume_chapter("ada").await.unwrap();
    assert_eq!(says_ended(&room, &thread), [false; 3]);

    room.continue_side(&thread).await.unwrap();
    room.prompt_side(&thread, "and the date?", None)
        .await
        .unwrap();
    until("the report waits for ada", || {
        request(&room, "ada~bob", &handoff).progress.len() == 1
    })
    .await;
    let id = ask_back(&room, &thread).await;
    until("ada's conversation hears the question", || {
        question_in(&room.tape("ada"), &id).is_some()
    })
    .await;
    assert!(
        open_card(&room, "bob").is_none(),
        "the delegation grants it"
    );
    assert!(!request(&room, "ada~bob", &handoff).released);
}

/// A handoff sent from Ada's own work thread goes on when that thread is
/// parked, and its result comes back to the thread.
#[tokio::test]
async fn a_request_outlives_the_work_thread_it_was_sent_from_parking() {
    let ada = Fake::new(Scripted::new(turn("noted", "Noted.", "end_turn")));
    let gate = Arc::new(Semaphore::new(0));
    let bob = Fake::new(Scripted::new(turn("built", "Built it.", "end_turn")).gated(gate.clone()));
    let room = room_with("lasting-park", &ada, vec![bob.clone()]);
    let own = room
        .start_side("ada", "Watch the build")
        .await
        .unwrap()
        .side_id;
    until("ada's thread has its agent and is idle", || {
        !ada.tools().is_empty() && idle(&room, &own)
    })
    .await;
    // The thread's own agent's tools, under the thread's lease.
    let tools = ada.tools().pop().unwrap();
    let sent = tools
        .call(
            "message_teammate",
            &json!({"to":"bob","message":"Build it.","intent":"handoff"}),
        )
        .await
        .unwrap();
    let handoff = requested(&serde_json::from_str(&sent).unwrap());
    until("bob's turn is running", || bob.prompts().len() == 1).await;

    room.park_side_thread(&own).unwrap();
    gate.add_permits(2);
    done(&room, &handoff).await;
    let result = request(&room, "ada~bob", &handoff);
    assert_eq!((result.failed, result.reply.as_str()), (false, "Built it."));
    until("ada's thread hears it", || {
        room.log
            .load(&StreamId::Side(own.clone()))
            .iter()
            .any(|v| v["kind"] == "delivery" && v["cause"]["requestId"] == handoff.as_str())
    })
    .await;
}

/// A handoff a job's firing sent goes on after that firing has ended and let
/// go of its agent: the result reaches the job.
#[tokio::test]
async fn a_request_outlives_the_job_firing_it_was_sent_from() {
    let ada_gate = Arc::new(Semaphore::new(0));
    let ada =
        Fake::new(Scripted::new(turn("watched", "Watching.", "end_turn")).gated(ada_gate.clone()));
    let bob_gate = Arc::new(Semaphore::new(0));
    let bob =
        Fake::new(Scripted::new(turn("built", "Built it.", "end_turn")).gated(bob_gate.clone()));
    let room = room_with("lasting-job", &ada, vec![bob.clone()]);
    let job = planted_job(&room, "job-build");
    assert!(room.fire_job(&job).unwrap());
    until("the firing's turn is running", || ada.prompts().len() == 1).await;
    // The firing's own agent's tools, under the firing's lease.
    let tools = ada.tools().pop().unwrap();
    let sent = tools
        .call(
            "message_teammate",
            &json!({"to":"bob","message":"Build it.","intent":"handoff"}),
        )
        .await
        .unwrap();
    let handoff = requested(&serde_json::from_str(&sent).unwrap());
    until("bob's turn is running", || bob.prompts().len() == 1).await;

    ada_gate.add_permits(2);
    until("the firing has let go of its agent", || {
        room.link_of(&ThreadId::job(&job.id))
            .is_some_and(|link| link.state == ThreadState::Parked)
    })
    .await;
    ada_gate.add_permits(8);
    bob_gate.add_permits(2);
    done(&room, &handoff).await;
    let result = request(&room, "ada~bob", &handoff);
    assert_eq!((result.failed, result.reply.as_str()), (false, "Built it."));
    until("the job hears it", || {
        room.log
            .load(&StreamId::Job(job.id.clone()))
            .iter()
            .any(|v| v["kind"] == "delivery" && v["cause"]["requestId"] == handoff.as_str())
    })
    .await;
}

/// A handoff the last process accepted is taken up after the restart, and
/// still goes on when its sender's chapter turns over while it works.
#[tokio::test]
async fn a_request_taken_up_after_a_restart_outlives_a_chapter_turning_over() {
    let ada = Fake::new(Scripted::new(turn("heard", "Heard.", "end_turn")));
    let gate = Arc::new(Semaphore::new(0));
    let bob = Fake::new(Scripted::new(turn("built", "Built it.", "end_turn")).gated(gate.clone()));
    let room = room_with("lasting-restart", &ada, vec![bob.clone()]);
    room.save_pair(&Pair {
        id: "ada~bob".into(),
        a: "ada".into(),
        b: "bob".into(),
        exchanges: 0,
        paused: false,
        requests: vec![saved_request("left-over", Intent::Handoff, Phase::Queued)],
    })
    .unwrap();
    room.recover_exchanges().await;
    until("bob's turn is running", || bob.prompts().len() == 1).await;

    next_chapter(&room, &ada, "ada").await;
    gate.add_permits(2);
    done(&room, "left-over").await;
    let result = request(&room, "ada~bob", "left-over");
    assert_eq!((result.failed, result.reply.as_str()), (false, "Built it."));
}

/// What a chapter's turnover leaves standing, every explicit end still
/// ends: the delegation is released, its link says so, and the person's
/// next turn in the thread reports to nobody.
#[tokio::test]
async fn each_explicit_end_still_ends_a_delegation_a_chapter_left_standing() {
    for end in [
        "stop-exchange",
        "delegator-reattached",
        "colleague-reattached",
        "delegator-stopped",
        "colleague-stopped",
    ] {
        let (room, agents, handoff, thread) =
            handed_over_and_done(&format!("lasting-ends-{end}")).await;
        next_chapter(&room, &agents, "ada").await;
        match end {
            "stop-exchange" => room.stop_exchange("ada", "bob").unwrap(),
            "delegator-reattached" => room.reattach("ada").await.unwrap(),
            "colleague-reattached" => room.reattach("bob").await.unwrap(),
            "delegator-stopped" => room.stop("ada").unwrap(),
            _ => room.stop("bob").unwrap(),
        }
        is_released(&room, &handoff);
        talks_without_a_report(&room, &handoff, &thread).await;
    }
}
