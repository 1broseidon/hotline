//! The funnel's own rules: what a line carries, and how a job fires on its
//! own thread rather than the DM's.

use super::*;
use crate::contract::{End, ScheduleKind, ThreadId};
use crate::session::Brief;

/// The index is written as the tape is, and a search finds the turn without
/// anything having asked for a rebuild.
#[tokio::test]
async fn what_is_appended_is_indexed() {
    let room = room("index", Fake::new(Scripted::new(spoken_turn())));
    room.start("ada").await.unwrap();
    room.prompt("ada", "what is here?", None, None)
        .await
        .unwrap();
    settled(&room, "ada", 5).await;

    let found = crate::store::search::search(room.log.root(), "ada", "here", None).unwrap();
    assert_eq!(
        found["hits"][0]["excerpt"], "what is here?",
        "the user's line was never indexed: {found}"
    );
}

fn attachment(name: &str, path: &str) -> Attachment {
    Attachment {
        created: None,
        kind: AttachmentKind::File,
        name: name.to_string(),
        path: path.to_string(),
        mime_type: None,
        size: None,
        width: None,
        height: None,
        origin: None,
    }
}

fn brief(
    kind: ScheduleKind,
    when: Option<i64>,
    every: Option<i64>,
    prompt: &str,
    quiet: bool,
) -> Brief {
    Brief::job(kind, when, every, prompt, quiet).unwrap()
}

/// Creating, silencing and cancelling a job go through the room, not around
/// it. A cancelled job that never ran ends without a post, and is listed as
/// ended until it is cancelled again.
#[tokio::test]
async fn a_job_is_created_silenced_and_cancelled_on_the_room() {
    let room = room("schedule-api", Fake::new(Scripted::new(Vec::new())));
    let when = now_ms() + 60_000;
    let job = room
        .schedule_create(
            "ada",
            brief(
                ScheduleKind::Schedule,
                Some(when),
                None,
                "check the crane",
                true,
            ),
            false,
        )
        .unwrap();
    assert_eq!(job.prompt, "check the crane");
    assert_eq!(job.quiet, Some(true));
    assert_eq!(job.when, Some(when));
    assert_eq!(room.schedule_list().len(), 1);

    room.schedule_set_quiet(&job.id, false).unwrap();
    assert_eq!(room.schedule_list()[0].quiet, None);

    room.schedule_cancel(&job.id).unwrap();
    assert_eq!(room.schedule_list(), [] as [ScheduledJob; 0]);
    let ended = crate::room::all_jobs(&room.log);
    assert_eq!(ended.len(), 1);
    assert!(ended[0].ended_at.is_some());
    assert!(
        tape(&room, "ada")
            .iter()
            .all(|event| event["kind"] != "agent"),
        "a job that never ran has nothing to say for itself"
    );

    room.schedule_cancel(&job.id).unwrap();
    assert_eq!(crate::room::all_jobs(&room.log), [] as [ScheduledJob; 0]);
}

/// The one schema refuses what a job cannot be: a subagent's trigger, an
/// end on a one-shot, an empty count.
#[tokio::test]
async fn a_job_is_a_brief_that_runs_later() {
    let room = room("schedule-brief", Fake::new(Scripted::new(Vec::new())));
    let refused = room
        .schedule_create(
            "ada",
            Brief::subagent("check now".to_string(), Vec::new()),
            false,
        )
        .unwrap_err();
    assert!(refused.contains("subagent runs now"), "{refused}");
    let mut once = brief(
        ScheduleKind::Schedule,
        Some(now_ms() + 60_000),
        None,
        "once",
        false,
    );
    once.ends.after = Some(2);
    assert!(room.schedule_create("ada", once, false).is_err());
    let mut never = brief(ScheduleKind::Loop, None, Some(60_000), "loop", false);
    never.ends.after = Some(0);
    assert!(room.schedule_create("ada", never, false).is_err());
}

