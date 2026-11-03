//! Threads, asides and files across a link (`docs/design/thread-awareness.md`
//! §3): a work thread on one desk informs the teammate on the other that
//! handed it over, a thread is named across the link as `deskId/side:<id>`,
//! asides and files cross under the receiving desk's grant, an older desk
//! only loses asides, and the person's Stop crosses as theirs.

use super::*;
use crate::contract::{Aside, AsideLevel, AsideSource, LinkedFile, ThreadId};
use crate::session::peers::HANDOFF_STOPPED_BY_THE_PERSON;
use tempfile::TempDir;
use tokio::sync::Semaphore;

fn asides(room: &Room, persona_id: &str) -> Vec<Aside> {
    crate::store::asides::of(&room.log, persona_id)
}

/// The asides `persona_id` was left by `by`, but for the nudge a handoff's
/// result leaves beside it, which the sender's own desk leaves.
fn asides_by(room: &Room, persona_id: &str, by: &str) -> Vec<Aside> {
    asides(room, persona_id)
        .into_iter()
        .filter(|aside| aside.by.as_deref() == Some(by) && !aside.id.starts_with("result:"))
        .collect()
}

fn clementine_on_laptop() -> String {
    format!("{LAPTOP}/clementine")
}

fn lou_on_grizzly() -> String {
    format!("{GRIZZLY}/lou")
}

async fn tool(room: &Arc<Room>, from: &str, name: &str, arguments: Value) -> Result<Value, String> {
    let answer = TeammateTools::new(room, from)
        .call(name, &arguments)
        .await?;
    Ok(serde_json::from_str(&answer).unwrap_or(Value::String(answer)))
}

/// Lou's work thread on grizzly for Clementine's handoff, once it has one.
fn lou_thread(grizzly: &Room, handoff: &str) -> String {
    request(grizzly, handoff).unwrap()["thread"]
        .as_str()
        .expect("the handoff opened a work thread on grizzly")
        .to_string()
}

/// A teammate working in a directory of its own, with workspace reach, so
/// what it may read is that directory alone.
fn in_own_workspace(room: &Room, persona_id: &str) -> TempDir {
    let dir = TempDir::new().unwrap();
    let mut persona = room.persona(persona_id).unwrap();
    persona.cwd = dir.path().to_string_lossy().into_owned();
    persona.reach = Some(crate::contract::Reach::Workspace);
    crate::room::append_persona(&room.log, &persona).unwrap();
    dir
}

