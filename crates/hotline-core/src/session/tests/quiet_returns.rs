//! A turn of the main conversation that nobody started is quiet, whatever
//! the agent runs on: a line of Hotline's that work came back begins (a
//! command that ended, the asides' wake), work the agent began by itself
//! between turns (Claude Code waking for its own helper or background
//! command), and the rest of any turn once it has replied and parked for its
//! helpers (both drivers park). Its words are kept as the turn's path, marked
//! as said, so the person can still read them; its one post is its way to
//! the chat; and the person's own line steered into it is answered out loud.

use super::*;

fn says(id: &str, text: &str) -> Update {
    Update::Message {
        kind: MessageKind::Agent,
        id: id.to_string(),
        text: text.to_string(),
    }
}

fn ends() -> Update {
    Update::Turn {
        stop_reason: "end_turn".to_string(),
        usage: None,
    }
}

fn helper(id: &str) -> Update {
    Update::ToolCall {
        call_id: id.to_string(),
        title: format!("Build part {id}"),
        kind: crate::session::jobs::SUBAGENT.to_string(),
    }
}

fn reports(id: &str) -> Update {
    Update::ToolResult {
        call_id: id.to_string(),
        ok: true,
        output: format!("Part {id} built."),
        images: Vec::new(),
    }
}

/// The event with `id`, as the tape holds it.
fn event(room: &Room, id: &str) -> Value {
    tape(room, "ada")
        .into_iter()
        .find(|event| event["id"] == id)
        .unwrap_or_else(|| panic!("{id} is not on the tape: {:?}", tape(room, "ada")))
}

/// The words the person reads: each `agent` line.
fn bubbles(room: &Room) -> Vec<String> {
    tape(room, "ada")
        .into_iter()
        .filter(|event| event["kind"] == "agent")
        .filter_map(|event| event["text"].as_str().map(str::to_string))
        .collect()
}

fn last_turn(room: &Room) -> Value {
    tape(room, "ada")
        .into_iter()
        .rfind(|event| event["kind"] == "turn")
        .expect("a turn ended")
}

/// Hotline's line that a command left running has ended begins a quiet
/// turn: what the agent writes is its path, said and not read, the turn is
/// marked quiet, and no phone is told. Its one post is a bubble.
#[tokio::test]
async fn a_line_that_work_came_back_begins_a_quiet_turn_that_posts_once() {
    let gate = Arc::new(Semaphore::new(0));
    let room = room(
        "quiet-return-line",
        Fake::new(
            Scripted::turns(vec![vec![says("m-back", "The build passed."), ends()]])
                .gated(gate.clone()),
        ),
    );
    pair_a_phone(&room);
    room.start("ada").await.unwrap();
    let session = room.session("ada").unwrap();
    {
        let _working = room.working().unwrap();
        room.dispatch(
            session,
            Wired::back(timed(
                now_ms(),
                "a command you left running has ended. managed job result: ok",
            )),
        );
    }
    until("the return's turn began", || {
        room.info("ada").state == SessionState::Thinking
    })
    .await;
    let dm = ThreadId::dm("ada");
    assert!(room.is_quiet(&dm));
    assert_eq!(
        room.post_to_chat("ada", &dm, "The build is done.", false)
            .await
            .as_deref(),
        Ok("Posted to the chat.")
    );
    gate.add_permits(2);
    until_state(&room, "ada", SessionState::Ready).await;
    let said = event(&room, "m-back");
    assert_eq!(said["kind"], "thought", "{said}");
    assert_eq!(said["said"], true, "{said}");
    assert_eq!(last_turn(&room)["quiet"], true);
    assert_eq!(bubbles(&room), ["The build is done."]);
    let sent = lock(&room.push.sent).clone();
    assert_eq!(sent.len(), 1, "the post alone is pushed: {sent:?}");
    assert_eq!(sent[0]["body"], "The build is done.");
}

/// A turn that replied and parked for its helpers is quiet from the park:
/// the reply before it is a bubble, the words after a helper came back are
/// its path. A post is refused before the park, where the reply is the chat,
/// and taken after it. Both drivers park this way.
#[tokio::test]
async fn the_rest_of_a_turn_after_it_parks_is_quiet() {
    let turn = vec![
        helper("h1"),
        says("m-reply", "Two helpers are building it."),
        Update::Parked,
        reports("h1"),
        says("m-done", "It is built and in the dev window."),
        ends(),
    ];
    let gate = Arc::new(Semaphore::new(0));
    let room = room(
        "quiet-after-park",
        Fake::new(Scripted::turns(vec![turn]).gated(gate.clone())),
    );
    room.start("ada").await.unwrap();
    room.prompt("ada", "build it", None, None).await.unwrap();
    let dm = ThreadId::dm("ada");

    gate.add_permits(1);
    until("the helper started", || {
        tape(&room, "ada")
            .iter()
            .any(|event| event["kind"] == "tool")
    })
    .await;
    assert!(!room.is_quiet(&dm), "the person's turn is read");
    assert!(
        room.post_to_chat("ada", &dm, "Early.", false)
            .await
            .is_err_and(|error| error.contains("already the chat"))
    );

    gate.add_permits(2);
    until("the turn parked", || room.info("ada").awaiting_subagents).await;
    assert_eq!(event(&room, "m-reply")["kind"], "agent");
    assert!(room.is_quiet(&dm), "what wakes it next nobody waits on");
    gate.add_permits(2);
    until("the helper's report was taken in", || {
        tape(&room, "ada")
            .iter()
            .any(|event| event["id"] == "m-done")
    })
    .await;
    assert_eq!(
        room.post_to_chat("ada", &dm, "Built: it is in the dev window.", false)
            .await
            .as_deref(),
        Ok("Posted to the chat.")
    );
    gate.add_permits(1);
    until_state(&room, "ada", SessionState::Ready).await;
    let done = event(&room, "m-done");
    assert_eq!(done["kind"], "thought", "{done}");
    assert_eq!(done["said"], true, "{done}");
    assert_eq!(last_turn(&room)["quiet"], true);
    assert_eq!(
        bubbles(&room),
        [
            "Two helpers are building it.",
            "Built: it is in the dev window."
        ]
    );
}

