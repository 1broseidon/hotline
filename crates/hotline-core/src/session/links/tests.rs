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
use crate::test_support::until;
use std::sync::Mutex;
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
    desk_on(name, teammate, other, dials, Fake::new(answers()))
}

/// The same, its agents built from `agents`.
fn desk_on(
    name: &str,
    teammate: (&str, &str),
    other: (&str, &str),
    dials: bool,
    agents: Arc<Fake>,
) -> Arc<Room> {
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
    Room::with_agents(log, Arc::new(DeskKeys), agents)
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

/// What one direction of the channel does with each call's params before the
/// other desk answers it: `None` lets it through, `Some` refuses it with that
/// sentence, as a desk that cannot read it would.
type Screen = Box<dyn Fn(&Value) -> Option<String> + Send>;

fn through() -> Screen {
    Box::new(|_| None)
}

/// One direction of an open channel: `from`'s roster published to `to`, as
/// the channel does before anything else, then every call `from` makes for
/// `to_id`'s desk seen by `screen` and answered by `to`'s federation
/// handler, as from `from_id`.
fn pump(
    from: &Arc<Room>,
    to: &Arc<Room>,
    from_id: &str,
    to_id: &str,
    screen: Screen,
) -> (u64, tokio::task::JoinHandle<()>) {
    to.federation_roster(from_id, from.federation_roster_entries())
        .unwrap();
    let (calls, mut waiting) = mpsc::channel::<linked::Call>(32);
    let generation = from.links.opened(to_id, calls);
    let to = to.clone();
    let from_id = from_id.to_string();
    let task = tokio::spawn(async move {
        while let Some(call) = waiting.recv().await {
            let answer = match (screen(&call.params), call.cmd) {
                (Some(refused), _) => Err(refused),
                (None, "federation.deliver") => {
                    let delivery = serde_json::from_value(call.params["delivery"].clone()).unwrap();
                    to.federation_deliver(&from_id, delivery)
                }
                (None, "federation.unlink") => to.unlinked(&from_id).await.map(|()| Value::Null),
                (None, other) => panic!("nothing sends {other} here"),
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
        Self::screened(laptop, grizzly, through())
    }

    /// The channel open, with every call grizzly makes of the laptop seen by
    /// `screen` first.
    fn screened(laptop: &Arc<Room>, grizzly: &Arc<Room>, screen: Screen) -> Self {
        Self {
            laptop: laptop.clone(),
            grizzly: grizzly.clone(),
            ends: vec![
                pump(laptop, grizzly, LAPTOP, GRIZZLY, through()),
                pump(grizzly, laptop, GRIZZLY, LAPTOP, screen),
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

/// Lou is known on the laptop by name from before, so a message can be
/// addressed to him and queued while grizzly is away.
fn lou_known(laptop: &Room) {
    let mut record = linked::record(&laptop.log, GRIZZLY).unwrap();
    record.teammates.push(Known {
        persona_id: "lou".into(),
        name: "Lou".into(),
    });
    linked::save(&laptop.log, &record).unwrap();
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
async fn a_linked_teammates_ask_is_answered_by_the_recipients_main_conversation() {
    let (laptop, grizzly) = pair("ask-main");
    let _channel = Channel::open(&laptop, &grizzly);
    grizzly
        .allow_sender("lou", &format!("{LAPTOP}/clementine"))
        .unwrap();
    let id = message(&laptop, "clementine", "Lou", "ask").await.unwrap();
    until("the answer is in Clementine's conversation", || {
        delivered(&laptop, "clementine", &id).is_some()
    })
    .await;

    let lou = grizzly.tape("lou");
    let question = lou
        .iter()
        .position(|v| v["cause"]["kind"] == "question" && v["cause"]["requestId"] == id.as_str())
        .expect("the question is in Lou's main conversation on grizzly");
    assert_eq!(lou[question]["text"], "Check the CI runners.");
    let reply = lou[question..]
        .iter()
        .find(|v| v["kind"] == "agent")
        .expect("Lou answers it there");
    let answer = delivered(&laptop, "clementine", &id).unwrap();
    assert_eq!(answer["cause"]["status"], "done");
    assert_eq!(answer["text"], reply["text"]);
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
    assert_eq!(
        grizzly.persona("lou").unwrap().allowed_senders,
        [] as [String; 0]
    );
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
async fn a_handoff_naming_its_thread_continues_it_on_the_recipients_desk() {
    let (laptop, grizzly) = pair("handoff-continues");
    let _channel = Channel::open(&laptop, &grizzly);
    grizzly
        .allow_sender("lou", &format!("{LAPTOP}/clementine"))
        .unwrap();
    let first = message(&laptop, "clementine", "Lou", "handoff")
        .await
        .unwrap();
    until("the first result", || {
        delivered(&laptop, "clementine", &first).is_some()
    })
    .await;
    let sent = TeammateTools::new(&laptop, "clementine")
        .call(
            "message_teammate",
            &json!({"to": "Lou", "message": "And the staging runners.", "intent": "handoff", "thread": first}),
        )
        .await
        .unwrap();
    let second = serde_json::from_str::<Value>(&sent).unwrap()["requestId"]
        .as_str()
        .unwrap()
        .to_string();
    until("the second result", || {
        delivered(&laptop, "clementine", &second).is_some()
    })
    .await;
    let thread = |id: &str| request(&grizzly, id).unwrap()["thread"].clone();
    assert!(thread(&first).is_string());
    assert_eq!(thread(&second), thread(&first));
}

#[tokio::test]
async fn a_message_waits_for_the_link_and_an_answer_waits_for_the_sender_to_come_back() {
    let (laptop, grizzly) = pair("offline");
    lou_known(&laptop);
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
    let (generation, task) = pump(&laptop, &grizzly, LAPTOP, GRIZZLY, through());
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
    assert_eq!(list(&laptop, "clementine").await, [] as [Value; 0]);
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
    assert_eq!(
        grizzly.persona("lou").unwrap().allowed_senders,
        [] as [String; 0]
    );
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

/// A linked desk is sent Lou's roster row and profile — his goal, agent,
/// model and capabilities, a granted server by its name — and lists them
/// as a local colleague's; never his workspace, the server's address or
/// login, his grants' ids or his allowed senders.
#[tokio::test]
async fn a_linked_desk_is_sent_roster_rows_and_nothing_else() {
    let (laptop, grizzly) = pair("roster-fields");
    grizzly
        .log
        .append(
            &StreamId::Room,
            &json!({"kind": "setting", "id": "mcpServers", "value": [
                {"id": "docs-server-id", "name": "Docs", "type": "http",
                 "url": "https://mcp.example/sk-live-URLTOKEN",
                 "auth": {"mode": "header", "name": "X-Leaky-Header"}},
            ]}),
        )
        .unwrap();
    let mut lou = grizzly.persona("lou").unwrap();
    lou.cwd = "/Users/lou/sk-live-CWDTOKEN".into();
    lou.model_id = Some("openai/gpt-5".into());
    lou.allowed_senders = vec!["someone-allowed".into()];
    lou.mcp_policy = crate::contract::McpPolicy {
        mode: crate::contract::PolicyMode::All,
        server_ids: Vec::new(),
    };
    crate::room::append_persona(&grizzly.log, &lou).unwrap();
    let published = serde_json::to_value(grizzly.federation_roster_entries()).unwrap();
    for row in published.as_array().unwrap() {
        let mut fields: Vec<&str> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        fields.sort_unstable();
        assert_eq!(
            fields,
            [
                "activity",
                "agent",
                "capabilities",
                "effort",
                "goal",
                "model",
                "name",
                "personaId",
                "state",
                "workingOn"
            ]
        );
        let mut working_on: Vec<&str> = row["workingOn"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        working_on.sort_unstable();
        assert_eq!(working_on, ["lastTurnAt", "title"]);
    }
    let sent = published.to_string();
    for never in [
        "sk-live",
        "mcp.example",
        "X-Leaky-Header",
        "docs-server-id",
        "someone-allowed",
        &*std::env::temp_dir().to_string_lossy(),
    ] {
        assert!(!sent.contains(never), "{never} crossed the link: {sent}");
    }

    let _channel = Channel::open(&laptop, &grizzly);
    let listed = list(&laptop, "clementine").await;
    assert_eq!(listed[0]["desk"], "grizzly");
    assert_eq!(listed[0]["goal"], lou.goal);
    assert_eq!(listed[0]["agent"], "Hotline Agent");
    assert_eq!(listed[0]["model"], "openai/gpt-5");
    assert_eq!(listed[0]["capabilities"]["computer"], "none");
    assert_eq!(listed[0]["capabilities"]["mcpServers"], json!(["Docs"]));
    assert_eq!(listed[0]["capabilities"]["reach"], "whole machine");
}

#[tokio::test]
async fn a_request_stopped_on_the_recipients_desk_is_answered_so_on_the_senders() {
    let (laptop, grizzly) = pair("stopped-there");
    let _channel = Channel::open(&laptop, &grizzly);
    let id = message(&laptop, "clementine", "Lou", "ask").await.unwrap();
    until("the card", || collaboration_card(&grizzly, "lou").is_some()).await;
    grizzly
        .stop_exchange(&format!("{LAPTOP}/clementine"), "lou")
        .unwrap();
    until("Clementine hears it was stopped", || {
        delivered(&laptop, "clementine", &id).is_some()
    })
    .await;
    let answer = delivered(&laptop, "clementine", &id).unwrap();
    assert_eq!(answer["cause"]["status"], "failed");
    assert_eq!(answer["text"], "The person stopped this exchange.");
    assert_eq!(request(&grizzly, &id).unwrap()["returned"], true);
}

#[tokio::test]
async fn a_linked_desk_sends_only_for_teammates_it_published_and_only_so_much_at_once() {
    let (laptop, grizzly) = pair("bounded");
    let _channel = Channel::open(&laptop, &grizzly);
    let ask = |n: usize, from: &str| LinkedDelivery::Request {
        request_id: format!("r{n}"),
        from_persona_id: from.into(),
        to: "lou".into(),
        message: "Check the CI runners.".into(),
        intent: "ask".into(),
        continues: None,
        about: None,
    };
    assert_eq!(
        grizzly
            .federation_deliver(LAPTOP, ask(0, "ghost"))
            .unwrap_err(),
        "That teammate is not on the roster its desk published here."
    );
    for n in 0..48 {
        grizzly
            .federation_deliver(LAPTOP, ask(n, "clementine"))
            .unwrap();
    }
    // The same request again is the same request, not a forty-ninth.
    grizzly
        .federation_deliver(LAPTOP, ask(47, "clementine"))
        .unwrap();
    assert_eq!(
        grizzly
            .federation_deliver(LAPTOP, ask(48, "clementine"))
            .unwrap_err(),
        "Too many messages from that desk are already waiting here."
    );
}

#[test]
fn an_answer_is_cut_to_fit_one_of_the_links_frames_however_it_escapes() {
    let fits = |reply: &str| {
        let frame = json!({"id": 1, "cmd": "federation.deliver", "params": {"delivery":
            LinkedDelivery::Result { request_id: "r".into(), reply: crossing(reply), failed: false, stopped_by_person: true }}});
        frame.to_string().len() < 1024 * 1024
    };
    assert_eq!(crossing("runner 3 is green"), "runner 3 is green");
    assert_eq!(crossing(&"x".repeat(300_000)).len(), REPLY_MAX);
    for worst in ["\"", "\u{1}", "\u{1F980}"] {
        assert!(fits(&worst.repeat(REPLY_MAX)), "{worst:?}");
    }
}

/// Every delivery grizzly has sent the laptop, in order.
type Carried = Arc<Mutex<Vec<Value>>>;

/// The channel open, with each delivery grizzly sends the laptop kept in
/// `carried`, and a report refused if the laptop is `older` than reports,
/// as a desk refuses a variant it cannot read.
fn watched(laptop: &Arc<Room>, grizzly: &Arc<Room>, carried: &Carried, older: bool) -> Channel {
    let carried = carried.clone();
    Channel::screened(
        laptop,
        grizzly,
        Box::new(move |params| {
            lock(&carried).push(params["delivery"].clone());
            (older && params["delivery"]["kind"] == "progress")
                .then(|| "unknown variant `progress`, expected `request` or `result`".to_string())
        }),
    )
}

fn reports_carried(carried: &Carried) -> Vec<Value> {
    lock(carried)
        .iter()
        .filter(|delivery| delivery["kind"] == "progress")
        .cloned()
        .collect()
}

/// The reports Clementine's tape shows, as the person reads them.
fn reports_shown(laptop: &Room) -> Vec<String> {
    laptop
        .tape("clementine")
        .into_iter()
        .filter(|event| {
            event["kind"] == "notice"
                && event["id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("progress:"))
        })
        .map(|event| event["text"].as_str().unwrap().to_string())
        .collect()
}

/// How many reports grizzly holds for a handoff, still to cross.
fn waiting_to_cross(grizzly: &Room, handoff: &str) -> usize {
    request(grizzly, handoff).unwrap()["progress"]
        .as_array()
        .map_or(0, Vec::len)
}

fn thread_on_grizzly(grizzly: &Room, handoff: &str) -> String {
    request(grizzly, handoff).unwrap()["thread"]
        .as_str()
        .expect("the handoff opened a work thread on grizzly")
        .to_string()
}

/// What Lou said in a thread on grizzly, in order.
fn said_in(grizzly: &Room, thread: &str) -> Vec<String> {
    grizzly
        .log
        .load(&StreamId::Side(thread.to_string()))
        .into_iter()
        .filter(|event| event["kind"] == "agent")
        .map(|event| event["text"].as_str().unwrap().to_string())
        .collect()
}

fn delivered_in(grizzly: &Room, thread: &str, request_id: &str) -> Option<Value> {
    grizzly
        .log
        .load(&StreamId::Side(thread.to_string()))
        .into_iter()
        .find(|event| event["kind"] == "delivery" && event["cause"]["requestId"] == request_id)
}

/// Clementine's handoff to Lou, its result heard on the laptop and Lou's
/// thread on grizzly closed by it.
async fn handed_back(laptop: &Arc<Room>, grizzly: &Arc<Room>) -> (String, String) {
    grizzly
        .allow_sender("lou", &format!("{LAPTOP}/clementine"))
        .unwrap();
    let handoff = message(laptop, "clementine", "Lou", "handoff")
        .await
        .unwrap();
    until("Clementine has heard the result", || {
        delivered(laptop, "clementine", &handoff).is_some_and(|event| event["receipt"] == "read")
            && !laptop.mid_turn("clementine")
    })
    .await;
    let thread = thread_on_grizzly(grizzly, &handoff);
    until("Lou's thread has closed", || !grizzly.work_is_open(&thread)).await;
    (handoff, thread)
}

#[tokio::test]
async fn a_turn_report_crosses_the_link_without_the_persons_words() {
    let (laptop, grizzly) = pair("report");
    let carried = Carried::default();
    let _channel = watched(&laptop, &grizzly, &carried, false);
    let (handoff, thread) = handed_back(&laptop, &grizzly).await;

    grizzly.continue_side(&thread).await.unwrap();
    grizzly
        .prompt_side(&thread, "my secret is swordfish", None)
        .await
        .unwrap();
    until("the report is shown to Clementine", || {
        !reports_shown(&laptop).is_empty()
    })
    .await;
    let report = said_in(&grizzly, &thread).last().unwrap().clone();
    assert_eq!(
        reports_shown(&laptop),
        [format!("Lou on grizzly, in your handoff thread: {report}")]
    );
    let crossed = reports_carried(&carried);
    assert_eq!(crossed.len(), 1);
    assert_eq!(crossed[0]["requestId"], handoff.as_str());
    assert_eq!(crossed[0]["text"], report.as_str());
    assert_eq!(crossed[0]["wakes"], false, "the person's turn wakes nobody");
    assert!(
        lock(&carried)
            .iter()
            .all(|delivery| !delivery.to_string().contains("swordfish")),
        "the person's words never crossed"
    );
    until("grizzly holds nothing more for it", || {
        waiting_to_cross(&grizzly, &handoff) == 0
    })
    .await;

    let heard = laptop
        .take_heard(&crate::contract::ThreadId::dm("clementine"))
        .expect("Clementine has the report to hear");
    assert!(
        heard.contains(&format!(
            "Lou on grizzly, in the thread you handed them:\n{report}"
        )),
        "{heard}"
    );
    assert!(!heard.contains("(side:"), "{heard}");
    assert!(!heard.contains("swordfish"), "{heard}");
}

/// The person on grizzly stops Lou's turn on Clementine's handoff: the stop
/// crosses as theirs, with what Lou had said, and Clementine hears it once,
/// neither woken nor handed a failure to retry.
#[tokio::test]
async fn the_persons_stop_on_a_linked_handoff_reaches_its_sender_without_waking_it() {
    let clementine = Fake::new(answers());
    let laptop = desk_on(
        "linked-stop-laptop",
        ("clementine", "Clementine"),
        (GRIZZLY, "grizzly"),
        true,
        clementine.clone(),
    );
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let lou = Fake::new(
        Scripted::new(vec![
            Update::Message {
                kind: MessageKind::Agent,
                id: "long".into(),
                text: "Running the long command.".into(),
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
    let grizzly = desk_on(
        "linked-stop-grizzly",
        ("lou", "Lou"),
        (LAPTOP, "laptop"),
        false,
        lou.clone(),
    );
    let carried = Carried::default();
    let _channel = watched(&laptop, &grizzly, &carried, false);
    laptop.start("clementine").await.unwrap();
    grizzly
        .allow_sender("lou", &format!("{LAPTOP}/clementine"))
        .unwrap();
    let handoff = message(&laptop, "clementine", "Lou", "handoff")
        .await
        .unwrap();
    until("Lou's handoff turn is running", || lou.prompts().len() == 1).await;
    let thread = thread_on_grizzly(&grizzly, &handoff);
    gate.add_permits(1);
    until("Lou has said what he is doing", || {
        said_in(&grizzly, &thread).len() == 1
    })
    .await;
    grizzly.cancel_side(&thread).unwrap();
    until("the laptop has settled the handoff", || {
        request(&laptop, &handoff).is_some_and(|r| r["phase"] != "running")
    })
    .await;

    let line = "The person stopped this handoff. Do not send it again unless they ask.";
    let said = format!("Running the long command.\n\n{line}");
    assert_eq!(request(&laptop, &handoff).unwrap()["phase"], "stopped");
    let crossed: Vec<Value> = lock(&carried)
        .iter()
        .filter(|delivery| delivery["kind"] == "result")
        .cloned()
        .collect();
    assert_eq!(crossed.len(), 1, "{crossed:?}");
    assert_eq!(crossed[0]["stoppedByPerson"], true);
    assert_eq!(crossed[0]["reply"], said.as_str());
    assert_eq!(
        reports_shown(&laptop),
        [format!("Lou on grizzly, in your handoff thread: {said}")]
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        delivered(&laptop, "clementine", &handoff).is_none(),
        "no failure is handed back"
    );
    assert!(clementine.prompts().is_empty(), "nothing woke Clementine");
    let heard = laptop
        .take_heard(&crate::contract::ThreadId::dm("clementine"))
        .expect("Clementine has the stop to hear");
    assert_eq!(heard.matches(line).count(), 1, "{heard}");
}

#[tokio::test]
async fn reports_wait_summed_while_the_link_is_down() {
    let (laptop, grizzly) = pair("reports-offline");
    let carried = Carried::default();
    let channel = watched(&laptop, &grizzly, &carried, false);
    let (handoff, thread) = handed_back(&laptop, &grizzly).await;
    channel.close();

    grizzly.continue_side(&thread).await.unwrap();
    for n in 1..=3 {
        grizzly
            .prompt_side(&thread, &format!("step {n}"), None)
            .await
            .unwrap();
        until("the report waits for the link", || {
            waiting_to_cross(&grizzly, &handoff) == n
        })
        .await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(reports_shown(&laptop).is_empty());
    let said = said_in(&grizzly, &thread);
    let reports = said[said.len() - 3..].join("\n\n");

    let _channel = watched(&laptop, &grizzly, &carried, false);
    until("the reports have crossed", || {
        !reports_shown(&laptop).is_empty() && waiting_to_cross(&grizzly, &handoff) == 0
    })
    .await;
    assert_eq!(
        reports_shown(&laptop),
        [format!("Lou on grizzly, in your handoff thread: {reports}")]
    );
    let crossed = reports_carried(&carried);
    assert_eq!(crossed.len(), 1, "one crossing carried the three");
    assert_eq!(crossed[0]["text"], reports.as_str());
}

#[tokio::test]
async fn a_waking_report_keeps_its_wake_across_the_link_and_wakes_the_idle_delegator() {
    let (laptop, grizzly) = pair("report-wakes");
    let carried = Carried::default();
    let channel = watched(&laptop, &grizzly, &carried, false);
    let (handoff, thread) = handed_back(&laptop, &grizzly).await;
    channel.close();
    let replies = || {
        laptop
            .tape("clementine")
            .iter()
            .filter(|event| event["kind"] == "agent")
            .count()
    };
    let before = replies();

    grizzly.continue_side(&thread).await.unwrap();
    grizzly.report_progress(
        &thread,
        "Waiting on the person: Approve the deploy".into(),
        true,
    );
    // The person's turn after it, and the thread's rest, leave its wake be.
    grizzly.prompt_side(&thread, "step", None).await.unwrap();
    until("the thread rests with both reports", || {
        waiting_to_cross(&grizzly, &handoff) == 2
            && !grizzly.work_thread_working(&crate::contract::ThreadId::side(&thread))
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let _channel = watched(&laptop, &grizzly, &carried, false);
    until("Clementine is woken and hears them", || {
        replies() > before && request(&laptop, &handoff).unwrap()["progress"].is_null()
    })
    .await;
    let crossed = reports_carried(&carried);
    assert_eq!(crossed.len(), 1);
    assert_eq!(crossed[0]["wakes"], true);
}

#[tokio::test]
async fn an_ask_naming_a_thread_and_a_question_back_cross_the_link() {
    let (laptop, grizzly) = pair("asks-across");
    let _channel = Channel::open(&laptop, &grizzly);
    let (handoff, thread) = handed_back(&laptop, &grizzly).await;

    let sent = TeammateTools::new(&laptop, "clementine")
        .call(
            "message_teammate",
            &json!({"to": "Lou", "message": "Which branch is it on?", "intent": "ask", "thread": handoff}),
        )
        .await
        .unwrap();
    let ask = serde_json::from_str::<Value>(&sent).unwrap()["requestId"]
        .as_str()
        .unwrap()
        .to_string();
    until("the answer is back on the laptop", || {
        delivered(&laptop, "clementine", &ask).is_some()
    })
    .await;
    assert_eq!(request(&grizzly, &ask).unwrap()["thread"], thread.as_str());
    let stream = grizzly.log.load(&StreamId::Side(thread.clone()));
    let question = stream
        .iter()
        .position(|v| v["cause"]["kind"] == "question" && v["cause"]["requestId"] == ask.as_str())
        .expect("the question is in Lou's thread on grizzly");
    let reply = stream[question..]
        .iter()
        .find(|v| v["kind"] == "agent")
        .expect("Lou answers it there");
    let answer = delivered(&laptop, "clementine", &ask).unwrap();
    assert_eq!(answer["cause"]["status"], "done");
    assert_eq!(answer["text"], reply["text"]);
    assert!(
        grizzly.work_is_open(&thread),
        "answering an ask does not close the thread"
    );

    let sent = TeammateTools::new(&grizzly, "lou")
        .for_work(thread.clone())
        .call(
            "message_teammate",
            &json!({"to": "Clementine", "message": "Which port?", "intent": "ask"}),
        )
        .await
        .unwrap();
    let back = serde_json::from_str::<Value>(&sent).unwrap()["requestId"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(request(&grizzly, &back).unwrap()["about"], handoff.as_str());
    until("the answer is back in Lou's thread", || {
        delivered_in(&grizzly, &thread, &back).is_some()
    })
    .await;
    let tape = laptop.tape("clementine");
    let asked = tape
        .iter()
        .position(|v| v["cause"]["kind"] == "question" && v["cause"]["requestId"] == back.as_str())
        .expect("the question is in Clementine's DM");
    assert_eq!(tape[asked]["cause"]["work"], "Check the CI runners.");
    assert_eq!(tape[asked]["text"], "Which port?");
    let reply = tape[asked..]
        .iter()
        .find(|v| v["kind"] == "agent")
        .expect("Clementine answers it");
    let answer = delivered_in(&grizzly, &thread, &back).unwrap();
    assert_eq!(answer["cause"]["status"], "done");
    assert_eq!(answer["text"], reply["text"]);
    assert!(
        collaboration_card(&laptop, "clementine").is_none(),
        "the delegation is the grant"
    );
}

#[tokio::test]
async fn an_older_desk_refusing_a_report_still_gets_the_handoffs_result() {
    let (laptop, grizzly) = pair("older-desk");
    lou_known(&laptop);
    grizzly
        .allow_sender("lou", &format!("{LAPTOP}/clementine"))
        .unwrap();
    // Only the laptop's calls cross at first: the handoff reaches grizzly,
    // and its result waits there.
    let (generation, task) = pump(&laptop, &grizzly, LAPTOP, GRIZZLY, through());
    let handoff = message(&laptop, "clementine", "Lou", "handoff")
        .await
        .unwrap();
    until("grizzly holds the result", || {
        request(&grizzly, &handoff).is_some_and(|r| r["phase"] == "reply")
    })
    .await;
    task.abort();
    laptop.links.closed(GRIZZLY, Some(generation), None);
    let thread = thread_on_grizzly(&grizzly, &handoff);
    until("Lou's thread has closed", || !grizzly.work_is_open(&thread)).await;
    grizzly.continue_side(&thread).await.unwrap();
    grizzly
        .prompt_side(&thread, "And the staging runners?", None)
        .await
        .unwrap();
    until("the report waits beside the result", || {
        waiting_to_cross(&grizzly, &handoff) == 1
    })
    .await;

    let carried = Carried::default();
    let _channel = watched(&laptop, &grizzly, &carried, true);
    until("the result reaches Clementine", || {
        delivered(&laptop, "clementine", &handoff).is_some()
    })
    .await;
    let result = delivered(&laptop, "clementine", &handoff).unwrap();
    assert_eq!(result["cause"]["status"], "done");
    assert_eq!(result["text"], "runner 0 is green");
    until("grizzly dropped the refused report", || {
        waiting_to_cross(&grizzly, &handoff) == 0
            && request(&grizzly, &handoff).is_some_and(|r| r["phase"] == "done")
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        reports_carried(&carried).len(),
        1,
        "refused once and never sent again"
    );
    assert!(reports_shown(&laptop).is_empty());
}

#[tokio::test]
async fn a_linked_desk_reports_and_asks_back_only_by_this_desks_own_requests() {
    let (laptop, grizzly) = pair("named-by-id");
    let _channel = Channel::open(&laptop, &grizzly);
    let report = |id: &str| LinkedDelivery::Progress {
        request_id: id.into(),
        text: "runner 3 is green".into(),
        wakes: true,
    };
    assert_eq!(
        laptop
            .federation_deliver(GRIZZLY, report("made-up"))
            .unwrap_err(),
        "This desk sent no such request."
    );
    // An ask has no thread that reports.
    let ask = message(&laptop, "clementine", "Lou", "ask").await.unwrap();
    assert_eq!(
        laptop
            .federation_deliver(GRIZZLY, report(&ask))
            .unwrap_err(),
        "This desk sent no such request."
    );
    assert!(reports_shown(&laptop).is_empty());

    let asking = |intent: &str, continues: Option<&str>, about: &str| LinkedDelivery::Request {
        request_id: "q1".into(),
        from_persona_id: "lou".into(),
        to: "clementine".into(),
        message: "Which port?".into(),
        intent: intent.into(),
        continues: continues.map(str::to_string),
        about: Some(about.into()),
    };
    for refused in [
        asking("handoff", None, "h1"),
        asking("ask", None, "not an id"),
        asking("ask", Some("h1"), "h1"),
    ] {
        assert_eq!(
            laptop.federation_deliver(GRIZZLY, refused).unwrap_err(),
            "Only an ask names the handoff it is about, by a request's id."
        );
    }
    // A request id that names no handoff of Clementine's to Lou grants
    // nothing: the question meets the person's card, as any would.
    laptop
        .federation_deliver(GRIZZLY, asking("ask", None, &ask))
        .unwrap();
    until("the card is on Clementine's tape", || {
        collaboration_card(&laptop, "clementine").is_some()
    })
    .await;
}

const THIRD: &str = "third-desk";

/// A third desk linked with the laptop, whose Lou is not grizzly's.
fn third_desk(laptop: &Room) {
    linked::save(
        &laptop.log,
        &Record {
            desk_id: THIRD.into(),
            name: "third".into(),
            dials: true,
            key: None,
            teammates: Vec::new(),
        },
    )
    .unwrap();
    laptop
        .federation_roster(
            THIRD,
            vec![LinkedTeammate {
                persona_id: "lou".into(),
                name: "Lou".into(),
                state: "idle".into(),
                ..Default::default()
            }],
        )
        .unwrap();
}

fn report_for(handoff: &str, text: &str) -> LinkedDelivery {
    LinkedDelivery::Progress {
        request_id: handoff.into(),
        text: text.into(),
        wakes: false,
    }
}

/// Whether a collaboration card asks Clementine's person about `sender`.
fn carded_for(laptop: &Room, sender: &str) -> bool {
    laptop.tape("clementine").iter().any(|event| {
        event["kind"] == "permission"
            && event["title"]
                .as_str()
                .is_some_and(|title| title.starts_with(&format!("Allow {sender} to ask")))
    })
}

#[tokio::test]
async fn a_report_from_a_desk_the_handoff_was_not_sent_to_is_refused() {
    let (laptop, grizzly) = pair("third-desk-report");
    let _channel = Channel::open(&laptop, &grizzly);
    let (handoff, _) = handed_back(&laptop, &grizzly).await;
    third_desk(&laptop);

    assert_eq!(
        laptop
            .federation_deliver(THIRD, report_for(&handoff, "runner 3 is green"))
            .unwrap_err(),
        "This desk sent no such request."
    );
    assert!(reports_shown(&laptop).is_empty());
    laptop
        .federation_deliver(GRIZZLY, report_for(&handoff, "runner 3 is green"))
        .unwrap();
    assert_eq!(
        reports_shown(&laptop),
        ["Lou on grizzly, in your handoff thread: runner 3 is green"]
    );
}

#[tokio::test]
async fn a_question_back_about_another_teammates_handoff_meets_the_card() {
    let (laptop, grizzly) = pair("about-another");
    let mut ray = persona("ray");
    ray.name = "Ray".into();
    enrol(&grizzly.log, &ray);
    let _channel = Channel::open(&laptop, &grizzly);
    let (handoff, _) = handed_back(&laptop, &grizzly).await;
    third_desk(&laptop);

    // The handoff went to Lou on grizzly: neither Ray beside him nor
    // another desk's Lou asks back about it without the person's card.
    let asking = |id: &str, from: &str| LinkedDelivery::Request {
        request_id: id.into(),
        from_persona_id: from.into(),
        to: "clementine".into(),
        message: "Which port?".into(),
        intent: "ask".into(),
        continues: None,
        about: Some(handoff.clone()),
    };
    laptop
        .federation_deliver(GRIZZLY, asking("ray-asks", "ray"))
        .unwrap();
    laptop
        .federation_deliver(THIRD, asking("lou-asks", "lou"))
        .unwrap();
    until("each question meets the person's card", || {
        carded_for(&laptop, "Ray on grizzly") && carded_for(&laptop, "Lou on third")
    })
    .await;
    assert!(delivered(&laptop, "clementine", "ray-asks").is_none());
    assert!(delivered(&laptop, "clementine", "lou-asks").is_none());
}

#[tokio::test]
async fn reports_from_a_thread_and_its_continuation_on_a_linked_desk_are_heard_as_one() {
    let (laptop, grizzly) = pair("continued-reports");
    let _channel = Channel::open(&laptop, &grizzly);
    let (first, _) = handed_back(&laptop, &grizzly).await;
    let sent = TeammateTools::new(&laptop, "clementine")
        .call(
            "message_teammate",
            &json!({"to": "Lou", "message": "And the staging runners.", "intent": "handoff", "thread": first}),
        )
        .await
        .unwrap();
    let second = serde_json::from_str::<Value>(&sent).unwrap()["requestId"]
        .as_str()
        .unwrap()
        .to_string();
    until("Clementine has heard the second result", || {
        delivered(&laptop, "clementine", &second).is_some_and(|event| event["receipt"] == "read")
            && !laptop.mid_turn("clementine")
    })
    .await;

    for (handoff, text) in [
        (&first, "Swatch 1 is ready."),
        (&second, "Swatch 2 is ready."),
    ] {
        laptop
            .federation_deliver(GRIZZLY, report_for(handoff, text))
            .unwrap();
    }
    let heard = laptop
        .take_heard(&crate::contract::ThreadId::dm("clementine"))
        .expect("Clementine has the reports to hear");
    assert_eq!(
        heard.matches("in the thread you handed them").count(),
        1,
        "{heard}"
    );
    assert!(
        heard.contains("Swatch 1 is ready.\n---\nSwatch 2 is ready."),
        "{heard}"
    );
}

#[tokio::test]
async fn a_report_erased_by_stop_exchange_while_another_crosses_never_crosses() {
    let (laptop, grizzly) = pair("erased-reports");
    let channel = Channel::open(&laptop, &grizzly);
    let (first, one) = handed_back(&laptop, &grizzly).await;
    let second = message(&laptop, "clementine", "Lou", "handoff")
        .await
        .unwrap();
    until("Clementine has heard the second result", || {
        delivered(&laptop, "clementine", &second).is_some_and(|event| event["receipt"] == "read")
            && !laptop.mid_turn("clementine")
    })
    .await;
    let two = thread_on_grizzly(&grizzly, &second);
    until("Lou's second thread has closed", || {
        !grizzly.work_is_open(&two)
    })
    .await;
    channel.close();
    for (handoff, thread) in [(&first, &one), (&second, &two)] {
        grizzly.continue_side(thread).await.unwrap();
        grizzly.prompt_side(thread, "step", None).await.unwrap();
        until("the report waits for the link", || {
            waiting_to_cross(&grizzly, handoff) == 1
        })
        .await;
    }

    // The test is the laptop's end: it holds the first report's call while
    // the person on grizzly stops the exchange.
    let (calls, mut crossing) = mpsc::channel::<linked::Call>(32);
    grizzly.links.opened(LAPTOP, calls);
    let held = tokio::time::timeout(Duration::from_secs(10), crossing.recv())
        .await
        .expect("a report crosses")
        .unwrap();
    assert_eq!(held.params["delivery"]["kind"], "progress");
    grizzly
        .stop_exchange(&format!("{LAPTOP}/clementine"), "lou")
        .unwrap();
    let _ = held.answer.send(Ok(Value::Null));
    // The stop itself is still told to the laptop; no report is.
    let until = tokio::time::Instant::now() + Duration::from_millis(500);
    while let Ok(Some(call)) = tokio::time::timeout_at(until, crossing.recv()).await {
        assert_ne!(
            call.params["delivery"]["kind"], "progress",
            "a report the stop erased crossed: {}",
            call.params
        );
        let _ = call.answer.send(Ok(Value::Null));
    }
}