/// Lou's work thread on grizzly tells Clementine, who handed it over, what
/// each of its turns that is not the result came to: a note aside on the
/// laptop, from that thread as grizzly names it, in Lou's words and never
/// the person's, and nothing in Clementine's chat. Clementine names the
/// thread back the same way: a handoff into it continues it there, and an
/// aside about it is heard in it there.
#[tokio::test]
async fn a_work_threads_note_crosses_as_an_aside_and_its_thread_is_named_back_across_the_link() {
    let (laptop, grizzly) = pair("named-across");
    let _channel = Channel::open(&laptop, &grizzly);
    grizzly
        .allow_sender("lou", &clementine_on_laptop())
        .unwrap();
    let first = message(&laptop, "clementine", "Lou", "handoff")
        .await
        .unwrap();
    until("the first result", || {
        delivered(&laptop, "clementine", &first).is_some()
    })
    .await;
    let thread = lou_thread(&grizzly, &first);
    assert!(
        asides_by(&laptop, "clementine", &lou_on_grizzly()).is_empty(),
        "the result is a delivery and the laptop's own nudge, not a note from there"
    );

    // The person on grizzly talks in Lou's thread.
    grizzly.continue_side(&thread).await.unwrap();
    grizzly
        .prompt_side(&thread, "Make it darker", None)
        .await
        .unwrap();
    until("Lou's note reaches Clementine", || {
        !asides_by(&laptop, "clementine", &lou_on_grizzly()).is_empty()
    })
    .await;
    let notes = asides_by(&laptop, "clementine", &lou_on_grizzly());
    assert_eq!(notes.len(), 1, "{notes:?}");
    let note = &notes[0];
    assert_eq!(note.level, AsideLevel::Note);
    let named = ThreadId::side(&thread).on_desk(GRIZZLY);
    assert_eq!(note.from, AsideSource::Thread(named.clone()));
    assert_eq!(named.to_string(), format!("{GRIZZLY}/side:{thread}"));
    assert_eq!(ThreadId::parse(&named.to_string()), Some(named.clone()));
    assert!(note.text.starts_with("runner "), "{}", note.text);
    assert!(!note.text.contains("Make it darker"), "{}", note.text);
    assert!(
        laptop
            .tape("clementine")
            .iter()
            .all(|event| !event.to_string().contains(&note.text)),
        "nothing of it is in Clementine's chat"
    );
    assert!(
        request(&grizzly, &first).unwrap().get("crossing").is_none(),
        "sent, and let go of"
    );

    // A handoff naming that thread as grizzly does continues it there.
    let sent = tool(
        &laptop,
        "clementine",
        "message_teammate",
        json!({"to": "Lou", "message": "And the staging runners.", "intent": "handoff",
               "thread": named.to_string()}),
    )
    .await
    .unwrap();
    let second = sent["requestId"].as_str().unwrap().to_string();
    until("the second result", || {
        delivered(&laptop, "clementine", &second).is_some()
    })
    .await;
    assert_eq!(lou_thread(&grizzly, &second), thread);

    // An aside about it is heard in it, on grizzly.
    let left = tool(
        &laptop,
        "clementine",
        "aside",
        json!({"to": "Lou", "text": "Staging is the one that matters.", "about": named.to_string()}),
    )
    .await
    .unwrap();
    assert_eq!(
        left,
        "Left Lou on grizzly an aside. It waits for their next turn and is not said to the person."
    );
    let heard = asides_by(&grizzly, "lou", &clementine_on_laptop());
    assert_eq!(heard.len(), 1, "{heard:?}");
    assert_eq!(heard[0].about, Some(ThreadId::side(&thread)));
    assert_eq!(
        heard[0].from,
        AsideSource::Thread(ThreadId::dm("clementine").on_desk(LAPTOP))
    );
    assert_eq!(heard[0].text, "Staging is the one that matters.");
    // The thread a teammate on the other desk does not have is refused there.
    let refused = tool(
        &laptop,
        "clementine",
        "aside",
        json!({"to": "Lou", "text": "x", "about": format!("{GRIZZLY}/side:nowhere")}),
    )
    .await
    .unwrap_err();
    assert_eq!(refused, "side:nowhere is not a thread of Lou's.");
    let refused = tool(
        &laptop,
        "clementine",
        "message_teammate",
        json!({"to": "Lou", "message": "More.", "intent": "handoff",
               "thread": format!("{GRIZZLY}/side:nowhere")}),
    )
    .await
    .unwrap();
    let refused = refused["requestId"].as_str().unwrap().to_string();
    until("the unknown thread is refused there", || {
        delivered(&laptop, "clementine", &refused).is_some()
    })
    .await;
    assert_eq!(
        delivered(&laptop, "clementine", &refused).unwrap()["text"],
        "That work thread was not opened by this teammate's handoff."
    );
}

