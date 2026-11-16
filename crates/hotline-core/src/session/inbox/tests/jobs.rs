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
        awaiting: None,
        from_item: None,
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

/// Ada's one-time job, made at the desk, handed its only run as the clock
/// hands it: the record says its last firing is on its thread.
const ONCE: &str = "job-once-runners";

fn ada_once() -> ScheduledJob {
    let now = now_ms();
    ScheduledJob {
        id: ONCE.to_string(),
        kind: ScheduleKind::Schedule,
        when: Some(now + 3_600_000),
        every: None,
        final_firing: Some(now),
        ..ada_loop()
    }
}

/// How many turns a job's thread has ended.
fn turns_of(room: &Room, job_id: &str) -> usize {
    room.log
        .load(&StreamId::Job(job_id.to_string()))
        .iter()
        .filter(|event| event["kind"] == "turn")
        .count()
}

/// What a job's thread said as notices.
fn notices_of(room: &Room, job_id: &str) -> Vec<String> {
    room.log
        .load(&StreamId::Job(job_id.to_string()))
        .iter()
        .filter(|event| event["kind"] == "notice")
        .filter_map(|event| event["text"].as_str().map(str::to_string))
        .collect()
}

fn once_record(room: &Room) -> ScheduledJob {
    crate::room::all_jobs(&room.log)
        .into_iter()
        .find(|job| job.id == ONCE)
        .unwrap()
}

fn once_closed(room: &Room) -> bool {
    room.link_of(&ThreadId::job(ONCE)).is_some_and(|link| {
        link.state == crate::thread::ThreadState::Closed(crate::contract::End::Done)
    })
}

