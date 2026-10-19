//! The funnel's own rules: what a line carries, whose voice is held, and how
//! a schedule fires.

use super::*;

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

/// A schedule's firing, as the scheduler will hand one over.
fn firing(kind: ScheduleKind, quiet: bool) -> ScheduledRun {
    ScheduledRun {
        job_id: "job-1".to_string(),
        kind,
        name: "Apple order check".to_string(),
        operator_created: false,
        quiet: quiet.then_some(true),
    }
}

fn attachment(name: &str, path: &str) -> Attachment {
    Attachment {
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

/// The whole point of a quiet run: the agent said something and the chat has
/// nothing in it, and the teammate speaks normally the moment it is over.
#[tokio::test]
async fn a_quiet_runs_words_are_thinking_and_the_next_plain_prompt_speaks() {
    let agents = Fake::new(Scripted::turns(vec![
        saying("quiet", "No change — staying silent per protocol."),
        saying("loud", "It moved."),
    ]));
    let prompts = agents.driver.prompts.clone();
    let room = room("quiet-run", agents);
    let mut deltas = room.subscribe_deltas();
    room.start("ada").await.unwrap();

    room.prompt_scheduled(
        "ada",
        "check the order page",
        firing(ScheduleKind::Loop, true),
    )
    .await
    .unwrap();
    let events = settled(&room, "ada", 3).await;
    assert_eq!(kinds(&events), ["user", "thought", "turn"]);
    assert_eq!(
        events[1]["id"], "m-quiet",
        "the words were demoted, not destroyed"
    );
    assert_eq!(
        events[1]["text"],
        "No change — staying silent per protocol."
    );

    // The tape keeps the bare prompt and the stamp naming the job; the agent
    // heard the framing that says a schedule woke it.
    assert_eq!(events[0]["text"], "check the order page");
    assert_eq!(events[0]["scheduled"]["jobId"], "job-1");
    assert_eq!(events[0]["scheduled"]["quiet"], true);
    assert_eq!(*lock(&prompts), ["loop · check the order page"]);

    room.prompt("ada", "and now?", None, None).await.unwrap();
    let events = settled(&room, "ada", 6).await;
    assert_eq!(
        kinds(&events),
        ["user", "thought", "turn", "user", "agent", "turn"],
        "the run's own turn boundary closed the window"
    );
    assert_eq!(events[4]["text"], "It moved.");

    // The live stream was muted exactly as long as the tape was: an indicator
    // that types and then produces nothing reads as a bug, not as silence.
    let mut streamed = Vec::new();
    while let Ok(delta) = deltas.try_recv() {
        streamed.push(delta);
    }
    assert_eq!(
        streamed,
        [
            StreamDelta::ThreadDelta {
                thread: ThreadId::dm("ada"),
                message_id: "m-quiet".to_string(),
                kind: DeltaKind::Thought,
                text: "No change — staying silent per protocol.".to_string(),
            },
            StreamDelta::ThreadDelta {
                thread: ThreadId::dm("ada"),
                message_id: "m-loud".to_string(),
                kind: DeltaKind::Text,
                text: "It moved.".to_string(),
            },
        ]
    );
}

/// A person who types during a quiet run is owed an answer they can read; the
/// schedule's silence was never about them.
#[tokio::test]
async fn a_person_typing_during_a_quiet_run_gets_a_bubble_back() {
    let gate = Arc::new(Semaphore::new(0));
    let mut driver = Scripted::turns(vec![
        saying("quiet", "the scheduled run's own words"),
        saying("loud", "It moved."),
    ]);
    driver.gate = Some(gate.clone());
    let room = room("quiet-interrupted", Fake::new(driver));
    room.start("ada").await.unwrap();

    room.prompt_scheduled(
        "ada",
        "check the order page",
        firing(ScheduleKind::Loop, true),
    )
    .await
    .unwrap();
    let events = settled(&room, "ada", 1).await;
    assert_eq!(events[0]["scheduled"]["quiet"], true);

    // The window is open and the run's turn is still gated when a person types.
    room.prompt("ada", "wait, what did you find?", None, None)
        .await
        .unwrap();
    settled(&room, "ada", 2).await;

    // One permit per update: the delta, the message, the turn.
    gate.add_permits(3);
    let events = settled(&room, "ada", 4).await;
    assert_eq!(
        kinds(&events),
        ["user", "user", "agent", "turn"],
        "the human closed the window, so the run's words are a bubble"
    );
}

/// A quiet run that finds something asks to be heard, once, and is: once its
/// silent turn is over, the teammate is prompted with the finding in the open
/// and answers the person as any reply is answered. Only the quiet turn was
/// armed.
#[tokio::test]
async fn a_quiet_run_that_asks_has_its_teammate_tell_the_person() {
    let gate = Arc::new(Semaphore::new(0));
    let mut driver = Scripted::turns(vec![
        saying("quiet", "Checked the page; it moved."),
        saying("loud", "Your Apple order shipped this morning."),
    ]);
    driver.gate = Some(gate.clone());
    let armed = driver.escalations.clone();
    let prompts = driver.prompts.clone();
    let room = room("quiet-escalates", Fake::new(driver));
    room.start("ada").await.unwrap();

    room.prompt_scheduled(
        "ada",
        "check the order page",
        firing(ScheduleKind::Loop, true),
    )
    .await
    .unwrap();
    heard(&prompts, 1).await;
    let escalation = lock(&armed)[0].clone().expect("a quiet run is armed");
    escalation.escalate("The order shipped.").unwrap();
    let refused = escalation.escalate("And another thing.").unwrap_err();
    assert!(refused.contains("one per run"), "{refused}");

    gate.add_permits(6);
    let events = settled(&room, "ada", 6).await;
    assert_eq!(
        kinds(&events),
        ["user", "thought", "turn", "user", "agent", "turn"],
        "the run stayed thinking; the teammate's answer is a reply"
    );
    assert_eq!(events[1]["text"], "Checked the page; it moved.");
    assert_eq!(events[3]["text"], "The order shipped.");
    assert_eq!(events[3]["scheduled"]["jobId"], "job-1");
    assert_eq!(events[3]["scheduled"].get("quiet"), None);
    assert_eq!(events[4]["text"], "Your Apple order shipped this morning.");

    let prompts = heard(&prompts, 2).await;
    assert!(
        prompts[1].contains("\"Apple order check\" found something")
            && prompts[1].contains("The order shipped."),
        "{}",
        prompts[1]
    );
    let armed = lock(&armed);
    assert_eq!(armed.len(), 2);
    assert!(armed[1].is_none(), "the answer is not a quiet run");
}

/// A quiet run that does not ask is not followed by anything.
#[tokio::test]
async fn a_quiet_run_that_does_not_ask_stays_silent() {
    let driver = Scripted::new(saying("quiet", "No change."));
    let prompts = driver.prompts.clone();
    let room = room("quiet-no-escalation", Fake::new(driver));
    room.start("ada").await.unwrap();

    room.prompt_scheduled(
        "ada",
        "check the order page",
        firing(ScheduleKind::Loop, true),
    )
    .await
    .unwrap();
    let events = settled(&room, "ada", 3).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        kinds(&settled(&room, "ada", 3).await),
        ["user", "thought", "turn"]
    );
    assert_eq!(events[1]["text"], "No change.");
    assert_eq!(lock(&prompts).len(), 1);
}

/// Loud runs already speak, so they have nothing to ask for.
#[tokio::test]
async fn a_loud_schedule_is_not_armed_to_escalate() {
    let driver = Scripted::new(saying("s", "Standup is at ten."));
    let armed = driver.escalations.clone();
    let room = room("scheduled-loud-unarmed", Fake::new(driver));
    room.start("ada").await.unwrap();

    room.prompt_scheduled(
        "ada",
        "post the standup",
        firing(ScheduleKind::Schedule, false),
    )
    .await
    .unwrap();
    settled(&room, "ada", 3).await;
    let armed = lock(&armed);
    assert_eq!(armed.len(), 1);
    assert!(armed[0].is_none());
}

/// A schedule that is not quiet is stamped and framed all the same; only the
/// voice is left alone.
#[tokio::test]
async fn a_loud_schedule_is_stamped_and_framed_and_keeps_its_voice() {
    let agents = Fake::new(Scripted::new(saying("s", "Standup is at ten.")));
    let prompts = agents.driver.prompts.clone();
    let room = room("scheduled-loud", agents);
    room.start("ada").await.unwrap();

    room.prompt_scheduled(
        "ada",
        "post the standup",
        firing(ScheduleKind::Schedule, false),
    )
    .await
    .unwrap();
    let events = settled(&room, "ada", 3).await;

    assert_eq!(kinds(&events), ["user", "agent", "turn"]);
    assert_eq!(events[0]["text"], "post the standup");
    assert_eq!(events[0]["scheduled"]["name"], "Apple order check");
    assert_eq!(events[0]["scheduled"].get("quiet"), None);
    assert_eq!(*lock(&prompts), ["scheduled · post the standup"]);
}

/// Creating, silencing and cancelling a job go through the room, not around it.
#[tokio::test]
async fn a_job_is_created_silenced_and_cancelled_on_the_room() {
    let room = room("schedule-api", Fake::new(Scripted::new(Vec::new())));
    let when = now_ms() + 60_000;
    let job = room
        .schedule_create(
            "ada",
            ScheduleKind::Schedule,
            Some(when),
            None,
            "check the crane",
            true,
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
        operator_created: false,
        next_at: now - overdue_by,
        created_at: now - overdue_by - 60_000,
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

/// A loop that fires writes the bare prompt on the tape and the framed line
/// to the driver, and a fire for a teammate that is not running starts it.
#[tokio::test]
async fn a_loop_job_fires_with_the_bare_prompt_on_the_tape_and_the_framed_text_on_the_wire() {
    let agents = Fake::new(Scripted::new(saying("s", "nothing changed")));
    let prompts = agents.driver.prompts.clone();
    let job = due_job("job-loop", ScheduleKind::Loop, "check the crane", false, 1);
    let room = room_due("schedule-loop-fire", agents, job);

    let events = settled(&room, "ada", 3).await;
    assert_eq!(kinds(&events), ["user", "agent", "turn"]);
    assert_eq!(events[0]["text"], "check the crane");
    assert_eq!(events[0]["scheduled"]["jobId"], "job-loop");
    assert_eq!(events[0]["scheduled"]["kind"], "loop");
    assert_eq!(events[0]["scheduled"].get("quiet"), None);
    assert_eq!(*lock(&prompts), ["loop · check the crane"]);

    let living = crate::room::schedules(&room.log);
    assert_eq!(living.len(), 1);
    assert_eq!(living[0].id, "job-loop");
    assert!(
        living[0].next_at > now_ms(),
        "a loop re-arms into the future, not onto the tick it just fired"
    );
}

/// The whole point of a quiet job, through the clock rather than a direct
/// prompt_scheduled: the agent's words land as thinking.
#[tokio::test]
async fn a_quiet_jobs_reply_lands_as_a_thought() {
    let agents = Fake::new(Scripted::new(saying(
        "quiet",
        "No change — staying silent per protocol.",
    )));
    let job = due_job(
        "job-quiet",
        ScheduleKind::Loop,
        "check the order page",
        true,
        1,
    );
    let room = room_due("schedule-quiet-fire", agents, job);

    let events = settled(&room, "ada", 3).await;
    assert_eq!(kinds(&events), ["user", "thought", "turn"]);
    assert_eq!(events[0]["scheduled"]["quiet"], true);
    assert_eq!(
        events[1]["text"],
        "No change — staying silent per protocol."
    );
}

/// A one-shot fires once and tombstones itself.
#[tokio::test]
async fn a_one_shot_fires_once_and_is_gone() {
    let agents = Fake::new(Scripted::new(saying("s", "the word")));
    let job = due_job("job-once", ScheduleKind::Schedule, "say the word", false, 1);
    let room = room_due("schedule-once-fire", agents, job);

    let events = settled(&room, "ada", 3).await;
    assert_eq!(events[0]["text"], "say the word");
    assert_eq!(events[0]["scheduled"]["kind"], "schedule");
    assert!(
        crate::room::schedules(&room.log)
            .iter()
            .all(|job| job.id != "job-once")
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

    let events = settled(&room, "ada", 3).await;
    let users = events
        .iter()
        .filter(|event| event["kind"] == "user")
        .count();
    assert_eq!(
        users, 1,
        "a missed loop is one firing, not five: {events:?}"
    );

    tokio::time::sleep(Duration::from_millis(200)).await;
    let later = tape(&room, "ada")
        .into_iter()
        .filter(|event| event["kind"] == "user")
        .count();
    assert_eq!(
        later, 1,
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

    let events = settled(&room, "ada", 3).await;
    assert_eq!(events[0]["text"], "say the word");
    assert_eq!(events[0]["scheduled"]["operatorCreated"], false);
    assert_eq!(*lock(&prompts), ["scheduled · say the word"]);
    assert_eq!(crate::room::schedules(&room.log), [] as [ScheduledJob; 0]);
}

/// A job the operator entered from the desk remains runnable when the
/// teammate's autonomous background grant is off.
#[tokio::test]
async fn a_due_operator_job_runs_without_a_background_grant() {
    let agents = Fake::new(Scripted::new(saying("s", "the word")));
    let prompts = agents.driver.prompts.clone();
    let mut job = due_job(
        "job-operator",
        ScheduleKind::Schedule,
        "say the word",
        false,
        1,
    );
    job.operator_created = true;
    let room = room_due_with_grant("schedule-operator", agents, job, false);

    let events = settled(&room, "ada", 3).await;
    assert_eq!(events[0]["text"], "say the word");
    assert_eq!(events[0]["scheduled"]["operatorCreated"], true);
    assert_eq!(*lock(&prompts), ["scheduled · say the word"]);
    assert_eq!(crate::room::schedules(&room.log), [] as [ScheduledJob; 0]);
}

/// A scheduled line can be queued behind a turn while its one-shot job is
/// already gone from the room stream. The final dispatch check still reads
/// the live grant and drops an agent-created line that lost permission.
#[tokio::test]
async fn a_queued_scheduled_line_is_dropped_when_background_work_is_revoked() {
    let gate = Arc::new(Semaphore::new(0));
    let mut driver = Scripted::turns(vec![
        vec![Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        }],
        vec![Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        }],
    ]);
    driver.gate = Some(gate.clone());
    let agents = Fake::new(driver);
    let prompts = agents.driver.prompts.clone();
    let room = room("scheduled-queue-revoked", agents);
    room.start("ada").await.unwrap();

    room.prompt("ada", "first", None, None).await.unwrap();
    until_state(&room, "ada", SessionState::Thinking).await;
    room.prompt_scheduled(
        "ada",
        "old scheduled work",
        firing(ScheduleKind::Schedule, false),
    )
    .await
    .unwrap();
    let queued = settled(&room, "ada", 2).await;
    assert_eq!(queued[1]["scheduled"]["operatorCreated"], false);

    // This mirrors the live record changing before the old turn gives the
    // queue its next dispatch opportunity. Policy update callers also revoke
    // the session, which clears the queue earlier; this check closes the
    // remaining race where a queued one-shot survives its room tombstone.
    let mut teammate = persona("ada");
    teammate.background_work = false;
    enrol(&room.log, &teammate);
    gate.add_permits(1);
    until_state(&room, "ada", SessionState::Ready).await;

    assert_eq!(*lock(&prompts), ["first"]);
    assert_eq!(
        tape(&room, "ada")
            .into_iter()
            .filter(|event| event["kind"] == "user")
            .count(),
        2,
        "the queued line remains a durable user fact even though no turn ran"
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