/// A job that is already due, planted on the stream before the room opens so
/// the clock's first look fires it — the same path a missed tick takes after
/// Hotline was closed.
fn due_job(
    id: &str,
    kind: ScheduleKind,
    prompt: &str,
    quiet: bool,
    overdue_by: i64,
) -> ScheduledJob {
    let now = now_ms();
    ScheduledJob {
        id: id.to_string(),
        persona_id: "ada".to_string(),
        kind,
        when: (kind == ScheduleKind::Schedule).then_some(now - overdue_by),
        every: (kind == ScheduleKind::Loop).then_some(15_000),
        prompt: prompt.to_string(),
        quiet: quiet.then_some(true),
        reminder: None,
        goal: None,
        until: None,
        times: None,
        without: None,
        operator_created: false,
        next_at: now - overdue_by,
        created_at: now - overdue_by - 60_000,
        last_post: None,
        ended_at: None,
    }
}

fn room_due(name: &str, agents: Arc<Fake>, job: ScheduledJob) -> Arc<Room> {
    room_due_with_grant(name, agents, job, true)
}

fn room_due_with_grant(
    name: &str,
    agents: Arc<Fake>,
    job: ScheduledJob,
    background_work: bool,
) -> Arc<Room> {
    let log = scratch(name);
    let mut teammate = persona("ada");
    teammate.background_work = background_work;
    enrol(&log, &teammate);
    crate::room::append_schedule(&log, &job).unwrap();
    Room::with_agents(log, Arc::new(DeskKeys), agents)
}

/// What a job's thread holds, its link left out, once it holds `count`.
async fn job_thread(room: &Room, job_id: &str, count: usize) -> Vec<Value> {
    let events = || -> Vec<Value> {
        room.log
            .load(&StreamId::Job(job_id.to_string()))
            .into_iter()
            .filter(|event| event["kind"] != "link")
            .collect()
    };
    until(&format!("the job's thread reaches {count} events"), || {
        events().len() >= count
    })
    .await;
    events()
}

/// The job's thread as its link says it stands.
fn job_state(room: &Room, job_id: &str) -> Option<crate::thread::ThreadState> {
    crate::thread::ThreadStore::new(&room.log)
        .load(&ThreadId::job(job_id))
        .map(|thread| thread.state)
}

/// A loop fires on its own thread: the bare prompt stamped with the job, and
/// the agent's turn. The DM's tape holds none of it but the report, posted
/// as the teammate's message from the job; and the job is parked after.
#[tokio::test]
async fn a_loop_fires_on_its_own_thread_and_its_report_is_posted() {
    let agents = Fake::new(Scripted::new(saying("s", "nothing changed")));
    let prompts = agents.driver.prompts.clone();
    let job = due_job("job-loop", ScheduleKind::Loop, "check the crane", false, 1);
    let room = room_due("schedule-loop-fire", agents, job);

    let events = job_thread(&room, "job-loop", 3).await;
    assert_eq!(kinds(&events), ["user", "agent", "turn"]);
    assert_eq!(events[0]["text"], "check the crane");
    assert_eq!(events[0]["scheduled"]["jobId"], "job-loop");
    assert_eq!(events[0]["scheduled"]["kind"], "loop");
    let heard = heard(&prompts, 1).await;
    assert!(heard[0].ends_with("loop · check the crane"), "{heard:?}");

    let posted = settled(&room, "ada", 1).await;
    assert_eq!(kinds(&posted), ["agent"], "only the report reached the DM");
    assert_eq!(posted[0]["text"], "nothing changed");
    assert_eq!(posted[0]["scheduled"]["jobId"], "job-loop");
    assert_eq!(posted[0]["from"]["kind"], "job");
    assert_eq!(
        room.info("ada").state,
        SessionState::Idle,
        "the DM never woke"
    );

    until("the job is parked", || {
        job_state(&room, "job-loop") == Some(crate::thread::ThreadState::Parked)
    })
    .await;
    let living = crate::room::schedules(&room.log);
    assert_eq!(living.len(), 1);
    assert!(
        living[0].next_at > now_ms(),
        "a loop re-arms into the future, not onto the tick it just fired"
    );
    assert_eq!(
        living[0].last_post.as_ref().map(|post| post.text.as_str()),
        Some("nothing changed")
    );
}

