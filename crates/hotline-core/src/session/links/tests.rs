//! Two desks, linked, talking through their exchange queues.
//!
//! The channel between them is stood in for here by a pump that hands each
//! call to the other room's federation handler, the way `remote/link.rs`
//! does over the sealed channel. What is under test is the rooms' half: who
//! asks the person, where the card is, what each sender hears and when, and
//! what a link's end does to what was waiting on it.

use super::*;
use crate::driver::{MessageKind, Update};
use crate::log::StreamId;
use crate::mcp::server::TeammateTools;
use crate::paths::thread_key;
use crate::session::tests::{DeskKeys, Fake, Scripted, enrol, persona, scratch};
use std::time::Duration;
use tokio::sync::mpsc;

const LAPTOP: &str = "laptop-desk";
const GRIZZLY: &str = "grizzly-desk";

fn answers() -> Scripted {
    Scripted::turns(
        (0..20)
            .map(|i| {
                vec![
                    Update::Message {
                        kind: MessageKind::Agent,
                        id: format!("lou-{i}"),
                        text: format!("runner {i} is green"),
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

/// A desk with one teammate, linked with the other desk by record.
fn desk(name: &str, teammate: (&str, &str), other: (&str, &str), dials: bool) -> Arc<Room> {
    let log = scratch(name);
    let mut one = persona(teammate.0);
    one.name = teammate.1.to_string();
    enrol(&log, &one);
    linked::save(
        &log,
        &Record {
            desk_id: other.0.into(),
            name: other.1.into(),
            dials,
            key: (!dials).then(|| "a-key".to_string()),
            teammates: Vec::new(),
        },
    )
    .unwrap();
    Room::with_agents(log, Arc::new(DeskKeys), Fake::new(answers()))
}

/// The laptop with Clementine and grizzly with Lou, linked both ways.
fn pair(name: &str) -> (Arc<Room>, Arc<Room>) {
    (
        desk(
            &format!("{name}-laptop"),
            ("clementine", "Clementine"),
            (GRIZZLY, "grizzly"),
            true,
        ),
        desk(
            &format!("{name}-grizzly"),
            ("lou", "Lou"),
            (LAPTOP, "laptop"),
            false,
        ),
    )
}

/// One direction of an open channel: every call `from` makes for `to_id`'s
/// desk is answered by `to`'s federation handler, as from `from_id`.
fn pump(
    from: &Arc<Room>,
    to: &Arc<Room>,
    from_id: &str,
    to_id: &str,
) -> (u64, tokio::task::JoinHandle<()>) {
    let (calls, mut waiting) = mpsc::channel::<linked::Call>(32);
    let generation = from.links.opened(to_id, calls);
    let to = to.clone();
    let from_id = from_id.to_string();
    let task = tokio::spawn(async move {
        while let Some(call) = waiting.recv().await {
            let answer = match call.cmd {
                "federation.deliver" => {
                    let delivery = serde_json::from_value(call.params["delivery"].clone()).unwrap();
                    to.federation_deliver(&from_id, delivery)
                }
                "federation.unlink" => to.unlinked(&from_id).await.map(|()| Value::Null),
                other => panic!("nothing sends {other} here"),
            };
            let _ = call.answer.send(answer.map_err(linked::CallError::Refused));
        }
    });
    (generation, task)
}

/// The channel between the two, open: each side's roster published to the
/// other, and calls answered both ways until it is closed.
struct Channel {
    laptop: Arc<Room>,
    grizzly: Arc<Room>,
    ends: Vec<(u64, tokio::task::JoinHandle<()>)>,
}

impl Channel {
    fn open(laptop: &Arc<Room>, grizzly: &Arc<Room>) -> Self {
        laptop
            .federation_roster(GRIZZLY, grizzly.federation_roster_entries())
            .unwrap();
        grizzly
            .federation_roster(LAPTOP, laptop.federation_roster_entries())
            .unwrap();
        Self {
            laptop: laptop.clone(),
            grizzly: grizzly.clone(),
            ends: vec![
                pump(laptop, grizzly, LAPTOP, GRIZZLY),
                pump(grizzly, laptop, GRIZZLY, LAPTOP),
            ],
        }
    }

    fn close(self) {
        let [(laptop, a), (grizzly, b)] = <[_; 2]>::try_from(self.ends).ok().unwrap();
        a.abort();
        b.abort();
        self.laptop.links.closed(GRIZZLY, Some(laptop), None);
        self.grizzly.links.closed(LAPTOP, Some(grizzly), None);
    }
}

async fn until(what: &str, mut condition: impl FnMut() -> bool) {
    for _ in 0..1000 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("never: {what}");
}

async fn message(room: &Arc<Room>, from: &str, to: &str, intent: &str) -> Result<String, String> {
    let sent = TeammateTools::new(room, from)
        .call(
            "message_teammate",
            &json!({"to": to, "message": "Check the CI runners.", "intent": intent}),
        )
        .await?;
    Ok(serde_json::from_str::<Value>(&sent).unwrap()["requestId"]
        .as_str()
        .unwrap()
        .to_string())
}

/// Every exchange request this room holds, as the room stream says it.
fn requests(room: &Room) -> Vec<Value> {
    room.log
        .load(&StreamId::Room)
        .into_iter()
        .filter(|event| event["kind"] == "exchange_pair")
        .flat_map(|pair| pair["requests"].as_array().cloned().unwrap_or_default())
        .collect()
}

fn request(room: &Room, id: &str) -> Option<Value> {
    requests(room)
        .into_iter()
        .find(|request| request["id"] == id)
}

fn collaboration_card(room: &Room, persona_id: &str) -> Option<Value> {
    room.tape(persona_id).into_iter().find(|event| {
        event["kind"] == "permission"
            && event["requestId"]
                .as_str()
                .is_some_and(|id| id.starts_with("collab:"))
            && event.get("decision").is_none()
    })
}

fn delivered(room: &Room, persona_id: &str, request_id: &str) -> Option<Value> {
    room.tape(persona_id)
        .into_iter()
        .find(|event| event["kind"] == "delivery" && event["cause"]["requestId"] == request_id)
}

async fn list(room: &Arc<Room>, persona_id: &str) -> Vec<Value> {
    let listed = TeammateTools::new(room, persona_id)
        .call("list_teammates", &json!({}))
        .await
        .unwrap();
    serde_json::from_str::<Value>(&listed).unwrap()["teammates"]
        .as_array()
        .unwrap()
        .clone()
}

#[tokio::test]
async fn an_ask_crosses_the_link_waits_for_the_recipients_person_and_its_answer_comes_back() {
    let (laptop, grizzly) = pair("ask");
    let _channel = Channel::open(&laptop, &grizzly);

    let listed = list(&laptop, "clementine").await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["personaId"], format!("{GRIZZLY}/lou"));
    assert_eq!(listed[0]["name"], "Lou");
    assert_eq!(listed[0]["desk"], "grizzly");
    assert_eq!(listed[0]["state"], "idle");

    let id = message(&laptop, "clementine", "Lou", "ask").await.unwrap();
    // Clementine is whole-machine on her own desk; on grizzly she is a
    // stranger, and Lou's person decides — on grizzly, on Lou's tape.
    until("the card is on Lou's tape", || {
        collaboration_card(&grizzly, "lou").is_some()
    })
    .await;
    let card = collaboration_card(&grizzly, "lou").unwrap();
    assert!(
        card["title"]
            .as_str()
            .unwrap()
            .starts_with("Allow Clementine on laptop to ask Lou to work?"),
        "{card}"
    );
    assert!(collaboration_card(&laptop, "clementine").is_none());
    assert!(delivered(&laptop, "clementine", &id).is_none());
    // The sender's desk holds no card and can answer none.
    assert!(
        laptop
            .answer_permission(
                "clementine",
                card["requestId"].as_str().unwrap(),
                "allow_session"
            )
            .await
            .is_err()
    );
    grizzly
        .answer_permission("lou", card["requestId"].as_str().unwrap(), "allow_session")
        .await
        .unwrap();

    until("the answer is in Clementine's conversation", || {
        delivered(&laptop, "clementine", &id).is_some()
    })
    .await;
    let answer = delivered(&laptop, "clementine", &id).unwrap();
    assert_eq!(answer["cause"]["kind"], "peer");
    assert_eq!(answer["cause"]["personaId"], format!("{GRIZZLY}/lou"));
    assert_eq!(answer["cause"]["name"], "Lou");
    assert_eq!(answer["cause"]["status"], "done");
    assert_eq!(answer["text"], "runner 0 is green");

    // Each desk keeps its own copy of the pair's thread.
    let here = laptop.log.load(&StreamId::Pair(
        thread_key("clementine", &format!("{GRIZZLY}/lou")).unwrap(),
    ));
    let texts: Vec<&str> = here.iter().filter_map(|v| v["text"].as_str()).collect();
    assert_eq!(texts, ["Check the CI runners.", "runner 0 is green"]);
    let there = grizzly.log.load(&StreamId::Pair(
        thread_key(&format!("{LAPTOP}/clementine"), "lou").unwrap(),
    ));
    assert!(there.iter().any(|v| v["text"] == "Check the CI runners."));
    assert!(there.iter().any(|v| v["text"] == "runner 0 is green"));
    // Lou's thread list names the pair by who is on the other desk.
    let threads = grizzly.peer_threads("lou");
    assert_eq!(threads[0].with_name, "Clementine on laptop");
    // Nothing of Clementine's landed on a tape of hers on grizzly.
    let tapes: Vec<String> = std::fs::read_dir(grizzly.log.root().join("transcripts"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        tapes.iter().all(|name| !name.contains("clementine")),
        "{tapes:?}"
    );
}

#[tokio::test]
async fn a_standing_grant_is_kept_by_the_qualified_id_and_a_handoff_runs_in_the_recipients_work_thread()
 {
    let (laptop, grizzly) = pair("handoff");
    let _channel = Channel::open(&laptop, &grizzly);
    let first = message(&laptop, "clementine", "Lou", "ask").await.unwrap();
    until("the card", || collaboration_card(&grizzly, "lou").is_some()).await;
    let card = collaboration_card(&grizzly, "lou").unwrap();
    grizzly
        .answer_permission("lou", card["requestId"].as_str().unwrap(), "allow_always")
        .await
        .unwrap();
    until("the first answer", || {
        delivered(&laptop, "clementine", &first).is_some()
    })
    .await;
    assert_eq!(
        grizzly.persona("lou").unwrap().allowed_senders,
        [format!("{LAPTOP}/clementine")]
    );

    let handoff = message(&laptop, "clementine", "Lou on grizzly", "handoff")
        .await
        .unwrap();
    until("the handoff's result", || {
        delivered(&laptop, "clementine", &handoff).is_some()
    })
    .await;
    assert!(collaboration_card(&grizzly, "lou").is_none());
    let answer = delivered(&laptop, "clementine", &handoff).unwrap();
    assert_eq!(answer["cause"]["status"], "done");
    // A grant given to a teammate of a linked desk ends with the link.
    grizzly.unlinked(LAPTOP).await.unwrap();
    assert!(grizzly.persona("lou").unwrap().allowed_senders.is_empty());
    assert!(
        grizzly
            .log
            .load(&StreamId::Room)
            .iter()
            .all(|event| { event["kind"] != "collaboration_informed" || event["deleted"] == true })
    );
    // The work ran in a thread of Lou's on grizzly, opened by Clementine.
    let thread = request(&grizzly, &handoff).unwrap()["thread"]
        .as_str()
        .expect("the handoff opened a work thread on grizzly")
        .to_string();
    let brief = grizzly
        .log
        .load(&StreamId::Side(thread))
        .into_iter()
        .find(|v| v["cause"]["kind"] == "handoff")
        .unwrap();
    assert_eq!(brief["cause"]["personaId"], format!("{LAPTOP}/clementine"));
}

#[tokio::test]
async fn a_message_waits_for_the_link_and_an_answer_waits_for_the_sender_to_come_back() {
    let (laptop, grizzly) = pair("offline");
    // Lou is known by name from before, so a message can be addressed and
    // queued while grizzly is away.
    let lou = Known {
        persona_id: "lou".into(),
        name: "Lou".into(),
    };
    let mut record = linked::record(&laptop.log, GRIZZLY).unwrap();
    record.teammates.push(lou);
    linked::save(&laptop.log, &record).unwrap();
    grizzly
        .allow_sender("lou", &format!("{LAPTOP}/clementine"))
        .unwrap();
    assert_eq!(list(&laptop, "clementine").await[0]["state"], "offline");

    let id = message(&laptop, "clementine", "Lou", "ask").await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(delivered(&laptop, "clementine", &id).is_none());
    assert!(requests(&grizzly).is_empty(), "nothing crossed");

    // Grizzly answers while the laptop's end of the channel is gone: the
    // answer is kept there, and crosses when the channel is back.
    let (generation, task) = pump(&laptop, &grizzly, LAPTOP, GRIZZLY);
    until("grizzly has the request", || {
        request(&grizzly, &id).is_some()
    })
    .await;
    task.abort();
    laptop.links.closed(GRIZZLY, Some(generation), None);
    until("grizzly has an answer waiting", || {
        request(&grizzly, &id).is_some_and(|r| r["phase"] == "reply")
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(delivered(&laptop, "clementine", &id).is_none());
    let _channel = Channel::open(&laptop, &grizzly);
    until("the answer arrived after all", || {
        delivered(&laptop, "clementine", &id).is_some()
    })
    .await;
}

#[tokio::test]
async fn an_ended_link_fails_what_waited_on_it_and_its_teammates_are_gone() {
    let (laptop, grizzly) = pair("unlinked");
    let channel = Channel::open(&laptop, &grizzly);
    channel.close();
    let id = message(&laptop, "clementine", "Lou", "ask").await.unwrap();
    laptop.unlinked(GRIZZLY).await.unwrap();

    let notice = laptop
        .tape("clementine")
        .into_iter()
        .find(|event| event["id"] == format!("exchange-ended:{id}"))
        .expect("Clementine is told");
    assert_eq!(
        notice["text"],
        "Message to Lou on grizzly (Check the CI runners.): Lou is on grizzly, which isn't linked any more."
    );
    assert!(list(&laptop, "clementine").await.is_empty());
    let refused = message(&laptop, "clementine", "Lou", "ask")
        .await
        .unwrap_err();
    assert_eq!(
        refused,
        "There is no teammate called 'Lou' in this room. list_teammates says who is here."
    );
    let qualified = message(&laptop, "clementine", &format!("{GRIZZLY}/lou"), "ask")
        .await
        .unwrap_err();
    assert_eq!(
        qualified,
        "That teammate's desk is not linked with this one any more."
    );
    assert!(linked::record(&laptop.log, GRIZZLY).is_none());
}

#[tokio::test]
async fn ending_the_link_withdraws_the_card_a_linked_sender_was_waiting_on() {
    let (laptop, grizzly) = pair("withdrawn");
    let _channel = Channel::open(&laptop, &grizzly);
    let id = message(&laptop, "clementine", "Lou", "ask").await.unwrap();
    until("the card", || collaboration_card(&grizzly, "lou").is_some()).await;
    let card = collaboration_card(&grizzly, "lou").unwrap();
    grizzly.unlinked(LAPTOP).await.unwrap();
    until("the card is withdrawn", || {
        grizzly
            .tape("lou")
            .into_iter()
            .any(|event| event["requestId"] == card["requestId"] && event["decision"] == "expired")
    })
    .await;
    assert!(
        grizzly
            .answer_permission("lou", card["requestId"].as_str().unwrap(), "allow_always")
            .await
            .is_err()
    );
    assert!(grizzly.persona("lou").unwrap().allowed_senders.is_empty());
    let stopped = request(&grizzly, &id).unwrap();
    assert_eq!(stopped["failed"], true);
    assert_eq!(
        stopped["reply"],
        "Clementine is on laptop, which isn't linked any more."
    );
}

#[tokio::test]
async fn a_denied_linked_sender_hears_so_and_nothing_ran() {
    let (laptop, grizzly) = pair("denied");
    let _channel = Channel::open(&laptop, &grizzly);
    let id = message(&laptop, "clementine", "Lou", "handoff")
        .await
        .unwrap();
    until("the card", || collaboration_card(&grizzly, "lou").is_some()).await;
    let card = collaboration_card(&grizzly, "lou").unwrap();
    grizzly
        .answer_permission("lou", card["requestId"].as_str().unwrap(), "deny")
        .await
        .unwrap();
    until("the refusal", || {
        delivered(&laptop, "clementine", &id).is_some()
    })
    .await;
    let refusal = delivered(&laptop, "clementine", &id).unwrap();
    assert_eq!(refusal["cause"]["status"], "failed");
    assert_eq!(
        refusal["text"],
        "The operator denied Clementine on laptop's request to ask Lou to work."
    );
    assert!(
        requests(&grizzly)
            .iter()
            .all(|request| request["thread"].is_null()),
        "no work thread was opened"
    );
}

#[tokio::test]
async fn a_name_two_teammates_share_is_refused_with_every_choice() {
    let (laptop, grizzly) = pair("ambiguous");
    let mut local_lou = persona("lou-here");
    local_lou.name = "Lou".into();
    enrol(&laptop.log, &local_lou);
    let _channel = Channel::open(&laptop, &grizzly);

    let refused = message(&laptop, "clementine", "lou", "ask")
        .await
        .unwrap_err();
    assert_eq!(
        refused,
        format!(
            "2 teammates are called 'lou': Lou on this desk (personaId lou-here); Lou on grizzly \
             (personaId {GRIZZLY}/lou). Name the one you mean by its personaId, or as Name on Desk \
             for one on a linked desk."
        )
    );
    // Either is reached when named plainly.
    message(&laptop, "clementine", "Lou on grizzly", "ask")
        .await
        .unwrap();
    message(&laptop, "clementine", "lou-here", "ask")
        .await
        .unwrap();
}

#[tokio::test]
async fn a_linked_desk_is_sent_roster_rows_and_nothing_else() {
    let (laptop, grizzly) = pair("roster-fields");
    let mut lou = grizzly.persona("lou").unwrap();
    lou.model_id = Some("secret-model".into());
    lou.allowed_senders = vec!["someone".into()];
    crate::room::append_persona(&grizzly.log, &lou).unwrap();
    let published = serde_json::to_value(grizzly.federation_roster_entries()).unwrap();
    for row in published.as_array().unwrap() {
        let mut fields: Vec<&str> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        fields.sort();
        assert_eq!(
            fields,
            ["activity", "name", "personaId", "state", "workingOn"]
        );
        let mut working_on: Vec<&str> = row["workingOn"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        working_on.sort();
        assert_eq!(working_on, ["lastTurnAt", "title"]);
    }
    assert!(!published.to_string().contains("secret-model"));
    assert!(
        !published
            .to_string()
            .contains(&*std::env::temp_dir().to_string_lossy())
    );
    let _ = laptop;
}
