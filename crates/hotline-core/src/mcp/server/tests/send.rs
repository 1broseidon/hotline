//! `send`'s description: the two ways a teammate writes to a colleague.

use super::*;

/// A quick line reads like a chat and a hand-off like a letter, and the
/// description says both, with the title left out of the quick one and an
/// answer sent back on `reply_to`. The schema asks for `to` alone.
#[test]
fn the_send_description_says_a_quick_line_and_a_hand_off() {
    let tools = listing(descriptors()).tools;
    let send = tools.iter().find(|tool| tool.name == SEND).unwrap();
    let said = send.description.as_deref().unwrap();
    for words in [
        "A quick question or answer is one short line in `message` and no `title`; the person reads it like a chat.",
        "A hand-off is a `title` and a full brief in `message`; the person reads it like a letter.",
        "Answer a quick question with a quick line, `reply_to` its id: that closes it as answered.",
        "in their inbox, or not sent.",
        "the call waits up to 45 seconds for their answer and says what it was.",
    ] {
        assert!(said.contains(words), "{words}: {said}");
    }
    assert_eq!(send.input_schema["required"], serde_json::json!(["to"]));
}

/// A `send` waiting on the person's first-contact card ends when the agent
/// cancels the call, as an ACP agent does when its turn is stopped or
/// interrupted: rmcp's client sends the cancellation when the call is
/// dropped, and the wait lets go of the desk at once, while the card stays
/// up for the person and the item stays held for it.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_send_lets_go_of_its_wait_and_leaves_the_card() {
    let room = room_with_a_conversation("send-cancelled");
    // Bob has the whole machine, more than Ada's own folders: a card asks.
    write_persona(
        room.log(),
        Persona {
            reach: Some(Reach::Machine),
            ..bob()
        },
    );
    let served = serve(tools(&room)).await.unwrap();
    let connected = client()
        .serve(transport(served.url(), served.token()))
        .await
        .unwrap();
    let peer = connected.peer().clone();
    let calling = tokio::spawn(async move {
        peer.call_tool_once(
            CallToolRequestParams::new(SEND).with_arguments(
                json!({ "to": ["bob"], "title": "Psst", "message": "the secret" })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
    });
    // Up, and not decided: a decision is the card written again under its id.
    let open_card = || {
        let cards: Vec<Value> = room
            .log()
            .load(&StreamId::Tape("ada".into()))
            .into_iter()
            .filter(|event| event["kind"] == "permission")
            .collect();
        !cards.is_empty() && cards.iter().all(|card| card["decision"].is_null())
    };
    crate::test_support::until("the card is up", open_card).await;
    assert!(
        room.prepare_restart().is_err(),
        "the send is still working, waiting on the card"
    );

    calling.abort();
    crate::test_support::until("the wait lets go", || room.prepare_restart().is_ok()).await;
    assert!(open_card(), "the card is still the person's");
    let held = crate::store::inbox::of(room.log(), "bob");
    assert_eq!(held.len(), 1);
    assert!(held[0].held);
    connected.cancel().await.ok();
}

/// The lead's voice front is shown its own `send`: a titled job with the
/// person's words, `wants` fixed to work, and none of the quick lines,
/// answers or follow-ups its call's grant would refuse. A front with no
/// lead call is shown no `send`, and every other teammate's is the general
/// one.
#[test]
fn the_leads_front_is_shown_send_as_a_titled_job_and_nothing_else() {
    let front = TeammateTools::front_descriptors(true);
    let (_, said, schema) = front.iter().find(|(name, _, _)| name == SEND).unwrap();
    assert!(
        said.starts_with("Hand a named teammate a job from this call"),
        "{said}"
    );
    assert!(said.contains("the person's words unchanged as the message"));
    assert!(said.contains("a question, a note or a check for a colleague goes through work_on_it"));
    for general in ["reply_to", "quick", "`answer`", "nothing`"] {
        assert!(!said.contains(general), "{general}: {said}");
    }
    assert_eq!(schema["properties"]["wants"]["enum"], json!(["work"]));
    assert_eq!(
        schema["required"],
        json!(["to", "title", "message", "wants"])
    );
    for general in ["reply_to", "waiting", "about", "files"] {
        assert!(schema["properties"].get(general).is_none(), "{general}");
    }
    assert!(
        !TeammateTools::front_descriptors(false)
            .iter()
            .any(|(name, _, _)| name == SEND)
    );
    let tools = listing(descriptors()).tools;
    let general = tools.iter().find(|tool| tool.name == SEND).unwrap();
    assert_ne!(general.description.as_deref(), Some(said.as_str()));
    assert_eq!(
        general.input_schema["properties"]["wants"]["enum"],
        json!(["answer", "work", "nothing"])
    );
}
