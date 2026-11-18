//! Topics (`docs/design/agent-inbox.md`, Topics): one send is one topic,
//! answers and follow-ups stay in it, an item kept before topics were
//! recorded is given one as it is read, a one-line ask is its own title,
//! and the send result says where each recipient's item landed.

use super::*;

/// Ada and Bob as `room` makes them, and Cy, whom neither has been
/// allowed to reach.
fn three(name: &str) -> (Arc<Room>, Arc<crate::session::tests::Fake>) {
    let (room, agents) = room(name);
    enrol(&room.log, &named("cy", "Cy", Some(Reach::Machine)));
    (room, agents)
}

/// One `send` to several shares one send id, which is the topic of every
/// item it made; an answer and a follow-up on it, from either side, are in
/// the same topic.
#[tokio::test]
async fn one_send_is_one_topic_and_its_answers_and_follow_ups_stay_in_it() {
    let (room, _) = room("topic-one-send");
    enrol(&room.log, &named("cy", "Cy", Some(Reach::Machine)));
    room.allow_sender("cy", "ada").unwrap();
    let sent = tool(
        &room,
        "ada",
        "send",
        json!({"to": ["Bob", "Cy"], "title": "Storage numbers", "message": "Run the bench."}),
    )
    .await
    .unwrap();
    let id = |n: usize| sent["sent"][n]["id"].as_str().unwrap().to_string();
    let (to_bob, to_cy) = (item(&room, "bob", &id(0)), item(&room, "cy", &id(1)));
    let send_id = to_bob.send_id.clone().expect("a send keeps its send id");
    assert_eq!(to_cy.send_id.as_deref(), Some(send_id.as_str()));
    assert_eq!(to_bob.topic.as_deref(), Some(send_id.as_str()));
    assert_eq!(to_cy.topic.as_deref(), Some(send_id.as_str()));

    tool(
        &room,
        "bob",
        "update_inbox",
        json!({"items": [{"id": id(0), "answer": "530 MB before, 341 after."}]}),
    )
    .await
    .unwrap();
    let answer = items(&room, "ada")
        .into_iter()
        .find(|reply| reply.reply_to.as_deref() == Some(id(0).as_str()))
        .unwrap();
    assert_eq!(answer.topic.as_deref(), Some(send_id.as_str()));
    assert_eq!(answer.send_id, None, "an answer is no send");

    let follow = send(
        &room,
        "ada",
        "bob",
        "Which machine?",
        json!({"reply_to": short(&answer.id)}),
    )
    .await;
    let follow = item(&room, "bob", &follow);
    assert_eq!(follow.topic.as_deref(), Some(send_id.as_str()));
    assert_ne!(follow.send_id.as_deref(), Some(send_id.as_str()));

    // A send of its own starts a topic of its own.
    let other = send(&room, "ada", "bob", "Lunch?", json!({})).await;
    let other = item(&room, "bob", &other);
    assert_eq!(other.topic, other.send_id);
    assert_ne!(other.topic.as_deref(), Some(send_id.as_str()));
}

