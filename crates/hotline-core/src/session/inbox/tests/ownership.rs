use super::*;
use crate::contract::InboxWorkOwner;
use crate::session::{ownership::WorkFor, timed};

// Receiver inbox, 2026-11-20. The outgoing question dfff696f is
// separate: it is not another row in the receiver's inbox.
const ROWS: [(&str, InboxWants, Option<&str>, &str, &str); 8] = [
    (
        "6d1b005a",
        InboxWants::Work,
        None,
        "Implement the inbox path and quick-message plan",
        "6d1b005a",
    ),
    (
        "cded8020",
        InboxWants::Work,
        Some("6d1b005a"),
        "Continue the inbox plan: step 2 rule settled, then steps 3–5",
        "cded8020",
    ),
    (
        "395df625",
        InboxWants::Work,
        None,
        "Inbox: tell the sender when the receiver could not run",
        "395df625",
    ),
    (
        "3b5ec796",
        InboxWants::Work,
        Some("395df625"),
        "Two fixes to 4547b4d6",
        "3b5ec796",
    ),
    (
        "2e3768ac",
        InboxWants::Work,
        Some("3b5ec796"),
        "Wake-failure fix does not fire on the real desk",
        "2e3768ac",
    ),
    (
        "b27c85d5",
        InboxWants::Work,
        None,
        "Desk UI: three look fixes for inbox turns",
        "b27c85d5",
    ),
    (
        "6210ea19",
        InboxWants::Nothing,
        Some("dfff696f"),
        "Attached: mabel-slice.jsonl",
        "b27c85d5",
    ),
    (
        "98367872",
        InboxWants::Work,
        None,
        "Consolidate where a teammate's words go",
        "98367872",
    ),
];

fn desk(name: &str, driver: Scripted) -> (Arc<Room>, Arc<Fake>) {
    let (room, agents) = room_on(name, driver);
    enrol(&room.log, &named("pork", "Pork Chop", Some(Reach::Machine)));
    room.allow_sender("pork", "ada").unwrap();
    room.allow_sender("ada", "pork").unwrap();
    (room, agents)
}

fn receiver_rows(room: &Room) -> Vec<InboxItem> {
    let pork = room.participant("pork").unwrap();
    ROWS.iter()
        .map(|&(id, wants, reply_to, title, _)| {
            let mut row = super::super::item(
                &pork,
                &ThreadId::dm("pork"),
                "ada",
                title.into(),
                String::new(),
                wants,
            );
            row.id = id.into();
            row.reply_to = reply_to.map(str::to_owned);
            row
        })
        .collect()
}