/// The safety net: a quiet turn that writes its news instead of posting it
/// does not lose it. The words are kept on the tape, marked as said, under
/// the turn the conversation draws as its time line, which opens to them.
#[tokio::test]
async fn words_a_quiet_turn_wrote_instead_of_posting_are_kept_as_said() {
    let turn = vec![
        helper("h1"),
        says("m-reply", "A helper is building it."),
        Update::Parked,
        reports("h1"),
        says("m-done", "Done: the inbox is in your dev window."),
        says(
            "m-done-2",
            "One thing for you: the phone build is untested.",
        ),
        ends(),
    ];
    let room = room("quiet-unposted", Fake::new(Scripted::turns(vec![turn])));
    room.start("ada").await.unwrap();
    room.prompt("ada", "build it", None, None).await.unwrap();
    until("the turn ended", || {
        tape(&room, "ada")
            .iter()
            .any(|event| event["kind"] == "turn")
    })
    .await;
    let kept: Vec<Value> = tape(&room, "ada")
        .into_iter()
        .filter(|event| event["kind"] == "thought" && event["said"] == true)
        .collect();
    assert_eq!(
        kept.iter()
            .filter_map(|event| event["text"].as_str())
            .collect::<Vec<_>>(),
        [
            "Done: the inbox is in your dev window.",
            "One thing for you: the phone build is untested."
        ]
    );
    assert_eq!(bubbles(&room), ["A helper is building it."]);
    assert_eq!(last_turn(&room)["quiet"], true);
}

/// The person's line steered into a quiet turn ends the quiet, as it does in
/// a turn the inbox woke: the rest of the turn answers them out loud.
#[tokio::test]
async fn the_persons_line_steered_into_a_parked_turn_is_answered_out_loud() {
    let turn = vec![
        helper("h1"),
        says("m-reply", "A helper is on it."),
        Update::Parked,
        says("m-answer", "It is about halfway."),
        ends(),
    ];
    let gate = Arc::new(Semaphore::new(0));
    let room = room(
        "quiet-steered",
        Fake::new(Scripted::turns(vec![turn]).gated(gate.clone()).consuming()),
    );
    pair_a_phone(&room);
    room.start("ada").await.unwrap();
    room.prompt("ada", "build it", None, None).await.unwrap();
    gate.add_permits(3);
    until("the turn parked", || room.info("ada").awaiting_subagents).await;
    let dm = ThreadId::dm("ada");
    assert!(room.is_quiet(&dm));

    room.prompt("ada", "how far along?", None, None)
        .await
        .unwrap();
    until("the person's line joined the turn", || !room.is_quiet(&dm)).await;
    gate.add_permits(2);
    until_state(&room, "ada", SessionState::Ready).await;
    assert_eq!(event(&room, "m-answer")["kind"], "agent");
    assert!(
        last_turn(&room).get("quiet").is_none(),
        "{}",
        last_turn(&room)
    );
    assert_eq!(
        bubbles(&room),
        ["A helper is on it.", "It is about halfway."]
    );
}

/// Work the agent began by itself between turns, as Claude Code does when
/// its own helper or background command finishes, is a quiet turn from its
/// first word: Hotline sent it nothing, and nobody waits on it. That takes
/// in the line Claude Code makes its model write after an empty answer.
#[tokio::test]
async fn work_the_agent_began_by_itself_is_a_quiet_turn_from_its_first_word() {
    let (scripted, unprompted) = Scripted::new(Vec::new()).with_unprompted();
    let room = room("quiet-unprompted", Fake::new(scripted));
    pair_a_phone(&room);
    room.start("ada").await.unwrap();

    let (sender, updates) = mpsc::channel(8);
    unprompted.send(updates).unwrap();
    sender
        .send(says("m-bg", "The tests finished: all green."))
        .await
        .unwrap();
    let dm = ThreadId::dm("ada");
    until("the agent's own turn is running", || room.is_quiet(&dm)).await;
    assert_eq!(
        room.post_to_chat("ada", &dm, "All green: it is merged.", false)
            .await
            .as_deref(),
        Ok("Posted to the chat.")
    );
    sender.send(ends()).await.unwrap();
    drop(sender);
    until_state(&room, "ada", SessionState::Ready).await;
    until("the turn is written", || {
        tape(&room, "ada")
            .iter()
            .any(|event| event["kind"] == "turn")
    })
    .await;
    let said = event(&room, "m-bg");
    assert_eq!(said["kind"], "thought", "{said}");
    assert_eq!(said["said"], true, "{said}");
    assert_eq!(last_turn(&room)["quiet"], true);
    assert_eq!(bubbles(&room), ["All green: it is merged."]);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let sent = lock(&room.push.sent).clone();
    assert_eq!(sent.len(), 1, "the post alone is pushed: {sent:?}");
}
