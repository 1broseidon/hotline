//! Subagent runs: a fresh agent for the teammate on a stream of its own, one
//! line on the tape, and the report handed back. See [`super::runner`].

use super::*;
use crate::contract::SubagentStatus;
use crate::session::runner::{RunEnd, RunSpec};
use tokio_util::sync::CancellationToken;

fn spec(room: &Room, run_id: &str, task: &str) -> RunSpec {
    RunSpec {
        persona_id: "ada".to_string(),
        run_id: run_id.to_string(),
        title: "Check the crane".to_string(),
        brief: crate::session::Brief::subagent(task.to_string(), Vec::new()),
        capability: room.capability_lease("ada"),
    }
}

fn run_stream(room: &Room, run_id: &str) -> Vec<Value> {
    room.log
        .load(&StreamId::Run(run_id.to_string()))
        .into_iter()
        .map(crate::thread::Link::wire)
        .collect()
}

fn worked() -> Vec<Update> {
    vec![
        Update::Message {
            kind: MessageKind::Agent,
            id: "m1".to_string(),
            text: "Looking at the log.".to_string(),
        },
        Update::ToolCall {
            call_id: "c1".to_string(),
            title: "read crane.log".to_string(),
            kind: "read".to_string(),
        },
        Update::ToolResult {
            call_id: "c1".to_string(),
            ok: true,
            output: "winch: jammed".to_string(),
            images: Vec::new(),
        },
        Update::Message {
            kind: MessageKind::Agent,
            id: "m2".to_string(),
            text: "The winch is jammed.".to_string(),
        },
        Update::Turn {
            stop_reason: "end_turn".to_string(),
            usage: None,
        },
    ]
}

/// A run held before its first update, which a cancel ends.
fn held() -> (Scripted, Arc<Semaphore>) {
    let gate = Arc::new(Semaphore::new(0));
    let mut driver = Scripted::new(worked());
    driver.gate = Some(gate.clone());
    driver.on_cancel = vec![Update::Turn {
        stop_reason: "aborted".to_string(),
        usage: None,
    }];
    (driver, gate)
}