/// A quiet job's words stay in its thread: a firing that finds nothing puts
/// nothing in the chat.
#[tokio::test]
async fn a_quiet_jobs_words_stay_in_its_thread() {
    let agents = Fake::new(Scripted::new(saying("quiet", "No change.")));
    let job = due_job(
        "job-quiet",
        ScheduleKind::Loop,
        "check the order page",
        true,
        1,
    );
    let room = room_due("schedule-quiet-fire", agents, job);

    let events = job_thread(&room, "job-quiet", 3).await;
    assert_eq!(kinds(&events), ["user", "agent", "turn"]);
    assert_eq!(events[0]["scheduled"]["quiet"], true);
    assert_eq!(events[1]["text"], "No change.");
    until("the job is parked", || {
        job_state(&room, "job-quiet") == Some(crate::thread::ThreadState::Parked)
    })
    .await;
    assert!(tape(&room, "ada").is_empty(), "{:?}", tape(&room, "ada"));
}

/// A one-shot fires once and ends; it stays on the list, ended, and its
/// thread is closed and kept.
#[tokio::test]
async fn a_one_shot_fires_once_and_ends() {
    let agents = Fake::new(Scripted::new(saying("s", "the word")));
    let job = due_job("job-once", ScheduleKind::Schedule, "say the word", false, 1);
    let room = room_due("schedule-once-fire", agents, job);

    let events = job_thread(&room, "job-once", 3).await;
    assert_eq!(events[0]["text"], "say the word");
    assert_eq!(events[0]["scheduled"]["kind"], "schedule");
    assert_eq!(crate::room::schedules(&room.log), [] as [ScheduledJob; 0]);
    until("the one-shot's thread is closed", || {
        matches!(
            job_state(&room, "job-once"),
            Some(crate::thread::ThreadState::Closed(End::Done))
        )
    })
    .await;
    let listed = crate::room::all_jobs(&room.log);
    assert_eq!(listed.len(), 1);
    assert!(listed[0].ended_at.is_some());
}

/// A plain reminder does no work: it is one post to the DM, stamped with its
/// job, and no agent is asked anything.
#[tokio::test]
async fn a_reminder_is_one_post_and_no_work() {
    let agents = Fake::new(Scripted::new(saying("s", "should not run")));
    let prompts = agents.driver.prompts.clone();
    let mut job = due_job("job-remind", ScheduleKind::Schedule, "Call Sam.", false, 1);
    job.reminder = Some(true);
    let room = room_due("schedule-reminder", agents, job);

    let posted = settled(&room, "ada", 1).await;
    assert_eq!(posted[0]["kind"], "agent");
    assert_eq!(posted[0]["text"], "Call Sam.");
    assert_eq!(posted[0]["scheduled"]["jobId"], "job-remind");
    assert!(posted[0].get("from").is_none(), "{posted:?}");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(lock(&prompts).is_empty(), "a reminder asked an agent");
    assert!(
        room.log
            .load(&StreamId::Job("job-remind".to_string()))
            .is_empty(),
        "a reminder has no thread"
    );
}

/// Jobs missed while Hotline was closed fire once at startup, not once per
/// missed interval. A loop five intervals overdue is still one firing.
#[tokio::test]
async fn a_missed_job_fires_once_at_startup() {
    let agents = Fake::new(Scripted::new(saying("s", "caught up")));
    let job = due_job(
        "job-missed",
        ScheduleKind::Loop,
        "sweep the harbour",
        false,
        5 * 15_000,
    );
    let room = room_due("schedule-missed-fire", agents, job);

    job_thread(&room, "job-missed", 3).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let firings = room
        .log
        .load(&StreamId::Job("job-missed".to_string()))
        .into_iter()
        .filter(|event| event["kind"] == "user")
        .count();
    assert_eq!(
        firings, 1,
        "the clock did not catch up the intervals it slept through"
    );
}