#[tokio::test]
async fn receiver_inbox_distinguishes_follow_ups_from_answers_to_its_question() {
    let (room, _) = desk("inbox-owner-table", replies());
    let rows = receiver_rows(&room);
    for row in &rows {
        room.keep_item(row).unwrap();
    }
    let mut question = super::super::item(
        &room.participant("ada").unwrap(),
        &ThreadId::dm("ada"),
        "pork",
        "Can you attach the slice?".into(),
        String::new(),
        InboxWants::Answer,
    );
    question.id = "dfff696f".into();
    room.keep_item(&question).unwrap();
    assert_eq!(room.colleague_work(&rows[6]), WorkFor::Person);
    question.work_owner = Some(InboxWorkOwner {
        name: "Pork Chop".into(),
        item: "b27c85d5".into(),
    });
    room.keep_item(&question).unwrap();
    for (row, &(_, _, _, _, owner)) in rows.iter().zip(&ROWS) {
        assert_eq!(
            room.colleague_work(row),
            WorkFor::Colleague {
                name: "Pork Chop".into(),
                item: owner.into()
            },
            "{}",
            row.id
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_follow_up_handoff_cannot_post_its_result_to_the_person() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let (room, agents) = desk("inbox-follow-up-post", replies().gated(gate.clone()));
    let rows = receiver_rows(&room);
    let mut original = rows[0].clone();
    original.state = InboxState::Closed;
    room.keep_item(&original).unwrap();
    let follow_up = &rows[1];
    room.keep_item(follow_up).unwrap();
    room.arrived(follow_up);
    until("the follow-up wakes Ada", || !agents.prompts().is_empty()).await;
    assert!(agents.prompts()[0].contains("for Pork Chop (item cded8020), not the person"));
    tool(
        &room,
        "ada",
        "update_inbox",
        json!({"items":[{
            "id": "cded8020", "state": "taken", "in_thread": false
        }]}),
    )
    .await
    .unwrap();
    assert_eq!(
        tool(
            &room,
            "ada",
            "post_to_chat",
            json!({"text":"The inbox plan is implemented.","kind":"result"})
        )
        .await
        .unwrap_err(),
        "The result goes to Pork Chop: answer item cded8020."
    );
    assert!(!room.has_posted(&ThreadId::dm("ada")));
    gate.add_permits(2);
}

#[tokio::test]
async fn the_answer_inherits_the_owner_captured_by_send() {
    let (room, _) = desk("inbox-owner-send", replies());
    let row = receiver_rows(&room).remove(5);
    room.keep_item(&row).unwrap();
    let question = {
        let _turn = room.open_posting(&ThreadId::dm("ada"), None);
        tool(
            &room,
            "ada",
            "update_inbox",
            json!({"items":[{
                "id": row.id, "state": "taken", "in_thread": false
            }]}),
        )
        .await
        .unwrap();
        send(
            &room,
            "ada",
            "pork",
            "Can you send the slice?",
            json!({"wants":"answer"}),
        )
        .await
    };
    let asked = item(&room, "pork", &question);
    assert!(!asked.for_person, "no loud wake for colleague-owned work");
    let answer = answer_with_send(&room, &asked, "Here it is.").await;
    assert_eq!(
        room.colleague_work(&answer),
        WorkFor::Colleague {
            name: "Pork Chop".into(),
            item: "b27c85d5".into(),
        }
    );
}

#[tokio::test]
async fn a_work_threads_report_opens_the_dm_for_that_threads_owner() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let ends = || Update::Turn {
        stop_reason: "end_turn".into(),
        usage: None,
    };
    let driver = Scripted::turns(vec![vec![ends()], vec![], vec![], vec![ends()], vec![]])
        .gated(gate.clone());
    let (room, agents) = desk("inbox-owner-thread-report", driver);
    let row = receiver_rows(&room).remove(5);
    room.keep_item(&row).unwrap();
    tool(
        &room,
        "ada",
        "update_inbox",
        json!({"items":[{"id":row.id,"state":"taken"}]}),
    )
    .await
    .unwrap();
    until("the colleague's work thread is running", || {
        agents.prompts().len() == 1
    })
    .await;
    let colleague_tools = agents.tools()[0].clone();
    assert!(agents.prompts()[0].contains("Desk UI: three look fixes"));

    room.start("ada").await.unwrap();
    room.prompt("ada", "Hello", None, None).await.unwrap();
    until("the DM answers Hello", || {
        agents.prompts().len() == 2 && room.info("ada").state == SessionState::Ready
    })
    .await;
    assert!(agents.prompts()[1].contains("Hello"));
    let posted = colleague_tools
        .call(
            "post_to_chat",
            &json!({"text":"Decision needed.","kind":"decision","wake":true}),
        )
        .await
        .unwrap();
    assert!(posted.contains("acting on it"));
    until("the DM hears the colleague's report", || {
        agents.prompts().len() == 3
    })
    .await;
    assert!(agents.prompts()[2].contains("a thread of yours reported back. This work is for Pork Chop (item b27c85d5), not the person"));
    until("the DM has read the report", || {
        room.info("ada").state == SessionState::Ready
    })
    .await;

    let side = room
        .start_side("ada", "The person's next task")
        .await
        .unwrap();
    until("the person's work thread is running", || {
        agents.prompts().len() == 4
    })
    .await;
    assert!(agents.prompts()[3].contains("The person's next task"));
    let person_tools = TeammateTools::new(&room, "ada").for_work(side.side_id);
    person_tools
        .call(
            "post_to_chat",
            &json!({"text":"Done.","kind":"result","wake":true}),
        )
        .await
        .unwrap();
    until("the DM hears the person's report", || {
        agents.prompts().len() == 5
    })
    .await;
    assert!(
        agents.prompts()[4]
            .contains("a thread of yours reported back. This work is for the person.")
    );
    gate.add_permits(2);
}

#[tokio::test]
async fn a_side_commands_launch_owner_survives_the_persons_hello() {
    use crate::session::jobs::ProcessReturn;
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let (driver, wakes) = replies().gated(gate.clone()).with_wakes();
    let (room, agents) = desk("side-command-ownership", driver);
    let row = receiver_rows(&room).remove(5);
    room.keep_item(&row).unwrap();
    let side = room.start_side("ada", "Start the side work").await.unwrap();
    let thread = ThreadId::side(&side.side_id);
    until("the side turn is in flight", || agents.prompts().len() == 1).await;
    let tools = &agents.tools()[0];
    tools
        .call(
            "update_inbox",
            &json!({"items":[{"id":row.id,"state":"taken","in_thread":false}]}),
        )
        .await
        .unwrap();
    let track = tools.ownership();
    let first = track(None);
    gate.add_permits(2);
    until("the first launch turn ends", || {
        !room.work_thread_working(&thread)
    })
    .await;

    room.prompt_side(&side.side_id, "Hello", None)
        .await
        .unwrap();
    until("the side is answering Hello", || {
        agents.prompts().len() == 2
    })
    .await;
    assert!(agents.prompts()[1].contains("Hello"));
    let second = track(None);
    gate.add_permits(2);
    until("the Hello turn ends", || !room.work_thread_working(&thread)).await;

    for (index, owner, expected) in [
        (
            3,
            first,
            "your command came back. This work is for Pork Chop (item b27c85d5), not the person",
        ),
        (
            4,
            second,
            "your command came back. This work is for the person.",
        ),
    ] {
        wakes
            .send(ProcessReturn {
                work_for: owner,
                text: timed(now_ms(), "A command finished."),
            })
            .unwrap();
        until(
            "the real side watcher hands the return to the driver",
            || agents.prompts().len() == index,
        )
        .await;
        assert!(
            agents.prompts()[index - 1].contains(expected),
            "{:?}",
            agents.prompts()
        );
        gate.add_permits(2);
        until("the return turn ends", || {
            !room.work_thread_working(&thread)
        })
        .await;
    }
}