/// Ada's one-time job runs, and while its turn is starting it sends Bob each
/// of `titles`; its turn then ends. Answers the items' ids, oldest first.
async fn once_sent(
    room: &Arc<Room>,
    gate: &tokio::sync::Semaphore,
    titles: &[&str],
) -> Vec<String> {
    crate::room::append_schedule(&room.log, &ada_once()).unwrap();
    room.fire_job(&ada_once()).unwrap();
    let mut ids = Vec::new();
    for title in titles {
        let sent = TeammateTools::new(room, "ada")
            .for_job(ONCE)
            .call("send", &json!({"to": ["bob"], "title": title}))
            .await
            .unwrap();
        ids.push(
            serde_json::from_str::<Value>(&sent).unwrap()["sent"][0]["id"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }
    gate.add_permits(1_000);
    until("the run's turn has ended", || turns_of(room, ONCE) == 1).await;
    until("the job is at rest", || room.job_at_rest(ONCE)).await;
    ids
}

/// Ada's one-time job, waiting on Bob, with Bob's items' ids. Its agent is
/// held starting while it sends, so what it sends is of its run's turn.
async fn waiting_on_bob(name: &str, titles: &[&str]) -> (Arc<Room>, Arc<Fake>, Vec<String>) {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let (room, agents) = room_on(name, replies().slow_to_start(gate.clone()));
    let ids = once_sent(&room, &gate, titles).await;
    (room, agents, ids)
}

async fn answer(room: &Arc<Room>, id: &str, words: &str) {
    tool(
        room,
        "bob",
        "update_inbox",
        json!({"items": [{"id": id, "answer": words}]}),
    )
    .await
    .unwrap();
}

/// A one-time job's only run that asked Bob is not over when its turn ends:
/// it waits, with no turn running and its agent let go of, its record
/// naming Bob and its thread saying so once. Bob's answer is heard in the
/// job, told it is the job's last run, and the job ends after that turn,
/// its thread closed; the main conversation never hears it.
#[tokio::test]
async fn a_one_time_jobs_run_waits_for_the_reply_hears_it_in_the_job_and_then_ends() {
    let (room, agents, ids) = waiting_on_bob("job-once-waits", &["Which runners are red?"]).await;
    assert!(room.job_lives(ONCE), "the job has not ended");
    assert_eq!(once_record(&room).awaiting, Some(vec!["bob".to_string()]));
    assert_eq!(room.working_jobs("ada"), [] as [String; 0], "no live turn");
    assert_eq!(
        notices_of(&room, ONCE),
        ["Waiting for Bob to reply. The job ends once the reply has come back."]
    );
    assert_eq!(
        room.link_of(&ThreadId::job(ONCE)).map(|link| link.state),
        Some(crate::thread::ThreadState::Parked)
    );

    answer(&room, &ids[0], "linux-2 and mac-1 are red.").await;
    until("the job's run hears the reply", || {
        agents.told().iter().any(|line| {
            line.contains("Re: Which runners are red?")
                && line.contains("This is the job's last run")
        })
    })
    .await;
    until("the job has ended", || once_closed(&room)).await;
    assert!(!room.job_lives(ONCE));
    assert_eq!(once_record(&room).awaiting, None);
    assert_eq!(turns_of(&room, ONCE), 2);
    tokio::time::sleep(heard::gather_delay() * 3).await;
    assert!(
        agents
            .told()
            .iter()
            .all(|line| !(line.contains("quiet inbox") && line.contains("Re: Which runners"))),
        "the main conversation never hears it: {:?}",
        agents.told()
    );
}

/// Bob never picks the item up: it expires as any item does, Hotline's
/// notice of it is heard in the waiting job, and the job ends.
#[tokio::test]
async fn an_item_the_waiting_job_sent_expires_its_notice_is_heard_in_the_job_and_it_ends() {
    let (room, agents, _) = waiting_on_bob("job-once-expires", &["Which runners are red?"]).await;
    room.inbox_due(now_ms() + expire_after() + 1_000).await;
    until("the job hears Hotline's notice", || {
        agents
            .told()
            .iter()
            .any(|line| line.contains("Expired: Which runners are red?"))
    })
    .await;
    until("the job has ended", || once_closed(&room)).await;
    assert_eq!(turns_of(&room, ONCE), 2);
}

/// A run that sent several items waits for each: the first reply is heard
/// in a turn of the job, which still waits on the other, named on its
/// record; the job ends after the turn that hears the last.
#[tokio::test]
async fn a_run_that_sent_several_items_waits_for_each_reply_and_ends_after_the_last() {
    let (room, agents, ids) = waiting_on_bob(
        "job-once-several",
        &["Which runners are red?", "Which runners are slow?"],
    )
    .await;
    answer(&room, &ids[0], "linux-2 is red.").await;
    until("the job hears the first reply", || {
        agents
            .told()
            .iter()
            .any(|line| line.contains("Re: Which runners are red?"))
    })
    .await;
    until("its turn ended", || turns_of(&room, ONCE) == 2).await;
    until("the job is at rest", || room.job_at_rest(ONCE)).await;
    assert!(room.job_lives(ONCE), "it still waits on the other");
    assert_eq!(once_record(&room).awaiting, Some(vec!["bob".to_string()]));
    assert_eq!(notices_of(&room, ONCE).len(), 1, "it says it waits once");

    answer(&room, &ids[1], "mac-1 is slow.").await;
    until("the job has ended", || once_closed(&room)).await;
    assert_eq!(turns_of(&room, ONCE), 3);
}

/// Bob closes the item with no reply: the clock's next look wakes the job
/// to hear where it stands, and the job ends after that turn.
#[tokio::test]
async fn an_item_closed_with_no_reply_is_heard_at_the_clocks_look_and_the_job_ends() {
    let (room, agents, ids) = waiting_on_bob("job-once-noted", &["Which runners are red?"]).await;
    tool(
        &room,
        "bob",
        "update_inbox",
        json!({"items": [{"id": ids[0], "state": "closed", "outcome": "noted"}]}),
    )
    .await
    .unwrap();
    room.inbox_due(now_ms()).await;
    until("the job hears where its item stands", || {
        agents
            .told()
            .iter()
            .any(|line| line.contains("SENT · what moved") && line.contains("closed, noted"))
    })
    .await;
    until("the job has ended", || once_closed(&room)).await;
}

/// The desk restarts while the job waits: it still waits, and Bob's answer
/// on the desk that opened is heard in the job, which then ends.
#[tokio::test]
async fn a_job_waiting_when_the_desk_restarts_still_waits_and_hears_the_reply_in_the_job() {
    let (old, _, ids) = waiting_on_bob("job-once-restart", &["Which runners are red?"]).await;
    at_rest(&old, "bob").await;
    let log = old.log.clone();
    drop(old);
    let agents = Fake::new(replies());
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
    room.recover_exchanges().await;
    assert!(room.job_lives(ONCE), "the restart did not end it");
    assert_eq!(once_record(&room).awaiting, Some(vec!["bob".to_string()]));

    answer(&room, &ids[0], "linux-2 and mac-1 are red.").await;
    until("the job hears the reply", || {
        agents
            .told()
            .iter()
            .any(|line| line.contains("Re: Which runners are red?"))
    })
    .await;
    until("the job has ended", || once_closed(&room)).await;
}

/// The person's Stop on a waiting job ends it at once: its thread says so
/// and closes, and Bob's answer after that is heard in the main
/// conversation, as a reply to a run that is over is.
#[tokio::test]
async fn the_persons_stop_ends_a_waiting_job_at_once() {
    let (room, _, ids) = waiting_on_bob("job-once-stopped", &["Which runners are red?"]).await;
    room.cancel_job_turn(ONCE).unwrap();
    assert!(once_closed(&room));
    assert!(!room.job_lives(ONCE));
    assert_eq!(
        notices_of(&room, ONCE).last().map(String::as_str),
        Some("Stopped. That was the job's last run, so it has ended.")
    );
    answer(&room, &ids[0], "linux-2 is red.").await;
    let reply = items(&room, "ada")
        .into_iter()
        .find(|reply| reply.reply_to.as_deref() == Some(ids[0].as_str()))
        .unwrap();
    assert_eq!(reply.about, None);
    assert_eq!(room.inbox_heard_in(&reply), ThreadId::dm("ada"));
    assert_eq!(turns_of(&room, ONCE), 1);
}
