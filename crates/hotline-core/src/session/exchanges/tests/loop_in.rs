//! The person's two acts on a work thread from its menu (decision 5): Loop
//! in…, which brings a colleague in for one answer, and End thread, which
//! ends a handed-over thread and its handoff.

use super::questions::answer_to;
use super::*;
use crate::contract::{Aside, AsideLevel, AsideSource, End, Reach, SideEnd};
use crate::thread::ThreadState;

fn asides(room: &Room, persona_id: &str) -> Vec<Aside> {
    crate::store::asides::of(&room.log, persona_id)
}

fn in_thread(room: &Room, thread: &str) -> Vec<serde_json::Value> {
    room.log.load(&StreamId::Side(thread.to_string()))
}

/// What Bob's answers wrote in the thread: deliveries in his name.
fn bobs_answers(room: &Room, thread: &str) -> Vec<serde_json::Value> {
    in_thread(room, thread)
        .into_iter()
        .filter(|v| v["kind"] == "delivery" && v["cause"]["personaId"] == "bob")
        .collect()
}

/// Ada's own work thread, with the person looping Bob in: Bob is left a
/// nudge from the thread, heard as the person's, the thread draws it, and Bob answers once there,
/// under his name, with no card for the grant, since the person's act is it.
#[tokio::test]
async fn a_looped_in_colleague_is_nudged_and_answers_once_in_the_thread() {
    let (room, agents) = setup("loop-in-answers");
    // Not a Whole machine Hotline Agent: no implicit grant to ask Bob.
    let mut ada = room.persona("ada").unwrap();
    ada.reach = Some(Reach::Workspace);
    crate::room::append_persona(&room.log, &ada).unwrap();
    let mut bob = room.persona("bob").unwrap();
    bob.name = "Bob".into();
    crate::room::append_persona(&room.log, &bob).unwrap();
    room.log
        .append(
            &StreamId::Room,
            &json!({"kind": "setting", "id": "personName", "value": "George"}),
        )
        .unwrap();
    let opened = room.start_side("ada", "Ship the release").await.unwrap();
    let thread = opened.side_id;
    until("ada's thread has answered", || {
        in_thread(&room, &thread)
            .iter()
            .any(|v| v["kind"] == "agent")
    })
    .await;
    until("ada's thread is at rest", || {
        room.sides.get(&thread).is_none_or(|side| !side.working())
    })
    .await;

    room.loop_in(&thread, "bob").await.unwrap();

    let nudge = asides(&room, "bob")
        .into_iter()
        .find(|aside| aside.looped_in)
        .expect("bob was left the loop-in aside");
    assert_eq!(nudge.level, AsideLevel::Nudge);
    assert_eq!(nudge.from, AsideSource::Thread(ThreadId::side(&thread)));
    assert_eq!(nudge.by.as_deref(), Some("ada"));
    assert_eq!(nudge.text, "George looped you in.");
    assert!(
        in_thread(&room, &thread)
            .iter()
            .any(|v| v["kind"] == "aside" && v["id"] == nudge.id.as_str() && v["loopedIn"] == true),
        "the thread draws who was looped in"
    );

    until("bob's answer is in the thread", || {
        !bobs_answers(&room, &thread).is_empty()
    })
    .await;
    let asked = agents
        .told()
        .into_iter()
        .find(|prompt| prompt.contains("looped you into"))
        .expect("bob was asked");
    until("bob hears who looped him in", || {
        agents.told().iter().any(|prompt| {
            prompt.contains(&format!(
                "George, in side:{thread} (nudge): George looped you in."
            ))
        })
    })
    .await;
    assert!(asked.contains("Ship the release"), "{asked}");
    assert!(
        asked.contains("result-0"),
        "the thread's last lines: {asked}"
    );
    let answers = bobs_answers(&room, &thread);
    assert_eq!(answers.len(), 1, "one answer: {answers:?}");
    assert_eq!(answers[0]["cause"]["kind"], "peer");
    assert_eq!(answers[0]["cause"]["status"], "done");
    assert!(
        room.tape("ada")
            .iter()
            .chain(room.tape("bob").iter())
            .all(|v| v["kind"] != "permission"),
        "no card asked for the grant"
    );
    // Ada's thread takes the answer on.
    let said = answers[0]["text"].as_str().unwrap().to_string();
    until("ada's thread hears bob", || {
        agents
            .prompts()
            .iter()
            .any(|prompt| prompt.starts_with("Bob answered") && prompt.contains(&said))
    })
    .await;
}

#[tokio::test]
async fn loop_in_is_refused_for_the_threads_own_teammate_and_an_ended_thread() {
    let (room, _agents) = setup("loop-in-refused");
    let opened = room.start_side("ada", "Ship it").await.unwrap();
    let own = room.loop_in(&opened.side_id, "ada").await.unwrap_err();
    assert!(own.contains("own teammate"), "{own}");
    room.archive_side(&opened.side_id, SideEnd::Person, None)
        .unwrap();
    let ended = room.loop_in(&opened.side_id, "bob").await.unwrap_err();
    assert!(ended.contains("has ended"), "{ended}");
    assert_eq!(asides(&room, "bob"), Vec::<Aside>::new());
}

/// Bob's handoff from Ada waits on the person, who ends its thread: it
/// closes as theirs, the handoff is released and stopped with no result, and
/// Ada is told once, by a note, with nothing in her chat and no turn.
#[tokio::test]
async fn the_persons_end_thread_releases_the_handoff_and_tells_its_opener_once_by_a_note() {
    let (room, id, _action) = suspended_handoff("end-thread").await;
    let thread = work_thread(&room, &id);
    let ada_before = room.tape("ada").len();

    room.archive_side(&thread, SideEnd::Person, None).unwrap();

    let link = room.link_of(&ThreadId::side(&thread)).unwrap();
    assert_eq!(link.state, ThreadState::Closed(End::Person));
    assert!(link.handoff_ended, "the bar says the handoff has ended");
    let request = room
        .exchange_pair("ada~bob")
        .unwrap()
        .requests
        .into_iter()
        .find(|r| r.id == id)
        .unwrap();
    assert!(request.released);
    assert_eq!(request.phase, Phase::Stopped);
    assert!(!request.owed_to_thread);

    let ended: Vec<Aside> = asides(&room, "ada")
        .into_iter()
        .filter(|aside| aside.text.contains("ended this thread"))
        .collect();
    assert_eq!(ended.len(), 1, "{ended:?}");
    assert_eq!(ended[0].level, AsideLevel::Note);
    assert_eq!(ended[0].from, AsideSource::Thread(ThreadId::side(&thread)));
    assert!(answer_to(&room, &id).is_none(), "no result and no failure");
    assert_eq!(room.tape("ada").len(), ada_before, "nothing in Ada's chat");
    assert!(!room.mid_turn("ada"), "Ada is not woken");

    // Ending it again tells nobody twice.
    room.archive_side(&thread, SideEnd::Person, None).unwrap();
    assert_eq!(
        asides(&room, "ada")
            .iter()
            .filter(|aside| aside.text.contains("ended this thread"))
            .count(),
        1
    );
}
