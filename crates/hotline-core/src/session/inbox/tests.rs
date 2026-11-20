//! The inbox end to end on scripted agents (`docs/design/agent-inbox.md`):
//! one verb, an item's states and receipts, the gather wake, the busy notice,
//! reading and sorting, taking work, replies, what the sender is told, and the
//! guards that were the ask's.

use super::*;
use crate::contract::{InboxOutcome, InboxState, Reach};
use crate::driver::{MessageKind, Update};
use crate::mcp::server::{TOOL_NAMES, TeammateTools};
use crate::session::Session;
use crate::session::tests::{DeskKeys, Fake, Scripted, enrol, persona, scratch};
use crate::test_support::until;
use std::time::Duration;

/// Each turn says `said-<n>` and ends.
fn replies() -> Scripted {
    Scripted::turns(
        (0..40)
            .map(|i| {
                vec![
                    Update::Message {
                        kind: MessageKind::Agent,
                        id: format!("said-{i}"),
                        text: format!("said-{i}"),
                    },
                    Update::Turn {
                        stop_reason: "end_turn".into(),
                        usage: None,
                    },
                ]
            })
            .collect(),
    )
}

fn named(id: &str, name: &str, reach: Option<Reach>) -> Persona {
    let mut teammate = persona(id);
    teammate.name = name.to_string();
    teammate.reach = reach;
    teammate
}

/// Ada and Bob, both Whole machine Hotline Agents the person has allowed to
/// hand each other work, so they reach each other without a card.
fn room_on(name: &str, script: Scripted) -> (Arc<Room>, Arc<Fake>) {
    let log = scratch(name);
    enrol(&log, &named("ada", "Ada", Some(Reach::Machine)));
    enrol(&log, &named("bob", "Bob", Some(Reach::Machine)));
    let agents = Fake::new(script);
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
    room.allow_sender("bob", "ada").unwrap();
    room.allow_sender("ada", "bob").unwrap();
    (room, agents)
}

fn room(name: &str) -> (Arc<Room>, Arc<Fake>) {
    room_on(name, replies())
}

async fn tool(room: &Arc<Room>, who: &str, name: &str, arguments: Value) -> Result<Value, String> {
    let said = TeammateTools::new(room, who).call(name, &arguments).await?;
    Ok(serde_json::from_str(&said).unwrap_or(Value::String(said)))
}

/// `from` sends `to` an item, and answers its id.
async fn send(room: &Arc<Room>, from: &str, to: &str, title: &str, extra: Value) -> String {
    let mut arguments =
        json!({"to": [to], "title": title, "message": format!("the words of {title}")});
    if let Value::Object(fields) = extra {
        for (key, value) in fields {
            arguments[key] = value;
        }
    }
    let sent = tool(room, from, "send", arguments).await.unwrap();
    sent["sent"][0]["id"].as_str().unwrap().to_string()
}

/// Answer with a quick line, without an explicit route or `wants`.
async fn answer_with_send(room: &Arc<Room>, question: &InboxItem, words: &str) -> InboxItem {
    let sent = tool(
        room,
        &question.to,
        "send",
        json!({"to": [question.from], "message": words, "reply_to": short(&question.id)}),
    )
    .await
    .expect("the answer was sent");
    assert_eq!(sent["sent"][0]["delivery"], "in inbox", "{sent}");
    let reply = item(
        room,
        &question.from,
        sent["sent"][0]["id"].as_str().unwrap(),
    );
    assert_eq!(reply.reply_to.as_deref(), Some(question.id.as_str()));
    assert_eq!(reply.topic, question.topic);
    assert_eq!(reply.wants, InboxWants::Nothing);
    assert_eq!(
        item(room, &question.to, &question.id).outcome,
        Some(InboxOutcome::Answered)
    );
    reply
}

fn items(room: &Room, persona_id: &str) -> Vec<InboxItem> {
    kept::of(&room.log, persona_id)
}

fn item(room: &Room, persona_id: &str, id: &str) -> InboxItem {
    items(room, persona_id)
        .into_iter()
        .find(|item| item.id == id)
        .expect("the item is kept")
}

/// The two teammates' brake, as the room stream last wrote it.
fn pair(room: &Room) -> Option<Value> {
    room.log
        .load(&StreamId::Room)
        .into_iter()
        .rfind(|event| event["kind"] == "exchange_pair" && event["id"] == "ada~bob")
}

fn states(item: &InboxItem) -> Vec<InboxState> {
    item.receipts.iter().map(|receipt| receipt.state).collect()
}

/// A teammate's main conversation, started and at rest.
async fn at_rest(room: &Arc<Room>, persona_id: &str) -> Arc<Session> {
    room.start(persona_id).await.unwrap();
    let session = room.session(persona_id).unwrap();
    until("at rest", || !lock(&session.turns).running).await;
    session
}