/// An aside crosses only under the grant an ask from its sender needs on the
/// receiving desk, and never raises the person's card there: without it,
/// nothing is left and the sender is told why.
#[tokio::test]
async fn an_aside_crosses_only_under_the_grant_an_ask_needs_there_and_raises_no_card() {
    let (laptop, grizzly) = pair("aside-grant");
    let _channel = Channel::open(&laptop, &grizzly);
    let refused = tool(
        &laptop,
        "clementine",
        "aside",
        json!({"to": "Lou", "text": "The numbers are in.", "level": "nudge"}),
    )
    .await
    .unwrap_err();
    assert_eq!(
        refused,
        "Clementine on laptop has not been allowed to ask Lou here, so the aside was not left. message_teammate asks the person there first."
    );
    assert!(collaboration_card(&grizzly, "lou").is_none());
    assert_eq!(asides(&grizzly, "lou").len(), 0);

    grizzly
        .allow_sender("lou", &clementine_on_laptop())
        .unwrap();
    tool(
        &laptop,
        "clementine",
        "aside",
        json!({"to": "Lou", "text": "The numbers are in."}),
    )
    .await
    .unwrap();
    let kept = asides(&grizzly, "lou");
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].by.as_deref(), Some(clementine_on_laptop().as_str()));
    assert!(
        kept[0].id.starts_with(&format!("{LAPTOP}:")),
        "{}",
        kept[0].id
    );
    // The sender's record shows what it said, to whom.
    assert!(
        laptop
            .tape("clementine")
            .iter()
            .any(|event| event["kind"] == "aside" && event["to"] == lou_on_grizzly())
    );
    // The same aside again is the one kept, never a second.
    let again = LinkedDelivery::Aside {
        id: kept[0].id.trim_start_matches(&format!("{LAPTOP}:")).into(),
        from_persona_id: "clementine".into(),
        to: "lou".into(),
        from: ThreadId::dm("clementine"),
        text: "The numbers are in.".into(),
        level: AsideLevel::Note,
        about: None,
        request_id: None,
        files: Vec::new(),
    };
    grizzly.federation_deliver(LAPTOP, again).unwrap();
    assert_eq!(asides(&grizzly, "lou").len(), 1);
}

