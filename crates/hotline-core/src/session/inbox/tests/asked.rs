//! What the person asked for, once it is finished (`inbox/asked.rs`): an
//! answer that finishes a send made for the person wakes its sender in a
//! turn the person reads, told to tell them the result in its own words;
//! a send to several finishes only when every item is closed, one held for
//! the person's card being still open; what was not for the person, and
//! what is still going, stays quiet; the person's follow-up makes its onward
//! send theirs; and a turn they read in flight is told it there.

use super::*;
use crate::session::tests::pair_a_phone;
use tokio::sync::Semaphore;

/// What Ada was handed when her inbox woke her: the lines that offer Bob's
/// or Cy's replies.
fn adas_wakes(agents: &crate::session::tests::Fake) -> Vec<String> {
    agents
        .told()
        .into_iter()
        .filter(|told| told.contains("· from Bob") || told.contains("· from Cy"))
        .collect()
}

fn idle(room: &Room, persona_id: &str) -> bool {
    room.session(persona_id)
        .is_ok_and(|session| !lock(&session.turns).running)
}

/// Ada, Bob and Cy, each allowed to reach the others.
fn team(name: &str) -> (Arc<Room>, Arc<crate::session::tests::Fake>) {
    let (room, agents) = room(name);
    enrol(&room.log, &named("cy", "Cy", Some(Reach::Machine)));
    room.allow_sender("cy", "ada").unwrap();
    room.allow_sender("ada", "cy").unwrap();
    (room, agents)
}

/// Ada sends in a turn the person reads, as their ask.
async fn ask(room: &Arc<Room>, to: &[&str], title: &str) -> Vec<String> {
    let _turn = room.open_posting(&ThreadId::dm("ada"), None);
    let sent = crate::session::tests::send_past_card(
        room,
        "ada",
        json!({"to": to, "title": title, "message": "?"}),
    )
    .await
    .unwrap();
    sent["sent"]
        .as_array()
        .unwrap()
        .iter()
        .map(|one| one["id"].as_str().unwrap().to_string())
        .collect()
}