/// An agent-created due job stays on the room stream while its standing grant
/// is off. Regranting it wakes the clock and fires the same job once.
#[tokio::test]
async fn a_due_agent_job_waits_for_a_grant_then_fires_once() {
    let agents = Fake::new(Scripted::new(saying("s", "the word")));
    let prompts = agents.driver.prompts.clone();
    let job = due_job(
        "job-paused",
        ScheduleKind::Schedule,
        "say the word",
        false,
        1,
    );
    let room = room_due_with_grant("schedule-paused", agents, job, false);

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(lock(&prompts).is_empty(), "a paused job was dispatched");
    assert_eq!(crate::room::schedules(&room.log).len(), 1);

    let mut teammate = persona("ada");
    teammate.background_work = true;
    enrol(&room.log, &teammate);
    room.reattach("ada").await.unwrap();

    let events = job_thread(&room, "job-paused", 3).await;
    assert_eq!(events[0]["text"], "say the word");
    assert_eq!(events[0]["scheduled"]["operatorCreated"], false);
    assert_eq!(heard(&prompts, 1).await.len(), 1);
    assert_eq!(crate::room::schedules(&room.log), [] as [ScheduledJob; 0]);
}

/// A job the operator entered from the desk remains runnable when the
/// teammate's autonomous background grant is off.
#[tokio::test]
async fn a_due_operator_job_runs_without_a_background_grant() {
    let agents = Fake::new(Scripted::new(saying("s", "the word")));
    let mut job = due_job(
        "job-operator",
        ScheduleKind::Schedule,
        "say the word",
        false,
        1,
    );
    job.operator_created = true;
    let room = room_due_with_grant("schedule-operator", agents, job, false);

    let events = job_thread(&room, "job-operator", 3).await;
    assert_eq!(events[0]["text"], "say the word");
    assert_eq!(events[0]["scheduled"]["operatorCreated"], true);
    assert_eq!(crate::room::schedules(&room.log), [] as [ScheduledJob; 0]);
}

/// The person talks to a job's thread: a turn on it like a firing's, and
/// nothing in the DM.
#[tokio::test]
async fn the_person_talks_to_a_jobs_thread() {
    let agents = Fake::new(Scripted::turns(vec![
        saying("fired", "Order still processing."),
        saying("asked", "Will skip the next check."),
    ]));
    let prompts = agents.driver.prompts.clone();
    let job = due_job("job-talk", ScheduleKind::Loop, "check the order", true, 1);
    let room = room_due("schedule-talk", agents, job);
    job_thread(&room, "job-talk", 3).await;
    until("the job is parked", || {
        job_state(&room, "job-talk") == Some(crate::thread::ThreadState::Parked)
    })
    .await;

    room.prompt_job("job-talk", "skip the next one", None)
        .await
        .unwrap();
    let events = job_thread(&room, "job-talk", 6).await;
    assert_eq!(events[3]["text"], "skip the next one");
    assert_eq!(events[4]["text"], "Will skip the next check.");
    let heard = heard(&prompts, 2).await;
    assert!(
        heard[1].contains("Order still processing.") && heard[1].ends_with("skip the next one"),
        "a parked job's fresh agent is told where it stood: {heard:?}"
    );
    assert_eq!(
        tape(&room, "ada"),
        [] as [Value; 0],
        "nothing reached the DM"
    );
}