/// The laptop is older: its roster says nothing of asides. Messages and
/// results still cross both ways; what Lou's thread would tell Clementine
/// is dropped on grizzly, and Lou's own aside is refused with why.
#[tokio::test]
async fn an_older_desk_keeps_its_messages_and_only_loses_asides() {
    let (laptop, grizzly) = pair("older");
    let _ends = [
        pump_saying(&laptop, &grizzly, LAPTOP, GRIZZLY, &[]),
        pump(&grizzly, &laptop, GRIZZLY, LAPTOP),
    ];
    grizzly
        .allow_sender("lou", &clementine_on_laptop())
        .unwrap();
    let handoff = message(&laptop, "clementine", "Lou", "handoff")
        .await
        .unwrap();
    until("the handoff's result", || {
        delivered(&laptop, "clementine", &handoff).is_some()
    })
    .await;
    let thread = lou_thread(&grizzly, &handoff);
    grizzly.continue_side(&thread).await.unwrap();
    grizzly
        .prompt_side(&thread, "Make it darker", None)
        .await
        .unwrap();
    until("Lou's turn is over and its note let go of", || {
        !grizzly.mid_turn("lou")
            && grizzly
                .log
                .load(&StreamId::Side(thread.clone()))
                .iter()
                .filter(|v| v["kind"] == "agent")
                .count()
                >= 2
            && request(&grizzly, &handoff)
                .unwrap()
                .get("crossing")
                .is_none()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(asides_by(&laptop, "clementine", &lou_on_grizzly()).len(), 0);

    let refused = tool(
        &grizzly,
        "lou",
        "aside",
        json!({"to": "Clementine", "text": "Done on my side."}),
    )
    .await
    .unwrap_err();
    assert_eq!(
        refused,
        "Clementine on laptop's desk is older and does not take asides, so nothing was left. Use message_teammate."
    );
    // A message still crosses.
    let ask = message(&laptop, "clementine", "Lou", "ask").await.unwrap();
    until("the ask's answer", || {
        delivered(&laptop, "clementine", &ask).is_some()
    })
    .await;
}

/// Files cross the link with the message or aside that carries them, into
/// the recipient's working directory there once it is let in; the message
/// names where. More than the link's cap is refused before anything is sent.
#[tokio::test]
async fn files_cross_the_link_into_the_recipients_inbox_and_over_the_cap_are_refused() {
    let (laptop, grizzly) = pair("files-across");
    let _channel = Channel::open(&laptop, &grizzly);
    let theirs = in_own_workspace(&laptop, "clementine");
    let lous = in_own_workspace(&grizzly, "lou");
    std::fs::write(theirs.path().join("q3.csv"), "quarter,total\nq3,12").unwrap();
    let id = tool(
        &laptop,
        "clementine",
        "message_teammate",
        json!({"to": "Lou", "message": "The Q3 numbers.", "files": ["q3.csv"]}),
    )
    .await
    .unwrap()["requestId"]
        .as_str()
        .unwrap()
        .to_string();
    until("the card on grizzly", || {
        collaboration_card(&grizzly, "lou").is_some()
    })
    .await;
    let landed = lous.path().join("inbox/clementine@laptop-desk/q3.csv");
    assert!(
        !landed.exists(),
        "nothing lands before the person lets it in"
    );
    let card = collaboration_card(&grizzly, "lou").unwrap();
    grizzly
        .answer_permission("lou", card["requestId"].as_str().unwrap(), "allow_session")
        .await
        .unwrap();
    until("the answer", || {
        delivered(&laptop, "clementine", &id).is_some()
    })
    .await;
    assert_eq!(
        std::fs::read_to_string(&landed).unwrap(),
        "quarter,total\nq3,12"
    );
    let asked = grizzly
        .tape("lou")
        .into_iter()
        .find(|event| event["cause"]["requestId"] == id.as_str())
        .expect("Lou was asked");
    assert!(
        asked["text"]
            .as_str()
            .unwrap()
            .ends_with("Files from Clementine on laptop, now in your working directory: inbox/clementine@laptop-desk/q3.csv"),
        "{asked}"
    );
    assert!(
        !laptop.log.root().join("files-between").join(&id).exists(),
        "the laptop let go of what it sent"
    );

    // An aside's files land the same way.
    tool(
        &laptop,
        "clementine",
        "aside",
        json!({"to": "Lou", "text": "Again, for the record.", "files": ["q3.csv"]}),
    )
    .await
    .unwrap();
    assert!(
        lous.path()
            .join("inbox/clementine@laptop-desk/q3 (2).csv")
            .exists()
    );

    // Over the link's cap: refused here with why, and nothing queued.
    std::fs::write(theirs.path().join("big.bin"), vec![0u8; 600 * 1024]).unwrap();
    let before = requests(&laptop).len();
    let refused = tool(
        &laptop,
        "clementine",
        "message_teammate",
        json!({"to": "Lou", "message": "The raw dump.", "files": ["big.bin"]}),
    )
    .await
    .unwrap_err();
    assert!(
        refused.starts_with("Files to a teammate on a linked desk come to at most 512 KB together"),
        "{refused}"
    );
    assert_eq!(requests(&laptop).len(), before);
    // And there, whatever the other desk sends.
    let forged = LinkedDelivery::Aside {
        id: "forged".into(),
        from_persona_id: "clementine".into(),
        to: "lou".into(),
        from: ThreadId::dm("clementine"),
        text: "Here.".into(),
        level: AsideLevel::Note,
        about: None,
        request_id: None,
        files: vec![LinkedFile {
            name: "../../escape".into(),
            data: "eA==".into(),
        }],
    };
    assert_eq!(
        grizzly.federation_deliver(LAPTOP, forged).unwrap_err(),
        "Each file needs a plain name of its own."
    );
}

/// The person on grizzly stops Lou's turn on Clementine's handoff: it
/// crosses as their stop, with what Lou had said, and the laptop ends the
/// handoff as theirs and tells Clementine once by a note aside, with nothing
/// delivered to wake her.
#[tokio::test]
async fn the_persons_stop_on_a_linked_handoff_reaches_its_sender_as_a_note_without_waking_it() {
    let gate = Arc::new(Semaphore::new(0));
    let lou = Fake::new(
        Scripted::new(vec![
            Update::Message {
                kind: MessageKind::Agent,
                id: "halfway".into(),
                text: "Halfway through the runners.".into(),
            },
            Update::Turn {
                stop_reason: "end_turn".into(),
                usage: None,
            },
        ])
        .on_cancel(vec![Update::Turn {
            stop_reason: "cancelled".into(),
            usage: None,
        }])
        .gated(gate.clone()),
    );
    let laptop = desk(
        "stop-across-laptop",
        ("clementine", "Clementine"),
        (GRIZZLY, "grizzly"),
        true,
    );
    let grizzly = desk_with(
        "stop-across-grizzly",
        ("lou", "Lou"),
        (LAPTOP, "laptop"),
        false,
        lou.clone(),
    );
    let _channel = Channel::open(&laptop, &grizzly);
    grizzly
        .allow_sender("lou", &clementine_on_laptop())
        .unwrap();
    let handoff = message(&laptop, "clementine", "Lou", "handoff")
        .await
        .unwrap();
    until("Lou is at work on it", || lou.prompts().len() == 1).await;
    let thread = lou_thread(&grizzly, &handoff);
    gate.add_permits(1);
    until("Lou has said where he is", || {
        grizzly
            .log
            .load(&StreamId::Side(thread.clone()))
            .iter()
            .any(|v| v["text"] == "Halfway through the runners.")
    })
    .await;
    grizzly.cancel_side(&thread).unwrap();
    let told = format!("Halfway through the runners.\n\n{HANDOFF_STOPPED_BY_THE_PERSON}");
    until("the laptop has ended it as the person's", || {
        request(&laptop, &handoff).is_some_and(|r| r["phase"] == "stopped")
    })
    .await;
    let ended = request(&laptop, &handoff).unwrap();
    assert_eq!(ended["stopped_by_person"], true);
    assert_eq!(ended["released"], true);
    let notes = asides_by(&laptop, "clementine", &lou_on_grizzly());
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].level, AsideLevel::Note);
    assert_eq!(notes[0].text, told);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        delivered(&laptop, "clementine", &handoff).is_none(),
        "no failure to retry, and nothing that wakes her"
    );
    assert_eq!(request(&grizzly, &handoff).unwrap()["returned"], true);
    assert_eq!(asides_by(&laptop, "clementine", &lou_on_grizzly()).len(), 1);
}