#[tokio::test]
async fn agents_reach_a_colleague_only_by_send() {
    for offered in ["send", "read_inbox", "update_inbox"] {
        assert!(TOOL_NAMES.contains(&offered), "{offered}");
    }
    for gone in ["message_teammate", "aside"] {
        assert!(!TOOL_NAMES.contains(&gone), "{gone}");
    }
    let (room, _) = room("inbox-one-verb");
    for gone in ["message_teammate", "aside"] {
        let main = tool(
            &room,
            "ada",
            gone,
            json!({"to": "bob", "message": "x", "text": "x"}),
        )
        .await
        .unwrap_err();
        // A conversation from 0.49 that names an old tool is told, in one
        // line, the one that took its place.
        let pointer =
            format!("`{gone}` is gone in this version of Hotline: use `send` to reach a teammate.");
        assert_eq!(main, pointer);
        for (thread, tools) in [
            ("work", TeammateTools::new(&room, "ada").for_work("s1")),
            ("job", TeammateTools::new(&room, "ada").for_job("j1")),
        ] {
            let refused = tools
                .call(gone, &json!({"to": "bob", "message": "x", "text": "x"}))
                .await
                .unwrap_err();
            assert_eq!(refused, pointer, "{thread}");
        }
    }
    let told = crate::mcp::server::how_to_use(&|_| true);
    assert!(
        told.contains("`update_inbox`")
            && !told.contains("message_teammate")
            && !told.contains("`aside`")
    );
}

#[tokio::test]
async fn an_item_keeps_its_state_and_each_receipt_with_its_time_across_a_restart() {
    let (room, _) = room("inbox-states");
    let id = send(
        &room,
        "ada",
        "bob",
        "Q3 numbers",
        json!({"wants": "answer", "waiting": true}),
    )
    .await;
    let sent = item(&room, "bob", &id);
    assert_eq!(sent.from, "ada");
    assert_eq!(sent.from_thread, ThreadId::dm("ada"));
    assert_eq!(sent.to, "bob");
    assert_eq!(sent.title, "Q3 numbers");
    assert_eq!(sent.body, "the words of Q3 numbers");
    assert_eq!(sent.wants, InboxWants::Answer);
    assert!(sent.waiting);
    assert_eq!(states(&sent)[0], InboxState::New);

    tool(&room, "bob", "read_inbox", json!({"ids": [short(&id)]}))
        .await
        .unwrap();
    tool(
        &room,
        "bob",
        "update_inbox",
        json!({"items": [{"id": id, "state": "later", "reason": "after the release"}]}),
    )
    .await
    .unwrap();
    tool(
        &room,
        "bob",
        "update_inbox",
        json!({"items": [{"id": id, "state": "closed", "outcome": "noted"}]}),
    )
    .await
    .unwrap();
    let log = room.log.clone();
    drop(room);
    // The stream is the whole of it: a room opened on it again reads the
    // same item.
    let reopened = Room::with_agents(log, Arc::new(DeskKeys), Fake::new(replies()));
    let kept = item(&reopened, "bob", &id);
    assert_eq!(kept.state, InboxState::Closed);
    assert_eq!(kept.outcome, Some(InboxOutcome::Noted));
    let moved: Vec<InboxState> = states(&kept)
        .into_iter()
        .filter(|state| *state != InboxState::Noticed)
        .collect();
    assert_eq!(
        moved,
        [
            InboxState::New,
            InboxState::Seen,
            InboxState::Later,
            InboxState::Closed
        ]
    );
    let later = kept
        .receipts
        .iter()
        .find(|receipt| receipt.state == InboxState::Later)
        .unwrap();
    assert_eq!(later.reason.as_deref(), Some("after the release"));
    assert!(
        kept.receipts
            .windows(2)
            .all(|pair| pair[0].at <= pair[1].at),
        "{:?}",
        kept.receipts
    );
}

#[tokio::test]
async fn items_that_land_together_wake_an_idle_receiver_once_with_titles_never_bodies() {
    let (room, agents) = room("inbox-gather");
    at_rest(&room, "bob").await;
    send(
        &room,
        "ada",
        "bob",
        "Fix the build",
        json!({"wants": "work", "waiting": true}),
    )
    .await;
    send(
        &room,
        "ada",
        "bob",
        "Logo colours",
        json!({"wants": "nothing"}),
    )
    .await;
    until("bob woke", || agents.told().len() == 1).await;
    tokio::time::sleep(heard::gather_delay() * 3).await;
    let woken = agents
        .told()
        .iter()
        .filter(|told| told.contains(heard::QUIET_WAKE_LINE))
        .count();
    assert_eq!(woken, 1, "one wake for both: {:?}", agents.told());
    let heard = &agents.told()[0];
    assert!(heard.contains("NEW 2"), "{heard}");
    assert!(
        heard.contains("\"Fix the build\" · from Ada · waiting on you · wants work"),
        "{heard}"
    );
    assert!(
        heard.contains("\"Logo colours\" · from Ada · wants nothing"),
        "{heard}"
    );
    assert!(heard.contains("<hotline_inbox>"), "fenced as data: {heard}");
    assert!(!heard.contains("the words of"), "never a body: {heard}");
    assert!(heard.contains(heard::QUIET_WAKE_LINE), "{heard}");
    for item in items(&room, "bob") {
        assert_eq!(item.state, InboxState::Noticed);
        assert!(item.offered.is_some());
    }
}