/// A job asks the person on its own thread: the `request_human` card is on
/// the job's stream, not the DM's tape, and the person's answer comes back
/// to the job as a turn on that thread, read once.
#[tokio::test]
async fn a_jobs_card_is_raised_and_answered_on_its_own_thread() {
    let agents = Fake::new(Scripted::new(saying("answered", "Using the blue crane.")));
    let prompts = agents.driver.prompts.clone();
    let mut job = due_job("job-ask", ScheduleKind::Loop, "check the cranes", false, 0);
    job.next_at = now_ms() + 3_600_000;
    let room = room_due("schedule-ask", agents, job);
    let tools = TeammateTools::new(&room, "ada")
        .with_capability(room.capability_lease("ada"))
        .for_job("job-ask");

    let asked = tools
        .call("request_human", &json!({"reason": "Which crane is mine?"}))
        .await
        .unwrap();
    assert!(asked.starts_with("Asked."), "{asked}");
    let card = job_thread(&room, "job-ask", 1).await;
    assert_eq!(kinds(&card), ["human_action"]);
    assert_eq!(card[0]["status"], "pending");
    assert_eq!(tape(&room, "ada"), [] as [Value; 0], "not on the DM");
    let action = card[0]["actionId"].as_str().unwrap().to_string();

    room.answer_human(
        "ada",
        &action,
        HumanAnswer::Done,
        Some("the blue one".to_string()),
    )
    .unwrap();
    let heard = heard(&prompts, 1).await;
    assert!(heard[0].contains("the blue one"), "{heard:?}");
    let events = job_thread(&room, "job-ask", 4).await;
    assert_eq!(
        kinds(&events),
        ["human_action", "delivery", "agent", "turn"]
    );
    let delivery = events
        .iter()
        .rfind(|event| event["kind"] == "delivery")
        .unwrap();
    assert_eq!(
        delivery["from"],
        json!({"thread": "job-ask", "kind": "job", "request": action})
    );
    until("the delivery is read", || {
        job_thread_now(&room, "job-ask")
            .iter()
            .rfind(|event| event["kind"] == "delivery")
            .is_some_and(|event| event["receipt"] == "read")
    })
    .await;
    assert!(
        job_thread_now(&room, "job-ask")
            .iter()
            .any(|event| event["kind"] == "human_action" && event["status"] == "done")
    );
    assert_eq!(
        tape(&room, "ada"),
        [] as [Value; 0],
        "nothing reached the DM"
    );
}

/// A job is a clone of the home agent: it has the teammate's computer, as a
/// work thread does, unless the teammate made it without.
#[cfg(unix)]
#[tokio::test]
async fn a_job_has_the_computer_unless_it_was_made_without_it() {
    let ComputerRoom { room, agents, .. } =
        computer_room("job-computer", Some("0.10.3"), TWO_RELEASES, true).await;
    let computer = |view: &Persona| view.computer.as_ref().is_some_and(|c| c.enabled);
    let mut built = Vec::new();
    for (id, without) in [
        ("job-whole", None),
        ("job-narrow", Some(vec![crate::contract::Ability::Computer])),
    ] {
        let mut job = due_job(id, ScheduleKind::Loop, "look at the screen", false, 0);
        job.next_at = now_ms() + 3_600_000;
        job.operator_created = true;
        job.without = without;
        crate::room::append_schedule(&room.log, &job).unwrap();
        let before = lock(&agents.views).len();
        room.fire_job(&job).unwrap();
        until("the job's agent is built", || {
            lock(&agents.views).len() > before
        })
        .await;
        built.push(lock(&agents.views).last().cloned().unwrap());
    }
    assert!(computer(&built[0]), "a job has its teammate's computer");
    assert!(!computer(&built[1]), "unless it was made without");
}

/// What a job's thread holds now, its link left out.
fn job_thread_now(room: &Room, job_id: &str) -> Vec<Value> {
    room.log
        .load(&StreamId::Job(job_id.to_string()))
        .into_iter()
        .filter(|event| event["kind"] != "link")
        .collect()
}