/// The stop crosses whichever way the two desks' records stand: the laptop
/// still holding the handoff as queued, its forward's answer lost, ends it
/// as the person's all the same; and grizzly, its stop not yet crossed,
/// takes the same request sent again as the one it has, not a refusal.
#[tokio::test]
async fn the_persons_stop_reaches_a_sender_still_holding_its_request_as_queued() {
    let (laptop, grizzly) = pair("stop-queued");
    let channel = Channel::open(&laptop, &grizzly);
    channel.close();
    let handoff = message(&laptop, "clementine", "Lou", "handoff")
        .await
        .unwrap();
    assert_eq!(request(&laptop, &handoff).unwrap()["phase"], "queued");
    laptop
        .federation_deliver(
            GRIZZLY,
            LinkedDelivery::Result {
                request_id: handoff.clone(),
                reply: format!("Started.\n\n{HANDOFF_STOPPED_BY_THE_PERSON}"),
                failed: true,
                stopped_by_person: true,
            },
        )
        .unwrap();
    let ended = request(&laptop, &handoff).unwrap();
    assert_eq!(ended["phase"], "stopped");
    assert_eq!(ended["stopped_by_person"], true);
    assert_eq!(
        asides_by(&laptop, "clementine", &lou_on_grizzly())[0].text,
        format!("Started.\n\n{HANDOFF_STOPPED_BY_THE_PERSON}")
    );
    // A stop is never taken for a request this desk did not send that desk.
    assert_eq!(
        laptop
            .federation_deliver(
                GRIZZLY,
                LinkedDelivery::Result {
                    request_id: "never-sent".into(),
                    reply: "x".into(),
                    failed: true,
                    stopped_by_person: true,
                },
            )
            .unwrap_err(),
        "This desk sent no such request."
    );
    // Nothing is forwarded once the channel is back.
    let _channel = Channel::open(&laptop, &grizzly);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(request(&grizzly, &handoff).is_none());

    // Grizzly holds a request its person stopped, the stop not yet crossed.
    let key = thread_key(&clementine_on_laptop(), "lou").unwrap();
    grizzly
        .log
        .append(
            &StreamId::Room,
            &json!({"kind": "exchange_pair", "id": key, "a": clementine_on_laptop(), "b": "lou",
                "exchanges": 1, "paused": false, "requests": [{
                    "id": "held", "from": clementine_on_laptop(), "to": "lou",
                    "message": "Check the CI runners.", "intent": "handoff", "phase": "stopped",
                    "reply": HANDOFF_STOPPED_BY_THE_PERSON, "failed": true, "released": true,
                    "stopped_by_person": true, "returned": false}]}),
        )
        .unwrap();
    let again = LinkedDelivery::Request {
        request_id: "held".into(),
        from_persona_id: "clementine".into(),
        to: "lou".into(),
        message: "Check the CI runners.".into(),
        intent: "handoff".into(),
        continues: None,
        about: None,
        files: Vec::new(),
    };
    assert_eq!(
        grizzly.federation_deliver(LAPTOP, again).unwrap(),
        json!({"accepted": true})
    );
}