/// An item written before topics and send ids were recorded is read with
/// the ones it would have had, and its line is left as it was: one send's
/// items share the first one's id, an answer takes its question's topic,
/// and an item alone is its own topic.
#[tokio::test]
async fn an_item_kept_before_topics_is_given_one_from_its_chain_as_it_is_read() {
    let (room, _) = three("topic-old");
    let old = |id: &str, from: &str, to: &str, ts: i64, extra: Value| {
        let mut line = json!({"kind": kept::KIND, "id": id, "ts": ts, "from": from,
            "fromThread": {"kind": "dm", "key": from}, "to": to, "title": "Storage numbers",
            "body": "", "wants": "answer", "state": "seen", "receipts": [], "senderTold": 0});
        if let Value::Object(fields) = extra {
            for (key, value) in fields {
                line[key] = value;
            }
        }
        room.log.append(&StreamId::Inbox(to.into()), &line).unwrap();
    };
    old("old-bob", "ada", "bob", 1_000, json!({"alsoTo": ["cy"]}));
    old("old-cy", "ada", "cy", 1_003, json!({"alsoTo": ["bob"]}));
    old(
        "old-answer",
        "bob",
        "ada",
        2_000,
        json!({"replyTo": "old-bob", "title": "Re: Storage numbers"}),
    );
    old("old-alone", "ada", "bob", 3_000, json!({"title": "Lunch?"}));
    // The same title to the same two, long after: another send.
    old("old-again", "ada", "bob", 60_000, json!({"alsoTo": ["cy"]}));

    let view = crate::store::inbox::view(&room.log, "ada", ["ada", "bob", "cy"]);
    let of = |id: &str| {
        view.sent
            .iter()
            .chain(&view.open)
            .find(|item| item.id == id)
            .cloned()
            .unwrap()
    };
    assert_eq!(of("old-bob").send_id.as_deref(), Some("old-bob"));
    assert_eq!(of("old-cy").send_id.as_deref(), Some("old-bob"));
    assert_eq!(of("old-cy").topic.as_deref(), Some("old-bob"));
    assert_eq!(of("old-answer").topic.as_deref(), Some("old-bob"));
    assert_eq!(of("old-answer").send_id, None);
    assert_eq!(of("old-alone").topic.as_deref(), Some("old-alone"));
    assert_eq!(of("old-again").topic.as_deref(), Some("old-again"));
    assert_eq!(
        item(&room, "cy", "old-cy").topic,
        None,
        "nothing is written back"
    );

    // A follow-up on an old item lands in the topic it was given.
    let follow = send(
        &room,
        "bob",
        "ada",
        "And Linux?",
        json!({"reply_to": "old-bob"}),
    )
    .await;
    assert_eq!(
        item(&room, "ada", &follow).topic.as_deref(),
        Some("old-bob")
    );
}

/// `send` without a title: a one-line message is the title and says
/// nothing more, so it is shown once; a longer one keeps every word, under
/// its first plain line.
#[tokio::test]
async fn send_without_a_title_takes_its_first_plain_line() {
    let (room, _) = room("topic-untitled");
    let sent = |message: &str| {
        let room = room.clone();
        let message = message.to_string();
        async move {
            let sent = tool(
                &room,
                "ada",
                "send",
                json!({"to": ["bob"], "message": message}),
            )
            .await?;
            Ok::<InboxItem, String>(item(&room, "bob", sent["sent"][0]["id"].as_str().unwrap()))
        }
    };
    let quick = sent("What was the score?").await.unwrap();
    assert_eq!(quick.title, "What was the score?");
    assert_eq!(quick.body, "", "said once");
    assert_eq!(quick.wants, InboxWants::Answer);

    let brief = sent("## Check the notes\n\n- the storage numbers\n- the links")
        .await
        .unwrap();
    assert_eq!(brief.title, "Check the notes");
    assert_eq!(
        brief.body,
        "## Check the notes\n\n- the storage numbers\n- the links"
    );

    let long = "word ".repeat(40);
    let cut = sent(&long).await.unwrap();
    assert!(cut.title.chars().count() <= TITLE_CHARS, "{}", cut.title);
    assert!(cut.title.ends_with('…'), "{}", cut.title);
    assert_eq!(cut.body, long.trim(), "every word kept");

    let nothing = sent("   ").await.unwrap_err();
    assert!(nothing.contains("needs a `message`"), "{nothing}");
}