async fn prompted(agents: &Fake) {
    for _ in 0..500 {
        if !lock(&agents.driver.prompts).is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    panic!("the run never prompted its agent");
}

#[tokio::test]
async fn a_run_works_on_its_own_stream_and_hands_back_its_last_words() {
    let agents = Fake::new(Scripted::new(worked()));
    let room = room("run-report", agents.clone());
    let outcome = room
        .run(
            spec(&room, "r1", "Find out why the crane stopped."),
            CancellationToken::new(),
        )
        .await;
    assert_eq!(outcome.end, RunEnd::Done);
    assert_eq!(outcome.report, "The winch is jammed.");
    assert!(
        room.subagents("ada").is_empty(),
        "a finished run leaves the row"
    );

    // The run's stream opens on its own line, then the task, and the
    // report is what it said once the work was done, not its opening.
    let stream = run_stream(&room, "r1");
    assert_eq!(
        kinds(&stream),
        ["subagent", "user", "agent", "tool", "agent", "turn"]
    );
    assert_eq!(stream[0]["status"], "done");
    assert_eq!(stream[1]["text"], "Find out why the crane stopped.");
    assert_eq!(stream[2]["text"], "Looking at the log.");
    assert_eq!(stream[3]["status"], "completed");

    // The teammate's tape holds one line for the whole run, rewritten
    // in place, and none of the run's words.
    let tape = tape(&room, "ada");
    assert_eq!(kinds(&tape), ["subagent"]);
    assert_eq!(tape[0]["id"], "link:run:r1");
    assert_eq!(tape[0]["runId"], "r1");
    assert_eq!(tape[0]["title"], "Check the crane");
    assert_eq!(tape[0]["status"], "done");
    assert!(tape[0]["elapsedMs"].as_i64().is_some());
    let stored = room.log.load(&StreamId::Tape("ada".to_string()));
    assert_eq!(stored[0]["kind"], "link");
    assert_eq!(stored[0]["threadKind"], "run");
    assert_eq!(stored[0]["state"], "closed");
    assert_eq!(stored[0]["end"], "done");

    // The teammate itself, told it is a run, none of the conversation, and
    // the task as its one message: a clone with a focus.
    let preamble = lock(&agents.preambles).last().cloned().unwrap();
    assert!(preamble.starts_with("You are Ada."), "{preamble}");
    assert!(
        preamble.contains("Keep the harbour running."),
        "a clone has its teammate's goal"
    );
    assert!(
        preamble.contains("This is a subagent run of Ada's"),
        "{preamble}"
    );
    assert_eq!(lock(&agents.seeds).last().unwrap().as_slice(), []);
    assert_eq!(
        lock(&agents.driver.prompts).last().unwrap(),
        "Your main conversation handed you this task:\n\nFind out why the crane stopped."
    );
}

#[tokio::test]
async fn a_run_is_on_its_teammates_live_model_with_no_session_to_reopen() {
    let agents = Fake::new(Scripted::new(worked()));
    let log = scratch("run-view");
    let mut ada = persona("ada");
    ada.model_id = Some("anthropic/older".to_string());
    ada.session_checkpoints = vec![SessionCheckpoint {
        backend_id: "hotline".to_string(),
        session_id: "s1".to_string(),
    }];
    enrol(&log, &ada);
    let room = Room::with_agents_and_computers(
        log,
        Arc::new(DeskKeys),
        agents.clone(),
        crate::computer::Computer::with_path(std::env::temp_dir().join("no-runtime")),
    );
    room.start("ada").await.unwrap();
    let tools = lock(&agents.tools).last().cloned().unwrap();
    assert!(
        tools.offers_subagents(),
        "the teammate's own session hands work off"
    );

    let outcome = room
        .run(spec(&room, "r2", "Anything."), CancellationToken::new())
        .await;
    assert_eq!(outcome.end, RunEnd::Done);
    let view = lock(&agents.views).last().cloned().unwrap();
    assert_eq!(
        view.model_id.as_deref(),
        Some("anthropic/claude"),
        "the model the session is on now, not the one on the record"
    );
    assert_eq!(view.session_checkpoints, [] as [SessionCheckpoint; 0]);
    let tools = lock(&agents.tools).last().cloned().unwrap();
    assert!(tools.in_run());
    assert!(!tools.offers_subagents(), "a run starts no runs of its own");
    assert!(tools.delegate().is_none());
}

#[tokio::test]
async fn a_cancelled_run_stops_its_agent_and_its_line_says_so() {
    let (driver, gate) = held();
    let agents = Fake::new(driver);
    let room = room("run-cancel", agents.clone());
    let cancel = CancellationToken::new();
    let running = tokio::spawn({
        let room = room.clone();
        let cancel = cancel.clone();
        async move { room.run(spec(&room, "r3", "Wait."), cancel).await }
    });
    gate.add_permits(2);
    for _ in 0..500 {
        if kinds(&run_stream(&room, "r3")).contains(&"tool".to_string()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert_eq!(tape(&room, "ada")[0]["status"], "running");
    // The roster row lists the run while it runs, wherever its line has
    // scrolled to on the tape.
    let listed = room.subagents("ada");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].run_id, "r3");
    assert_eq!(listed[0].title, "Check the crane");

    cancel.cancel();
    let outcome = running.await.unwrap();
    assert_eq!(outcome.end, RunEnd::Cancelled);
    assert!(
        room.subagents("ada").is_empty(),
        "a stopped run leaves the row"
    );
    assert!(outcome.report.contains("stopped"), "{}", outcome.report);
    assert!(agents.cancel_count() >= 1, "the agent was told to stop");
    let stream = run_stream(&room, "r3");
    assert_eq!(
        kinds(&stream),
        ["subagent", "user", "agent", "tool", "turn"]
    );
    assert_eq!(stream[0]["status"], "cancelled");
    assert_eq!(
        stream[3]["status"], "failed",
        "a tool the run never heard back about did not complete"
    );
    assert_eq!(tape(&room, "ada")[0]["status"], "cancelled");
}

#[tokio::test]
async fn a_runs_tools_read_the_conversation_and_die_with_the_teammates_session() {
    let (driver, _gate) = held();
    let agents = Fake::new(driver);
    let room = room("run-lease", agents.clone());
    let cancel = CancellationToken::new();
    let running = tokio::spawn({
        let room = room.clone();
        let cancel = cancel.clone();
        async move { room.run(spec(&room, "r4", "Look around."), cancel).await }
    });
    prompted(&agents).await;
    let tools = lock(&agents.tools).last().cloned().unwrap();
    tools.call("list_chapters", &json!({})).await.unwrap();
    tools.call("list_teammates", &json!({})).await.unwrap();
    tools.call("list_schedules", &json!({})).await.unwrap();
    for refused in ["react", "request_human", "message_teammate", "schedule"] {
        assert_eq!(
            tools.call(refused, &json!({})).await.unwrap_err(),
            format!("A subagent has no tool called '{refused}'.")
        );
    }

    // Stopping the teammate stops everything it started.
    room.capability_epoch("ada").stop();
    assert!(
        tools
            .call("list_chapters", &json!({}))
            .await
            .unwrap_err()
            .contains("revoked")
    );
    cancel.cancel();
    assert_eq!(running.await.unwrap().end, RunEnd::Cancelled);
}

#[tokio::test]
async fn a_run_for_a_teammate_whose_authority_is_gone_never_starts() {
    let agents = Fake::new(Scripted::new(worked()));
    let room = room("run-revoked", agents.clone());
    let stale = spec(&room, "r5", "Anything.");
    room.capability_epoch("ada").stop();
    let outcome = room.run(stale, CancellationToken::new()).await;
    assert_eq!(outcome.end, RunEnd::Failed);
    assert!(outcome.report.contains("revoked"), "{}", outcome.report);
    assert!(lock(&agents.driver.prompts).is_empty());
    assert_eq!(
        kinds(&run_stream(&room, "r5")),
        ["subagent", "user", "notice"]
    );
    assert_eq!(tape(&room, "ada")[0]["status"], "failed");
}

/// A clone runs on its teammate's own harness, ACP included: the run is
/// built for the teammate as it is, under the run kind's tools.
#[tokio::test]
async fn a_run_is_a_clone_on_its_teammates_own_harness() {
    let agents = Fake::new(Scripted::new(worked()));
    let log = scratch("run-acp");
    let mut ada = persona("ada");
    ada.backend_id = "claude-code".to_string();
    enrol(&log, &ada);
    let room = Room::with_agents_and_computers(
        log,
        Arc::new(DeskKeys),
        agents.clone(),
        crate::computer::Computer::with_path(std::env::temp_dir().join("no-runtime")),
    );
    let outcome = room
        .run(spec(&room, "r6", "Anything."), CancellationToken::new())
        .await;
    assert_eq!(outcome.end, RunEnd::Done, "{}", outcome.report);
    assert_eq!(outcome.report, "The winch is jammed.");
    let view = lock(&agents.views).last().cloned().unwrap();
    assert_eq!(view.backend_id, "claude-code");
    assert!(lock(&agents.tools).last().unwrap().in_run());
}

/// A run is the brief that starts now and reports to whoever asked. Work
/// for later, or for the person, is a job's, and the runner refuses it
/// rather than run it now.
#[tokio::test]
async fn a_run_refuses_a_brief_that_is_a_jobs() {
    let agents = Fake::new(Scripted::new(worked()));
    let room = room("run-brief", agents.clone());
    let mut later = spec(&room, "r8", "Check at nine.");
    later.brief.trigger = crate::session::Trigger::At(now_ms() + 60_000);
    let outcome = room.run(later, CancellationToken::new()).await;
    assert_eq!(outcome.end, RunEnd::Failed);
    assert!(outcome.report.contains("is a job"), "{}", outcome.report);
    assert!(lock(&agents.driver.prompts).is_empty());
}

/// A clone has the home thread's permission set, its computer included,
/// under a child of the home session's lease; the home agent can leave the
/// computer out for one task, and the run is built without it.
#[cfg(unix)]
#[tokio::test]
async fn a_run_has_the_computer_unless_the_home_agent_leaves_it_out() {
    let ComputerRoom { room, agents, .. } =
        computer_room("run-computer", Some("0.10.3"), TWO_RELEASES, true).await;
    room.start("ada").await.unwrap();
    let home = lock(&agents.tools).last().unwrap().capability().unwrap();
    let computer = |view: &Persona| view.computer.as_ref().is_some_and(|c| c.enabled);

    let whole = room
        .run(
            RunSpec {
                capability: home.clone(),
                ..spec(&room, "r9", "Look at the screen.")
            },
            CancellationToken::new(),
        )
        .await;
    assert_eq!(whole.end, RunEnd::Done, "{}", whole.report);
    let view = lock(&agents.views).last().cloned().unwrap();
    assert!(computer(&view), "a clone has its teammate's computer");

    let mut narrowed = spec(&room, "r10", "Read the log only.");
    narrowed.capability = home.clone();
    narrowed.brief.without = vec![crate::contract::Ability::Computer];
    let outcome = room.run(narrowed, CancellationToken::new()).await;
    assert_eq!(outcome.end, RunEnd::Done, "{}", outcome.report);
    let view = lock(&agents.views).last().cloned().unwrap();
    assert!(!computer(&view), "left out for this task");

    // Never wider: the run's lease is a child of the home session's.
    home.revoke();
    let stale = room
        .run(
            RunSpec {
                capability: home,
                ..spec(&room, "r11", "Anything.")
            },
            CancellationToken::new(),
        )
        .await;
    assert_eq!(stale.end, RunEnd::Failed);
    assert!(stale.report.contains("revoked"), "{}", stale.report);
}

#[tokio::test]
async fn a_run_the_last_process_left_running_is_cancelled_when_the_room_opens() {
    let log = scratch("run-orphan");
    enrol(&log, &persona("ada"));
    let left = TranscriptEvent::Subagent {
        id: "subagent:r7".to_string(),
        ts: 5,
        run_id: "r7".to_string(),
        title: "Check the crane".to_string(),
        status: SubagentStatus::Running,
        elapsed_ms: None,
    };
    log.append(
        &StreamId::Tape("ada".to_string()),
        &serde_json::to_value(&left).unwrap(),
    )
    .unwrap();
    let room = Room::with_agents_and_computers(
        log,
        Arc::new(DeskKeys),
        Fake::new(Scripted::new(worked())),
        crate::computer::Computer::with_path(std::env::temp_dir().join("no-runtime")),
    );
    let tape = tape(&room, "ada");
    assert_eq!(kinds(&tape), ["subagent"]);
    assert_eq!(tape[0]["status"], "cancelled");
    assert_eq!(tape[0]["ts"], 5, "the line keeps its place");
    assert_eq!(run_stream(&room, "r7")[0]["status"], "cancelled");
}

/// A harness's own subagent: the harness does the work and says so, and
/// the room writes it down the way it writes its own runs — a line on the
/// tape, a stream of its own, a row on the roster while it goes.
#[tokio::test]
async fn a_harness_subagent_is_written_as_a_run() {
    use crate::driver::SubagentReport;
    let room = room("harness-run", Fake::new(Scripted::new(Vec::new())));
    let (reports, received) = tokio::sync::mpsc::unbounded_channel();
    room.watch_subagents("ada".to_string(), received);
    let child = || "child-1".to_string();
    reports
        .send(SubagentReport::Started {
            child: child(),
            kind: crate::contract::RunKind::Subagent,
            title: "Test investigator".to_string(),
            task: "Run the tests and report.".to_string(),
        })
        .unwrap();
    for update in worked()
        .into_iter()
        .filter(|update| !matches!(update, Update::Turn { .. }))
    {
        reports
            .send(SubagentReport::Update {
                child: child(),
                update,
            })
            .unwrap();
    }
    // Another child's ending is not this one's.
    reports
        .send(SubagentReport::Ended {
            child: "child-2".to_string(),
            status: SubagentStatus::Done,
        })
        .unwrap();
    let settled = async |status: &str| {
        for _ in 0..500 {
            let tape = tape(&room, "ada");
            if tape.first().is_some_and(|line| line["status"] == status)
                && (status != "running"
                    || run_stream(&room, tape[0]["runId"].as_str().unwrap()).len() >= 4)
            {
                return tape;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        panic!("the run never got to {status}: {:?}", tape(&room, "ada"));
    };
    let tape = settled("running").await;
    assert_eq!(kinds(&tape), ["subagent"]);
    assert_eq!(tape[0]["title"], "Test investigator");
    let run_id = tape[0]["runId"].as_str().unwrap().to_string();
    let listed = room.subagents("ada");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].run_id, run_id);

    reports
        .send(SubagentReport::Ended {
            child: child(),
            status: SubagentStatus::Done,
        })
        .unwrap();
    let tape = settled("done").await;
    assert_eq!(kinds(&tape), ["subagent"], "none of the run's words");
    assert_eq!(
        room.subagents("ada"),
        [] as [crate::contract::RunningSubagent; 0]
    );
    let stream = run_stream(&room, &run_id);
    assert_eq!(
        kinds(&stream),
        ["subagent", "user", "agent", "tool", "agent"]
    );
    assert_eq!(stream[1]["text"], "Run the tests and report.");
    assert_eq!(stream[3]["status"], "completed");
}

/// A harness that stops reporting — restarted, gone — leaves no run
/// drawn running forever.
#[tokio::test]
async fn a_harness_subagent_nobody_heard_end_is_cancelled() {
    use crate::driver::SubagentReport;
    let room = room("harness-gone", Fake::new(Scripted::new(Vec::new())));
    let (reports, received) = tokio::sync::mpsc::unbounded_channel();
    room.watch_subagents("ada".to_string(), received);
    reports
        .send(SubagentReport::Started {
            child: "child-1".to_string(),
            kind: crate::contract::RunKind::Subagent,
            title: "Explorer".to_string(),
            task: String::new(),
        })
        .unwrap();
    drop(reports);
    for _ in 0..500 {
        if tape(&room, "ada")
            .first()
            .is_some_and(|line| line["status"] == "cancelled")
        {
            assert_eq!(
                room.subagents("ada"),
                [] as [crate::contract::RunningSubagent; 0]
            );
            let run_id = tape(&room, "ada")[0]["runId"].as_str().unwrap().to_string();
            assert_eq!(
                kinds(&run_stream(&room, &run_id)),
                ["subagent"],
                "no empty task"
            );
            return;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    panic!("the run was left running: {:?}", tape(&room, "ada"));
}

/// A command the harness left running past its turn is on the roster as a
/// background run, said apart from a subagent, for as long as it runs; its
/// end takes it off and leaves what came of it in the run.
#[tokio::test]
async fn a_harness_background_task_is_listed_by_its_kind_while_it_runs() {
    use crate::contract::RunKind;
    use crate::driver::SubagentReport;
    let room = room("harness-background", Fake::new(Scripted::new(Vec::new())));
    let (reports, received) = tokio::sync::mpsc::unbounded_channel();
    room.watch_subagents("ada".to_string(), received);
    reports
        .send(SubagentReport::Started {
            child: "t1".to_string(),
            kind: RunKind::Shell,
            title: "Crawl 12 sites".to_string(),
            task: "python3 crawl.py sites2 300".to_string(),
        })
        .unwrap();
    until("the task is listed", || !room.subagents("ada").is_empty()).await;
    let listed = room.subagents("ada");
    assert_eq!(listed[0].kind, RunKind::Shell);
    assert_eq!(listed[0].title, "Crawl 12 sites");
    assert_eq!(
        listed[0].command.as_deref(),
        Some("python3 crawl.py sites2 300"),
        "the roster says what it runs"
    );
    let run_id = listed[0].run_id.clone();
    assert_eq!(
        serde_json::to_value(&listed[0]).unwrap()["kind"],
        "shell",
        "the roster says the kind"
    );

    reports
        .send(SubagentReport::Update {
            child: "t1".to_string(),
            update: Update::Message {
                kind: MessageKind::Agent,
                id: "t1:summary".to_string(),
                text: "12 sites crawled".to_string(),
            },
        })
        .unwrap();
    reports
        .send(SubagentReport::Ended {
            child: "t1".to_string(),
            status: SubagentStatus::Done,
        })
        .unwrap();
    until("the task is off the roster", || {
        room.subagents("ada").is_empty()
    })
    .await;
    until("the run is done", || {
        tape(&room, "ada")
            .first()
            .is_some_and(|line| line["status"] == "done")
    })
    .await;
    let stream = run_stream(&room, &run_id);
    assert_eq!(kinds(&stream), ["subagent", "user", "agent"]);
    assert_eq!(stream[2]["text"], "12 sites crawled");
}

/// A command the harness left running is read as it runs: `process.output`
/// answers its run's id with the command and the latest of what it wrote,
/// each piece added and a whole output replacing what came before; its end
/// keeps the last of it in the run as its one step, and its line says it is
/// a command, on the tape and at the head of the run.
#[tokio::test]
async fn a_background_commands_output_is_read_while_it_runs_and_kept_when_it_ends() {
    use crate::contract::RunKind;
    use crate::driver::SubagentReport;
    let room = room("harness-output", Fake::new(Scripted::new(Vec::new())));
    let (reports, received) = tokio::sync::mpsc::unbounded_channel();
    room.watch_subagents("ada".to_string(), received);
    reports
        .send(SubagentReport::Started {
            child: "c1".to_string(),
            kind: RunKind::Shell,
            title: "make precheck".to_string(),
            task: "`make precheck`".to_string(),
        })
        .unwrap();
    until("the command is listed", || {
        !room.subagents("ada").is_empty()
    })
    .await;
    let run_id = room.subagents("ada")[0].run_id.clone();
    let output = |text: &str, replaces: bool| {
        reports
            .send(SubagentReport::Output {
                child: "c1".to_string(),
                text: text.to_string(),
                replaces,
            })
            .unwrap();
    };
    output("==> secrets\n", false);
    output("secrets pass\n", false);
    until("the output is read", || {
        room.process_output("ada", &run_id)
            .is_ok_and(|read| read.tail == "==> secrets\nsecrets pass\n")
    })
    .await;
    assert_eq!(
        room.process_output("ada", &run_id).unwrap().command,
        "make precheck"
    );
    output("all 8 legs pass\n", true);
    until("a whole output replaces it", || {
        room.process_output("ada", &run_id)
            .is_ok_and(|read| read.tail == "all 8 legs pass\n")
    })
    .await;

    reports
        .send(SubagentReport::Ended {
            child: "c1".to_string(),
            status: SubagentStatus::Done,
        })
        .unwrap();
    until("the command is off the roster", || {
        room.subagents("ada").is_empty()
    })
    .await;
    assert!(
        room.process_output("ada", &run_id).is_err(),
        "an ended command is read from its run"
    );
    let stream = run_stream(&room, &run_id);
    assert_eq!(kinds(&stream), ["subagent", "user", "tool"]);
    assert_eq!(stream[2]["status"], "completed");
    assert_eq!(stream[2]["output"][0]["text"], "all 8 legs pass\n");
    let raw = room.log.load(&StreamId::Run(run_id.clone()));
    assert_eq!(raw[0]["kind"], "link");
    assert_eq!(raw[0]["runKind"], "shell", "its line says it is a command");
    let tape_line = room
        .log
        .load(&StreamId::Tape("ada".to_string()))
        .into_iter()
        .find(|line| line["thread"] == run_id.as_str())
        .unwrap();
    assert_eq!(tape_line["runKind"], "shell");
}

/// A run's roster row says what it did last, its latest step or the first
/// clause of its latest words, and counts each step once however often it is
/// written; each change tells the roster, so nothing has to ask.
#[tokio::test]
async fn a_runs_roster_row_follows_its_latest_step_and_words() {
    use crate::driver::SubagentReport;
    let room = room("harness-steps", Fake::new(Scripted::new(Vec::new())));
    let mut told = room.subscribe_info();
    let (reports, received) = tokio::sync::mpsc::unbounded_channel();
    room.watch_subagents("ada".to_string(), received);
    let send = |update: Update| {
        reports
            .send(SubagentReport::Update {
                child: "child-1".to_string(),
                update,
            })
            .unwrap();
    };
    let row = || room.subagents("ada").first().cloned();
    reports
        .send(SubagentReport::Started {
            child: "child-1".to_string(),
            kind: crate::contract::RunKind::Subagent,
            title: "Audit the rows".to_string(),
            task: "Read the dock.".to_string(),
        })
        .unwrap();
    until("the run is listed", || row().is_some()).await;
    let listed = row().unwrap();
    assert_eq!(
        (listed.last_step, listed.steps),
        (None, 0),
        "nothing done yet"
    );
    assert!(listed.command.is_none());

    while told.try_recv().is_ok() {}
    send(Update::Message {
        kind: MessageKind::Agent,
        id: "m1".to_string(),
        text: "Starting with the right pane, then the conversation.".to_string(),
    });
    until("its words are its latest", || {
        row().and_then(|run| run.last_step).as_deref()
            == Some("\u{201c}Starting with the right pane\u{201d}")
    })
    .await;
    assert!(told.try_recv().is_ok(), "the roster was told");

    send(Update::ToolCall {
        call_id: "c1".to_string(),
        title: "Read ui/src/components/Dock.tsx".to_string(),
        kind: "read".to_string(),
    });
    until("its step is its latest", || {
        row().is_some_and(|run| run.last_step.as_deref() == Some("Read Dock.tsx") && run.steps == 1)
    })
    .await;

    send(Update::ToolResult {
        call_id: "c1".to_string(),
        ok: true,
        output: "export function Dock".to_string(),
        images: Vec::new(),
    });
    send(Update::Message {
        kind: MessageKind::Thought,
        id: "t1".to_string(),
        text: "Nothing carries a last step yet, so every row says working.".to_string(),
    });
    until("its thinking is its latest", || {
        row().and_then(|run| run.last_step).as_deref()
            == Some("\u{201c}Nothing carries a last step yet\u{201d}")
    })
    .await;
    assert_eq!(
        row().unwrap().steps,
        1,
        "a step that ends is not a second step"
    );

    reports
        .send(SubagentReport::Ended {
            child: "child-1".to_string(),
            status: SubagentStatus::Done,
        })
        .unwrap();
    until("the run is off the roster", || row().is_none()).await;
}

/// A subagent's row on the roster is as it always was: no kind.
#[test]
fn a_subagents_roster_row_carries_no_kind() {
    let row = crate::contract::RunningSubagent {
        run_id: "r".into(),
        title: "Explorer".into(),
        started_at: 1,
        kind: crate::contract::RunKind::Subagent,
        last_step: None,
        steps: 0,
        command: None,
    };
    assert!(serde_json::to_value(&row).unwrap().get("kind").is_none());
}

/// A subagent told its task apart from its announcement, as the RFD tells
/// it, has the task as its run's first line all the same.
#[tokio::test]
async fn a_subagents_task_said_after_its_start_opens_its_run() {
    use crate::contract::RunKind;
    use crate::driver::SubagentReport;
    let room = room("harness-told", Fake::new(Scripted::new(Vec::new())));
    let (reports, received) = tokio::sync::mpsc::unbounded_channel();
    room.watch_subagents("ada".to_string(), received);
    reports
        .send(SubagentReport::Started {
            child: "child-1".to_string(),
            kind: RunKind::Subagent,
            title: "Edge cases".to_string(),
            task: String::new(),
        })
        .unwrap();
    reports
        .send(SubagentReport::Told {
            child: "child-1".to_string(),
            text: "Find the edge cases.".to_string(),
        })
        .unwrap();
    until("the run is listed", || !room.subagents("ada").is_empty()).await;
    let run_id = room.subagents("ada")[0].run_id.clone();
    until("its task is written", || {
        run_stream(&room, &run_id).len() >= 2
    })
    .await;
    let stream = run_stream(&room, &run_id);
    assert_eq!(kinds(&stream), ["subagent", "user"]);
    assert_eq!(stream[1]["text"], "Find the edge cases.");
}

/// Claude's adapter can tell a subagent its prompt twice, under a stand-in
/// id and then under its call's: the run shows it once. The same words told
/// again after the subagent has done something are a new message, and show.
#[tokio::test]
async fn a_subagents_prompt_told_twice_in_a_row_is_written_once() {
    use crate::contract::RunKind;
    use crate::driver::SubagentReport;
    let room = room("harness-told-twice", Fake::new(Scripted::new(Vec::new())));
    let (reports, received) = tokio::sync::mpsc::unbounded_channel();
    room.watch_subagents("ada".to_string(), received);
    let told = |text: &str| SubagentReport::Told {
        child: "child-1".to_string(),
        text: text.to_string(),
    };
    reports
        .send(SubagentReport::Started {
            child: "child-1".to_string(),
            kind: RunKind::Subagent,
            title: "Edge cases".to_string(),
            task: String::new(),
        })
        .unwrap();
    until("the run is listed", || !room.subagents("ada").is_empty()).await;
    let run_id = room.subagents("ada")[0].run_id.clone();
    reports.send(told("Find the edge cases.")).unwrap();
    reports.send(told("Find the edge cases.\n")).unwrap();
    reports
        .send(SubagentReport::Update {
            child: "child-1".to_string(),
            update: Update::Message {
                kind: MessageKind::Agent,
                id: "child-1:a1".to_string(),
                text: "Three edge cases.".to_string(),
            },
        })
        .unwrap();
    reports
        .send(SubagentReport::Ended {
            child: "child-1".to_string(),
            status: SubagentStatus::Done,
        })
        .unwrap();
    until("the run is done", || room.subagents("ada").is_empty()).await;
    until("its answer is written", || {
        run_stream(&room, &run_id)
            .iter()
            .any(|line| line["kind"] == "agent")
    })
    .await;
    let stream = run_stream(&room, &run_id);
    assert_eq!(kinds(&stream), ["subagent", "user", "agent"], "{stream:?}");
}

/// A spawn that carries the prompt writes it, and the same prompt sent as
/// the subagent's first message is not written again.
#[tokio::test]
async fn a_subagents_prompt_said_at_its_spawn_and_told_again_is_written_once() {
    use crate::contract::RunKind;
    use crate::driver::SubagentReport;
    let room = room("harness-spawn-told", Fake::new(Scripted::new(Vec::new())));
    let (reports, received) = tokio::sync::mpsc::unbounded_channel();
    room.watch_subagents("ada".to_string(), received);
    reports
        .send(SubagentReport::Started {
            child: "child-1".to_string(),
            kind: RunKind::Subagent,
            title: "Edge cases".to_string(),
            task: "Find the edge cases.".to_string(),
        })
        .unwrap();
    reports
        .send(SubagentReport::Told {
            child: "child-1".to_string(),
            text: "Find the edge cases.".to_string(),
        })
        .unwrap();
    reports
        .send(SubagentReport::Update {
            child: "child-1".to_string(),
            update: Update::Message {
                kind: MessageKind::Agent,
                id: "child-1:a1".to_string(),
                text: "Three edge cases.".to_string(),
            },
        })
        .unwrap();
    until("the run is listed", || !room.subagents("ada").is_empty()).await;
    let run_id = room.subagents("ada")[0].run_id.clone();
    reports
        .send(SubagentReport::Ended {
            child: "child-1".to_string(),
            status: SubagentStatus::Done,
        })
        .unwrap();
    until("its answer is written", || {
        run_stream(&room, &run_id)
            .iter()
            .any(|line| line["kind"] == "agent")
    })
    .await;
    assert_eq!(
        kinds(&run_stream(&room, &run_id)),
        ["subagent", "user", "agent"]
    );
}