async fn answer(room: &Arc<Room>, who: &str, id: &str, words: &str) {
    tool(
        room,
        who,
        "update_inbox",
        json!({"items": [{"id": id, "answer": words}]}),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn an_answer_that_finishes_the_persons_ask_wakes_a_turn_they_read_told_to_say_it_in_its_own_words()
 {
    let (room, agents) = room("asked-finished");
    pair_a_phone(&room);
    at_rest(&room, "ada").await;
    let asked = ask(&room, &["bob"], "Score of the last game?").await;
    assert!(item(&room, "bob", &asked[0]).for_person);
    answer(&room, "bob", &asked[0], "Devils 7, Capitals 4, Saturday.").await;
    until("Ada woke", || !adas_wakes(&agents).is_empty()).await;
    let told = adas_wakes(&agents).remove(0);
    assert!(
        told.contains("inbox · Finished: what you sent colleagues for something the person asked of you, \"Score of the last game?\" (Bob: answered). What came back is above, under INBOX"),
        "{told}"
    );
    assert!(
        told.contains("They read this turn: tell them the result in your own words.")
            && told.contains("Do not paste or quote your colleague's reply"),
        "{told}"
    );
    assert!(
        told.contains("finishes what the person asked of you"),
        "the index marks it too: {told}"
    );
    assert!(
        told.contains("message: \"Devils 7, Capitals 4, Saturday.\""),
        "{told}"
    );
    assert!(!told.contains("`reply_to`"), "{told}");
    assert!(!told.contains(QUIET_WAKE_LINE), "not quiet: {told}");
    until("Ada's turn ended", || {
        idle(&room, "ada") && room.tape("ada").iter().any(|event| event["kind"] == "turn")
    })
    .await;
    let tape = room.tape("ada");
    assert!(
        tape.iter().any(|event| event["kind"] == "agent"),
        "what she says is a reply the person reads"
    );
    let ended = tape.iter().rfind(|event| event["kind"] == "turn").unwrap();
    assert!(ended.get("quiet").is_none(), "{ended}");
    until("the phone is told", || !lock(&room.push.sent).is_empty()).await;
    assert!(room.finished_asks("ada").is_empty(), "told once");
}

/// Mack and Nina on Claude Code with Haiku, 2026-11-21: the person asked Mack,
/// Mack asked Nina, and Nina answered "6" with `send`, `reply_to` and, as
/// Haiku does every time, `wants: answer`. Mack's turn the person read told
/// them; the answer, shown whole there, was left seen, so Mack was woken
/// again in a quiet turn to sort it, whose line invites telling the person
/// a second time. The answer to the teammate's own question is done with
/// once the turn that told it has run: nothing wakes the teammate again.
#[tokio::test]
async fn an_answer_shown_in_the_turn_that_tells_the_person_is_not_offered_again() {
    let (room, agents) = room("asked-answer-once");
    at_rest(&room, "ada").await;
    let asked = ask(&room, &["bob"], "What is 3 plus 3?").await;
    let answered = crate::session::tests::send_past_card(
        &room,
        "bob",
        json!({"to": ["ada"], "message": "6", "reply_to": asked[0], "wants": "answer"}),
    )
    .await
    .unwrap();
    let reply = answered["sent"][0]["id"].as_str().unwrap().to_string();
    until("Ada's turn the person reads ended", || {
        idle(&room, "ada") && room.tape("ada").iter().any(|event| event["kind"] == "turn")
    })
    .await;
    let told = adas_wakes(&agents);
    assert_eq!(told.len(), 1, "{told:?}");
    assert!(
        told[0].contains("Finished:") && told[0].contains("message: \"6\""),
        "{}",
        told[0]
    );
    until("the answer is noted", || {
        item(&room, "ada", &reply).state == InboxState::Closed
    })
    .await;
    assert_eq!(
        item(&room, "ada", &reply).outcome,
        Some(InboxOutcome::Noted)
    );
    // The idle pick-up has nothing to offer: no quiet turn follows.
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    assert!(
        agents
            .told()
            .iter()
            .all(|line| !line.contains("read, not sorted")),
        "{:?}",
        agents.told()
    );
    assert_eq!(adas_wakes(&agents).len(), 1);
}

/// A send to several is finished when the last of its items closes: Bob's
/// answer alone is progress, heard in a quiet turn, whose line says the
/// result is told in a turn the person reads once it is all in.
#[tokio::test]
async fn a_send_to_several_finishes_when_every_item_is_closed_and_until_then_its_answers_are_quiet()
{
    let (room, agents) = team("asked-group");
    at_rest(&room, "ada").await;
    let asked = ask(&room, &["bob", "cy"], "Release on Thursday?").await;
    answer(&room, "bob", &asked[0], "Fine by me.").await;
    until("Ada woke", || !adas_wakes(&agents).is_empty()).await;
    let first = adas_wakes(&agents).remove(0);
    assert!(first.contains(QUIET_WAKE_LINE), "{first}");
    assert!(!first.contains("Finished:"), "{first}");
    until("Ada's quiet turn ended", || {
        idle(&room, "ada") && room.tape("ada").iter().any(|event| event["kind"] == "turn")
    })
    .await;
    assert!(
        !room
            .tape("ada")
            .iter()
            .any(|event| event["kind"] == "agent"),
        "nothing reached the person"
    );

    tool(
        &room,
        "cy",
        "update_inbox",
        json!({"items": [{"id": asked[1], "state": "closed", "outcome": "dropped", "reason": "not mine"}]}),
    )
    .await
    .unwrap();
    until("Ada woke to tell the person", || {
        agents.told().iter().any(|told| told.contains("Finished:"))
    })
    .await;
    let finished = agents
        .told()
        .into_iter()
        .find(|told| told.contains("Finished:"))
        .unwrap();
    assert!(
        finished.contains("\"Release on Thursday?\" (Bob: answered, Cy: dropped)"),
        "a decline finishes it too, with nothing landing in Ada's inbox: {finished}"
    );
    assert!(finished.contains("They read this turn: tell them the result in your own words."));
}

/// An item held for the person's first-contact card is not finished, and an
/// item sent outside a turn the person reads is no ask of theirs: both are
/// quiet when answered.
#[tokio::test]
async fn a_held_item_keeps_a_send_open_and_a_send_not_for_the_person_stays_quiet() {
    // Ada and Bob work in their workspaces, and Cy has the whole machine,
    // more than Ada: her items to Cy wait on the person's card.
    let log = scratch("asked-held");
    enrol(&log, &named("ada", "Ada", None));
    enrol(&log, &named("bob", "Bob", None));
    enrol(&log, &named("cy", "Cy", Some(Reach::Machine)));
    let agents = crate::session::tests::Fake::new(replies());
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
    at_rest(&room, "ada").await;
    let asked = ask(&room, &["bob", "cy"], "Who has the keys?").await;
    assert!(item(&room, "cy", &asked[1]).held);
    answer(&room, "bob", &asked[0], "I do.").await;
    until("Ada woke", || !adas_wakes(&agents).is_empty()).await;
    assert!(adas_wakes(&agents)[0].contains(QUIET_WAKE_LINE));
    until("Ada's turn ended", || idle(&room, "ada")).await;

    let unasked = send(&room, "ada", "bob", "Lunch?", json!({})).await;
    assert!(!item(&room, "bob", &unasked).for_person);
    answer(&room, "bob", &unasked, "Noon.").await;
    let lunch = || {
        adas_wakes(&agents)
            .into_iter()
            .find(|told| told.contains("\"Re: Lunch?\""))
    };
    until("Ada woke for Lunch", || lunch().is_some()).await;
    assert!(lunch().unwrap().contains(QUIET_WAKE_LINE));
    assert!(
        agents.told().iter().all(|told| !told.contains("Finished:")),
        "nothing finished"
    );
}

/// The person's Ask a follow-up starts a turn they read, so what the
/// teammate sends on from it is theirs.
#[tokio::test]
async fn the_persons_follow_up_makes_the_send_it_starts_theirs() {
    let gate = Arc::new(Semaphore::new(0));
    let (room, _) = room_on("asked-follow-up", replies().gated(gate.clone()));
    at_rest(&room, "ada").await;
    let first = send(
        &room,
        "ada",
        "bob",
        "Who do the Devils play next?",
        json!({}),
    )
    .await;
    assert!(
        !item(&room, "bob", &first).for_person,
        "sent outside a turn"
    );
    room.prompt_about(
        "ada",
        "What was the score from the last game?",
        None,
        Some(first.clone()),
        None,
    )
    .await
    .unwrap();
    let main = ThreadId::dm("ada");
    until("the follow-up's turn is open", || room.is_read_turn(&main)).await;
    let onward = send(
        &room,
        "ada",
        "bob",
        "Final score of the last game?",
        json!({"reply_to": first}),
    )
    .await;
    assert!(item(&room, "bob", &onward).for_person);
    gate.add_permits(1_000);
}

/// What finishes while the person's own turn is in flight is said into that
/// turn, and told: no later turn tells it again.
#[tokio::test]
async fn an_ask_that_finishes_mid_turn_is_said_into_the_turn_the_person_reads() {
    let gate = Arc::new(Semaphore::new(0));
    let (room, agents) = room_on("asked-mid-turn", replies().gated(gate.clone()).steerable());
    at_rest(&room, "ada").await;
    room.prompt("ada", "Ask Bob the score.", None, None)
        .await
        .unwrap();
    let main = ThreadId::dm("ada");
    until("the person's turn is open", || room.is_read_turn(&main)).await;
    let asked = send(&room, "ada", "bob", "Score?", json!({})).await;
    answer(&room, "bob", &asked, "7–4.").await;
    until("said into the turn", || {
        agents.steered().iter().any(|line| {
            line.contains("is finished, \"Score?\" (Bob: answered)")
                && line.contains("Tell the person in your own words")
        })
    })
    .await;
    assert!(room.finished_asks("ada").is_empty(), "told");
    gate.add_permits(1_000);
}