#[tokio::test]
async fn a_busy_receiver_is_told_mid_turn_and_woken_once_its_turn_ends() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let (room, agents) = room_on("inbox-busy", replies().gated(gate.clone()).consuming());
    room.start("bob").await.unwrap();
    room.prompt("bob", "the person's work", None, None)
        .await
        .unwrap();
    until("bob is working", || agents.prompts().len() == 1).await;
    send(
        &room,
        "ada",
        "bob",
        "Deploy notes",
        json!({"waiting": true}),
    )
    .await;
    until("the notice reached the turn", || {
        agents.steered().len() == 1
    })
    .await;
    let notice = &agents.steered()[0];
    assert!(notice.contains("1 new message in your inbox"), "{notice}");
    assert!(
        notice.contains("\"Deploy notes\" from Ada (waiting on you)"),
        "{notice}"
    );
    assert!(!notice.contains("the words of"), "{notice}");
    tokio::time::sleep(heard::gather_delay() * 3).await;
    assert_eq!(agents.prompts().len(), 1, "no turn is started for it");
    assert!(room.sides("bob").is_empty(), "nor a work thread");
    assert_eq!(items(&room, "bob")[0].state, InboxState::Noticed);
    gate.add_permits(100);
    until("bob is woken once its turn ends", || {
        agents.told().len() == 2
    })
    .await;
    assert!(
        agents.told()[1].contains("\"Deploy notes\""),
        "{}",
        agents.told()[1]
    );
}

#[tokio::test]
async fn a_driver_that_takes_no_line_mid_turn_hears_its_items_in_a_quiet_turn_after_its_own() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let (room, agents) = room_on("inbox-busy-unsteered", replies().gated(gate.clone()));
    room.start("bob").await.unwrap();
    room.prompt("bob", "the person's work", None, None)
        .await
        .unwrap();
    until("bob is working", || agents.prompts().len() == 1).await;
    send(&room, "ada", "bob", "Deploy notes", json!({})).await;
    tokio::time::sleep(heard::gather_delay() * 3).await;
    assert_eq!(agents.prompts().len(), 1, "nothing starts while it works");
    gate.add_permits(100);
    until("the next turn", || agents.told().len() == 2).await;
    tokio::time::sleep(heard::gather_delay() * 3).await;
    let woken = agents
        .told()
        .iter()
        .filter(|told| told.contains(heard::QUIET_WAKE_LINE))
        .count();
    assert_eq!(woken, 1, "heard once: {:?}", agents.told());
    assert!(
        agents.told()[1].contains(heard::QUIET_WAKE_LINE)
            && agents.told()[1].contains("\"Deploy notes\""),
        "{}",
        agents.told()[1]
    );
}

#[tokio::test]
async fn a_turn_is_offered_an_item_once_and_keeps_the_unread_count() {
    let (room, agents) = room("inbox-offered-once");
    let session = at_rest(&room, "bob").await;
    // Busy as it lands, so no wake takes it first.
    lock(&session.turns).running = true;
    send(&room, "ada", "bob", "Crane permit", json!({})).await;
    tokio::time::sleep(heard::gather_delay() * 2).await;
    lock(&session.turns).running = false;
    lock(&session.turns).waiting.clear();
    room.prompt("bob", "hello", None, None).await.unwrap();
    until("bob's turn", || agents.told().len() == 1).await;
    assert!(agents.told()[0].contains("NEW 1"), "{}", agents.told()[0]);
    assert!(
        agents.told()[0].contains("\"Crane permit\""),
        "{}",
        agents.told()[0]
    );
    until("at rest", || !lock(&session.turns).running).await;
    room.prompt("bob", "and now", None, None).await.unwrap();
    until("bob's next turn", || agents.told().len() == 2).await;
    let next = &agents.told()[1];
    assert!(next.contains("INBOX · 1 unread"), "{next}");
    assert!(!next.contains("Crane permit"), "not offered again: {next}");
}

#[tokio::test]
async fn read_inbox_lists_open_items_without_their_words_and_reading_marks_them_seen() {
    let (room, _) = room("inbox-read");
    let first = send(&room, "ada", "bob", "First", json!({"wants": "work"})).await;
    let second = send(&room, "ada", "bob", "Second", json!({"waiting": true})).await;
    let listed = tool(&room, "bob", "read_inbox", json!({})).await.unwrap();
    let rows = listed["items"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["title"], "Second", "newest first");
    assert_eq!(rows[0]["waiting"], true);
    assert_eq!(rows[1]["wants"], "work");
    assert_eq!(rows[1]["from"], "Ada");
    assert!(rows.iter().all(|row| row.get("message").is_none()));
    assert!(!listed.to_string().contains("the words of"));
    let read = tool(&room, "bob", "read_inbox", json!({"ids": [short(&first)]}))
        .await
        .unwrap();
    assert_eq!(read["items"][0]["message"], "the words of First");
    assert_eq!(item(&room, "bob", &first).state, InboxState::Seen);
    assert!(item(&room, "bob", &second).state.unread());
    assert_eq!(room.inbox_unread("bob"), 1);
}