/// A firing can wait behind a turn the person started in the job's thread.
/// The check as its turn begins still reads the live grant, and drops an
/// agent-created firing that lost permission while it waited.
#[tokio::test]
async fn a_queued_firing_is_dropped_when_background_work_is_revoked() {
    let gate = Arc::new(Semaphore::new(0));
    let mut driver = Scripted::turns(vec![
        saying("asked", "Looking."),
        saying("fired", "No change."),
    ]);
    driver.gate = Some(gate.clone());
    let agents = Fake::new(driver);
    let prompts = agents.driver.prompts.clone();
    let mut job = due_job(
        "job-revoked",
        ScheduleKind::Loop,
        "check the crane",
        true,
        0,
    );
    job.next_at = now_ms() + 3_600_000;
    let room = room_due("schedule-queue-revoked", agents, job.clone());

    room.prompt_job("job-revoked", "how is it going?", None)
        .await
        .unwrap();
    heard(&prompts, 1).await;
    room.fire_job(&job).unwrap();
    job_thread(&room, "job-revoked", 2).await;

    let mut teammate = persona("ada");
    teammate.background_work = false;
    enrol(&room.log, &teammate);
    // One permit lets out one update; the firing's turn would need its own.
    gate.add_permits(10);
    until("the person's turn ended", || {
        room.log
            .load(&StreamId::Job("job-revoked".to_string()))
            .iter()
            .any(|event| event["kind"] == "turn")
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(
        lock(&prompts).len(),
        1,
        "the firing never reached the agent"
    );
    let firings = room
        .log
        .load(&StreamId::Job("job-revoked".to_string()))
        .into_iter()
        .filter(|event| event["kind"] == "user" && event.get("scheduled").is_some())
        .count();
    assert_eq!(
        firings, 1,
        "the firing remains a durable fact though no turn ran"
    );
}

/// What a message answers is on the line it is written as, and a mark that has
/// gone stale is claimed by nothing.
#[tokio::test]
async fn a_reply_is_stamped_on_its_own_line_and_only_while_the_mark_is_fresh() {
    let room = room("reply", Fake::new(Scripted::new(Vec::new())));
    room.start("ada").await.unwrap();

    room.prompt("ada", "this one", Some("a1".to_string()), None)
        .await
        .unwrap();
    let events = settled(&room, "ada", 1).await;
    assert_eq!(events[0]["replyTo"], "a1");

    let session = room.session("ada").unwrap();
    *lock(&session.pending_reply) = Some(Mark {
        value: "a1".to_string(),
        until: now_ms() - 1,
    });
    room.prompt("ada", "and this one", None, None)
        .await
        .unwrap();
    let events = settled(&room, "ada", 2).await;
    assert_eq!(
        events[1].get("replyTo"),
        None,
        "an expired mark is not claimed by a later, unrelated message"
    );
}

/// The record keeps what was attached, and the driver is handed the same
/// files beside the words rather than inside them — because how an attachment
/// reaches an agent is the driver's answer and not the room's.
#[tokio::test]
async fn attachments_land_on_the_line_and_beside_what_the_driver_hears() {
    let agents = Fake::new(Scripted::new(Vec::new()));
    let prompts = agents.driver.prompts.clone();
    let handed = agents.driver.attachments.clone();
    let room = room("attachments", agents);
    room.start("ada").await.unwrap();

    room.prompt(
        "ada",
        "read these",
        None,
        Some(vec![
            attachment("note.txt", "/tmp/note.txt"),
            attachment("shot.png", "/tmp/shot.png"),
        ]),
    )
    .await
    .unwrap();

    let events = settled(&room, "ada", 1).await;
    assert_eq!(events[0]["attachments"][0]["name"], "note.txt");
    assert_eq!(events[0]["attachments"][1]["path"], "/tmp/shot.png");
    assert_eq!(heard(&prompts, 1).await, ["read these"]);
    let handed = lock(&handed).clone();
    assert_eq!(
        handed[0]
            .iter()
            .map(|attachment| attachment.path.as_str())
            .collect::<Vec<_>>(),
        ["/tmp/note.txt", "/tmp/shot.png"]
    );
}

/// Hotline's own words to a running teammate: the driver hears them, and the
/// conversation has no line saying anybody spoke.
#[tokio::test]
async fn a_nudge_reaches_the_driver_and_never_the_tape() {
    let agents = Fake::new(Scripted::new(vec![Update::Turn {
        stop_reason: "end_turn".to_string(),
        usage: None,
    }]));
    let prompts = agents.driver.prompts.clone();
    let room = room("nudge", agents);
    room.start("ada").await.unwrap();

    room.nudge("ada", "while you were away the user asked you to hurry")
        .unwrap();
    assert_eq!(
        heard(&prompts, 1).await,
        ["while you were away the user asked you to hurry"]
    );

    let events = settled(&room, "ada", 1).await;
    assert_eq!(
        kinds(&events),
        ["turn"],
        "the turn it ran is on the tape; the words that started it are not"
    );
}