/// The send result says where each recipient's item landed, in the words
/// the desk uses: in their inbox, or awaiting the person's approval, and
/// never that a held one was received.
#[tokio::test]
async fn the_send_result_says_where_each_recipients_item_landed() {
    // Ada and Bob work in their workspaces, and Cy has the whole machine,
    // more than Ada: her items to Cy wait on the person's card.
    let log = scratch("topic-delivery");
    enrol(&log, &named("ada", "Ada", None));
    enrol(&log, &named("bob", "Bob", None));
    enrol(&log, &named("cy", "Cy", Some(Reach::Machine)));
    let room = Room::with_agents(log, Arc::new(DeskKeys), Fake::new(replies()));
    let sent = tool(
        &room,
        "ada",
        "send",
        json!({"to": ["Bob", "Cy"], "title": "Storage numbers", "message": "Run the bench."}),
    )
    .await
    .unwrap();
    assert_eq!(
        sent["delivery"], "Bob: in inbox · Cy: awaiting approval",
        "{sent}"
    );
    assert_eq!(sent["sent"][0]["delivery"], "in inbox");
    assert_eq!(sent["sent"][1]["delivery"], "awaiting approval");
    let note = sent["note"].as_str().unwrap();
    assert!(
        note.starts_with("Bob: in inbox · Cy: awaiting approval. "),
        "{note}"
    );
    assert!(!note.contains("In their inbox"), "{note}");
    assert!(
        note.contains("has not yet allowed you to reach Cy"),
        "{note}"
    );
}

/// A question answered with `send` and `reply_to` closes as answered, as
/// `update_inbox`'s `answer` closes it, and the answer wants nothing back.
/// A reply to an item that wants work is a question about it, and leaves
/// it open.
#[tokio::test]
async fn a_reply_sent_with_reply_to_closes_the_question_as_answered() {
    let (room, _) = room("topic-answered");
    let asked = send(&room, "ada", "bob", "What was the score?", json!({})).await;
    let reply = tool(
        &room,
        "bob",
        "send",
        json!({"to": ["ada"], "message": "Devils 7–4.", "reply_to": short(&asked)}),
    )
    .await
    .unwrap();
    let question = item(&room, "bob", &asked);
    assert_eq!(question.state, InboxState::Closed);
    assert_eq!(question.outcome, Some(InboxOutcome::Answered));
    let answer = item(&room, "ada", reply["sent"][0]["id"].as_str().unwrap());
    assert_eq!(answer.wants, InboxWants::Nothing);
    assert_eq!(answer.reply_to.as_deref(), Some(asked.as_str()));
    assert_eq!(answer.topic, question.topic);

    let work = send(
        &room,
        "ada",
        "bob",
        "Fix the crane",
        json!({"wants": "work"}),
    )
    .await;
    tool(
        &room,
        "bob",
        "send",
        json!({"to": ["ada"], "message": "Which crane?", "reply_to": short(&work)}),
    )
    .await
    .unwrap();
    assert_ne!(item(&room, "bob", &work).state, InboxState::Closed);
}

/// A teammate can follow up on its own send that nobody has answered: the
/// follow-up names it and is in its topic, and the first stays open.
#[tokio::test]
async fn a_teammate_follows_up_on_its_own_unanswered_send() {
    let (room, _) = room("topic-own-send");
    let first = send(&room, "ada", "bob", "Logo colours?", json!({})).await;
    let again = tool(
        &room,
        "ada",
        "send",
        json!({"to": ["bob"], "message": "Did you see this?", "reply_to": short(&first)}),
    )
    .await
    .unwrap();
    let again = item(&room, "bob", again["sent"][0]["id"].as_str().unwrap());
    assert_eq!(again.reply_to.as_deref(), Some(first.as_str()));
    assert_eq!(again.topic, item(&room, "bob", &first).topic);
    assert_eq!(again.wants, InboxWants::Answer);
    assert_eq!(item(&room, "bob", &first).state, InboxState::New);
    let nobody = tool(
        &room,
        "ada",
        "send",
        json!({"to": ["bob"], "message": "?", "reply_to": "zzzzzzzz"}),
    )
    .await
    .unwrap_err();
    assert!(nobody.contains("among what you sent"), "{nobody}");
}