#[tokio::test]
async fn an_item_one_send_made_for_several_says_who_else_got_it_and_an_old_one_still_loads() {
    let (room, _) = room("inbox-also-to");
    enrol(&room.log, &named("cy", "Cy", Some(Reach::Machine)));
    room.allow_sender("cy", "ada").unwrap();
    let sent = tool(
        &room,
        "ada",
        "send",
        json!({"to": ["Bob", "Cy"], "title": "Release moves to Thursday", "wants": "nothing"}),
    )
    .await
    .unwrap();
    let id = |n: usize| sent["sent"][n]["id"].as_str().unwrap().to_string();
    assert_eq!(item(&room, "bob", &id(0)).also_to, ["cy"]);
    assert_eq!(item(&room, "cy", &id(1)).also_to, ["bob"]);
    let listed = tool(&room, "bob", "read_inbox", json!({})).await.unwrap();
    assert_eq!(listed["items"][0]["alsoTo"], json!(["Cy"]), "{listed}");
    let read = tool(&room, "cy", "read_inbox", json!({"ids": [short(&id(1))]}))
        .await
        .unwrap();
    assert_eq!(read["items"][0]["alsoTo"], json!(["Bob"]), "{read}");

    // One to a single teammate says nothing of it; one kept before the
    // field was recorded reads as that.
    let alone = send(&room, "ada", "bob", "Alone", json!({})).await;
    assert_eq!(item(&room, "bob", &alone).also_to, Vec::<String>::new());
    let line = kept::line(&item(&room, "bob", &alone));
    assert!(line.get("alsoTo").is_none(), "{line}");
    let mut old = line;
    old["id"] = json!("old-item-0000");
    room.log
        .append(&StreamId::Inbox("bob".to_string()), &old)
        .unwrap();
    assert_eq!(
        item(&room, "bob", "old-item-0000").also_to,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn read_inbox_lists_closed_items_and_what_was_sent_newest_first_with_where_each_stands() {
    let (room, _) = room("inbox-read-lists");
    let done = send(&room, "ada", "bob", "Done one", json!({})).await;
    let seen = send(&room, "ada", "bob", "Seen one", json!({"wants": "work"})).await;
    tool(&room, "bob", "read_inbox", json!({"ids": [short(&seen)]}))
        .await
        .unwrap();
    tool(
        &room,
        "bob",
        "update_inbox",
        json!({"items": [{"id": short(&done), "state": "closed", "outcome": "noted"}]}),
    )
    .await
    .unwrap();

    let open = tool(&room, "bob", "read_inbox", json!({})).await.unwrap();
    let open: Vec<&str> = open["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row["title"].as_str())
        .collect();
    assert_eq!(open, ["Seen one"], "the default is unchanged");

    let closed = tool(&room, "bob", "read_inbox", json!({"list": "closed"}))
        .await
        .unwrap();
    assert_eq!(closed["items"].as_array().unwrap().len(), 1, "{closed}");
    assert_eq!(closed["items"][0]["title"], "Done one");
    assert_eq!(closed["items"][0]["outcome"], "noted");
    assert_eq!(closed["items"][0]["from"], "Ada");
    assert!(!closed.to_string().contains("the words of"), "{closed}");

    let sent = tool(&room, "ada", "read_inbox", json!({"list": "sent"}))
        .await
        .unwrap();
    let rows = sent["items"].as_array().unwrap();
    assert_eq!(rows[0]["title"], "Seen one", "newest first: {sent}");
    assert_eq!(rows[0]["to"], "Bob");
    assert_eq!(rows[0]["state"], "seen", "read, not yet taken: {sent}");
    assert!(rows[0]["since"].is_string(), "{sent}");
    assert!(rows[0].get("from").is_none(), "{sent}");
    assert_eq!(rows[1]["state"], "closed");
    assert_eq!(rows[1]["outcome"], "noted");

    // More than the list shows, kept as sent ones are (the brake would
    // pause twelve sends in a row).
    let ada = room.persona("ada").unwrap();
    for n in 0..LISTED {
        let mut more = super::item(
            &ada,
            &ThreadId::dm("ada"),
            "bob",
            format!("More {n}"),
            String::new(),
            InboxWants::Nothing,
        );
        moved(&mut more, InboxState::New, None, None, None);
        room.keep_item(&more).unwrap();
    }
    let sent = tool(&room, "ada", "read_inbox", json!({"list": "sent"}))
        .await
        .unwrap();
    assert_eq!(sent["items"].as_array().unwrap().len(), LISTED);
    assert_eq!(sent["older"], 2, "{sent}");
    let wrong = tool(&room, "ada", "read_inbox", json!({"list": "everything"})).await;
    assert!(wrong.is_err_and(|error| error.contains("open, closed or sent")));
}

#[tokio::test]
async fn update_inbox_sorts_several_items_in_one_call_in_the_order_given_or_none() {
    let (room, _) = room("inbox-sort");
    let later = send(
        &room,
        "ada",
        "bob",
        "Later one",
        json!({"wants": "nothing"}),
    )
    .await;
    let noted = send(
        &room,
        "ada",
        "bob",
        "Noted one",
        json!({"wants": "nothing"}),
    )
    .await;
    let folded = send(
        &room,
        "ada",
        "bob",
        "Folded one",
        json!({"wants": "answer"}),
    )
    .await;
    // One bad entry and nothing moves.
    let refused = tool(
        &room,
        "bob",
        "update_inbox",
        json!({"items": [
            {"id": noted, "state": "closed", "outcome": "noted"},
            {"id": later, "state": "later"},
        ]}),
    )
    .await
    .unwrap_err();
    assert!(refused.contains("needs a `reason`"), "{refused}");
    assert!(item(&room, "bob", &noted).state.unread());
    let sorted = tool(
        &room,
        "bob",
        "update_inbox",
        json!({"items": [
            {"id": short(&noted), "state": "closed", "outcome": "noted"},
            {"id": folded, "state": "combined", "into": later},
            {"id": later, "state": "later", "reason": "after lunch"},
        ]}),
    )
    .await
    .unwrap();
    assert_eq!(sorted["open"], 1, "{sorted}");
    assert_eq!(
        item(&room, "bob", &noted).outcome,
        Some(InboxOutcome::Noted)
    );
    let combined = item(&room, "bob", &folded);
    assert_eq!(combined.outcome, Some(InboxOutcome::Combined));
    assert_eq!(combined.combined_into.as_deref(), Some(later.as_str()));
    assert_eq!(item(&room, "bob", &later).state, InboxState::Later);
    // The main conversation's record of the sort, in the order given.
    let line = room
        .tape("bob")
        .into_iter()
        .find(|event| event["kind"] == "inbox_sorted")
        .expect("a sorted line on bob's main conversation");
    let order: Vec<&str> = line["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["item"].as_str().unwrap())
        .collect();
    assert_eq!(order, [noted.as_str(), folded.as_str(), later.as_str()]);
    assert_eq!(line["entries"][1]["into"], later.as_str());
    assert_eq!(line["entries"][2]["state"], "later");
    assert_eq!(line["entries"][2]["reason"], "after lunch");
    assert_eq!(line["open"], 1);
}

#[tokio::test]
async fn taking_work_opens_a_work_thread_seeded_with_what_was_combined_into_it_and_its_result_comes_back()
 {
    let (room, _) = room("inbox-take");
    at_rest(&room, "ada").await;
    let main = send(
        &room,
        "ada",
        "bob",
        "Fix the crane",
        json!({"wants": "work"}),
    )
    .await;
    let extra = send(
        &room,
        "ada",
        "bob",
        "Also the winch",
        json!({"wants": "work"}),
    )
    .await;
    assert!(room.sides("bob").is_empty(), "sending starts nothing");
    let sorted = tool(
        &room,
        "bob",
        "update_inbox",
        json!({"items": [
            {"id": extra, "state": "combined", "into": main},
            {"id": main, "state": "taken"},
        ]}),
    )
    .await
    .unwrap();
    assert_eq!(
        sorted["workThreads"].as_array().unwrap().len(),
        1,
        "{sorted}"
    );
    let again = tool(
        &room,
        "bob",
        "update_inbox",
        json!({"items": [{"id": main, "state": "taken"}]}),
    )
    .await
    .unwrap_err();
    assert!(again.contains("already taken"), "taken once: {again}");
    until("the work is done", || {
        item(&room, "bob", &main).state == InboxState::Closed
    })
    .await;
    let taken = item(&room, "bob", &main);
    assert_eq!(taken.outcome, Some(InboxOutcome::Done));
    assert!(states(&taken).contains(&InboxState::Taken));
    let work = taken.work.expect("it says where the work ran");
    let opened = room.log.load(&StreamId::Side(work.key.clone()));
    let brief = opened
        .iter()
        .find(|event| event["cause"]["kind"] == "handoff")
        .expect("the work thread opens with the items");
    let words = brief["text"].as_str().unwrap();
    assert!(
        words.contains("the words of Fix the crane")
            && words.contains("the words of Also the winch"),
        "{words}"
    );
    // The result is an item in the sender's inbox that answers the one taken.
    let reply = items(&room, "ada")
        .into_iter()
        .find(|reply| reply.reply_to.as_deref() == Some(main.as_str()))
        .expect("a reply in ada's inbox");
    assert_eq!(reply.from, "bob");
    assert_eq!(reply.title, "Re: Fix the crane");
    assert_eq!(reply.from_thread, work);
    assert!(reply.body.starts_with("said-"), "{}", reply.body);
}

#[tokio::test]
async fn work_done_in_the_turn_starts_no_thread_and_its_answer_closes_it_as_answered() {
    let (room, _) = room("inbox-take-here");
    let id = send(
        &room,
        "ada",
        "bob",
        "Rename the file",
        json!({"wants": "work"}),
    )
    .await;
    tool(
        &room,
        "bob",
        "update_inbox",
        json!({"items": [{"id": id, "state": "taken", "in_thread": false}]}),
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(room.sides("bob"), [] as [crate::contract::RunningSide; 0]);
    assert_eq!(item(&room, "bob", &id).state, InboxState::Taken);
    tool(
        &room,
        "bob",
        "update_inbox",
        json!({"items": [{"id": id, "answer": "Renamed it to crane.md."}]}),
    )
    .await
    .unwrap();
    let done = item(&room, "bob", &id);
    assert_eq!(done.outcome, Some(InboxOutcome::Answered));
    let reply = items(&room, "ada")
        .into_iter()
        .find(|reply| reply.reply_to.as_deref() == Some(id.as_str()))
        .unwrap();
    assert_eq!(reply.body, "Renamed it to crane.md.");
    assert_eq!(reply.wants, InboxWants::Nothing);
}

/// Answered is what an `answer` does, and Done what finished work does: a
/// teammate closes nothing as answered by naming it, only by answering.
#[test]
fn a_teammate_cannot_close_an_item_as_answered_without_answering_it() {
    let Err(refused) = Sorting::from_arguments(
        &json!({"items": [{"id": "abcd", "state": "closed", "outcome": "answered"}]}),
    ) else {
        panic!("answered is not a teammate's outcome");
    };
    assert!(refused.contains("done, noted or dropped"), "{refused}");
    let done = Sorting::from_arguments(
        &json!({"items": [{"id": "abcd", "state": "closed", "outcome": "done"}]}),
    )
    .unwrap();
    assert_eq!(done[0].sort, Some(Sort::Closed(InboxOutcome::Done)));
}

/// Both ways of answering must wake the original thread and offer the item
/// there, not just retain the same topic in the sender's inbox.
async fn thread_hears_both_answers(room: &Arc<Room>, agents: &Fake, thread: &ThreadId) {
    until("Ada's thread is at rest", || {
        room.sides
            .get(&thread.key)
            .is_some_and(|side| !side.turn_running())
            && agents.told().len() == 1
    })
    .await;
    let tools = agents
        .tools()
        .into_iter()
        .next()
        .expect("Ada's thread started");
    let mut reply_ids = Vec::new();
    for (message, answer, quick) in [
        ("Where is the crane spec?", "In /shared/crane.pdf", false),
        ("Who signed the spec?", "Kim signed it.", true),
    ] {
        let sent = tools
            .call("send", &json!({"to": ["bob"], "message": message}))
            .await
            .unwrap();
        let sent: Value = serde_json::from_str(&sent).unwrap();
        let question = item(room, "bob", sent["sent"][0]["id"].as_str().unwrap());
        assert_eq!(&question.from_thread, thread);
        let reply = if quick {
            answer_with_send(room, &question, answer).await
        } else {
            tool(
                room,
                "bob",
                "update_inbox",
                json!({"items": [{"id": question.id, "answer": answer}]}),
            )
            .await
            .unwrap();
            items(room, "ada")
                .into_iter()
                .find(|reply| reply.reply_to.as_deref() == Some(question.id.as_str()))
                .expect("the answer is in Ada's inbox")
        };
        assert_eq!(reply.about.as_ref(), Some(thread));
        assert_eq!(&room.inbox_heard_in(&reply), thread);
        reply_ids.push(reply.id);
    }
    let stream = StreamId::of_thread(thread).expect("the thread has a transcript");
    until("the origin thread wakes and hears both replies", || {
        let told = agents.told();
        reply_ids.iter().all(|id| {
            told.iter()
                .any(|line| line.contains(heard::WAKE_LINE) && line.contains(short(id)))
        }) && room
            .log
            .load(&stream)
            .iter()
            .filter(|event| event["kind"] == "turn")
            .count()
            >= 2
    })
    .await;
    assert!(agents.told().iter().all(|line| {
        !line.contains("quiet inbox") || reply_ids.iter().all(|id| !line.contains(short(id)))
    }));
    assert!(
        room.tape("ada")
            .iter()
            .all(|event| !event.to_string().contains("crane.pdf")),
        "the answer's body did not go to the person's chat"
    );
}

#[tokio::test(start_paused = true)]
async fn an_answer_is_heard_in_the_live_work_thread_it_was_asked_from() {
    let (room, agents) = room("inbox-reply-thread");
    let side = room.start_side("ada", "the crane").await.unwrap().side_id;
    thread_hears_both_answers(&room, &agents, &ThreadId::side(&side)).await;
}

#[tokio::test(start_paused = true)]
async fn both_answer_styles_are_heard_in_the_live_call_that_asked() {
    let (room, agents) = room("inbox-reply-call");
    let call = ThreadId::call("crane-call");
    room.call_began(&call, "ada", now_ms());
    room.prompt_call_work("ada", &call, "Look up the crane spec")
        .await
        .unwrap();
    thread_hears_both_answers(&room, &agents, &call).await;
}

#[tokio::test]
async fn the_sender_is_told_once_what_became_of_its_items_at_its_next_turn() {
    let (room, agents) = room("inbox-receipts");
    let session = at_rest(&room, "ada").await;
    let id = send(&room, "ada", "bob", "Pick a name", json!({})).await;
    tool(
        &room,
        "bob",
        "update_inbox",
        json!({"items": [{"id": id, "state": "later", "reason": "after the release"}]}),
    )
    .await
    .unwrap();
    room.prompt("ada", "anything?", None, None).await.unwrap();
    until("ada's turn", || agents.told().len() == 1).await;
    let told = &agents.told()[0];
    assert!(told.contains("SENT · what moved 1"), "{told}");
    assert!(told.contains("\"Pick a name\" to Bob · later"), "{told}");
    assert!(told.contains(": \"after the release\""), "{told}");
    until("at rest", || !lock(&session.turns).running).await;
    room.prompt("ada", "and now?", None, None).await.unwrap();
    until("ada's next turn", || agents.told().len() == 2).await;
    assert!(
        !agents.told()[1].contains("Pick a name"),
        "{}",
        agents.told()[1]
    );
}

#[tokio::test]
async fn first_contact_holds_an_item_until_the_card_and_a_no_drops_it_unseen() {
    let log = scratch("inbox-consent");
    // Bob has the whole machine, more than Ada: her items to him wait on the
    // person's card.
    enrol(&log, &named("ada", "Ada", None));
    enrol(&log, &named("bob", "Bob", Some(Reach::Machine)));
    let room = Room::with_agents(log, Arc::new(DeskKeys), Fake::new(replies()));
    let sent = crate::session::tests::send_past_card(
        &room,
        "ada",
        json!({"to": ["bob"], "title": "Psst", "message": "the secret"}),
    )
    .await
    .unwrap();
    let id = sent["sent"][0]["id"].as_str().unwrap().to_string();
    assert!(
        kept::received(&room.log, "bob").is_empty(),
        "held, not bob's yet"
    );
    let card = || {
        room.tape("ada").into_iter().find(|event| {
            event["kind"] == "permission"
                && event["requestId"]
                    .as_str()
                    .is_some_and(|id| id.starts_with(crate::session::peers::COLLAB_REQUEST_PREFIX))
                && event.get("decision").is_none()
        })
    };
    until("the first-contact card", || card().is_some()).await;
    let request = card().unwrap()["requestId"].as_str().unwrap().to_string();
    room.answer_permission("ada", &request, "deny")
        .await
        .unwrap();
    until("the item is dropped", || {
        item(&room, "bob", &id).state == InboxState::Closed
    })
    .await;
    let dropped = item(&room, "bob", &id);
    assert!(dropped.held, "bob never had it");
    assert_eq!(dropped.outcome, Some(InboxOutcome::Dropped));
    assert!(dropped.reason.as_deref().unwrap().contains("denied"));
    assert_eq!(kept::received(&room.log, "bob"), [] as [InboxItem; 0]);

    // Allowed, the next one lands as any other.
    let next = crate::session::tests::send_past_card(
        &room,
        "ada",
        json!({"to": ["bob"], "title": "Again", "message": "again"}),
    )
    .await
    .unwrap()["sent"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    until("the second card", || card().is_some()).await;
    let request = card().unwrap()["requestId"].as_str().unwrap().to_string();
    room.answer_permission("ada", &request, "allow_always")
        .await
        .unwrap();
    until("it lands", || !item(&room, "bob", &next).held).await;
    assert_eq!(states(&item(&room, "bob", &next))[0], InboxState::New);
}

#[tokio::test]
async fn the_brake_counts_items_and_answers_and_holds_at_the_cap() {
    let (room, _) = room("inbox-brake");
    room.log
        .append(
            &StreamId::Room,
            &json!({"kind": "exchange_pair", "id": "ada~bob", "a": "ada", "b": "bob",
                "exchanges": 10, "paused": false, "requests": []}),
        )
        .unwrap();
    let id = send(&room, "ada", "bob", "Eleventh", json!({})).await;
    assert_eq!(pair(&room).unwrap()["exchanges"], 11);
    tool(
        &room,
        "bob",
        "update_inbox",
        json!({"items": [{"id": id, "answer": "Twelfth"}]}),
    )
    .await
    .unwrap();
    let brake = pair(&room).unwrap();
    assert_eq!(brake["exchanges"], 12);
    assert_eq!(brake["paused"], true);
    assert!(
        room.tape("ada")
            .iter()
            .any(|event| event["kind"] == "exchange_paused" && event["status"] == "pending")
    );
    let refused = tool(
        &room,
        "ada",
        "send",
        json!({"to": ["bob"], "title": "Thirteenth"}),
    )
    .await
    .unwrap_err();
    assert!(refused.contains("paused at 12"), "{refused}");
    room.resume_exchange("ada", "bob").unwrap();
    send(&room, "ada", "bob", "After keep going", json!({})).await;
}

#[tokio::test]
async fn loop_in_puts_an_item_in_the_colleagues_inbox_whose_answer_is_heard_in_the_thread() {
    let (room, agents) = room("inbox-loop-in");
    let side = room.start_side("ada", "the crane").await.unwrap().side_id;
    until("ada's thread is at rest", || {
        room.sides
            .get(&side)
            .is_some_and(|side| !side.turn_running())
            && agents.told().len() == 1
    })
    .await;
    room.loop_in(&side, "bob").await.unwrap();
    let asked = items(&room, "bob");
    assert_eq!(asked.len(), 1);
    assert!(asked[0].looped_in);
    assert_eq!(asked[0].from_thread, ThreadId::side(&side));
    assert!(
        asked[0].title.contains("looped you into Ada's"),
        "{}",
        asked[0].title
    );
    assert!(
        pair(&room).is_none_or(|pair| pair["requests"].as_array().unwrap().is_empty()),
        "no ask is made"
    );
    // The thread's own record draws the quiet line.
    assert!(
        room.log
            .load(&StreamId::Side(side.clone()))
            .iter()
            .any(|event| event["kind"] == "aside" && event["loopedIn"] == true)
    );
    tool(
        &room,
        "bob",
        "update_inbox",
        json!({"items": [{"id": asked[0].id, "answer": "Use the spare winch."}]}),
    )
    .await
    .unwrap();
    until("the thread hears it", || {
        agents
            .told()
            .iter()
            .any(|told| told.contains("from Bob") && told.contains("Re: "))
    })
    .await;
}

#[tokio::test]
async fn a_linked_teammate_is_refused_and_a_teammate_cannot_send_itself() {
    let (room, _) = room("inbox-refusals");
    let itself = tool(&room, "ada", "send", json!({"to": ["ada"], "title": "Me"}))
        .await
        .unwrap_err();
    assert!(itself.contains("not to yourself"), "{itself}");
    let nobody = tool(&room, "ada", "send", json!({"to": ["Zed"], "title": "Who"}))
        .await
        .unwrap_err();
    assert!(nobody.contains("no teammate called"), "{nobody}");
    let untitled = tool(&room, "ada", "send", json!({"to": ["bob"], "title": "  "}))
        .await
        .unwrap_err();
    assert!(untitled.contains("needs a `message`"), "{untitled}");
    // The whole team, each once.
    let team = tool(
        &room,
        "ada",
        "send",
        json!({"to": ["team", "Bob"], "title": "Standup moved"}),
    )
    .await
    .unwrap();
    assert_eq!(team["sent"].as_array().unwrap().len(), 1, "{team}");
}

/// A send in a turn the person reads is drawn there as a row that says where
/// the item stands, so its result tells the teammate the row is the receipt:
/// no line saying it was sent, only what the row does not say, or nothing.
/// A quiet turn's send, and one made outside any turn, are told no such thing.
#[tokio::test]
async fn the_send_result_says_the_row_is_the_receipt_in_a_turn_the_person_reads() {
    let (room, _) = room("send-receipt");
    let main = ThreadId::dm("ada");
    let note = |sent: Value| sent["note"].as_str().unwrap().to_string();
    let send_now = |title: &str| {
        let room = room.clone();
        let title = title.to_string();
        async move {
            tool(
                &room,
                "ada",
                "send",
                json!({"to": ["bob"], "title": title, "message": "?"}),
            )
            .await
            .unwrap()
        }
    };
    let asked = {
        let _turn = room.open_posting(&main, None);
        note(send_now("Score of the last game?").await)
    };
    assert!(
        asked.ends_with(
            "do not send it again. The person sees this as a row in their chat with where it stands, so do not say you sent it or will pass on its answer. Say only what the row does not, or end your turn with nothing."
        ),
        "{asked}"
    );
    assert!(ROW_IS_THE_RECEIPT.split_whitespace().count() <= 40);
    let quiet = {
        let _turn = room.open_posting(&main, None);
        room.set_quiet(&main, true);
        note(send_now("Thursday review").await)
    };
    let outside = note(send_now("Standup").await);
    for unread in [&quiet, &outside] {
        assert!(!unread.contains("row"), "{unread}");
        assert!(unread.ends_with("do not send it again."), "{unread}");
    }
}

mod asked;
mod brake;
mod clock;
mod held;
mod jobs;
mod limit;
mod newer;
mod pick_up;
mod prompted;
mod quiet;
mod retention;
mod taken;
mod titles;
mod topics;