/// A question back from Lou's thread to Clementine, who handed it over,
/// reaches her on the laptop with no card while the handoff stands, found
/// by its id in the laptop's own record. The same id said of another
/// teammate's question, or a note on a handoff the laptop never sent, is
/// not honoured.
#[tokio::test]
async fn a_linked_desk_asks_back_and_notes_only_by_this_desks_own_handoffs() {
    let (laptop, grizzly) = pair("ask-back-across");
    let mut dot = persona("dot");
    dot.name = "Dot".into();
    enrol(&laptop.log, &dot);
    let _channel = Channel::open(&laptop, &grizzly);
    grizzly
        .allow_sender("lou", &clementine_on_laptop())
        .unwrap();
    let handoff = message(&laptop, "clementine", "Lou", "handoff")
        .await
        .unwrap();
    until("the handoff's result", || {
        delivered(&laptop, "clementine", &handoff).is_some()
    })
    .await;
    let thread = lou_thread(&grizzly, &handoff);
    grizzly.continue_side(&thread).await.unwrap();

    let asked = TeammateTools::new(&grizzly, "lou")
        .for_work(&thread)
        .call(
            "message_teammate",
            &json!({"to": "Clementine", "message": "Which runners?"}),
        )
        .await
        .unwrap();
    let asked = serde_json::from_str::<Value>(&asked).unwrap()["requestId"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        request(&grizzly, &asked).unwrap()["about"],
        handoff.as_str()
    );
    until("the laptop has the question", || {
        request(&laptop, &asked).is_some()
    })
    .await;
    assert_eq!(request(&laptop, &asked).unwrap()["about"], handoff.as_str());
    until("Clementine is asked", || {
        laptop
            .tape("clementine")
            .iter()
            .any(|event| event["cause"]["requestId"] == asked.as_str())
    })
    .await;
    assert!(
        collaboration_card(&laptop, "clementine").is_none(),
        "no card while the handoff stands"
    );

    // Said of Dot, whom the handoff was not from, it meets Dot's card.
    laptop
        .federation_deliver(
            GRIZZLY,
            LinkedDelivery::Request {
                request_id: "about-another".into(),
                from_persona_id: "lou".into(),
                to: "dot".into(),
                message: "Which runners?".into(),
                intent: "ask".into(),
                continues: None,
                about: Some(handoff.clone()),
                files: Vec::new(),
            },
        )
        .unwrap();
    until("Dot's person is asked first", || {
        collaboration_card(&laptop, "dot").is_some()
    })
    .await;

    // A note on a handoff the laptop never sent grizzly is refused.
    let note = |request_id: &str| LinkedDelivery::Aside {
        id: format!("note-{request_id}"),
        from_persona_id: "lou".into(),
        to: "clementine".into(),
        from: ThreadId::side(&thread),
        text: "Done.".into(),
        level: AsideLevel::Person,
        about: None,
        request_id: Some(request_id.into()),
        files: Vec::new(),
    };
    assert_eq!(
        laptop
            .federation_deliver(GRIZZLY, note("never-sent"))
            .unwrap_err(),
        "This desk sent no such handoff."
    );
    // One on its own handoff is heard as a note, whatever level it says.
    laptop.federation_deliver(GRIZZLY, note(&handoff)).unwrap();
    let kept = asides(&laptop, "clementine")
        .into_iter()
        .find(|aside| aside.id == format!("{GRIZZLY}:note-{handoff}"))
        .unwrap();
    assert_eq!(kept.level, AsideLevel::Note);
}
