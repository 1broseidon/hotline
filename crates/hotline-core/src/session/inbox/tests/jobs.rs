//! A scheduled job that asks a colleague: the reply comes back into the run
//! that sent it while that run is still going, which it is until the job's
//! next run begins or the job ends; after that it lands in the main
//! conversation, quietly, as any item does.

use super::*;
use crate::contract::{ScheduleKind, ScheduledJob};

const JOB: &str = "job-runners";

/// Ada's loop, made at the desk, that fires only when a test fires it.
fn ada_loop() -> ScheduledJob {
    let now = now_ms();
    ScheduledJob {
        id: JOB.to_string(),
        persona_id: "ada".to_string(),
        kind: ScheduleKind::Loop,
        when: None,
        every: Some(3_600_000),
        calendar: None,
        prompt: "Ask Bob which runners are red, then sum it up".to_string(),
        quiet: None,
        reminder: None,
        goal: None,
        until: None,
        times: None,
        without: None,
        operator_created: true,
        next_at: now + 3_600_000,
        created_at: now - 60_000,
        last_post: None,
        ended_at: None,
        final_firing: None,
        skipped: None,
    }
}

/// How many turns the job's thread has ended.
fn job_turns(room: &Room) -> usize {
    room.log
        .load(&StreamId::Job(JOB.to_string()))
        .iter()
        .filter(|event| event["kind"] == "turn")
        .count()
}

/// The job fires, and its run's turn ends.
async fn run(room: &Arc<Room>) {
    let before = job_turns(room);
    room.fire_job(&ada_loop()).unwrap();
    until("the run's turn has ended", || job_turns(room) > before).await;
}

/// Ada's job sends Bob an item, and Bob answers it: the reply in Ada's inbox.
async fn asked_and_answered(room: &Arc<Room>) -> InboxItem {
    let sent = TeammateTools::new(room, "ada")
        .for_job(JOB)
        .call(
            "send",
            &json!({"to": ["bob"], "title": "Which runners are red?", "message": "For the morning summary."}),
        )
        .await
        .unwrap();
    let id = serde_json::from_str::<Value>(&sent).unwrap()["sent"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    tool(
        room,
        "bob",
        "update_inbox",
        json!({"items": [{"id": id, "answer": "linux-2 and mac-1 are red."}]}),
    )
    .await
    .unwrap();
    items(room, "ada")
        .into_iter()
        .find(|reply| reply.reply_to.as_deref() == Some(id.as_str()))
        .expect("the reply is in Ada's inbox")
}

/// The run's turn has ended, but the job's next run has not begun: the run
/// is still going, so Bob's reply is heard in it, a turn of the job's thread
/// that offers it, and Ada's main conversation is not woken.
#[tokio::test]
async fn a_reply_that_comes_back_before_the_jobs_next_run_is_heard_in_that_run() {
    let (room, agents) = room("job-reply-in-run");
    crate::room::append_schedule(&room.log, &ada_loop()).unwrap();
    run(&room).await;
    let reply = asked_and_answered(&room).await;
    assert_eq!(reply.about, Some(ThreadId::job(JOB)));
    assert_eq!(room.inbox_heard_in(&reply), ThreadId::job(JOB));
    until("the job's run hears the reply", || {
        agents.told().iter().any(|line| {
            line.contains("Replies to what this run sent")
                && line.contains("Re: Which runners are red?")
        })
    })
    .await;
    assert!(item(&room, "ada", &reply.id).offered.is_some());
    tokio::time::sleep(heard::gather_delay() * 3).await;
    assert!(
        agents
            .told()
            .iter()
            .all(|line| !line.contains("quiet inbox")),
        "Ada's main conversation was not woken: {:?}",
        agents.told()
    );
}

/// Once the job's next run has begun, the run that sent the item is over:
/// Bob's reply lands in Ada's main conversation, quietly, as today.
#[tokio::test]
async fn once_the_jobs_next_run_has_begun_a_reply_to_the_run_before_lands_in_the_main_conversation()
{
    let (room, agents) = room("job-reply-after-run");
    crate::room::append_schedule(&room.log, &ada_loop()).unwrap();
    run(&room).await;
    let sent = TeammateTools::new(&room, "ada")
        .for_job(JOB)
        .call(
            "send",
            &json!({"to": ["bob"], "title": "Which runners are red?"}),
        )
        .await
        .unwrap();
    let id = serde_json::from_str::<Value>(&sent).unwrap()["sent"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    // The next run begins after a clock tick, so it began after the send.
    tokio::time::sleep(Duration::from_millis(5)).await;
    run(&room).await;
    tool(
        &room,
        "bob",
        "update_inbox",
        json!({"items": [{"id": id, "answer": "linux-2 is red."}]}),
    )
    .await
    .unwrap();
    let reply = items(&room, "ada")
        .into_iter()
        .find(|reply| reply.reply_to.as_deref() == Some(id.as_str()))
        .expect("the reply is in Ada's inbox");
    assert_eq!(reply.about, None);
    assert_eq!(room.inbox_heard_in(&reply), ThreadId::dm("ada"));
    until("Ada's main conversation hears it in a quiet turn", || {
        agents
            .told()
            .iter()
            .any(|line| line.contains("quiet inbox") && line.contains("Re: Which runners are red?"))
    })
    .await;
    assert!(
        agents
            .told()
            .iter()
            .all(|line| !line.contains("Replies to what this run sent")),
        "the job is not woken for it"
    );
}
